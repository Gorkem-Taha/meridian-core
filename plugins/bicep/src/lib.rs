//! Compiles a Meridian infrastructure document into an Azure Bicep file and
//! deploys it to a resource group with the `az` command line.
//!
//! # What a document looks like to this target
//!
//! A resource's `type` is the Azure resource type with its API version
//! (`Microsoft.Network/publicIPAddresses@2023-09-01`) and its `params` are
//! the resource's body, exactly as Azure documents it: `name`, `location`,
//! `sku`, `properties` and the rest.
//!
//! One resource must have the type `deployment`. It is not part of the Bicep
//! file: it says where the file is deployed (`resourceGroup`, and optionally
//! `name` and `subscription`).
//!
//! What Meridian adds is the wiring. `${web_ip}` refers to another resource
//! of the document by its symbolic name and `${web_ip.properties.ipAddress}`
//! reads from it; either inside longer text becomes Bicep string
//! interpolation. Anything else between `${` and `}` is Bicep's own
//! expression language, such as `${resourceGroup().location}`, and is handed
//! over as written. `dependsOn` becomes Bicep's `dependsOn` unless a
//! reference already implies the order. Outputs become `output` statements,
//! always of type string.

// A refusal carries its position and resource by value, as the SDK's does.
#![allow(clippy::result_large_err)]

mod emit;
mod runner;
mod sort;
mod transform;
mod validate;

use std::collections::BTreeSet;

use meridian_sdk::{
    Artifact, Capabilities, Category, CompileError, Data, Emitted, Emitter, Position, Resource,
    ResourceGraph,
};

pub use emit::{DEPLOYMENT_PATH, TEMPLATE_PATH};
pub use runner::BicepRunner;

/// The target name a document selects with `spec.target`.
pub const NAME: &str = "bicep";

/// A Bicep value.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Expr {
    /// An expression written as it stands: a symbol, a property access, a
    /// function call, a number.
    Raw(String),
    /// A string literal, which may interpolate expressions.
    Text(Vec<Piece>),
    Array(Vec<Expr>),
    Object(Vec<(String, Expr)>),
}

/// One piece of a string literal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Piece {
    Literal(String),
    Interpolated(String),
}

/// One `resource` statement.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Statement {
    /// The symbolic name, which is the resource's id.
    pub symbol: String,
    /// `Microsoft.Network/publicIPAddresses@2023-09-01`.
    pub r#type: String,
    pub body: Vec<(String, Expr)>,
    /// The Azure name, when the document wrote it as a plain string.
    pub azure_name: Option<String>,
    pub depends_on: Vec<String>,
    /// The ids of the other resources the body refers to.
    pub refs: BTreeSet<String>,
    pub pos: Position,
}

/// Where the file is deployed, from the document's `deployment` resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Deployment {
    pub name: String,
    pub resource_group: String,
    pub subscription: String,
}

/// A whole Bicep file and where it goes.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Template {
    pub document: String,
    pub deployment: Deployment,
    pub statements: Vec<Statement>,
    pub outputs: Vec<(String, Expr, Position)>,
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

/// A name Bicep accepts for a symbol or an output.
pub(crate) fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The Bicep emitter.
pub struct Bicep;

impl Emitter for Bicep {
    fn name(&self) -> &str {
        NAME
    }

    fn supported_resource_types(&self) -> Vec<String> {
        ["<Namespace>/<type>@<api-version>", "deployment"]
            .map(str::to_owned)
            .to_vec()
    }

    /// A deployment cannot act on one resource because another changed, and
    /// it has no conditional that reads the state of what it deploys to.
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
