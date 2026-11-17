//! Writing Go template text: the expressions a reference becomes, and the
//! escaping that keeps the author's own text from being read as a template.

use std::fmt::Write as _;

use meridian_sdk::Value;

use crate::path::{is_identifier, Step};

/// One piece of a string once its references are resolved.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    /// Text the author wrote.
    Text(String),
    /// A value the chart reads when it is rendered.
    Expr {
        /// The Go template expression that yields it.
        go: String,
        /// What it yields with the document's own values. The chart and the
        /// release are compiled from one document, so this is known.
        now: Value,
    },
}

/// Text that Helm must render as itself. Outside an action only `{{` means
/// anything to the template engine, and an action that yields `{{` is how a
/// literal one is written. The backquoted form needs no escaping in any YAML
/// string style, which a double-quoted `"{{"` would.
pub(crate) fn literal(text: &str) -> String {
    text.replace("{{", "{{ `{{` }}")
}

/// A Go interpreted string literal.
pub(crate) fn go_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c == '\u{7f}' => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The expression that follows a path from a root such as `.Values`.
pub(crate) fn go_path(root: &str, steps: &[Step]) -> String {
    let mut current = root.to_owned();
    for step in steps {
        current = match step {
            Step::Key(key) if is_identifier(key) => format!("{current}.{key}"),
            Step::Key(key) => format!("(index {current} {})", go_string(key)),
            Step::Index(index) => format!("(index {current} {index})"),
        };
    }
    current
}

/// A value as it reads inside text: a string as itself, a number or a boolean
/// the way JSON writes it. `None` for a value that has no one-line form.
pub(crate) fn as_text(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        // A plugin is never sent a secret as a value; one would be written as its mask.
        Value::Secret(_) => Some(meridian_sdk::SENSITIVE.to_owned()),
        Value::Bool(_) | Value::Int(_) | Value::Uint(_) | Value::Float(_) => {
            Some(value.canonical())
        }
        Value::Null | Value::List(_) | Value::Map(_) => None,
    }
}

/// The text the pieces come to with the document's own values. `Err` names
/// the kind of value that cannot be part of text.
pub(crate) fn render_now(pieces: &[Piece]) -> Result<String, &'static str> {
    let mut out = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => out.push_str(text),
            Piece::Expr { now, .. } => out.push_str(&as_text(now).ok_or(now.describe())?),
        }
    }
    Ok(out)
}

/// What a string scalar becomes in a template.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scalar {
    /// No references: the text itself, to be written as a YAML string.
    Text(String),
    /// A template action that writes the whole YAML value.
    Action(String),
}

/// Turns resolved pieces into a scalar.
///
/// Every action ends in `toRawJson`, so whatever a value holds is written as
/// valid YAML: a string arrives quoted and escaped, a number stays a number,
/// a map or a list is written in flow style. The raw form is the one that
/// leaves `<`, `>` and `&` readable. Helm reads values-file numbers as
/// floats and prints a large one as `1e+06`; inside text a number therefore
/// goes through `toRawJson` as well, which writes it in full.
pub(crate) fn scalar(pieces: &[Piece]) -> Result<Scalar, &'static str> {
    if pieces.iter().all(|p| matches!(p, Piece::Text(_))) {
        return Ok(Scalar::Text(render_now(pieces)?));
    }
    if let [Piece::Expr { go, .. }] = pieces {
        return Ok(Scalar::Action(format!("{{{{ {go} | toRawJson }}}}")));
    }
    let mut format = String::new();
    let mut arguments = String::new();
    for piece in pieces {
        match piece {
            Piece::Text(text) => format.push_str(&text.replace('%', "%%")),
            Piece::Expr { go, now } => {
                format.push_str("%s");
                match now {
                    Value::String(_) => {
                        let _ = write!(arguments, " {go}");
                    }
                    Value::Bool(_) | Value::Int(_) | Value::Uint(_) | Value::Float(_) => {
                        let _ = write!(arguments, " (toRawJson {go})");
                    }
                    other => return Err(other.describe()),
                }
            }
        }
    }
    Ok(Scalar::Action(format!(
        "{{{{ printf {}{arguments} | toRawJson }}}}",
        go_string(&format)
    )))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Piece {
        Piece::Text(s.to_owned())
    }

    fn expr(go: &str, now: Value) -> Piece {
        Piece::Expr {
            go: go.to_owned(),
            now,
        }
    }

    #[test]
    fn only_an_opening_delimiter_needs_escaping() {
        assert_eq!(literal("plain }} text"), "plain }} text");
        assert_eq!(literal("{{ .Values.x }}"), "{{ `{{` }} .Values.x }}");
        assert_eq!(literal("a{{{b"), "a{{ `{{` }}{b");
        assert_eq!(literal("{{/* c */}}"), "{{ `{{` }}/* c */}}");
    }

    #[test]
    fn a_go_string_escapes_what_go_would_misread() {
        assert_eq!(go_string("a \"b\" \\ c"), r#""a \"b\" \\ c""#);
        assert_eq!(go_string("one\ntwo\t\u{1}"), r#""one\ntwo\t\u0001""#);
        assert_eq!(go_string("naïve {{ }}"), "\"naïve {{ }}\"");
    }

    #[test]
    fn a_path_uses_index_for_what_is_not_an_identifier() {
        use Step::{Index, Key};
        assert_eq!(
            go_path(".Values", &[Key("image".into()), Key("tag".into())]),
            ".Values.image.tag"
        );
        assert_eq!(
            go_path(
                ".Values",
                &[Key("hosts".into()), Index(0), Key("name".into())]
            ),
            "(index .Values.hosts 0).name"
        );
        assert_eq!(
            go_path(".Values", &[Key("my-key".into()), Key("a".into())]),
            "(index .Values \"my-key\").a"
        );
    }

    #[test]
    fn a_whole_reference_keeps_its_type_and_text_is_built_with_printf() {
        assert_eq!(
            scalar(&[text("nginx")]).unwrap(),
            Scalar::Text("nginx".into())
        );
        assert_eq!(
            scalar(&[expr(".Values.replicas", Value::Int(2))]).unwrap(),
            Scalar::Action("{{ .Values.replicas | toRawJson }}".into())
        );
        assert_eq!(
            scalar(&[
                text("nginx:"),
                expr(".Values.tag", Value::String("1.27".into())),
                text(" 100% \"quoted\""),
                expr(".Values.port", Value::Int(8080)),
            ])
            .unwrap(),
            Scalar::Action(
                r#"{{ printf "nginx:%s 100%% \"quoted\"%s" .Values.tag (toRawJson .Values.port) | toRawJson }}"#
                    .into()
            )
        );
        assert_eq!(
            scalar(&[text("x"), expr(".Values.m", Value::Map(Default::default()))]).unwrap_err(),
            "a mapping"
        );
    }

    #[test]
    fn pieces_render_with_the_documents_own_values() {
        assert_eq!(
            render_now(&[
                expr(".Release.Name", Value::String("web".into())),
                text("-"),
                expr(".Values.n", Value::Int(3)),
                expr(".Values.on", Value::Bool(true)),
            ])
            .unwrap(),
            "web-3true"
        );
        assert_eq!(
            render_now(&[expr(".Values.l", Value::List(Vec::new()))]).unwrap_err(),
            "a list"
        );
    }
}
