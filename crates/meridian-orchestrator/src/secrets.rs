//! Fetching a stage's secrets, just before the stage needs them, and what a
//! stage may do with them afterwards.

use std::collections::BTreeMap;
use std::path::Path;

use meridian_ir::{Document, Secret, Secrets};
use meridian_sdk::Value;
use meridian_secret::{from_env, from_exec, from_file, FetchError, FileOptions, SecretTable};

/// Fetches every secret the document declares into `secrets`.
///
/// An error names the secret and its provider and says what the provider
/// said, which is never any part of a value. An optional secret that is not
/// found reads as null.
pub(crate) fn fetch(doc: &Document, secrets: &mut Secrets) -> Result<(), String> {
    for secret in doc.secrets() {
        match fetch_one(secret) {
            Ok(value) => secrets.hold(&secret.name, value),
            Err(FetchError::Missing(_)) if secret.optional => secrets.absent(&secret.name),
            Err(e) => {
                return Err(format!(
                    "secret {:?} (from: {}): {e}",
                    secret.name, secret.from
                ))
            }
        }
    }
    Ok(())
}

fn fetch_one(secret: &Secret) -> Result<meridian_secret::SecretValue, FetchError> {
    match secret.from.as_str() {
        "env" => from_env(&secret.key),
        "file" => from_file(
            Path::new(&secret.path),
            FileOptions {
                allow_wide_mode: secret.allow_wide_mode,
                keep_trailing_newline: secret.keep_trailing_newline,
            },
        ),
        "exec" => from_exec(&secret.command),
        // `validate` refuses any other provider before a run starts.
        other => Err(FetchError::Failed(format!(
            "{other:?} is not a provider this version has"
        ))),
    }
}

/// Where the first secret in a document's params is: the resource and the
/// param, as a reader would name them.
pub(crate) fn first_in_params(doc: &Document) -> Option<(String, String)> {
    doc.resources().iter().find_map(|r| {
        r.params.iter().find_map(|(name, value)| {
            path_to_secret(value, &format!("params.{name}")).map(|at| (r.id.clone(), at))
        })
    })
}

fn path_to_secret(value: &Value, path: &str) -> Option<String> {
    match value {
        Value::Secret(_) => Some(path.to_owned()),
        Value::List(items) => items
            .iter()
            .enumerate()
            .find_map(|(i, item)| path_to_secret(item, &format!("{path}[{i}]"))),
        Value::Map(entries) => entries
            .iter()
            .find_map(|(key, item)| path_to_secret(item, &format!("{path}.{key}"))),
        _ => None,
    }
}

/// Writes each secret in a document's params as what stands for it: the name
/// of one that is not fetched, as `(secret name)`, with the text around it.
/// This is what `compile` writes, since it never fetches a secret.
pub(crate) fn describe_in_params(doc: &mut Document, table: &SecretTable) {
    let resources = match doc {
        Document::Infrastructure(d) => &mut d.spec.resources,
        Document::ResourceSet(d) => &mut d.spec.resources,
    };
    for r in resources {
        let params = std::mem::take(&mut r.params);
        r.params = params
            .into_iter()
            .map(|(name, value)| (name, described(value, table)))
            .collect::<BTreeMap<_, _>>();
    }
}

fn described(value: Value, table: &SecretTable) -> Value {
    match value {
        Value::Secret(at) => Value::String(table.describe(at)),
        Value::List(items) => Value::List(items.into_iter().map(|v| described(v, table)).collect()),
        Value::Map(entries) => Value::Map(
            entries
                .into_iter()
                .map(|(k, v)| (k, described(v, table)))
                .collect(),
        ),
        other => other,
    }
}
