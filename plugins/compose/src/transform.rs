//! Stage one: each resource becomes an element of the Compose file, and
//! every reference between resources is checked and becomes a name.

use std::collections::BTreeMap;

use meridian_sdk::expr::{self, Part};
use meridian_sdk::{CompileError, Resource, ResourceGraph, Value, Warning};
use meridian_yaml::Yaml;

use crate::{
    plain, refuse, refuse_at, Dependency, Entry, Kind, Project, Settings, NAME, PROJECT_TYPE,
};

/// The attributes of a service, from the Compose specification.
const SERVICE_KEYS: &[&str] = &[
    "annotations",
    "attach",
    "blkio_config",
    "build",
    "cap_add",
    "cap_drop",
    "cgroup",
    "cgroup_parent",
    "command",
    "configs",
    "container_name",
    "cpu_count",
    "cpu_percent",
    "cpu_period",
    "cpu_quota",
    "cpu_rt_period",
    "cpu_rt_runtime",
    "cpu_shares",
    "cpus",
    "cpuset",
    "credential_spec",
    "depends_on",
    "deploy",
    "develop",
    "device_cgroup_rules",
    "devices",
    "dns",
    "dns_opt",
    "dns_search",
    "domainname",
    "driver_opts",
    "entrypoint",
    "env_file",
    "environment",
    "expose",
    "extends",
    "external_links",
    "extra_hosts",
    "gpus",
    "group_add",
    "healthcheck",
    "hostname",
    "image",
    "init",
    "ipc",
    "isolation",
    "label_file",
    "labels",
    "links",
    "logging",
    "mac_address",
    "mem_limit",
    "mem_reservation",
    "mem_swappiness",
    "memswap_limit",
    "models",
    "network_mode",
    "networks",
    "oom_kill_disable",
    "oom_score_adj",
    "pid",
    "pids_limit",
    "platform",
    "ports",
    "post_start",
    "pre_stop",
    "privileged",
    "profiles",
    "provider",
    "pull_policy",
    "read_only",
    "restart",
    "runtime",
    "scale",
    "secrets",
    "security_opt",
    "shm_size",
    "stdin_open",
    "stop_grace_period",
    "stop_signal",
    "storage_opt",
    "sysctls",
    "tmpfs",
    "tty",
    "ulimits",
    "use_api_socket",
    "user",
    "userns_mode",
    "uts",
    "volumes",
    "volumes_from",
    "working_dir",
];

const NETWORK_KEYS: &[&str] = &[
    "attachable",
    "driver",
    "driver_opts",
    "enable_ipv4",
    "enable_ipv6",
    "external",
    "internal",
    "ipam",
    "labels",
    "name",
];

const VOLUME_KEYS: &[&str] = &["driver", "driver_opts", "external", "labels", "name"];

const CONFIG_KEYS: &[&str] = &[
    "content",
    "environment",
    "external",
    "file",
    "labels",
    "name",
    "template_driver",
];

const SECRET_KEYS: &[&str] = &[
    "driver",
    "driver_opts",
    "environment",
    "external",
    "file",
    "labels",
    "name",
    "template_driver",
];

const PROJECT_KEYS: &[&str] = &["directory", "name", "profiles", "wait", "wait_timeout"];

fn keys_of(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Service => SERVICE_KEYS,
        Kind::Network => NETWORK_KEYS,
        Kind::Volume => VOLUME_KEYS,
        Kind::Config => CONFIG_KEYS,
        Kind::Secret => SECRET_KEYS,
    }
}

/// What every id in the document is, and what a reference to it is written as.
struct Names {
    kinds: BTreeMap<String, Kind>,
    /// The id of the `project` resource and the project's name.
    project: Option<String>,
    project_name: String,
}

pub(crate) fn transform(graph: &ResourceGraph) -> Result<Project, CompileError> {
    let mut project = Project {
        document: graph.name.clone(),
        settings: Settings {
            name: graph.name.clone(),
            wait: true,
            ..Settings::default()
        },
        ..Project::default()
    };

    let mut kinds = BTreeMap::new();
    for r in &graph.resources {
        check_meta(r)?;
        if r.r#type == PROJECT_TYPE {
            if project.settings_id.is_some() {
                return Err(refuse(
                    r,
                    "duplicate-project",
                    "a document is one Compose project; this one has two resources of type \
                     \"project\""
                        .to_owned(),
                ));
            }
            project.settings_id = Some(r.id.clone());
            read_settings(r, &mut project.settings)?;
            continue;
        }
        let Some(kind) = Kind::from_type(&r.r#type) else {
            return Err(refuse(
                r,
                "unsupported-resource-type",
                format!(
                    "{:?} is not something a Compose file declares (accepted: config, network, \
                     project, secret, service, volume)",
                    r.r#type
                ),
            ));
        };
        if !is_name(&r.id) {
            return Err(refuse(
                r,
                "invalid-name",
                format!(
                    "resource id {:?} cannot name a {} in a Compose file: a name starts with a \
                     letter or digit and holds only letters, digits, dots, hyphens and underscores",
                    r.id,
                    kind.as_str()
                ),
            ));
        }
        kinds.insert(r.id.clone(), kind);
    }
    let names = Names {
        kinds,
        project: project.settings_id.clone(),
        project_name: project.settings.name.clone(),
    };

    for r in &graph.resources {
        let Some(&kind) = names.kinds.get(&r.id) else {
            continue;
        };
        let mut body = Vec::with_capacity(r.params.len());
        let mut depends_on = Vec::new();
        for (name, value) in &r.params {
            if !keys_of(kind).contains(&name.as_str()) && !name.starts_with("x-") {
                return Err(refuse(r, "unsupported-param", unknown_key(kind, name)));
            }
            let rewritten = rewrite(value, &names)
                .map_err(|msg| refuse(r, "invalid-reference", format!("param {name:?}: {msg}")))?;
            if kind == Kind::Service && name == "depends_on" {
                depends_on = read_depends_on(&rewritten)
                    .map_err(|msg| refuse(r, "invalid-param", format!("param {name:?}: {msg}")))?;
            } else {
                body.push((name.clone(), rewritten));
            }
        }
        let entry = Entry {
            id: r.id.clone(),
            kind,
            body,
            depends_on,
            pos: r.pos.clone(),
        };
        if project.settings.directory.is_empty() {
            for (what, path) in relative_paths(&entry) {
                project.warnings.push(Warning {
                    target: NAME.to_owned(),
                    resource: r.id.clone(),
                    msg: format!(
                        "{what} {path:?} is a relative path, which Compose resolves against the \
                         directory Meridian keeps the compiled file in and not the document's. \
                         Write an absolute path, or name the directory with \"directory\" on a \
                         resource of type \"project\""
                    ),
                });
            }
        }
        project.entries.push(entry);
    }

    for output in &graph.outputs {
        let value = output_template(&output.value, &names, &project).map_err(|msg| {
            refuse_at(
                "",
                &output.pos,
                "invalid-reference",
                format!("output {:?}: {msg}", output.name),
            )
        })?;
        project.outputs.push((output.name.clone(), value));
    }
    Ok(project)
}

/// Whether Compose accepts a string as the name of a service, network,
/// volume, config or secret.
fn is_name(id: &str) -> bool {
    id.starts_with(|c: char| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
}

fn check_meta(r: &Resource) -> Result<(), CompileError> {
    if !r.runtime_when.is_empty() {
        return Err(refuse(
            r,
            "unsupported-runtime-condition",
            format!(
                "resource carries runtimeWhen {:?}, which this target cannot express: Compose \
                 has no condition that reads the state of the host when the project is brought \
                 up. A profile, chosen before it runs, is the nearest thing it has",
                r.runtime_when
            ),
        ));
    }
    match r.state.as_str() {
        "" | "present" => Ok(()),
        "absent" => Err(refuse(
            r,
            "unsupported-state",
            "state \"absent\" has no meaning on this target: Compose removes the containers of \
             a service its file no longer declares, so remove the resource from the document"
                .to_owned(),
        )),
        other => Err(refuse(
            r,
            "unsupported-state",
            format!(
                "resource type {:?} does not accept state {other:?} on this target (accepted: present)",
                r.r#type
            ),
        )),
    }
}

fn read_settings(r: &Resource, settings: &mut Settings) -> Result<(), CompileError> {
    let invalid = |msg: String| refuse(r, "invalid-param", msg);
    for (name, value) in &r.params {
        match (name.as_str(), value) {
            ("name", Value::String(s)) => settings.name = s.clone(),
            ("directory", Value::String(s)) if is_absolute(s) => settings.directory = s.clone(),
            ("directory", _) => {
                return Err(invalid(
                    "param \"directory\" of the project must be an absolute path: it is the \
                     directory Compose resolves the file's relative paths against"
                        .to_owned(),
                ))
            }
            ("profiles", Value::List(items)) => {
                for item in items {
                    let Value::String(profile) = item else {
                        return Err(invalid("every profile must be a string".to_owned()));
                    };
                    settings.profiles.push(profile.clone());
                }
            }
            ("profiles", _) => {
                return Err(invalid(
                    "param \"profiles\" of the project must be a list".to_owned(),
                ))
            }
            ("wait", Value::Bool(wait)) => settings.wait = *wait,
            ("wait", _) => {
                return Err(invalid(
                    "param \"wait\" of the project must be true or false".to_owned(),
                ))
            }
            ("wait_timeout", Value::Int(seconds)) if *seconds > 0 => {
                settings.wait_timeout = *seconds as u64;
            }
            ("wait_timeout", _) => {
                return Err(invalid(
                    "param \"wait_timeout\" of the project must be a whole number of seconds, \
                     greater than zero"
                        .to_owned(),
                ))
            }
            ("name", _) => {
                return Err(invalid(
                    "param \"name\" of the project must be a string".to_owned(),
                ))
            }
            _ => {
                return Err(refuse(
                    r,
                    "unsupported-param",
                    format!(
                        "resource type \"project\" does not accept param {name:?} on this target \
                         (accepted: {})",
                        PROJECT_KEYS.join(", ")
                    ),
                ))
            }
        }
    }
    Ok(())
}

fn unknown_key(kind: Kind, name: &str) -> String {
    let nearest = keys_of(kind)
        .iter()
        .map(|key| (distance(name, key), *key))
        .filter(|(d, _)| *d <= 2)
        .min();
    let hint = match nearest {
        Some((_, key)) => format!("did you mean {key:?}?"),
        None => "an extension field starts with \"x-\"".to_owned(),
    };
    format!(
        "{name:?} is not an attribute of a Compose {}; {hint}",
        kind.as_str()
    )
}

/// The edit distance between two short words.
fn distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substituted = diagonal + usize::from(ca != cb);
            diagonal = row[j + 1];
            row[j + 1] = substituted.min(row[j] + 1).min(diagonal + 1);
        }
    }
    row[b.len()]
}

/// Rewrites every `${id}` in a value to the name Compose knows that resource
/// by, refusing a reference to nothing.
fn rewrite(value: &Value, names: &Names) -> Result<Yaml, String> {
    Ok(match value {
        Value::String(s) => Yaml::str(rewrite_string(s, names)?),
        Value::List(items) => Yaml::Seq(
            items
                .iter()
                .map(|item| rewrite(item, names))
                .collect::<Result<_, _>>()?,
        ),
        Value::Map(map) => Yaml::Map(
            map.iter()
                .map(|(k, v)| Ok((k.clone(), rewrite(v, names)?)))
                .collect::<Result<_, String>>()?,
        ),
        other => plain(other),
    })
}

fn rewrite_string(text: &str, names: &Names) -> Result<String, String> {
    let mut out = String::with_capacity(text.len());
    for part in expr::parse(text)? {
        match part {
            // What was `$${` in the document is `${` here, and Compose reads
            // it as a variable of its own.
            Part::Text(literal) => out.push_str(&literal),
            Part::Expr(expression) => {
                let (name, rest) = expr::head(&expression);
                if names.project.as_deref() == Some(name) && matches!(rest, "" | ".name") {
                    out.push_str(&names.project_name);
                    continue;
                }
                if !names.kinds.contains_key(name) {
                    return Err(unknown_reference(&expression, name));
                }
                if !rest.is_empty() {
                    return Err(format!(
                        "${{{expression}}} is not a reference a Compose file can hold: nothing \
                         about another resource is known until Docker runs it. Write ${{{name}}} \
                         for its name"
                    ));
                }
                out.push_str(name);
            }
        }
    }
    Ok(out)
}

fn unknown_reference(expression: &str, name: &str) -> String {
    format!(
        "${{{expression}}} refers to {name:?}, which is not a resource in this document. For one \
         of Compose's own variables, write $${{{expression}}}"
    )
}

/// Reads a `depends_on` the author wrote, in either of Compose's forms.
fn read_depends_on(value: &Yaml) -> Result<Vec<Dependency>, String> {
    let fresh = |service: &str| Dependency {
        service: service.to_owned(),
        condition: String::new(),
        restart: false,
        required: None,
    };
    match value {
        Yaml::Seq(items) => items
            .iter()
            .map(|item| match item {
                Yaml::Str(service) => Ok(fresh(service)),
                _ => Err("every entry of the list must be the name of a service".to_owned()),
            })
            .collect(),
        Yaml::Map(entries) => entries
            .iter()
            .map(|(service, fields)| {
                let mut dependency = fresh(service);
                let fields = match fields {
                    Yaml::Null => &[][..],
                    Yaml::Map(fields) => fields.as_slice(),
                    _ => return Err(format!("the entry for {service:?} must be a mapping")),
                };
                for (field, value) in fields {
                    match (field.as_str(), value) {
                        ("condition", Yaml::Str(condition)) => {
                            dependency.condition.clone_from(condition);
                        }
                        ("restart", Yaml::Bool(restart)) => dependency.restart = *restart,
                        ("required", Yaml::Bool(required)) => dependency.required = Some(*required),
                        ("condition", _) => {
                            return Err(format!("the condition for {service:?} must be a string"))
                        }
                        ("restart" | "required", _) => {
                            return Err(format!("{field:?} for {service:?} must be true or false"))
                        }
                        _ => {
                            return Err(format!(
                                "{field:?} is not a field of a dependency (accepted: condition, \
                                 required, restart)"
                            ))
                        }
                    }
                }
                Ok(dependency)
            })
            .collect(),
        _ => Err("must be a list of service names, or a mapping keyed by them".to_owned()),
    }
}

fn is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('~')
        || (path.len() > 2
            && path.as_bytes()[1] == b':'
            && path.as_bytes()[0].is_ascii_alphabetic())
}

/// Whether a path is one Compose would resolve against the project
/// directory. A URL, or anything Compose will substitute a variable into, is
/// not ours to judge.
fn is_relative(path: &str) -> bool {
    !path.is_empty()
        && !is_absolute(path)
        && !path.contains("://")
        && !path.starts_with("git@")
        && !path.contains('$')
}

/// The relative host paths an element names, each with what it is.
fn relative_paths(entry: &Entry) -> Vec<(&'static str, String)> {
    let mut found = Vec::new();
    let mut check = |what: &'static str, path: &str| {
        if is_relative(path) {
            found.push((what, path.to_owned()));
        }
    };
    let field = |fields: &[(String, Yaml)], name: &str| -> Option<String> {
        fields.iter().find_map(|(k, v)| match v {
            Yaml::Str(s) if k == name => Some(s.clone()),
            _ => None,
        })
    };
    match entry.kind {
        Kind::Service => {
            match entry.attribute("build") {
                Some(Yaml::Str(context)) => check("the build context", context),
                Some(Yaml::Map(fields)) => {
                    if let Some(context) = field(fields, "context") {
                        check("the build context", &context);
                    }
                }
                _ => {}
            }
            match entry.attribute("env_file") {
                Some(Yaml::Str(path)) => check("the env_file", path),
                Some(Yaml::Seq(items)) => {
                    for item in items {
                        match item {
                            Yaml::Str(path) => check("the env_file", path),
                            Yaml::Map(fields) => {
                                if let Some(path) = field(fields, "path") {
                                    check("the env_file", &path);
                                }
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
            if let Some(Yaml::Seq(mounts)) = entry.attribute("volumes") {
                for mount in mounts {
                    match mount {
                        // In the short form only a source that starts with a
                        // dot is a path; a bare word is a named volume.
                        Yaml::Str(short) => {
                            let source = short.split(':').next().unwrap_or_default();
                            if source.starts_with('.') && short.contains(':') {
                                check("the bind mount", source);
                            }
                        }
                        Yaml::Map(fields) if field(fields, "type").as_deref() == Some("bind") => {
                            if let Some(source) = field(fields, "source") {
                                check("the bind mount", &source);
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        Kind::Config | Kind::Secret => {
            if let Some(Yaml::Str(path)) = entry.attribute("file") {
                check("the file", path);
            }
        }
        Kind::Network | Kind::Volume => {}
    }
    found
}

/// The name Docker knows a network, volume, config or secret by.
fn real_name(entry: &Entry, project_name: &str) -> String {
    if let Some(Yaml::Str(name)) = entry.attribute("name") {
        return name.clone();
    }
    if entry.attribute("external") == Some(&Yaml::Bool(true)) {
        return entry.id.clone();
    }
    format!("{project_name}_{}", entry.id)
}

/// Checks an output's value and fills in what is known before anything runs.
/// What only Docker can say is left as `${service.port.80}` for the runner.
fn output_template(text: &str, names: &Names, project: &Project) -> Result<String, String> {
    let mut out = String::new();
    let literal = |text: &str| text.replace("${", "$${");
    for part in expr::parse(text)? {
        let expression = match part {
            Part::Text(text) => {
                out.push_str(&literal(&text));
                continue;
            }
            Part::Expr(expression) => expression,
        };
        let (name, rest) = expr::head(&expression);
        if names.project.as_deref() == Some(name) && matches!(rest, "" | ".name") {
            out.push_str(&literal(&names.project_name));
            continue;
        }
        let Some(entry) = project.entry(name) else {
            return Err(format!(
                "${{{expression}}} refers to {name:?}, which is not a resource in this document"
            ));
        };
        if rest.is_empty() {
            out.push_str(&literal(&match entry.kind {
                Kind::Service => entry.id.clone(),
                _ => real_name(entry, &names.project_name),
            }));
            continue;
        }
        if entry.kind != Kind::Service {
            return Err(format!(
                "${{{expression}}} reads {rest:?} of a {}, which has nothing to read but its \
                 name: write ${{{name}}}",
                entry.kind.as_str()
            ));
        }
        let attribute = Attribute::parse(&rest[1..]).ok_or_else(|| {
            format!(
                "${{{expression}}} is not something this target can read from a service \
                 (accepted: ${{{name}}} for its name, ${{{name}.id}} for its container's id, \
                 ${{{name}.port.80}} for the host port published for its port 80, \
                 ${{{name}.address.80}} for that port with its host, and /udp after a port \
                 that is not TCP)"
            )
        })?;
        out.push_str(&format!("${{{name}.{attribute}}}"));
    }
    Ok(out)
}

/// What the runner can read from a running service.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Attribute {
    /// The id of the service's first container.
    Id,
    /// The host port published for a container port.
    Port { port: u16, udp: bool },
    /// The same, with the address it is published on.
    Address { port: u16, udp: bool },
}

impl Attribute {
    /// Reads `id`, `port.80`, `port.53/udp` or `address.80`.
    pub(crate) fn parse(text: &str) -> Option<Attribute> {
        if text == "id" {
            return Some(Attribute::Id);
        }
        let (what, port) = text.split_once('.')?;
        let (port, udp) = match port.split_once('/') {
            None => (port, false),
            Some((port, "tcp")) => (port, false),
            Some((port, "udp")) => (port, true),
            Some(_) => return None,
        };
        let port: u16 = port.parse().ok().filter(|p| *p > 0)?;
        match what {
            "port" => Some(Attribute::Port { port, udp }),
            "address" => Some(Attribute::Address { port, udp }),
            _ => None,
        }
    }
}

impl std::fmt::Display for Attribute {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (what, port, udp) = match self {
            Attribute::Id => return f.write_str("id"),
            Attribute::Port { port, udp } => ("port", port, udp),
            Attribute::Address { port, udp } => ("address", port, udp),
        };
        write!(f, "{what}.{port}{}", if *udp { "/udp" } else { "" })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_is_what_compose_accepts() {
        for (id, want) in [
            ("web", true),
            ("web-1", true),
            ("web_1.blue", true),
            ("1web", true),
            ("_web", false),
            ("-web", false),
            ("web app", false),
            ("", false),
        ] {
            assert_eq!(is_name(id), want, "{id:?}");
        }
    }

    #[test]
    fn a_misspelt_attribute_is_matched_to_the_nearest_real_one() {
        assert_eq!(distance("enviroment", "environment"), 1);
        assert_eq!(distance("port", "ports"), 1);
        assert_eq!(distance("image", "image"), 0);
        assert_eq!(
            unknown_key(Kind::Service, "enviroment"),
            "\"enviroment\" is not an attribute of a Compose service; did you mean \"environment\"?"
        );
        assert_eq!(
            unknown_key(Kind::Volume, "replicas"),
            "\"replicas\" is not an attribute of a Compose volume; an extension field starts with \"x-\""
        );
    }

    #[test]
    fn the_attribute_lists_are_sorted_so_a_suggestion_is_stable() {
        for kind in Kind::ALL {
            let keys = keys_of(kind);
            let mut sorted = keys.to_vec();
            sorted.sort_unstable();
            assert_eq!(keys, sorted, "{}", kind.as_str());
        }
    }

    #[test]
    fn a_path_is_relative_unless_it_is_absolute_a_url_or_a_variable() {
        for (path, want) in [
            ("./app", true),
            ("app", true),
            ("../shared/.env", true),
            ("/srv/app", false),
            ("~/app", false),
            ("C:\\app", false),
            ("https://github.com/acme/app.git", false),
            ("git@github.com:acme/app.git", false),
            ("${APP_DIR}/app", false),
            ("", false),
        ] {
            assert_eq!(is_relative(path), want, "{path:?}");
        }
    }

    #[test]
    fn a_service_attribute_reads_back_as_it_was_written() {
        for (text, want) in [
            ("id", Some(Attribute::Id)),
            (
                "port.80",
                Some(Attribute::Port {
                    port: 80,
                    udp: false,
                }),
            ),
            (
                "port.53/udp",
                Some(Attribute::Port {
                    port: 53,
                    udp: true,
                }),
            ),
            (
                "address.8080/tcp",
                Some(Attribute::Address {
                    port: 8080,
                    udp: false,
                }),
            ),
            ("port", None),
            ("port.0", None),
            ("port.http", None),
            ("port.80/sctp", None),
            ("port.70000", None),
            ("ip", None),
        ] {
            assert_eq!(Attribute::parse(text), want, "{text:?}");
        }
        assert_eq!(
            Attribute::parse("port.53/udp").unwrap().to_string(),
            "port.53/udp"
        );
        assert_eq!(
            Attribute::parse("address.80/tcp").unwrap().to_string(),
            "address.80"
        );
    }

    #[test]
    fn depends_on_is_read_in_both_of_composes_forms() {
        let short = Yaml::Seq(vec![Yaml::str("db"), Yaml::str("cache")]);
        let read = read_depends_on(&short).unwrap();
        assert_eq!(read.len(), 2);
        assert_eq!(read[0].service, "db");
        assert_eq!(read[0].condition, "");

        let long = Yaml::Map(vec![
            (
                "db".to_owned(),
                Yaml::Map(vec![
                    (
                        "condition".to_owned(),
                        Yaml::str("service_completed_successfully"),
                    ),
                    ("required".to_owned(), Yaml::Bool(false)),
                ]),
            ),
            ("cache".to_owned(), Yaml::Null),
        ]);
        let read = read_depends_on(&long).unwrap();
        assert_eq!(read[0].condition, "service_completed_successfully");
        assert_eq!(read[0].required, Some(false));
        assert_eq!(read[1].condition, "");

        let wrong = Yaml::Map(vec![(
            "db".to_owned(),
            Yaml::Map(vec![("when".to_owned(), Yaml::str("ready"))]),
        )]);
        assert_eq!(
            read_depends_on(&wrong).unwrap_err(),
            "\"when\" is not a field of a dependency (accepted: condition, required, restart)"
        );
        assert!(read_depends_on(&Yaml::str("db")).is_err());
    }
}
