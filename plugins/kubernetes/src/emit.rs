//! Stage four: the objects become their files.

use std::collections::BTreeMap;

use meridian_sdk::Value;
use meridian_yaml::Yaml;

use crate::{Manifests, OutputPart};

/// Every object, one YAML document each, in apply order.
pub const MANIFESTS_PATH: &str = "manifests.yaml";
/// Which cluster they go to, what `kubectl` will call each object, and how to
/// read each output. The runner reads it; Kubernetes never does.
pub const CLUSTER_PATH: &str = "cluster.json";

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

/// A map with the keys a reader looks for first moved to the front.
fn leading(map: &BTreeMap<String, Value>, first: &[&str]) -> Yaml {
    let mut pairs = Vec::with_capacity(map.len());
    for key in first {
        if let Some(value) = map.get(*key) {
            pairs.push(((*key).to_owned(), yaml(value)));
        }
    }
    for (key, value) in map {
        if !first.contains(&key.as_str()) {
            pairs.push((key.clone(), yaml(value)));
        }
    }
    Yaml::Map(pairs)
}

fn string(text: &str) -> Value {
    Value::String(text.to_owned())
}

/// Renders the manifests. This stage performs no mapping and no ordering.
/// Each object is laid out the way Kubernetes' own documentation does:
/// `apiVersion`, `kind`, `metadata`, then the rest.
pub(crate) fn serialise(manifests: &Manifests) -> BTreeMap<String, String> {
    let mut text = String::new();
    for object in &manifests.objects {
        let mut pairs = vec![
            ("apiVersion".to_owned(), Yaml::str(&object.api_version)),
            ("kind".to_owned(), Yaml::str(&object.kind)),
        ];
        if let Some(Value::Map(metadata)) = object.body.get("metadata") {
            pairs.push((
                "metadata".to_owned(),
                leading(metadata, &["name", "namespace"]),
            ));
        }
        for (key, value) in &object.body {
            if key != "metadata" {
                pairs.push((key.clone(), yaml(value)));
            }
        }
        text.push_str("---\n");
        text.push_str(&meridian_yaml::write(&Yaml::Map(pairs)));
    }

    let outputs = manifests
        .outputs
        .iter()
        .map(|(name, parts)| {
            let parts = parts
                .iter()
                .map(|part| match part {
                    OutputPart::Text(literal) => {
                        Value::Map(BTreeMap::from([("text".to_owned(), string(literal))]))
                    }
                    OutputPart::Read { object, path } => {
                        let object = &manifests.objects[*object];
                        Value::Map(BTreeMap::from([
                            ("get".to_owned(), string(&object.get_as())),
                            ("name".to_owned(), string(&object.name)),
                            ("namespace".to_owned(), string(&object.namespace)),
                            ("path".to_owned(), string(path)),
                        ]))
                    }
                })
                .collect();
            Value::Map(BTreeMap::from([
                ("name".to_owned(), string(name)),
                ("parts".to_owned(), Value::List(parts)),
            ]))
        })
        .collect();
    let cluster = Value::Map(BTreeMap::from([
        ("context".to_owned(), string(&manifests.cluster.context)),
        ("namespace".to_owned(), string(&manifests.cluster.namespace)),
        (
            "kubeconfig".to_owned(),
            string(&manifests.cluster.kubeconfig),
        ),
        (
            "objects".to_owned(),
            Value::Map(
                manifests
                    .objects
                    .iter()
                    .map(|o| (o.applied_as(), string(&o.id)))
                    .collect(),
            ),
        ),
        ("outputs".to_owned(), Value::List(outputs)),
    ]));

    BTreeMap::from([
        (MANIFESTS_PATH.to_owned(), text),
        (CLUSTER_PATH.to_owned(), cluster.pretty()),
    ])
}
