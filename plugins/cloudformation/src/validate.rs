//! Stage three: the constraints that are CloudFormation's and not Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::CompileError;

use crate::{refuse_at, Template, NAME};

/// The capabilities a stack can acknowledge.
const CAPABILITIES: &[&str] = &[
    "CAPABILITY_AUTO_EXPAND",
    "CAPABILITY_IAM",
    "CAPABILITY_NAMED_IAM",
];

pub(crate) fn validate(template: &Template) -> Result<(), CompileError> {
    let document = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };

    let name = &template.settings.name;
    let valid = name.len() <= 128
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(document(
            "invalid-stack-name",
            format!(
                "{name:?} cannot be a stack name: one starts with a letter and holds only \
                 letters, digits and hyphens. Name the stack with a resource of type \"stack\""
            ),
        ));
    }
    for capability in &template.settings.capabilities {
        if !CAPABILITIES.contains(&capability.as_str()) {
            return Err(document(
                "invalid-param",
                format!(
                    "{capability:?} is not a capability a stack can acknowledge (accepted: {})",
                    CAPABILITIES.join(", ")
                ),
            ));
        }
    }
    if template.entries.is_empty() {
        return Err(document(
            "empty-template",
            "a template needs at least one resource".to_owned(),
        ));
    }

    // Two ids can differ and still become the same logical id: web_vm and
    // web-vm are both WebVm.
    let mut seen: BTreeMap<&str, &str> = BTreeMap::new();
    for entry in &template.entries {
        if let Some(previous) = seen.insert(&entry.logical, &entry.id) {
            return Err(refuse_at(
                &entry.id,
                &entry.pos,
                "duplicate-logical-id",
                format!(
                    "resource ids {previous:?} and {:?} both become the logical id {:?}; \
                     CloudFormation's logical ids hold only letters and digits",
                    entry.id, entry.logical
                ),
            ));
        }
    }
    let mut outputs: BTreeMap<&str, &str> = BTreeMap::new();
    for (name, logical, _, pos) in &template.outputs {
        if let Some(previous) = outputs.insert(logical, name) {
            return Err(refuse_at(
                "",
                pos,
                "duplicate-output",
                format!("outputs {previous:?} and {name:?} both become the logical id {logical:?}"),
            ));
        }
    }
    Ok(())
}
