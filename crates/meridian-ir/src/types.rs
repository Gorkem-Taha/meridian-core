use std::collections::BTreeMap;
use std::fmt;

use crate::error::Position;
use meridian_value::Value;

/// The only `apiVersion` this build understands. Any other value is an
/// explicit error, never a silent partial parse.
pub const API_VERSION_V1: &str = "meridian/v1";

/// `targetHost.from` value that resolves against another document's outputs.
pub const TARGET_HOST_FROM_INFRASTRUCTURE: &str = "infrastructure";
/// `targetHost.from` value that names an already-existing node directly.
pub const TARGET_HOST_FROM_INLINE: &str = "inline";

/// The two tiers of the IR: provisioning and host configuration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Infrastructure,
    ResourceSet,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Infrastructure => "Infrastructure",
            Kind::ResourceSet => "ResourceSet",
        }
    }
}

impl fmt::Display for Kind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The policy applied when an edge points at a resource that is not in the
/// compiled graph, typically because `when` pruned it.
///
/// Parsing keeps whatever the document said; an unrecognised policy is a
/// validation error, so it is reported alongside a document's other problems
/// instead of stopping the parse.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum IfMissing {
    /// Not stated. Treated as [`IfMissing::Error`].
    #[default]
    Unset,
    /// A dangling edge is an authoring mistake.
    Error,
    Skip,
    Unknown(String),
}

impl IfMissing {
    pub(crate) fn from_text(s: &str) -> Self {
        match s {
            "" => IfMissing::Unset,
            "error" => IfMissing::Error,
            "skip" => IfMissing::Skip,
            other => IfMissing::Unknown(other.to_owned()),
        }
    }

    /// Whether a dangling edge with this policy is dropped instead of refused.
    pub fn skips(&self) -> bool {
        matches!(self, IfMissing::Skip)
    }
}

/// Document identity.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Metadata {
    pub name: String,
    pub labels: BTreeMap<String, String>,
}

/// A hard ordering constraint: the referenced resource must be applied before
/// the resource declaring it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Dependency {
    pub resource: String,
    pub if_missing: IfMissing,
    pub pos: Position,
}

/// A conditional, deferred trigger. Deliberately a separate type from
/// [`Dependency`], because it is not an ordering constraint.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Notification {
    pub resource: String,
    pub on: String,
    pub action: String,
    pub if_missing: IfMissing,
    pub pos: Position,
}

/// One declared unit of desired state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resource {
    pub id: String,
    pub r#type: String,
    pub state: String,
    pub params: BTreeMap<String, Value>,
    pub depends_on: Vec<Dependency>,
    pub notifies: Vec<Notification>,
    pub when: String,
    pub runtime_when: String,
    /// Position in the document's declaration order. It is the sole tiebreak
    /// for topological sort, so downstream ordering never depends on anything
    /// but what the author wrote.
    pub index: usize,
    pub pos: Position,
}

/// A value an Infrastructure document exposes to ResourceSet documents.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Output {
    pub name: String,
    pub value: String,
    pub pos: Position,
}

/// Binds a ResourceSet to the node it configures. `from: infrastructure`
/// resolves `ref`/`output` against a sibling Infrastructure document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TargetHost {
    pub from: String,
    pub r#ref: String,
    pub output: String,
    pub host: String,
    pub pos: Position,
}

/// The body of a `kind: Infrastructure` document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InfrastructureSpec {
    pub target: String,
    pub target_version: String,
    pub resources: Vec<Resource>,
    pub outputs: Vec<Output>,
}

/// The provisioning tier: VMs, networks, and anything else that compiles to an
/// infrastructure-as-code tool.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Infrastructure {
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: InfrastructureSpec,
    pub pos: Position,
}

impl Infrastructure {
    /// The named output, if it was declared.
    pub fn output(&self, name: &str) -> Option<&Output> {
        self.spec.outputs.iter().find(|o| o.name == name)
    }
}

/// The body of a `kind: ResourceSet` document.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceSetSpec {
    pub target: String,
    pub target_version: String,
    pub target_host: Option<TargetHost>,
    pub resources: Vec<Resource>,
}

/// The host-configuration tier: packages, files, services.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ResourceSet {
    pub api_version: String,
    pub metadata: Metadata,
    pub spec: ResourceSetSpec,
    pub pos: Position,
}

/// A parsed document of either kind.
#[derive(Clone, Debug, PartialEq)]
pub enum Document {
    Infrastructure(Infrastructure),
    ResourceSet(ResourceSet),
}

impl Document {
    pub fn kind(&self) -> Kind {
        match self {
            Document::Infrastructure(_) => Kind::Infrastructure,
            Document::ResourceSet(_) => Kind::ResourceSet,
        }
    }

    pub fn api_version(&self) -> &str {
        match self {
            Document::Infrastructure(d) => &d.api_version,
            Document::ResourceSet(d) => &d.api_version,
        }
    }

    /// `metadata.name`.
    pub fn name(&self) -> &str {
        match self {
            Document::Infrastructure(d) => &d.metadata.name,
            Document::ResourceSet(d) => &d.metadata.name,
        }
    }

    /// `spec.target`.
    pub fn target(&self) -> &str {
        match self {
            Document::Infrastructure(d) => &d.spec.target,
            Document::ResourceSet(d) => &d.spec.target,
        }
    }

    pub fn resources(&self) -> &[Resource] {
        match self {
            Document::Infrastructure(d) => &d.spec.resources,
            Document::ResourceSet(d) => &d.spec.resources,
        }
    }

    pub fn position(&self) -> &Position {
        match self {
            Document::Infrastructure(d) => &d.pos,
            Document::ResourceSet(d) => &d.pos,
        }
    }

    pub fn as_infrastructure(&self) -> Option<&Infrastructure> {
        match self {
            Document::Infrastructure(d) => Some(d),
            Document::ResourceSet(_) => None,
        }
    }

    pub fn as_resource_set(&self) -> Option<&ResourceSet> {
        match self {
            Document::ResourceSet(d) => Some(d),
            Document::Infrastructure(_) => None,
        }
    }
}

impl From<Infrastructure> for Document {
    fn from(d: Infrastructure) -> Self {
        Document::Infrastructure(d)
    }
}

impl From<ResourceSet> for Document {
    fn from(d: ResourceSet) -> Self {
        Document::ResourceSet(d)
    }
}
