//! Stage three: the constraints that are Helm's and Kubernetes' and not
//! Meridian's.

use std::collections::BTreeMap;

use meridian_sdk::CompileError;

use crate::{refuse_at, Chart, NAME};

/// A DNS label: lowercase letters, digits and hyphens, starting and ending
/// with a letter or a digit.
fn is_label(name: &str, max: usize) -> bool {
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty()
        && name.len() <= max
        && name.starts_with(edge)
        && name.ends_with(edge)
        && name.chars().all(|c| edge(c) || c == '-')
}

/// The widest set of names any kind accepts: a DNS subdomain, with the colon
/// that RBAC names such as `system:reader` use.
fn is_object_name(name: &str) -> bool {
    let edge = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit();
    !name.is_empty()
        && name.len() <= 253
        && name.starts_with(edge)
        && name.ends_with(edge)
        && name
            .chars()
            .all(|c| edge(c) || matches!(c, '-' | '.' | ':'))
}

/// `1.2.3`, with an optional `-prerelease` and `+build`.
fn is_semver(version: &str) -> bool {
    let (version, build) = version.split_once('+').unwrap_or((version, "x"));
    let (core, pre) = version.split_once('-').unwrap_or((version, "x"));
    let word = |s: &str| {
        !s.is_empty()
            && s.chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
    };
    let numbers: Vec<&str> = core.split('.').collect();
    numbers.len() == 3
        && numbers
            .iter()
            .all(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
        && word(pre)
        && word(build)
}

/// A Go duration made of hours, minutes and seconds: `90s`, `5m`, `1h30m`.
fn is_duration(text: &str) -> bool {
    let mut digits = 0;
    let mut units = 0;
    for c in text.chars() {
        match c {
            '0'..='9' => digits += 1,
            'h' | 'm' | 's' if digits > 0 => {
                digits = 0;
                units += 1;
            }
            _ => return false,
        }
    }
    units > 0 && digits == 0
}

pub(crate) fn validate(chart: &Chart) -> Result<(), CompileError> {
    let document = |rule: &str, msg: String| CompileError {
        target: NAME.to_owned(),
        rule: rule.to_owned(),
        msg,
        ..CompileError::default()
    };

    let release = &chart.release;
    if !is_label(&release.name, 53) {
        return Err(document(
            "invalid-release-name",
            format!(
                "{:?} cannot be a release name: one holds only lowercase letters, digits and \
                 hyphens, starts and ends with a letter or a digit, and is at most 53 \
                 characters. Name the release with a resource of type \"release\"",
                release.name
            ),
        ));
    }
    if !is_label(&release.namespace, 63) {
        return Err(document(
            "invalid-param",
            format!(
                "{:?} cannot be the release's namespace: one holds only lowercase letters, \
                 digits and hyphens, and is at most 63 characters",
                release.namespace
            ),
        ));
    }
    if !release.timeout.is_empty() && !is_duration(&release.timeout) {
        return Err(document(
            "invalid-param",
            format!(
                "{:?} is not a timeout Helm reads: write a duration such as 90s, 5m or 1h30m",
                release.timeout
            ),
        ));
    }
    if !is_label(&chart.chart.name, 63) {
        return Err(document(
            "invalid-chart-name",
            format!(
                "{:?} cannot be a chart name: one holds only lowercase letters, digits and \
                 hyphens. Name the chart with a resource of type \"chart\"",
                chart.chart.name
            ),
        ));
    }
    if !is_semver(&chart.chart.version) {
        return Err(document(
            "invalid-param",
            format!(
                "{:?} is not a version a chart can have: Helm requires a semantic version \
                 such as 1.4.0",
                chart.chart.version
            ),
        ));
    }
    if chart.manifests.is_empty() {
        return Err(document(
            "empty-chart",
            "a chart needs at least one Kubernetes object".to_owned(),
        ));
    }

    let mut seen: BTreeMap<(&str, &str, &str, &str), &str> = BTreeMap::new();
    for manifest in &chart.manifests {
        if !is_object_name(&manifest.name) {
            return Err(refuse_at(
                &manifest.id,
                &manifest.pos,
                "invalid-name",
                format!(
                    "{:?} cannot be the name of a Kubernetes object: one holds only lowercase \
                     letters, digits, hyphens and dots, starts and ends with a letter or a \
                     digit, and is at most 253 characters. Set metadata.name",
                    manifest.name
                ),
            ));
        }
        let key = (
            manifest.api_version.as_str(),
            manifest.kind.as_str(),
            manifest.namespace.as_str(),
            manifest.name.as_str(),
        );
        if let Some(previous) = seen.insert(key, &manifest.id) {
            return Err(refuse_at(
                &manifest.id,
                &manifest.pos,
                "duplicate-object",
                format!(
                    "resources {previous:?} and {:?} are both the {} named {:?}; a release \
                     holds an object once",
                    manifest.id, manifest.kind, manifest.name
                ),
            ));
        }
    }

    let mut outputs: BTreeMap<&str, ()> = BTreeMap::new();
    for output in &chart.outputs {
        if outputs.insert(&output.name, ()).is_some() {
            return Err(document(
                "duplicate-output",
                format!("two outputs are named {:?}", output.name),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_rules_of_what_they_name() {
        assert!(is_label("web-app", 53));
        assert!(!is_label("web_app", 53));
        assert!(!is_label("-web", 53));
        assert!(!is_label("Web", 53));
        assert!(!is_label(&"a".repeat(54), 53));
        assert!(is_object_name("web.example.com"));
        assert!(is_object_name("system:reader"));
        assert!(!is_object_name("web_config"));
        assert!(!is_object_name(""));
    }

    #[test]
    fn a_chart_version_is_a_semantic_version() {
        for good in ["0.1.0", "1.2.3-rc.1", "1.2.3+build.5", "1.2.3-beta+exp.sha"] {
            assert!(is_semver(good), "{good}");
        }
        for bad in ["1.0", "v1.0.0", "1.0.0-", "one", "1.0.0+", "1..0"] {
            assert!(!is_semver(bad), "{bad}");
        }
    }

    #[test]
    fn a_timeout_is_a_duration() {
        for good in ["90s", "5m", "1h30m", "1h2m3s"] {
            assert!(is_duration(good), "{good}");
        }
        for bad in ["", "5", "m", "5 m", "5d", "5m30"] {
            assert!(!is_duration(bad), "{bad}");
        }
    }
}
