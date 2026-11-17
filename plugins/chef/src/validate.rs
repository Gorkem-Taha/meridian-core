//! Stage three: the constraints that are Chef's and not Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::{CompileError, Value};

use crate::{refuse_at, Block, Cookbook, NAME};

/// The property Chef identifies the managed thing by, for the resources
/// where two blocks managing one thing would fight each other.
fn identity(block: &Block) -> Option<(&'static str, &Value)> {
    let property = match block.resource {
        "package" => "package_name",
        "service" => "service_name",
        "file" | "directory" => "path",
        "user" => "username",
        // A membership block shares its group with the block that creates it.
        "group" if !block.name.contains(':') => "group_name",
        _ => return None,
    };
    Some((property, block.properties.get(property)?))
}

pub(crate) fn validate(cookbook: &Cookbook) -> Result<(), CompileError> {
    if cookbook.blocks.is_empty() {
        return Err(CompileError {
            target: NAME.to_owned(),
            rule: "empty-cookbook".to_owned(),
            msg: "there is nothing to converge: the document declares no resources".to_owned(),
            ..CompileError::default()
        });
    }
    // Chef merges or warns about two blocks of one name; neither is what the
    // author of two resources meant.
    let mut seen: BTreeMap<(&str, String), &str> = BTreeMap::new();
    for block in &cookbook.blocks {
        let Some((property, value)) = identity(block) else {
            continue;
        };
        let kind = if block.resource == "directory" {
            "file"
        } else {
            block.resource
        };
        if let Some(previous) = seen.insert((kind, value.canonical()), &block.resource_id) {
            return Err(refuse_at(
                &block.resource_id,
                &block.pos,
                "duplicate-resource",
                format!(
                    "manages the same {kind} as resource {previous:?}: both set {property} to {}",
                    value.canonical()
                ),
            ));
        }
    }
    for block in &cookbook.blocks {
        if block.only_if.contains('\n') {
            return Err(refuse_at(
                &block.resource_id,
                &block.pos,
                "invalid-runtime-condition",
                "runtimeWhen must be one line of Ruby on this target".to_owned(),
            ));
        }
    }
    Ok(())
}
