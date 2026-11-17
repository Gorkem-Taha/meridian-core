//! The record of a run: what was asked, who asked, and how each step ended.
//!
//! A run has two blobs under `runs/<id>/`. `run.json` is the summary, written
//! when the run starts and again at every step boundary, so a run that dies
//! leaves a record that still says `running`, which is the state most worth
//! being able to see. `events.jsonl` is every event, in order.
//!
//! The record says what Meridian did. It is never the truth about
//! infrastructure, and it holds no value a document or a tool produced.

use std::collections::BTreeMap;

use meridian_sdk::{canonical_map, decode_map, Value};

use crate::backend::Backend;
use crate::event::{self, Event};
use crate::{StageId, StageKind};

/// How a run stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    /// Still going, or it died without saying how it ended.
    Running,
    Succeeded,
    Failed,
}

impl RunStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            RunStatus::Running => "running",
            RunStatus::Succeeded => "succeeded",
            RunStatus::Failed => "failed",
        }
    }

    fn parse(text: &str) -> RunStatus {
        match text {
            "succeeded" => RunStatus::Succeeded,
            "failed" => RunStatus::Failed,
            _ => RunStatus::Running,
        }
    }
}

/// How a step stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepStatus {
    Running,
    /// It did what was asked: compiled, planned or applied.
    Succeeded,
    Failed,
    /// A resumed run found it already applied from the same artifact.
    Skipped,
    /// It waits on a value that does not exist yet.
    Deferred,
    /// An earlier step failed, so it was not attempted.
    NotRun,
    /// It was put back to what was last applied successfully.
    RolledBack,
}

impl StepStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            StepStatus::Running => "running",
            StepStatus::Succeeded => "succeeded",
            StepStatus::Failed => "failed",
            StepStatus::Skipped => "skipped",
            StepStatus::Deferred => "deferred",
            StepStatus::NotRun => "not-run",
            StepStatus::RolledBack => "rolled-back",
        }
    }

    fn parse(text: &str) -> StepStatus {
        match text {
            "succeeded" => StepStatus::Succeeded,
            "failed" => StepStatus::Failed,
            "skipped" => StepStatus::Skipped,
            "deferred" => StepStatus::Deferred,
            "not-run" => StepStatus::NotRun,
            "rolled-back" => StepStatus::RolledBack,
            _ => StepStatus::Running,
        }
    }
}

/// One step of a run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepRecord {
    pub id: StageId,
    pub target: String,
    /// The machine the step configures, when there is one.
    pub endpoint: String,
    pub status: StepStatus,
    /// The hash of what the step compiled to, when it got that far.
    pub artifact: String,
    /// How many changes the tool reported.
    pub changes: u64,
    /// For a failed step, the fixed word that says what kind of failure.
    pub reason: String,
    /// The artifact hash of the step's last successful apply before this
    /// run, which is what a rollback of this run goes back to. Empty when
    /// there was none, or when the run did not apply.
    pub previous: String,
}

/// One run.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunRecord {
    pub id: String,
    /// `plan`, `apply`, `drift` or `rollback`.
    pub mode: String,
    /// What was typed, as far as the caller passed it on.
    pub command: String,
    /// Best effort: the operating system user and the machine's name. It is
    /// not an identity and authenticates nothing.
    pub user: String,
    pub host: String,
    /// The label selector the run was narrowed with; empty for none.
    pub selector: String,
    pub started: String,
    /// Empty while the run is going.
    pub finished: String,
    pub status: RunStatus,
    /// The run this one continues, when it was started with `--resume`.
    pub resumes: String,
    pub steps: Vec<StepRecord>,
}

fn kind_text(kind: StageKind) -> &'static str {
    kind.as_str()
}

fn kind_parse(text: &str) -> StageKind {
    if text == "ResourceSet" {
        StageKind::ResourceSet
    } else {
        StageKind::Infrastructure
    }
}

impl RunRecord {
    /// The step named `id`, when the run reached it.
    pub fn step(&self, id: &StageId) -> Option<&StepRecord> {
        self.steps.iter().find(|step| &step.id == id)
    }

    /// Records a step, replacing what was recorded for it before.
    pub fn set_step(&mut self, step: StepRecord) {
        match self.steps.iter_mut().find(|s| s.id == step.id) {
            Some(existing) => *existing = step,
            None => self.steps.push(step),
        }
    }

    pub fn to_json(&self) -> String {
        let text = |s: &str| Value::from(s);
        let steps = self
            .steps
            .iter()
            .map(|step| {
                Value::Map(
                    [
                        ("kind", text(kind_text(step.id.kind))),
                        ("name", text(&step.id.name)),
                        ("target", text(&step.target)),
                        ("endpoint", text(&step.endpoint)),
                        ("status", text(step.status.as_str())),
                        ("artifact", text(&step.artifact)),
                        ("changes", Value::Uint(step.changes)),
                        ("reason", text(&step.reason)),
                        ("previous", text(&step.previous)),
                    ]
                    .into_iter()
                    .map(|(key, value)| (key.to_owned(), value))
                    .collect(),
                )
            })
            .collect();
        let map: BTreeMap<String, Value> = [
            ("id", text(&self.id)),
            ("mode", text(&self.mode)),
            ("command", text(&self.command)),
            ("user", text(&self.user)),
            ("host", text(&self.host)),
            ("selector", text(&self.selector)),
            ("started", text(&self.started)),
            ("finished", text(&self.finished)),
            ("status", text(self.status.as_str())),
            ("resumes", text(&self.resumes)),
            ("steps", Value::List(steps)),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        canonical_map(&map) + "\n"
    }

    pub fn from_json(bytes: &[u8]) -> Result<RunRecord, String> {
        let map = decode_map(bytes)?;
        let text_of = |map: &BTreeMap<String, Value>, name: &str| match map.get(name) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let text = |name: &str| text_of(&map, name);
        let mut steps = Vec::new();
        if let Some(Value::List(list)) = map.get("steps") {
            for item in list {
                let Value::Map(step) = item else { continue };
                steps.push(StepRecord {
                    id: StageId {
                        kind: kind_parse(&text_of(step, "kind")),
                        name: text_of(step, "name"),
                    },
                    target: text_of(step, "target"),
                    endpoint: text_of(step, "endpoint"),
                    status: StepStatus::parse(&text_of(step, "status")),
                    artifact: text_of(step, "artifact"),
                    changes: match step.get("changes") {
                        Some(Value::Uint(n)) => *n,
                        Some(Value::Int(n)) => u64::try_from(*n).unwrap_or(0),
                        _ => 0,
                    },
                    reason: text_of(step, "reason"),
                    previous: text_of(step, "previous"),
                });
            }
        }
        let id = text("id");
        if id.is_empty() {
            return Err("the run record has no id".to_owned());
        }
        Ok(RunRecord {
            id,
            mode: text("mode"),
            command: text("command"),
            user: text("user"),
            host: text("host"),
            selector: text("selector"),
            started: text("started"),
            finished: text("finished"),
            status: RunStatus::parse(&text("status")),
            resumes: text("resumes"),
            steps,
        })
    }
}

/// The key of a run's summary.
pub fn run_key(id: &str) -> String {
    format!("runs/{id}/run.json")
}

/// The key of a run's events.
pub fn events_key(id: &str) -> String {
    format!("runs/{id}/events.jsonl")
}

/// The ids of every recorded run, oldest first.
pub fn list_runs(backend: &dyn Backend) -> Result<Vec<String>, String> {
    let mut ids: Vec<String> = backend
        .list("runs/")?
        .iter()
        .filter_map(|key| {
            key.strip_prefix("runs/")?
                .strip_suffix("/run.json")
                .map(str::to_owned)
        })
        .filter(|id| crate::time::is_run_id(id))
        .collect();
    ids.sort();
    ids.dedup();
    Ok(ids)
}

/// One run's summary, or `None` when there is no such run.
pub fn load_run(backend: &dyn Backend, id: &str) -> Result<Option<RunRecord>, String> {
    if !crate::time::is_run_id(id) {
        return Err(format!("{id:?} is not a run id"));
    }
    match backend.get(&run_key(id))? {
        None => Ok(None),
        Some(bytes) => RunRecord::from_json(&bytes)
            .map(Some)
            .map_err(|e| format!("the record of run {id} cannot be read: {e}")),
    }
}

/// One run's events, in order.
pub fn load_events(backend: &dyn Backend, id: &str) -> Result<Vec<Event>, String> {
    if !crate::time::is_run_id(id) {
        return Err(format!("{id:?} is not a run id"));
    }
    Ok(backend
        .get(&events_key(id))?
        .map(|bytes| event::from_jsonl(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default())
}

/// The newest run, or the newest of one mode.
pub fn last_run(backend: &dyn Backend, mode: Option<&str>) -> Result<Option<RunRecord>, String> {
    for id in list_runs(backend)?.iter().rev() {
        if let Some(record) = load_run(backend, id)? {
            if mode.is_none_or(|mode| record.mode == mode) {
                return Ok(Some(record));
            }
        }
    }
    Ok(None)
}

/// Removes every run but the newest `keep`.
pub fn prune(backend: &dyn Backend, keep: usize) -> Result<(), String> {
    let ids = list_runs(backend)?;
    let excess = ids.len().saturating_sub(keep);
    for id in &ids[..excess] {
        for key in backend.list(&format!("runs/{id}/"))? {
            backend.delete(&key)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;

    fn record(id: &str, mode: &str) -> RunRecord {
        RunRecord {
            id: id.to_owned(),
            mode: mode.to_owned(),
            command: "apply site.yaml".to_owned(),
            user: "mike".to_owned(),
            host: "build-07".to_owned(),
            selector: "env=prod".to_owned(),
            started: "2026-11-17T09:14:03.000Z".to_owned(),
            finished: String::new(),
            status: RunStatus::Running,
            resumes: String::new(),
            steps: vec![StepRecord {
                id: StageId {
                    kind: StageKind::ResourceSet,
                    name: "web".to_owned(),
                },
                target: "ansible".to_owned(),
                endpoint: "10.0.0.5".to_owned(),
                status: StepStatus::Failed,
                artifact: "sha256:abc".to_owned(),
                changes: 2,
                reason: "tool-failed".to_owned(),
                previous: "sha256:old".to_owned(),
            }],
        }
    }

    #[test]
    fn a_record_reads_back_as_it_was_written() {
        let record = record("2026-11-17T09-14-03Z-7f3a", "apply");
        assert_eq!(
            RunRecord::from_json(record.to_json().as_bytes()).unwrap(),
            record
        );
        assert!(RunRecord::from_json(b"{}").is_err());
    }

    #[test]
    fn a_step_is_recorded_once_and_its_last_state_wins() {
        let mut record = record("a", "apply");
        let mut step = record.steps[0].clone();
        step.status = StepStatus::RolledBack;
        record.set_step(step.clone());
        assert_eq!(record.steps, [step.clone()]);
        assert_eq!(record.step(&step.id), Some(&step));
    }

    #[test]
    fn runs_are_listed_oldest_first_and_the_last_of_a_mode_is_found() {
        let backend = MemoryBackend::new();
        for (id, mode) in [
            ("2026-11-17T09-00-00Z-0001", "apply"),
            ("2026-11-17T10-00-00Z-0002", "plan"),
            ("2026-11-16T09-00-00Z-0003", "apply"),
        ] {
            backend
                .put(&run_key(id), record(id, mode).to_json().as_bytes())
                .unwrap();
        }
        backend.put("runs/not a run id/run.json", b"{}").unwrap();
        assert_eq!(
            list_runs(&backend).unwrap(),
            [
                "2026-11-16T09-00-00Z-0003",
                "2026-11-17T09-00-00Z-0001",
                "2026-11-17T10-00-00Z-0002"
            ]
        );
        assert_eq!(
            last_run(&backend, None).unwrap().unwrap().id,
            "2026-11-17T10-00-00Z-0002"
        );
        assert_eq!(
            last_run(&backend, Some("apply")).unwrap().unwrap().id,
            "2026-11-17T09-00-00Z-0001"
        );
        assert!(last_run(&backend, Some("drift")).unwrap().is_none());
        assert!(load_run(&backend, "../../etc").is_err());
        assert!(load_run(&backend, "2020-01-01T00-00-00Z-ffff")
            .unwrap()
            .is_none());
    }

    #[test]
    fn pruning_keeps_the_newest_runs_and_all_of_each() {
        let backend = MemoryBackend::new();
        for n in 1..=5 {
            let id = format!("2026-11-17T09-00-0{n}Z-000{n}");
            backend
                .put(&run_key(&id), record(&id, "apply").to_json().as_bytes())
                .unwrap();
            backend.put(&events_key(&id), b"").unwrap();
        }
        prune(&backend, 2).unwrap();
        assert_eq!(
            backend.list("runs/").unwrap(),
            [
                "runs/2026-11-17T09-00-04Z-0004/events.jsonl",
                "runs/2026-11-17T09-00-04Z-0004/run.json",
                "runs/2026-11-17T09-00-05Z-0005/events.jsonl",
                "runs/2026-11-17T09-00-05Z-0005/run.json"
            ]
        );
        prune(&backend, 50).unwrap();
        assert_eq!(backend.list("runs/").unwrap().len(), 4);
    }
}
