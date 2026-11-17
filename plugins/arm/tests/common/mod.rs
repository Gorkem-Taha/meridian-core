//! A check that a compiled template has the shape Resource Manager's
//! deployment template schema describes. It is written out here and not
//! fetched, so the tests need no network.

use meridian_sdk::{decode_value, Value};
use meridian_target_arm::expression;

const SCHEMAS: &[&str] = &[
    "https://schema.management.azure.com/schemas/2019-04-01/deploymentTemplate.json#",
    "https://schema.management.azure.com/schemas/2018-05-01/subscriptionDeploymentTemplate.json#",
];
const ROOT_KEYS: &[&str] = &[
    "$schema",
    "contentVersion",
    "metadata",
    "outputs",
    "parameters",
    "resources",
];
const TYPES: &[&str] = &[
    "array",
    "bool",
    "int",
    "object",
    "secureObject",
    "securestring",
    "string",
];

fn text<'a>(value: &'a Value, key: &str, what: &str) -> &'a str {
    value
        .as_map()
        .and_then(|m| m.get(key))
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{what} needs a string {key:?}"))
}

/// Every string anywhere in a value, with a path for messages.
fn strings<'a>(value: &'a Value, path: String, out: &mut Vec<(String, &'a str)>) {
    match value {
        Value::String(s) => out.push((path, s)),
        Value::List(items) => {
            for (i, item) in items.iter().enumerate() {
                strings(item, format!("{path}[{i}]"), out);
            }
        }
        Value::Map(map) => {
            for (key, item) in map {
                strings(item, format!("{path}.{key}"), out);
            }
        }
        _ => {}
    }
}

/// Panics, saying what is wrong, unless `template` is a well-formed ARM
/// deployment template.
pub fn assert_valid_template(template: &str) {
    assert!(
        template.len() < 4 * 1024 * 1024,
        "a template is at most 4 MB"
    );
    let root = decode_value(template.as_bytes()).expect("the template is JSON");
    let map = root.as_map().expect("the template is an object");
    for key in map.keys() {
        assert!(ROOT_KEYS.contains(&key.as_str()), "unexpected key {key:?}");
    }
    assert!(SCHEMAS.contains(&text(&root, "$schema", "the template")));
    let version = text(&root, "contentVersion", "the template");
    assert!(
        version.split('.').count() == 4
            && version
                .split('.')
                .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit())),
        "contentVersion {version:?}"
    );

    for (name, parameter) in map
        .get("parameters")
        .and_then(Value::as_map)
        .into_iter()
        .flatten()
    {
        let t = text(parameter, "type", &format!("parameter {name}"));
        assert!(TYPES.contains(&t), "parameter {name} has type {t:?}");
    }
    for (name, output) in map
        .get("outputs")
        .and_then(Value::as_map)
        .into_iter()
        .flatten()
    {
        let t = text(output, "type", &format!("output {name}"));
        assert!(TYPES.contains(&t), "output {name} has type {t:?}");
        assert!(
            output.as_map().unwrap().contains_key("value"),
            "output {name} has no value"
        );
    }

    let resources = map
        .get("resources")
        .and_then(Value::as_list)
        .expect("resources is an array");
    assert!(!resources.is_empty(), "a template has resources");
    let types: Vec<&str> = resources
        .iter()
        .map(|r| text(r, "type", "a resource"))
        .collect();
    for resource in resources {
        let t = text(resource, "type", "a resource");
        let name = text(resource, "name", t);
        let api = text(resource, "apiVersion", t);
        assert!(t.contains('.') && t.contains('/'), "type {t:?}");
        assert!(
            api.len() >= 10 && api[..4].chars().all(|c| c.is_ascii_digit()),
            "{t} has apiVersion {api:?}"
        );
        if expression::inner(name).is_none() {
            assert_eq!(
                name.split('/').count(),
                t.split('/').count() - 1,
                "{t} is named {name:?}"
            );
        }
        let depends = resource.as_map().unwrap().get("dependsOn");
        for item in depends.and_then(Value::as_list).into_iter().flatten() {
            let item = item.as_str().expect("dependsOn holds strings");
            assert!(
                [
                    "[resourceId(",
                    "[subscriptionResourceId(",
                    "[extensionResourceId("
                ]
                .iter()
                .any(|start| item.starts_with(start)),
                "dependsOn {item:?} is not a resource id"
            );
            assert!(
                types.iter().any(|t| item.contains(&format!("'{t}'"))),
                "dependsOn {item:?} names no resource of the template"
            );
        }
    }

    let mut all = Vec::new();
    strings(&root, String::new(), &mut all);
    for (path, s) in all {
        if let Some(inner) = expression::inner(s) {
            if let Err(why) = expression::check(inner) {
                panic!("{path}: {s:?} is not a well-formed expression: {why}");
            }
        }
    }
}
