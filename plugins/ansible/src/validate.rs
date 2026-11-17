//! Stage three: the constraints that are Ansible's and not Meridian's.

use std::collections::{HashMap, HashSet};

use meridian_sdk::CompileError;

use crate::{Playbook, Task, NAME};

fn fail(task: &Task, rule: &str, msg: String) -> CompileError {
    CompileError {
        target: NAME.to_owned(),
        resource: task.resource_id.clone(),
        rule: rule.to_owned(),
        msg,
        pos: task.pos.clone(),
    }
}

/// Checks a play before anything is serialised. Every failure names the
/// offending resource and the rule it broke, so the message points at the
/// line the author must change.
pub(crate) fn validate(playbook: &Playbook) -> Result<(), CompileError> {
    if playbook.hosts.is_empty() {
        return Err(CompileError {
            target: NAME.to_owned(),
            rule: "missing-host".to_owned(),
            msg: "play has no target host".to_owned(),
            ..CompileError::default()
        });
    }

    // Handlers are notified by task name, so a duplicate name makes the
    // playbook ambiguous and not merely untidy.
    let mut names: HashMap<&str, &str> = HashMap::new();
    for task in &playbook.tasks {
        check_name(task, "task")?;
        if let Some(previous) = names.insert(&task.name, &task.resource_id) {
            return Err(fail(
                task,
                "duplicate-task-name",
                format!(
                    "task name {:?} is already used by resource {:?}",
                    task.name, previous
                ),
            ));
        }
    }

    let mut handler_names = HashSet::new();
    for handler in &playbook.handlers {
        check_name(handler, "handler")?;
        if !handler_names.insert(handler.name.as_str()) {
            return Err(fail(
                handler,
                "duplicate-handler-name",
                format!("handler name {:?} is declared twice", handler.name),
            ));
        }
        if !handler.when.is_empty() {
            return Err(fail(
                handler,
                "conditional-handler",
                format!(
                    "handler {:?} carries a condition; a handler runs because \
                     something changed, not because a condition holds",
                    handler.name
                ),
            ));
        }
    }

    for task in &playbook.tasks {
        for notified in &task.notify {
            if !handler_names.contains(notified.as_str()) {
                return Err(fail(
                    task,
                    "missing-handler",
                    format!("notifies handler {notified:?}, which no handler block defines"),
                ));
            }
        }
        if task.module.is_empty() {
            return Err(fail(
                task,
                "missing-module",
                "task has no module".to_owned(),
            ));
        }
        if task.args.is_empty() {
            return Err(fail(
                task,
                "empty-module-args",
                format!("module {} was given no arguments", task.module),
            ));
        }
    }
    Ok(())
}

fn check_name(task: &Task, what: &str) -> Result<(), CompileError> {
    if task.name.trim().is_empty() {
        return Err(fail(
            task,
            "empty-name",
            format!("{what} has an empty name"),
        ));
    }
    if task.name.contains(['\n', '\r']) {
        return Err(fail(
            task,
            "multiline-name",
            format!("{what} name {:?} spans more than one line", task.name),
        ));
    }
    Ok(())
}
