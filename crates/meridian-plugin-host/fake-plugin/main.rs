//! A small, complete plugin for the host's tests.
//!
//! Its "tool" is a directory. Compiling turns each `file` resource into one
//! artifact file; applying copies the artifact into the work directory and
//! remembers what it wrote, so a plan is a real comparison against real
//! state. That is enough to exercise every part of the protocol without
//! installing anything.
//!
//! A few magic words in a file's content make it misbehave on purpose:
//! `FAIL` makes the tool fail at that file, `PANIC` panics the runner, and
//! `EXIT` kills the whole plugin process mid-run.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use meridian_sdk::{
    canonical_map, decode_map, serve, Applied, Artifact, Capabilities, Category, Change,
    ChangeAction, CompileError, Data, Emitted, Emitter, Log, Operations, Output, Plan, Plugin,
    ResourceGraph, RunContext, RunError, Runner, ServeError, Value, Warning,
};

struct Fake;

impl Emitter for Fake {
    fn name(&self) -> &str {
        "fake"
    }

    fn supported_resource_types(&self) -> Vec<String> {
        vec!["file".to_owned()]
    }

    fn capabilities(&self) -> Capabilities {
        Capabilities {
            native_notify: false,
            synthetic_notify: true,
            runtime_condition: false,
        }
    }

    fn category(&self) -> Category {
        Category::ConfigurationManagement
    }

    fn emit(&self, graph: &ResourceGraph, data: &Data) -> Result<Emitted, CompileError> {
        let mut emitted = Emitted::default();
        for resource in &graph.resources {
            if resource.r#type != "file" {
                return Err(CompileError::for_resource(
                    resource,
                    "unsupported-resource-type",
                    format!(
                        "the fake target has no mapping for type {:?}",
                        resource.r#type
                    ),
                ));
            }
            let content = match resource.params.get("content") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.canonical(),
                None => String::new(),
            };
            if resource.state == "absent" {
                emitted.warnings.push(Warning {
                    target: String::new(),
                    resource: resource.id.clone(),
                    msg: "an absent file is simply left out".to_owned(),
                });
                continue;
            }
            emitted
                .artifact
                .files
                .insert(format!("files/{}", resource.id), content);
        }
        // The resolved data is echoed so a test can see it arrive intact.
        if !data.is_empty() {
            emitted
                .artifact
                .files
                .insert("data.json".to_owned(), canonical_map(data));
        }
        Ok(emitted)
    }
}

struct FakeRunner {
    operations: Operations,
}

/// What the last apply wrote, by artifact path.
fn read_state(work_dir: &Path) -> Result<BTreeMap<String, Value>, RunError> {
    match fs::read(work_dir.join("state.json")) {
        Ok(bytes) => decode_map(&bytes).map_err(RunError::new),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(e) => Err(RunError::new(format!("reading the state file: {e}"))),
    }
}

fn diff(state: &BTreeMap<String, Value>, artifact: &Artifact) -> Vec<Change> {
    let mut changes = Vec::new();
    for (path, content) in &artifact.files {
        let action = match state.get(path) {
            None => ChangeAction::Create,
            Some(Value::String(old)) if old == content => continue,
            Some(_) => ChangeAction::Update,
        };
        changes.push(Change {
            resource: path.clone(),
            action,
            summary: format!("{} bytes", content.len()),
        });
    }
    for path in state.keys() {
        if !artifact.files.contains_key(path) {
            changes.push(Change {
                resource: path.clone(),
                action: ChangeAction::Delete,
                summary: String::new(),
            });
        }
    }
    changes
}

fn outputs_of(state: &BTreeMap<String, Value>, context: &RunContext) -> Vec<Output> {
    let mut outputs = vec![
        Output {
            name: "file_count".to_owned(),
            value: Value::Int(state.len() as i64),
            sensitive: false,
        },
        Output {
            name: "token".to_owned(),
            value: Value::from("s3cret-for-the-next-stage"),
            sensitive: true,
        },
    ];
    if let Some(greeting) = context.inputs.get("greeting") {
        outputs.push(Output {
            name: "greeting".to_owned(),
            value: greeting.clone(),
            sensitive: false,
        });
    }
    outputs
}

impl Runner for FakeRunner {
    fn operations(&self) -> Operations {
        self.operations
    }

    fn plan(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Plan, RunError> {
        let state = read_state(&context.work_dir)?;
        log.out(&format!(
            "comparing {} files for {} on {}",
            artifact.files.len(),
            context.document,
            context.host
        ));
        let changes = diff(&state, artifact);
        log.err(&format!("{} to change", changes.len()));
        Ok(Plan {
            changes,
            unknown: artifact.files.values().any(|c| c.contains("UNKNOWN")),
        })
    }

    fn apply(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Applied, RunError> {
        let mut state = read_state(&context.work_dir)?;
        let planned = diff(&state, artifact);
        let mut done = Vec::new();
        let save = |state: &BTreeMap<String, Value>| {
            fs::write(context.work_dir.join("state.json"), canonical_map(state))
                .map_err(|e| RunError::new(format!("writing the state file: {e}")))
        };

        for change in planned {
            let path = context.work_dir.join("current").join(&change.resource);
            log.out(&format!("{} {}", change.action, change.resource));
            if change.action == ChangeAction::Delete {
                let _ = fs::remove_file(&path);
                state.remove(&change.resource);
                done.push(change);
                continue;
            }
            let content = &artifact.files[&change.resource];
            if content.contains("PANIC") {
                panic!("the fake runner was told to panic");
            }
            if content.contains("EXIT") {
                eprintln!("the fake plugin was told to die");
                std::process::exit(7);
            }
            if content.contains("FAIL") {
                save(&state)?;
                return Err(RunError {
                    msg: format!("could not write {}", change.resource),
                    command: "fake-tool apply --password=***".to_owned(),
                    exit_code: Some(3),
                    output: "disk on fire\n".to_owned(),
                    changes: done,
                });
            }
            let written = path
                .parent()
                .map(fs::create_dir_all)
                .unwrap_or(Ok(()))
                .and_then(|()| fs::write(&path, content));
            if let Err(e) = written {
                save(&state)?;
                return Err(RunError {
                    msg: format!("writing {}: {e}", path.display()),
                    changes: done,
                    ..RunError::default()
                });
            }
            state.insert(change.resource.clone(), Value::from(content.as_str()));
            done.push(change);
        }
        save(&state)?;
        Ok(Applied {
            changes: done,
            outputs: outputs_of(&state, context),
        })
    }

    fn outputs(&self, context: &RunContext) -> Result<Vec<Output>, RunError> {
        let state = read_state(&context.work_dir)?;
        if state.is_empty() {
            return Err(RunError::new("nothing has been applied yet"));
        }
        Ok(outputs_of(&state, context))
    }
}

fn main() -> Result<(), ServeError> {
    // The declared operations are chosen by the test, so the host can be
    // checked against a plugin that offers only some of them.
    let operations = match std::env::var("FAKE_OPERATIONS").as_deref() {
        Ok("none") => None,
        Ok("plan") => Some(Operations {
            plan: true,
            apply: false,
            outputs: false,
        }),
        _ => Some(Operations::ALL),
    };
    let plugin = Plugin::new(Fake);
    match operations {
        Some(operations) => serve(plugin.with_runner(FakeRunner { operations })),
        None => serve(plugin),
    }
}
