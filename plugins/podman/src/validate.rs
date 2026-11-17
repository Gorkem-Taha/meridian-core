//! Stage three: the constraints that are Quadlet's and systemd's, not
//! Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::{CompileError, Warning};

use crate::keys::{self, Lookup};
use crate::{refuse_at, Kind, Unit, Units, NAME};

/// The longest unit file stem accepted. systemd's limit for a whole unit name
/// is 255 bytes; this leaves room for the longest suffix Quadlet adds.
const MAX_ID: usize = 200;

/// A resource id that can be a unit file's stem: it starts with a letter or a
/// digit and holds only letters, digits, `_` and `-`. systemd would accept
/// more, but `@` makes a unit a template and a dot would end the name in a
/// `${id.name}` reference.
fn is_unit_stem(id: &str) -> bool {
    id.len() <= MAX_ID
        && id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The keys one of which a kind of unit cannot do without.
fn required(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Container => &["Image", "Rootfs"],
        Kind::Kube => &["Yaml"],
        Kind::Image => &["Image"],
        Kind::Build => &["ImageTag"],
        Kind::Pod | Kind::Volume | Kind::Network => &[],
    }
}

pub(crate) fn validate(units: &Units) -> Result<Vec<Warning>, CompileError> {
    let mut warnings = Vec::new();
    let mut services: BTreeMap<&str, &Unit> = BTreeMap::new();

    for unit in &units.units {
        let refuse = |rule: &str, msg: String| refuse_at(&unit.id, &unit.pos, rule, msg);

        if !is_unit_stem(&unit.id) {
            return Err(refuse(
                "invalid-name",
                format!(
                    "resource id {:?} cannot name a unit file: a name starts with a letter or \
                     a digit, holds only letters, digits, underscores and hyphens, and is at \
                     most {MAX_ID} characters long",
                    unit.id
                ),
            ));
        }

        if let Some(other) = services.insert(&unit.service, unit) {
            return Err(refuse(
                "duplicate-service",
                format!(
                    "{} and {} would both run as {}; rename one of the resources, or give one \
                     a ServiceName",
                    other.file(),
                    unit.file(),
                    unit.service
                ),
            ));
        }

        let needed = required(unit.kind);
        if !needed.is_empty() && !needed.iter().any(|key| unit.body.contains_key(*key)) {
            return Err(refuse(
                "missing-param",
                format!(
                    "resource type {:?} requires param {} on this target",
                    unit.kind.as_str(),
                    needed
                        .iter()
                        .map(|key| format!("{key:?}"))
                        .collect::<Vec<_>>()
                        .join(" or ")
                ),
            ));
        }
        if unit.kind == Kind::Build
            && !unit.body.contains_key("File")
            && !unit.body.contains_key("SetWorkingDirectory")
        {
            return Err(refuse(
                "missing-param",
                "resource type \"build\" requires param \"File\" or \"SetWorkingDirectory\" on \
                 this target, to say where the Containerfile is"
                    .to_owned(),
            ));
        }

        for key in unit.body.keys() {
            if keys::lookup(unit.kind, key) == Lookup::Unknown {
                warnings.push(Warning {
                    target: NAME.to_owned(),
                    resource: unit.id.clone(),
                    msg: format!(
                        "{key:?} is not a key this plugin knows for a {} unit. It is written \
                         as given; a Podman that does not know it either will refuse the unit \
                         when it is planned",
                        unit.kind.as_str()
                    ),
                });
            }
        }
    }
    Ok(warnings)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_unit_stem_is_what_systemd_and_quadlet_both_accept() {
        for (id, want) in [
            ("web", true),
            ("web-1", true),
            ("web_db-2", true),
            ("web.v2", false),
            ("9lives", true),
            ("", false),
            ("-web", false),
            ("web@1", false),
            ("web app", false),
            ("web/app", false),
            ("wéb", false),
        ] {
            assert_eq!(is_unit_stem(id), want, "{id:?}");
        }
        assert!(!is_unit_stem(&"a".repeat(MAX_ID + 1)));
    }
}
