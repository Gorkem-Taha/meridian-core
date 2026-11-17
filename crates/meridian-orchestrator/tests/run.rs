//! A whole workflow, run through real plugin processes.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use meridian_ir::Bundle;
use meridian_orchestrator::{
    artifact_hash, run, work_dir, Mode, Observer, Options, Outcome, Quiet, RunState, StageId,
    StageKind, StageReport, MASK,
};
use meridian_plugin_host::{Registry, StartOptions};
use meridian_sdk::{ChangeAction, Data, LogLine, Value};

/// An Infrastructure document that "provisions" a machine by writing its
/// address into a file, and a ResourceSet configured on whatever address that
/// turns out to be.
const WORKFLOW: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: web
spec:
  target: fake
  targetHost:
    from: infrastructure
    ref: network
    output: address
  resources:
    - id: motd
      type: file
      params:
        content: "welcome\n"
---
apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: network
spec:
  target: fake
  resources:
    - id: vm
      type: file
      params:
        content: 10.0.0.5
    - id: key
      type: file
      params:
        content: hunter2-very-secret
  outputs:
    - name: address
      value: "${vm.content}"
    - name: secret_key
      value: "${key.content}"
"#;

const INLINE: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: web
spec:
  target: TARGET
  targetHost:
    from: inline
    host: web01.example.com
  resources:
    - id: motd
      type: file
      params:
        content: "CONTENT"
"#;

fn bundle(text: &str) -> Bundle {
    let bundle = Bundle::new(meridian_ir::parse("workflow.yaml", text.as_bytes()).unwrap());
    bundle.validate().unwrap();
    bundle
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    let dir = meridian_target_fake::plugin_dir();
    let found = registry
        .discover(dir.to_str().unwrap(), &StartOptions::default())
        .expect("the fake plugin loads");
    assert_eq!(found.len(), 1);
    registry
}

fn options(test: &str) -> Options {
    let state_dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = fs::remove_dir_all(&state_dir);
    Options {
        state_dir,
        data: Data::new(),
    }
}

fn id(kind: StageKind, name: &str) -> StageId {
    StageId {
        kind,
        name: name.to_owned(),
    }
}

/// Records what an observer is told, in order.
#[derive(Default)]
struct Recorder {
    events: Vec<String>,
}

impl Observer for Recorder {
    fn stage_started(&mut self, id: &StageId, target: &str) {
        self.events.push(format!("start {id} with {target}"));
    }
    fn log(&mut self, id: &StageId, line: &LogLine) {
        self.events.push(format!("log {}: {}", id.name, line.text));
    }
    fn stage_finished(&mut self, report: &StageReport) {
        self.events.push(format!("finish {}", report.id));
    }
}

fn file(dir: &Path, relative: &str) -> PathBuf {
    dir.join("current").join(relative)
}

#[test]
fn compile_touches_nothing_and_defers_what_depends_on_infrastructure() {
    let registry = registry();
    let options = options("compile");
    let report = run(
        &bundle(WORKFLOW),
        &registry,
        Mode::Compile,
        &options,
        &mut Quiet,
    );

    // Infrastructure runs first, whatever order the documents were written in.
    assert_eq!(
        report.stages[0].id,
        id(StageKind::Infrastructure, "network")
    );
    assert_eq!(report.stages[0].outcome, Outcome::Compiled);
    let artifact = report.stages[0].artifact.as_ref().unwrap();
    assert_eq!(artifact.files["files/vm"], "10.0.0.5");
    assert_eq!(artifact.files["outputs/address"], "${vm.content}");

    // The address does not exist until the infrastructure does.
    assert_eq!(
        report.stages[1].outcome,
        Outcome::Deferred {
            waiting_on: id(StageKind::Infrastructure, "network"),
            output: "address".to_owned(),
        }
    );
    assert!(report.succeeded(), "a deferred stage is not a failure");
    assert!(
        !file(
            &work_dir(&options.state_dir, &report.stages[0].id, "fake"),
            "files/vm"
        )
        .exists(),
        "compiling applied something"
    );
}

#[test]
fn apply_hands_an_output_to_the_next_stage_and_masks_the_secret_one() {
    let registry = registry();
    let options = options("apply");
    let bundle = bundle(WORKFLOW);
    let mut recorder = Recorder::default();
    let report = run(&bundle, &registry, Mode::Apply, &options, &mut recorder);
    assert!(report.succeeded(), "{report:?}");

    let network = &report.stages[0];
    let Outcome::Applied { changes, outputs } = &network.outcome else {
        panic!("{:?}", network.outcome);
    };
    assert_eq!(changes.len(), 4);
    let address = outputs.iter().find(|o| o.name == "address").unwrap();
    assert_eq!(address.value, Value::from("10.0.0.5"));
    assert!(
        outputs
            .iter()
            .find(|o| o.name == "secret_key")
            .unwrap()
            .sensitive
    );

    // The ResourceSet was compiled for, and applied to, the address the
    // infrastructure stage produced.
    let web = &report.stages[1];
    assert_eq!(web.id, id(StageKind::ResourceSet, "web"));
    assert_eq!(web.host, "10.0.0.5");
    assert!(matches!(&web.outcome, Outcome::Applied { changes, .. } if changes.len() == 1));
    let web_dir = work_dir(&options.state_dir, &web.id, "fake");
    assert_eq!(
        fs::read_to_string(file(&web_dir, "files/motd")).unwrap(),
        "welcome\n"
    );

    // Each stage has its own directory, so one tool never sees another's state.
    let network_dir = work_dir(&options.state_dir, &network.id, "fake");
    assert_ne!(web_dir, network_dir);
    assert!(file(&network_dir, "files/vm").exists());

    // The second stage's tool echoed its inputs, the secret among them. What
    // reached the observer has it masked, and the plain address intact.
    let echoed = recorder
        .events
        .iter()
        .find(|e| e.starts_with("log web: inputs:"))
        .expect("the tool echoed its inputs");
    assert!(echoed.contains("\"address\":\"10.0.0.5\""), "{echoed}");
    assert!(echoed.contains(MASK), "{echoed}");
    assert!(
        !recorder.events.iter().any(|e| e.contains("hunter2")),
        "a secret reached the observer: {:?}",
        recorder.events
    );
    assert_eq!(
        recorder.events.first().map(String::as_str),
        Some("start Infrastructure \"network\" with fake")
    );
    assert_eq!(
        recorder.events.last().map(String::as_str),
        Some("finish ResourceSet \"web\"")
    );

    // The run-state records what was applied, and holds no outputs at all.
    let state_file = options.state_dir.join("state.json");
    let state = RunState::load(&state_file);
    let record = state.get(&web.id).expect("the stage was recorded");
    assert!(record.applied);
    assert_eq!(record.target, "fake");
    assert_eq!(
        record.artifact_hash,
        artifact_hash(web.artifact.as_ref().unwrap())
    );
    assert!(!fs::read_to_string(&state_file).unwrap().contains("hunter2"));
}

#[test]
fn a_plan_after_an_apply_sees_the_existing_infrastructure() {
    let registry = registry();
    let options = options("plan-after-apply");
    let bundle = bundle(WORKFLOW);

    // Before anything exists: the infrastructure is all to be created, and
    // the stage that needs its address cannot be planned yet.
    let before = run(&bundle, &registry, Mode::Plan, &options, &mut Quiet);
    let Outcome::Planned(plan) = &before.stages[0].outcome else {
        panic!("{:?}", before.stages[0].outcome);
    };
    assert_eq!(plan.changes.len(), 4);
    assert!(plan
        .changes
        .iter()
        .all(|c| c.action == ChangeAction::Create));
    assert!(matches!(before.stages[1].outcome, Outcome::Deferred { .. }));

    assert!(run(&bundle, &registry, Mode::Apply, &options, &mut Quiet).succeeded());

    // A fresh run has no outputs in memory. It reads them back from the tool
    // that owns them, so the second stage can now be planned, and both are
    // in sync.
    let after = run(&bundle, &registry, Mode::Plan, &options, &mut Quiet);
    for stage in &after.stages {
        let Outcome::Planned(plan) = &stage.outcome else {
            panic!("{}: {:?}", stage.id, stage.outcome);
        };
        assert!(plan.in_sync(), "{}: {:?}", stage.id, plan.changes);
    }
    assert_eq!(after.stages[1].host, "10.0.0.5");
}

#[test]
fn a_failed_stage_stops_the_run_and_later_stages_are_not_attempted() {
    let registry = registry();
    let options = options("failure");
    let broken = WORKFLOW.replace("content: 10.0.0.5", "content: FAIL 10.0.0.5");
    let report = run(
        &bundle(&broken),
        &registry,
        Mode::Apply,
        &options,
        &mut Quiet,
    );

    assert!(!report.succeeded());
    let Outcome::Failed(why) = &report.stages[0].outcome else {
        panic!("{:?}", report.stages[0].outcome);
    };
    assert!(why.starts_with("could not write files/vm"), "{why}");
    assert_eq!(
        report.stages[1].outcome,
        Outcome::NotRun {
            because_of: id(StageKind::Infrastructure, "network"),
        }
    );
    assert!(report.stages[1].artifact.is_none());

    let state = RunState::load(&options.state_dir.join("state.json"));
    assert!(!state.get(&report.stages[0].id).unwrap().applied);
    assert!(state.get(&report.stages[1].id).is_none());
}

#[test]
fn a_document_for_a_target_with_no_plugin_fails_and_names_what_is_loaded() {
    let registry = registry();
    let text = INLINE
        .replace("TARGET", "terraform")
        .replace("CONTENT", "x");
    let report = run(
        &bundle(&text),
        &registry,
        Mode::Plan,
        &options("no-plugin"),
        &mut Quiet,
    );
    assert_eq!(
        report.stages[0].outcome,
        Outcome::Failed(
            "no plugin provides target \"terraform\" (loaded targets: fake)".to_owned()
        )
    );
}

#[test]
fn a_refusal_to_compile_fails_the_stage_at_the_authors_line() {
    let registry = registry();
    let text = INLINE
        .replace("TARGET", "fake")
        .replace("CONTENT", "x")
        .replace("type: file", "type: firewall_rule");
    let report = run(
        &bundle(&text),
        &registry,
        Mode::Apply,
        &options("refusal"),
        &mut Quiet,
    );
    let Outcome::Failed(why) = &report.stages[0].outcome else {
        panic!("{:?}", report.stages[0].outcome);
    };
    assert_eq!(
        why,
        "workflow.yaml:11:7: fake: resource \"motd\" [unsupported-resource-type]: \
         the fake target has no mapping for type \"firewall_rule\""
    );
}

#[test]
fn a_target_that_only_compiles_can_be_planned_but_not_applied() {
    let mut env: Vec<(String, String)> = std::env::vars().collect();
    env.push(("FAKE_OPERATIONS".to_owned(), "none".to_owned()));
    let mut registry = Registry::new();
    registry
        .load(
            meridian_target_fake::plugin_path().to_str().unwrap(),
            &StartOptions {
                env: Some(env),
                ..StartOptions::default()
            },
        )
        .unwrap();
    let bundle = bundle(&INLINE.replace("TARGET", "fake").replace("CONTENT", "x"));
    let options = options("compile-only");

    let planned = run(&bundle, &registry, Mode::Plan, &options, &mut Quiet);
    assert_eq!(planned.stages[0].outcome, Outcome::Compiled);
    assert!(planned.stages[0].artifact.is_some());

    let applied = run(&bundle, &registry, Mode::Apply, &options, &mut Quiet);
    let Outcome::Failed(why) = &applied.stages[0].outcome else {
        panic!("{:?}", applied.stages[0].outcome);
    };
    assert!(why.starts_with("target \"fake\" only compiles"), "{why}");
}

#[test]
fn an_artifact_hash_depends_on_content_and_not_on_how_files_are_split() {
    let artifact = |files: &[(&str, &str)]| meridian_sdk::Artifact {
        files: files
            .iter()
            .map(|(p, c)| ((*p).to_owned(), (*c).to_owned()))
            .collect(),
    };
    let a = artifact_hash(&artifact(&[("a", "bc"), ("d", "")]));
    assert_eq!(a, artifact_hash(&artifact(&[("d", ""), ("a", "bc")])));
    assert_ne!(a, artifact_hash(&artifact(&[("ab", "c"), ("d", "")])));
    assert_ne!(a, artifact_hash(&artifact(&[("a", "bcd")])));
    assert!(a.starts_with("sha256:") && a.len() == 71);
}
