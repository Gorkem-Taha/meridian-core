//! Stage three: the constraints that are Terraform's and not Meridian's.

use std::collections::BTreeSet;

use meridian_sdk::{CompileError, Value};

use crate::{refuse_at, Config, Kind};

/// A name Terraform accepts for a block or an output: a letter or underscore,
/// then letters, digits, underscores and hyphens.
fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

pub(crate) fn validate(config: &Config) -> Result<(), CompileError> {
    let mut providers = BTreeSet::new();
    let mut backends = 0;
    for block in &config.blocks {
        if !is_identifier(&block.name) {
            return Err(refuse_at(
                &block.name,
                &block.pos,
                "invalid-name",
                format!(
                    "resource id {:?} cannot be a Terraform name: a name starts with a letter \
                     or underscore and holds only letters, digits, underscores and hyphens",
                    block.name
                ),
            ));
        }
        match block.kind {
            Kind::Provider => {
                let alias = match block.body.get("alias") {
                    None => String::new(),
                    Some(Value::String(alias)) => alias.clone(),
                    Some(_) => {
                        return Err(refuse_at(
                            &block.name,
                            &block.pos,
                            "invalid-param",
                            "a provider's alias must be a string".to_owned(),
                        ))
                    }
                };
                if !providers.insert((block.r#type.clone(), alias.clone())) {
                    let msg = if alias.is_empty() {
                        format!(
                            "provider {:?} is configured twice; give all but one an alias",
                            block.r#type
                        )
                    } else {
                        format!(
                            "provider {:?} has two configurations with the alias {alias:?}",
                            block.r#type
                        )
                    };
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "duplicate-provider",
                        msg,
                    ));
                }
            }
            Kind::Backend => {
                backends += 1;
                if backends > 1 {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "duplicate-backend",
                        "a configuration stores its state in one backend; this document declares two"
                            .to_owned(),
                    ));
                }
            }
            Kind::Module => {
                if !matches!(block.body.get("source"), Some(Value::String(_))) {
                    return Err(refuse_at(
                        &block.name,
                        &block.pos,
                        "missing-param",
                        "resource type \"module\" requires param \"source\" on this target"
                            .to_owned(),
                    ));
                }
            }
            Kind::Data | Kind::Managed => {}
        }
    }

    let mut outputs = BTreeSet::new();
    for (name, _, pos) in &config.outputs {
        if !is_identifier(name) {
            return Err(refuse_at(
                "",
                pos,
                "invalid-name",
                format!("output name {name:?} cannot be a Terraform name"),
            ));
        }
        if !outputs.insert(name) {
            return Err(refuse_at(
                "",
                pos,
                "duplicate-output",
                format!("output {name:?} is declared twice"),
            ));
        }
    }
    Ok(())
}
