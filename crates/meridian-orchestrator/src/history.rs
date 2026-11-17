//! Snapshots of what was applied, so it can be applied again.
//!
//! Putting a stage back means applying the artifact of its last successful
//! apply, and to do that Meridian must still have it. After every successful
//! apply the artifact is kept under `history/` in the state directory, beside
//! the stage's working directory.
//!
//! A snapshot is kept on this machine only and is never sent to a backend.
//! An artifact can hold whatever its document holds, and the plugin already
//! writes it into the working directory next door, so keeping it here leaks
//! nothing new; sending it to a shared store would.

use std::path::Path;

use meridian_sdk::{canonical_map, decode_map, Artifact, Value};

use crate::backend::{Backend, LocalBackend};
use crate::{artifact_hash, stage_key, StageId};

/// The directory under the state directory that holds every snapshot.
pub const HISTORY_DIR: &str = "history";

fn store(state_dir: &Path) -> LocalBackend {
    LocalBackend::new(state_dir.join(HISTORY_DIR))
}

/// The key of one snapshot, or `None` when `hash` is not an artifact hash.
fn key(id: &StageId, hash: &str) -> Option<String> {
    let hex = hash.strip_prefix("sha256:")?;
    let safe = hex.len() == 64 && hex.chars().all(|c| c.is_ascii_hexdigit());
    safe.then(|| format!("{}/{hex}.json", stage_key(id)))
}

/// Keeps the artifact a stage was applied from.
pub fn save(state_dir: &Path, id: &StageId, artifact: &Artifact) -> Result<(), String> {
    let hash = artifact_hash(artifact);
    let key = key(id, &hash).ok_or_else(|| format!("{hash:?} is not an artifact hash"))?;
    let files = artifact
        .files
        .iter()
        .map(|(path, content)| (path.clone(), Value::from(content.as_str())))
        .collect();
    store(state_dir).put(&key, canonical_map(&files).as_bytes())
}

/// The artifact with this hash, when it is kept here. A snapshot that no
/// longer hashes to its name has been changed, and is an error, not an
/// artifact: applying it would apply something nobody compiled.
pub fn load(state_dir: &Path, id: &StageId, hash: &str) -> Result<Option<Artifact>, String> {
    let Some(key) = key(id, hash) else {
        return Ok(None);
    };
    let Some(bytes) = store(state_dir).get(&key)? else {
        return Ok(None);
    };
    let map = decode_map(&bytes).map_err(|e| format!("the snapshot {key} cannot be read: {e}"))?;
    let mut artifact = Artifact::default();
    for (path, content) in map {
        let Value::String(content) = content else {
            return Err(format!(
                "the snapshot {key} cannot be read: {path} is not text"
            ));
        };
        artifact.files.insert(path, content);
    }
    if artifact_hash(&artifact) != hash {
        return Err(format!(
            "the snapshot {key} has been changed since it was kept, so it will not be applied"
        ));
    }
    Ok(Some(artifact))
}

/// Removes every snapshot of a stage but those with the hashes in `keep`.
pub fn prune(state_dir: &Path, id: &StageId, keep: &[&str]) -> Result<(), String> {
    let store = store(state_dir);
    let kept: Vec<String> = keep.iter().filter_map(|hash| key(id, hash)).collect();
    for found in store.list(&format!("{}/", stage_key(id)))? {
        if !kept.contains(&found) {
            store.delete(&found)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::StageKind;

    fn scratch(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir()
            .join(format!("meridian-history-test-{}", std::process::id()))
            .join(name);
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn id() -> StageId {
        StageId {
            kind: StageKind::ResourceSet,
            name: "web".to_owned(),
        }
    }

    fn artifact(content: &str) -> Artifact {
        let mut artifact = Artifact::default();
        artifact
            .files
            .insert("site.yml".to_owned(), content.to_owned());
        artifact
    }

    #[test]
    fn a_snapshot_reads_back_and_only_the_kept_ones_survive_pruning() {
        let dir = scratch("keep");
        let (a, b, c) = (artifact("a"), artifact("b"), artifact("c"));
        for each in [&a, &b, &c] {
            save(&dir, &id(), each).unwrap();
        }
        assert_eq!(
            load(&dir, &id(), &artifact_hash(&b)).unwrap(),
            Some(b.clone())
        );
        prune(&dir, &id(), &[&artifact_hash(&b), &artifact_hash(&c), ""]).unwrap();
        assert_eq!(load(&dir, &id(), &artifact_hash(&a)).unwrap(), None);
        assert_eq!(load(&dir, &id(), &artifact_hash(&b)).unwrap(), Some(b));
        assert_eq!(load(&dir, &id(), &artifact_hash(&c)).unwrap(), Some(c));
        // Another stage's snapshots are its own.
        let other = StageId {
            kind: StageKind::Infrastructure,
            name: "web".to_owned(),
        };
        assert_eq!(
            load(&dir, &other, &artifact_hash(&artifact("b"))).unwrap(),
            None
        );
    }

    #[test]
    fn a_hash_that_is_not_one_finds_nothing_and_never_becomes_a_path() {
        let dir = scratch("bad-hash");
        save(&dir, &id(), &artifact("a")).unwrap();
        for bad in ["", "sha256:", "sha256:../../../etc/passwd", "md5:abc"] {
            assert_eq!(load(&dir, &id(), bad).unwrap(), None, "{bad:?}");
        }
    }

    #[test]
    fn a_snapshot_that_was_changed_is_refused() {
        let dir = scratch("tampered");
        let original = artifact("a");
        save(&dir, &id(), &original).unwrap();
        let hash = artifact_hash(&original);
        let file = dir
            .join(HISTORY_DIR)
            .join("resourceset/web")
            .join(format!("{}.json", hash.trim_start_matches("sha256:")));
        std::fs::write(&file, r#"{"site.yml":"something else"}"#).unwrap();
        let err = load(&dir, &id(), &hash).unwrap_err();
        assert!(err.contains("has been changed since it was kept"), "{err}");
    }
}
