//! Stage three: the constraints that are Puppet's and not Meridian's.

use std::collections::{HashMap, HashSet};

use meridian_sdk::{CompileError, Value};

use crate::transform::mapping;
use crate::{refuse_declaration as fail, Attr, Declaration, Manifest, NAME};

/// Checks a manifest before anything is serialised. Every failure names the
/// offending resource and the rule it broke.
pub(crate) fn validate(manifest: &Manifest) -> Result<(), CompileError> {
    if manifest.host.is_empty() {
        return Err(CompileError {
            target: NAME.to_owned(),
            rule: "missing-host".to_owned(),
            msg: "manifest has no node to assign the class to".to_owned(),
            ..CompileError::default()
        });
    }

    // Puppet requires a title to be unique for its type across the whole
    // catalog. Meridian ids are already unique within a document, so this
    // catches a tree assembled some other way.
    let mut titles: HashMap<(&str, &str), &str> = HashMap::new();
    // The namevar check is the one that catches real documents. Two resources
    // with different ids can still name the same package or the same path,
    // and Puppet only discovers that at apply time, on the node.
    let mut names: HashMap<(&str, &str), &str> = HashMap::new();
    let declared: HashSet<&str> = manifest
        .resources
        .iter()
        .map(|d| d.resource_id.as_str())
        .collect();

    for d in &manifest.resources {
        check_title(d)?;
        if let Some(previous) = titles.insert((&d.r#type, &d.title), &d.resource_id) {
            return Err(fail(
                d,
                "duplicate-title",
                format!(
                    "a {} titled {:?} is already declared by resource {:?}; \
                     Puppet requires a title to be unique for its type across the catalog",
                    d.r#type, d.title, previous
                ),
            ));
        }

        if let Some(namevar) = namevar_of(d) {
            if let Some(previous) = names.insert((&d.r#type, namevar), &d.resource_id) {
                return Err(fail(
                    d,
                    "duplicate-namevar",
                    format!(
                        "this {} manages {:?}, which resource {:?} already manages; \
                         two declarations of one thing is a conflict Puppet reports on the \
                         node at apply time, not here, so it is refused now",
                        d.r#type, namevar, previous
                    ),
                ));
            }
        }

        if d.attrs.is_empty() {
            return Err(fail(
                d,
                "empty-resource",
                format!(
                    "{} resource {:?} was given no attributes",
                    d.r#type, d.title
                ),
            ));
        }

        for reference in d.require.iter().chain(&d.notify) {
            if !declared.contains(reference.title.as_str()) {
                return Err(fail(
                    d,
                    "dangling-reference",
                    format!("references {reference}, which this manifest does not declare"),
                ));
            }
        }
    }
    Ok(())
}

/// Rejects a title Puppet could not parse inside a declaration. A title is
/// written between single quotes, so a quote or a newline in one would change
/// the manifest's structure.
fn check_title(d: &Declaration) -> Result<(), CompileError> {
    if d.title.trim().is_empty() {
        return Err(fail(
            d,
            "empty-title",
            "resource has an empty title".to_owned(),
        ));
    }
    if d.title.contains(['\n', '\r']) {
        return Err(fail(
            d,
            "multiline-title",
            format!("resource title {:?} spans more than one line", d.title),
        ));
    }
    if d.title.contains('\'') {
        return Err(fail(
            d,
            "quoted-title",
            format!(
                "resource title {:?} contains a single quote, which would \
                 terminate the title Puppet reads",
                d.title
            ),
        ));
    }
    Ok(())
}

/// The value Puppet identifies the resource by, when it is a string.
fn namevar_of(d: &Declaration) -> Option<&str> {
    // The native type names are the IR type names for every type this target
    // maps, so the table is consulted by the declaration's own type.
    let attribute = mapping(&d.r#type)?.namevar;
    match d.attrs.get(attribute)? {
        Attr::Value(Value::String(s)) => Some(s),
        _ => None,
    }
}
