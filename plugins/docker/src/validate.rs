//! Stage three: the constraints that are Compose's and not Meridian's.

use meridian_sdk::CompileError;

use crate::{refuse_at, Composition, Section, NAME};

/// A Compose project name: lower-case letters, digits, hyphens and
/// underscores, starting with a letter or digit.
fn is_project_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

pub(crate) fn validate(composition: &Composition) -> Result<(), CompileError> {
    let whole = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };
    if !is_project_name(&composition.project.name) {
        return Err(whole(
            "invalid-name",
            format!(
                "{:?} cannot be the name of a Compose project: a name holds only lower-case \
                 letters, digits, hyphens and underscores, and starts with a letter or digit. \
                 Set name on a resource of type \"project\"",
                composition.project.name
            ),
        ));
    }
    if !composition.project.host.is_empty() && !composition.project.context.is_empty() {
        return Err(whole(
            "invalid-param",
            "the project names both a host and a context; Docker takes one or the other".to_owned(),
        ));
    }
    if !composition
        .elements
        .iter()
        .any(|e| e.section == Section::Service)
    {
        return Err(whole(
            "empty-project",
            "there is nothing to run: the document declares no service".to_owned(),
        ));
    }
    for element in &composition.elements {
        if element.section == Section::Service
            && !element.body.contains_key("image")
            && !element.body.contains_key("build")
            && !element.body.contains_key("extends")
        {
            return Err(refuse_at(
                &element.id,
                &element.pos,
                "missing-param",
                "a service needs an image to run, or a build to make one".to_owned(),
            ));
        }
    }
    let mut outputs = std::collections::BTreeSet::new();
    for (name, _) in &composition.outputs {
        if !outputs.insert(name) {
            return Err(whole(
                "duplicate-output",
                format!("output {name:?} is declared twice"),
            ));
        }
    }
    Ok(())
}
