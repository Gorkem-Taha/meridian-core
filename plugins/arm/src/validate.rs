//! Stage three: the constraints that are Resource Manager's and not
//! Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::{CompileError, Position, Value, Warning};

use crate::expression;
use crate::{refuse_at, Scope, Template, NAME};

/// The types a template parameter can have.
pub(crate) const PARAMETER_TYPES: &[&str] = &[
    "array",
    "bool",
    "int",
    "object",
    "secureObject",
    "securestring",
    "string",
];

/// What a parameter's declaration may hold.
const PARAMETER_KEYS: &[&str] = &[
    "allowedValues",
    "defaultValue",
    "maxLength",
    "maxValue",
    "metadata",
    "minLength",
    "minValue",
    "nullable",
    "type",
];

/// Resource Manager's own limits on one template.
pub(crate) const MAX_RESOURCES: usize = 800;
pub(crate) const MAX_PARAMETERS: usize = 256;
pub(crate) const MAX_OUTPUTS: usize = 64;

/// Whether a parameter type holds a secret.
pub(crate) fn is_secure(parameter_type: &str) -> bool {
    parameter_type.eq_ignore_ascii_case("securestring")
        || parameter_type.eq_ignore_ascii_case("secureObject")
}

fn document(rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    }
}

/// Letters, digits and `-_.()`, which is what Azure allows in the name of a
/// deployment and of a resource group.
fn is_azure_name(name: &str, max: usize) -> bool {
    !name.is_empty()
        && name.chars().count() <= max
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || "-_.()".contains(c))
}

pub(crate) fn validate(template: &Template) -> Result<Vec<Warning>, CompileError> {
    let settings = &template.settings;
    if !is_azure_name(&settings.name, 64) {
        return Err(document(
            "invalid-deployment-name",
            format!(
                "{:?} cannot be a deployment name: one holds up to 64 letters, digits and \
                 the characters -_.(). Name the deployment with a resource of type \"deployment\"",
                settings.name
            ),
        ));
    }
    match settings.scope {
        Scope::ResourceGroup => {
            if settings.resource_group.is_empty() {
                return Err(document(
                    "missing-param",
                    "a deployment needs the resource group it deploys to: add a resource of \
                     type \"deployment\" with param \"resourceGroup\", or give it scope \
                     \"subscription\""
                        .to_owned(),
                ));
            }
            if !is_azure_name(&settings.resource_group, 90)
                || settings.resource_group.ends_with('.')
            {
                return Err(document(
                    "invalid-param",
                    format!(
                        "{:?} cannot be a resource group name: one holds up to 90 letters, \
                         digits and the characters -_.(), and does not end with a period",
                        settings.resource_group
                    ),
                ));
            }
            if !settings.location.is_empty() {
                return Err(document(
                    "invalid-param",
                    "a deployment to a resource group takes no \"location\": it is stored in \
                     the group's own. Give each resource its location instead"
                        .to_owned(),
                ));
            }
        }
        Scope::Subscription => {
            if settings.location.is_empty() {
                return Err(document(
                    "missing-param",
                    "a deployment to a subscription needs param \"location\": the region that \
                     stores the deployment's record"
                        .to_owned(),
                ));
            }
            if !settings.resource_group.is_empty() {
                return Err(document(
                    "invalid-param",
                    "a deployment to a subscription takes no \"resourceGroup\"".to_owned(),
                ));
            }
        }
    }
    match (settings.mode.as_str(), settings.scope) {
        ("Incremental", _) | ("Complete", Scope::ResourceGroup) => {}
        ("Complete", Scope::Subscription) => {
            return Err(document(
                "invalid-param",
                "mode \"Complete\" exists only for a deployment to a resource group".to_owned(),
            ))
        }
        (other, _) => {
            return Err(document(
                "invalid-param",
                format!("{other:?} is not a deployment mode (accepted: Incremental, Complete)"),
            ))
        }
    }

    if template.entries.is_empty() {
        return Err(document(
            "empty-template",
            "a template needs at least one resource".to_owned(),
        ));
    }
    for (what, count, max) in [
        ("resources", template.entries.len(), MAX_RESOURCES),
        ("parameters", template.parameters.len(), MAX_PARAMETERS),
        ("outputs", template.outputs.len(), MAX_OUTPUTS),
    ] {
        if count > max {
            return Err(document(
                "template-too-large",
                format!("the document has {count} {what}, and a template holds at most {max}"),
            ));
        }
    }

    let mut warnings = Vec::new();
    for parameter in &template.parameters {
        let refuse =
            |rule: &str, msg: String| refuse_at(&parameter.name, &parameter.pos, rule, msg);
        for key in parameter.definition.keys() {
            if !PARAMETER_KEYS.contains(&key.as_str()) {
                return Err(refuse(
                    "unsupported-param",
                    format!(
                        "resource type \"parameter\" does not accept param {key:?} on this \
                         target (accepted: {})",
                        PARAMETER_KEYS.join(", ")
                    ),
                ));
            }
        }
        let parameter_type = match parameter.definition.get("type") {
            Some(Value::String(t)) => t,
            Some(_) => {
                return Err(refuse(
                    "invalid-param",
                    "param \"type\" of a parameter must be text".to_owned(),
                ))
            }
            None => {
                return Err(refuse(
                    "missing-param",
                    format!(
                        "a parameter needs param \"type\" (accepted: {})",
                        PARAMETER_TYPES.join(", ")
                    ),
                ))
            }
        };
        if !PARAMETER_TYPES
            .iter()
            .any(|t| t.eq_ignore_ascii_case(parameter_type))
        {
            return Err(refuse(
                "invalid-param",
                format!(
                    "{parameter_type:?} is not a type a parameter can have (accepted: {})",
                    PARAMETER_TYPES.join(", ")
                ),
            ));
        }
        check_expressions(
            &Value::Map(parameter.definition.clone()),
            &parameter.name,
            &parameter.pos,
        )?;
        // A secret written into the template is readable by anyone who can
        // read the deployment's history.
        let written_secret = matches!(
            parameter.definition.get("defaultValue"),
            Some(Value::String(v)) if !v.is_empty() && expression::inner(v).is_none()
        ) || matches!(
            parameter.definition.get("defaultValue"),
            Some(Value::Map(_) | Value::List(_))
        );
        if is_secure(parameter_type) && written_secret {
            warnings.push(Warning {
                target: NAME.to_owned(),
                resource: parameter.name.clone(),
                msg: format!(
                    "the {parameter_type} parameter has its default written in the document, \
                     so the secret is stored in the template and in the deployment's history; \
                     pass it when deploying instead"
                ),
            });
        }
    }

    // Azure compares types and names without regard to case.
    let mut seen: BTreeMap<(String, String), &str> = BTreeMap::new();
    for entry in &template.entries {
        let key = (entry.r#type.to_lowercase(), entry.name.to_lowercase());
        // A condition or a copy loop is how one template declares the same
        // name twice on purpose.
        let deliberate = entry.body.contains_key("condition") || entry.body.contains_key("copy");
        if let Some(previous) = seen.insert(key, &entry.id) {
            if !deliberate {
                return Err(refuse_at(
                    &entry.id,
                    &entry.pos,
                    "duplicate-resource",
                    format!(
                        "resources {previous:?} and {:?} are both the {} named {:?}; a template \
                         declares a resource once",
                        entry.id, entry.r#type, entry.name
                    ),
                ));
            }
        }
        check_expressions(&Value::String(entry.name.clone()), &entry.id, &entry.pos)?;
        check_expressions(&Value::Map(entry.body.clone()), &entry.id, &entry.pos)?;
    }

    let mut names: BTreeMap<String, &str> = BTreeMap::new();
    for (name, value, pos) in &template.outputs {
        if let Some(previous) = names.insert(name.to_lowercase(), name) {
            return Err(refuse_at(
                "",
                pos,
                "duplicate-output",
                format!(
                    "outputs {previous:?} and {name:?} differ only in case, and Resource \
                     Manager does not tell them apart"
                ),
            ));
        }
        if let Some(inner) = expression::inner(value) {
            expression::check(inner).map_err(|why| {
                refuse_at(
                    "",
                    pos,
                    "invalid-expression",
                    format!(
                        "output {name:?}: {value:?} is not a well-formed ARM expression: {why}"
                    ),
                )
            })?;
        }
    }
    Ok(warnings)
}

/// Checks every expression anywhere inside a value.
fn check_expressions(value: &Value, resource: &str, pos: &Position) -> Result<(), CompileError> {
    match value {
        Value::String(text) => match expression::inner(text) {
            Some(inner) => expression::check(inner).map_err(|why| {
                refuse_at(
                    resource,
                    pos,
                    "invalid-expression",
                    format!("{text:?} is not a well-formed ARM expression: {why}"),
                )
            }),
            None => Ok(()),
        },
        Value::List(items) => items
            .iter()
            .try_for_each(|item| check_expressions(item, resource, pos)),
        Value::Map(map) => map
            .values()
            .try_for_each(|item| check_expressions(item, resource, pos)),
        _ => Ok(()),
    }
}
