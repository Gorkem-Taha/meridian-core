//! Stage three: the constraints that are Pulumi's and not Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::{CompileError, Position};

use crate::transform::PULUMI_NAMESPACE;
use crate::{refuse_at, Program};

/// A name the rest of a program can refer to inside `${}`: a letter or
/// underscore, then letters, digits, underscores and hyphens.
fn is_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// A project name: letters, digits, underscores, hyphens and periods.
fn is_project_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
}

/// A stack name, which may carry an organisation and project in front:
/// `acme/web/prod`.
fn is_stack_name(name: &str) -> bool {
    !name.is_empty()
        && name.split('/').count() <= 3
        && name.split('/').all(|part| {
            !part.is_empty()
                && part
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_-.".contains(c))
        })
}

/// A provider setting's name: `<package>:<key>`.
fn is_setting_name(name: &str) -> bool {
    name.split_once(':').is_some_and(|(package, key)| {
        let word = |w: &str, extra: &str| {
            !w.is_empty()
                && w.chars()
                    .all(|c| c.is_ascii_alphanumeric() || extra.contains(c))
        };
        word(package, "-_") && word(key, "-_.")
    })
}

pub(crate) fn validate(program: &Program) -> Result<(), CompileError> {
    if !is_project_name(&program.project) {
        return Err(refuse_at(
            "",
            &Position::default(),
            "invalid-name",
            format!(
                "the document's name {:?} cannot be a Pulumi project name: a project name holds \
                 only letters, digits, underscores, hyphens and periods",
                program.project
            ),
        ));
    }

    if let Some((id, pos)) = &program.settings.declared {
        if !is_stack_name(&program.settings.stack) {
            return Err(refuse_at(
                id,
                pos,
                "invalid-name",
                format!(
                    "{:?} cannot be a Pulumi stack name: a stack name holds only letters, \
                     digits, underscores, hyphens and periods, with an organisation and \
                     project in front at most",
                    program.settings.stack
                ),
            ));
        }
    }

    // Resources, function calls and configuration values share one set of
    // names in a program.
    let mut names: BTreeMap<&str, &str> = BTreeMap::new();
    for entry in &program.entries {
        let ok = if entry.addressable() {
            is_name(&entry.name)
        } else {
            is_setting_name(&entry.name)
        };
        if !ok {
            let what = if entry.name == entry.id {
                format!("resource id {:?}", entry.id)
            } else {
                format!("name {:?}", entry.name)
            };
            return Err(refuse_at(
                &entry.id,
                &entry.pos,
                "invalid-name",
                format!(
                    "{what} cannot be a name in a Pulumi program: a name starts with a letter \
                     or underscore and holds only letters, digits, underscores and hyphens"
                ),
            ));
        }
        if entry.name == PULUMI_NAMESPACE {
            return Err(refuse_at(
                &entry.id,
                &entry.pos,
                "invalid-name",
                "the name \"pulumi\" is Pulumi's own: ${pulumi.stack} and its like would no \
                 longer be readable"
                    .to_owned(),
            ));
        }
        if let Some(first) = names.insert(&entry.name, &entry.id) {
            return Err(refuse_at(
                &entry.id,
                &entry.pos,
                "duplicate-name",
                format!(
                    "the name {:?} is already taken by resource {first:?}; resources, function \
                     calls and configuration values share one set of names in a Pulumi program",
                    entry.name
                ),
            ));
        }
    }

    let mut outputs = BTreeMap::new();
    for (name, _, pos) in &program.outputs {
        if name.is_empty() {
            return Err(refuse_at(
                "",
                pos,
                "invalid-name",
                "an output needs a name".to_owned(),
            ));
        }
        if outputs.insert(name, ()).is_some() {
            return Err(refuse_at(
                "",
                pos,
                "duplicate-output",
                format!("output {name:?} is declared twice"),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_pulumis_rules() {
        assert!(is_name("web_vm"));
        assert!(is_name("my-bucket"));
        assert!(is_name("_private"));
        assert!(!is_name("9lives"));
        assert!(!is_name("web.vm"));
        assert!(!is_name(""));

        assert!(is_project_name("web-vm"));
        assert!(is_project_name("web.vm_2"));
        assert!(!is_project_name("web vm"));
        assert!(!is_project_name(""));

        assert!(is_stack_name("dev"));
        assert!(is_stack_name("acme/web/prod"));
        assert!(!is_stack_name("a/b/c/d"));
        assert!(!is_stack_name("prod eu"));
        assert!(!is_stack_name("acme//prod"));

        assert!(is_setting_name("aws:region"));
        assert!(is_setting_name("azure-native:location"));
        assert!(!is_setting_name("aws:"));
        assert!(!is_setting_name("aws:region:extra"));
    }
}
