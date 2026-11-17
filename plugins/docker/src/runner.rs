//! The half that brings a Compose project up with `docker compose`. It sits
//! outside the compile chain: nothing here is reached by a compile.

use meridian_sdk::tool::{write_artifact, Tool};
use meridian_sdk::{
    decode_value, Applied, Artifact, Change, ChangeAction, Log, NoLog, Operations, Output, Plan,
    RunContext, RunError, Runner, Value,
};

use crate::{COMPOSE_PATH, PROJECT_PATH};

/// Names the program to run, when it is not `docker` on `PATH`.
pub const PROGRAM_VARIABLE: &str = "MERIDIAN_DOCKER";
/// Extra arguments for `docker compose up`, split on whitespace: `--wait`,
/// `--build`, `--pull always`.
pub const ARGS_VARIABLE: &str = "MERIDIAN_DOCKER_ARGS";

/// Brings a compiled Compose project up on a Docker daemon.
///
/// A plan is `docker compose --dry-run up`: Compose works out what it would
/// create, recreate and remove, and touches nothing. An apply is
/// `docker compose up --detach --remove-orphans`, so a service the document
/// no longer declares is removed.
pub struct DockerRunner {
    pub program: String,
    /// Arguments added to `docker compose up`, after Meridian's own.
    pub extra_args: Vec<String>,
}

/// One piece of an output, as `project.json` records it.
enum Part {
    Text(String),
    Port { service: String, port: String },
    Inspect { service: String, path: String },
}

/// What `project.json` says.
struct Project {
    name: String,
    host: String,
    context: String,
    services: Vec<String>,
    outputs: Vec<(String, Vec<Part>)>,
}

impl Project {
    fn read(text: &str) -> Result<Project, RunError> {
        let bad = || RunError::new(format!("{PROJECT_PATH} is not one this plugin compiled"));
        let value = decode_value(text.as_bytes()).map_err(|_| bad())?;
        let map = value.as_map().ok_or_else(bad)?;
        let string = |map: &std::collections::BTreeMap<String, Value>, key: &str| {
            map.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let mut outputs = Vec::new();
        for output in map.get("outputs").and_then(Value::as_list).unwrap_or(&[]) {
            let output = output.as_map().ok_or_else(bad)?;
            let mut parts = Vec::new();
            for part in output.get("parts").and_then(Value::as_list).unwrap_or(&[]) {
                let part = part.as_map().ok_or_else(bad)?;
                let service = string(part, "service");
                parts.push(
                    if let Some(text) = part.get("text").and_then(Value::as_str) {
                        Part::Text(text.to_owned())
                    } else if part.contains_key("port") {
                        Part::Port {
                            service,
                            port: string(part, "port"),
                        }
                    } else {
                        Part::Inspect {
                            service,
                            path: string(part, "inspect"),
                        }
                    },
                );
            }
            outputs.push((string(output, "name"), parts));
        }
        let project = Project {
            name: string(map, "name"),
            host: string(map, "host"),
            context: string(map, "context"),
            services: map
                .get("services")
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .iter()
                .filter_map(|s| Some(s.as_str()?.to_owned()))
                .collect(),
            outputs,
        };
        if project.name.is_empty() {
            return Err(bad());
        }
        Ok(project)
    }

    /// The document's id for something Compose names in its report, or the
    /// name itself when it is not one of the document's.
    fn resource(&self, kind: &str, name: &str) -> String {
        let own = if kind == "Container" {
            name.strip_prefix(&format!("{}-", self.name))
                .and_then(|rest| rest.rsplit_once('-'))
                .filter(|(_, replica)| replica.chars().all(|c| c.is_ascii_digit()))
                .map(|(service, _)| service)
                .filter(|service| self.services.iter().any(|s| s == service))
        } else {
            name.strip_prefix(&format!("{}_", self.name))
        };
        own.unwrap_or(name).to_owned()
    }
}

impl DockerRunner {
    /// The runner as the environment configures it.
    pub fn from_env() -> Self {
        DockerRunner {
            program: std::env::var(PROGRAM_VARIABLE)
                .ok()
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| "docker".to_owned()),
            extra_args: std::env::var(ARGS_VARIABLE)
                .map(|v| v.split_whitespace().map(str::to_owned).collect())
                .unwrap_or_default(),
        }
    }

    /// `docker`, aimed at the project's daemon.
    fn docker(&self, context: &RunContext, project: &Project) -> Tool {
        let mut tool = Tool::new(&self.program).dir(&context.work_dir);
        if !project.host.is_empty() {
            tool = tool.args(["--host", &project.host]);
        }
        if !project.context.is_empty() {
            tool = tool.args(["--context", &project.context]);
        }
        tool
    }

    /// `docker compose`, aimed at the project.
    fn compose(&self, context: &RunContext, project: &Project) -> Tool {
        self.docker(context, project)
            .args(["compose", "--project-name", &project.name])
            .args(["--file", COMPOSE_PATH])
    }

    fn prepare(&self, context: &RunContext, artifact: &Artifact) -> Result<Project, RunError> {
        let (Some(_), Some(settings)) = (
            artifact.files.get(COMPOSE_PATH),
            artifact.files.get(PROJECT_PATH),
        ) else {
            return Err(RunError::new(format!(
                "the artifact has no {COMPOSE_PATH} and {PROJECT_PATH}; it was not compiled by this plugin"
            )));
        };
        let project = Project::read(settings)?;
        write_artifact(&context.work_dir, artifact)?;
        Ok(project)
    }

    /// One part of an output, read from the running project.
    fn read_part(
        &self,
        context: &RunContext,
        project: &Project,
        output: &str,
        part: &Part,
    ) -> Result<Value, RunError> {
        let what = format!("docker could not read output {output:?}");
        match part {
            Part::Text(text) => Ok(Value::String(text.clone())),
            Part::Port { service, port } => {
                let (number, protocol) = port.split_once('/').unwrap_or((port, "tcp"));
                let finished = self
                    .compose(context, project)
                    .args(["port", "--protocol", protocol, service, number])
                    .capture_stdout()
                    .run_ok(&what, &NoLog)?;
                // `0.0.0.0:32768`, or `[::]:32768`.
                let published = finished.stdout.lines().next().unwrap_or("").trim();
                match published.rsplit_once(':') {
                    Some((_, host_port)) if !host_port.is_empty() => {
                        Ok(Value::String(host_port.to_owned()))
                    }
                    _ => Err(RunError::new(format!(
                        "output {output:?}: service {service:?} does not publish port {port}"
                    ))),
                }
            }
            Part::Inspect { service, path } => {
                let finished = self
                    .compose(context, project)
                    .args(["ps", "--all", "--quiet", service])
                    .capture_stdout()
                    .run_ok(&what, &NoLog)?;
                let Some(container) = finished.stdout.split_whitespace().next() else {
                    return Err(RunError::new(format!(
                        "output {output:?}: service {service:?} has no container"
                    )));
                };
                let finished = self
                    .docker(context, project)
                    .args(["inspect", container])
                    .capture_stdout()
                    .run_ok(&what, &NoLog)?;
                let inspected = decode_value(finished.stdout.trim().as_bytes())
                    .map_err(|e| RunError::new(format!("{what}: {e}")))?;
                let found = inspected
                    .as_list()
                    .and_then(|list| list.first())
                    .and_then(|container| walk(container, path));
                match found {
                    Some(Value::Null) | None => Err(RunError::new(format!(
                        "output {output:?}: the container of service {service:?} has nothing at {path}"
                    ))),
                    Some(value) => Ok(value.clone()),
                }
            }
        }
    }

    fn read_outputs(
        &self,
        context: &RunContext,
        project: &Project,
    ) -> Result<Vec<Output>, RunError> {
        let mut outputs = Vec::new();
        for (name, parts) in &project.outputs {
            let mut values = Vec::new();
            for part in parts {
                values.push(self.read_part(context, project, name, part)?);
            }
            // One reference alone keeps its type; among text it is text.
            let value = if values.len() == 1 {
                values.remove(0)
            } else {
                Value::String(
                    values
                        .iter()
                        .map(|v| match v {
                            Value::String(s) => s.clone(),
                            other => other.canonical(),
                        })
                        .collect(),
                )
            };
            outputs.push(Output {
                name: name.clone(),
                value,
                sensitive: false,
            });
        }
        Ok(outputs)
    }
}

/// Follows `.field` and `[index]` steps into a value.
fn walk<'v>(mut value: &'v Value, path: &str) -> Option<&'v Value> {
    for step in path.split(['.', '[']).filter(|s| !s.is_empty()) {
        value = match step.strip_suffix(']') {
            Some(index) => value.as_list()?.get(index.parse::<usize>().ok()?)?,
            None => value.as_map()?.get(step)?,
        };
    }
    Some(value)
}

/// Reads what `docker compose up` printed: one line a step, such as
/// `Container web-app-1  Recreated`, with `DRY-RUN MODE -` in front of each
/// in a dry run.
fn read_report(text: &str, project: &Project) -> Vec<Change> {
    let mut changes: Vec<Change> = Vec::new();
    for line in text.lines() {
        let mut words = line
            .split_whitespace()
            .skip_while(|w| matches!(*w, "DRY-RUN" | "MODE" | "-"));
        let (Some(kind), Some(name), Some(status)) = (words.next(), words.next(), words.next())
        else {
            continue;
        };
        if !matches!(kind, "Container" | "Network" | "Volume") {
            continue;
        }
        let action = match status {
            "Created" => ChangeAction::Create,
            "Recreated" | "Started" => ChangeAction::Update,
            "Removed" => ChangeAction::Delete,
            // Steps in progress, and "Running" for what is already as declared.
            _ => continue,
        };
        let summary = format!("{} {name}", kind.to_ascii_lowercase());
        // A container that is created or recreated is then started; that is
        // one change, not two.
        if changes.iter().any(|c| c.summary == summary) {
            continue;
        }
        changes.push(Change {
            resource: project.resource(kind, name),
            action,
            summary,
        });
    }
    changes
}

impl Runner for DockerRunner {
    fn operations(&self) -> Operations {
        Operations::ALL
    }

    fn plan(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Plan, RunError> {
        let project = self.prepare(context, artifact)?;
        log.out(&format!(
            "project {}: asking Compose what bringing it up would change",
            project.name
        ));
        let finished = self
            .compose(context, &project)
            .args(["--dry-run", "up", "--detach", "--remove-orphans"])
            .args(self.extra_args.iter().cloned())
            .capture_stdout()
            .run_ok("docker compose could not preview the project", &NoLog)?;
        let report = format!("{}\n{}", finished.stdout, finished.stderr);
        let plan = Plan {
            changes: read_report(&report, &project),
            unknown: false,
        };
        for change in &plan.changes {
            log.out(&change.to_string());
        }
        Ok(plan)
    }

    fn apply(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Applied, RunError> {
        let project = self.prepare(context, artifact)?;
        let finished = self
            .compose(context, &project)
            .args(["up", "--detach", "--remove-orphans"])
            .args(self.extra_args.iter().cloned())
            .run_ok("docker compose could not bring the project up", log)?;
        let report = format!("{}\n{}", finished.stdout, finished.stderr);
        Ok(Applied {
            changes: read_report(&report, &project),
            outputs: self.read_outputs(context, &project)?,
        })
    }

    fn outputs(&self, context: &RunContext) -> Result<Vec<Output>, RunError> {
        let settings =
            std::fs::read_to_string(context.work_dir.join(PROJECT_PATH)).map_err(|_| {
                RunError::new(
                    "nothing has been applied for this document yet, so it has no outputs",
                )
            })?;
        let project = Project::read(&settings)?;
        self.read_outputs(context, &project)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_walks_fields_and_indexes() {
        let value =
            decode_value(br#"{"State":{"Status":"running"},"Mounts":[{"Source":"/data"}]}"#)
                .unwrap();
        assert_eq!(
            walk(&value, ".State.Status"),
            Some(&Value::String("running".into()))
        );
        assert_eq!(
            walk(&value, ".Mounts[0].Source"),
            Some(&Value::String("/data".into()))
        );
        assert_eq!(walk(&value, ".Mounts[1].Source"), None);
        assert_eq!(walk(&value, ".State.Missing"), None);
    }
}
