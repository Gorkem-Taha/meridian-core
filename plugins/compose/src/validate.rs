//! Stage three: the constraints that are Compose's and not Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::CompileError;
use meridian_yaml::Yaml;

use crate::{refuse_at, Entry, Kind, Project, NAME};

/// What a dependency can wait for.
const CONDITIONS: &[&str] = &[
    "service_completed_successfully",
    "service_healthy",
    "service_started",
];

pub(crate) fn validate(project: &Project) -> Result<(), CompileError> {
    let document = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };

    let name = &project.settings.name;
    if !is_project_name(name) {
        return Err(document(
            "invalid-project-name",
            format!(
                "{name:?} cannot be a Compose project name: one starts with a lowercase letter \
                 or a digit and holds only lowercase letters, digits, hyphens and underscores. \
                 Name the project with a resource of type \"project\""
            ),
        ));
    }
    if project.of_kind(Kind::Service).next().is_none() {
        return Err(document(
            "empty-project",
            "a Compose project needs at least one service".to_owned(),
        ));
    }

    for entry in project.of_kind(Kind::Service) {
        let refuse = |rule: &str, msg: String| refuse_at(&entry.id, &entry.pos, rule, msg);
        if !["image", "build", "extends", "provider"]
            .iter()
            .any(|key| entry.attribute(key).is_some())
        {
            return Err(refuse(
                "missing-image",
                "a service needs an \"image\" to run or a \"build\" to make one from".to_owned(),
            ));
        }
        for dependency in &entry.depends_on {
            if dependency.service == entry.id {
                return Err(refuse(
                    "invalid-reference",
                    "a service cannot depend on itself".to_owned(),
                ));
            }
            match project.entry(&dependency.service) {
                Some(other) if other.kind == Kind::Service => {}
                Some(other) => {
                    return Err(refuse(
                        "invalid-reference",
                        format!(
                            "depends_on names {:?}, which is a {} and not a service",
                            dependency.service,
                            other.kind.as_str()
                        ),
                    ))
                }
                None => {
                    return Err(refuse(
                        "invalid-reference",
                        format!(
                            "depends_on names {:?}, which is not a service in this document",
                            dependency.service
                        ),
                    ))
                }
            }
            if !CONDITIONS.contains(&dependency.condition.as_str()) {
                return Err(refuse(
                    "invalid-param",
                    format!(
                        "{:?} is not something a dependency can wait for (accepted: {})",
                        dependency.condition,
                        CONDITIONS.join(", ")
                    ),
                ));
            }
            if dependency.condition == "service_healthy"
                && !project
                    .entry(&dependency.service)
                    .is_some_and(Entry::has_healthcheck)
            {
                return Err(refuse(
                    "invalid-param",
                    format!(
                        "depends_on waits for {:?} to be healthy, and that service declares no \
                         healthcheck, so it never would be",
                        dependency.service
                    ),
                ));
            }
        }
        for (kind, name) in used(entry) {
            // Every service is on the network named default unless it says
            // otherwise, whether or not the file declares it.
            if kind == Kind::Network && name == "default" {
                continue;
            }
            match project.entry(&name) {
                Some(other) if other.kind == kind => {}
                Some(other) => {
                    return Err(refuse(
                        "invalid-reference",
                        format!(
                            "uses {name:?} as a {}, and the document declares it as a {}",
                            kind.as_str(),
                            other.kind.as_str()
                        ),
                    ))
                }
                None => {
                    return Err(refuse(
                        "invalid-reference",
                        format!(
                            "uses the {kind} {name:?}, which the document does not declare. Add \
                             a resource of type {kind:?} with that id",
                            kind = kind.as_str()
                        ),
                    ))
                }
            }
        }
    }

    if let Some(cycle) = find_cycle(project) {
        let first = project
            .entry(&cycle[0])
            .expect("a cycle holds only services of the project");
        return Err(refuse_at(
            &first.id,
            &first.pos,
            "dependency-cycle",
            format!(
                "the services depend on each other in a circle ({}), so Compose could start \
                 none of them first. A notify counts: the notified service starts after the \
                 one that notifies it",
                cycle.join(" -> ")
            ),
        ));
    }
    Ok(())
}

/// Whether Compose accepts a string as a project name.
pub(crate) fn is_project_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_lowercase() || c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

/// The networks, named volumes, configs and secrets a service says it uses.
/// A name Compose will substitute a variable into is not ours to judge.
fn used(service: &Entry) -> Vec<(Kind, String)> {
    let mut found = Vec::new();
    let mut add = |kind: Kind, name: &str| {
        if !name.is_empty() && !name.contains('$') {
            found.push((kind, name.to_owned()));
        }
    };
    let field = |fields: &[(String, Yaml)], name: &str| -> Option<String> {
        fields.iter().find_map(|(k, v)| match v {
            Yaml::Str(s) if k == name => Some(s.clone()),
            _ => None,
        })
    };

    match service.attribute("networks") {
        Some(Yaml::Seq(items)) => {
            for item in items {
                if let Yaml::Str(name) = item {
                    add(Kind::Network, name);
                }
            }
        }
        Some(Yaml::Map(entries)) => {
            for (name, _) in entries {
                add(Kind::Network, name);
            }
        }
        _ => {}
    }

    if let Some(Yaml::Seq(mounts)) = service.attribute("volumes") {
        for mount in mounts {
            match mount {
                Yaml::Str(short) => {
                    if let Some(name) = named_volume(short) {
                        add(Kind::Volume, name);
                    }
                }
                Yaml::Map(fields) if field(fields, "type").as_deref() == Some("volume") => {
                    if let Some(source) = field(fields, "source") {
                        add(Kind::Volume, &source);
                    }
                }
                _ => {}
            }
        }
    }

    for (attribute, kind) in [("configs", Kind::Config), ("secrets", Kind::Secret)] {
        if let Some(Yaml::Seq(items)) = service.attribute(attribute) {
            for item in items {
                match item {
                    Yaml::Str(name) => add(kind, name),
                    Yaml::Map(fields) => {
                        if let Some(source) = field(fields, "source") {
                            add(kind, &source);
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    found
}

/// The named volume a short-form mount uses, if it uses one.
///
/// `data:/var/lib/data` mounts the volume `data`. `/var/lib/data` alone is an
/// anonymous volume, and a source that is a path (`./data`, `/srv/data`,
/// `~/data`, `C:\data`) is a bind mount.
fn named_volume(short: &str) -> Option<&str> {
    let (source, rest) = short.split_once(':')?;
    let path = source.starts_with(['.', '/', '~', '\\'])
        || (source.len() == 1 && rest.starts_with(['\\', '/']));
    (!path).then_some(source)
}

/// One circle of services that depend on each other, as the ids along it
/// with the first repeated at the end, or `None` when there is none.
fn find_cycle(project: &Project) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Mark {
        Visiting,
        Done,
    }
    fn visit<'a>(
        id: &'a str,
        project: &'a Project,
        marks: &mut BTreeMap<&'a str, Mark>,
        path: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        match marks.get(id) {
            Some(Mark::Done) => return None,
            Some(Mark::Visiting) => {
                let start = path.iter().position(|p| *p == id).unwrap_or(0);
                let mut cycle: Vec<String> =
                    path[start..].iter().map(|p| (*p).to_owned()).collect();
                cycle.push(id.to_owned());
                return Some(cycle);
            }
            None => {}
        }
        marks.insert(id, Mark::Visiting);
        path.push(id);
        if let Some(entry) = project.entry(id) {
            for dependency in &entry.depends_on {
                if let Some(cycle) = visit(&dependency.service, project, marks, path) {
                    return Some(cycle);
                }
            }
        }
        path.pop();
        marks.insert(id, Mark::Done);
        None
    }

    let mut marks = BTreeMap::new();
    project
        .of_kind(Kind::Service)
        .find_map(|entry| visit(&entry.id, project, &mut marks, &mut Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_project_name_is_what_compose_accepts() {
        for (name, want) in [
            ("shop", true),
            ("shop-prod_2", true),
            ("2shop", true),
            ("Shop", false),
            ("-shop", false),
            ("_shop", false),
            ("shop.prod", false),
            ("", false),
        ] {
            assert_eq!(is_project_name(name), want, "{name:?}");
        }
    }

    #[test]
    fn a_short_mount_names_a_volume_unless_its_source_is_a_path() {
        for (short, want) in [
            ("data:/var/lib/data", Some("data")),
            ("data:/var/lib/data:ro", Some("data")),
            ("/var/lib/data", None),
            ("./data:/var/lib/data", None),
            ("../data:/var/lib/data", None),
            ("/srv/data:/var/lib/data", None),
            ("~/data:/var/lib/data", None),
            ("C:\\data:/var/lib/data", None),
            ("C:/data:/var/lib/data", None),
        ] {
            assert_eq!(named_volume(short), want, "{short:?}");
        }
    }
}
