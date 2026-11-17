//! Stage three: the constraints that are Packer's and not Meridian's.

use std::collections::BTreeSet;

use meridian_sdk::{CompileError, Value};

use crate::{refuse_at, Block, Kind, Template};

/// A name Packer accepts for a block: a letter or underscore, then letters,
/// digits, underscores and hyphens.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn require_string(block: &Block, what: &str, param: &str) -> Result<(), CompileError> {
    if matches!(block.body.get(param), Some(Value::String(s)) if !s.is_empty()) {
        return Ok(());
    }
    Err(refuse_at(
        &block.name,
        &block.pos,
        "missing-param",
        format!("resource type {what:?} requires the string param {param:?} on this target"),
    ))
}

pub(crate) fn validate(template: &Template) -> Result<(), CompileError> {
    let mut cleanup: Option<&Block> = None;
    for block in &template.blocks {
        if !is_identifier(&block.name) {
            return Err(refuse_at(
                &block.name,
                &block.pos,
                "invalid-name",
                format!(
                    "resource id {:?} cannot be a Packer name: a name starts with a letter or \
                     underscore and holds only letters, digits, underscores and hyphens",
                    block.name
                ),
            ));
        }
        match block.kind {
            Kind::Plugin => {
                require_string(block, "plugin", "source")?;
                require_string(block, "plugin", "version")?;
                if let Some(extra) = block
                    .body
                    .keys()
                    .find(|k| !matches!(k.as_str(), "source" | "version"))
                {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "invalid-param",
                        format!(
                            "a plugin requirement takes the params \"source\" and \"version\" \
                             only, not {extra:?}"
                        ),
                    ));
                }
            }
            Kind::Local => {
                if !block.body.contains_key("value") {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "missing-param",
                        "resource type \"local\" requires param \"value\" on this target"
                            .to_owned(),
                    ));
                }
                if let Some(extra) = block.body.keys().find(|k| k.as_str() != "value") {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "invalid-param",
                        format!("a local takes the param \"value\" only, not {extra:?}"),
                    ));
                }
            }
            Kind::ErrorCleanup => {
                if let Some(first) = cleanup {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "duplicate-error-cleanup",
                        format!(
                            "a build has one error-cleanup provisioner, and {:?} is already it",
                            first.name
                        ),
                    ));
                }
                cleanup = Some(block);
            }
            Kind::Variable
            | Kind::Data
            | Kind::Source
            | Kind::Provisioner
            | Kind::PostProcessor => {}
        }
    }

    if template.of_kind(Kind::Source).next().is_none() {
        return Err(refuse_at(
            "",
            &template
                .blocks
                .first()
                .map(|b| b.pos.clone())
                .unwrap_or_default(),
            "missing-source",
            "a build needs at least one source to build from: add a resource of type \
             source.<builder>, such as source.amazon-ebs or source.docker"
                .to_owned(),
        ));
    }

    let mut outputs = BTreeSet::new();
    for output in &template.outputs {
        if output.name.is_empty() {
            return Err(refuse_at(
                "",
                &output.pos,
                "invalid-name",
                "an output needs a name".to_owned(),
            ));
        }
        if !outputs.insert(&output.name) {
            return Err(refuse_at(
                "",
                &output.pos,
                "duplicate-output",
                format!("output {:?} is declared twice", output.name),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packer_name_starts_with_a_letter_or_underscore() {
        for (name, want) in [
            ("ubuntu", true),
            ("_x", true),
            ("ubuntu-22_04", true),
            ("22-04", false),
            ("", false),
            ("a.b", false),
            ("a b", false),
        ] {
            assert_eq!(is_identifier(name), want, "{name}");
        }
    }
}
