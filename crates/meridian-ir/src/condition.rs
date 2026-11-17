//! The condition stage: `when` expressions, and what happens to the edges of
//! a resource one of them removes.
//!
//! A `when` is decided by Meridian before anything is compiled, so a resource
//! is either in the compiled output or it is not, and no target ever sees a
//! conditional. The language is deliberately small: it reads the data given to
//! the command and compares it. It has no functions over the node, because
//! nothing gathers facts from a node yet, and a condition that could not be
//! decided would be worse than one that cannot be written.
//!
//! ```text
//! expr    := and ( "||" and )*
//! and     := equal ( "&&" equal )*
//! equal   := unary ( ( "==" | "!=" ) unary )?
//! unary   := "!" unary | primary
//! primary := "(" expr ")" | "true" | "false" | "null" | number | string
//!          | "defined" "(" path ")" | path
//! path    := "data" ( "." name )+
//! ```

use std::collections::{BTreeMap, HashSet};
use std::fmt;

use meridian_value::Value;

use crate::error::Error;
use crate::types::{IfMissing, Resource};

/// The data a condition is decided against: the same map every target is
/// handed when it compiles.
pub type Data = BTreeMap<String, Value>;

/// A parsed `when` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition(Expr);

#[derive(Clone, Debug, PartialEq)]
enum Expr {
    Literal(Value),
    /// The keys after `data`.
    Path(Vec<String>),
    Defined(Vec<String>),
    Not(Box<Expr>),
    And(Box<Expr>, Box<Expr>),
    Or(Box<Expr>, Box<Expr>),
    Equal(Box<Expr>, Box<Expr>),
    NotEqual(Box<Expr>, Box<Expr>),
}

#[derive(Clone, Debug, PartialEq)]
enum Token {
    Open,
    Close,
    Dot,
    Not,
    And,
    Or,
    Equal,
    NotEqual,
    Name(String),
    Number(Value),
    Text(String),
}

impl fmt::Display for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Token::Open => f.write_str("\"(\""),
            Token::Close => f.write_str("\")\""),
            Token::Dot => f.write_str("\".\""),
            Token::Not => f.write_str("\"!\""),
            Token::And => f.write_str("\"&&\""),
            Token::Or => f.write_str("\"||\""),
            Token::Equal => f.write_str("\"==\""),
            Token::NotEqual => f.write_str("\"!=\""),
            Token::Name(name) => write!(f, "{name:?}"),
            Token::Number(value) => f.write_str(&value.canonical()),
            Token::Text(text) => write!(f, "the text {text:?}"),
        }
    }
}

fn tokens(text: &str) -> Result<Vec<Token>, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        let next = chars.get(i + 1).copied();
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                out.push(Token::Open);
                i += 1;
            }
            ')' => {
                out.push(Token::Close);
                i += 1;
            }
            '.' => {
                out.push(Token::Dot);
                i += 1;
            }
            '!' if next == Some('=') => {
                out.push(Token::NotEqual);
                i += 2;
            }
            '!' => {
                out.push(Token::Not);
                i += 1;
            }
            '=' if next == Some('=') => {
                out.push(Token::Equal);
                i += 2;
            }
            '&' if next == Some('&') => {
                out.push(Token::And);
                i += 2;
            }
            '|' if next == Some('|') => {
                out.push(Token::Or);
                i += 2;
            }
            '\'' | '"' => {
                let mut value = String::new();
                let mut j = i + 1;
                loop {
                    match chars.get(j) {
                        None => return Err("a quoted text is never closed".to_owned()),
                        Some(&q) if q == c => break,
                        Some('\\') => {
                            let Some(&escaped) = chars.get(j + 1) else {
                                return Err("a quoted text is never closed".to_owned());
                            };
                            value.push(escaped);
                            j += 2;
                        }
                        Some(&other) => {
                            value.push(other);
                            j += 1;
                        }
                    }
                }
                out.push(Token::Text(value));
                i = j + 1;
            }
            c if c.is_ascii_digit() || (c == '-' && next.is_some_and(|n| n.is_ascii_digit())) => {
                let start = i;
                i += 1;
                while i < chars.len()
                    && (chars[i].is_ascii_digit()
                        || (chars[i] == '.'
                            && chars.get(i + 1).is_some_and(|n| n.is_ascii_digit())))
                {
                    i += 1;
                }
                let literal: String = chars[start..i].iter().collect();
                let value = if let Ok(n) = literal.parse::<i64>() {
                    Value::Int(n)
                } else if let Ok(n) = literal.parse::<f64>() {
                    Value::Float(n)
                } else {
                    return Err(format!("{literal:?} is not a number"));
                };
                out.push(Token::Number(value));
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric() || chars[i] == '_' || chars[i] == '-')
                {
                    i += 1;
                }
                out.push(Token::Name(chars[start..i].iter().collect()));
            }
            other => {
                return Err(format!(
                    "unexpected {:?} (a condition is built from data.<key>, true, false, \
                     null, numbers, quoted text, defined(...), !, ==, !=, && and ||)",
                    other.to_string()
                ))
            }
        }
    }
    Ok(out)
}

struct Parser {
    tokens: Vec<Token>,
    at: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.at)
    }

    fn take(&mut self, token: &Token) -> bool {
        if self.peek() == Some(token) {
            self.at += 1;
            true
        } else {
            false
        }
    }

    fn expect(&mut self, token: Token) -> Result<(), String> {
        if self.take(&token) {
            return Ok(());
        }
        Err(match self.peek() {
            Some(found) => format!("expected {token}, found {found}"),
            None => format!("expected {token}, but the condition ends"),
        })
    }

    fn or(&mut self) -> Result<Expr, String> {
        let mut left = self.and()?;
        while self.take(&Token::Or) {
            left = Expr::Or(Box::new(left), Box::new(self.and()?));
        }
        Ok(left)
    }

    fn and(&mut self) -> Result<Expr, String> {
        let mut left = self.equal()?;
        while self.take(&Token::And) {
            left = Expr::And(Box::new(left), Box::new(self.equal()?));
        }
        Ok(left)
    }

    fn equal(&mut self) -> Result<Expr, String> {
        let left = self.unary()?;
        if self.take(&Token::Equal) {
            return Ok(Expr::Equal(Box::new(left), Box::new(self.unary()?)));
        }
        if self.take(&Token::NotEqual) {
            return Ok(Expr::NotEqual(Box::new(left), Box::new(self.unary()?)));
        }
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, String> {
        if self.take(&Token::Not) {
            return Ok(Expr::Not(Box::new(self.unary()?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        let Some(token) = self.peek().cloned() else {
            return Err("the condition ends where a value was expected".to_owned());
        };
        self.at += 1;
        match token {
            Token::Open => {
                let inner = self.or()?;
                self.expect(Token::Close)?;
                Ok(inner)
            }
            Token::Number(value) => Ok(Expr::Literal(value)),
            Token::Text(text) => Ok(Expr::Literal(Value::String(text))),
            Token::Name(name) => match name.as_str() {
                "true" => Ok(Expr::Literal(Value::Bool(true))),
                "false" => Ok(Expr::Literal(Value::Bool(false))),
                "null" => Ok(Expr::Literal(Value::Null)),
                "defined" => {
                    self.expect(Token::Open)?;
                    let Some(Token::Name(root)) = self.peek().cloned() else {
                        return Err("defined(...) takes a data.<key> path".to_owned());
                    };
                    self.at += 1;
                    let path = self.path(&root)?;
                    self.expect(Token::Close)?;
                    Ok(Expr::Defined(path))
                }
                _ => Ok(Expr::Path(self.path(&name)?)),
            },
            other => Err(format!("unexpected {other} where a value was expected")),
        }
    }

    /// The keys of a path whose first name has already been read.
    fn path(&mut self, root: &str) -> Result<Vec<String>, String> {
        match root {
            "data" => {}
            "facts" => {
                return Err(
                    "facts are not available: Meridian does not gather facts from a node \
                     yet, so a condition can only read data.<key>. To decide on the node \
                     while applying, use runtimeWhen on a target that supports it"
                        .to_owned(),
                )
            }
            other => {
                return Err(format!(
                    "unknown name {other:?} (a condition reads data.<key>; text is quoted)"
                ))
            }
        }
        let mut keys = Vec::new();
        while self.take(&Token::Dot) {
            match self.peek().cloned() {
                Some(Token::Name(key)) => {
                    self.at += 1;
                    keys.push(key);
                }
                // A key that is all digits arrives as a number.
                Some(Token::Number(Value::Int(n))) if n >= 0 => {
                    self.at += 1;
                    keys.push(n.to_string());
                }
                _ => return Err("a \".\" must be followed by a key".to_owned()),
            }
        }
        if keys.is_empty() {
            return Err("data must be followed by a key, as in data.environment".to_owned());
        }
        Ok(keys)
    }
}

fn lookup<'a>(data: &'a Data, keys: &[String]) -> Result<Option<&'a Value>, String> {
    let mut current = match data.get(&keys[0]) {
        Some(value) => value,
        None => return Ok(None),
    };
    for (i, key) in keys.iter().enumerate().skip(1) {
        match current {
            Value::Map(map) => match map.get(key) {
                Some(value) => current = value,
                None => return Ok(None),
            },
            other => {
                return Err(format!(
                    "data.{} is {}, so it has no key {key:?}",
                    keys[..i].join("."),
                    other.describe()
                ))
            }
        }
    }
    Ok(Some(current))
}

/// Numbers compare by value whatever they were written as; everything else
/// compares as itself.
fn same(a: &Value, b: &Value) -> bool {
    fn number(v: &Value) -> Option<f64> {
        match v {
            Value::Int(n) => Some(*n as f64),
            Value::Uint(n) => Some(*n as f64),
            Value::Float(n) => Some(*n),
            _ => None,
        }
    }
    match (number(a), number(b)) {
        (Some(x), Some(y)) => x == y,
        _ => a == b,
    }
}

impl Expr {
    fn value(&self, data: &Data) -> Result<Value, String> {
        Ok(match self {
            Expr::Literal(value) => value.clone(),
            Expr::Path(keys) => match lookup(data, keys)? {
                Some(value) => value.clone(),
                None => {
                    return Err(format!(
                        "data.{path} is not set (give it with --data, or test for it with \
                         defined(data.{path}))",
                        path = keys.join(".")
                    ))
                }
            },
            Expr::Defined(keys) => Value::Bool(lookup(data, keys)?.is_some()),
            Expr::Not(inner) => Value::Bool(!inner.truth(data)?),
            // The right side is only decided when the left leaves the answer
            // open, so `defined(data.x) && data.x == 1` can guard itself.
            Expr::And(left, right) => Value::Bool(left.truth(data)? && right.truth(data)?),
            Expr::Or(left, right) => Value::Bool(left.truth(data)? || right.truth(data)?),
            Expr::Equal(left, right) => Value::Bool(same(&left.value(data)?, &right.value(data)?)),
            Expr::NotEqual(left, right) => {
                Value::Bool(!same(&left.value(data)?, &right.value(data)?))
            }
        })
    }

    /// A value used as a yes or no. Only a boolean is one: text, a number or
    /// null deciding whether a resource exists would be a guess.
    fn truth(&self, data: &Data) -> Result<bool, String> {
        match self.value(data)? {
            Value::Bool(b) => Ok(b),
            other => Err(format!(
                "{} is {}, not true or false; compare it with == or !=",
                self.describe(),
                other.describe()
            )),
        }
    }

    fn describe(&self) -> String {
        match self {
            Expr::Path(keys) => format!("data.{}", keys.join(".")),
            Expr::Literal(value) => value.canonical(),
            _ => "the expression".to_owned(),
        }
    }
}

impl Condition {
    /// Reads a `when` expression. Nothing is evaluated: this is the check
    /// `meridian validate` makes.
    pub fn parse(text: &str) -> Result<Condition, String> {
        let mut parser = Parser {
            tokens: tokens(text)?,
            at: 0,
        };
        let expr = parser.or()?;
        if let Some(extra) = parser.peek() {
            return Err(format!("unexpected {extra} after the end of the condition"));
        }
        Ok(Condition(expr))
    }

    /// Decides the condition against `data`. Anything that is not plainly
    /// true or false is an error, never a guess.
    pub fn evaluate(&self, data: &Data) -> Result<bool, String> {
        self.0.truth(data)
    }
}

/// Decides every `when` in a document's resources and returns the ones that
/// remain, unconditional, in their declared order.
///
/// An edge that pointed at a removed resource follows its `ifMissing`: `skip`
/// drops the edge, and `error` (the default) refuses the document, because a
/// resource that depends on something that is not there is far more often a
/// mistake than an intention. Edges *from* a removed resource go with it.
pub fn prune(resources: &[Resource], data: &Data) -> Result<Vec<Resource>, Error> {
    let mut removed: HashSet<&str> = HashSet::new();
    for (i, r) in resources.iter().enumerate() {
        if r.when.is_empty() {
            continue;
        }
        let keep = Condition::parse(&r.when)
            .and_then(|condition| condition.evaluate(data))
            .map_err(|why| {
                Error::new(
                    r.pos.clone(),
                    format!("spec.resources[{i}].when"),
                    format!("resource {:?}: {why}", r.id),
                )
            })?;
        if !keep {
            removed.insert(&r.id);
        }
    }

    let mut kept = Vec::with_capacity(resources.len() - removed.len());
    for (i, r) in resources.iter().enumerate() {
        if removed.contains(r.id.as_str()) {
            continue;
        }
        let mut r = r.clone();
        r.when = String::new();

        let dangling = |verb: &str, field: &str, j: usize, missing: &str, pos| {
            Error::new(
                pos,
                format!("spec.resources[{i}].{field}[{j}].resource"),
                format!(
                    "resource {:?} {verb} {missing:?}, which its `when` removed from this \
                     run; set ifMissing: skip on this entry if the resource should carry \
                     on without it",
                    r.id
                ),
            )
        };
        for (j, dep) in r.depends_on.iter().enumerate() {
            if removed.contains(dep.resource.as_str()) && dep.if_missing != IfMissing::Skip {
                return Err(dangling(
                    "depends on",
                    "dependsOn",
                    j,
                    &dep.resource,
                    dep.pos.clone(),
                ));
            }
        }
        for (j, n) in r.notifies.iter().enumerate() {
            if removed.contains(n.resource.as_str()) && n.if_missing != IfMissing::Skip {
                return Err(dangling(
                    "notifies",
                    "notifies",
                    j,
                    &n.resource,
                    n.pos.clone(),
                ));
            }
        }
        r.depends_on
            .retain(|dep| !removed.contains(dep.resource.as_str()));
        r.notifies
            .retain(|n| !removed.contains(n.resource.as_str()));
        kept.push(r);
    }
    Ok(kept)
}
