//! Compiles a Meridian document into a Docker Compose file and brings the
//! project up with `docker compose`.
//!
//! # What a document looks like to this target
//!
//! A resource's `type` is one of the things a Compose file declares:
//! `service`, `network`, `volume`, `config` or `secret`. Its `params` are
//! that element's attributes, exactly as the Compose specification documents
//! them, and its id is the element's name in the file.
//!
//! One resource may have the type `project`. It is not part of the file: it
//! holds the settings of the deployment itself (`name`, `directory`,
//! `profiles`, `wait`, `wait_timeout`).
//!
//! What Meridian adds is the wiring. `${db}` refers to another resource of
//! the document and is checked when the document compiles, so a misspelt
//! service, network or volume never reaches Docker. `dependsOn` between two
//! services becomes `depends_on`, waiting for the other service to be healthy
//! when it declares a healthcheck. `notifies` with the action `restart`
//! becomes `depends_on` with `restart: true`, which is Compose's own way of
//! restarting one service because another changed.
//!
//! An Infrastructure document may declare outputs. Compose has no outputs of
//! its own, so the runner answers them by asking Docker: `${web.port.80}` is
//! the host port published for the service's port 80.
//!
//! Compose reads `${NAME}` as one of its own variables. Meridian's escape for
//! a literal `${` is `$${`, so `$${NAME}` in a document reaches Compose as
//! `${NAME}`.

// A refusal carries its position and resource by value, as the SDK's does.
#![allow(clippy::result_large_err)]

mod emit;
mod runner;
mod sort;
mod transform;
mod validate;

use std::collections::BTreeMap;

use meridian_sdk::{
    Artifact, Capabilities, Category, CompileError, Data, Emitted, Emitter, Position, Resource,
    ResourceGraph, Value, Warning,
};
use meridian_yaml::Yaml;

pub use emit::{COMPOSE_PATH, PROJECT_PATH};
pub use runner::{ComposeRunner, ARGS_VARIABLE, COMPOSE_VARIABLE, DOCKER_VARIABLE};

/// The target name a document selects with `spec.target`.
pub const NAME: &str = "compose";

/// The resource type that holds the deployment's settings.
pub(crate) const PROJECT_TYPE: &str = "project";

/// What a resource is in a Compose file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Service,
    Network,
    Volume,
    Config,
    Secret,
}

impl Kind {
    pub(crate) const ALL: [Kind; 5] = [
        Kind::Service,
        Kind::Network,
        Kind::Volume,
        Kind::Config,
        Kind::Secret,
    ];

    pub(crate) fn from_type(t: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.as_str() == t)
    }

    /// The resource type, which is also how a message names the kind.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Kind::Service => "service",
            Kind::Network => "network",
            Kind::Volume => "volume",
            Kind::Config => "config",
            Kind::Secret => "secret",
        }
    }

    /// The top-level key of the Compose file that holds this kind.
    pub(crate) fn section(self) -> &'static str {
        match self {
            Kind::Service => "services",
            Kind::Network => "networks",
            Kind::Volume => "volumes",
            Kind::Config => "configs",
            Kind::Secret => "secrets",
        }
    }
}

/// One entry of a service's `depends_on`, in the long form.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Dependency {
    pub service: String,
    /// `service_started`, `service_healthy` or
    /// `service_completed_successfully`.
    pub condition: String,
    /// Restart this service when the other one is updated.
    pub restart: bool,
    /// Compose's `required`, when the author wrote one.
    pub required: Option<bool>,
}

/// One element of the Compose file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The resource's id, which is also its name in the file.
    pub id: String,
    pub kind: Kind,
    /// The element's attributes, sorted by name. A service's `depends_on` is
    /// kept apart in `depends_on` until the file is written.
    pub body: Vec<(String, Yaml)>,
    pub depends_on: Vec<Dependency>,
    pub pos: Position,
}

impl Entry {
    pub(crate) fn attribute(&self, name: &str) -> Option<&Yaml> {
        self.body.iter().find(|(k, _)| k == name).map(|(_, v)| v)
    }

    /// Whether a service declares a healthcheck Compose can wait for.
    pub(crate) fn has_healthcheck(&self) -> bool {
        match self.attribute("healthcheck") {
            Some(Yaml::Map(fields)) => !fields
                .iter()
                .any(|(k, v)| k == "disable" && *v == Yaml::Bool(true)),
            _ => false,
        }
    }
}

/// The settings of the deployment, from the document's `project` resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings {
    pub name: String,
    /// The directory relative paths in the file are resolved against. Empty
    /// means the directory Meridian keeps the compiled file in.
    pub directory: String,
    pub profiles: Vec<String>,
    /// Wait for every service to be running, or healthy, before an apply
    /// reports success.
    pub wait: bool,
    /// How long to wait, in seconds. Zero leaves it to Compose.
    pub wait_timeout: u64,
}

/// A whole project and how to bring it up.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Project {
    pub document: String,
    /// The id of the `project` resource, when the document has one.
    pub settings_id: Option<String>,
    pub settings: Settings,
    pub entries: Vec<Entry>,
    /// Output name, and its value with everything known at compile time
    /// already filled in. What is left in `${...}` the runner asks Docker.
    pub outputs: Vec<(String, String)>,
    pub warnings: Vec<Warning>,
}

impl Project {
    pub(crate) fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.id == id)
    }

    pub(crate) fn of_kind(&self, kind: Kind) -> impl Iterator<Item = &Entry> {
        self.entries.iter().filter(move |e| e.kind == kind)
    }
}

/// A refusal of one resource, stamped with this target.
pub(crate) fn refuse(r: &Resource, rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        ..CompileError::for_resource(r, rule, msg)
    }
}

pub(crate) fn refuse_at(resource: &str, pos: &Position, rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        resource: resource.to_owned(),
        rule: rule.to_owned(),
        msg,
        pos: pos.clone(),
    }
}

/// A param value with no references in it, as YAML.
pub(crate) fn plain(value: &Value) -> Yaml {
    match value {
        Value::Null => Yaml::Null,
        Value::Bool(b) => Yaml::Bool(*b),
        Value::Int(i) => Yaml::Int(*i),
        Value::Uint(u) => Yaml::Uint(*u),
        Value::Float(f) => Yaml::Float(*f),
        Value::String(s) => Yaml::str(s),
        Value::List(items) => Yaml::Seq(items.iter().map(plain).collect()),
        Value::Map(map) => Yaml::Map(map.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
    }
}

/// The compiled files, by path.
pub(crate) type Files = BTreeMap<String, String>;

/// The Docker Compose emitter.
pub struct Compose;

impl Emitter for Compose {
    fn name(&self) -> &str {
        NAME
    }

    fn supported_resource_types(&self) -> Vec<String> {
        [
            "config", "network", "project", "secret", "service", "volume",
        ]
        .map(str::to_owned)
        .to_vec()
    }

    /// Compose restarts a service when one it depends on is updated
    /// (`depends_on` with `restart: true`), which is a notify the tool
    /// performs itself. It has no conditional that reads the state of the
    /// host at the instant of apply.
    fn capabilities(&self) -> Capabilities {
        Capabilities {
            native_notify: true,
            synthetic_notify: false,
            runtime_condition: false,
        }
    }

    fn category(&self) -> Category {
        Category::Containerization
    }

    fn emit(&self, graph: &ResourceGraph, _data: &Data) -> Result<Emitted, CompileError> {
        let mut project = transform::transform(graph)?;
        sort::apply_ordering(graph, &mut project)?;
        validate::validate(&project)?;
        Ok(Emitted {
            artifact: Artifact {
                files: emit::serialise(&project),
            },
            warnings: project.warnings,
        })
    }
}
