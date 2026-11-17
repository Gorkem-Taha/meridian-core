//! The half that brings a project up with `docker compose`. It sits outside
//! the compile chain: nothing here is reached by a compile.
//!
//! Compose has no preview that can be read back reliably, so a plan is worked
//! out from what Compose and Docker report as data. Compose stamps every
//! container it creates with a hash of its service's configuration, and
//! recreates the container when the hash of the file's service differs. A
//! plan asks Compose for the hashes of the compiled file and Docker for the
//! hashes of the running containers, and compares them the same way.

use std::collections::{BTreeMap, BTreeSet};

use meridian_sdk::expr::{self, Part};
use meridian_sdk::tool::{write_artifact, Tool};
use meridian_sdk::{
    decode_value, Applied, Artifact, Change, ChangeAction, Log, NoLog, Operations, Output, Plan,
    RunContext, RunError, Runner, Value,
};

use crate::transform::Attribute;
use crate::{COMPOSE_PATH, PROJECT_PATH};

/// Names the Docker program, when it is not `docker` on `PATH`.
pub const DOCKER_VARIABLE: &str = "MERIDIAN_DOCKER";
/// The Compose command, split on whitespace, when it is not the Docker
/// program's `compose` subcommand: `docker-compose`, or `podman compose`.
pub const COMPOSE_VARIABLE: &str = "MERIDIAN_COMPOSE";
/// Extra arguments for every Compose call, split on whitespace:
/// `--env-file /etc/shop.env`.
pub const ARGS_VARIABLE: &str = "MERIDIAN_COMPOSE_ARGS";

/// The label Compose puts on everything that belongs to a project.
const PROJECT_LABEL: &str = "com.docker.compose.project";

/// One line per container: its service, the hash of the configuration it was
/// created from, its state, its name, and whether `compose run` made it.
const CONTAINER_FORMAT: &str = "{{.Label \"com.docker.compose.service\"}}|{{.Label \"com.docker.compose.config-hash\"}}|{{.State}}|{{.Names}}|{{.Label \"com.docker.compose.oneoff\"}}";

/// Brings a compiled project up.
pub struct ComposeRunner {
    /// The Docker program, asked what is running.
    pub docker: String,
    /// The Compose command: a program and the arguments that select Compose.
    pub compose: Vec<String>,
    /// Arguments added to every Compose call, after Meridian's own.
    pub extra_args: Vec<String>,
}

/// What `project.json` says about the deployment.
#[derive(Debug, Default, PartialEq)]
struct Settings {
    name: String,
    directory: String,
    profiles: Vec<String>,
    wait: bool,
    wait_timeout: u64,
    /// Output name to its value, with `${service.port.80}` left to resolve.
    outputs: BTreeMap<String, String>,
}

impl Settings {
    fn read(text: &str) -> Result<Settings, RunError> {
        let bad = || RunError::new(format!("{PROJECT_PATH} is not one this plugin compiled"));
        let value = decode_value(text.as_bytes()).map_err(|_| bad())?;
        let map = value.as_map().ok_or_else(bad)?;
        let string = |key: &str| {
            map.get(key)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
        };
        let settings = Settings {
            name: string("name"),
            directory: string("directory"),
            profiles: map
                .get("profiles")
                .and_then(Value::as_list)
                .unwrap_or(&[])
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect(),
            wait: map.get("wait") != Some(&Value::Bool(false)),
            wait_timeout: map.get("wait_timeout").and_then(whole).unwrap_or(0),
            outputs: map
                .get("outputs")
                .and_then(Value::as_map)
                .map(|m| {
                    m.iter()
                        .filter_map(|(k, v)| Some((k.clone(), v.as_str()?.to_owned())))
                        .collect()
                })
                .unwrap_or_default(),
        };
        if settings.name.is_empty() {
            return Err(bad());
        }
        Ok(settings)
    }
}

/// A whole number, however the JSON decoder chose to hold it.
fn whole(value: &Value) -> Option<u64> {
    match value {
        Value::Int(i) => u64::try_from(*i).ok(),
        Value::Uint(u) => Some(*u),
        Value::Float(f) if *f >= 0.0 && f.fract() == 0.0 => Some(*f as u64),
        _ => None,
    }
}

/// One service of the compiled file, as Compose reads it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Service {
    pub name: String,
    /// The hash Compose would stamp on a container created from it.
    pub hash: String,
    pub image: String,
    /// The service is built from source, not only pulled.
    pub builds: bool,
    /// How many containers the service should have.
    pub scale: u64,
}

/// What the compiled file asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Desired {
    pub services: Vec<Service>,
    /// The key in the file and the name Docker knows it by, for every
    /// network Compose would create.
    pub networks: Vec<(String, String)>,
    pub volumes: Vec<(String, String)>,
}

/// One container of the project that exists now.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Container {
    pub service: String,
    pub hash: String,
    pub state: String,
    pub name: String,
}

/// What Docker has for the project now.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Actual {
    pub containers: Vec<Container>,
    pub networks: BTreeSet<String>,
    pub volumes: BTreeSet<String>,
}

/// Reads `docker compose config --format json` and `config --hash '*'`.
pub(crate) fn read_desired(config: &str, hashes: &str) -> Result<Desired, String> {
    let config = decode_value(config.trim().as_bytes())?;
    let section = |key: &str| {
        config
            .as_map()
            .and_then(|m| m.get(key))
            .and_then(Value::as_map)
    };
    let hashes: BTreeMap<&str, &str> = hashes
        .lines()
        .filter_map(|line| line.trim().split_once(' '))
        .collect();

    let mut desired = Desired::default();
    for (name, body) in section("services").into_iter().flatten() {
        let body = body.as_map();
        let field = |key: &str| body.and_then(|b| b.get(key));
        let replicas = field("deploy")
            .and_then(Value::as_map)
            .and_then(|d| d.get("replicas"));
        desired.services.push(Service {
            name: name.clone(),
            hash: hashes.get(name.as_str()).copied().unwrap_or("").to_owned(),
            image: field("image")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_owned(),
            builds: field("build").is_some_and(|b| *b != Value::Null),
            scale: field("scale").or(replicas).and_then(whole).unwrap_or(1),
        });
    }
    let named = |key: &str| -> Vec<(String, String)> {
        section(key)
            .into_iter()
            .flatten()
            .filter_map(|(key, body)| {
                let body = body.as_map()?;
                // An external one is somebody else's to create.
                if body
                    .get("external")
                    .is_some_and(|e| *e != Value::Bool(false))
                {
                    return None;
                }
                Some((key.clone(), body.get("name")?.as_str()?.to_owned()))
            })
            .collect()
    };
    desired.networks = named("networks");
    desired.volumes = named("volumes");
    Ok(desired)
}

/// Reads the listing [`CONTAINER_FORMAT`] asks Docker for. A container that
/// `compose run` started is not part of the project's desired state.
pub(crate) fn read_containers(listing: &str) -> Vec<Container> {
    listing
        .lines()
        .filter_map(|line| {
            let mut fields = line.trim().split('|');
            let container = Container {
                service: fields.next()?.to_owned(),
                hash: fields.next()?.to_owned(),
                state: fields.next()?.to_owned(),
                name: fields.next()?.to_owned(),
            };
            let one_off = fields.next().unwrap_or("").eq_ignore_ascii_case("true");
            (!one_off && !container.service.is_empty()).then_some(container)
        })
        .collect()
}

/// What bringing the project up would change.
pub(crate) fn diff(desired: &Desired, actual: &Actual) -> Plan {
    let mut changes = Vec::new();
    let change = |resource: &str, action, summary: String| Change {
        resource: resource.to_owned(),
        action,
        summary,
    };
    for (key, name) in &desired.networks {
        if !actual.networks.contains(name) {
            changes.push(change(key, ChangeAction::Create, format!("network {name}")));
        }
    }
    for (key, name) in &desired.volumes {
        if !actual.volumes.contains(name) {
            changes.push(change(key, ChangeAction::Create, format!("volume {name}")));
        }
    }

    for service in &desired.services {
        let containers: Vec<&Container> = actual
            .containers
            .iter()
            .filter(|c| c.service == service.name)
            .collect();
        let existing = containers.len() as u64;
        let from = if service.builds {
            "built from source".to_owned()
        } else {
            format!("image {}", service.image)
        };
        let stopped = containers.iter().find(|c| c.state != "running");
        if existing == 0 {
            if service.scale > 0 {
                let summary = match service.scale {
                    1 => format!("container, {from}"),
                    n => format!("{n} containers, {from}"),
                };
                changes.push(change(&service.name, ChangeAction::Create, summary));
            }
        } else if service.scale == 0 {
            changes.push(change(
                &service.name,
                ChangeAction::Delete,
                "scaled to no containers".to_owned(),
            ));
        } else if containers.iter().any(|c| c.hash != service.hash) {
            changes.push(change(
                &service.name,
                ChangeAction::Replace,
                "its configuration changed, so its container is recreated".to_owned(),
            ));
        } else if existing != service.scale {
            changes.push(change(
                &service.name,
                ChangeAction::Update,
                format!("scaled from {existing} to {}", service.scale),
            ));
        } else if let Some(container) = stopped {
            changes.push(change(
                &service.name,
                ChangeAction::Update,
                format!(
                    "started; container {} is {}",
                    container.name, container.state
                ),
            ));
        }
    }

    // `up --remove-orphans` removes the containers of a service the file no
    // longer declares.
    let orphans: BTreeSet<&str> = actual
        .containers
        .iter()
        .map(|c| c.service.as_str())
        .filter(|name| !desired.services.iter().any(|s| s.name == *name))
        .collect();
    for orphan in orphans {
        changes.push(change(
            orphan,
            ChangeAction::Delete,
            "no longer in the document, so its containers are removed".to_owned(),
        ));
    }

    Plan {
        changes,
        // Whether a build would produce a different image cannot be known
        // without building it.
        unknown: desired.services.iter().any(|s| s.builds),
    }
}

/// The value of `DOCKER_HOST` for a document's host, when it has one that is
/// not this machine. A bare host name is reached over SSH.
pub(crate) fn docker_host(host: &str) -> Option<String> {
    match host {
        "" | "localhost" | "127.0.0.1" | "::1" => None,
        url if url.contains("://") => Some(url.to_owned()),
        name => Some(format!("ssh://{name}")),
    }
}

/// What `docker compose port` printed, as a port or as an address a person
/// could connect to from the machine Docker runs on.
pub(crate) fn published(printed: &str, attribute: &Attribute) -> Option<String> {
    let line = printed.lines().next()?.trim();
    let (host, port) = line.rsplit_once(':')?;
    if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) || port == "0" {
        return None;
    }
    Some(match attribute {
        Attribute::Address { .. } => {
            // Published on every interface, which is not an address to dial.
            let host = match host {
                "0.0.0.0" | "[::]" | "::" | "" => "localhost",
                host => host,
            };
            format!("{host}:{port}")
        }
        _ => port.to_owned(),
    })
}

impl ComposeRunner {
    /// The runner as the environment configures it.
    pub fn from_env() -> Self {
        let set = |variable: &str| {
            std::env::var(variable)
                .ok()
                .filter(|v| !v.trim().is_empty())
        };
        let words =
            |text: String| -> Vec<String> { text.split_whitespace().map(str::to_owned).collect() };
        let docker = set(DOCKER_VARIABLE).unwrap_or_else(|| "docker".to_owned());
        ComposeRunner {
            compose: set(COMPOSE_VARIABLE)
                .map(words)
                .unwrap_or_else(|| vec![docker.clone(), "compose".to_owned()]),
            docker,
            extra_args: set(ARGS_VARIABLE).map(words).unwrap_or_default(),
        }
    }

    fn at_host(tool: Tool, context: &RunContext) -> Tool {
        match docker_host(&context.host) {
            Some(host) => tool.env("DOCKER_HOST", host),
            None => tool,
        }
    }

    /// One Compose call against the project.
    fn compose(&self, context: &RunContext, settings: &Settings) -> Tool {
        let (program, selects) = self
            .compose
            .split_first()
            .map_or(("docker", &[][..]), |(p, rest)| (p.as_str(), rest));
        let mut tool = Tool::new(program)
            .args(selects.iter().cloned())
            .args(["--ansi", "never"])
            .args(["--project-name", &settings.name])
            .args(["--file", COMPOSE_PATH])
            .dir(&context.work_dir);
        if !settings.directory.is_empty() {
            tool = tool.args(["--project-directory", &settings.directory]);
        }
        for profile in &settings.profiles {
            tool = tool.args(["--profile", profile]);
        }
        Self::at_host(tool.args(self.extra_args.iter().cloned()), context)
    }

    /// One listing from Docker of what belongs to the project.
    fn listing(
        &self,
        context: &RunContext,
        settings: &Settings,
        what: &[&str],
        format: &str,
    ) -> Result<String, RunError> {
        let tool = Tool::new(&self.docker)
            .args(what.iter().copied())
            .args([
                "--filter",
                &format!("label={PROJECT_LABEL}={}", settings.name),
            ])
            .args(["--format", format])
            .dir(&context.work_dir)
            .capture_stdout();
        Ok(Self::at_host(tool, context)
            .run_ok("docker could not list what the project has running", &NoLog)?
            .stdout)
    }

    /// Writes the artifact and compares it with what Docker has.
    fn survey(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<(Settings, Desired, Plan), RunError> {
        let (Some(_), Some(settings)) = (
            artifact.files.get(COMPOSE_PATH),
            artifact.files.get(PROJECT_PATH),
        ) else {
            return Err(RunError::new(format!(
                "the artifact has no {COMPOSE_PATH} and {PROJECT_PATH}; it was not compiled by this plugin"
            )));
        };
        let settings = Settings::read(settings)?;
        write_artifact(&context.work_dir, artifact)?;

        log.out(&format!(
            "project {}: comparing the file with what Docker is running",
            settings.name
        ));
        // Compose's own reading of the file, which is also its validation.
        let config = self
            .compose(context, &settings)
            .args(["config", "--format", "json"])
            .capture_stdout()
            .run_ok("docker compose rejected the compiled file", log)?;
        let hashes = self
            .compose(context, &settings)
            .args(["config", "--hash", "*"])
            .capture_stdout()
            .run_ok("docker compose could not hash the services", log)?;
        let desired = read_desired(&config.stdout, &hashes.stdout).map_err(|e| {
            RunError::new(format!(
                "what docker compose said about the file could not be read: {e}"
            ))
        })?;

        let lines = |text: String| text.lines().map(|l| l.trim().to_owned()).collect();
        let actual = Actual {
            containers: read_containers(&self.listing(
                context,
                &settings,
                &["ps", "--all"],
                CONTAINER_FORMAT,
            )?),
            networks: lines(self.listing(context, &settings, &["network", "ls"], "{{.Name}}")?),
            volumes: lines(self.listing(context, &settings, &["volume", "ls"], "{{.Name}}")?),
        };
        let plan = diff(&desired, &actual);
        Ok((settings, desired, plan))
    }

    /// Answers the outputs the document declared by asking Docker.
    fn read_outputs(
        &self,
        context: &RunContext,
        settings: &Settings,
    ) -> Result<Vec<Output>, RunError> {
        let mut outputs = Vec::with_capacity(settings.outputs.len());
        for (name, template) in &settings.outputs {
            let bad = || RunError::new(format!("{PROJECT_PATH} is not one this plugin compiled"));
            let mut value = String::new();
            for part in expr::parse(template).map_err(|_| bad())? {
                match part {
                    Part::Text(text) => value.push_str(&text),
                    Part::Expr(expression) => {
                        let (service, attribute) = expression.split_once('.').ok_or_else(bad)?;
                        let attribute = Attribute::parse(attribute).ok_or_else(bad)?;
                        value.push_str(
                            &self.read_attribute(context, settings, name, service, &attribute)?,
                        );
                    }
                }
            }
            outputs.push(Output {
                name: name.clone(),
                value: Value::String(value),
                sensitive: false,
            });
        }
        Ok(outputs)
    }

    fn read_attribute(
        &self,
        context: &RunContext,
        settings: &Settings,
        output: &str,
        service: &str,
        attribute: &Attribute,
    ) -> Result<String, RunError> {
        let (port, udp) = match attribute {
            Attribute::Id => {
                let finished = self
                    .compose(context, settings)
                    .args(["ps", "--quiet", service])
                    .capture_stdout()
                    .run(&NoLog)?;
                return match finished.stdout.lines().next().map(str::trim) {
                    Some(id) if finished.success() && !id.is_empty() => Ok(id.to_owned()),
                    _ => Err(finished.into_error(format!(
                        "output {output:?} reads the container of service {service:?}, and it has none running"
                    ))),
                };
            }
            Attribute::Port { port, udp } | Attribute::Address { port, udp } => (*port, *udp),
        };
        let mut tool = self.compose(context, settings).arg("port");
        if udp {
            tool = tool.args(["--protocol", "udp"]);
        }
        let finished = tool
            .args([service.to_owned(), port.to_string()])
            .capture_stdout()
            .run(&NoLog)?;
        match published(&finished.stdout, attribute) {
            Some(value) if finished.success() => Ok(value),
            _ => Err(finished.into_error(format!(
                "output {output:?} reads the host port for port {port} of service {service:?}, \
                 and the running service publishes none. List it under the service's \"ports\""
            ))),
        }
    }
}

impl Runner for ComposeRunner {
    fn operations(&self) -> Operations {
        Operations::ALL
    }

    fn plan(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Plan, RunError> {
        let (_, _, plan) = self.survey(context, artifact, log)?;
        Ok(plan)
    }

    fn apply(
        &self,
        context: &RunContext,
        artifact: &Artifact,
        log: &dyn Log,
    ) -> Result<Applied, RunError> {
        let (settings, desired, plan) = self.survey(context, artifact, log)?;
        let mut up = self
            .compose(context, &settings)
            .args(["up", "--detach", "--remove-orphans"]);
        if desired.services.iter().any(|s| s.builds) {
            up = up.arg("--build");
        }
        if settings.wait {
            up = up.arg("--wait");
            if settings.wait_timeout > 0 {
                up = up.args(["--wait-timeout", &settings.wait_timeout.to_string()]);
            }
        }
        up.run_ok(
            "docker compose could not bring the project up; what it printed says which service \
             failed",
            log,
        )?;
        Ok(Applied {
            changes: plan.changes,
            outputs: self.read_outputs(context, &settings)?,
        })
    }

    fn outputs(&self, context: &RunContext) -> Result<Vec<Output>, RunError> {
        let settings =
            std::fs::read_to_string(context.work_dir.join(PROJECT_PATH)).map_err(|_| {
                RunError::new(
                    "nothing has been applied for this document yet, so it has no outputs",
                )
            })?;
        self.read_outputs(context, &Settings::read(&settings)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn service(name: &str, hash: &str) -> Service {
        Service {
            name: name.to_owned(),
            hash: hash.to_owned(),
            image: format!("{name}:1"),
            builds: false,
            scale: 1,
        }
    }

    fn container(service: &str, hash: &str, state: &str) -> Container {
        Container {
            service: service.to_owned(),
            hash: hash.to_owned(),
            state: state.to_owned(),
            name: format!("shop-{service}-1"),
        }
    }

    fn shown(plan: &Plan) -> Vec<String> {
        plan.changes.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn composes_own_reading_of_the_file_becomes_the_desired_state() {
        let config = r#"{
          "name": "shop",
          "services": {
            "web": {"image": "nginx:1.27", "deploy": {"replicas": 3}},
            "api": {"build": {"context": "/srv/api"}, "scale": 2},
            "db": {"image": "postgres:17"}
          },
          "networks": {
            "default": {"name": "shop_default", "ipam": {}},
            "edge": {"name": "edge", "external": true}
          },
          "volumes": {"data": {"name": "shop_data"}}
        }"#;
        let desired = read_desired(config, "web aaa\napi bbb\ndb ccc\n").unwrap();
        assert_eq!(
            desired.services,
            [
                Service {
                    name: "api".into(),
                    hash: "bbb".into(),
                    image: String::new(),
                    builds: true,
                    scale: 2,
                },
                service("db", "ccc").with_image("postgres:17"),
                Service {
                    scale: 3,
                    ..service("web", "aaa").with_image("nginx:1.27")
                },
            ]
        );
        // The external network is not Compose's to create.
        assert_eq!(
            desired.networks,
            [("default".into(), "shop_default".into())]
        );
        assert_eq!(desired.volumes, [("data".into(), "shop_data".into())]);
        assert!(read_desired("not json", "").is_err());
    }

    impl Service {
        fn with_image(mut self, image: &str) -> Service {
            image.clone_into(&mut self.image);
            self
        }
    }

    #[test]
    fn a_container_listing_leaves_out_what_compose_run_started() {
        let listing = "web|aaa|running|shop-web-1|False\n\
                       db|ccc|exited|shop-db-1|False\n\
                       web|aaa|running|shop-web-run-1f2e|True\n\
                       \n";
        assert_eq!(
            read_containers(listing),
            [
                container("web", "aaa", "running"),
                container("db", "ccc", "exited")
            ]
        );
    }

    #[test]
    fn a_project_that_is_not_running_is_all_creates() {
        let desired = Desired {
            services: vec![service("db", "ccc"), service("web", "aaa")],
            networks: vec![("default".into(), "shop_default".into())],
            volumes: vec![("data".into(), "shop_data".into())],
        };
        let plan = diff(&desired, &Actual::default());
        assert_eq!(
            shown(&plan),
            [
                "create default: network shop_default",
                "create data: volume shop_data",
                "create db: container, image db:1",
                "create web: container, image web:1",
            ]
        );
        assert!(!plan.unknown);
    }

    #[test]
    fn a_project_that_matches_the_file_is_in_sync() {
        let desired = Desired {
            services: vec![service("db", "ccc"), service("web", "aaa")],
            networks: vec![("default".into(), "shop_default".into())],
            volumes: vec![("data".into(), "shop_data".into())],
        };
        let actual = Actual {
            containers: vec![
                container("db", "ccc", "running"),
                container("web", "aaa", "running"),
            ],
            networks: ["shop_default".to_owned()].into(),
            volumes: ["shop_data".to_owned()].into(),
        };
        assert!(diff(&desired, &actual).in_sync());
    }

    #[test]
    fn each_kind_of_difference_is_its_own_change() {
        let desired = Desired {
            services: vec![
                service("changed", "new"),
                Service {
                    scale: 3,
                    ..service("grown", "g")
                },
                Service {
                    scale: 0,
                    ..service("idle", "i")
                },
                Service {
                    scale: 0,
                    ..service("never", "n")
                },
                service("stopped", "s"),
                service("steady", "t"),
            ],
            ..Desired::default()
        };
        let actual = Actual {
            containers: vec![
                container("changed", "old", "running"),
                container("grown", "g", "running"),
                container("idle", "i", "running"),
                container("stopped", "s", "exited"),
                container("steady", "t", "running"),
                container("gone", "x", "running"),
                Container {
                    name: "shop-gone-2".into(),
                    ..container("gone", "x", "running")
                },
            ],
            ..Actual::default()
        };
        assert_eq!(
            shown(&diff(&desired, &actual)),
            [
                "replace changed: its configuration changed, so its container is recreated",
                "update grown: scaled from 1 to 3",
                "delete idle: scaled to no containers",
                "update stopped: started; container shop-stopped-1 is exited",
                // Two containers of one orphaned service are one change.
                "delete gone: no longer in the document, so its containers are removed",
            ]
        );
    }

    #[test]
    fn a_service_built_from_source_makes_the_plan_uncertain() {
        let desired = Desired {
            services: vec![Service {
                builds: true,
                ..service("api", "b")
            }],
            ..Desired::default()
        };
        let running = Actual {
            containers: vec![container("api", "b", "running")],
            ..Actual::default()
        };
        let plan = diff(&desired, &running);
        assert!(plan.changes.is_empty());
        assert!(plan.unknown && !plan.in_sync());
        assert_eq!(
            shown(&diff(&desired, &Actual::default())),
            ["create api: container, built from source"]
        );
    }

    #[test]
    fn a_host_other_than_this_machine_is_reached_over_ssh() {
        assert_eq!(docker_host(""), None);
        assert_eq!(docker_host("localhost"), None);
        assert_eq!(docker_host("127.0.0.1"), None);
        assert_eq!(
            docker_host("web01.example.com").as_deref(),
            Some("ssh://web01.example.com")
        );
        assert_eq!(
            docker_host("deploy@10.0.0.7").as_deref(),
            Some("ssh://deploy@10.0.0.7")
        );
        assert_eq!(
            docker_host("tcp://10.0.0.7:2376").as_deref(),
            Some("tcp://10.0.0.7:2376")
        );
    }

    #[test]
    fn a_published_port_is_read_as_a_port_or_an_address() {
        let port = Attribute::Port {
            port: 80,
            udp: false,
        };
        let address = Attribute::Address {
            port: 80,
            udp: false,
        };
        assert_eq!(
            published("0.0.0.0:32768\n", &port).as_deref(),
            Some("32768")
        );
        assert_eq!(
            published("0.0.0.0:32768\n", &address).as_deref(),
            Some("localhost:32768")
        );
        assert_eq!(
            published("[::]:32768\n", &address).as_deref(),
            Some("localhost:32768")
        );
        assert_eq!(
            published("127.0.0.1:8080\n", &address).as_deref(),
            Some("127.0.0.1:8080")
        );
        assert_eq!(published("", &port), None);
        assert_eq!(published(":0\n", &port), None);
        assert_eq!(published("no port 80/tcp for container\n", &port), None);
    }

    #[test]
    fn the_settings_file_is_read_back_and_a_strange_one_is_refused() {
        let settings = Settings::read(
            r#"{"name":"shop","directory":"/srv/shop","profiles":["debug"],"wait":false,"wait_timeout":90,"outputs":{"url":"http://localhost:${web.port.80}/"}}"#,
        )
        .unwrap();
        assert_eq!(
            settings,
            Settings {
                name: "shop".into(),
                directory: "/srv/shop".into(),
                profiles: vec!["debug".into()],
                wait: false,
                wait_timeout: 90,
                outputs: [(
                    "url".to_owned(),
                    "http://localhost:${web.port.80}/".to_owned()
                )]
                .into(),
            }
        );
        assert!(Settings::read(r#"{"name":"shop"}"#).unwrap().wait);
        for strange in ["", "[]", "{}", r#"{"name":""}"#] {
            assert_eq!(
                Settings::read(strange).unwrap_err().msg,
                "project.json is not one this plugin compiled"
            );
        }
    }
}
