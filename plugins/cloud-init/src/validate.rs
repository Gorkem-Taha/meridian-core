//! Stage three: the constraints that are cloud-init's and not Meridian's.

use std::collections::HashMap;

use meridian_sdk::CompileError;

use crate::{CloudConfig, NAME};

/// The values cloud-init accepts for a file's `encoding`.
pub(crate) const ENCODINGS: &[&str] = &[
    "b64",
    "base64",
    "gz",
    "gz+b64",
    "gz+base64",
    "gzip",
    "gzip+b64",
    "gzip+base64",
    "text/plain",
];

/// Checks a document before anything is serialised. Every failure names the
/// offending resource and the rule it broke, so the message points at the
/// line the author must change.
pub(crate) fn validate(config: &CloudConfig) -> Result<(), CompileError> {
    if config.host.is_empty() {
        return Err(CompileError {
            target: NAME.to_owned(),
            rule: "missing-host".to_owned(),
            msg: "the document has no target host".to_owned(),
            ..CompileError::default()
        });
    }

    // cloud-init writes the list in order, so a second entry for a path
    // silently replaces the first unless it appends to it.
    let mut paths: HashMap<&str, &str> = HashMap::new();
    for file in &config.files {
        let entry = &file.item;
        if let Some(previous) = paths.insert(&entry.path, &file.id) {
            if !entry.append {
                return Err(file.refuse(
                    "duplicate-file-path",
                    format!(
                        "path {:?} is already written by resource {:?}; \
                         set append: true if this one adds to it",
                        entry.path, previous
                    ),
                ));
            }
        }
        if !entry.permissions.is_empty() && !is_mode(&entry.permissions) {
            return Err(file.refuse(
                "invalid-mode",
                format!(
                    "mode {:?} is not an octal file mode such as \"0644\"",
                    entry.permissions
                ),
            ));
        }
        if !entry.encoding.is_empty() && !ENCODINGS.contains(&entry.encoding.as_str()) {
            return Err(file.refuse(
                "invalid-encoding",
                format!(
                    "encoding {:?} is not one cloud-init reads (accepted: {})",
                    entry.encoding,
                    ENCODINGS.join(", ")
                ),
            ));
        }
        // The two are written as one `owner:group` value.
        for (what, value) in [("owner", &entry.owner), ("group", &entry.group)] {
            if value.contains(':') {
                return Err(file.refuse(
                    "invalid-owner",
                    format!("{what} {value:?} holds a colon; give the owner and the group as separate params"),
                ));
            }
        }
    }

    let mut names: HashMap<&str, &str> = HashMap::new();
    for user in &config.users {
        check_name(user, "user", &user.item.name)?;
        if let Some(previous) = names.insert(&user.item.name, &user.id) {
            return Err(user.refuse(
                "duplicate-user",
                format!(
                    "user {:?} is already created by resource {:?}",
                    user.item.name, previous
                ),
            ));
        }
    }
    names.clear();
    for group in &config.groups {
        check_name(group, "group", &group.item.name)?;
        if let Some(previous) = names.insert(&group.item.name, &group.id) {
            return Err(group.refuse(
                "duplicate-group",
                format!(
                    "group {:?} is already created by resource {:?}",
                    group.item.name, previous
                ),
            ));
        }
    }
    names.clear();
    for package in &config.packages {
        // A colon is part of a package name on a multi-architecture system.
        if package.item.name.chars().any(char::is_whitespace) {
            return Err(package.refuse(
                "invalid-name",
                format!("package name {:?} holds a space", package.item.name),
            ));
        }
        if let Some(previous) = names.insert(&package.item.name, &package.id) {
            return Err(package.refuse(
                "duplicate-package",
                format!(
                    "package {:?} is already installed by resource {:?}",
                    package.item.name, previous
                ),
            ));
        }
    }
    Ok(())
}

/// Three or four octal digits, as cloud-init's `permissions` takes them.
fn is_mode(mode: &str) -> bool {
    (3..=4).contains(&mode.len()) && mode.bytes().all(|b| (b'0'..=b'7').contains(&b))
}

fn check_name<T>(placed: &crate::Placed<T>, what: &str, name: &str) -> Result<(), CompileError> {
    if name.chars().any(|c| c.is_whitespace() || c == ':') {
        return Err(placed.refuse(
            "invalid-name",
            format!("{what} name {name:?} holds a space or a colon"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mode_is_three_or_four_octal_digits() {
        for good in ["644", "0644", "1777", "000"] {
            assert!(is_mode(good), "{good}");
        }
        for bad in ["", "64", "06444", "0688", "rwxr-xr-x", "u+x", "0o644"] {
            assert!(!is_mode(bad), "{bad}");
        }
    }
}
