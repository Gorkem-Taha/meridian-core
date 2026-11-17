//! Meridian's compile rules: policies over the layers of a hierarchy, checked
//! before anything is compiled.
//!
//! A hierarchy says how layers merge. Compile rules say what a layer is
//! *allowed* to do in that merge: that a more specific layer may not change a
//! value a less specific one set, that only some layers may set a key, that a
//! key must be set, that a value is forbidden. A rules file is a deny list
//! out of the box: everything is allowed except what it forbids, and the
//! sets in [`DEFAULT_USE`] are in force unless it says otherwise. It is a file
//! of its own beside the hierarchy. It is never a layer and never merged, so nothing
//! the rules constrain can rewrite them.
//!
//! The rules are evaluated over a [`Stack`]: an ordered list of layers, a
//! [`MergePlan`] that says how keys combine, and the merged result. Nothing
//! in the evaluation knows about targets or segments, which is what makes a
//! rule mean the same for every tool.
//!
//! ```
//! use meridian_rules::{evaluate, Layer, Rules, Stack};
//! use meridian_resolve::{Data, Hierarchy};
//! # fn data(yaml: &str) -> Data {
//! #     match meridian_ir::parse_value("x", yaml.as_bytes()).unwrap() {
//! #         meridian_ir::Value::Map(m) => m,
//! #         _ => unreachable!(),
//! #     }
//! # }
//! let rules = Rules::parse("rules.yaml", b"rulesVersion: 1\nuse: [protect-variables]\n").unwrap();
//! let layers = [
//!     Layer::new("common.yaml", data("variables: {region: eu-west-1}")),
//!     Layer::new("node/web01.yaml", data("variables: {region: us-east-1}")),
//! ];
//! let hierarchy = Hierarchy::new(["common.yaml", "node/web01.yaml"]);
//! let resolved = data("variables: {region: us-east-1}");
//! let violations = evaluate(&rules, &Stack { layers: &layers, plan: &hierarchy, resolved: &resolved }).unwrap();
//! assert_eq!(
//!     violations[0].to_string(),
//!     "[meridian/protect-variables] layer \"node/web01.yaml\" changes \"variables.region\", \
//!      which layer \"common.yaml\" set"
//! );
//! ```

mod eval;
mod files;
mod model;
mod parse;
mod pattern;

use std::fmt;

use meridian_resolve::{Hierarchy, LoadError, Loader, Resolved, Scope};

pub use eval::{
    evaluate, Layer, MergeKind, MergePlan, Origin, Stack, Violation, CAPABILITY_RULE,
    UNCLAIMED_RULE,
};
pub use files::{check_files, Checked, Files, RULES_FILE};
pub use model::{
    Check, DefaultAccess, Group, Rule, Rules, COMMAND_LINE, DEFAULT_USE, PLACEHOLDERS, SHIPPED,
    UNCLAIMED,
};
pub use parse::RULES_VERSION;
pub use pattern::{KeyPattern, PathGlob};

/// The rules could not be read or evaluated. Not a violation: a mistake in
/// the rules file or the hierarchy itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error(String);

impl Error {
    pub(crate) fn new(msg: impl Into<String>) -> Self {
        Error(msg.into())
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for Error {}

/// Why layered data was not handed over.
#[derive(Clone, Debug, PartialEq)]
pub enum Refusal {
    /// The hierarchy, a layer or the rules file is itself wrong.
    Error(Error),
    /// The layers break the rules. Every violation is listed, never only
    /// the first.
    Violations(Vec<Violation>),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Refusal::Error(e) => e.fmt(f),
            Refusal::Violations(violations) => {
                write!(
                    f,
                    "compile rules: {} violation{}; nothing was compiled",
                    violations.len(),
                    if violations.len() == 1 { "" } else { "s" }
                )?;
                for violation in violations {
                    write!(f, "\n  - {violation}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for Refusal {}

impl From<Error> for Refusal {
    fn from(e: Error) -> Self {
        Refusal::Error(e)
    }
}

/// The layers of a hierarchy as they were loaded, and their merged result.
#[derive(Clone, Debug, PartialEq)]
pub struct Loaded {
    /// The layers that exist, least specific first, command-line ones last.
    pub layers: Vec<Layer>,
    /// The merged result, command-line layers included.
    pub resolved: Resolved,
}

/// Loads and merges a hierarchy, then the `command_line` layers over it (a
/// key in one replaces the same key beneath it, whole), without checking any
/// rule.
pub fn load(
    hierarchy: &Hierarchy,
    scope: &Scope,
    loader: &dyn Loader,
    command_line: &[Layer],
) -> Result<Loaded, Error> {
    let mut resolved = meridian_resolve::resolve(hierarchy, scope, loader)
        .map_err(|e| Error::new(e.to_string()))?;
    let mut layers = Vec::new();
    for layer in resolved.layers.iter().filter(|l| l.found) {
        // The same loader that just produced the layer for the merge.
        let data = loader.load(&layer.path).map_err(|e| {
            Error::new(match e {
                LoadError::NotFound => format!(
                    "{:?}: the layer disappeared while it was being read",
                    layer.path
                ),
                LoadError::Failed(why) => format!("{:?}: {why}", layer.path),
            })
        })?;
        layers.push(Layer::new(layer.path.clone(), data));
    }
    for layer in command_line {
        resolved.data.extend(layer.data.clone());
        layers.push(Layer {
            origin: Origin::CommandLine,
            ..layer.clone()
        });
    }
    Ok(Loaded { layers, resolved })
}

/// The one call a compiler makes: load and merge the hierarchy and the
/// command-line layers, check every rule, and hand the merged data over only
/// when none is broken.
///
/// `command_line` holds what `--data` and `--set` give. Those values are
/// merged last and win, so they are checked as the most specific layers, in
/// the group `(command line)`: a protected value cannot be changed from the
/// command line either.
pub fn enforce(
    rules: &Rules,
    hierarchy: &Hierarchy,
    scope: &Scope,
    loader: &dyn Loader,
    command_line: &[Layer],
) -> Result<Loaded, Refusal> {
    let loaded = load(hierarchy, scope, loader, command_line)?;
    let violations = evaluate(
        rules,
        &Stack {
            layers: &loaded.layers,
            plan: hierarchy,
            resolved: &loaded.resolved.data,
        },
    )?;
    if violations.is_empty() {
        Ok(loaded)
    } else {
        Err(Refusal::Violations(violations))
    }
}
