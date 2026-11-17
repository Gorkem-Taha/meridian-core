//! The orchestrator's own run-state: which document was last applied, from
//! which artifact, and whether it worked.
//!
//! It exists because the tools disagree about state. Terraform keeps its own
//! and knows nothing of what was configured afterwards; Ansible keeps none.
//! This file records only what Meridian itself did. It is never the source of
//! truth about infrastructure, and nothing in it is secret: outputs are not
//! stored here, they are read back from the tool that owns them.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use meridian_sdk::{canonical_map, decode_map, Artifact, Value};
use sha2::{Digest, Sha256};

use crate::{StageId, StageKind};

/// What happened the last time a stage was applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageRecord {
    pub target: String,
    /// The hash of the artifact that was applied, from [`artifact_hash`].
    pub artifact_hash: String,
    /// Whether the apply succeeded.
    pub applied: bool,
}

/// Every stage's last apply.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunState {
    stages: BTreeMap<String, StageRecord>,
}

fn key(id: &StageId) -> String {
    let kind = match id.kind {
        StageKind::Infrastructure => "infrastructure",
        StageKind::ResourceSet => "resourceset",
    };
    format!("{kind}/{}", id.name)
}

impl RunState {
    /// Reads the file. A missing or unreadable one is an empty state: the
    /// file is a record, and losing it loses history, not infrastructure.
    pub fn load(path: &Path) -> RunState {
        let Ok(bytes) = std::fs::read(path) else {
            return RunState::default();
        };
        let Ok(map) = decode_map(&bytes) else {
            return RunState::default();
        };
        let mut state = RunState::default();
        for (key, value) in map {
            let Value::Map(fields) = value else { continue };
            let text = |name: &str| match fields.get(name) {
                Some(Value::String(s)) => s.clone(),
                _ => String::new(),
            };
            state.stages.insert(
                key,
                StageRecord {
                    target: text("target"),
                    artifact_hash: text("artifactHash"),
                    applied: text("status") == "applied",
                },
            );
        }
        state
    }

    pub fn get(&self, id: &StageId) -> Option<&StageRecord> {
        self.stages.get(&key(id))
    }

    pub fn set(&mut self, id: &StageId, record: StageRecord) {
        self.stages.insert(key(id), record);
    }

    /// Writes the file, replacing it in one step so a crash never leaves half
    /// of one.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let map: BTreeMap<String, Value> = self
            .stages
            .iter()
            .map(|(key, record)| {
                let fields: BTreeMap<String, Value> = [
                    ("target".to_owned(), Value::from(record.target.as_str())),
                    (
                        "artifactHash".to_owned(),
                        Value::from(record.artifact_hash.as_str()),
                    ),
                    (
                        "status".to_owned(),
                        Value::from(if record.applied { "applied" } else { "failed" }),
                    ),
                ]
                .into_iter()
                .collect();
                (key.clone(), Value::Map(fields))
            })
            .collect();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let staged = path.with_extension("json.tmp");
        std::fs::write(&staged, canonical_map(&map) + "\n")?;
        std::fs::rename(&staged, path)
    }
}

/// A stable fingerprint of an artifact: the same files with the same content
/// always hash the same, whatever order they were produced in.
pub fn artifact_hash(artifact: &Artifact) -> String {
    let mut hasher = Sha256::new();
    for (path, content) in &artifact.files {
        // Lengths go in, so no two different sets of files can run together
        // into the same bytes.
        hasher.update((path.len() as u64).to_be_bytes());
        hasher.update(path.as_bytes());
        hasher.update((content.len() as u64).to_be_bytes());
        hasher.update(content.as_bytes());
    }
    let mut out = String::from("sha256:");
    for byte in hasher.finalize() {
        let _ = write!(out, "{byte:02x}");
    }
    out
}
