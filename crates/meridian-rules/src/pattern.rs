//! The two pattern languages a rules file uses: key patterns, which select
//! values inside a layer, and globs, which select layer files and strings.

use std::fmt;

/// One segment of a key pattern.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Segment {
    /// Exactly this key.
    Literal(String),
    /// Any one key.
    One,
    /// Any run of keys, including none.
    Any,
}

/// A dotted path pattern over the keys of a layer, such as `variables.**` or
/// `**.password`.
///
/// A pattern selects the key it names **and everything beneath it**, so
/// `variables` and `variables.**` mean the same. That is deliberate: a rule
/// that protects a mapping protects what is in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPattern {
    source: String,
    segments: Vec<Segment>,
}

impl KeyPattern {
    /// Parses a pattern. Segments are separated by dots; `*` is any one key,
    /// `**` any run of keys; a key that itself contains a dot, a star or a
    /// quote is written in double quotes, with `\"` and `\\` inside.
    pub fn parse(source: &str) -> Result<Self, String> {
        let mut segments = Vec::new();
        for (text, quoted) in split_key(source)? {
            let segment = match (quoted, text.as_str()) {
                (false, "*") => Segment::One,
                (false, "**") => Segment::Any,
                (false, "") => return Err(format!("the key pattern {source:?} has an empty segment")),
                (false, t) if t.contains('*') => {
                    return Err(format!(
                        "the key pattern {source:?} mixes `*` with other characters in the segment {t:?}; \
                         a segment is a key, `*` or `**` (quote a key that contains a star)"
                    ))
                }
                _ => Segment::Literal(text),
            };
            // Two `**` in a row mean what one means.
            if segment == Segment::Any && segments.last() == Some(&Segment::Any) {
                continue;
            }
            segments.push(segment);
        }
        if segments.is_empty() {
            return Err("a key pattern cannot be empty".to_owned());
        }
        Ok(KeyPattern {
            source: source.to_owned(),
            segments,
        })
    }

    /// Whether the pattern has no wildcard, so it names exactly one key.
    pub fn is_exact(&self) -> bool {
        self.segments
            .iter()
            .all(|s| matches!(s, Segment::Literal(_)))
    }

    /// The keys of an exact pattern, outermost first.
    pub fn exact_path(&self) -> Option<Vec<&str>> {
        self.segments
            .iter()
            .map(|s| match s {
                Segment::Literal(l) => Some(l.as_str()),
                _ => None,
            })
            .collect()
    }

    /// Whether the pattern selects `path`: it matches the path itself or one
    /// of the keys above it.
    pub fn selects<S: AsRef<str>>(&self, path: &[S]) -> bool {
        (1..=path.len()).any(|n| matches(&self.segments, &path[..n]))
    }

    /// Whether the pattern could select something beneath `path` without
    /// selecting `path` itself. A value written at such a path is not one the
    /// pattern names, yet it stands in for everything the pattern does name.
    pub fn reaches_below<S: AsRef<str>>(&self, path: &[S]) -> bool {
        !self.selects(path) && prefix_matches(&self.segments, path)
    }
}

impl fmt::Display for KeyPattern {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

fn matches<S: AsRef<str>>(pattern: &[Segment], path: &[S]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((Segment::Any, rest)) => (0..=path.len()).any(|skip| matches(rest, &path[skip..])),
        Some((first, rest)) => match path.split_first() {
            None => false,
            Some((key, tail)) => {
                let here = match first {
                    Segment::Literal(l) => l == key.as_ref(),
                    _ => true,
                };
                here && matches(rest, tail)
            }
        },
    }
}

/// Whether `path` could be the start of something `pattern` matches.
fn prefix_matches<S: AsRef<str>>(pattern: &[Segment], path: &[S]) -> bool {
    let Some((key, tail)) = path.split_first() else {
        return true;
    };
    match pattern.split_first() {
        None => false,
        Some((Segment::Any, rest)) => {
            // `**` swallows this key, or stands for nothing.
            prefix_matches(pattern, tail) || prefix_matches(rest, path)
        }
        Some((Segment::Literal(l), rest)) => l == key.as_ref() && prefix_matches(rest, tail),
        Some((Segment::One, rest)) => prefix_matches(rest, tail),
    }
}

/// Splits a dotted key into segments, honouring double quotes. The flag says
/// whether the segment was quoted, which is what makes `"*"` a key.
fn split_key(source: &str) -> Result<Vec<(String, bool)>, String> {
    let mut out = Vec::new();
    let mut chars = source.chars().peekable();
    loop {
        let mut text = String::new();
        let quoted = chars.peek() == Some(&'"');
        if quoted {
            chars.next();
            loop {
                match chars.next() {
                    None => return Err(format!("the key pattern {source:?} has an unclosed quote")),
                    Some('"') => break,
                    Some('\\') => match chars.next() {
                        Some(c @ ('"' | '\\')) => text.push(c),
                        _ => {
                            return Err(format!(
                                "the key pattern {source:?} has a backslash that is not `\\\"` or `\\\\`"
                            ))
                        }
                    },
                    Some(c) => text.push(c),
                }
            }
            match chars.peek() {
                None | Some('.') => {}
                Some(_) => {
                    return Err(format!(
                        "the key pattern {source:?} has text after a closing quote; quote the whole segment"
                    ))
                }
            }
        } else {
            while let Some(&c) = chars.peek() {
                if c == '.' {
                    break;
                }
                if c == '"' {
                    return Err(format!(
                        "the key pattern {source:?} has a quote in the middle of a segment; quote the whole segment"
                    ));
                }
                text.push(c);
                chars.next();
            }
        }
        out.push((text, quoted));
        match chars.next() {
            None => return Ok(out),
            Some(_dot) => {}
        }
    }
}

/// Writes a path the way a pattern would have to be written to name it, so a
/// refusal can be pasted into a rules file.
pub fn display_path<S: AsRef<str>>(path: &[S]) -> String {
    path.iter()
        .map(|key| {
            let key = key.as_ref();
            if key.is_empty() || key.contains(['.', '*', '"', '\\']) {
                format!("\"{}\"", key.replace('\\', "\\\\").replace('"', "\\\""))
            } else {
                key.to_owned()
            }
        })
        .collect::<Vec<_>>()
        .join(".")
}

/// A glob over a layer's path: `*` and `?` stay inside one path element,
/// `**` as a whole element is any run of elements, including none.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathGlob {
    source: String,
    elements: Vec<String>,
}

impl PathGlob {
    pub fn parse(source: &str) -> Result<Self, String> {
        let elements: Vec<String> = source
            .split('/')
            .filter(|e| !e.is_empty() && *e != ".")
            .map(str::to_owned)
            .collect();
        if elements.is_empty() {
            return Err("a layer path pattern cannot be empty".to_owned());
        }
        if elements.iter().any(|e| e == "..") {
            return Err(format!(
                "the layer path pattern {source:?} contains `..`; patterns are matched against cleaned paths"
            ));
        }
        if let Some(bad) = elements.iter().find(|e| e.contains("**") && *e != "**") {
            return Err(format!(
                "the layer path pattern {source:?} uses `**` inside the element {bad:?}; `**` must be a whole element"
            ));
        }
        Ok(PathGlob {
            source: source.to_owned(),
            elements,
        })
    }

    pub fn matches(&self, path: &str) -> bool {
        let parts: Vec<&str> = path
            .split('/')
            .filter(|e| !e.is_empty() && *e != ".")
            .collect();
        path_matches(&self.elements, &parts)
    }
}

impl fmt::Display for PathGlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.source)
    }
}

fn path_matches(pattern: &[String], path: &[&str]) -> bool {
    match pattern.split_first() {
        None => path.is_empty(),
        Some((first, rest)) if first == "**" => {
            (0..=path.len()).any(|skip| path_matches(rest, &path[skip..]))
        }
        Some((first, rest)) => match path.split_first() {
            None => false,
            Some((part, tail)) => glob(first, part, false) && path_matches(rest, tail),
        },
    }
}

/// A plain glob over text: `*` is any run of characters, `?` any one.
pub fn glob(pattern: &str, text: &str, ignore_case: bool) -> bool {
    let fold = |s: &str| -> Vec<char> {
        if ignore_case {
            s.chars().flat_map(char::to_lowercase).collect()
        } else {
            s.chars().collect()
        }
    };
    let (p, t) = (fold(pattern), fold(text));
    // The classic two-pointer match with one backtrack point: linear in
    // practice, and never exponential however many stars a pattern has.
    let (mut pi, mut ti) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) && p[pi] != '*' {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = Some((pi, ti));
            pi += 1;
        } else if let Some((sp, st)) = star {
            pi = sp + 1;
            ti = st + 1;
            star = Some((sp, st + 1));
        } else {
            return false;
        }
    }
    p[pi..].iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(p: &str) -> KeyPattern {
        KeyPattern::parse(p).unwrap()
    }

    #[test]
    fn a_pattern_selects_its_key_and_everything_beneath() {
        assert!(key("variables").selects(&["variables"]));
        assert!(key("variables").selects(&["variables", "db", "host"]));
        assert!(key("variables.**").selects(&["variables", "db"]));
        assert!(key("variables.**").selects(&["variables"]));
        assert!(!key("variables").selects(&["variable"]));
        assert!(!key("variables.db").selects(&["variables"]));
        assert!(!key("variables.db").selects(&["variables", "cache"]));
    }

    #[test]
    fn wildcards() {
        assert!(key("*.password").selects(&["db", "password"]));
        assert!(!key("*.password").selects(&["password"]));
        assert!(!key("*.password").selects(&["a", "b", "password"]));
        assert!(key("**.password").selects(&["password"]));
        assert!(key("**.password").selects(&["a", "b", "password", "salt"]));
        assert!(key("**").selects(&["anything"]));
        assert!(key("a.**.z").selects(&["a", "z"]));
        assert!(key("a.**.z").selects(&["a", "b", "c", "z"]));
        assert!(!key("a.**.z").selects(&["a", "b"]));
    }

    #[test]
    fn keys_are_case_sensitive_like_the_data() {
        assert!(!key("variables").selects(&["Variables"]));
    }

    #[test]
    fn quoting() {
        assert!(key("variables.\"a.b\"").selects(&["variables", "a.b"]));
        assert!(!key("variables.\"a.b\"").selects(&["variables", "a", "b"]));
        assert!(key("\"*\"").selects(&["*"]));
        assert!(!key("\"*\"").selects(&["x"]));
        assert_eq!(
            display_path(&["variables", "a.b", "c"]),
            "variables.\"a.b\".c"
        );
        let round = display_path(&["we\"ird\\", "*"]);
        assert!(key(&round).selects(&["we\"ird\\", "*"]));
    }

    #[test]
    fn bad_patterns_are_refused() {
        for bad in [
            "", "a..b", ".a", "a.", "a*b", "\"open", "\"a\"b", "a\"b\"", "\"a\\n\"",
        ] {
            assert!(KeyPattern::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn reaches_below() {
        assert!(key("secrets.db.password").reaches_below(&["secrets"]));
        assert!(key("secrets.db.password").reaches_below(&["secrets", "db"]));
        assert!(!key("secrets.db.password").reaches_below(&["secrets", "db", "password"]));
        assert!(!key("secrets.db.password").reaches_below(&["secrets", "cache"]));
        assert!(key("**.password").reaches_below(&["anything", "at", "all"]));
        assert!(!key("secrets").reaches_below(&["secrets"]));
        assert!(key("*.password").reaches_below(&["x"]));
        assert!(!key("*.password").reaches_below(&["x", "y"]));
    }

    #[test]
    fn path_globs() {
        let g = |p: &str| PathGlob::parse(p).unwrap();
        assert!(g("common.yaml").matches("common.yaml"));
        assert!(!g("common.yaml").matches("x/common.yaml"));
        assert!(g("node/*").matches("node/web01.yaml"));
        assert!(!g("node/*").matches("node/a/web01.yaml"));
        assert!(g("node/**").matches("node/a/web01.yaml"));
        assert!(g("node/**").matches("node"));
        assert!(g("**/*.yaml").matches("a/b/c.yaml"));
        assert!(g("**/*.yaml").matches("c.yaml"));
        assert!(g("**").matches("anything/at/all"));
        assert!(!g("Node/*").matches("node/x.yaml"));
        assert!(PathGlob::parse("a/../b").is_err());
        assert!(PathGlob::parse("a/**b").is_err());
        assert!(PathGlob::parse("").is_err());
    }

    #[test]
    fn text_globs() {
        assert!(glob("*:latest", "nginx:latest", false));
        assert!(!glob("*:latest", "nginx:1.27", false));
        assert!(!glob("*:latest", "nginx:LATEST", false));
        assert!(glob("*:latest", "nginx:LATEST", true));
        assert!(glob("a?c", "abc", false));
        assert!(glob("", "", false));
        assert!(!glob("", "x", false));
        assert!(glob("***a***", "bab", false));
        // Many stars against a long non-match must return, not hang.
        assert!(!glob(&"*a".repeat(40), &"a".repeat(39), false));
    }
}
