//! Putting back what an apply changed, and telling drift from change, run
//! through real plugin processes.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};

use meridian_ir::Bundle;
use meridian_orchestrator::record::{self, StepStatus};
use meridian_orchestrator::{
    artifact_hash, event, history, lock, run, work_dir, Drift, Event, Mode, Observer, OnFailure,
    Options, Outcome, Quiet, Refusal, Resume, RollbackOutcome, RollbackReport, RunReport, RunState,
    StageId, StageKind,
};
use meridian_plugin_host::{Registry, StartOptions};
use meridian_sdk::{ChangeAction, LogLine};

/// A machine with a key, and a ResourceSet configured on its address.
const WORKFLOW: &str = r#"apiVersion: meridian/v1
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
        content: KEY
  outputs:
    - name: address
      value: "${vm.content}"
---
apiVersion: meridian/v1
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
        content: "MOTD"
"#;

fn workflow(key: &str, motd: &str) -> Bundle {
    bundle(&WORKFLOW.replace("KEY", key).replace("MOTD", motd))
}

fn bundle(text: &str) -> Bundle {
    let bundle = Bundle::new(meridian_ir::parse("workflow.yaml", text.as_bytes()).unwrap());
    bundle.validate().unwrap();
    bundle
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    let dir = meridian_target_fake::plugin_dir();
    registry
        .discover(dir.to_str().unwrap(), &StartOptions::default())
        .expect("the fake plugin loads");
    registry
}

fn options(test: &str) -> Options {
    let dir: PathBuf = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("undo")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    Options::new(dir)
}

fn network() -> StageId {
    StageId {
        kind: StageKind::Infrastructure,
        name: "network".to_owned(),
    }
}

fn web() -> StageId {
    StageId {
        kind: StageKind::ResourceSet,
        name: "web".to_owned(),
    }
}

/// What the fake tool has on disk for a stage's file.
fn on_disk(options: &Options, id: &StageId, file: &str) -> String {
    fs::read_to_string(
        work_dir(&options.state_dir, id, "fake")
            .join("current/files")
            .join(file),
    )
    .unwrap()
}

#[derive(Default)]
struct Recorder {
    events: Vec<Event>,
    logs: Vec<String>,
    rollbacks: Vec<RollbackReport>,
}

impl Observer for Recorder {
    fn log(&mut self, id: &StageId, line: &LogLine) {
        self.logs.push(format!("{}: {}", id.name, line.text));
    }
    fn event(&mut self, event: &Event) {
        self.events.push(event.clone());
    }
    fn rollback_finished(&mut self, report: &RollbackReport) {
        self.rollbacks.push(report.clone());
    }
}

fn apply(registry: &Registry, options: &Options, bundle: &Bundle) -> RunReport {
    run(bundle, registry, Mode::Apply, options, &mut Quiet)
}

fn hash_of(report: &RunReport, index: usize) -> String {
    artifact_hash(report.stages[index].artifact.as_ref().unwrap())
}

#[test]
fn a_failed_apply_puts_back_every_stage_it_changed_last_first() {
    let registry = registry();
    let options = options("on-failure");
    let first = apply(&registry, &options, &workflow("key-one", "one"));
    assert!(first.succeeded(), "{first:?}");

    // The second apply changes the machine's key, and then fails on the
    // stage after it.
    let mut recorder = Recorder::default();
    let second = run(
        &workflow("key-two", "FAIL two"),
        &registry,
        Mode::Apply,
        &Options {
            on_failure: OnFailure::Rollback,
            ..options.clone()
        },
        &mut recorder,
    );
    assert!(!second.succeeded());
    assert!(matches!(second.stages[1].outcome, Outcome::Failed(_)));

    // Both stages went back: the one that failed first, then the one
    // before it.
    assert_eq!(second.rollbacks, recorder.rollbacks);
    let [web_back, network_back] = second.rollbacks.as_slice() else {
        panic!("{:?}", second.rollbacks);
    };
    assert_eq!(web_back.id, web());
    assert_eq!(web_back.host, "10.0.0.5");
    assert_eq!(
        web_back.outcome,
        RollbackOutcome::RolledBack {
            to: hash_of(&first, 1),
            changes: Vec::new(),
        }
    );
    assert_eq!(network_back.id, network());
    let RollbackOutcome::RolledBack { to, changes } = &network_back.outcome else {
        panic!("{:?}", network_back.outcome);
    };
    assert_eq!(*to, hash_of(&first, 0));
    assert_eq!(changes.len(), 1, "{changes:?}");
    assert_eq!(changes[0].resource, "files/key");
    assert_eq!(changes[0].action, ChangeAction::Update);

    // The world is as the first apply left it.
    assert_eq!(on_disk(&options, &network(), "key"), "key-one");
    assert_eq!(on_disk(&options, &web(), "motd"), "one");

    // So is the run-state: applied, from the first apply's artifacts.
    let backend = options.store();
    let state = RunState::read(backend.as_ref()).unwrap();
    for (id, index) in [(network(), 0), (web(), 1)] {
        let stage = state.get(&id).unwrap();
        assert!(stage.applied, "{id}");
        assert_eq!(stage.artifact_hash, hash_of(&first, index), "{id}");
        assert_eq!(stage.last_good, hash_of(&first, index), "{id}");
        assert_eq!(stage.run, second.run_id, "{id}");
    }

    // The record says the run failed, and what was put back.
    let record = record::load_run(backend.as_ref(), &second.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.status, record::RunStatus::Failed);
    assert_eq!(record.step(&web()).unwrap().status, StepStatus::RolledBack);
    assert_eq!(
        record.step(&network()).unwrap().status,
        StepStatus::RolledBack
    );
    let rollback_events: Vec<(&str, &str)> = recorder
        .events
        .iter()
        .filter(|e| e.kind.starts_with("rollback."))
        .map(|e| (e.kind.as_str(), e.step.as_str()))
        .collect();
    assert_eq!(
        rollback_events,
        [
            ("rollback.started", "web"),
            ("rollback.succeeded", "web"),
            ("rollback.started", "network"),
            ("rollback.succeeded", "network"),
        ]
    );
    let done = recorder
        .events
        .iter()
        .find(|e| e.kind == event::ROLLBACK_SUCCEEDED && e.step == "network")
        .unwrap();
    assert_eq!(
        done.detail["to"].as_str(),
        Some(hash_of(&first, 0).as_str())
    );
    let finished = recorder
        .events
        .iter()
        .find(|e| e.kind == event::RUN_FAILED)
        .unwrap();
    assert_eq!(finished.detail["rolledBack"], meridian_sdk::Value::Int(2));
    assert!(lock::holder(backend.as_ref()).unwrap().is_none());

    // What was put back is in sync with the documents it came from.
    let check = run(
        &workflow("key-one", "one"),
        &registry,
        Mode::Drift,
        &options,
        &mut Quiet,
    );
    assert!(check.succeeded() && !check.drifted(), "{check:?}");
}

#[test]
fn a_first_apply_has_nothing_to_go_back_to_and_says_so() {
    let registry = registry();
    let options = Options {
        on_failure: OnFailure::Rollback,
        ..options("first-apply")
    };
    let mut recorder = Recorder::default();
    let report = run(
        &workflow("key-one", "FAIL one"),
        &registry,
        Mode::Apply,
        &options,
        &mut recorder,
    );
    assert!(!report.succeeded());
    assert_eq!(report.rollbacks.len(), 2);
    for rollback in &report.rollbacks {
        let RollbackOutcome::Unavailable(why) = &rollback.outcome else {
            panic!("{:?}", rollback.outcome);
        };
        assert!(
            why.starts_with("it had never been applied successfully before"),
            "{why}"
        );
    }
    // What the first stage created is left as it is, not half removed.
    assert_eq!(on_disk(&options, &network(), "key"), "key-one");
    let unavailable: Vec<&Event> = recorder
        .events
        .iter()
        .filter(|e| e.kind == event::ROLLBACK_UNAVAILABLE)
        .collect();
    assert_eq!(unavailable.len(), 2);
    assert!(unavailable
        .iter()
        .all(|e| e.detail["reason"].as_str() == Some("first-apply")));
    // Nothing was handed to a tool for the rollback.
    assert!(!recorder
        .events
        .iter()
        .any(|e| e.kind == event::ROLLBACK_STARTED));
}

#[test]
fn the_default_is_to_stop_and_leave_what_was_applied() {
    let registry = registry();
    let options = options("stop");
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    let failed = apply(&registry, &options, &workflow("key-two", "FAIL two"));
    assert!(!failed.succeeded());
    assert!(failed.rollbacks.is_empty());
    assert_eq!(on_disk(&options, &network(), "key"), "key-two");
}

#[test]
fn the_last_apply_can_be_put_back_on_request_even_when_it_succeeded() {
    let registry = registry();
    let options = options("on-request");
    let first = apply(&registry, &options, &workflow("key-one", "one"));
    let second = apply(&registry, &options, &workflow("key-two", "two"));
    assert!(first.succeeded() && second.succeeded());
    assert_eq!(on_disk(&options, &web(), "motd"), "two");

    // The documents on disk are the new ones. A rollback does not compile
    // them: it applies what was kept from before.
    let mut recorder = Recorder::default();
    let back = run(
        &workflow("key-two", "two"),
        &registry,
        Mode::Rollback,
        &options,
        &mut recorder,
    );
    assert!(back.succeeded(), "{back:?}");
    assert!(back.stages.is_empty());
    let ids: Vec<&StageId> = back.rollbacks.iter().map(|r| &r.id).collect();
    assert_eq!(ids, [&web(), &network()]);
    assert!(back.rollbacks.iter().all(
        |r| matches!(&r.outcome, RollbackOutcome::RolledBack { changes, .. } if changes.len() == 1)
    ));
    assert_eq!(on_disk(&options, &network(), "key"), "key-one");
    assert_eq!(on_disk(&options, &web(), "motd"), "one");
    assert!(recorder.logs.iter().any(|l| l.starts_with("web:")));

    // It is a run of its own, recorded as a rollback of the apply.
    let backend = options.store();
    let record = record::load_run(backend.as_ref(), &back.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.mode, "rollback");
    assert_eq!(record.status, record::RunStatus::Succeeded);
    assert_eq!(record.steps.len(), 2);
    assert!(record
        .steps
        .iter()
        .all(|s| s.status == StepStatus::RolledBack));
    assert_eq!(
        recorder.events[1].detail["rollsBack"].as_str(),
        Some(second.run_id.as_str())
    );

    // The same apply cannot be put back twice: it is no longer what is
    // there.
    let again = run(
        &workflow("key-two", "two"),
        &registry,
        Mode::Rollback,
        &options,
        &mut Quiet,
    );
    assert!(!again.succeeded());
    for rollback in &again.rollbacks {
        let RollbackOutcome::Unavailable(why) = &rollback.outcome else {
            panic!("{:?}", rollback.outcome);
        };
        assert_eq!(
            *why,
            format!(
                "it has been applied again since run {}, by run {}, so that run is no longer \
                 what there is to put back",
                second.run_id, back.run_id
            )
        );
    }
    assert_eq!(on_disk(&options, &web(), "motd"), "one");

    // A named run works the same way, and the first apply has nothing
    // before it.
    let named = run(
        &workflow("key-two", "two"),
        &registry,
        Mode::Rollback,
        &Options {
            rollback_of: Resume::Run(first.run_id.clone()),
            ..options.clone()
        },
        &mut Quiet,
    );
    assert!(named
        .rollbacks
        .iter()
        .all(|r| matches!(r.outcome, RollbackOutcome::Unavailable(_))));
}

#[test]
fn an_apply_that_changed_nothing_has_nothing_to_put_back() {
    let registry = registry();
    let options = options("unchanged");
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    let back = run(
        &workflow("key-one", "one"),
        &registry,
        Mode::Rollback,
        &options,
        &mut Quiet,
    );
    assert!(back.succeeded(), "{back:?}");
    assert!(back
        .rollbacks
        .iter()
        .all(|r| r.outcome == RollbackOutcome::Unchanged));
    assert_eq!(on_disk(&options, &network(), "key"), "key-one");
}

#[test]
fn a_rollback_that_fails_stops_and_says_what_is_where() {
    let registry = registry();
    let options = options("rollback-fails");
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    assert!(apply(&registry, &options, &workflow("key-two", "two")).succeeded());

    // The file the rollback has to write is now a directory, so the tool
    // cannot put it back.
    let motd = work_dir(&options.state_dir, &web(), "fake").join("current/files/motd");
    fs::remove_file(&motd).unwrap();
    fs::create_dir(&motd).unwrap();

    let mut recorder = Recorder::default();
    let back = run(
        &workflow("key-two", "two"),
        &registry,
        Mode::Rollback,
        &options,
        &mut recorder,
    );
    assert!(!back.succeeded());
    let [web_back, network_back] = back.rollbacks.as_slice() else {
        panic!("{:?}", back.rollbacks);
    };
    let RollbackOutcome::Failed(why) = &web_back.outcome else {
        panic!("{:?}", web_back.outcome);
    };
    assert!(why.starts_with("writing "), "{why}");
    // The stage before it was built on the one that would not go back, so
    // it is left alone.
    assert_eq!(
        network_back.outcome,
        RollbackOutcome::NotAttempted { because_of: web() }
    );
    assert_eq!(on_disk(&options, &network(), "key"), "key-two");

    let failed = recorder
        .events
        .iter()
        .find(|e| e.kind == event::ROLLBACK_FAILED)
        .unwrap();
    assert_eq!(failed.step, "web");
    assert_eq!(failed.detail["reason"].as_str(), Some("tool-failed"));
    // What the tool said is for the terminal, not the record.
    let backend = options.store();
    let kept = String::from_utf8(
        backend
            .get(&record::events_key(&back.run_id))
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert!(!kept.contains("writing "), "{kept}");

    // The run-state does not pretend: the last attempt on the stage failed.
    let state = RunState::read(backend.as_ref()).unwrap();
    assert!(!state.get(&web()).unwrap().applied);
    assert!(lock::holder(backend.as_ref()).unwrap().is_none());
}

#[test]
fn a_stage_without_its_snapshot_is_reported_with_what_to_do_by_hand() {
    let registry = registry();
    let options = options("no-snapshot");
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    assert!(apply(&registry, &options, &workflow("key-two", "two")).succeeded());
    // Another machine, or a cleaned one: the snapshots are not here.
    fs::remove_dir_all(options.state_dir.join(history::HISTORY_DIR)).unwrap();

    let back = run(
        &workflow("key-two", "two"),
        &registry,
        Mode::Rollback,
        &options,
        &mut Quiet,
    );
    assert!(!back.succeeded());
    for rollback in &back.rollbacks {
        assert_eq!(
            rollback.outcome,
            RollbackOutcome::Unavailable(
                "the artifact it was last applied from successfully is not kept on this \
                 machine; check out the revision of the document that was applied before, and \
                 apply it"
                    .to_owned()
            )
        );
    }
    assert_eq!(on_disk(&options, &web(), "motd"), "two");
}

#[test]
fn a_rollback_needs_the_documents_and_something_to_put_back() {
    let registry = registry();
    let options = options("refusals");
    let refusal = |options: &Options| {
        let report = run(
            &workflow("key-one", "one"),
            &registry,
            Mode::Rollback,
            options,
            &mut Quiet,
        );
        assert!(report.rollbacks.is_empty());
        match report.refused {
            Some(Refusal::Rollback(why)) => why,
            other => panic!("expected a refusal, got {other:?}"),
        }
    };
    assert_eq!(
        refusal(&options),
        "there is no apply to roll back: none has been recorded in this state"
    );

    let planned = run(
        &workflow("key-one", "one"),
        &registry,
        Mode::Plan,
        &options,
        &mut Quiet,
    );
    assert_eq!(
        refusal(&Options {
            rollback_of: Resume::Run(planned.run_id.clone()),
            ..options.clone()
        }),
        format!(
            "run {} was a plan, and only an apply can be rolled back",
            planned.run_id
        )
    );

    // An apply that was refused before any tool ran changed nothing.
    let refused = apply(
        &registry,
        &options,
        &bundle(&WORKFLOW.replace("target: fake", "target: missing")),
    );
    assert!(!refused.succeeded());
    assert_eq!(
        refusal(&options),
        format!(
            "run {} has nothing to roll back: no stage it applied is left to put back",
            refused.run_id
        )
    );

    // Given only some of the documents, the others are named, not guessed.
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());
    assert!(apply(&registry, &options, &workflow("key-two", "two")).succeeded());
    let only_network = WORKFLOW
        .split("---")
        .next()
        .unwrap()
        .replace("KEY", "key-two");
    let partial = run(
        &bundle(&only_network),
        &registry,
        Mode::Rollback,
        &options,
        &mut Quiet,
    );
    assert!(!partial.succeeded());
    let RollbackOutcome::Unavailable(why) = &partial.rollbacks[0].outcome else {
        panic!("{:?}", partial.rollbacks[0].outcome);
    };
    assert!(
        why.starts_with("its document is not among the files given"),
        "{why}"
    );
    assert!(matches!(
        partial.rollbacks[1].outcome,
        RollbackOutcome::RolledBack { .. }
    ));
}

#[test]
fn only_the_snapshots_a_rollback_could_want_are_kept() {
    let registry = registry();
    let options = options("snapshots");
    let mut hashes = Vec::new();
    for motd in ["one", "two", "three"] {
        let report = apply(&registry, &options, &workflow("key-one", motd));
        assert!(report.succeeded());
        assert!(report.warnings.is_empty(), "{:?}", report.warnings);
        hashes.push(hash_of(&report, 1));
    }
    let load = |hash: &str| history::load(&options.state_dir, &web(), hash).unwrap();
    assert!(load(&hashes[0]).is_none());
    assert_eq!(load(&hashes[1]).unwrap().files["files/motd"], "two");
    assert_eq!(load(&hashes[2]).unwrap().files["files/motd"], "three");
    // A snapshot is beside the working directories, on this machine, and
    // never in the record.
    assert!(options
        .store()
        .list("runs/")
        .unwrap()
        .iter()
        .all(|key| !key.contains("history")));
}

fn drift(registry: &Registry, options: &Options, bundle: &Bundle) -> (RunReport, Vec<Event>) {
    let mut recorder = Recorder::default();
    let report = run(bundle, registry, Mode::Drift, options, &mut recorder);
    (report, recorder.events)
}

fn drift_of(report: &RunReport, index: usize) -> &Drift {
    match &report.stages[index].outcome {
        Outcome::Drift(drift) => drift,
        other => panic!("{other:?}"),
    }
}

#[test]
fn drift_is_a_difference_nobody_asked_for() {
    let registry = registry();
    let options = options("drift");
    let documents = workflow("key-one", "one");

    // Before anything is applied there is nothing to compare with.
    let (before, _) = drift(&registry, &options, &documents);
    assert!(before.succeeded() && !before.drifted());
    assert_eq!(*drift_of(&before, 0), Drift::NeverApplied);
    assert!(matches!(before.stages[1].outcome, Outcome::Deferred { .. }));

    assert!(apply(&registry, &options, &documents).succeeded());
    let (in_sync, events) = drift(&registry, &options, &documents);
    assert!(in_sync.succeeded() && !in_sync.drifted());
    assert_eq!(*drift_of(&in_sync, 0), Drift::InSync);
    assert_eq!(*drift_of(&in_sync, 1), Drift::InSync);
    assert_eq!(
        events
            .iter()
            .filter(|e| e.kind == event::DRIFT_NONE)
            .count(),
        2
    );

    // Someone changes the machine behind Meridian's back. The fake tool's
    // idea of the world is its state file.
    let tool_state = work_dir(&options.state_dir, &web(), "fake").join("state.json");
    let text = fs::read_to_string(&tool_state).unwrap();
    fs::write(&tool_state, text.replace("\"one\"", "\"edited by hand\"")).unwrap();

    let (drifted, events) = drift(&registry, &options, &documents);
    assert!(drifted.succeeded(), "a check that finds drift did its job");
    assert!(drifted.drifted());
    assert_eq!(*drift_of(&drifted, 0), Drift::InSync);
    let Drift::Drifted(changes) = drift_of(&drifted, 1) else {
        panic!("{:?}", drift_of(&drifted, 1));
    };
    assert_eq!(changes.len(), 1);
    assert_eq!(changes[0].resource, "files/motd");
    assert_eq!(changes[0].action, ChangeAction::Update);
    let detected = events
        .iter()
        .find(|e| e.kind == event::DRIFT_DETECTED)
        .unwrap();
    assert_eq!(detected.step, "web");
    assert_eq!(detected.endpoint, "10.0.0.5");
    assert_eq!(detected.detail["changes"], meridian_sdk::Value::Int(1));

    // The check changed nothing, and is a recorded run that held the lock.
    assert_eq!(on_disk(&options, &web(), "motd"), "one");
    let backend = options.store();
    let record = record::load_run(backend.as_ref(), &drifted.run_id)
        .unwrap()
        .unwrap();
    assert_eq!(record.mode, "drift");
    assert!(lock::holder(backend.as_ref()).unwrap().is_none());

    // Applying again restores it.
    assert!(apply(&registry, &options, &documents).succeeded());
    assert!(!drift(&registry, &options, &documents).0.drifted());
}

#[test]
fn a_changed_document_is_pending_and_not_drift() {
    let registry = registry();
    let options = options("pending");
    assert!(apply(&registry, &options, &workflow("key-one", "one")).succeeded());

    let (report, events) = drift(&registry, &options, &workflow("key-one", "two"));
    assert!(report.succeeded() && !report.drifted());
    assert_eq!(*drift_of(&report, 0), Drift::InSync);
    let Drift::Pending {
        changes,
        last_apply_failed,
    } = drift_of(&report, 1)
    else {
        panic!("{:?}", drift_of(&report, 1));
    };
    assert_eq!(changes.len(), 1);
    assert!(!last_apply_failed);
    let unknown = events
        .iter()
        .find(|e| e.kind == event::DRIFT_UNKNOWN)
        .unwrap();
    assert_eq!(unknown.detail["reason"].as_str(), Some("document-changed"));

    // An apply that failed part way leaves the stage between two artifacts.
    assert!(!apply(&registry, &options, &workflow("key-one", "FAIL two")).succeeded());
    let (report, events) = drift(&registry, &options, &workflow("key-one", "FAIL two"));
    assert!(matches!(
        drift_of(&report, 1),
        Drift::Pending {
            last_apply_failed: true,
            ..
        }
    ));
    assert_eq!(
        events
            .iter()
            .find(|e| e.kind == event::DRIFT_UNKNOWN)
            .unwrap()
            .detail["reason"]
            .as_str(),
        Some("last-apply-failed")
    );
}

#[test]
fn a_tool_that_cannot_preview_is_unknown_and_never_in_sync() {
    let registry = registry();
    let options = options("unknown");
    // The fake tool reports that it cannot preview a file holding this word.
    let documents = workflow("key-one", "UNKNOWN");
    assert!(apply(&registry, &options, &documents).succeeded());
    let (report, events) = drift(&registry, &options, &documents);
    assert!(report.succeeded() && !report.drifted());
    assert_eq!(*drift_of(&report, 1), Drift::Unknown);
    assert_eq!(
        events
            .iter()
            .find(|e| e.kind == event::DRIFT_UNKNOWN)
            .unwrap()
            .detail["reason"]
            .as_str(),
        Some("no-preview")
    );
}
