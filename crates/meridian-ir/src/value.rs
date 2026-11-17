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

    pub fn as_map(&self) -> Option<&BTreeMap<String, Value>> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    pub fn as_list(&self) -> Option<&[Value]> {
        match self {
            Value::List(l) => Some(l),
            _ => None,
        }
    }

    /// The word a diagnostic uses for this value's shape.
    pub fn describe(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "a boolean",
            Value::Int(_) | Value::Uint(_) | Value::Float(_) => "a number",
            Value::String(_) => "a string",
            Value::List(_) => "a list",
            Value::Map(_) => "a mapping",
        }
    }

    /// Renders the value as compact JSON with map keys in sorted order.
    ///
    /// Two values that compare equal always render identically, which makes
    /// this the determinism oracle for the whole toolchain, and it is also the
    /// encoding values cross the plugin boundary in. The bytes are exactly
    /// what Go's `encoding/json` produces for the same value, because plugins
    /// built against the Go SDK were written and tested against that encoding:
    /// `<`, `>` and `&` are escaped, and a float is written the shortest way
    /// that round-trips, without an exponent between 1e-6 and 1e21.
    ///
    /// JSON has no NaN or infinity. They render as the quoted strings `"NaN"`,
    /// `"+Inf"` and `"-Inf"`.
    pub fn canonical(&self) -> String {
        let mut out = String::new();
        self.write_canonical(&mut out);
        out
    }

    fn write_canonical(&self, out: &mut String) {
        match self {
            Value::Null => out.push_str("null"),
            Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Value::Int(i) => out.push_str(&i.to_string()),
            Value::Uint(u) => out.push_str(&u.to_string()),
            Value::Float(f) => write_float(*f, out),
            Value::String(s) => write_string(s, out),
            Value::List(items) => {
                out.push('[');
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    item.write_canonical(out);
                }
                out.push(']');
            }
            Value::Map(map) => write_map(map, out),
        }
    }
}

/// [`Value::canonical`] for a top-level map, such as a resource's params.
pub fn canonical_map(map: &BTreeMap<String, Value>) -> String {
    let mut out = String::new();
    write_map(map, &mut out);
    out
}

fn write_map(map: &BTreeMap<String, Value>, out: &mut String) {
    out.push('{');
    for (i, (key, value)) in map.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        write_string(key, out);
        out.push(':');
        value.write_canonical(out);
    }
    out.push('}');
}

fn write_float(f: f64, out: &mut String) {
    if f.is_nan() {
        return out.push_str("\"NaN\"");
    }
    if f.is_infinite() {
        return out.push_str(if f > 0.0 { "\"+Inf\"" } else { "\"-Inf\"" });
    }
    let abs = f.abs();
    if abs != 0.0 && !(1e-6..1e21).contains(&abs) {
        // Rust writes 1e21 and 1e-7; Go writes 1e+21 and 1e-7.
        let text = format!("{f:e}");
        match text.split_once('e') {
            Some((mantissa, exponent)) if !exponent.starts_with('-') => {
                out.push_str(mantissa);
                out.push_str("e+");
                out.push_str(exponent);
            }
            _ => out.push_str(&text),
        }
    } else {
        out.push_str(&f.to_string());
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '<' | '>' | '&' | '\u{2028}' | '\u{2029}' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::String(s.to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every expectation here was produced by running the value through Go's
    /// `json.Marshal`. Plugins receive values in this encoding, so the two
    /// have to agree to the byte.
    #[test]
    fn floats_render_as_go_does() {
        for (f, want) in [
            (0.0, "0"),
            (-0.0, "-0"),
            (1.0, "1"),
            (-1.0, "-1"),
            (80.0, "80"),
            (8080.0, "8080"),
            (0.5, "0.5"),
            (1.5, "1.5"),
            (6.02214, "6.02214"),
            (0.1, "0.1"),
            (1.0 / 3.0, "0.3333333333333333"),
            (1234.5678, "1234.5678"),
            (100000.0, "100000"),
            (1e6, "1000000"),
            (1e-5, "0.00001"),
            (1e-6, "0.000001"),
            (9.99e-7, "9.99e-7"),
            (1e-7, "1e-7"),
            (-2.5e-8, "-2.5e-8"),
            (1.5e-10, "1.5e-10"),
            (5e-324, "5e-324"),
            (1e20, "100000000000000000000"),
            (123456789012345680000.0, "123456789012345680000"),
            (1e21, "1e+21"),
            (1.5e21, "1.5e+21"),
            (1e22, "1e+22"),
            (1e100, "1e+100"),
            (f64::MAX, "1.7976931348623157e+308"),
        ] {
            assert_eq!(Value::Float(f).canonical(), want, "{f:e}");
        }
    }

    #[test]
    fn strings_render_as_go_does() {
        for (s, want) in [
            ("plain", r#""plain""#),
            ("q\"uote", r#""q\"uote""#),
            ("back\\slash", r#""back\\slash""#),
            ("nl\n", r#""nl\n""#),
            ("cr\r", r#""cr\r""#),
            ("tab\t", r#""tab\t""#),
            ("\u{8}\u{c}", r#""\b\f""#),
            ("\u{0}\u{1}\u{1f}", r#""\u0000\u0001\u001f""#),
            ("\u{7f}", "\"\u{7f}\""),
            ("<tag> & more", r#""\u003ctag\u003e \u0026 more""#),
            ("\u{2028}\u{2029}", r#""\u2028\u2029""#),
            ("héllo ✓ 😀", r#""héllo ✓ 😀""#),
            ("a/b", r#""a/b""#),
        ] {
            assert_eq!(Value::from(s).canonical(), want);
        }
    }

    #[test]
    fn maps_render_sorted_and_non_finite_floats_are_quoted() {
        let map: BTreeMap<String, Value> = [
            (
                "b".to_owned(),
                Value::List(vec![Value::Int(3), Value::Null, Value::Bool(true)]),
            ),
            ("a".to_owned(), Value::Uint(u64::MAX)),
            ("c".to_owned(), Value::Float(f64::NAN)),
            ("d".to_owned(), Value::Float(f64::NEG_INFINITY)),
        ]
        .into_iter()
        .collect();
        assert_eq!(
            canonical_map(&map),
            r#"{"a":18446744073709551615,"b":[3,null,true],"c":"NaN","d":"-Inf"}"#
        );
        assert_eq!(Value::Map(map.clone()).canonical(), canonical_map(&map));
    }
}
