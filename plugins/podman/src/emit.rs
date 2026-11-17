//! Stage four: units become their files.

use std::collections::BTreeMap;

use meridian_sdk::Value;

use crate::{OutputPart, Unit, Units};

/// The file that tells the runner what the unit files are: which service each
/// one becomes, and what the document's outputs are made of. It is the only
/// file of the artifact that is not installed on the host.
pub const META_PATH: &str = "meridian-podman.json";

/// The order sections are written in. The unit's own section comes second,
/// where a person reading the file looks for it.
const BEFORE: &[&str] = &["Unit"];
const AFTER: &[&str] = &["Service", "Quadlet", "Install"];

fn write_section(out: &mut String, name: &str, keys: &BTreeMap<String, Vec<String>>) {
    if !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(&format!("[{name}]\n"));
    for (key, values) in keys {
        for value in values {
            out.push_str(&format!("{key}={value}\n"));
        }
    }
}

fn unit_file(document: &str, unit: &Unit) -> String {
    let mut out = format!(
        "# Written by Meridian from the document \"{}\". The next apply overwrites \
         any change made here.\n",
        document.replace(['\n', '\r'], " ")
    );
    for name in BEFORE {
        if let Some(keys) = unit.sections.get(name).filter(|k| !k.is_empty()) {
            write_section(&mut out, name, keys);
        }
    }
    // The unit's own section is written even when it is empty: a volume with
    // no settings is still a volume, and Quadlet wants to see the section.
    write_section(&mut out, unit.kind.section(), &unit.body);
    for name in AFTER {
        if let Some(keys) = unit.sections.get(name).filter(|k| !k.is_empty()) {
            write_section(&mut out, name, keys);
        }
    }
    out
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

fn meta(units: &Units) -> String {
    let listed = units
        .units
        .iter()
        .map(|unit| {
            map([
                ("file", text(&unit.file())),
                ("id", text(&unit.id)),
                ("kind", text(unit.kind.as_str())),
                ("service", text(&unit.service)),
            ])
        })
        .collect();
    let outputs = units
        .outputs
        .iter()
        .map(|output| {
            let parts = output
                .parts
                .iter()
                .map(|part| match part {
                    OutputPart::Text(literal) => text(literal),
                    OutputPart::Inspect { noun, name, path } => map([
                        ("inspect", text(noun)),
                        ("name", text(name)),
                        ("path", text(path)),
                    ]),
                })
                .collect();
            map([("name", text(&output.name)), ("value", Value::List(parts))])
        })
        .collect();
    map([
        ("document", text(&units.document)),
        ("outputs", Value::List(outputs)),
        ("units", Value::List(listed)),
    ])
    .pretty()
}

/// Renders the units. This stage performs no mapping and no ordering. Keys
/// are sorted within each section, so the same document always renders to the
/// same bytes.
pub(crate) fn serialise(units: &Units) -> BTreeMap<String, String> {
    let mut files = BTreeMap::from([(META_PATH.to_owned(), meta(units))]);
    for unit in &units.units {
        files.insert(unit.file(), unit_file(&units.document, unit));
    }
    files
}
