//! Stage three: the constraints that are Salt's and not Meridian's.

use std::collections::HashMap;

use meridian_sdk::CompileError;

use crate::{Reference, Sls, State, NAME};

/// Top-level keys of a state file that Salt reads as declarations of its own
/// and never as a state's ID.
const RESERVED_IDS: &[&str] = &["exclude", "extend", "include"];

/// What must not appear in a runtime condition: it is written inside a Jinja
/// `if` tag, so anything that opens or closes a tag would end it early.
const JINJA_DELIMITERS: &[&str] = &["{%", "%}", "{{", "}}", "{#", "#}"];

fn fail(state: &State, rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        resource: state.resource_id.clone(),
        rule: rule.to_owned(),
        msg,
        pos: state.pos.clone(),
    }
}

fn fail_document(rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    }
}

/// Checks a state file before anything is serialised. Every failure names the
/// offending resource and the rule it broke, so the message points at the
/// line the author must change.
pub(crate) fn validate(sls: &Sls) -> Result<(), CompileError> {
    check_name(&sls.name)?;
    check_host(&sls.host)?;

    // Requisites name a state by its ID, so a duplicate makes the file
    // ambiguous and Salt refuses to render it.
    let mut ids: HashMap<&str, &State> = HashMap::new();
    for state in sls.all() {
        check_id(state)?;
        if let Some(previous) = ids.insert(&state.id, state) {
            return Err(fail(
                state,
                "duplicate-state-id",
                format!(
                    "state ID {:?} is already used by resource {:?}",
                    state.id, previous.resource_id
                ),
            ));
        }
    }

    for state in sls.all() {
        if state.module.is_empty() || state.function.is_empty() {
            return Err(fail(
                state,
                "missing-function",
                "state has no function".to_owned(),
            ));
        }
        if !state.args.iter().any(|(name, _)| name == "name") {
            return Err(fail(
                state,
                "missing-name",
                format!("{}.{} was given no name", state.module, state.function),
            ));
        }
        for (kind, references) in [
            ("require", &state.require),
            ("watch", &state.watch),
            ("onchanges", &state.onchanges),
        ] {
            for reference in references {
                check_reference(state, kind, reference, &ids)?;
            }
        }
        check_condition(state)?;
    }

    for reaction in &sls.reactions {
        if !reaction.when.is_empty() {
            return Err(fail(
                reaction,
                "conditional-reaction",
                format!(
                    "state {:?} carries a condition; it runs because something \
                     changed, not because a condition holds",
                    reaction.id
                ),
            ));
        }
        if reaction.watch.is_empty() && reaction.onchanges.is_empty() {
            return Err(fail(
                reaction,
                "unbound-reaction",
                format!("state {:?} reacts to nothing", reaction.id),
            ));
        }
    }
    Ok(())
}

/// A state file is applied by name, and Salt reads a dot in that name as a
/// directory separator.
fn check_name(name: &str) -> Result<(), CompileError> {
    if name.is_empty() {
        return Err(fail_document(
            "missing-name",
            "the document has no name to call its state file by".to_owned(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        return Err(fail_document(
            "invalid-sls-name",
            format!(
                "document name {name:?} cannot name a state file; Salt applies a state by name \
                 and allows letters, digits, \"_\" and \"-\" there (a dot means a directory)"
            ),
        ));
    }
    if name == "top" {
        return Err(fail_document(
            "reserved-sls-name",
            "a document named \"top\" would compile to top.sls, which Salt reads as the top file"
                .to_owned(),
        ));
    }
    Ok(())
}

/// The top file matches the host as an exact minion ID, in a comma-separated
/// list of one.
fn check_host(host: &str) -> Result<(), CompileError> {
    if host.is_empty() {
        return Err(fail_document(
            "missing-host",
            "state file has no target host".to_owned(),
        ));
    }
    if host.contains(|c: char| c.is_whitespace() || c == ',') {
        return Err(fail_document(
            "invalid-host",
            format!("host {host:?} cannot be a minion ID: it holds a space or a comma"),
        ));
    }
    Ok(())
}

fn check_id(state: &State) -> Result<(), CompileError> {
    if state.id.trim().is_empty() {
        return Err(fail(state, "empty-id", "state has an empty ID".to_owned()));
    }
    if state.id.contains(['\n', '\r']) {
        return Err(fail(
            state,
            "multiline-id",
            format!("state ID {:?} spans more than one line", state.id),
        ));
    }
    if RESERVED_IDS.contains(&state.id.as_str()) {
        return Err(fail(
            state,
            "reserved-id",
            format!(
                "state ID {:?} is a declaration Salt reads for itself (reserved: {})",
                state.id,
                RESERVED_IDS.join(", ")
            ),
        ));
    }
    Ok(())
}

fn check_reference(
    state: &State,
    kind: &str,
    reference: &Reference,
    ids: &HashMap<&str, &State>,
) -> Result<(), CompileError> {
    match ids.get(reference.id.as_str()) {
        Some(found) if found.module == reference.module => {}
        _ => {
            return Err(fail(
                state,
                "missing-requisite",
                format!(
                    "{kind} names {}: {:?}, which no state in the file is",
                    reference.module, reference.id
                ),
            ));
        }
    }
    if reference.id == state.id {
        return Err(fail(
            state,
            "self-requisite",
            format!("state {:?} names itself in its {kind}", state.id),
        ));
    }
    Ok(())
}

fn check_condition(state: &State) -> Result<(), CompileError> {
    let when = &state.when;
    if when.is_empty() {
        return Ok(());
    }
    if when.trim().is_empty() {
        return Err(fail(
            state,
            "invalid-runtime-condition",
            "runtimeWhen is blank".to_owned(),
        ));
    }
    if when.contains(['\n', '\r']) {
        return Err(fail(
            state,
            "invalid-runtime-condition",
            "runtimeWhen spans more than one line".to_owned(),
        ));
    }
    if let Some(found) = JINJA_DELIMITERS.iter().find(|d| when.contains(*d)) {
        return Err(fail(
            state,
            "invalid-runtime-condition",
            format!(
                "runtimeWhen {when:?} holds {found:?}; on this target it is a bare Jinja \
                 expression such as grains['os_family'] == 'Debian', written without tag delimiters"
            ),
        ));
    }
    Ok(())
}
