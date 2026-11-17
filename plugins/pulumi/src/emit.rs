//! Stage four: a program becomes its files.

use std::collections::BTreeMap;

use meridian_sdk::Value;
use meridian_yaml::Yaml;

use crate::{Entry, Kind, Program};

/// The program, which is also Pulumi's project file.
pub const PROGRAM_PATH: &str = "Pulumi.yaml";
/// How to deploy it: the stack's name and where its state lives. The runner
/// reads it; Pulumi never does.
pub const STACK_PATH: &str = "stack.json";

/// A document value as YAML. Maps are written in sorted order.
fn yaml(value: &Value) -> Yaml {
    match value {
        Value::Null => Yaml::Null,
        Value::Bool(b) => Yaml::Bool(*b),
        Value::Int(n) => Yaml::Int(*n),
        Value::Uint(n) => Yaml::Uint(*n),
        Value::Float(f) => Yaml::Float(*f),
        Value::String(s) => Yaml::str(s),
        Value::List(items) => Yaml::Seq(items.iter().map(yaml).collect()),
        Value::Map(map) => yaml_map(map),
    }
}

fn yaml_map(map: &BTreeMap<String, Value>) -> Yaml {
    Yaml::Map(map.iter().map(|(k, v)| (k.clone(), yaml(v))).collect())
}

fn pair(key: &str, value: Yaml) -> (String, Yaml) {
    (key.to_owned(), value)
}

/// A resource: its type, then its properties, then its options.
fn resource(entry: &Entry) -> Yaml {
    let mut body = vec![pair("type", Yaml::str(&entry.token))];
    if !entry.body.is_empty() {
        body.push(pair("properties", yaml_map(&entry.body)));
    }
    if !entry.options.is_empty() {
        body.push(pair("options", yaml_map(&entry.options)));
    }
    Yaml::Map(body)
}

/// A function call, as the variable that holds its result.
fn invoke(entry: &Entry) -> Yaml {
    let mut call = vec![pair("function", Yaml::str(&entry.token))];
    if !entry.body.is_empty() {
        call.push(pair("arguments", yaml_map(&entry.body)));
    }
    if !entry.options.is_empty() {
        call.push(pair("options", yaml_map(&entry.options)));
    }
    Yaml::Map(vec![pair("fn::invoke", Yaml::Map(call))])
}

/// Renders the program. This stage performs no mapping and no ordering.
/// Entries are written in the document's order and keys in sorted order, so
/// the same document always renders to the same bytes.
pub(crate) fn serialise(program: &Program) -> BTreeMap<String, String> {
    let of_kind = |kind: Kind, render: fn(&Entry) -> Yaml| -> Vec<(String, Yaml)> {
        program
            .entries
            .iter()
            .filter(|e| e.kind == kind)
            .map(|e| (e.name.clone(), render(e)))
            .collect()
    };

    let mut root = vec![
        pair("name", Yaml::str(&program.project)),
        pair("runtime", Yaml::str("yaml")),
        pair(
            "description",
            Yaml::str(format!(
                "Compiled by Meridian from the document {}. Do not edit.",
                program.project
            )),
        ),
    ];
    if !program.settings.backend.is_empty() {
        root.push(pair(
            "backend",
            Yaml::Map(vec![pair("url", Yaml::str(&program.settings.backend))]),
        ));
    }
    for (section, kind, render) in [
        (
            "config",
            Kind::Config,
            (|e| yaml_map(&e.body)) as fn(&Entry) -> Yaml,
        ),
        ("variables", Kind::Invoke, invoke),
        ("resources", Kind::Managed, resource),
    ] {
        let entries = of_kind(kind, render);
        if !entries.is_empty() {
            root.push(pair(section, Yaml::Map(entries)));
        }
    }
    if !program.outputs.is_empty() {
        root.push(pair(
            "outputs",
            Yaml::Map(
                program
                    .outputs
                    .iter()
                    .map(|(name, value, _)| (name.clone(), yaml(value)))
                    .collect(),
            ),
        ));
    }

    let stack = Value::Map(BTreeMap::from([
        ("project".to_owned(), Value::String(program.project.clone())),
        (
            "stack".to_owned(),
            Value::String(program.settings.stack.clone()),
        ),
        (
            "backend".to_owned(),
            Value::String(program.settings.backend.clone()),
        ),
        (
            "secretsProvider".to_owned(),
            Value::String(program.settings.secrets_provider.clone()),
        ),
    ]));

    BTreeMap::from([
        (
            PROGRAM_PATH.to_owned(),
            meridian_yaml::write(&Yaml::Map(root)),
        ),
        (STACK_PATH.to_owned(), stack.pretty()),
    ])
}
