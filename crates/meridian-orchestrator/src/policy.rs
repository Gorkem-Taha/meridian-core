//! Asking a policy whether a change may be made, before it is made.
//!
//! A policy engine is a program named `meridian-policy-<name>`, found where
//! plugins are found. For each stage it is started once, with the policy
//! files and directories as its arguments and one JSON document, the
//! [input](input), on standard input. It answers on standard output with one
//! JSON object:
//!
//! ```json
//! {"deny": ["why not"], "warn": ["worth knowing"]}
//! ```
//!
//! and exits 0. Either list may be missing. Anything else is a denial: an
//! engine that cannot be started, exits with another status, or answers with
//! something that is not that object has not said yes, and a run does not
//! proceed on a policy that could not be asked.
//!
//! The input holds what the document holds, secrets included, so it is
//! handed over on standard input and nowhere else: it is never written to
//! disk and never recorded. Neither are an engine's messages, which may
//! quote it. A run's record keeps how many there were.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use meridian_sdk::{decode_value, Artifact, Plan, ResourceGraph, Value};

/// What every policy engine's file name starts with.
pub const ENGINE_PREFIX: &str = "meridian-policy-";

/// The engine used when none is named.
pub const DEFAULT_ENGINE: &str = "opa";

/// Which policies a run is checked against, and by what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Policy {
    /// The engine's name, as in `meridian-policy-<name>`.
    pub engine: String,
    /// The engine's program.
    pub program: PathBuf,
    /// The policy files and directories, handed to the engine as they are.
    pub paths: Vec<PathBuf>,
}

impl Policy {
    /// Finds `meridian-policy-<engine>` in `dirs`, the directories plugins
    /// are looked for in.
    pub fn find(engine: &str, dirs: &[PathBuf], paths: Vec<PathBuf>) -> Result<Policy, String> {
        let safe = !engine.is_empty()
            && engine
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if !safe {
            return Err(format!("{engine:?} is not a policy engine name"));
        }
        for path in &paths {
            if !path.exists() {
                return Err(format!("the policy {} does not exist", path.display()));
            }
        }
        let file = format!("{ENGINE_PREFIX}{engine}");
        for dir in dirs {
            for candidate in [dir.join(&file), dir.join(format!("{file}.exe"))] {
                if candidate.is_file() {
                    return Ok(Policy {
                        engine: engine.to_owned(),
                        program: candidate,
                        paths,
                    });
                }
            }
        }
        let searched: Vec<String> = dirs.iter().map(|d| d.display().to_string()).collect();
        Err(format!(
            "no policy engine {engine:?}: there is no program named {file} in {}",
            if searched.is_empty() {
                "any plugin directory".to_owned()
            } else {
                searched.join(", ")
            }
        ))
    }

    /// Asks the engine about one stage.
    ///
    /// This never fails: an engine that could not be asked is a denial that
    /// says so.
    pub fn check(&self, input: &Value) -> Decisions {
        match self.ask(input) {
            Ok(decisions) => decisions,
            Err(why) => Decisions {
                deny: vec![format!(
                    "the policy engine {:?} could not be asked, and a policy that was not asked \
                     has not allowed anything: {why}",
                    self.engine
                )],
                warn: Vec::new(),
                asked: false,
            },
        }
    }

    fn ask(&self, input: &Value) -> Result<Decisions, String> {
        let program = self.program.display();
        let mut child = Command::new(&self.program)
            .args(&self.paths)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("starting {program}: {e}"))?;

        // Written from its own thread: an engine may answer before it has
        // read everything, and a full pipe on either side would otherwise
        // stop both.
        let mut stdin = child.stdin.take().expect("stdin was piped");
        let document = input.canonical().into_bytes();
        let writer = std::thread::spawn(move || {
            // An engine that stops reading has made up its mind; what it
            // says and how it exits is what counts.
            let _ = stdin.write_all(&document);
        });
        let mut stderr = child.stderr.take().expect("stderr was piped");
        let errors = std::thread::spawn(move || {
            let mut text = String::new();
            let _ = stderr.read_to_string(&mut text);
            text
        });
        let mut answer = Vec::new();
        let read = child
            .stdout
            .take()
            .expect("stdout was piped")
            .read_to_end(&mut answer);
        let status = child
            .wait()
            .map_err(|e| format!("waiting for {program}: {e}"))?;
        let _ = writer.join();
        let errors = errors.join().unwrap_or_default();
        read.map_err(|e| format!("reading the answer of {program}: {e}"))?;

        if !status.success() {
            let said = last_line(&errors);
            return Err(match status.code() {
                Some(code) => format!("{program} exited with status {code}{said}"),
                None => format!("{program} was stopped by a signal{said}"),
            });
        }
        Decisions::from_json(&answer).map_err(|why| format!("the answer of {program} {why}"))
    }
}

/// The last thing an engine said on standard error, as a clause to end a
/// sentence with. An engine's own failure is about the engine or the policy
/// files, so it is shown.
fn last_line(errors: &str) -> String {
    match errors.lines().rev().find(|line| !line.trim().is_empty()) {
        Some(line) => format!(": {}", line.trim()),
        None => String::new(),
    }
}

/// What a policy said about one stage.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Decisions {
    /// Why the change may not be made. Empty when it may.
    pub deny: Vec<String>,
    /// Things worth knowing that do not stop the change.
    pub warn: Vec<String>,
    /// Whether the engine answered. False when [`Decisions::deny`] holds
    /// the reason it could not be asked.
    pub asked: bool,
}

impl Decisions {
    /// Whether the policy allows the change.
    pub fn allowed(&self) -> bool {
        self.deny.is_empty()
    }

    /// Reads an engine's answer.
    pub fn from_json(json: &[u8]) -> Result<Decisions, String> {
        if json.iter().all(u8::is_ascii_whitespace) {
            return Err("is empty".to_owned());
        }
        let value = decode_value(json).map_err(|e| format!("is not JSON: {e}"))?;
        let Some(map) = value.as_map() else {
            return Err(format!(
                "is {}, and it has to be an object with `deny` and `warn`",
                value.describe()
            ));
        };
        if let Some(key) = map.keys().find(|k| *k != "deny" && *k != "warn") {
            return Err(format!(
                "has the key {key:?}, and only `deny` and `warn` are understood"
            ));
        }
        let messages = |key: &str| -> Result<Vec<String>, String> {
            let Some(value) = map.get(key) else {
                return Ok(Vec::new());
            };
            let Some(list) = value.as_list() else {
                return Err(format!(
                    "has `{key}` as {}, and it has to be a list of messages",
                    value.describe()
                ));
            };
            list.iter()
                .map(|item| match item {
                    Value::String(text) => Ok(text.clone()),
                    // Conftest lets a rule answer with an object that
                    // carries its message under `msg`.
                    Value::Map(fields) => match fields.get("msg") {
                        Some(Value::String(text)) => Ok(text.clone()),
                        _ => Err(format!(
                            "has an object in `{key}` without a `msg` that is a string"
                        )),
                    },
                    other => Err(format!(
                        "has {} in `{key}`, and a message has to be a string",
                        other.describe()
                    )),
                })
                .collect()
        };
        Ok(Decisions {
            deny: messages("deny")?,
            warn: messages("warn")?,
            asked: true,
        })
    }
}

/// What a stage's policy input is built from.
pub struct Subject<'a> {
    /// The run's mode: `plan` or `apply`.
    pub mode: &'a str,
    pub run: &'a str,
    /// `Infrastructure` or `ResourceSet`.
    pub kind: &'a str,
    pub name: &'a str,
    pub target: &'a str,
    /// The target's category, as its plugin declares it.
    pub category: &'a str,
    /// The machine the stage configures; empty when there is none.
    pub host: &'a str,
    pub labels: &'a BTreeMap<String, String>,
    /// The resources as they were handed to the target, after every `when`
    /// was decided.
    pub graph: &'a ResourceGraph,
    pub artifact: &'a Artifact,
    /// What the tool says applying would change, when the target can say.
    pub plan: Option<&'a Plan>,
}

/// The document a policy is asked about. Its shape is part of Meridian's
/// interface, documented in `docs/reference/policy.md`: a field is added,
/// never renamed or removed.
pub fn input(subject: &Subject<'_>) -> Value {
    let text = |s: &str| Value::from(s);
    let map = |fields: Vec<(&str, Value)>| {
        Value::Map(
            fields
                .into_iter()
                .map(|(key, value)| (key.to_owned(), value))
                .collect(),
        )
    };
    let resources = subject
        .graph
        .resources
        .iter()
        .map(|resource| {
            map(vec![
                ("id", text(&resource.id)),
                ("type", text(&resource.r#type)),
                ("state", text(&resource.state)),
                ("params", Value::Map(resource.params.clone())),
            ])
        })
        .collect();
    let files = subject
        .artifact
        .files
        .iter()
        .map(|(path, content)| (path.clone(), text(content)))
        .collect();
    let plan = match subject.plan {
        None => Value::Null,
        Some(plan) => map(vec![
            (
                "changes",
                Value::List(
                    plan.changes
                        .iter()
                        .map(|change| {
                            map(vec![
                                ("resource", text(&change.resource)),
                                ("action", text(change.action.as_str())),
                                ("summary", text(&change.summary)),
                            ])
                        })
                        .collect(),
                ),
            ),
            ("unknown", Value::Bool(plan.unknown)),
        ]),
    };
    map(vec![
        (
            "run",
            map(vec![
                ("id", text(subject.run)),
                ("mode", text(subject.mode)),
            ]),
        ),
        (
            "stage",
            map(vec![
                ("name", text(subject.name)),
                ("kind", text(subject.kind)),
                ("target", text(subject.target)),
                ("category", text(subject.category)),
                ("host", text(subject.host)),
                (
                    "labels",
                    Value::Map(
                        subject
                            .labels
                            .iter()
                            .map(|(key, value)| (key.clone(), text(value)))
                            .collect(),
                    ),
                ),
            ]),
        ),
        ("resources", Value::List(resources)),
        ("artifact", map(vec![("files", Value::Map(files))])),
        ("plan", plan),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_answer_is_read_and_anything_else_is_refused() {
        let ok = Decisions::from_json(br#"{"deny": ["no"], "warn": [{"msg": "hm"}]}"#).unwrap();
        assert_eq!(ok.deny, ["no"]);
        assert_eq!(ok.warn, ["hm"]);
        assert!(ok.asked && !ok.allowed());

        let empty = Decisions::from_json(b"{}").unwrap();
        assert!(empty.allowed() && empty.warn.is_empty());

        for (answer, says) in [
            ("", "is empty"),
            ("nope", "is not JSON"),
            ("[]", "has to be an object"),
            (r#"{"deny": "no"}"#, "has to be a list"),
            (r#"{"deny": [1]}"#, "has to be a string"),
            (r#"{"deny": [{"message": "x"}]}"#, "without a `msg`"),
            (r#"{"allow": true}"#, "only `deny` and `warn`"),
        ] {
            let error = Decisions::from_json(answer.as_bytes()).unwrap_err();
            assert!(error.contains(says), "{answer:?} gave {error:?}");
        }
    }

    #[test]
    fn an_engine_is_found_by_name_and_a_bad_name_or_missing_policy_is_refused() {
        let dir = std::env::temp_dir().join(format!("meridian-policy-find-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("meridian-policy-yes"), "").unwrap();
        let dirs = vec![dir.clone()];

        let found = Policy::find("yes", &dirs, vec![dir.clone()]).unwrap();
        assert_eq!(found.program, dir.join("meridian-policy-yes"));

        let missing = Policy::find("no", &dirs, Vec::new()).unwrap_err();
        assert!(
            missing.contains("no program named meridian-policy-no"),
            "{missing}"
        );
        let unsafe_name = Policy::find("../x", &dirs, Vec::new()).unwrap_err();
        assert!(
            unsafe_name.contains("is not a policy engine name"),
            "{unsafe_name}"
        );
        let no_file = Policy::find("yes", &dirs, vec![dir.join("absent.rego")]).unwrap_err();
        assert!(no_file.contains("does not exist"), "{no_file}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_engine_that_cannot_be_started_is_a_denial() {
        let policy = Policy {
            engine: "gone".to_owned(),
            program: PathBuf::from("/nonexistent/meridian-policy-gone"),
            paths: Vec::new(),
        };
        let decisions = policy.check(&Value::Null);
        assert!(!decisions.allowed() && !decisions.asked);
        assert!(
            decisions.deny[0].contains("could not be asked"),
            "{:?}",
            decisions.deny
        );
    }
}
