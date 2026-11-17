//! Interpolation: expressions written inside a value between an opening
//! dollar and two braces, and two closing braces.
//!
//! The spelling sits beside the reference a tool resolves, `${id.path}`, and
//! collides with nothing a tool owns (`{{ }}`, `%{ }`). What is not an
//! interpolation is passed through byte for byte, including a reference and
//! the escape for a literal one.

use std::collections::BTreeSet;

use meridian_value::Value;

use crate::{Expr, Names, Scope};

/// What opens an interpolation.
pub const OPEN: &str = "${{";
/// What closes one.
pub const CLOSE: &str = "}}";

#[derive(Clone, Debug, PartialEq)]
enum Part {
    Text(String),
    Expr { source: String, expr: Expr },
}

/// A value's text, split into what is literal and what is decided.
#[derive(Clone, Debug, PartialEq)]
pub struct Template {
    parts: Vec<Part>,
}

/// Whether `text` holds an interpolation at all. Text that holds none is
/// never touched, so everything written before interpolation existed keeps
/// its exact bytes.
pub fn has_interpolation(text: &str) -> bool {
    let mut rest = text;
    while let Some(at) = rest.find(OPEN) {
        if !rest[..at].ends_with('$') {
            return true;
        }
        rest = &rest[at + OPEN.len()..];
    }
    false
}

/// Where the expression that starts at the beginning of `body` ends: the
/// offset of its closing braces. Quoted text and nested brackets are stepped
/// over, so a mapping literal or a quoted brace does not end it early.
fn closing(body: &str) -> Option<usize> {
    let bytes = body.as_bytes();
    let mut depth = 0usize;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            quote @ (b'\'' | b'"') => {
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    if bytes[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
            }
            b'{' | b'[' | b'(' => depth += 1,
            b'}' if depth == 0 => {
                return (bytes.get(i + 1) == Some(&b'}')).then_some(i);
            }
            b'}' | b']' | b')' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    None
}

impl Template {
    /// Splits `text` and reads every expression in it. Nothing is decided.
    pub fn parse(text: &str) -> Result<Template, String> {
        let mut parts = Vec::new();
        let mut literal = String::new();
        let mut rest = text;
        while let Some(at) = rest.find(OPEN) {
            // An escaped opening is left exactly as written: the plugin's
            // own rule for an escaped reference turns it into the literal.
            if rest[..at].ends_with('$') {
                literal.push_str(&rest[..at + OPEN.len()]);
                rest = &rest[at + OPEN.len()..];
                continue;
            }
            literal.push_str(&rest[..at]);
            let body = &rest[at + OPEN.len()..];
            // When the brackets inside do not balance, the first closing
            // braces end it, and the expression itself says what is wrong.
            let Some(end) = closing(body).or_else(|| body.find(CLOSE)) else {
                return Err(format!(
                    "{text:?} opens an interpolation that is never closed with {CLOSE}"
                ));
            };
            let source = body[..end].trim();
            if source.is_empty() {
                return Err(format!("{text:?} holds an empty interpolation"));
            }
            let expr =
                Expr::parse(source).map_err(|why| format!("in {OPEN} {source} {CLOSE}: {why}"))?;
            if !literal.is_empty() {
                parts.push(Part::Text(std::mem::take(&mut literal)));
            }
            parts.push(Part::Expr {
                source: source.to_owned(),
                expr,
            });
            rest = &body[end + CLOSE.len()..];
        }
        literal.push_str(rest);
        if !literal.is_empty() {
            parts.push(Part::Text(literal));
        }
        Ok(Template { parts })
    }

    /// Whether the text holds no interpolation, and so is its own value.
    pub fn is_literal(&self) -> bool {
        !self.parts.iter().any(|p| matches!(p, Part::Expr { .. }))
    }

    /// The single expression the text consists of, when it is nothing else.
    /// Its value keeps its type.
    fn only(&self) -> Option<(&str, &Expr)> {
        match self.parts.as_slice() {
            [Part::Expr { source, expr }] => Some((source, expr)),
            _ => None,
        }
    }

    /// Checks every expression's names and functions; see [`Expr::check`].
    pub fn check(&self, names: &Names) -> Result<(), String> {
        for part in &self.parts {
            if let Part::Expr { source, expr } = part {
                expr.check(names)
                    .map_err(|why| format!("in {OPEN} {source} {CLOSE}: {why}"))?;
            }
        }
        Ok(())
    }

    /// Every `(root, key)` the expressions read; see [`Expr::reads`].
    pub fn reads(&self) -> BTreeSet<(String, Option<String>)> {
        let mut out = BTreeSet::new();
        for part in &self.parts {
            if let Part::Expr { expr, .. } = part {
                out.extend(expr.reads());
            }
        }
        out
    }

    /// The expression's own text, when the whole of the text is one
    /// interpolation. A condition may be written this way, and means the
    /// same as the expression written bare.
    pub fn sole_source(&self) -> Option<&str> {
        self.only().map(|(source, _)| source)
    }

    /// [`Template::eval`], for a value a plugin will read afterwards: every
    /// value an interpolation places is made literal, so text that came from
    /// data can never turn into a reference the tool resolves. What the
    /// author wrote around the interpolation is left as written.
    pub fn eval_literal(&self, scope: &Scope) -> Result<Value, String> {
        self.decide(scope, true)
    }

    /// Decides the text.
    ///
    /// Text that is one interpolation and nothing else becomes that
    /// expression's value with its type: a number stays a number and a list
    /// stays a list. An interpolation inside longer text must be text, a
    /// number or a boolean, and is placed as text.
    pub fn eval(&self, scope: &Scope) -> Result<Value, String> {
        self.decide(scope, false)
    }

    fn decide(&self, scope: &Scope, literal: bool) -> Result<Value, String> {
        let placed = |value: Value| if literal { escape(value) } else { value };
        if let Some((source, expr)) = self.only() {
            return expr
                .eval(scope)
                .map(placed)
                .map_err(|why| format!("in {OPEN} {source} {CLOSE}: {why}"));
        }
        let mut out = String::new();
        // Whether the text last added was placed by an interpolation. A
        // placed value that ends in a dollar, followed by text that opens a
        // brace (or the other way round), would join into a reference that
        // neither side wrote, so the join is made literal as well.
        let mut last_placed = false;
        let mut add = |out: &mut String, text: &str, is_placed: bool| -> Result<(), String> {
            if text.is_empty() {
                return Ok(());
            }
            // The other way round there is nothing to add that would help: a
            // placed value that ends in a dollar, directly before a
            // reference the author wrote, is read by every plugin as the
            // escape for a literal, and the reference would be handed over
            // as text. No spelling keeps both the dollar and the reference,
            // so it is refused, and the author is told how to keep them
            // apart.
            if literal
                && last_placed
                && !is_placed
                && out.ends_with('$')
                && text.starts_with(REFERENCE)
            {
                let reference = text.split_inclusive('}').next().unwrap_or(text);
                return Err(format!(
                    "the value placed before {reference} ends in \"$\", and a \"$\" directly \
                     before a reference makes it literal text, so the reference would be \
                     lost; put a space or other text between the two"
                ));
            }
            if literal && (is_placed || last_placed) && out.ends_with('$') && text.starts_with('{')
            {
                out.push('$');
            }
            out.push_str(text);
            last_placed = is_placed;
            Ok(())
        };
        for part in &self.parts {
            match part {
                Part::Text(text) => add(&mut out, text, false)?,
                Part::Expr { source, expr } => {
                    let fail = |why: String| format!("in {OPEN} {source} {CLOSE}: {why}");
                    match expr.eval(scope).map_err(fail)? {
                        value @ Value::String(_) => {
                            if let Value::String(s) = placed(value) {
                                add(&mut out, &s, true)?;
                            }
                        }
                        value @ (Value::Bool(_)
                        | Value::Int(_)
                        | Value::Uint(_)
                        | Value::Float(_)) => add(&mut out, &value.canonical(), true)?,
                        Value::Null => {
                            return Err(fail(
                                "the value is null, which has no text form; give a default \
                                 with ??"
                                    .to_owned(),
                            ))
                        }
                        other => {
                            return Err(fail(format!(
                                "{} cannot be placed inside text; use join(...) or \
                                 toJson(...), or make the interpolation the whole value",
                                other.describe()
                            )))
                        }
                    }
                }
            }
        }
        Ok(Value::String(out))
    }
}

/// What opens a reference the tool resolves, and its escaped, literal form.
const REFERENCE: &str = concat!("$", "{");
const ESCAPED: &str = concat!("$$", "{");

/// Makes every reference opening in a placed value literal, however deep.
fn escape(value: Value) -> Value {
    match value {
        Value::String(s) if s.contains(REFERENCE) => Value::String(s.replace(REFERENCE, ESCAPED)),
        Value::List(items) => Value::List(items.into_iter().map(escape).collect()),
        Value::Map(entries) => {
            Value::Map(entries.into_iter().map(|(k, v)| (k, escape(v))).collect())
        }
        other => other,
    }
}
