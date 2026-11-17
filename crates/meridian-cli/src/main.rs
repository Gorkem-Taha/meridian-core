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
use meridian_plugin_host::{Registry, StartOptions};
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
        /// Where fetched modules are kept, for a file written @module:path
        #[arg(
            long,
            value_name = "DIR",
            env = "MERIDIAN_STATE_DIR",
            default_value = ".meridian"
        )]
        state_dir: PathBuf,
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
    /// Work with compile rules: the policies layered data must keep
    Rules {
        #[command(subcommand)]
        command: RulesCommand,
    },
    /// Rewrite documents into a newer language version (apiVersion)
    Migrate {
        /// The files to rewrite, in place
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// The language version to move to, such as meridian/v1. Defaults to
        /// the newest this build supports
        #[arg(long, value_name = "APIVERSION")]
        to: Option<String>,
        /// Change nothing; exit non-zero if any file would change
        #[arg(long)]
        check: bool,
    },
    /// Show every version this build carries: core, language, SDK and
    /// plugin protocol
    Version,
    /// Fetch, pin and check the modules the project's meridian.yaml requires
    Module {
        #[command(subcommand)]
        command: ModuleCommand,
        /// Where fetched modules are kept
        #[arg(
            long,
            global = true,
            value_name = "DIR",
            env = "MERIDIAN_STATE_DIR",
            default_value = ".meridian"
        )]
        state_dir: PathBuf,
    },
}

#[derive(Subcommand)]
enum ModuleCommand {
    /// Fetch what meridian.yaml requires and write meridian.lock
    Install {
        /// Fail if meridian.lock would have to change; for CI
        #[arg(long)]
        locked: bool,
    },
    /// Move pinned modules to the newest version that still fits
    Update {
        /// The modules to move; every module when none is named
        names: Vec<String>,
    },
    /// Check every pinned module against the hash in meridian.lock
    Verify,
    /// List the pinned modules, their versions and where they come from
    List,
    /// Write this module as the archive a registry stores
    Pack {
        /// The file to write. Defaults to a file under the state directory
        #[arg(short, long, value_name = "FILE")]
        out: Option<PathBuf>,
    },
    /// Publish this module to an OCI registry, under its version
    Push {
        /// The repository to publish to, such as
        /// registry.example.com/team/module, with no scheme and no tag
        repository: String,
        /// Talk plain HTTP. For a registry on a trusted network only
        #[arg(long)]
        insecure: bool,
    },
}

#[derive(Subcommand)]
enum RulesCommand {
    /// Load a hierarchy's layers and check them against its compile rules,
    /// compiling nothing
    Check {
        /// The hierarchy file; its layers are found relative to it
        #[arg(long, value_name = "FILE")]
        hierarchy: PathBuf,
        /// The rules file. Defaults to rules.yaml beside the hierarchy file
        #[arg(long, value_name = "FILE")]
        rules: Option<PathBuf>,
        /// Fail when there is no rules file, so that removing it cannot
        /// switch the rules off unnoticed
        #[arg(long)]
        require_rules: bool,
        /// One value, as on compile: merged over the hierarchy last and
        /// checked as its most specific layer. The hierarchy's layer paths
        /// also read it as %{KEY}; may be given more than once
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
        /// A YAML or JSON file of values merged over the hierarchy, checked
        /// as its most specific layer; may be given more than once
        #[arg(long, value_name = "FILE")]
        data: Vec<PathBuf>,
        /// Where fetched modules are kept, for a --data file named
        /// @<module>:<path>
        #[arg(
            long,
            value_name = "DIR",
            env = "MERIDIAN_STATE_DIR",
            default_value = ".meridian"
        )]
        state_dir: PathBuf,
    },
}

#[derive(Args)]
struct PluginDirs {
    /// A directory to look for plugins in; may be given more than once.
    /// Defaults to the directory the meridian binary is in
    #[arg(
        long = "plugin-dir",
        value_name = "DIR",
        env = "MERIDIAN_PLUGIN_DIR",
        value_delimiter = ':'
    )]
    dirs: Vec<PathBuf>,
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
    /// One value for `when` conditions, as in `--set env=prod` or
    /// `--set region.name=eu-west-1`; may be given more than once. It is
    /// applied after every --data file, so it wins. The value is typed as
    /// YAML would type it: true, 3 and null are not text, "3" is
    #[arg(long, value_name = "KEY=VALUE")]
    set: Vec<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let outcome = match cli.command {
        Command::Validate { files, state_dir } => load(&files, &state_dir).map(|bundle| {
            println!("{} valid", count(bundle.documents.len(), "document"));
        }),
        Command::Module { command, state_dir } => module(command, &state_dir),
        Command::Compile { workflow, out } => orchestrate(&workflow, Mode::Compile, out.as_deref()),
        Command::Plan { workflow } => orchestrate(&workflow, Mode::Plan, None),
        Command::Apply { workflow } => orchestrate(&workflow, Mode::Apply, None),
        Command::Plugins { plugins } => list_plugins(&plugins),
        Command::Rules {
            command:
                RulesCommand::Check {
                    hierarchy,
                    rules,
                    require_rules,
                    set,
                    data,
                    state_dir,
                },
        } => check_rules(
            meridian_rules::Files {
                hierarchy,
                rules,
                require_rules,
            },
            &set,
            &data,
            &state_dir,
        ),
        Command::Migrate { files, to, check } => migrate(&files, to.as_deref(), check),
        Command::Version => {
            print!("{}", versions());
            Ok(())
        }
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
fn load(files: &[PathBuf], state_dir: &Path) -> Result<Bundle, Failure> {
    let problems = |errors: meridian_ir::ErrorList| {
        eprintln!("error: {errors}");
        Failure::Reported
    };
    let files = in_modules(files, state_dir)?;
    let bundle = meridian_ir::load(&files).map_err(problems)?;
    bundle.validate().map_err(problems)?;
    // Once per run, however many documents: see the versioning policy.
    if let Some(warning) = bundle.deprecation_warning() {
        eprintln!("warning: {warning}");
    }
    Ok(bundle)
}

/// Every version stream of this build, one per line.
fn versions() -> String {
    use meridian_ir::Status;
    use meridian_sdk::version::{describe_protocols, supported_protocols};
    let mut out = format!("meridian {}\n", env!("CARGO_PKG_VERSION"));
    for v in meridian_ir::VersionTable::builtin().versions() {
        let status = match v.status {
            Status::Supported => "supported".to_owned(),
            Status::Deprecated { since } => format!("deprecated since meridian {since}"),
            Status::Removed { in_release } => format!("removed in meridian {in_release}"),
        };
        out.push_str(&format!("  language         {}  {status}\n", v.name()));
    }
    out.push_str(&format!("  plugin sdk       {}\n", meridian_sdk::VERSION));
    out.push_str(&format!(
        "  plugin protocol  {}\n",
        describe_protocols(&supported_protocols())
    ));
    out.push_str(&format!(
        "  lockfile format  {}\n",
        meridian_module::LOCK_VERSION
    ));
    out
}

/// Rewrites each file into the language version asked for. A file is either
/// rewritten whole or left exactly as it was.
fn migrate(files: &[PathBuf], to: Option<&str>, check: bool) -> Result<(), Failure> {
    use meridian_ir::{MigrateError, Migrated, VersionTable};
    let table = VersionTable::builtin();
    let target = match to {
        Some(text) => table
            .check(text)
            .map_err(|e| Failure::Message(format!("--to: {e}")))?,
        None => table
            .current()
            .ok_or_else(|| Failure::Message("this build supports no language version".into()))?,
    };
    let mut failed = false;
    let mut would_change = 0usize;
    for file in files {
        let name = file.display();
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(e) => {
                eprintln!("error: {name}: cannot read file: {e}");
                failed = true;
                continue;
            }
        };
        match meridian_ir::migrate(&table, meridian_ir::STEPS, &text, target.number) {
            Ok(Migrated::Unchanged) => println!("{name}: already at {}", target.name()),
            Ok(Migrated::Rewritten(_)) if check => {
                would_change += 1;
                println!("{name}: would be migrated to {}", target.name());
            }
            Ok(Migrated::Rewritten(new)) => match replace_file(file, &new) {
                Ok(()) => println!("{name}: migrated to {}", target.name()),
                Err(e) => {
                    eprintln!("error: {name}: cannot write file: {e}");
                    failed = true;
                }
            },
            Err(MigrateError::Refused(why)) => {
                eprintln!("error: {name}: {why}");
                failed = true;
            }
            Err(MigrateError::NeedsDecisions(decisions)) => {
                eprintln!(
                    "error: {name}: not migrated, because it needs {} only you can make:",
                    count(decisions.len(), "decision")
                );
                for decision in decisions {
                    eprintln!("  {decision}");
                }
                failed = true;
            }
        }
    }
    if failed {
        return Err(Failure::Reported);
    }
    if would_change > 0 {
        return Err(Failure::Message(format!(
            "{} would change; run without --check to migrate",
            count(would_change, "file")
        )));
    }
    Ok(())
}

/// Replaces a file's content by writing beside it and renaming, so a crash
/// leaves the old file or the new one and never half of either.
fn replace_file(path: &Path, content: &str) -> std::io::Result<()> {
    let mut staged = path.as_os_str().to_owned();
    staged.push(".meridian-tmp");
    let staged = PathBuf::from(staged);
    std::fs::write(&staged, content)?;
    std::fs::rename(&staged, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&staged);
    })
}

/// The project in the current directory, as the module commands see it.
fn project(state_dir: &Path) -> meridian_module::Project<'_> {
    meridian_module::Project {
        dir: Path::new("."),
        state_dir,
        core_version: env!("CARGO_PKG_VERSION"),
    }
}

fn module_failure(e: meridian_module::Error) -> Failure {
    Failure::Message(e.to_string())
}

/// Turns each file written `@<module>:<path>` into the file it names in a
/// pinned module. Nothing is fetched: a module that is not installed, or
/// does not match its pin, stops the command here.
fn in_modules(files: &[PathBuf], state_dir: &Path) -> Result<Vec<PathBuf>, Failure> {
    use meridian_module::Modules;
    let is_reference = |file: &PathBuf| file.to_str().is_some_and(Modules::is_reference);
    if !files.iter().any(is_reference) {
        return Ok(files.to_vec());
    }
    let modules = Modules::load(&project(state_dir)).map_err(module_failure)?;
    files
        .iter()
        .map(|file| match file.to_str() {
            Some(text) if Modules::is_reference(text) => {
                modules.resolve(text).map_err(module_failure)
            }
            _ => Ok(file.clone()),
        })
        .collect()
}

fn module(command: ModuleCommand, state_dir: &Path) -> Result<(), Failure> {
    use meridian_module::{Change, Mode, LOCK_FILE};
    let project = project(state_dir);
    let named = |m: &meridian_module::Locked| {
        if m.version.is_empty() {
            m.name.clone()
        } else {
            format!("{} {}", m.name, m.version)
        }
    };
    let mode = match command {
        ModuleCommand::Install { locked: true } => Mode::Locked,
        ModuleCommand::Install { locked: false } => Mode::Install,
        ModuleCommand::Update { names } => Mode::Update(names),
        ModuleCommand::Verify => {
            let checks = meridian_module::verify(&project).map_err(module_failure)?;
            if checks.is_empty() {
                println!("no modules are pinned");
            }
            let mut failed = 0;
            for check in &checks {
                match &check.problem {
                    None => println!("ok      {}", named(&check.module)),
                    Some(problem) => {
                        failed += 1;
                        println!("FAILED  {}", named(&check.module));
                        for line in problem.lines() {
                            println!("    {line}");
                        }
                    }
                }
            }
            return if failed == 0 {
                Ok(())
            } else {
                Err(Failure::Message(format!(
                    "{} failed verification",
                    count(failed, "module")
                )))
            };
        }
        ModuleCommand::List => {
            let lock = meridian_module::Lock::load(project.dir)
                .map_err(module_failure)?
                .unwrap_or_default();
            if lock.modules.is_empty() {
                println!("no modules are pinned");
            }
            let width = |f: fn(&meridian_module::Locked) -> usize| {
                lock.modules.iter().map(f).max().unwrap_or(0)
            };
            let (names, versions) = (width(|m| m.name.len()), width(|m| m.version.len()));
            for m in &lock.modules {
                let version = if m.version.is_empty() {
                    "-"
                } else {
                    &m.version
                };
                println!("{:<names$}  {version:<versions$}  {}", m.name, m.source);
            }
            return Ok(());
        }
        ModuleCommand::Pack { out } => return pack_module(&project, out),
        ModuleCommand::Push {
            repository,
            insecure,
        } => return push_module(&project, &repository, insecure),
    };

    let report = meridian_module::install(&project, mode).map_err(module_failure)?;
    let mut unchanged = 0;
    for (m, change) in &report.changes {
        match change {
            Change::Added => println!("+ {} ({})", named(m), m.source),
            Change::Updated { from } if *from != m.version && !m.version.is_empty() => {
                println!("~ {} {from} -> {}", m.name, m.version)
            }
            Change::Updated { from } => println!("~ {} (was {from})", named(m)),
            Change::Removed => println!("- {}", named(m)),
            Change::Unchanged => unchanged += 1,
        }
    }
    if unchanged > 0 {
        println!("{} unchanged", count(unchanged, "module"));
    }
    if report.wrote_lock {
        println!("wrote {LOCK_FILE}");
    } else if report.lock.modules.is_empty() {
        println!("the project requires no modules");
    } else {
        println!("{LOCK_FILE} is up to date");
    }
    Ok(())
}

fn pack_module(
    project: &meridian_module::Project<'_>,
    out: Option<PathBuf>,
) -> Result<(), Failure> {
    let manifest = meridian_module::Manifest::load(project.dir).map_err(module_failure)?;
    let Some(version) = &manifest.version else {
        return Err(Failure::Message(format!(
            "{} has no metadata.version, and a module is published as a version",
            manifest.name
        )));
    };
    let archive = meridian_module::pack(project.dir).map_err(module_failure)?;
    let integrity = meridian_module::hash_dir(project.dir).map_err(module_failure)?;
    let out = out.unwrap_or_else(|| {
        project.state_dir.join("packages").join(format!(
            "{}-{version}.tar.gz",
            manifest.name.replace('/', "-")
        ))
    });
    let written = (|| {
        if let Some(parent) = out.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&out, &archive)
    })();
    if let Err(e) = written {
        return Err(Failure::Message(format!("writing {}: {e}", out.display())));
    }
    println!("packed {} {version} into {}", manifest.name, out.display());
    println!("  size:       {} bytes", archive.len());
    println!("  integrity:  {integrity}");
    println!("  media type: {}", meridian_module::LAYER_MEDIA_TYPE);
    Ok(())
}

fn push_module(
    project: &meridian_module::Project<'_>,
    repository: &str,
    insecure: bool,
) -> Result<(), Failure> {
    let manifest = meridian_module::Manifest::load(project.dir).map_err(module_failure)?;
    let Some(version) = &manifest.version else {
        return Err(Failure::Message(format!(
            "{} has no metadata.version, and a module is published as a version",
            manifest.name
        )));
    };
    let archive = meridian_module::pack(project.dir).map_err(module_failure)?;
    let pushed = meridian_module::push(
        repository,
        insecure,
        &manifest.name,
        version,
        &manifest.description,
        &archive,
    )
    .map_err(module_failure)?;
    if pushed.already_published {
        println!(
            "{} {version} is already published in {repository}, with this content",
            manifest.name
        );
    } else {
        println!(
            "published {} {version} to {repository}:{}",
            manifest.name, pushed.tag
        );
    }
    println!("  digest:     {}", pushed.digest);
    let integrity = meridian_module::hash_dir(project.dir).map_err(module_failure)?;
    println!("  integrity:  {integrity}");
    Ok(())
}

/// Reads the `--data` files into one map. Each file is a mapping; a key in a
/// later file replaces the same key from an earlier one, whole.
fn read_data(files: &[PathBuf], state_dir: &Path) -> Result<Data, Failure> {
    let mut data = Data::new();
    for file in &in_modules(files, state_dir)? {
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

/// `meridian rules check`: loads the layers, checks every rule, and says
/// which layer belongs to whom. Every violation is printed, not only the
/// first, and none of them shows a value.
fn check_rules(
    files: meridian_rules::Files,
    set: &[String],
    data_files: &[PathBuf],
    state_dir: &Path,
) -> Result<(), Failure> {
    // Each --data file is a layer of its own, so a refusal can say which.
    let mut command_line = Vec::new();
    for file in data_files {
        let data = read_data(std::slice::from_ref(file), state_dir)?;
        command_line.push(meridian_rules::Layer::command_line(
            format!("--data {}", file.display()),
            data,
        ));
    }
    // The --set values are what they are on compile: data, merged last. They
    // are also what a layer path's %{KEY} reads.
    let mut values = Data::new();
    apply_sets(&mut values, set)?;
    let scope: meridian_resolve::Scope = set
        .iter()
        .filter_map(|pair| pair.split_once('='))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect();
    if !values.is_empty() {
        command_line.push(meridian_rules::Layer::set("--set", values));
    }
    let checked = meridian_rules::check_files(&files, &scope, &command_line)
        .map_err(|refusal| Failure::Message(refusal.to_string()))?;
    let Some((path, rules)) = &checked.rules else {
        println!(
            "no rules file ({} beside {}): nothing was checked",
            meridian_rules::RULES_FILE,
            files.hierarchy.display()
        );
        return Ok(());
    };
    println!(
        "rules: {} ({})",
        path.display(),
        count(rules.rules.len(), "rule")
    );
    println!("layers, least specific first:");
    let width = checked
        .groups
        .iter()
        .map(|(p, _)| p.len())
        .max()
        .unwrap_or(0);
    for (layer, group) in &checked.groups {
        if rules.groups.is_empty() {
            println!("  {layer}");
        } else {
            println!("  {layer:width$}  {group}");
        }
    }
    println!("no violations");
    Ok(())
}

/// Applies the `--set key=value` options to the data, in the order given.
/// A dotted key sets a value inside a mapping, creating the mappings on the
/// way; the other keys of those mappings are kept.
fn apply_sets(data: &mut Data, sets: &[String]) -> Result<(), Failure> {
    use meridian_ir::Value;
    for set in sets {
        let refuse = |why: &str| Failure::Message(format!("--set {set}: {why}"));
        let Some((key, raw)) = set.split_once('=') else {
            return Err(refuse("expected KEY=VALUE, as in --set env=prod"));
        };
        let path: Vec<&str> = key.split('.').collect();
        if path.iter().any(|part| part.trim().is_empty()) {
            return Err(refuse(
                "the key is empty, or has an empty part between dots",
            ));
        }
        // The value is one YAML scalar, list or mapping. Text that YAML
        // cannot read as a value is taken as the text it is.
        let value = match meridian_ir::parse_value("--set", raw.as_bytes()) {
            Ok(value) => value,
            Err(_) => Value::String(raw.to_owned()),
        };
        let mut holder = &mut *data;
        for (i, part) in path[..path.len() - 1].iter().enumerate() {
            let entry = holder
                .entry((*part).to_owned())
                .or_insert_with(|| Value::Map(Data::new()));
            holder = match entry {
                Value::Map(inner) => inner,
                other => {
                    return Err(refuse(&format!(
                        "{} is already set to {}, so it cannot hold the key {:?}",
                        path[..=i].join("."),
                        other.describe(),
                        path[i + 1]
                    )))
                }
            };
        }
        holder.insert(path[path.len() - 1].to_owned(), value);
    }
    Ok(())
}

fn registry(plugins: &PluginDirs) -> Result<Registry, Failure> {
    let dirs = if plugins.dirs.is_empty() {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .into_iter()
            .collect()
    } else {
        plugins.dirs.clone()
    };
    let mut registry = Registry::new();
    for dir in dirs {
        registry
            .discover(&dir.to_string_lossy(), &StartOptions::default())
            .map_err(|e| Failure::Message(e.to_string()))?;
    }
    Ok(registry)
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
    }
    registry
        .close()
        .map_err(|e| Failure::Message(e.to_string()))
}

fn orchestrate(workflow: &Workflow, mode: Mode, out: Option<&Path>) -> Result<(), Failure> {
    let bundle = load(&workflow.files, &workflow.state_dir)?;
    let mut selector = Selector::all();
    for text in &workflow.selector {
        selector = selector.and(Selector::parse(text).map_err(Failure::Message)?);
    }
    let mut data = read_data(&workflow.data, &workflow.state_dir)?;
    apply_sets(&mut data, &workflow.set)?;
    if selected(&bundle, &selector, &data) == 0 {
        // Doing nothing and reporting success would read as "all applied".
        return Err(Failure::Message(format!(
            "no document's labels match the selector {selector}"
        )));
    }
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
        // Every decision a condition made, so that something missing from
        // the compiled output is never a surprise.
        if !report.removed.is_empty() {
            writeln!(out, "  when: {} removed", report.removed.len())?;
            for removed in &report.removed {
                writeln!(
                    out,
                    "    {}: {} is false",
                    removed.subject, removed.condition
                )?;
            }
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
            Outcome::LeftOut { condition } => {
                writeln!(out, "  skipped: its when ({condition}) is false")?
            }
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
