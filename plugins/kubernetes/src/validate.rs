//! Stage three: the constraints that are Kubernetes' and not Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::CompileError;

use crate::{refuse_at, Manifests, NAME};

/// A DNS subdomain name: lower-case letters, digits, hyphens and periods,
/// starting and ending with a letter or digit, at most 253 characters.
fn is_object_name(name: &str) -> bool {
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty()
        && name.len() <= 253
        && name.starts_with(edge)
        && name.ends_with(edge)
        && name.chars().all(|c| edge(c) || c == '-' || c == '.')
}

pub(crate) fn validate(manifests: &Manifests) -> Result<(), CompileError> {
    if manifests.objects.is_empty() {
        return Err(CompileError {
            target: NAME.to_owned(),
            rule: "empty-manifests".to_owned(),
            msg: "there is nothing to apply: the document declares no objects".to_owned(),
            ..CompileError::default()
        });
    }
    let mut seen: BTreeMap<(String, &str, &str), &str> = BTreeMap::new();
    for object in &manifests.objects {
        if !is_object_name(&object.name) {
            return Err(refuse_at(
                &object.id,
                &object.pos,
                "invalid-name",
                format!(
                    "{:?} cannot be the name of a Kubernetes object: a name holds only \
                     lower-case letters, digits, hyphens and periods, and starts and ends with \
                     a letter or digit. Set metadata.name",
                    object.name
                ),
            ));
        }
        let key = (
            object.get_as(),
            object.namespace.as_str(),
            object.name.as_str(),
        );
        if let Some(previous) = seen.insert(key, &object.id) {
            return Err(refuse_at(
                &object.id,
                &object.pos,
                "duplicate-object",
                format!(
                    "this {} is named {:?}, which resource {previous:?} already is; the second \
                     would overwrite the first",
                    object.kind, object.name
                ),
            ));
        }
    }
    let mut outputs = BTreeMap::new();
    for (name, _) in &manifests.outputs {
        if outputs.insert(name, ()).is_some() {
            return Err(CompileError {
                target: NAME.to_owned(),
                rule: "duplicate-output".to_owned(),
                msg: format!("output {name:?} is declared twice"),
                ..CompileError::default()
            });
        }
    }
    Ok(())
}
