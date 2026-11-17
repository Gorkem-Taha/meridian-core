//! The `meridian` command.
//!
//! It is a thin shell: it reads documents, loads plugins, and prints what the
//! orchestrator reports. Stdout carries results, stderr carries the tools'
//! own output and errors, and the exit status says whether everything asked
//! for was done.

mod import;

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
    /// Turn a tool's existing configuration into Meridian documents
    ///
    /// Reads the files, or the files in the directories, and writes one
    /// document for each thing the plugin finds in them. Whatever cannot be
    /// carried over is reported and marked in the documents with
    /// TODO(meridian-import); nothing is dropped quietly. Nothing is run and
    /// nothing is changed: importing only reads.
    Import {
        /// The files to import, or directories holding them
        #[arg(required = true)]
        paths: Vec<PathBuf>,
        /// The tool the files belong to, as the name of its plugin, such as
        /// terraform or ansible. Left out, it is worked out from the file
        /// names when only one plugin claims them
        #[arg(long, value_name = "TARGET")]
        from: Option<String>,
        /// What to call the imported document. Defaults to the name of the
        /// directory or file
        #[arg(long, value_name = "NAME")]
        name: Option<String>,
        /// Write the documents into this directory, one file each. Left out,
        /// they are printed
        #[arg(short, long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Look in the directories inside a directory too
        #[arg(short, long)]
        recursive: bool,
        /// Replace a document that already exists in the output directory
        #[arg(long)]
        force: bool,
        /// Exit with an error when anything was not carried over as written.
        /// The documents are still written
        #[arg(long)]
        strict: bool,
        #[command(flatten)]
        plugins: PluginDirs,
    },
    /// List the plugins Meridian can find and what each can do
    Plugins {
        #[command(flatten)]
        plugins: PluginDirs,
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
        Command::Import {
            paths,
            from,
            name,
            out,
            recursive,
            force,
            strict,
            plugins,
        } => import_configuration(
            &plugins,
            &import::Request {
                paths: &paths,
                from: from.as_deref(),
                name: name.as_deref(),
                out: out.as_deref(),
                recursive,
                force,
                strict,
            },
        ),
        Command::Plugins { plugins } => list_plugins(&plugins),
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

fn import_configuration(plugins: &PluginDirs, request: &import::Request) -> Result<(), Failure> {
    let registry = registry(plugins)?;
    let outcome = import::run(&registry, request);
    if let Err(e) = registry.close() {
        eprintln!("warning: {e}");
    }
    outcome
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
        if !info.import_formats.is_empty() {
            can.push("import");
        }
        println!("{name}");
        println!("  category:  {}", info.category);
        println!("  can:       {}", can.join(", "));
        println!("  resources: {}", info.resource_types.join(", "));
        if !info.target_versions.is_empty() {
            println!("  versions:  {}", info.target_versions.join(", "));
        }
        for format in &info.import_formats {
            println!(
                "  imports:   {} ({})",
                format.description,
                format.patterns.join(", ")
            );
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
