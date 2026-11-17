//! The `meridian` command.
//!
//! It is a thin shell: it reads documents, loads plugins, and prints what the
//! orchestrator reports. Stdout carries results, stderr carries the tools'
//! own output and errors, and the exit status says whether everything asked
//! for was done.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};
use meridian_ir::Bundle;
use meridian_orchestrator::{
    run_selected, selected, Mode, Observer, Options, Outcome, Selector, StageId, StageReport, MASK,
};
use meridian_plugin_host::{
    signature::signature_path, Policy, PublicKey, Registry, SecretKey, Signature, StartOptions,
    Verification,
};
use meridian_sdk::{ChangeAction, Data, LogLine, LogStream, Output};

#[derive(Parser)]
#[command(
    name = "meridian",
    version,
    about = "One document, any tool: compile Meridian documents and run the tools they target"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Check documents for errors without loading any plugin
    Validate {
        /// The documents to check
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Compile each document into its target's own files, and change nothing
    Compile {
        #[command(flatten)]
        workflow: Workflow,
        /// Write the compiled files here, one directory per document
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
    },
    /// Show what applying the documents would change
    Plan {
        #[command(flatten)]
        workflow: Workflow,
    },
    /// Make the documents' desired state real
    Apply {
        #[command(flatten)]
        workflow: Workflow,
    },
    /// List the plugins Meridian can find and what each can do
    Plugins {
        #[command(flatten)]
        plugins: PluginDirs,
    },
    /// Make signing keys, sign plugin binaries and check their signatures
    Signing {
        #[command(subcommand)]
        command: SigningCommand,
    },
}

#[derive(Subcommand)]
enum SigningCommand {
    /// Make a new key pair for signing plugins
    Keygen {
        /// Where to write the secret key, which signs. Keep it private
        #[arg(long, value_name = "FILE")]
        secret: PathBuf,
        /// Where to write the public key, which those who run the plugins trust
        #[arg(long, value_name = "FILE")]
        public: PathBuf,
    },
    /// Sign plugin binaries, writing <BINARY>.sig beside each
    Sign {
        /// The secret key to sign with
        #[arg(long, value_name = "FILE")]
        key: PathBuf,
        /// The binaries to sign
        #[arg(required = true)]
        binaries: Vec<PathBuf>,
    },
    /// Check the signatures of plugin binaries against the trusted keys
    Verify {
        #[command(flatten)]
        keys: TrustedKeys,
        /// The binaries to check
        #[arg(required = true)]
        binaries: Vec<PathBuf>,
    },
}

/// The public key this build's own plugins were signed with, when it is a
/// release build: the release workflow sets the variable while it compiles.
/// A build from source has none.
const RELEASE_PUBLIC_KEY: Option<&str> = option_env!("MERIDIAN_RELEASE_PUBLIC_KEY");

#[derive(Args)]
struct TrustedKeys {
    /// A public key file whose signatures on plugins are trusted, or a
    /// directory of *.pub files; may be given more than once. A released
    /// meridian already trusts the key its own plugins were signed with
    #[arg(
        long = "trusted-key",
        value_name = "FILE",
        env = "MERIDIAN_TRUSTED_KEYS",
        value_delimiter = PATH_LIST_SEPARATOR
    )]
    trusted_keys: Vec<PathBuf>,
}

impl TrustedKeys {
    /// Every key to trust: the release's own, then the ones named.
    fn read(&self) -> Result<Vec<PublicKey>, Failure> {
        let mut keys = Vec::new();
        if let Some(key) = RELEASE_PUBLIC_KEY.filter(|k| !k.trim().is_empty()) {
            keys.push(PublicKey::parse(key).map_err(|e| {
                Failure::Message(format!("the public key built into this meridian: {e}"))
            })?);
        }
        for path in &self.trusted_keys {
            if path.is_dir() {
                let mut files: Vec<PathBuf> = std::fs::read_dir(path)
                    .map_err(|e| {
                        Failure::Message(format!(
                            "reading the key directory {}: {e}",
                            path.display()
                        ))
                    })?
                    .filter_map(Result::ok)
                    .map(|entry| entry.path())
                    .filter(|file| file.extension().is_some_and(|ext| ext == "pub"))
                    .collect();
                files.sort();
                for file in files {
                    keys.push(PublicKey::read(&file).map_err(Failure::Message)?);
                }
            } else {
                keys.push(PublicKey::read(path).map_err(Failure::Message)?);
            }
        }
        Ok(keys)
    }
}

/// What separates the directories in `MERIDIAN_PLUGIN_DIR`: the system's own
/// separator for a list of paths, since a Windows path holds a colon.
const PATH_LIST_SEPARATOR: char = if cfg!(windows) { ';' } else { ':' };

#[derive(Args)]
struct PluginDirs {
    /// A directory to look for plugins in; may be given more than once.
    /// Defaults to the directory the meridian binary is in
    #[arg(
        long = "plugin-dir",
        value_name = "DIR",
        env = "MERIDIAN_PLUGIN_DIR",
        value_delimiter = PATH_LIST_SEPARATOR
    )]
    dirs: Vec<PathBuf>,
    /// What to do about plugin signatures: off (do not look), warn (start an
    /// unsigned or wrongly signed plugin, and say so) or require (start only
    /// plugins signed by a trusted key). Defaults to require in a released
    /// meridian and to off in one built from source
    #[arg(
        long = "plugin-signatures",
        value_name = "POLICY",
        env = "MERIDIAN_PLUGIN_SIGNATURES"
    )]
    signatures: Option<Policy>,
    #[command(flatten)]
    keys: TrustedKeys,
}

impl PluginDirs {
    fn verification(&self) -> Result<Verification, Failure> {
        let policy = self.signatures.unwrap_or(match RELEASE_PUBLIC_KEY {
            Some(key) if !key.trim().is_empty() => Policy::Require,
            _ => Policy::Off,
        });
        // With signatures off the keys are not needed, and a key file that
        // has gone missing should not stop a run that does not use it.
        let trusted = match policy {
            Policy::Off => Vec::new(),
            Policy::Warn | Policy::Require => self.keys.read()?,
        };
        Ok(Verification { policy, trusted })
    }
}

#[derive(Args)]
struct Workflow {
    /// The documents that make up the workflow
    #[arg(required = true)]
    files: Vec<PathBuf>,
    #[command(flatten)]
    plugins: PluginDirs,
    /// Where each tool's working files and Meridian's run-state are kept
    #[arg(
        long,
        value_name = "DIR",
        env = "MERIDIAN_STATE_DIR",
        default_value = ".meridian"
    )]
    state_dir: PathBuf,
    /// Run only the documents whose metadata.labels match, as in
    /// `tier=web,env!=prod`; may be given more than once, and every
    /// requirement must hold. Forms: key=value, key!=value, key, !key
    #[arg(short = 'l', long, value_name = "SELECTOR")]
    selector: Vec<String>,
    /// A YAML or JSON file of values that `when` conditions read as
    /// data.<key>; may be given more than once, and a later file's keys
    /// replace an earlier file's
    #[arg(long, value_name = "FILE")]
    data: Vec<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::Validate { files } => load(&files).map(|bundle| {
            println!("{} valid", count(bundle.documents.len(), "document"));
        }),
        Command::Compile { workflow, out } => orchestrate(&workflow, Mode::Compile, out.as_deref()),
        Command::Plan { workflow } => orchestrate(&workflow, Mode::Plan, None),
        Command::Apply { workflow } => orchestrate(&workflow, Mode::Apply, None),
        Command::Plugins { plugins } => list_plugins(&plugins),
        Command::Signing { command } => signing(command),
    };
    match outcome {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Reported) => ExitCode::FAILURE,
        Err(Failure::Message(msg)) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

enum Failure {
    /// Something to print.
    Message(String),
    /// Already printed, stage by stage.
    Reported,
}

fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// Reads and validates the documents. Every problem is reported, each at its
/// own line, before anything else happens.
fn load(files: &[PathBuf]) -> Result<Bundle, Failure> {
    let problems = |errors: meridian_ir::ErrorList| {
        eprintln!("error: {errors}");
        Failure::Reported
    };
    let bundle = meridian_ir::load(files).map_err(problems)?;
    bundle.validate().map_err(problems)?;
    Ok(bundle)
}

/// Reads the `--data` files into one map. Each file is a mapping; a key in a
/// later file replaces the same key from an earlier one, whole.
fn read_data(files: &[PathBuf]) -> Result<Data, Failure> {
    let mut data = Data::new();
    for file in files {
        let name = file.to_string_lossy();
        let raw = std::fs::read(file)
            .map_err(|e| Failure::Message(format!("reading the data file {name}: {e}")))?;
        match meridian_ir::parse_value(&name, &raw) {
            Ok(meridian_ir::Value::Map(map)) => data.extend(map),
            Ok(meridian_ir::Value::Null) => {}
            Ok(other) => {
                return Err(Failure::Message(format!(
                    "the data file {name} must hold a mapping of names to values, and it \
                     holds {}",
                    other.describe()
                )))
            }
            Err(e) => return Err(Failure::Message(e.to_string())),
        }
    }
    Ok(data)
}

fn registry(plugins: &PluginDirs) -> Result<Registry, Failure> {
    let dirs = if plugins.dirs.is_empty() {
        own_directory().into_iter().collect()
    } else {
        plugins.dirs.clone()
    };
    let options = StartOptions {
        verification: plugins.verification()?,
        ..StartOptions::default()
    };
    let mut registry = Registry::new();
    for dir in dirs {
        let loaded = registry
            .discover(&dir.to_string_lossy(), &options)
            .map_err(|e| Failure::Message(e.to_string()))?;
        // Under `warn` a plugin that did not pass was started all the same;
        // this is the saying so.
        for info in loaded {
            match &info.signature {
                Signature::Unsigned => {
                    eprintln!("warning: plugin {} is not signed", info.path)
                }
                Signature::Invalid(why) => eprintln!(
                    "warning: plugin {}: its signature does not pass: {why}",
                    info.path
                ),
                Signature::NotChecked | Signature::Verified { .. } => {}
            }
        }
    }
    Ok(registry)
}

fn signing(command: SigningCommand) -> Result<(), Failure> {
    match command {
        SigningCommand::Keygen { secret, public } => {
            // Neither file is replaced: writing over a secret key loses the
            // only thing that can make signatures its public key accepts.
            for path in [&secret, &public] {
                if path.exists() {
                    return Err(Failure::Message(format!(
                        "{} already exists; a key is not written over",
                        path.display()
                    )));
                }
            }
            let key = SecretKey::generate().map_err(Failure::Message)?;
            write_secret(&secret, &format!("{}\n", key.to_text()))
                .and_then(|()| std::fs::write(&public, format!("{}\n", key.public().to_text())))
                .map_err(|e| Failure::Message(format!("writing the key pair: {e}")))?;
            println!("key {}", key.public().id());
            println!("  secret: {}", secret.display());
            println!("  public: {}", public.display());
            Ok(())
        }
        SigningCommand::Sign { key, binaries } => {
            let key = SecretKey::read(&key).map_err(Failure::Message)?;
            for binary in binaries {
                let text = key.sign_file(&binary).map_err(Failure::Message)?;
                let path = signature_path(&binary);
                std::fs::write(&path, text)
                    .map_err(|e| Failure::Message(format!("writing {}: {e}", path.display())))?;
                println!("signed {} with key {}", binary.display(), key.public().id());
            }
            Ok(())
        }
        SigningCommand::Verify { keys, binaries } => {
            let trusted = keys.read()?;
            let mut failed = false;
            for binary in binaries {
                let found = meridian_plugin_host::signature::verify_file(&binary, &trusted);
                failed |= !matches!(found, Signature::Verified { .. });
                println!("{}: {found}", binary.display());
            }
            if failed {
                Err(Failure::Reported)
            } else {
                Ok(())
            }
        }
    }
}

/// Writes a secret key so that only its owner can read it, where the system
/// has such a thing as a file mode.
fn write_secret(path: &Path, text: &str) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)?.write_all(text.as_bytes())
}

/// The directory the `meridian` binary really is in.
///
/// A package manager often puts a link to the binary on the `PATH` and keeps
/// the binary itself, with its plugins beside it, somewhere else. The link is
/// followed, so the plugins are found wherever the command was started from.
fn own_directory() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe = meridian_sdk::tool::absolute_path(&exe);
    exe.parent().map(Path::to_path_buf)
}

fn list_plugins(plugins: &PluginDirs) -> Result<(), Failure> {
    let registry = registry(plugins)?;
    let names = registry.names();
    if names.is_empty() {
        println!("no plugins found");
        return Ok(());
    }
    for name in names {
        let info = registry
            .target(name)
            .expect("a listed target exists")
            .info();
        let mut can = vec!["compile"];
        if info.operations.plan {
            can.push("plan");
        }
        if info.operations.apply {
            can.push("apply");
        }
        if info.operations.outputs {
            can.push("outputs");
        }
        println!("{name}");
        println!("  category:  {}", info.category);
        println!("  can:       {}", can.join(", "));
        println!("  resources: {}", info.resource_types.join(", "));
        if !info.target_versions.is_empty() {
            println!("  versions:  {}", info.target_versions.join(", "));
        }
        println!(
            "  sdk:       {} (protocol {})",
            info.sdk_version, info.protocol_version
        );
        println!("  binary:    {}", info.path);
        println!("  signature: {}", info.signature);
    }
    registry
        .close()
        .map_err(|e| Failure::Message(e.to_string()))
}

fn orchestrate(workflow: &Workflow, mode: Mode, out: Option<&Path>) -> Result<(), Failure> {
    let bundle = load(&workflow.files)?;
    let mut selector = Selector::all();
    for text in &workflow.selector {
        selector = selector.and(Selector::parse(text).map_err(Failure::Message)?);
    }
    if selected(&bundle, &selector) == 0 {
        // Doing nothing and reporting success would read as "all applied".
        return Err(Failure::Message(format!(
            "no document's labels match the selector {selector}"
        )));
    }
    let data = read_data(&workflow.data)?;
    let registry = registry(&workflow.plugins)?;
    let options = Options {
        state_dir: workflow.state_dir.clone(),
        data,
    };
    let mut printer = Printer { mode, out };
    let report = run_selected(&bundle, &registry, mode, &options, &selector, &mut printer);
    // A plugin that will not exit is worth a warning, not a failed run.
    if let Err(e) = registry.close() {
        eprintln!("warning: {e}");
    }
    if report.succeeded() {
        Ok(())
    } else {
        Err(Failure::Reported)
    }
}

/// Prints a run as it happens.
struct Printer<'a> {
    mode: Mode,
    out: Option<&'a Path>,
}

impl Observer for Printer<'_> {
    fn log(&mut self, id: &StageId, line: &LogLine) {
        // The tools' own chatter goes to stderr, so stdout stays a clean
        // record of what Meridian found and did.
        let mark = match line.stream {
            LogStream::Stdout => "|",
            LogStream::Stderr => "!",
        };
        eprintln!("  {} {mark} {}", id.name, line.text);
    }

    fn stage_finished(&mut self, report: &StageReport) {
        let mut out = std::io::stdout().lock();
        let _ = self.print(&mut out, report);
    }
}

impl Printer<'_> {
    fn print(&self, out: &mut dyn Write, report: &StageReport) -> std::io::Result<()> {
        let on = if report.host.is_empty() {
            String::new()
        } else {
            format!(" on {}", report.host)
        };
        writeln!(
            out,
            "{} ({}, target {}{on})",
            report.id.name,
            report.id.kind.as_str(),
            report.target
        )?;
        for warning in &report.warnings {
            writeln!(out, "  warning: {warning}")?;
        }

        match &report.outcome {
            Outcome::Compiled => {
                let files = report.artifact.as_ref().map(|a| a.files.len()).unwrap_or(0);
                match (self.mode, self.out) {
                    (Mode::Compile, Some(dir)) => {
                        let dir = dir.join(&report.id.name);
                        match write_artifact(&dir, report) {
                            Ok(()) => writeln!(
                                out,
                                "  compiled {} into {}",
                                count(files, "file"),
                                dir.display()
                            )?,
                            Err(e) => writeln!(out, "  error: writing {}: {e}", dir.display())?,
                        }
                    }
                    (Mode::Compile, None) => {
                        writeln!(out, "  compiled {}", count(files, "file"))?;
                        if let Some(artifact) = &report.artifact {
                            for path in artifact.files.keys() {
                                writeln!(out, "    {path}")?;
                            }
                        }
                    }
                    _ => writeln!(
                        out,
                        "  compiled {}; this target only compiles, so there is no preview",
                        count(files, "file")
                    )?,
                }
            }
            Outcome::Planned(plan) => {
                for change in &plan.changes {
                    writeln!(out, "  {}", change_line(change))?;
                }
                if plan.unknown {
                    writeln!(out, "  this tool cannot preview; applying may change things")?;
                } else if plan.changes.is_empty() {
                    writeln!(out, "  in sync, nothing to change")?;
                } else {
                    writeln!(out, "  {} to make", count(plan.changes.len(), "change"))?;
                }
            }
            Outcome::Applied { changes, outputs } => {
                for change in changes {
                    writeln!(out, "  {}", change_line(change))?;
                }
                if changes.is_empty() {
                    writeln!(out, "  applied, nothing needed changing")?;
                } else {
                    writeln!(out, "  applied {}", count(changes.len(), "change"))?;
                }
                for output in outputs {
                    writeln!(out, "  output {} = {}", output.name, shown(output))?;
                }
            }
            Outcome::Deferred { waiting_on, output } => writeln!(
                out,
                "  deferred: needs the output {output:?} of {waiting_on}, which has not been applied yet"
            )?,
            Outcome::Failed(why) => {
                // The first line beside the label, the rest indented under it.
                let mut lines = why.lines();
                writeln!(out, "  FAILED: {}", lines.next().unwrap_or_default())?;
                for line in lines {
                    writeln!(out, "    {line}")?;
                }
            }
            Outcome::NotRun { because_of } => {
                writeln!(out, "  not run, because {because_of} failed")?
            }
        }
        Ok(())
    }
}

fn change_line(change: &meridian_sdk::Change) -> String {
    let sign = match change.action {
        ChangeAction::Create => '+',
        ChangeAction::Update => '~',
        ChangeAction::Delete => '-',
        ChangeAction::Replace => '!',
    };
    format!("{sign} {change}")
}

/// An output's value as shown to a person: never a sensitive one.
fn shown(output: &Output) -> String {
    if output.sensitive {
        MASK.to_owned()
    } else {
        output.value.canonical()
    }
}

/// Writes a compiled artifact under `dir`, refusing any path that would land
/// outside it. An artifact's paths come from a plugin, and a plugin is not
/// trusted to choose where on the operator's disk a file goes.
fn write_artifact(dir: &Path, report: &StageReport) -> std::io::Result<()> {
    let Some(artifact) = &report.artifact else {
        return Ok(());
    };
    for (relative, content) in &artifact.files {
        let path = Path::new(relative);
        let safe = !relative.is_empty()
            && path
                .components()
                .all(|c| matches!(c, std::path::Component::Normal(_)));
        if !safe {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("the plugin produced the unsafe path {relative:?}"),
            ));
        }
        let path = dir.join(path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, content)?;
    }
    Ok(())
}
