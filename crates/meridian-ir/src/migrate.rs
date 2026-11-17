//! Moving documents from one language version to the next.
//!
//! A migration is a chain of steps, one per version boundary. A step either
//! rewrites a file or reports the decisions only a person can make; it never
//! guesses, because an approximate rewrite of production configuration is
//! worse than an honest refusal.
//!
//! Files are rewritten as text, so comments, ordering and layout survive.

use crate::api::{api_version_name, VersionTable};

/// What a step found it could not decide for the author, one line each.
pub type Decisions = Vec<String>;

/// The rewrite across one version boundary.
#[derive(Clone, Copy)]
pub struct Step {
    /// The version the step reads.
    pub from: u32,
    /// Rewrites the text of one file, which may hold several documents, in
    /// every way the boundary needs apart from the `apiVersion` itself.
    pub rewrite: fn(&str) -> Result<String, Decisions>,
}

/// The steps this build ships. `meridian/v1` is the only version, so there is
/// no boundary to cross yet; the release that adds `v2` adds its step here.
pub const STEPS: &[Step] = &[];

/// The result of migrating one file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Migrated {
    /// Every document was already at the version asked for.
    Unchanged,
    /// The file's new text.
    Rewritten(String),
}

/// Why a file was not migrated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MigrateError {
    /// The request itself cannot be met, whatever the file holds.
    Refused(String),
    /// A step needs the author to decide something first.
    NeedsDecisions(Decisions),
}

/// Where in a file an `apiVersion` value was found.
struct Found {
    /// Byte range of the value, without quotes.
    start: usize,
    end: usize,
}

/// Finds every top-level `apiVersion` in a YAML or JSON file. Only a key at
/// the start of a line counts in YAML, so a Kubernetes object's `apiVersion`
/// inside a resource's params is never touched.
fn find_api_versions(text: &str) -> Vec<Found> {
    let json = matches!(text.trim_start().as_bytes().first(), Some(b'{' | b'['));
    let mut found = Vec::new();
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let line_start = offset;
        offset += line.len();
        let (key, indent_ok) = if json {
            ("\"apiVersion\"", true)
        } else {
            ("apiVersion", !line.starts_with([' ', '\t']))
        };
        let trimmed = line.trim_start();
        if !indent_ok || !trimmed.starts_with(key) {
            continue;
        }
        let after_key = line.len() - trimmed.len() + key.len();
        let rest = &line[after_key..];
        let Some(colon) = rest.find(':') else {
            continue;
        };
        if !rest[..colon].trim().is_empty() {
            continue;
        }
        let value = &rest[colon + 1..];
        let lead = value.len() - value.trim_start().len();
        let value = value.trim_start();
        let (open, body) = match value.as_bytes().first() {
            Some(b'"') => (1, &value[1..]),
            Some(b'\'') => (1, &value[1..]),
            _ => (0, value),
        };
        let len = body
            .find(|c: char| c == '"' || c == '\'' || c == ',' || c == '#' || c.is_whitespace())
            .unwrap_or(body.len());
        if len == 0 {
            continue;
        }
        let start = line_start + after_key + colon + 1 + lead + open;
        found.push(Found {
            start,
            end: start + len,
        });
    }
    found
}

/// Migrates the text of one file to language version `to`.
///
/// Every document in the file must be at or below `to`: a migration never
/// goes backward, because that would mean discarding what the newer version
/// said.
pub fn migrate(
    table: &VersionTable,
    steps: &[Step],
    text: &str,
    to: u32,
) -> Result<Migrated, MigrateError> {
    let refuse = |msg: String| Err(MigrateError::Refused(msg));
    match table.check(&api_version_name(to)) {
        Ok(_) => {}
        Err(e) => return refuse(format!("cannot migrate to {}: {e}", api_version_name(to))),
    }

    let found = find_api_versions(text);
    if found.is_empty() {
        return refuse("no apiVersion found; is this a Meridian document?".to_owned());
    }
    let mut lowest = to;
    for f in &found {
        let value = &text[f.start..f.end];
        let Some(number) = VersionTable::number_of(value) else {
            return refuse(format!(
                "apiVersion {value:?} is not a Meridian language version"
            ));
        };
        if number > to {
            return refuse(format!(
                "a document is at {value}, which is newer than {}; a migration never goes \
                 backward",
                api_version_name(to)
            ));
        }
        lowest = lowest.min(number);
    }
    if lowest == to {
        return Ok(Migrated::Unchanged);
    }

    // A file's documents move together, so a file mixing versions is brought
    // to its lowest one's next version first. Each step sees the whole file.
    let mut text = text.to_owned();
    for from in lowest..to {
        let Some(step) = steps.iter().find(|s| s.from == from) else {
            return refuse(format!(
                "this build has no migration from {} to {}",
                api_version_name(from),
                api_version_name(from + 1)
            ));
        };
        text = (step.rewrite)(&text).map_err(MigrateError::NeedsDecisions)?;
        // Replace back to front, so earlier offsets stay true.
        let old = api_version_name(from);
        for f in find_api_versions(&text).into_iter().rev() {
            if text[f.start..f.end] == old {
                text.replace_range(f.start..f.end, &api_version_name(from + 1));
            }
        }
    }
    Ok(Migrated::Rewritten(text))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::{ApiVersion, Status};

    fn table() -> VersionTable {
        VersionTable::new(vec![
            ApiVersion {
                number: 1,
                status: Status::Deprecated { since: "1.9.0" },
            },
            ApiVersion {
                number: 2,
                status: Status::Supported,
            },
            ApiVersion {
                number: 3,
                status: Status::Supported,
            },
        ])
    }

    fn rename(text: &str) -> Result<String, Decisions> {
        Ok(text.replace("runtimeWhen:", "onlyIf:"))
    }

    fn undecidable(text: &str) -> Result<String, Decisions> {
        if text.contains("ifMissing") {
            Err(vec![
                "ifMissing changed its default; state it on each dependency".to_owned(),
            ])
        } else {
            Ok(text.to_owned())
        }
    }

    const STEPS: &[Step] = &[
        Step {
            from: 1,
            rewrite: rename,
        },
        Step {
            from: 2,
            rewrite: undecidable,
        },
    ];

    const DOC: &str = "# the web tier\napiVersion: meridian/v1   # pinned\nkind: ResourceSet\nspec:\n  resources:\n    - id: a\n      runtimeWhen: x\n      params:\n        apiVersion: meridian/v1\n---\napiVersion: \"meridian/v1\"\nkind: ResourceSet\n";

    #[test]
    fn a_file_already_at_the_version_is_left_alone() {
        assert_eq!(
            migrate(&table(), STEPS, DOC, 1).unwrap(),
            Migrated::Unchanged
        );
        assert_eq!(
            migrate(&VersionTable::builtin(), super::STEPS, DOC, 1).unwrap(),
            Migrated::Unchanged
        );
    }

    #[test]
    fn a_step_rewrites_the_file_and_only_the_top_level_version() {
        let Migrated::Rewritten(text) = migrate(&table(), STEPS, DOC, 2).unwrap() else {
            panic!("expected a rewrite");
        };
        assert_eq!(
            text,
            "# the web tier\napiVersion: meridian/v2   # pinned\nkind: ResourceSet\nspec:\n  resources:\n    - id: a\n      onlyIf: x\n      params:\n        apiVersion: meridian/v1\n---\napiVersion: \"meridian/v2\"\nkind: ResourceSet\n"
        );
    }

    #[test]
    fn steps_chain_across_several_versions() {
        let Migrated::Rewritten(text) = migrate(&table(), STEPS, DOC, 3).unwrap() else {
            panic!("expected a rewrite");
        };
        assert!(
            text.contains("apiVersion: meridian/v3   # pinned"),
            "{text}"
        );
        assert!(text.contains("apiVersion: \"meridian/v3\""), "{text}");
    }

    #[test]
    fn json_is_rewritten_in_place() {
        let json = "{\n  \"apiVersion\": \"meridian/v1\",\n  \"kind\": \"ResourceSet\"\n}\n";
        let Migrated::Rewritten(text) = migrate(&table(), STEPS, json, 2).unwrap() else {
            panic!("expected a rewrite");
        };
        assert_eq!(
            text,
            "{\n  \"apiVersion\": \"meridian/v2\",\n  \"kind\": \"ResourceSet\"\n}\n"
        );
    }

    #[test]
    fn what_only_a_person_can_decide_is_reported_and_nothing_is_written() {
        let doc = "apiVersion: meridian/v2\nspec:\n  x: {ifMissing: skip}\n";
        assert_eq!(
            migrate(&table(), STEPS, doc, 3).unwrap_err(),
            MigrateError::NeedsDecisions(vec![
                "ifMissing changed its default; state it on each dependency".to_owned()
            ])
        );
    }

    #[test]
    fn a_migration_never_goes_backward_or_to_a_version_nobody_knows() {
        let refused = |text: &str, to| match migrate(&table(), STEPS, text, to).unwrap_err() {
            MigrateError::Refused(msg) => msg,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            refused("apiVersion: meridian/v3\n", 2),
            "a document is at meridian/v3, which is newer than meridian/v2; a migration never \
             goes backward"
        );
        assert!(
            refused(DOC, 9).starts_with("cannot migrate to meridian/v9: unsupported apiVersion")
        );
        assert_eq!(
            refused("kind: ResourceSet\n", 2),
            "no apiVersion found; is this a Meridian document?"
        );
        assert_eq!(
            refused("apiVersion: apps/v1\n", 2),
            "apiVersion \"apps/v1\" is not a Meridian language version"
        );
        assert_eq!(
            match migrate(&table(), &[], DOC, 2).unwrap_err() {
                MigrateError::Refused(msg) => msg,
                other => panic!("{other:?}"),
            },
            "this build has no migration from meridian/v1 to meridian/v2"
        );
    }
}
