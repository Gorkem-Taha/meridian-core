//! The language versions a build knows, and what it does with each.
//!
//! A document's `apiVersion` is looked up here before anything else in the
//! document is read. The table is data, so supporting, deprecating and
//! removing a version are each a one-line change with the behaviour already
//! written and tested.

use std::fmt;

/// The group every Meridian language version belongs to: `meridian/v1`.
pub const API_GROUP: &str = "meridian";

/// What a build does with documents of one language version.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Documents compile.
    Supported,
    /// Documents compile, and a run that reads one warns once.
    Deprecated {
        /// The core release that started warning.
        since: &'static str,
    },
    /// Documents are refused, by name.
    Removed {
        /// The core release that stopped reading it.
        in_release: &'static str,
    },
}

/// One language version and its state in this build.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApiVersion {
    /// The whole number after `meridian/v`.
    pub number: u32,
    pub status: Status,
}

impl ApiVersion {
    /// The version as a document writes it, such as `meridian/v1`.
    pub fn name(&self) -> String {
        api_version_name(self.number)
    }
}

/// `meridian/v<number>`.
pub fn api_version_name(number: u32) -> String {
    format!("{API_GROUP}/v{number}")
}

/// Why an `apiVersion` was refused. The text says which of several different
/// mistakes was made, because each has a different fix.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Unsupported(pub String);

impl fmt::Display for Unsupported {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Unsupported {}

/// The language versions a build knows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VersionTable {
    /// Ascending by number.
    versions: Vec<ApiVersion>,
}

/// Every language version this build knows. Add a version here; deprecate or
/// remove one by changing its status, never by deleting its row, so the
/// refusal can still say what happened to it.
const BUILTIN: &[ApiVersion] = &[ApiVersion {
    number: 1,
    status: Status::Supported,
}];

impl VersionTable {
    /// The table this build ships.
    pub fn builtin() -> VersionTable {
        VersionTable::new(BUILTIN.to_vec())
    }

    /// A table of the given versions. A build has one, from
    /// [`VersionTable::builtin`]; this exists so the behaviour of a state no
    /// shipped version is in yet can be tested before one is.
    pub fn new(mut versions: Vec<ApiVersion>) -> VersionTable {
        versions.sort_by_key(|v| v.number);
        versions.dedup_by_key(|v| v.number);
        VersionTable { versions }
    }

    /// Every known version, oldest first.
    pub fn versions(&self) -> &[ApiVersion] {
        &self.versions
    }

    /// The newest version documents may be written in: the one `migrate`
    /// moves to, and the successor a deprecation names.
    pub fn current(&self) -> Option<ApiVersion> {
        self.versions
            .iter()
            .rev()
            .find(|v| v.status == Status::Supported)
            .copied()
    }

    /// The versions that compile, as text: `meridian/v1` or
    /// `meridian/v1 to meridian/v2`.
    pub fn readable(&self) -> String {
        let readable: Vec<&ApiVersion> = self
            .versions
            .iter()
            .filter(|v| !matches!(v.status, Status::Removed { .. }))
            .collect();
        match (readable.first(), readable.last()) {
            (Some(first), Some(last)) if first.number != last.number => {
                format!("{} to {}", first.name(), last.name())
            }
            (Some(only), _) => only.name(),
            _ => "no version".to_owned(),
        }
    }

    /// Reads the number out of an `apiVersion`, without judging it.
    pub fn number_of(text: &str) -> Option<u32> {
        let rest = text.strip_prefix(API_GROUP)?.strip_prefix("/v")?;
        // `v01` and `v+1` are not how anyone writes a version.
        if rest.is_empty() || rest.starts_with('0') || !rest.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        rest.parse().ok()
    }

    /// Looks an `apiVersion` up. `Ok` carries the version, whose status says
    /// whether the caller owes a deprecation warning.
    pub fn check(&self, text: &str) -> Result<ApiVersion, Unsupported> {
        let example = self
            .current()
            .map_or_else(|| api_version_name(1), |v| v.name());
        let Some(number) = VersionTable::number_of(text) else {
            let looks_like_ours = text
                .strip_prefix(API_GROUP)
                .is_some_and(|rest| rest.starts_with("/v"));
            let why = if looks_like_ours {
                format!("versions are whole numbers, as in {example:?}")
            } else {
                format!("expected {example:?}")
            };
            return Err(Unsupported(format!(
                "unsupported apiVersion {text:?}: {why}"
            )));
        };
        let known = self.versions.iter().find(|v| v.number == number);
        let migrate = || match self.current() {
            Some(current) => format!("; run `meridian migrate --to {}`", current.name()),
            None => String::new(),
        };
        match known {
            Some(v) => match v.status {
                Status::Removed { in_release } => Err(Unsupported(format!(
                    "unsupported apiVersion {text:?}: it was removed in meridian {in_release}{}",
                    migrate()
                ))),
                _ => Ok(*v),
            },
            None => {
                let newest = self.versions.last().map_or(0, |v| v.number);
                let why = if number > newest {
                    format!(
                        "it is newer than this build, which understands {}; upgrade meridian",
                        self.readable()
                    )
                } else {
                    format!("this build understands {}{}", self.readable(), migrate())
                };
                Err(Unsupported(format!(
                    "unsupported apiVersion {text:?}: {why}"
                )))
            }
        }
    }

    /// The single warning a run owes for the deprecated versions among the
    /// documents it read, or `None` when it read none.
    ///
    /// `documents` is each document's `apiVersion` and a name to call it by.
    /// One warning per run, however many documents: a warning per document
    /// teaches people to filter warnings.
    pub fn deprecation_warning<'a>(
        &self,
        documents: impl IntoIterator<Item = (&'a str, &'a str)>,
    ) -> Option<String> {
        let mut count = 0usize;
        let mut first: Option<(&str, ApiVersion, &'static str)> = None;
        for (api_version, name) in documents {
            let Ok(v) = self.check(api_version) else {
                continue;
            };
            if let Status::Deprecated { since } = v.status {
                count += 1;
                // The oldest version is the most urgent one to name.
                if first.is_none_or(|(_, seen, _)| v.number < seen.number) {
                    first = Some((name, v, since));
                }
            }
        }
        let (name, version, since) = first?;
        let documents = if count == 1 {
            format!("document {name:?} uses")
        } else {
            format!("{count} documents, the first being {name:?}, use")
        };
        let successor = match self.current() {
            Some(current) => format!(
                "; run `meridian migrate --to {}` to move to {}",
                current.name(),
                current.name()
            ),
            None => String::new(),
        };
        Some(format!(
            "{documents} apiVersion {:?}, which is deprecated since meridian {since} and will \
             stop being read{successor}",
            version.name()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn three() -> VersionTable {
        VersionTable::new(vec![
            ApiVersion {
                number: 3,
                status: Status::Supported,
            },
            ApiVersion {
                number: 1,
                status: Status::Removed {
                    in_release: "3.0.0",
                },
            },
            ApiVersion {
                number: 2,
                status: Status::Deprecated { since: "2.4.0" },
            },
        ])
    }

    #[test]
    fn this_build_reads_version_one_and_nothing_else() {
        let table = VersionTable::builtin();
        assert_eq!(
            table.check("meridian/v1").unwrap().status,
            Status::Supported
        );
        assert_eq!(table.current().unwrap().name(), "meridian/v1");
        assert_eq!(table.readable(), "meridian/v1");
        assert_eq!(
            table.check("meridian/v2").unwrap_err().to_string(),
            "unsupported apiVersion \"meridian/v2\": it is newer than this build, which \
             understands meridian/v1; upgrade meridian"
        );
    }

    #[test]
    fn a_refusal_says_which_mistake_was_made() {
        let table = three();
        for (text, want) in [
            (
                "meridian/v1.2",
                "versions are whole numbers, as in \"meridian/v3\"",
            ),
            ("meridian/v", "versions are whole numbers"),
            ("meridian/v01", "versions are whole numbers"),
            ("meridian/vone", "versions are whole numbers"),
            ("acme/v1", "expected \"meridian/v3\""),
            ("v1", "expected \"meridian/v3\""),
            ("meridian/v0", "versions are whole numbers"),
            (
                "meridian/v9",
                "it is newer than this build, which understands meridian/v2 to meridian/v3; \
                 upgrade meridian",
            ),
            (
                "meridian/v1",
                "it was removed in meridian 3.0.0; run `meridian migrate --to meridian/v3`",
            ),
        ] {
            let err = table.check(text).unwrap_err().to_string();
            assert!(err.contains(want), "{text}: {err}");
            assert!(
                err.starts_with(&format!("unsupported apiVersion {text:?}: ")),
                "{err}"
            );
        }
    }

    #[test]
    fn a_deprecated_version_still_compiles() {
        let v = three().check("meridian/v2").unwrap();
        assert_eq!(v.status, Status::Deprecated { since: "2.4.0" });
    }

    #[test]
    fn a_run_warns_once_however_many_documents_are_deprecated() {
        let table = three();
        assert_eq!(table.deprecation_warning([("meridian/v3", "web")]), None);
        assert_eq!(
            table
                .deprecation_warning([("meridian/v3", "db"), ("meridian/v2", "web")])
                .unwrap(),
            "document \"web\" uses apiVersion \"meridian/v2\", which is deprecated since \
             meridian 2.4.0 and will stop being read; run `meridian migrate --to meridian/v3` \
             to move to meridian/v3"
        );
        let many = table
            .deprecation_warning([
                ("meridian/v2", "web"),
                ("meridian/v3", "db"),
                ("meridian/v2", "cache"),
            ])
            .unwrap();
        assert!(
            many.starts_with("2 documents, the first being \"web\", use apiVersion"),
            "{many}"
        );
    }

    #[test]
    fn a_gap_below_the_newest_version_is_not_called_newer() {
        let table = VersionTable::new(vec![ApiVersion {
            number: 3,
            status: Status::Supported,
        }]);
        let err = table.check("meridian/v2").unwrap_err().to_string();
        assert!(
            err.ends_with(
                "this build understands meridian/v3; run `meridian migrate --to meridian/v3`"
            ),
            "{err}"
        );
    }
}
