//! Agreeing on a protocol version, and checking an SDK release.
//!
//! The host and the plugin each speak a range of protocol versions. The host
//! offers its range in [`PROTOCOLS_ENV`]; the plugin answers, in its
//! handshake line, with the highest version both have. Neither side needs the
//! other to have been upgraded first: a plugin that predates the variable
//! announces its one version, and a host that predates it accepts or refuses
//! that version as it always did.

use std::ops::RangeInclusive;

use crate::{MIN_PROTOCOL_VERSION, PROTOCOL_VERSION};

/// The environment variable a host sets on a plugin it starts: the protocol
/// versions the host speaks, lowest first, separated by commas.
pub const PROTOCOLS_ENV: &str = "MERIDIAN_PLUGIN_PROTOCOLS";

/// The protocol versions this SDK speaks, as a host and as a plugin.
pub fn supported_protocols() -> RangeInclusive<u32> {
    MIN_PROTOCOL_VERSION..=PROTOCOL_VERSION
}

/// A range as a person reads it: `2`, or `2 to 3`.
pub fn describe_protocols(range: &RangeInclusive<u32>) -> String {
    if range.start() == range.end() {
        range.start().to_string()
    } else {
        format!("{} to {}", range.start(), range.end())
    }
}

/// What a host puts in [`PROTOCOLS_ENV`] for the range it speaks.
pub fn offer(range: &RangeInclusive<u32>) -> String {
    range
        .clone()
        .map(|v| v.to_string())
        .collect::<Vec<_>>()
        .join(",")
}

/// The version a plugin announces, given what the host offered.
///
/// It is the highest version both sides speak. With nothing offered, or
/// nothing in common, it is the plugin's own newest: the host then refuses
/// the plugin by version, which is a clearer failure than a plugin that will
/// not start. Entries that are not numbers are ignored, so a future host can
/// extend the list's syntax without breaking this SDK.
pub fn choose(offered: Option<&str>, mine: &RangeInclusive<u32>) -> u32 {
    offered
        .into_iter()
        .flat_map(|list| list.split(','))
        .filter_map(|entry| entry.trim().parse::<u32>().ok())
        .filter(|v| mine.contains(v))
        .max()
        .unwrap_or(*mine.end())
}

/// Checks, on the host, the version a plugin announced. The error says which
/// side is behind, because the fix differs.
pub fn accept(announced: u32, mine: &RangeInclusive<u32>) -> Result<(), String> {
    if mine.contains(&announced) {
        return Ok(());
    }
    let advice = if announced < *mine.start() {
        "the plugin is older than this host supports: rebuild it against a current SDK"
    } else {
        "the plugin is newer than this host: upgrade meridian"
    };
    Err(format!(
        "the plugin speaks protocol version {announced} and this host speaks {}; {advice}",
        describe_protocols(mine)
    ))
}

fn parse_release(text: &str) -> Option<(u64, u64, u64)> {
    // A pre-release or build suffix does not change which SDK it is.
    let core = text.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let release = (next()?, next()?, next()?);
    parts.next().is_none().then_some(release)
}

/// Whether a plugin built against SDK `plugin` loads on a host shipping SDK
/// `host`.
///
/// The promise is SemVer's: a plugin loads on a host whose SDK is the same
/// major and not older than the plugin's. Before `1.0` the minor version is
/// the breaking position. A plugin built against a newer SDK than the host
/// ships may call something the host does not have, so it is refused by name
/// here instead of failing at an arbitrary call.
pub fn sdk_compatible(plugin: &str, host: &str) -> Result<(), String> {
    let (Some(p), Some(h)) = (parse_release(plugin), parse_release(host)) else {
        return Err(format!(
            "the plugin reports SDK version {plugin:?}, which is not a release number"
        ));
    };
    let line = |(major, minor, _): (u64, u64, u64)| {
        if major == 0 {
            format!("0.{minor}.x")
        } else {
            format!("{major}.x")
        }
    };
    let same_line = p.0 == h.0 && (p.0 > 0 || p.1 == h.1);
    if !same_line {
        let which = if p > h { "newer" } else { "older" };
        let fix = if p > h {
            "upgrade meridian"
        } else {
            "rebuild the plugin against a current SDK"
        };
        return Err(format!(
            "the plugin was built against SDK {plugin} ({}), and this host ships SDK {host} \
             ({}); the plugin's SDK is {which} than this host supports: {fix}",
            line(p),
            line(h)
        ));
    }
    if p > h {
        return Err(format!(
            "the plugin was built against SDK {plugin}, and this host ships the older SDK \
             {host}; upgrade meridian"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plugin_takes_the_highest_version_both_speak() {
        assert_eq!(choose(Some("2,3"), &(2..=3)), 3);
        assert_eq!(choose(Some("2,3"), &(1..=2)), 2);
        assert_eq!(choose(Some("3,4,5"), &(2..=4)), 4);
        assert_eq!(choose(Some(" 2 , 3 "), &(3..=3)), 3);
    }

    #[test]
    fn with_nothing_offered_or_nothing_shared_the_plugin_announces_its_newest() {
        assert_eq!(choose(None, &(2..=3)), 3);
        assert_eq!(choose(Some(""), &(2..=3)), 3);
        assert_eq!(choose(Some("5,6"), &(2..=3)), 3);
        assert_eq!(choose(Some("two,;"), &(2..=3)), 3);
    }

    #[test]
    fn the_offer_lists_every_version_lowest_first() {
        assert_eq!(offer(&(2..=2)), "2");
        assert_eq!(offer(&(2..=4)), "2,3,4");
        assert_eq!(choose(Some(&offer(&(2..=4))), &(1..=3)), 3);
    }

    #[test]
    fn a_refusal_says_which_side_is_behind() {
        assert_eq!(accept(2, &(2..=3)), Ok(()));
        assert_eq!(accept(3, &(2..=3)), Ok(()));
        assert_eq!(
            accept(1, &(2..=3)).unwrap_err(),
            "the plugin speaks protocol version 1 and this host speaks 2 to 3; the plugin is \
             older than this host supports: rebuild it against a current SDK"
        );
        assert_eq!(
            accept(5, &(2..=2)).unwrap_err(),
            "the plugin speaks protocol version 5 and this host speaks 2; the plugin is newer \
             than this host: upgrade meridian"
        );
    }

    #[test]
    fn this_sdk_speaks_a_range_that_holds_its_newest_version() {
        let range = supported_protocols();
        assert!(range.contains(&PROTOCOL_VERSION));
        assert!(range.start() <= range.end());
        // A host carries at most two adjacent protocol versions.
        assert!(range.end() - range.start() <= 1);
    }

    #[test]
    fn a_plugin_loads_on_a_host_whose_sdk_is_the_same_line_and_not_older() {
        for (plugin, host) in [
            ("0.1.0", "0.1.0"),
            ("0.1.0", "0.1.4"),
            ("1.2.0", "1.2.0"),
            ("1.2.5", "1.4.0"),
            ("1.0.0-beta.1", "1.0.0"),
        ] {
            assert_eq!(sdk_compatible(plugin, host), Ok(()), "{plugin} on {host}");
        }
    }

    #[test]
    fn a_plugin_from_another_sdk_line_or_a_newer_sdk_is_refused_by_name() {
        assert_eq!(
            sdk_compatible("1.4.0", "2.3.0").unwrap_err(),
            "the plugin was built against SDK 1.4.0 (1.x), and this host ships SDK 2.3.0 (2.x); \
             the plugin's SDK is older than this host supports: rebuild the plugin against a \
             current SDK"
        );
        assert_eq!(
            sdk_compatible("0.2.0", "0.1.0").unwrap_err(),
            "the plugin was built against SDK 0.2.0 (0.2.x), and this host ships SDK 0.1.0 \
             (0.1.x); the plugin's SDK is newer than this host supports: upgrade meridian"
        );
        assert_eq!(
            sdk_compatible("1.5.0", "1.2.0").unwrap_err(),
            "the plugin was built against SDK 1.5.0, and this host ships the older SDK 1.2.0; \
             upgrade meridian"
        );
        assert_eq!(
            sdk_compatible("latest", "1.2.0").unwrap_err(),
            "the plugin reports SDK version \"latest\", which is not a release number"
        );
    }
}
