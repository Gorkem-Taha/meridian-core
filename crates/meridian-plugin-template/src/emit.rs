//! Stage four: serialisation, and nothing else. By now every decision has
//! been made, so this stage cannot fail.
//!
//! The output must be the same bytes every time for the same document: write
//! in a fixed order, never in the order of a hash map. The golden-file tests
//! compare byte for byte.

use std::collections::BTreeMap;

use crate::Entry;

/// The manifest's path inside the artifact.
pub const MANIFEST_PATH: &str = "manifest.tsv";

/// The artifact: the manifest, and the content of each file beside it.
///
/// TODO(plugin): write your tool's own files. For YAML, `meridian-yaml` has a
/// deterministic writer the first-party plugins share.
pub(crate) fn serialise(entries: &[Entry]) -> BTreeMap<String, String> {
    let mut files = BTreeMap::new();
    let mut manifest = String::new();
    for entry in entries {
        if entry.present {
            let content = format!("content/{}", entry.id);
            manifest.push_str(&format!(
                "present\t{}\t{}\t{content}\n",
                entry.path, entry.mode
            ));
            files.insert(content, entry.content.clone());
        } else {
            manifest.push_str(&format!("absent\t{}\n", entry.path));
        }
    }
    files.insert(MANIFEST_PATH.to_owned(), manifest);
    files
}
