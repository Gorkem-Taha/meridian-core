//! Stage four: the configuration becomes a Datum project of one node.
//!
//! `Invoke-DscRunner` takes a Datum project and compiles it before it runs
//! anything. Meridian has already done the resolving Datum exists for, so the
//! project it writes has one layer and one node, and Datum's compile of it
//! changes nothing.

use std::collections::BTreeMap;

use meridian_sdk::Value;
use meridian_yaml::Yaml;

use crate::Configuration;

/// The project's definition: where its one layer is, and the runner's
/// settings.
pub const DATUM_PATH: &str = "Datum.yml";

/// Where a node's resources are written.
pub fn node_path(node: &str) -> String {
    format!("Projects/Present/{node}.yml")
}

/// The configuration format this output is written to, which the runner
/// checks against the range it supports.
const CONFIGURATION_VERSION: f64 = 0.5;
/// The runner release this output is written for.
const RUNNER_VERSION: &str = "1.1.0";

fn yaml(value: &Value) -> Yaml {
    match value {
        Value::Null => Yaml::Null,
        Value::Bool(b) => Yaml::Bool(*b),
        Value::Int(i) => Yaml::Int(*i),
        Value::Uint(u) => Yaml::Uint(*u),
        Value::Float(f) => Yaml::Float(*f),
        Value::String(s) => Yaml::str(s),
        // A plugin is never sent a secret as a value; one would be written as its mask.
        Value::Secret(_) => Yaml::str(meridian_sdk::SENSITIVE),
        Value::List(items) => Yaml::Seq(items.iter().map(yaml).collect()),
        Value::Map(map) => Yaml::Map(map.iter().map(|(k, v)| (k.clone(), yaml(v))).collect()),
    }
}

fn map<const N: usize>(pairs: [(&str, Yaml); N]) -> Yaml {
    Yaml::Map(pairs.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

/// Renders the project. This stage performs no mapping and no ordering.
pub(crate) fn serialise(configuration: &Configuration) -> BTreeMap<String, String> {
    let settings = &configuration.settings;
    let datum = map([
        (
            "ResolutionPrecedence",
            Yaml::Seq(vec![Yaml::str(
                "Projects\\$($Node.ProjectPresence)\\$($Node.Project)",
            )]),
        ),
        ("DatumHandlersThrowOnError", Yaml::Bool(true)),
        ("default_lookup_options", Yaml::str("MostSpecific")),
        (
            "PipelineRunnerSettings",
            map([
                ("ConfigurationVersion", Yaml::Float(CONFIGURATION_VERSION)),
                ("PipelineRunnerVersion", Yaml::str(RUNNER_VERSION)),
                ("Engine", Yaml::str(&settings.engine)),
                // A resource that reads another uses the runner's function
                // language, not a script, so scripts stay off.
                ("AllowExecutionScripts", Yaml::Bool(false)),
                ("Target", Yaml::str(&settings.target)),
            ]),
        ),
        (
            "DatumHandlers",
            map([(
                "Datum.InvokeCommand::InvokeCommand",
                map([("SkipDuringLoad", Yaml::Bool(true))]),
            )]),
        ),
    ]);

    let identities = |ids: &[String]| {
        Yaml::Seq(
            ids.iter()
                .map(|id| Yaml::str(configuration.identity(id)))
                .collect(),
        )
    };
    let resources = configuration
        .instances
        .iter()
        .map(|instance| {
            let mut pairs = vec![
                ("name".to_owned(), Yaml::str(&instance.id)),
                ("type".to_owned(), Yaml::str(&instance.r#type)),
            ];
            if !instance.pre_condition.is_empty() {
                pairs.push((
                    "preCondition".to_owned(),
                    Yaml::str(instance.pre_condition.trim()),
                ));
            }
            if !instance.depends_on.is_empty() {
                pairs.push(("dependsOn".to_owned(), identities(&instance.depends_on)));
            }
            if !instance.notify.is_empty() {
                pairs.push(("notify".to_owned(), identities(&instance.notify)));
            }
            if settings.target != "Local" {
                let mut target = vec![
                    ("action".to_owned(), Yaml::str(&settings.target)),
                    ("computerName".to_owned(), Yaml::str(&configuration.host)),
                ];
                if !settings.credential.is_empty() {
                    target.push(("credentialRef".to_owned(), Yaml::str(&settings.credential)));
                }
                pairs.push(("target".to_owned(), Yaml::Map(target)));
            }
            pairs.push((
                "properties".to_owned(),
                yaml(&Value::Map(instance.properties.clone())),
            ));
            Yaml::Map(pairs)
        })
        .collect();
    let node = map([
        ("parameters", Yaml::Map(Vec::new())),
        ("variables", Yaml::Map(Vec::new())),
        ("resources", Yaml::Seq(resources)),
    ]);

    BTreeMap::from([
        (DATUM_PATH.to_owned(), meridian_yaml::write(&datum)),
        (node_path(&configuration.node), meridian_yaml::write(&node)),
    ])
}
