//! The `meridian` command.
//!
//! It is a thin shell: it reads documents, loads plugins, and prints what the
//! orchestrator reports. Stdout carries results, stderr carries the tools'
//! own output and errors, and the exit status says whether everything asked
//! for was done.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use clap::{Args, Parser, Subcommand, ValueEnum};
use meridian_ir::Bundle;
use meridian_orchestrator::{
    lock, record, run_selected, selected, Backend, Event, ExecBackend, LocalBackend, Mode,
    Observer, Options, Outcome, Resume, RunRecord, Selector, StageId, StageReport, MASK,
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
        /// Continue an apply that failed or was interrupted: stages it
        /// already applied, from documents that have not changed since, are
        /// left alone. With no value, the last apply; or a run id
        #[arg(long, value_name = "RUN", num_args = 0..=1, default_missing_value = "last")]
        resume: Option<String>,
    },
    /// Show the current or last run, step by step, and who holds the lock
    Status {
        #[command(flatten)]
        state: State,
    },
    /// List the runs recorded in the state, oldest first
    Runs {
        #[command(flatten)]
        state: State,
    },
    /// Print a run's events, one JSON object per line
    Events {
        /// The run; the last one when not given
        run: Option<String>,
        /// Only events of this type, such as step.failed; step.* takes a
        /// whole family; may be given more than once
        #[arg(long = "type", value_name = "TYPE")]
        types: Vec<String>,
        #[command(flatten)]
        state: State,
    },
    /// Remove the state lock a run left behind when it was killed
    Unlock {
        /// The lock's id, as `meridian status` and the refused run show it
        id: String,
        #[command(flatten)]
        state: State,
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

/// Where Meridian keeps what it knows.
#[derive(Args)]
struct State {
    #[command(flatten)]
    plugins: PluginDirs,
    /// Where each tool's working files are kept, and with the local backend
    /// Meridian's own record: the run-state, the runs and the lock
    #[arg(
        long,
        value_name = "DIR",
        env = "MERIDIAN_STATE_DIR",
        default_value = ".meridian"
    )]
    state_dir: PathBuf,
    /// Where Meridian's own record is kept: `local`, the state directory,
    /// or the name of a backend program, meridian-backend-<NAME>, in a
    /// plugin directory
    #[arg(
        long,
        value_name = "NAME",
        env = "MERIDIAN_BACKEND",
        default_value = "local"
    )]
    backend: String,
    /// A setting for the backend, as key=value; may be given more than
    /// once. The backend program reads it as MERIDIAN_BACKEND_<KEY>
    #[arg(long, value_name = "KEY=VALUE")]
    backend_config: Vec<String>,
}

/// How the events of a run are written to standard output.
#[derive(Clone, Copy, ValueEnum)]
enum EventFormat {
    /// One JSON object per line
    Json,
}

#[derive(Args)]
struct Workflow {
    /// The documents that make up the workflow
    #[arg(required = true)]
    files: Vec<PathBuf>,
    #[command(flatten)]
    state: State,
    /// How long to wait for another run to release the state lock, as in
    /// 30s, 5m or 1h. By default a locked state is refused at once
    #[arg(long, value_name = "DURATION", default_value = "0s", value_parser = parse_duration)]
    lock_timeout: Duration,
    /// Write every event of the run to standard output as it happens, one
    /// JSON object per line. The report for people moves to standard error
    #[arg(long, value_name = "FORMAT")]
    events: Option<EventFormat>,
    /// How many runs' records to keep; older ones are removed when a run ends
    #[arg(long, value_name = "N", default_value_t = meridian_orchestrator::DEFAULT_KEEP_RUNS)]
    keep_runs: usize,
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
        Command::Compile { workflow, out } => {
            orchestrate(&workflow, Mode::Compile, out.as_deref(), None)
        }
        Command::Plan { workflow } => orchestrate(&workflow, Mode::Plan, None, None),
        Command::Apply { workflow, resume } => {
            orchestrate(&workflow, Mode::Apply, None, resume.as_deref())
        }
        Command::Status { state } => status(&state),
        Command::Runs { state } => runs(&state),
        Command::Events { run, types, state } => events(&state, run.as_deref(), &types),
        Command::Unlock { id, state } => unlock(&state, &id),
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

/// A length of time as a person writes one: `90`, `30s`, `5m`, `1h`,
/// `250ms`. A bare number is seconds.
fn parse_duration(text: &str) -> Result<Duration, String> {
    let text = text.trim();
    let digits = text.chars().take_while(char::is_ascii_digit).count();
    let (number, unit) = text.split_at(digits);
    let number: u64 = number
        .parse()
        .map_err(|_| format!("{text:?} is not a duration; write one as 30s, 5m or 1h"))?;
    match unit {
        "ms" => Ok(Duration::from_millis(number)),
        "" | "s" => Ok(Duration::from_secs(number)),
        "m" => Ok(Duration::from_secs(number.saturating_mul(60))),
        "h" => Ok(Duration::from_secs(number.saturating_mul(3600))),
        other => Err(format!(
            "{other:?} is not a unit of time; use ms, s, m or h"
        )),
    }
}

/// The directories plugins, and backend programs, are looked for in.
fn plugin_dirs(plugins: &PluginDirs) -> Vec<PathBuf> {
    if plugins.dirs.is_empty() {
        std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .into_iter()
            .collect()
    } else {
        plugins.dirs.clone()
    }
}

/// Where Meridian's record is kept, as the command line chose it.
fn backend(state: &State) -> Result<Arc<dyn Backend>, Failure> {
    let mut settings = BTreeMap::new();
    for setting in &state.backend_config {
        let Some((key, value)) = setting.split_once('=').filter(|(key, _)| !key.is_empty()) else {
            return Err(Failure::Message(format!(
                "--backend-config takes key=value, and was given {setting:?}"
            )));
        };
        settings.insert(key.to_owned(), value.to_owned());
    }
    if state.backend == "local" {
        if let Some(key) = settings.keys().next() {
            return Err(Failure::Message(format!(
                "the local backend takes no settings, and was given {key:?}; it keeps the \
                 record in --state-dir"
            )));
        }
        return Ok(Arc::new(LocalBackend::new(&state.state_dir)));
    }
    ExecBackend::find(&state.backend, &plugin_dirs(&state.plugins), settings)
        .map(|found| Arc::new(found) as Arc<dyn Backend>)
        .map_err(Failure::Message)
}

fn registry(plugins: &PluginDirs) -> Result<Registry, Failure> {
    let dirs = plugin_dirs(plugins);
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

fn orchestrate(
    workflow: &Workflow,
    mode: Mode,
    out: Option<&Path>,
    resume: Option<&str>,
) -> Result<(), Failure> {
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
    // What was typed, for the record: the files and the selection. Backend
    // settings are left out, because one of them can be a credential.
    let mut command = vec![mode.as_str().to_owned()];
    command.extend(workflow.files.iter().map(|f| f.display().to_string()));
    for text in &workflow.selector {
        command.push(format!("-l {text}"));
    }
    if resume.is_some() {
        command.push("--resume".to_owned());
    }
    let options = Options {
        data,
        // Compiling keeps no record, so it needs no backend to be there.
        backend: match mode {
            Mode::Compile => None,
            Mode::Plan | Mode::Apply => Some(backend(&workflow.state)?),
        },
        lock_timeout: workflow.lock_timeout,
        resume: resume.map(|run| match run {
            "last" => Resume::Last,
            id => Resume::Run(id.to_owned()),
        }),
        command: command.join(" "),
        keep_runs: workflow.keep_runs,
        ..Options::new(&workflow.state.state_dir)
    };
    let registry = registry(&workflow.state.plugins)?;
    let mut printer = Printer {
        mode,
        out,
        events: workflow.events.is_some(),
    };
    let report = run_selected(&bundle, &registry, mode, &options, &selector, &mut printer);
    // A plugin that will not exit is worth a warning, not a failed run.
    if let Err(e) = registry.close() {
        eprintln!("warning: {e}");
    }
    for warning in &report.warnings {
        eprintln!("warning: {warning}");
    }
    if let Some(refusal) = &report.refused {
        return Err(Failure::Message(refusal.to_string()));
    }
    if report.succeeded() {
        return Ok(());
    }
    if mode == Mode::Apply {
        eprintln!(
            "run {} failed. Once the cause is fixed, `meridian apply --resume` with the same \
             documents continues from the stage that failed",
            report.run_id
        );
    }
    Err(Failure::Reported)
}

/// Shows the current or last run, and whether the state is locked.
fn status(state: &State) -> Result<(), Failure> {
    let backend = backend(state)?;
    let backend = backend.as_ref();
    let held = lock::holder(backend).map_err(Failure::Message)?;
    match record::last_run(backend, None).map_err(Failure::Message)? {
        None => println!("no run has been recorded in {}", backend.describe()),
        Some(run) => print_run(&run, held.as_ref().is_some_and(|lock| lock.id == run.id)),
    }
    match held {
        Some(lock) => println!("locked: {lock}"),
        None => println!("not locked"),
    }
    Ok(())
}

fn print_run(run: &RunRecord, holds_lock: bool) {
    let status = match (run.status, holds_lock) {
        (record::RunStatus::Running, false) => {
            "interrupted: it never said how it ended, and it does not hold the lock"
        }
        (status, _) => status.as_str(),
    };
    println!("run {}: {}, {status}", run.id, run.mode);
    if !run.command.is_empty() {
        println!("  command:  meridian {}", run.command);
    }
    let by = match (run.user.as_str(), run.host.as_str()) {
        ("", "") => String::new(),
        (user, host) => format!(
            " by {}@{}",
            if user.is_empty() { "?" } else { user },
            if host.is_empty() { "?" } else { host }
        ),
    };
    println!("  started:  {}{by}", run.started);
    if !run.finished.is_empty() {
        println!("  finished: {}", run.finished);
    }
    if !run.resumes.is_empty() {
        println!("  resumes:  {}", run.resumes);
    }
    for step in &run.steps {
        let on = if step.endpoint.is_empty() {
            String::new()
        } else {
            format!(" on {}", step.endpoint)
        };
        let mut line = format!(
            "  {} ({}, target {}{on}): {}",
            step.id.name,
            step.id.kind.as_str(),
            step.target,
            step.status.as_str()
        );
        if !step.reason.is_empty() {
            line.push_str(&format!(" ({})", step.reason));
        }
        if step.changes > 0 {
            line.push_str(&format!(", {}", count(step.changes as usize, "change")));
        }
        println!("{line}");
    }
}

/// Lists every recorded run, oldest first.
fn runs(state: &State) -> Result<(), Failure> {
    let backend = backend(state)?;
    let backend = backend.as_ref();
    let ids = record::list_runs(backend).map_err(Failure::Message)?;
    if ids.is_empty() {
        println!("no run has been recorded in {}", backend.describe());
        return Ok(());
    }
    for id in ids {
        let Some(run) = record::load_run(backend, &id).map_err(Failure::Message)? else {
            continue;
        };
        let failed = run
            .steps
            .iter()
            .filter(|step| step.status == record::StepStatus::Failed)
            .count();
        let mut line = format!(
            "{}  {:<8} {:<10} {}",
            run.id,
            run.mode,
            run.status.as_str(),
            count(run.steps.len(), "step")
        );
        if failed > 0 {
            line.push_str(&format!(", {failed} failed"));
        }
        if !run.resumes.is_empty() {
            line.push_str(&format!(", resumes {}", run.resumes));
        }
        println!("{line}");
    }
    Ok(())
}

/// Prints a run's events as they were recorded.
fn events(state: &State, run: Option<&str>, types: &[String]) -> Result<(), Failure> {
    let backend = backend(state)?;
    let backend = backend.as_ref();
    let id = match run {
        Some(id) => {
            if record::load_run(backend, id)
                .map_err(Failure::Message)?
                .is_none()
            {
                return Err(Failure::Message(format!(
                    "there is no run {id} in {}",
                    backend.describe()
                )));
            }
            id.to_owned()
        }
        None => match record::last_run(backend, None).map_err(Failure::Message)? {
            Some(run) => run.id,
            None => {
                return Err(Failure::Message(format!(
                    "no run has been recorded in {}",
                    backend.describe()
                )))
            }
        },
    };
    let mut out = std::io::stdout().lock();
    for event in record::load_events(backend, &id).map_err(Failure::Message)? {
        if types.is_empty() || types.iter().any(|pattern| event.matches(pattern)) {
            let _ = writeln!(out, "{}", event.to_json());
        }
    }
    Ok(())
}

/// Removes a lock a run left behind.
fn unlock(state: &State, id: &str) -> Result<(), Failure> {
    let backend = backend(state)?;
    let removed = lock::force_unlock(backend.as_ref(), id).map_err(Failure::Message)?;
    println!("removed {removed}");
    Ok(())
}

/// Prints a run as it happens.
struct Printer<'a> {
    mode: Mode,
    out: Option<&'a Path>,
    /// Whether standard output carries the events, which moves the report
    /// for people to standard error.
    events: bool,
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
        if self.events {
            let _ = self.print(&mut std::io::stderr().lock(), report);
        } else {
            let _ = self.print(&mut std::io::stdout().lock(), report);
        }
    }

    fn event(&mut self, event: &Event) {
        if self.events {
            let _ = writeln!(std::io::stdout().lock(), "{}", event.to_json());
        }
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
            Outcome::Skipped { applied_in } => writeln!(
                out,
                "  already applied by run {applied_in}, and unchanged since; left alone"
            )?,
            Outcome::Failed(why) => {
                // What the tool had already done is part of what happened.
                for change in &report.done_before_failing {
                    writeln!(out, "  {}", change_line(change))?;
                }
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
