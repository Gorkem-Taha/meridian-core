//! The condition stage: `when` expressions, and what happens to the edges of
//! a resource one of them removes.
//!
//! A `when` is decided by Meridian before anything is compiled, so a resource
//! is either in the compiled output or it is not, and no target ever sees a
//! conditional. The language is Meridian's one expression language, read and
//! decided by `meridian-expr`; this module says what a condition may read
//! (the data given to the command) and what a false one does to a document.
//!
//! Nothing gathers facts from a node yet, so `facts.*` is refused when the
//! condition is read: a condition that could not be decided would be worse
//! than one that cannot be written.

use std::collections::{BTreeMap, HashSet};

use meridian_expr::{Expr, Names, Scope};
use meridian_value::Value;

use crate::error::Error;
use crate::types::{IfMissing, Resource};

/// The data a condition is decided against: the same map every target is
/// handed when it compiles.
pub type Data = BTreeMap<String, Value>;

/// A parsed `when` expression.
#[derive(Clone, Debug, PartialEq)]
pub struct Condition(Expr);

/// What a `when` may read.
fn names() -> Names {
    Names::new()
        .allow("data")
        .refuse(
            "facts",
            "facts are not available: Meridian does not gather facts from a node yet, so a \
             condition can only read data.<key>. To decide on the node while applying, use \
             runtimeWhen on a target that supports it",
        )
        .hint("a condition reads data.<key>; text is quoted")
}

/// The scope a `when` is decided in.
fn scope(data: &Data) -> Scope {
    Scope::new()
        .with("data", Value::Map(data.clone()))
        .with_unset_hint(
            "data",
            "give it with --data or --set, or test for it with defined({path})",
        )
}

impl Condition {
    /// Reads a `when` expression. Nothing is evaluated: this is the check
    /// `meridian validate` makes.
    pub fn parse(text: &str) -> Result<Condition, String> {
        let expr = Expr::parse_condition(text)?;
        expr.check(&names())?;
        Ok(Condition(expr))
    }

    /// Decides the condition against `data`. Anything that is not plainly
    /// true or false is an error, never a guess.
    pub fn evaluate(&self, data: &Data) -> Result<bool, String> {
        self.0.truth(&scope(data))
    }
}

/// Decides every `when` in a document's resources and returns the ones that
/// remain, unconditional, in their declared order.
///
/// An edge that pointed at a removed resource follows its `ifMissing`: `skip`
/// drops the edge, and `error` (the default) refuses the document, because a
/// resource that depends on something that is not there is far more often a
/// mistake than an intention. Edges *from* a removed resource go with it.
pub fn prune(resources: &[Resource], data: &Data) -> Result<Vec<Resource>, Error> {
    let scope = scope(data);
    let mut removed: HashSet<&str> = HashSet::new();
    for (i, r) in resources.iter().enumerate() {
        if r.when.is_empty() {
            continue;
        }
        let keep = Condition::parse(&r.when)
            .and_then(|condition| condition.0.truth(&scope))
            .map_err(|why| {
                Error::new(
                    r.pos.clone(),
                    format!("spec.resources[{i}].when"),
                    format!("resource {:?}: {why}", r.id),
                )
            })?;
        if !keep {
            removed.insert(&r.id);
        }
    }

    let mut kept = Vec::with_capacity(resources.len() - removed.len());
    for (i, r) in resources.iter().enumerate() {
        if removed.contains(r.id.as_str()) {
            continue;
        }
        let mut r = r.clone();
        r.when = String::new();

        let dangling = |verb: &str, field: &str, j: usize, missing: &str, pos| {
            Error::new(
                pos,
                format!("spec.resources[{i}].{field}[{j}].resource"),
                format!(
                    "resource {:?} {verb} {missing:?}, which its `when` removed from this \
                     run; set ifMissing: skip on this entry if the resource should carry \
                     on without it",
                    r.id
                ),
            )
        };
        for (j, dep) in r.depends_on.iter().enumerate() {
            if removed.contains(dep.resource.as_str()) && dep.if_missing != IfMissing::Skip {
                return Err(dangling(
                    "depends on",
                    "dependsOn",
                    j,
                    &dep.resource,
                    dep.pos.clone(),
                ));
            }
        }
        for (j, n) in r.notifies.iter().enumerate() {
            if removed.contains(n.resource.as_str()) && n.if_missing != IfMissing::Skip {
                return Err(dangling(
                    "notifies",
                    "notifies",
                    j,
                    &n.resource,
                    n.pos.clone(),
                ));
            }
        }
        r.depends_on
            .retain(|dep| !removed.contains(dep.resource.as_str()));
        r.notifies
            .retain(|n| !removed.contains(n.resource.as_str()));
        kept.push(r);
    }
    Ok(kept)
}
