//! Stage four: a project becomes its files.

use std::collections::BTreeMap;

use meridian_sdk::Value;
use meridian_yaml::Yaml;

use crate::{Entry, Files, Kind, Project};

/// The Compose file itself.
pub const COMPOSE_PATH: &str = "compose.yaml";
/// How to bring it up: the project's name and settings, and the outputs the
/// document asked for. The runner reads it; Compose never does.
pub const PROJECT_PATH: &str = "project.json";

/// Renders the project. This stage performs no mapping and no ordering.
/// Elements are written in the document's apply order and attributes in
/// sorted order, so the same document always renders to the same bytes.
pub(crate) fn serialise(project: &Project) -> Files {
    let mut root = vec![("name".to_owned(), Yaml::str(&project.settings.name))];
    for kind in Kind::ALL {
        let section: Vec<(String, Yaml)> = project
            .of_kind(kind)
            .map(|entry| (entry.id.clone(), Yaml::Map(body(entry))))
            .collect();
        if !section.is_empty() {
            root.push((kind.section().to_owned(), Yaml::Map(section)));
        }
    }
    let compose = format!(
        "# Compiled by Meridian from the document {}. Do not edit.\n{}",
        project.document,
        meridian_yaml::write(&Yaml::Map(root))
    );

    let settings = &project.settings;
    let sidecar = Value::Map(BTreeMap::from([
        ("name".to_owned(), Value::String(settings.name.clone())),
        (
            "directory".to_owned(),
            Value::String(settings.directory.clone()),
        ),
        (
            "profiles".to_owned(),
            Value::List(
                settings
                    .profiles
                    .iter()
                    .cloned()
                    .map(Value::String)
                    .collect(),
            ),
        ),
        ("wait".to_owned(), Value::Bool(settings.wait)),
        (
            "wait_timeout".to_owned(),
            Value::Uint(settings.wait_timeout),
        ),
        (
            "outputs".to_owned(),
            Value::Map(
                project
                    .outputs
                    .iter()
                    .map(|(name, value)| (name.clone(), Value::String(value.clone())))
                    .collect(),
            ),
        ),
    ]));

    Files::from([
        (COMPOSE_PATH.to_owned(), compose),
        (PROJECT_PATH.to_owned(), sidecar.pretty()),
    ])
}

/// An element's attributes, with a service's `depends_on` in its sorted place.
fn body(entry: &Entry) -> Vec<(String, Yaml)> {
    let mut body = entry.body.clone();
    if !entry.depends_on.is_empty() {
        let depends_on = entry
            .depends_on
            .iter()
            .map(|dependency| {
                let mut fields = vec![("condition".to_owned(), Yaml::str(&dependency.condition))];
                if let Some(required) = dependency.required {
                    fields.push(("required".to_owned(), Yaml::Bool(required)));
                }
                if dependency.restart {
                    fields.push(("restart".to_owned(), Yaml::Bool(true)));
                }
                (dependency.service.clone(), Yaml::Map(fields))
            })
            .collect();
        let at = body.partition_point(|(key, _)| key.as_str() < "depends_on");
        body.insert(at, ("depends_on".to_owned(), Yaml::Map(depends_on)));
    }
    body
}
