//! `meridian.lock`: exactly which version of each module a project uses, and
//! the hash its content must have.

use std::path::Path;

use meridian_ir::Value;

use crate::Error;

/// The lockfile's name, beside the manifest.
pub const LOCK_FILE: &str = "meridian.lock";

/// The lockfile format this build writes. A newer build reads an older
/// lockfile; an older build refuses a newer one instead of guessing at it.
pub const LOCK_VERSION: u64 = 1;

/// One pinned module.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Locked {
    pub name: String,
    /// Empty when the module declares no version, which only a `path` module
    /// or a git `ref` may do.
    pub version: String,
    /// The source's locator: `git+…`, `oci+…` or `path+…`.
    pub source: String,
    /// Exactly what was fetched: a commit, or a manifest digest. Empty for a
    /// `path` module.
    pub resolved: String,
    /// The content hash. Empty for a `path` module, which is a working copy.
    pub integrity: String,
    /// The modules this one requires, by name, sorted.
    pub requires: Vec<String>,
}

/// A parsed lockfile.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Lock {
    /// Sorted by name.
    pub modules: Vec<Locked>,
}

/// Quotes a string for YAML when leaving it bare could change what it is.
fn scalar(s: &str) -> String {
    // Starting with a letter rules out everything YAML would read as a
    // number, a list item or an alias.
    let plain = s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'.' | b'/' | b'_' | b'-' | b'+' | b':' | b'@')
        })
        && !s.ends_with(':')
        && !matches!(
            s.to_ascii_lowercase().as_str(),
            "true" | "false" | "null" | "yes" | "no" | "on" | "off" | "y" | "n"
        );
    if plain {
        s.to_owned()
    } else {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    }
}

impl Lock {
    pub fn get(&self, name: &str) -> Option<&Locked> {
        self.modules.iter().find(|m| m.name == name)
    }

    /// The lockfile's text. The same modules always render to the same
    /// bytes: sorted, with no timestamps, so a diff shows only what moved.
    pub fn render(&self) -> String {
        let mut modules = self.modules.clone();
        modules.sort_by(|a, b| a.name.cmp(&b.name));
        let mut out = format!(
            "# Written by `meridian module`. Commit it; do not edit it.\nlockVersion: {LOCK_VERSION}\n"
        );
        if modules.is_empty() {
            out.push_str("modules: []\n");
            return out;
        }
        out.push_str("modules:\n");
        for m in &modules {
            out.push_str(&format!("  - name: {}\n", scalar(&m.name)));
            if !m.version.is_empty() {
                out.push_str(&format!("    version: {}\n", scalar(&m.version)));
            }
            out.push_str(&format!("    source: {}\n", scalar(&m.source)));
            if !m.resolved.is_empty() {
                out.push_str(&format!("    resolved: {}\n", scalar(&m.resolved)));
            }
            if !m.integrity.is_empty() {
                out.push_str(&format!("    integrity: {}\n", scalar(&m.integrity)));
            }
            if !m.requires.is_empty() {
                let mut requires = m.requires.clone();
                requires.sort();
                let list: Vec<String> = requires.iter().map(|r| scalar(r)).collect();
                out.push_str(&format!("    requires: [{}]\n", list.join(", ")));
            }
        }
        out
    }

    /// Reads the lockfile in `dir`; `None` when there is none.
    pub fn load(dir: &Path) -> Result<Option<Lock>, Error> {
        let path = dir.join(LOCK_FILE);
        let name = path.display().to_string();
        match std::fs::read(&path) {
            Ok(data) => Lock::parse(&name, &data).map(Some),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(Error::new(format!("{name}: cannot read file: {e}"))),
        }
    }

    /// Parses a lockfile. `file` is used only in messages.
    pub fn parse(file: &str, data: &[u8]) -> Result<Lock, Error> {
        let bad = |msg: String| Error::new(format!("{file}: {msg}"));
        let value = meridian_ir::parse_value(file, data).map_err(|e| Error::new(e.to_string()))?;
        let Some(top) = value.as_map() else {
            return Err(bad(
                "the lockfile is not a mapping; run `meridian module install` to write it again"
                    .into(),
            ));
        };
        match top.get("lockVersion") {
            Some(Value::Int(v)) if *v >= 1 && (*v as u64) <= LOCK_VERSION => {}
            Some(Value::Int(v)) if *v >= 1 => {
                return Err(bad(format!(
                    "the lockfile is format {v}, and this build reads up to {LOCK_VERSION}; it \
                     was written by a newer meridian. Upgrade meridian"
                )))
            }
            _ => return Err(bad("lockVersion: expected a whole number".into())),
        }
        let mut lock = Lock::default();
        let items = match top.get("modules") {
            None | Some(Value::Null) => &[][..],
            Some(Value::List(items)) => items.as_slice(),
            Some(_) => return Err(bad("modules: expected a list".into())),
        };
        for (i, item) in items.iter().enumerate() {
            let Some(map) = item.as_map() else {
                return Err(bad(format!("modules[{i}]: expected a mapping")));
            };
            let text = |key: &str| -> Result<String, Error> {
                match map.get(key) {
                    None | Some(Value::Null) => Ok(String::new()),
                    Some(Value::String(s)) => Ok(s.clone()),
                    Some(_) => Err(bad(format!("modules[{i}].{key}: expected text"))),
                }
            };
            let mut requires = Vec::new();
            match map.get("requires") {
                None | Some(Value::Null) => {}
                Some(Value::List(names)) => {
                    for n in names {
                        match n.as_str() {
                            Some(n) => requires.push(n.to_owned()),
                            None => {
                                return Err(bad(format!("modules[{i}].requires: expected names")))
                            }
                        }
                    }
                }
                Some(_) => return Err(bad(format!("modules[{i}].requires: expected a list"))),
            }
            let locked = Locked {
                name: text("name")?,
                version: text("version")?,
                source: text("source")?,
                resolved: text("resolved")?,
                integrity: text("integrity")?,
                requires,
            };
            if locked.name.is_empty() || locked.source.is_empty() {
                return Err(bad(format!("modules[{i}]: needs a name and a source")));
            }
            lock.modules.push(locked);
        }
        lock.modules.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Lock {
        Lock {
            modules: vec![
                Locked {
                    name: "acme/nginx".into(),
                    version: "1.4.2".into(),
                    source: "git+https://git.example.com/acme/nginx.git//modules/nginx".into(),
                    resolved: "4f2a9c1e5b3d8f6a7c2e4b1d0f9a8c79c1d7be0a".into(),
                    integrity: "h1:91c4".into(),
                    requires: vec!["acme/local".into(), "acme/base".into()],
                },
                Locked {
                    name: "acme/base".into(),
                    version: "2.0.0".into(),
                    source: "oci+registry.example.com/acme/base".into(),
                    resolved: "sha256:ab12".into(),
                    integrity: "h1:6b1f".into(),
                    requires: vec![],
                },
                Locked {
                    name: "acme/local".into(),
                    source: "path+../local dir".into(),
                    ..Locked::default()
                },
            ],
        }
    }

    #[test]
    fn the_lockfile_is_sorted_and_reads_back_as_written() {
        let text = sample().render();
        assert_eq!(
            text,
            "# Written by `meridian module`. Commit it; do not edit it.\n\
             lockVersion: 1\n\
             modules:\n  \
             - name: acme/base\n    \
             version: \"2.0.0\"\n    \
             source: oci+registry.example.com/acme/base\n    \
             resolved: sha256:ab12\n    \
             integrity: h1:6b1f\n  \
             - name: acme/local\n    \
             source: \"path+../local dir\"\n  \
             - name: acme/nginx\n    \
             version: \"1.4.2\"\n    \
             source: git+https://git.example.com/acme/nginx.git//modules/nginx\n    \
             resolved: \"4f2a9c1e5b3d8f6a7c2e4b1d0f9a8c79c1d7be0a\"\n    \
             integrity: h1:91c4\n    \
             requires: [acme/base, acme/local]\n"
        );
        let read = Lock::parse("meridian.lock", text.as_bytes()).unwrap();
        assert_eq!(read.render(), text);
        assert_eq!(read.get("acme/nginx").unwrap().version, "1.4.2");
        assert_eq!(read.get("acme/local").unwrap().source, "path+../local dir");
    }

    #[test]
    fn awkward_values_survive_the_round_trip() {
        for value in [
            "1.4",
            "12345678",
            "1e5",
            "true",
            "a: b",
            "0.1.0-rc.1",
            "he said \"hi\"",
            "#x",
        ] {
            let lock = Lock {
                modules: vec![Locked {
                    name: "a".into(),
                    version: value.into(),
                    source: "path+x".into(),
                    resolved: value.into(),
                    ..Locked::default()
                }],
            };
            let read = Lock::parse("meridian.lock", lock.render().as_bytes()).unwrap();
            assert_eq!(read, lock, "{value}");
        }
    }

    #[test]
    fn an_empty_lockfile_is_valid_and_a_newer_one_is_refused() {
        let empty = Lock::default().render();
        assert!(empty.ends_with("lockVersion: 1\nmodules: []\n"));
        assert_eq!(
            Lock::parse("meridian.lock", empty.as_bytes()).unwrap(),
            Lock::default()
        );
        assert_eq!(
            Lock::parse("meridian.lock", b"lockVersion: 2\nmodules: []\n")
                .unwrap_err()
                .to_string(),
            "meridian.lock: the lockfile is format 2, and this build reads up to 1; it was \
             written by a newer meridian. Upgrade meridian"
        );
        assert_eq!(
            Lock::parse("meridian.lock", b"modules: []\n")
                .unwrap_err()
                .to_string(),
            "meridian.lock: lockVersion: expected a whole number"
        );
    }
}
