//! Stage four: a template becomes its files.

use std::collections::BTreeMap;

use meridian_sdk::Value;

use crate::{Scope, Template};

/// The template itself.
pub const TEMPLATE_PATH: &str = "template.json";
/// How to deploy it: the deployment's name and settings, and which resource
/// of the document each Azure resource is. The runner reads it; Resource
/// Manager never does.
pub const DEPLOYMENT_PATH: &str = "deployment.json";

/// The schema a template for each scope declares.
pub(crate) fn schema(scope: Scope) -> &'static str {
    match scope {
        Scope::ResourceGroup => {
            "https://schema.management.azure.com/schemas/2019-04-01/deploymentTemplate.json#"
        }
        Scope::Subscription => {
            "https://schema.management.azure.com/schemas/2018-05-01/subscriptionDeploymentTemplate.json#"
        }
    }
}

fn text(s: &str) -> Value {
    Value::String(s.to_owned())
}

fn map<const N: usize>(entries: [(&str, Value); N]) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (key.to_owned(), value))
            .collect(),
    )
}

/// Renders the template. This stage performs no mapping and no ordering.
/// Resources are written in the document's apply order and keys in sorted
/// order, so the same document always renders to the same bytes.
pub(crate) fn serialise(template: &Template) -> BTreeMap<String, String> {
    let settings = &template.settings;
    let mut root = BTreeMap::from([
        ("$schema".to_owned(), text(schema(settings.scope))),
        ("contentVersion".to_owned(), text("1.0.0.0")),
        (
            "metadata".to_owned(),
            map([(
                "description",
                Value::String(format!(
                    "Compiled by Meridian from the document {}. Do not edit.",
                    template.document
                )),
            )]),
        ),
    ]);

    if !template.parameters.is_empty() {
        root.insert(
            "parameters".to_owned(),
            Value::Map(
                template
                    .parameters
                    .iter()
                    .map(|p| (p.name.clone(), Value::Map(p.definition.clone())))
                    .collect(),
            ),
        );
    }

    let resources = template
        .entries
        .iter()
        .map(|entry| {
            let mut body = entry.body.clone();
            body.insert("type".to_owned(), text(&entry.r#type));
            body.insert("apiVersion".to_owned(), text(&entry.api_version));
            body.insert("name".to_owned(), text(&entry.name));
            if !entry.depends_on.is_empty() {
                body.insert(
                    "dependsOn".to_owned(),
                    Value::List(entry.depends_on.iter().map(|d| text(d)).collect()),
                );
            }
            Value::Map(body)
        })
        .collect();
    root.insert("resources".to_owned(), Value::List(resources));

    if !template.outputs.is_empty() {
        root.insert(
            "outputs".to_owned(),
            Value::Map(
                template
                    .outputs
                    .iter()
                    .map(|(name, value, _)| {
                        (
                            name.clone(),
                            map([("type", text("string")), ("value", text(value))]),
                        )
                    })
                    .collect(),
            ),
        );
    }

    let deployment = map([
        ("name", text(&settings.name)),
        ("scope", text(settings.scope.as_str())),
        ("resourceGroup", text(&settings.resource_group)),
        ("location", text(&settings.location)),
        ("subscription", text(&settings.subscription)),
        ("mode", text(&settings.mode)),
        (
            "parameters",
            Value::Map(
                template
                    .parameters
                    .iter()
                    .filter_map(|p| Some((p.name.clone(), p.definition.get("type")?.clone())))
                    .collect(),
            ),
        ),
        (
            "resources",
            Value::Map(
                template
                    .entries
                    .iter()
                    .map(|e| {
                        (
                            e.id.clone(),
                            map([("type", text(&e.r#type)), ("name", text(&e.name))]),
                        )
                    })
                    .collect(),
            ),
        ),
    ]);

    BTreeMap::from([
        (TEMPLATE_PATH.to_owned(), Value::Map(root).pretty()),
        (DEPLOYMENT_PATH.to_owned(), deployment.pretty()),
    ])
}
