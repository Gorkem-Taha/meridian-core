//! Stage three: the constraints that are Bicep's and not Meridian's.

use std::collections::BTreeSet;

use meridian_sdk::CompileError;

use crate::{is_identifier, refuse_at, Template, NAME};

pub(crate) fn validate(template: &Template) -> Result<(), CompileError> {
    let document = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };

    if template.deployment.resource_group.is_empty() {
        return Err(document(
            "missing-deployment",
            "the document does not say which resource group to deploy to. Add a resource of \
             type \"deployment\" with a resourceGroup param"
                .to_owned(),
        ));
    }
    let name = &template.deployment.name;
    let valid = !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.()".contains(c));
    if !valid {
        return Err(document(
            "invalid-deployment-name",
            format!(
                "{name:?} cannot be a deployment name: one holds only letters, digits, hyphens, \
                 underscores, periods and parentheses"
            ),
        ));
    }
    if template.statements.is_empty() {
        return Err(document(
            "empty-template",
            "a Bicep file needs at least one resource".to_owned(),
        ));
    }

    for statement in &template.statements {
        if !is_identifier(&statement.symbol) {
            return Err(refuse_at(
                &statement.symbol,
                &statement.pos,
                "invalid-name",
                format!(
                    "resource id {:?} cannot be a Bicep symbolic name: one starts with a letter \
                     or underscore and holds only letters, digits and underscores",
                    statement.symbol
                ),
            ));
        }
        if !statement.body.iter().any(|(key, _)| key == "name") {
            return Err(refuse_at(
                &statement.symbol,
                &statement.pos,
                "missing-param",
                format!(
                    "resource type {:?} requires param \"name\" on this target: the resource's \
                     name in Azure",
                    statement.r#type
                ),
            ));
        }
    }

    let mut outputs = BTreeSet::new();
    for (name, _, pos) in &template.outputs {
        if !is_identifier(name) {
            return Err(refuse_at(
                "",
                pos,
                "invalid-name",
                format!("output name {name:?} cannot be a Bicep name"),
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
        if template.statements.iter().any(|s| &s.symbol == name) {
            return Err(refuse_at(
                "",
                pos,
                "duplicate-name",
                format!(
                    "output {name:?} has the same name as a resource; Bicep keeps both in one namespace"
                ),
            ));
        }
    }
    Ok(())
}
