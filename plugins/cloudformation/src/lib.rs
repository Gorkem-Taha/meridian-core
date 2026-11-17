//! Compiles a Meridian infrastructure document into an AWS CloudFormation
//! template and deploys it as a stack with the `aws` command line.
//!
//! # What a document looks like to this target
//!
//! A resource's `type` is the CloudFormation resource type
//! (`AWS::EC2::Instance`) and its `params` are that type's properties,
//! exactly as AWS documents them. A few keys are resource attributes and not
//! properties, and are written beside `Properties` instead of inside it:
//! `DeletionPolicy`, `UpdateReplacePolicy`, `CreationPolicy`, `UpdatePolicy`,
//! `Condition` and `Metadata`.
//!
//! One resource may have the type `stack`. It is not part of the template: it
//! holds the settings of the deployment itself (`name`, `region`,
//! `capabilities`).
//!
//! What Meridian adds is the wiring. `${web_vm}` refers to another resource
//! of the document and becomes a `Ref`; `${web_vm.PublicIp}` becomes
//! `Fn::GetAtt`; either inside longer text becomes `Fn::Sub`. `dependsOn`
//! becomes `DependsOn` unless a reference already implies the order. Outputs
//! become the template's `Outputs`.
//!
//! CloudFormation's logical ids hold only letters and digits, so a resource
//! id such as `web_vm` is written `WebVm`. The mapping travels with the
//! artifact, and everything Meridian reports uses the id the author wrote.

// A refusal carries its position and resource by value, as the SDK's does.
#![allow(clippy::result_large_err)]

mod emit;
mod runner;
mod sort;
mod transform;
mod validate;

use std::collections::{BTreeMap, BTreeSet};

use meridian_sdk::{
    Artifact, Capabilities, Category, CompileError, Data, Emitted, Emitter, Position, Resource,
    ResourceGraph, Value,
};
use meridian_yaml::Yaml;

pub use emit::{STACK_PATH, TEMPLATE_PATH};
pub use runner::CloudFormationRunner;

/// The target name a document selects with `spec.target`.
pub const NAME: &str = "cloudformation";

/// One resource of the template.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Entry {
    /// The resource's id in the document.
    pub id: String,
    /// Its logical id in the template.
    pub logical: String,
    pub r#type: String,
    pub properties: Vec<(String, Yaml)>,
    /// Resource attributes, written beside `Properties`.
    pub attributes: Vec<(String, Yaml)>,
    pub depends_on: Vec<String>,
    /// The ids of the other resources the properties refer to.
    pub refs: BTreeSet<String>,
    pub pos: Position,
}

/// The settings of the deployment, from the document's `stack` resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Settings {
    pub name: String,
    pub region: String,
    pub capabilities: Vec<String>,
}

/// A whole template and how to deploy it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Template {
    pub document: String,
    pub settings: Settings,
    pub entries: Vec<Entry>,
    /// Output name as written, its logical id, and its value.
    pub outputs: Vec<(String, String, Yaml, Position)>,
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

/// Turns a document id into a CloudFormation logical id: `web_vm` is `WebVm`.
/// Returns `None` when nothing alphanumeric is left to name it by.
pub(crate) fn logical_id(id: &str) -> Option<String> {
    let mut out = String::with_capacity(id.len());
    for word in id.split(|c: char| !c.is_ascii_alphanumeric()) {
        let mut chars = word.chars();
        if let Some(first) = chars.next() {
            out.push(first.to_ascii_uppercase());
            out.push_str(chars.as_str());
        }
    }
    (!out.is_empty()).then_some(out)
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
        // A plugin is never sent a secret as a value; one would be written as its mask.
        Value::Secret(_) => Yaml::str(meridian_sdk::SENSITIVE),
        Value::List(items) => Yaml::Seq(items.iter().map(plain).collect()),
        Value::Map(map) => Yaml::Map(map.iter().map(|(k, v)| (k.clone(), plain(v))).collect()),
    }
}

/// The names of the two files, kept in one place for the runner.
pub(crate) type Files = BTreeMap<String, String>;

/// The CloudFormation emitter.
pub struct CloudFormation;

impl Emitter for CloudFormation {
    fn name(&self) -> &str {
        NAME
    }

    fn supported_resource_types(&self) -> Vec<String> {
        ["<Vendor>::<Service>::<Type>", "stack"]
            .map(str::to_owned)
            .to_vec()
    }

    /// CloudFormation cannot act on one resource because another changed,
    /// natively or otherwise, and it has no apply-time conditional that reads
    /// the state of the world.
    fn capabilities(&self) -> Capabilities {
        Capabilities::default()
    }

    fn category(&self) -> Category {
        Category::InfrastructureAsCode
    }

    fn emit(&self, graph: &ResourceGraph, _data: &Data) -> Result<Emitted, CompileError> {
        let mut template = transform::transform(graph)?;
        sort::apply_ordering(graph, &mut template)?;
        validate::validate(&template)?;
        Ok(Emitted {
            artifact: Artifact {
                files: emit::serialise(&template),
            },
            warnings: Vec::new(),
        })
    }
}
