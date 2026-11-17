//! Reading the manifests the tools print: which objects they hold, and how
//! two sets of them differ.
//!
//! Helm and Kustomize both write block-style YAML with one object per
//! document, its `apiVersion`, `kind` and `metadata` at the margin. That is
//! all this module relies on, so it reads the few lines it needs and does not
//! carry a YAML parser into the plugin.

use std::collections::BTreeMap;

use meridian_sdk::ChangeAction;

/// What identifies an object in a cluster.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Key {
    pub api_version: String,
    pub kind: String,
    /// Empty when the manifest leaves the namespace to the release.
    pub namespace: String,
    pub name: String,
}

/// One object of a rendered manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Object {
    pub key: Key,
    /// The manifest without comments and surrounding blank lines, so the same
    /// object printed by two commands compares equal.
    pub text: String,
    /// Helm runs a hook at a point of the release's life and does not keep it
    /// as part of the release.
    pub hook: bool,
}

/// A scalar as written after `key:`, without its quotes or a trailing
/// comment.
fn scalar(written: &str) -> String {
    let written = written.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = written.strip_prefix(quote) {
            if let Some(end) = rest.find(quote) {
                return rest[..end].to_owned();
            }
        }
    }
    match written.find(" #") {
        Some(comment) => written[..comment].trim_end().to_owned(),
        None => written.to_owned(),
    }
}

fn read(document: &str) -> Option<Object> {
    let mut key = Key {
        api_version: String::new(),
        kind: String::new(),
        namespace: String::new(),
        name: String::new(),
    };
    let mut in_metadata = false;
    let mut hook = false;
    let mut lines = Vec::new();
    for line in document.lines() {
        let line = line.trim_end();
        if line.starts_with('#') {
            continue;
        }
        lines.push(line);
        let field = line.trim_start();
        hook |= ["helm.sh/hook:", "\"helm.sh/hook\":", "'helm.sh/hook':"]
            .iter()
            .any(|marker| field.starts_with(marker));
        if !line.starts_with(' ') {
            in_metadata = line == "metadata:";
            if let Some(value) = line.strip_prefix("apiVersion:") {
                key.api_version = scalar(value);
            } else if let Some(value) = line.strip_prefix("kind:") {
                key.kind = scalar(value);
            }
        } else if in_metadata && line.starts_with("  ") && !line.starts_with("   ") {
            if let Some(value) = field.strip_prefix("name:") {
                key.name = scalar(value);
            } else if let Some(value) = field.strip_prefix("namespace:") {
                key.namespace = scalar(value);
            }
        }
    }
    if key.kind.is_empty() || key.name.is_empty() {
        return None;
    }
    let first = lines.iter().position(|l| !l.is_empty())?;
    let last = lines.iter().rposition(|l| !l.is_empty())?;
    Some(Object {
        key,
        text: lines[first..=last].join("\n"),
        hook,
    })
}

/// The objects of a manifest stream, in the order they appear. A document
/// that is not an object, such as an empty one, is skipped.
pub(crate) fn objects(text: &str) -> Vec<Object> {
    let mut found = Vec::new();
    let mut document = String::new();
    for line in text.lines() {
        if line == "---" || line.starts_with("--- ") {
            found.extend(read(&document));
            document.clear();
        } else {
            document.push_str(line);
            document.push('\n');
        }
    }
    found.extend(read(&document));
    found
}

/// How the objects of a release would change: what is new and what differs,
/// in the order of the new manifest, then what is gone. Hooks are left out on
/// both sides, since a release does not hold them.
pub(crate) fn diff(old: &[Object], new: &[Object]) -> Vec<(ChangeAction, Key)> {
    let before: BTreeMap<&Key, &str> = old
        .iter()
        .filter(|o| !o.hook)
        .map(|o| (&o.key, o.text.as_str()))
        .collect();
    let mut changes = Vec::new();
    let mut kept = BTreeMap::new();
    for object in new.iter().filter(|o| !o.hook) {
        kept.insert(&object.key, ());
        match before.get(&object.key) {
            None => changes.push((ChangeAction::Create, object.key.clone())),
            Some(text) if *text != object.text => {
                changes.push((ChangeAction::Update, object.key.clone()));
            }
            Some(_) => {}
        }
    }
    for object in old.iter().filter(|o| !o.hook) {
        if !kept.contains_key(&object.key) {
            changes.push((ChangeAction::Delete, object.key.clone()));
        }
    }
    changes
}

#[cfg(test)]
mod tests {
    use super::*;

    const RENDERED: &str = "---
# Source: web-app/templates/web_config.yaml
apiVersion: v1
kind: ConfigMap
metadata:
  name: web-config
  labels:
    name: not-this-one
data:
  nginx.conf: |
    name: nor-this
    kind: Fake
---
# Source: web-app/templates/web.yaml
apiVersion: apps/v1
kind: Deployment
metadata:
  namespace: \"prod\"
  name: 'web' # the workload
spec:
  replicas: 2
  template:
    metadata:
      name: inner
---
";

    fn key(api_version: &str, kind: &str, namespace: &str, name: &str) -> Key {
        Key {
            api_version: api_version.into(),
            kind: kind.into(),
            namespace: namespace.into(),
            name: name.into(),
        }
    }

    #[test]
    fn objects_are_found_by_their_own_fields_only() {
        let found = objects(RENDERED);
        assert_eq!(
            found.iter().map(|o| o.key.clone()).collect::<Vec<_>>(),
            [
                key("v1", "ConfigMap", "", "web-config"),
                key("apps/v1", "Deployment", "prod", "web"),
            ]
        );
        assert!(found[0].text.starts_with("apiVersion: v1\nkind: ConfigMap"));
        assert!(found[0].text.ends_with("    kind: Fake"));
        assert!(!found[0].text.contains("# Source"));
    }

    #[test]
    fn empty_documents_and_comments_are_not_objects() {
        assert_eq!(objects(""), []);
        assert_eq!(objects("---\n# only a comment\n---\n\n"), []);
    }

    #[test]
    fn a_hook_is_recognised_by_its_annotation() {
        let hook = "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: migrate\n  annotations:\n    \"helm.sh/hook\": pre-upgrade\n";
        let weight_only = "apiVersion: batch/v1\nkind: Job\nmetadata:\n  name: migrate\n  annotations:\n    helm.sh/hook-weight: \"1\"\n";
        assert!(objects(hook)[0].hook);
        assert!(!objects(weight_only)[0].hook);
    }

    #[test]
    fn a_diff_finds_what_is_new_changed_and_gone() {
        let old = objects(RENDERED);
        let new = objects(
            "apiVersion: apps/v1\nkind: Deployment\nmetadata:\n  namespace: prod\n  name: web\nspec:\n  replicas: 3\n---\napiVersion: v1\nkind: Service\nmetadata:\n  name: web\n",
        );
        assert_eq!(
            diff(&old, &new),
            [
                (
                    ChangeAction::Update,
                    key("apps/v1", "Deployment", "prod", "web")
                ),
                (ChangeAction::Create, key("v1", "Service", "", "web")),
                (
                    ChangeAction::Delete,
                    key("v1", "ConfigMap", "", "web-config")
                ),
            ]
        );
        assert_eq!(diff(&old, &old), []);
    }

    #[test]
    fn the_same_object_from_two_commands_compares_equal() {
        let templated = "---\n# Source: web/templates/a.yaml\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a   \ndata:\n  k: v\n\n";
        let stored = "---\n# Source: web/templates/manifests.yaml\napiVersion: v1\nkind: ConfigMap\nmetadata:\n  name: a\ndata:\n  k: v\n";
        assert_eq!(diff(&objects(stored), &objects(templated)), []);
    }
}
