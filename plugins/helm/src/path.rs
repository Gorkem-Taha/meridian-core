//! The path half of a reference: what follows the resource's id in
//! `${values.image.tag}` or `${web_svc.status.loadBalancer.ingress[0].ip}`.

use std::fmt::Write as _;

use meridian_sdk::Value;

/// One step into a value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    /// A field of a map: `.name` or `["a.b/c"]`.
    Key(String),
    /// An item of a list: `[0]`.
    Index(usize),
}

/// Reads the steps of a path. An empty path is no steps.
pub(crate) fn parse(path: &str) -> Result<Vec<Step>, String> {
    let mut steps = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            let end = after
                .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == '-'))
                .unwrap_or(after.len());
            if end == 0 {
                return Err("a `.` must be followed by a field name".to_owned());
            }
            steps.push(Step::Key(after[..end].to_owned()));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            let Some(end) = after.find(']') else {
                return Err("a `[` is never closed".to_owned());
            };
            let inner = after[..end].trim();
            let quoted = |q: char| {
                inner
                    .strip_prefix(q)
                    .and_then(|s| s.strip_suffix(q))
                    .filter(|_| inner.len() >= 2)
            };
            if let Some(key) = quoted('"').or_else(|| quoted('\'')) {
                steps.push(Step::Key(key.to_owned()));
            } else if let Ok(index) = inner.parse::<usize>() {
                steps.push(Step::Index(index));
            } else {
                return Err(format!(
                    "[{inner}] is neither an index such as [0] nor a quoted key such as [\"a.b\"]"
                ));
            }
            rest = &after[end + 1..];
        } else {
            return Err(format!("{rest:?} is not a `.field` or an `[index]`"));
        }
    }
    Ok(steps)
}

/// The path as a person would write it, without a leading dot.
pub(crate) fn show(steps: &[Step]) -> String {
    let mut out = String::new();
    for step in steps {
        match step {
            Step::Key(key) if is_dotted(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Step::Key(key) => {
                let _ = write!(out, "[{key:?}]");
            }
            Step::Index(index) => {
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}

/// Whether a key can follow a dot in a reference.
fn is_dotted(key: &str) -> bool {
    !key.is_empty()
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Whether a key can be written as `.key` in a Go template.
pub(crate) fn is_identifier(key: &str) -> bool {
    let mut chars = key.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Follows a path into a value. `Err` holds how far the path could be
/// followed, as text.
pub(crate) fn lookup<'a>(root: &'a Value, steps: &[Step]) -> Result<&'a Value, String> {
    let mut current = root;
    for (i, step) in steps.iter().enumerate() {
        let next = match (step, current) {
            (Step::Key(key), Value::Map(map)) => map.get(key),
            (Step::Index(index), Value::List(items)) => items.get(*index),
            _ => None,
        };
        current = next.ok_or_else(|| show(&steps[..=i]))?;
    }
    Ok(current)
}

/// The steps as JSON, for the runner to read back.
pub(crate) fn to_value(steps: &[Step]) -> Value {
    Value::List(
        steps
            .iter()
            .map(|step| match step {
                Step::Key(key) => Value::String(key.clone()),
                Step::Index(index) => Value::Int(*index as i64),
            })
            .collect(),
    )
}

/// The reverse of [`to_value`].
pub(crate) fn from_value(value: &Value) -> Option<Vec<Step>> {
    value
        .as_list()?
        .iter()
        .map(|step| match step {
            Value::String(key) => Some(Step::Key(key.clone())),
            Value::Uint(index) => Some(Step::Index(*index as usize)),
            Value::Int(index) => usize::try_from(*index).ok().map(Step::Index),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Step {
        Step::Key(s.to_owned())
    }

    #[test]
    fn a_path_is_fields_indexes_and_quoted_keys() {
        assert_eq!(parse("").unwrap(), []);
        assert_eq!(parse(".image.tag").unwrap(), [key("image"), key("tag")]);
        assert_eq!(
            parse(".status.loadBalancer.ingress[0].ip").unwrap(),
            [
                key("status"),
                key("loadBalancer"),
                key("ingress"),
                Step::Index(0),
                key("ip")
            ]
        );
        assert_eq!(
            parse(".metadata.annotations[\"example.com/owner\"]").unwrap(),
            [key("metadata"), key("annotations"), key("example.com/owner")]
        );
        assert_eq!(parse(".data['tls.crt']").unwrap(), [key("data"), key("tls.crt")]);
    }

    #[test]
    fn a_malformed_path_says_what_is_wrong() {
        assert!(parse(".").unwrap_err().contains("field name"));
        assert!(parse(".a[0").unwrap_err().contains("never closed"));
        assert!(parse(".a[x]").unwrap_err().contains("neither an index"));
        assert!(parse("(1)").unwrap_err().contains("is not a"));
    }

    #[test]
    fn a_path_is_shown_the_way_it_was_written() {
        let steps = parse(".a.b-c[2][\"x.y\"]").unwrap();
        assert_eq!(show(&steps), "a.b-c[2][\"x.y\"]");
        assert_eq!(from_value(&to_value(&steps)).unwrap(), steps);
    }

    #[test]
    fn a_lookup_reports_how_far_it_got() {
        let root = meridian_sdk::decode_value(br#"{"a":{"b":[10,20]}}"#).unwrap();
        assert_eq!(
            lookup(&root, &parse(".a.b[1]").unwrap()).unwrap(),
            &Value::Int(20)
        );
        assert_eq!(lookup(&root, &parse(".a.c.d").unwrap()).unwrap_err(), "a.c");
        assert_eq!(lookup(&root, &parse(".a.b[5]").unwrap()).unwrap_err(), "a.b[5]");
    }
}
