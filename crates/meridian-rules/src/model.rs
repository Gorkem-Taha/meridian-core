//! A parsed rules file.

use std::collections::BTreeSet;

use meridian_ir::Value;

use crate::pattern::{KeyPattern, PathGlob};

/// The name given to a layer no group claims.
pub const UNCLAIMED: &str = "(unclaimed)";

/// The group of the `--data` files given on the command line. They are merged
/// over the hierarchy, so the rules treat them as its most specific layers.
pub const COMMAND_LINE: &str = "(command line)";

/// A named set of layers: who owns which files.
#[derive(Clone, Debug, PartialEq)]
pub struct Group {
    pub name: String,
    pub paths: Vec<PathGlob>,
    /// The keys the group's layers may set: its `capabilities.keys`. `None`
    /// when the group does not list the capability, which leaves it to the
    /// file's default.
    pub keys: Option<Vec<KeyPattern>>,
}

/// What a group may do with a capability it does not list.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DefaultAccess {
    /// Anything. The default when the file does not say, so that a rules
    /// file enforces only what it asks for.
    #[default]
    Allow,
    /// Nothing.
    Deny,
}

/// What a rule checks.
#[derive(Clone, Debug, PartialEq)]
pub enum Check {
    /// A more specific layer may not change a value a less specific layer
    /// set. Restating the same value is allowed.
    NoOverride,
    /// Once a layer sets the key, no more specific layer may set it at all,
    /// not even to the same value.
    Final,
    /// Only the named groups may set the key.
    Restrict { allow: Vec<String> },
    /// The key must be set: in every layer of the groups the rule applies
    /// to, or, when the rule names none, in the merged result.
    Required,
    /// The key may not hold one of these values.
    ForbiddenValue {
        values: Vec<Value>,
        matches: Vec<String>,
        ignore_case: bool,
    },
    /// The key may only be merged with one of these strategies, by the names
    /// the merge plan gives them.
    MergeStrategy { allow: Vec<String> },
}

impl Check {
    /// The name the rule goes by in a rules file and in a refusal.
    pub fn name(&self) -> &'static str {
        match self {
            Check::NoOverride => "no-override",
            Check::Final => "final",
            Check::Restrict { .. } => "restrict",
            Check::Required => "required",
            Check::ForbiddenValue { .. } => "forbidden-value",
            Check::MergeStrategy { .. } => "merge-strategy",
        }
    }
}

/// One rule.
#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub id: String,
    pub keys: Vec<KeyPattern>,
    pub check: Check,
    /// The groups whose values the rule protects (`no-override`, `final`).
    /// `None` is every layer.
    pub set_by: Option<Vec<String>>,
    /// The groups the rule constrains. `None` is every layer.
    pub applies_to: Option<Vec<String>>,
    /// The author's own words, added to every refusal the rule produces.
    pub message: String,
}

/// A parsed rules file: the groups, then every rule in force, shipped sets
/// first.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rules {
    /// What a group may do with a capability it does not list.
    pub default: DefaultAccess,
    pub groups: Vec<Group>,
    pub rules: Vec<Rule>,
}

impl Rules {
    /// The group owning the layer at `path`: the first whose patterns match,
    /// or [`UNCLAIMED`].
    pub fn group_of(&self, path: &str) -> &str {
        self.group(path).map_or(UNCLAIMED, |g| g.name.as_str())
    }

    pub(crate) fn group(&self, path: &str) -> Option<&Group> {
        self.groups
            .iter()
            .find(|g| g.paths.iter().any(|p| p.matches(path)))
    }

    pub(crate) fn group_names(&self) -> BTreeSet<&str> {
        self.groups.iter().map(|g| g.name.as_str()).collect()
    }
}

/// The names of the rule sets that ship with Meridian, for `use:`.
pub const SHIPPED: &[(&str, &str)] = &[
    (
        "protect-variables",
        "a more specific layer may not change a value under `variables` that a less specific layer set",
    ),
    (
        "protect-secrets",
        "a more specific layer may not change a value under `secrets` or `credentials` that a less specific layer set",
    ),
    (
        "protect-all",
        "a more specific layer may not change any value a less specific layer set",
    ),
];

pub(crate) fn shipped(name: &str) -> Option<Rule> {
    let keys: &[&str] = match name {
        "protect-variables" => &["variables"],
        "protect-secrets" => &["secrets", "credentials"],
        "protect-all" => &["**"],
        _ => return None,
    };
    Some(Rule {
        id: format!("meridian/{name}"),
        keys: keys
            .iter()
            .map(|k| KeyPattern::parse(k).expect("a shipped pattern parses"))
            .collect(),
        check: Check::NoOverride,
        set_by: None,
        applies_to: None,
        message: String::new(),
    })
}
