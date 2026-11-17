//! Choosing documents by their labels.

use std::collections::BTreeMap;
use std::fmt;

/// A label selector: a list of requirements a document's `metadata.labels`
/// must all meet for the document to be part of a run.
///
/// Written as `tier=web,env!=prod,owner`:
///
/// - `key=value`: the label is set to exactly that value
/// - `key!=value`: the label is not set to that value (unset counts)
/// - `key`: the label is set, to anything
/// - `!key`: the label is not set
///
/// The empty selector selects every document.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Selector(Vec<Requirement>);

#[derive(Clone, Debug, PartialEq, Eq)]
enum Requirement {
    Is(String, String),
    IsNot(String, String),
    Set(String),
    Unset(String),
}

impl Selector {
    /// The selector that selects everything.
    pub fn all() -> Self {
        Selector::default()
    }

    /// Reads a selector. Several may be joined with commas; every
    /// requirement must hold.
    pub fn parse(text: &str) -> Result<Selector, String> {
        let mut out = Vec::new();
        for part in text.split(',') {
            let part = part.trim();
            if part.is_empty() {
                return Err(format!(
                    "selector {text:?} has an empty requirement (write key=value, \
                     key!=value, key or !key, separated by commas)"
                ));
            }
            let key = |key: &str| {
                let key = key.trim();
                if key.is_empty() || key.contains(['=', '!']) {
                    Err(format!(
                        "selector requirement {part:?} has no usable label name (write \
                         key=value, key!=value, key or !key)"
                    ))
                } else {
                    Ok(key.to_owned())
                }
            };
            out.push(if let Some((k, v)) = part.split_once("!=") {
                Requirement::IsNot(key(k)?, v.trim().to_owned())
            } else if let Some((k, v)) = part.split_once('=') {
                // `==` is accepted as a spelling of `=`.
                let v = v.strip_prefix('=').unwrap_or(v);
                Requirement::Is(key(k)?, v.trim().to_owned())
            } else if let Some(k) = part.strip_prefix('!') {
                Requirement::Unset(key(k)?)
            } else {
                Requirement::Set(key(part)?)
            });
        }
        Ok(Selector(out))
    }

    /// Joins another selector's requirements onto this one's.
    pub fn and(mut self, other: Selector) -> Selector {
        self.0.extend(other.0);
        self
    }

    /// Whether the selector selects everything.
    pub fn is_all(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether a document carrying `labels` is selected.
    pub fn matches(&self, labels: &BTreeMap<String, String>) -> bool {
        self.0.iter().all(|requirement| match requirement {
            Requirement::Is(k, v) => labels.get(k) == Some(v),
            Requirement::IsNot(k, v) => labels.get(k) != Some(v),
            Requirement::Set(k) => labels.contains_key(k),
            Requirement::Unset(k) => !labels.contains_key(k),
        })
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .0
            .iter()
            .map(|requirement| match requirement {
                Requirement::Is(k, v) => format!("{k}={v}"),
                Requirement::IsNot(k, v) => format!("{k}!={v}"),
                Requirement::Set(k) => k.clone(),
                Requirement::Unset(k) => format!("!{k}"),
            })
            .collect();
        f.write_str(&parts.join(","))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labels(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect()
    }

    #[test]
    fn every_requirement_must_hold() {
        let web = labels(&[("tier", "web"), ("env", "prod")]);
        for (selector, want) in [
            ("tier=web", true),
            ("tier==web", true),
            ("tier=db", false),
            ("tier=web,env=prod", true),
            ("tier=web,env=dev", false),
            ("env!=dev", true),
            ("env!=prod", false),
            ("owner!=ops", true),
            ("tier", true),
            ("owner", false),
            ("!owner", true),
            ("!tier", false),
            (" tier = web , env ", true),
        ] {
            let parsed = Selector::parse(selector).unwrap();
            assert_eq!(parsed.matches(&web), want, "{selector}");
        }
    }

    #[test]
    fn the_empty_selector_selects_everything() {
        assert!(Selector::all().is_all());
        assert!(Selector::all().matches(&labels(&[])));
        assert!(!Selector::parse("a").unwrap().is_all());
    }

    #[test]
    fn a_malformed_selector_is_refused() {
        for bad in ["", "a,,b", "=web", "!=web", "!", "a=b,"] {
            assert!(Selector::parse(bad).is_err(), "{bad:?} should be refused");
        }
    }

    #[test]
    fn a_selector_prints_the_way_it_is_written() {
        let text = "tier=web,env!=prod,owner,!legacy";
        assert_eq!(Selector::parse(text).unwrap().to_string(), text);
        let joined = Selector::parse("a=1")
            .unwrap()
            .and(Selector::parse("b").unwrap());
        assert_eq!(joined.to_string(), "a=1,b");
    }
}
