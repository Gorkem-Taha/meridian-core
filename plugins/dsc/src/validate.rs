//! Stage three: the constraints that are the runner's and not Meridian's.

use meridian_sdk::CompileError;

use crate::{refuse_at, Configuration, NAME};

pub(crate) fn validate(configuration: &Configuration) -> Result<(), CompileError> {
    let whole = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };
    if configuration.instances.is_empty() {
        return Err(whole(
            "empty-configuration",
            "there is nothing to configure: the document declares no DSC resources".to_owned(),
        ));
    }
    // The node's name is the compiled file's name.
    let node = &configuration.node;
    let safe = !node.is_empty()
        && !node.starts_with('.')
        && node
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        && !node.contains("..");
    if !safe {
        return Err(whole(
            "invalid-name",
            format!(
                "document name {node:?} cannot name a DscPipelineRunner node: it becomes a file \
                 name, so it may hold only letters, digits, hyphens, underscores and periods"
            ),
        ));
    }
    if configuration.settings.target != "Local" && configuration.host.is_empty() {
        return Err(whole(
            "missing-param",
            format!(
                "the runner's target is {}, which needs a machine to reach; give the document a \
                 targetHost",
                configuration.settings.target
            ),
        ));
    }
    if configuration.settings.target == "Local" && !configuration.settings.credential.is_empty() {
        return Err(whole(
            "invalid-param",
            "the runner has a credential but its target is Local, which connects to nothing"
                .to_owned(),
        ));
    }
    for instance in &configuration.instances {
        // The runner reads `Module/Resource/name`, so a slash in the name
        // would name a different resource.
        if instance.id.contains('/') {
            return Err(refuse_at(
                &instance.id,
                &instance.pos,
                "invalid-name",
                "a resource id cannot hold a slash on this target".to_owned(),
            ));
        }
        if instance.pre_condition.contains('\n') {
            return Err(refuse_at(
                &instance.id,
                &instance.pos,
                "invalid-runtime-condition",
                "runtimeWhen must be one line of PowerShell on this target".to_owned(),
            ));
        }
    }
    Ok(())
}
