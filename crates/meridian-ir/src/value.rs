use std::collections::BTreeMap;

/// A free-form value as it appears under a resource's `params`.
///
/// Maps are ordered by key, so nothing downstream can come to depend on the
/// order a document happened to declare its keys in.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(i64),
    /// An integer literal too large for `Int`.
    Uint(u64),
    Float(f64),
    String(String),
    List(Vec<Value>),
    Map(BTreeMap<String, Value>),
}

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::String(s) => Some(s),
            _ => None,
        }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}
