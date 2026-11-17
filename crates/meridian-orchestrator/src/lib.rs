//! Runs a workflow.
//!
//! A workflow is a set of documents. Each document is a stage: it is compiled
//! for its target, and then, depending on what was asked, previewed or
//! applied by that target's plugin. Stages run in an order that lets a later
//! one use what an earlier one produced: every Infrastructure document first,
//! then every ResourceSet, each group in the order it was written.
//!
//! Before a document reaches its plugin the orchestrator settles everything
//! that is the core's to settle: whether the document's labels select it for
//! this run, whether its kind is one the target's category takes, whether the
//! target compiles for the version the document asks for, and which resources
//! their `when` conditions leave in.
//!
//! The orchestrator knows nothing about any tool. It routes on what a plugin
//! declares, hands it an artifact and a directory, and reads back changes and
//! outputs. Everything tool-shaped happens behind the plugin boundary.
//!
//! A run stops at the first stage that fails. Stages after it are reported as
//! not run, never attempted against infrastructure in an unknown state.

mod select;
mod state;

use std::collections::BTreeMap;
use std::fmt;
use std::path::{Path, PathBuf};

use meridian_ast::ResourceGraph;
use meridian_ir::{Bundle, Document, Infrastructure, ResourceSet, TARGET_HOST_FROM_INFRASTRUCTURE};
use meridian_plugin_host::{to_sdk, EmitError, Registry, RunFailure};
use meridian_sdk::{
    Artifact, Category, Change, Data, LogLine, Output, Plan, RunContext, Value, Warning,
};

pub use select::Selector;
pub use state::{artifact_hash, RunState, StageRecord};

/// How far a run goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// Compile every document and stop. Nothing is contacted.
    Compile,
    /// Compile, then ask each tool what applying would change.
    Plan,
    /// Compile, then make it so.
    Apply,
}

/// Which kind of document a stage came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum StageKind {
    Infrastructure,
    ResourceSet,
}

impl StageKind {
    pub fn as_str(self) -> &'static str {
        match self {
            StageKind::Infrastructure => "Infrastructure",
            StageKind::ResourceSet => "ResourceSet",
        }
    }
}

/// Names a stage: the kind and name of the document it came from.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct StageId {
    pub kind: StageKind,
    pub name: String,
}

impl fmt::Display for StageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}", self.kind.as_str(), self.name)
    }
}

/// How a stage ended.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// The document compiled. This is as far as [`Mode::Compile`] goes, and
    /// as far as a plan can go with a target that only compiles.
    Compiled,
    /// The tool previewed the change.
    Planned(Plan),
    /// The tool applied the change.
    Applied {
        changes: Vec<Change>,
        outputs: Vec<Output>,
    },
    /// The stage needs a value that does not exist yet: an output of an
    /// Infrastructure document that has not been applied. It is not a
    /// failure. A plan simply cannot see past infrastructure that is not
    /// there.
    Deferred { waiting_on: StageId, output: String },
    /// The stage failed, and the run stopped here.
    Failed(String),
    /// An earlier stage failed, so this one was not attempted.
    NotRun { because_of: StageId },
}

/// What one stage did.
#[derive(Clone, Debug, PartialEq)]
pub struct StageReport {
    pub id: StageId,
    /// The target the document selected.
    pub target: String,
    /// The host the document configures; empty for an Infrastructure
    /// document, or when the host is not known yet.
    pub host: String,
    /// What the document compiled to, when it got that far.
    pub artifact: Option<Artifact>,
    pub warnings: Vec<Warning>,
    pub outcome: Outcome,
}

/// What a whole run did, stage by stage, in the order they ran.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RunReport {
    pub stages: Vec<StageReport>,
}

impl RunReport {
    /// Whether every stage did what was asked of it. A deferred stage counts:
    /// nothing went wrong.
    pub fn succeeded(&self) -> bool {
        !self
            .stages
            .iter()
            .any(|s| matches!(s.outcome, Outcome::Failed(_) | Outcome::NotRun { .. }))
    }
}

/// Watches a run as it happens.
pub trait Observer {
    /// A stage is about to compile.
    fn stage_started(&mut self, _id: &StageId, _target: &str) {}
    /// The tool printed a line. Sensitive values are already masked.
    fn log(&mut self, _id: &StageId, _line: &LogLine) {}
    /// A stage ended.
    fn stage_finished(&mut self, _report: &StageReport) {}
}

/// An observer that ignores everything.
pub struct Quiet;

impl Observer for Quiet {}

/// Where a run keeps things, and what it is given.
#[derive(Clone, Debug)]
pub struct Options {
    /// The directory holding each stage's work directory and the run-state
    /// file. It is created if it does not exist.
    pub state_dir: PathBuf,
    /// The data `when` conditions are decided against, also handed to every
    /// target when it compiles.
    pub data: Data,
}

/// The text shown in place of a sensitive value.
pub const MASK: &str = "(sensitive)";

/// Runs every document in `bundle` through its target.
///
/// The bundle must already be valid: [`Bundle::validate`] is the caller's
/// job, because its errors are about the documents and belong in front of
/// the author before anything starts.
pub fn run(
    bundle: &Bundle,
    registry: &Registry,
    mode: Mode,
    options: &Options,
    observer: &mut dyn Observer,
) -> RunReport {
    run_selected(bundle, registry, mode, options, &Selector::all(), observer)
}

/// [`run`], for only the documents whose labels `selector` matches.
///
/// A document the selector leaves out is not a stage, but it stays loaded: a
/// selected ResourceSet can still name it as the source of its target host,
/// and finds that host from what the document's last apply left behind.
pub fn run_selected(
    bundle: &Bundle,
    registry: &Registry,
    mode: Mode,
    options: &Options,
    selector: &Selector,
    observer: &mut dyn Observer,
) -> RunReport {
    let mut run = Run {
        bundle,
        registry,
        mode,
        options,
        outputs: BTreeMap::new(),
        secrets: Vec::new(),
        state: None,
    };
    let mut report = RunReport::default();
    let mut failed: Option<StageId> = None;

    for stage in stages(bundle, selector) {
        let id = stage.id();
        let target = stage.target().to_owned();
        if let Some(because_of) = &failed {
            let skipped = StageReport {
                id,
                target,
                host: String::new(),
                artifact: None,
                warnings: Vec::new(),
                outcome: Outcome::NotRun {
                    because_of: because_of.clone(),
                },
            };
            observer.stage_finished(&skipped);
            report.stages.push(skipped);
            continue;
        }
        observer.stage_started(&id, &target);
        let stage_report = run.stage(stage, observer);
        if matches!(stage_report.outcome, Outcome::Failed(_)) {
            failed = Some(stage_report.id.clone());
        }
        observer.stage_finished(&stage_report);
        report.stages.push(stage_report);
    }
    report
}

/// One document, as a stage.
#[derive(Clone, Copy)]
enum Stage<'a> {
    Infrastructure(&'a Infrastructure),
    ResourceSet(&'a ResourceSet),
}

impl Stage<'_> {
    fn id(&self) -> StageId {
        match self {
            Stage::Infrastructure(d) => StageId {
                kind: StageKind::Infrastructure,
                name: d.metadata.name.clone(),
            },
            Stage::ResourceSet(d) => StageId {
                kind: StageKind::ResourceSet,
                name: d.metadata.name.clone(),
            },
        }
    }

    fn target(&self) -> &str {
        match self {
            Stage::Infrastructure(d) => &d.spec.target,
            Stage::ResourceSet(d) => &d.spec.target,
        }
    }

    fn target_version(&self) -> &str {
        match self {
            Stage::Infrastructure(d) => &d.spec.target_version,
            Stage::ResourceSet(d) => &d.spec.target_version,
        }
    }
}

/// How many documents of `bundle` a selector picks. A run that picks none
/// does nothing, which a caller will usually want to say out loud.
pub fn selected(bundle: &Bundle, selector: &Selector) -> usize {
    stages(bundle, selector).len()
}

/// Whether a document of this kind can be aimed at a target of this
/// category, and if not, why.
///
/// The two kinds answer different questions. An Infrastructure document says
/// what should exist; a ResourceSet says how a machine that exists is
/// configured. A tool that provisions has no machine to be pointed at, and a
/// tool that configures one machine has nothing to provision, so the wrong
/// pairing is refused before the plugin sees it instead of compiling into
/// something that quietly means less than the document says.
///
/// Containerization takes both: a container project either stands on its
/// own (Infrastructure) or runs on the daemon of a host the document names
/// (ResourceSet). A plugin that does not state a category was built before
/// categories existed, and is not second-guessed.
pub fn kind_fits(kind: StageKind, category: Category) -> Result<(), String> {
    let wants = match (kind, category) {
        (StageKind::ResourceSet, Category::InfrastructureAsCode)
        | (StageKind::ResourceSet, Category::ContainerOrchestration) => StageKind::Infrastructure,
        (StageKind::Infrastructure, Category::ConfigurationManagement) => StageKind::ResourceSet,
        _ => return Ok(()),
    };
    let why = match wants {
        StageKind::Infrastructure => {
            "it creates what should exist and is not pointed at one machine, so it takes \
             kind: Infrastructure, without a targetHost"
        }
        StageKind::ResourceSet => {
            "it configures a machine that already exists, so it takes kind: ResourceSet, \
             with a targetHost"
        }
    };
    let (document, tool) = match kind {
        StageKind::ResourceSet => ("a ResourceSet", "an infrastructure-as-code"),
        StageKind::Infrastructure => ("an Infrastructure", "a configuration-management"),
    };
    let tool = match category {
        Category::ContainerOrchestration => "a container-orchestration",
        _ => tool,
    };
    Err(format!(
        "this is {document} document, and its target is {tool} tool: {why}"
    ))
}

/// Whether `asked`, a document's `targetVersion`, is one of the versions a
/// target declares or a more specific version under one of them: `3.1` is
/// under `3`, and `1.10` is not under `1.1`.
pub fn version_fits(asked: &str, declared: &[String]) -> bool {
    declared.iter().any(|have| {
        asked == have
            || asked
                .strip_prefix(have.as_str())
                .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// Every Infrastructure document, then every ResourceSet, each in the order
/// written. A ResourceSet can only depend on an Infrastructure document, so
/// this order satisfies every dependency a workflow can express.
fn stages<'a>(bundle: &'a Bundle, selector: &Selector) -> Vec<Stage<'a>> {
    let selected = |doc: &&Document| selector.matches(doc.labels());
    let infrastructure = bundle
        .documents
        .iter()
        .filter(selected)
        .filter_map(Document::as_infrastructure)
        .map(Stage::Infrastructure);
    let resource_sets = bundle
        .documents
        .iter()
        .filter(selected)
        .filter_map(Document::as_resource_set)
        .map(Stage::ResourceSet);
    infrastructure.chain(resource_sets).collect()
}

struct Run<'a> {
    bundle: &'a Bundle,
    registry: &'a Registry,
    mode: Mode,
    options: &'a Options,
    /// The outputs of each Infrastructure document, once known.
    outputs: BTreeMap<String, Vec<Output>>,
    /// Sensitive values seen so far, masked out of everything shown.
    secrets: Vec<String>,
    state: Option<RunState>,
}

/// Why a stage stopped before reaching the tool.
enum Halt {
    Failed(String),
    Deferred { waiting_on: StageId, output: String },
}

impl Run<'_> {
    fn stage(&mut self, stage: Stage<'_>, observer: &mut dyn Observer) -> StageReport {
        let id = stage.id();
        let mut report = StageReport {
            id: id.clone(),
            target: stage.target().to_owned(),
            host: String::new(),
            artifact: None,
            warnings: Vec::new(),
            outcome: Outcome::Compiled,
        };
        let fail = |mut report: StageReport, why: String| {
            report.outcome = Outcome::Failed(why);
            report
        };

        let Some(target) = self.registry.target(stage.target()) else {
            let known = self.registry.names().join(", ");
            let known = if known.is_empty() {
                "no plugins are loaded".to_owned()
            } else {
                format!("loaded targets: {known}")
            };
            return fail(
                report,
                format!("no plugin provides target {:?} ({known})", stage.target()),
            );
        };

        let info = target.info();
        if let Err(why) = kind_fits(id.kind, info.category) {
            return fail(report, why);
        }
        let asked = stage.target_version();
        if !asked.is_empty() {
            if info.target_versions.is_empty() {
                return fail(
                    report,
                    format!(
                        "the document asks for targetVersion {asked:?}, but target {:?} does \
                         not say which versions of its tool it compiles for, so the request \
                         cannot be honoured; remove targetVersion",
                        stage.target()
                    ),
                );
            }
            if !version_fits(asked, &info.target_versions) {
                return fail(
                    report,
                    format!(
                        "the document asks for targetVersion {asked:?}, and target {:?} \
                         compiles for: {}",
                        stage.target(),
                        info.target_versions.join(", ")
                    ),
                );
            }
        }

        // Decide every `when`, so the tree holds only what will be applied.
        let pruned = |resources: &[meridian_ir::Resource]| {
            meridian_ir::prune(resources, &self.options.data).map_err(|e| e.to_string())
        };

        // Resolve what the stage needs from earlier ones, then build its tree.
        let (graph, inputs) = match stage {
            Stage::Infrastructure(doc) => {
                let mut doc = doc.clone();
                doc.spec.resources = match pruned(&doc.spec.resources) {
                    Ok(resources) => resources,
                    Err(why) => return fail(report, why),
                };
                match ResourceGraph::build_infrastructure(&doc) {
                    Ok(graph) => (graph, Data::new()),
                    Err(e) => return fail(report, e.to_string()),
                }
            }
            Stage::ResourceSet(doc) => {
                let mut doc = doc.clone();
                doc.spec.resources = match pruned(&doc.spec.resources) {
                    Ok(resources) => resources,
                    Err(why) => return fail(report, why),
                };
                let doc = &doc;
                let (host, inputs) = match self.host_of(doc) {
                    Ok(found) => found,
                    Err(Halt::Failed(why)) => return fail(report, why),
                    Err(Halt::Deferred { waiting_on, output }) => {
                        report.outcome = Outcome::Deferred { waiting_on, output };
                        return report;
                    }
                };
                report.host = host.clone();
                match ResourceGraph::build(doc, &host) {
                    Ok(graph) => (graph, inputs),
                    Err(e) => return fail(report, e.to_string()),
                }
            }
        };

        let emitted = match target.emit(&to_sdk(&graph), &self.options.data) {
            Ok(emitted) => emitted,
            Err(EmitError::Refused(e)) => return fail(report, e.to_string()),
            Err(EmitError::Plugin(e)) => return fail(report, e.to_string()),
        };
        report.warnings = emitted.warnings;
        report.artifact = Some(emitted.artifact.clone());
        if self.mode == Mode::Compile {
            return report;
        }

        let operations = target.info().operations;
        if self.mode == Mode::Plan && !operations.plan {
            // Compiling is still a result worth having; the target just
            // cannot say what applying it would change.
            return report;
        }
        if self.mode == Mode::Apply && !operations.apply {
            return fail(
                report,
                format!(
                    "target {:?} only compiles, so Meridian cannot apply this document; \
                     use `meridian compile` and apply the result with the tool itself",
                    stage.target()
                ),
            );
        }

        let context = match self.context(&id, stage.target(), &report.host, inputs) {
            Ok(context) => context,
            Err(why) => return fail(report, why),
        };
        let secrets = self.secrets.clone();
        let mut log = |mut line: LogLine| {
            line.text = mask(&line.text, &secrets);
            observer.log(&id, &line);
        };

        match self.mode {
            Mode::Plan => match target.plan(&context, &emitted.artifact, &mut log) {
                Ok(plan) => report.outcome = Outcome::Planned(plan),
                Err(e) => return fail(report, self.describe(e)),
            },
            Mode::Apply => {
                let outcome = target.apply(&context, &emitted.artifact, &mut log);
                let hash = artifact_hash(&emitted.artifact);
                match outcome {
                    Ok(applied) => {
                        self.record(&id, stage.target(), &hash, true);
                        self.learn(&id, &applied.outputs);
                        report.outcome = Outcome::Applied {
                            changes: applied.changes,
                            outputs: applied.outputs,
                        };
                    }
                    Err(e) => {
                        self.record(&id, stage.target(), &hash, false);
                        return fail(report, self.describe(e));
                    }
                }
            }
            Mode::Compile => {}
        }
        report
    }

    /// The host a ResourceSet configures, and the values it receives from the
    /// Infrastructure document it names.
    fn host_of(&mut self, doc: &ResourceSet) -> Result<(String, Data), Halt> {
        let Some(target_host) = &doc.spec.target_host else {
            return Err(Halt::Failed(format!(
                "document {:?} declares no targetHost",
                doc.metadata.name
            )));
        };
        if target_host.from != TARGET_HOST_FROM_INFRASTRUCTURE {
            return Ok((target_host.host.clone(), Data::new()));
        }

        let source = StageId {
            kind: StageKind::Infrastructure,
            name: target_host.r#ref.clone(),
        };
        let wanted = target_host.output.clone();
        if !self.outputs.contains_key(&source.name) {
            self.read_outputs(&source);
        }
        let Some(outputs) = self.outputs.get(&source.name) else {
            return Err(Halt::Deferred {
                waiting_on: source,
                output: wanted,
            });
        };
        let Some(output) = outputs.iter().find(|o| o.name == wanted) else {
            let have: Vec<&str> = outputs.iter().map(|o| o.name.as_str()).collect();
            return Err(Halt::Failed(format!(
                "{source} did not produce the output {wanted:?} that this document's \
                 targetHost needs (it produced: {})",
                if have.is_empty() {
                    "nothing".to_owned()
                } else {
                    have.join(", ")
                }
            )));
        };
        let Value::String(host) = &output.value else {
            return Err(Halt::Failed(format!(
                "output {wanted:?} of {source} is {}, and a target host has to be a string",
                output.value.describe()
            )));
        };
        let inputs = outputs
            .iter()
            .map(|o| (o.name.clone(), o.value.clone()))
            .collect();
        Ok((host.clone(), inputs))
    }

    /// Asks an Infrastructure document's tool what its last apply produced.
    /// This is how a plan, or an apply of a later run, sees infrastructure
    /// that already exists. Anything short of an answer leaves the outputs
    /// unknown, which defers whoever needs them.
    fn read_outputs(&mut self, source: &StageId) {
        let Some(doc) = self.bundle.infrastructure(&source.name) else {
            return;
        };
        let Some(target) = self.registry.target(&doc.spec.target) else {
            return;
        };
        if !target.info().operations.outputs {
            return;
        }
        let Ok(context) = self.context(source, &doc.spec.target, "", Data::new()) else {
            return;
        };
        if let Ok(outputs) = target.outputs(&context) {
            self.learn(source, &outputs);
        }
    }

    fn learn(&mut self, id: &StageId, outputs: &[Output]) {
        for output in outputs.iter().filter(|o| o.sensitive) {
            // Only strings are masked: a sensitive number or flag is too
            // short to search for without mangling unrelated text.
            if let Value::String(s) = &output.value {
                if s.len() >= 4 && !self.secrets.contains(s) {
                    self.secrets.push(s.clone());
                }
            }
        }
        if id.kind == StageKind::Infrastructure {
            self.outputs.insert(id.name.clone(), outputs.to_vec());
        }
    }

    fn context(
        &self,
        id: &StageId,
        target: &str,
        host: &str,
        inputs: Data,
    ) -> Result<RunContext, String> {
        let work_dir = work_dir(&self.options.state_dir, id, target);
        std::fs::create_dir_all(&work_dir)
            .map_err(|e| format!("creating the work directory {}: {e}", work_dir.display()))?;
        // A plugin is a separate process with its own working directory, so
        // it is always given an absolute path.
        let work_dir = work_dir
            .canonicalize()
            .map_err(|e| format!("resolving the work directory {}: {e}", work_dir.display()))?;
        Ok(RunContext {
            document: id.name.clone(),
            host: host.to_owned(),
            work_dir,
            inputs,
        })
    }

    fn record(&mut self, id: &StageId, target: &str, hash: &str, applied: bool) {
        let path = self.options.state_dir.join("state.json");
        let state = self.state.get_or_insert_with(|| RunState::load(&path));
        state.set(
            id,
            StageRecord {
                target: target.to_owned(),
                artifact_hash: hash.to_owned(),
                applied,
            },
        );
        // The run-state is a convenience for the operator, not what the next
        // run decides on, so failing to write it does not fail the stage.
        let _ = state.save(&path);
    }

    fn describe(&self, failure: RunFailure) -> String {
        mask(&failure.to_string(), &self.secrets)
    }
}

/// Where a stage's tool keeps its state: one directory per document and
/// target, so changing a document's target never points a new tool at the
/// old one's files.
pub fn work_dir(state_dir: &Path, id: &StageId, target: &str) -> PathBuf {
    let kind = match id.kind {
        StageKind::Infrastructure => "infrastructure",
        StageKind::ResourceSet => "resourceset",
    };
    state_dir.join(kind).join(&id.name).join(target)
}

/// Replaces every sensitive value in `text`.
pub fn mask(text: &str, secrets: &[String]) -> String {
    let mut out = text.to_owned();
    for secret in secrets {
        if out.contains(secret.as_str()) {
            out = out.replace(secret.as_str(), MASK);
        }
    }
    out
}
