//! The keys Quadlet documents for each kind of unit, as of Podman 5.
//!
//! The table is here to catch a typing mistake while the document is still in
//! front of its author. Quadlet refuses a unit with a key it does not know,
//! and it does so inside systemd's generator, where the only trace is a line
//! in the journal and a service that does not exist.
//!
//! It is not the authority: Podman adds keys with every release. A key that
//! is only a different capitalisation of a known one is refused, because it
//! cannot be right. Any other unknown key is passed through with a warning.

use crate::Kind;

const CONTAINER: &[&str] = &[
    "AddCapability",
    "AddDevice",
    "AddHost",
    "Annotation",
    "AutoUpdate",
    "CgroupsMode",
    "ContainerName",
    "ContainersConfModule",
    "DNS",
    "DNSOption",
    "DNSSearch",
    "DropCapability",
    "Entrypoint",
    "Environment",
    "EnvironmentFile",
    "EnvironmentHost",
    "Exec",
    "ExposeHostPort",
    "GIDMap",
    "GlobalArgs",
    "Group",
    "GroupAdd",
    "HealthCmd",
    "HealthInterval",
    "HealthLogDestination",
    "HealthMaxLogCount",
    "HealthMaxLogSize",
    "HealthOnFailure",
    "HealthRetries",
    "HealthStartPeriod",
    "HealthStartupCmd",
    "HealthStartupInterval",
    "HealthStartupRetries",
    "HealthStartupSuccess",
    "HealthStartupTimeout",
    "HealthTimeout",
    "HostName",
    "IP",
    "IP6",
    "Image",
    "Label",
    "LogDriver",
    "LogOpt",
    "Mask",
    "Memory",
    "Mount",
    "Network",
    "NetworkAlias",
    "NoNewPrivileges",
    "Notify",
    "PidsLimit",
    "Pod",
    "PodmanArgs",
    "PublishPort",
    "Pull",
    "ReadOnly",
    "ReadOnlyTmpfs",
    "ReloadCmd",
    "ReloadSignal",
    "RemapGid",
    "RemapUid",
    "RemapUidSize",
    "RemapUsers",
    "Retry",
    "RetryDelay",
    "Rootfs",
    "RunInit",
    "SeccompProfile",
    "Secret",
    "SecurityLabelDisable",
    "SecurityLabelFileType",
    "SecurityLabelLevel",
    "SecurityLabelNested",
    "SecurityLabelType",
    "ServiceName",
    "ShmSize",
    "StartWithPod",
    "StopSignal",
    "StopTimeout",
    "SubGIDMap",
    "SubUIDMap",
    "Sysctl",
    "Timezone",
    "Tmpfs",
    "UIDMap",
    "Ulimit",
    "Unmask",
    "User",
    "UserNS",
    "VolatileTmp",
    "Volume",
    "WorkingDir",
];

const POD: &[&str] = &[
    "AddHost",
    "ContainersConfModule",
    "DNS",
    "DNSOption",
    "DNSSearch",
    "GIDMap",
    "GlobalArgs",
    "HostName",
    "IP",
    "IP6",
    "Network",
    "NetworkAlias",
    "PodName",
    "PodmanArgs",
    "PublishPort",
    "ServiceName",
    "ShmSize",
    "SubGIDMap",
    "SubUIDMap",
    "UIDMap",
    "UserNS",
    "Volume",
];

const KUBE: &[&str] = &[
    "AutoUpdate",
    "ConfigMap",
    "ContainersConfModule",
    "ExitCodePropagation",
    "GlobalArgs",
    "KubeDownForce",
    "LogDriver",
    "Network",
    "PodmanArgs",
    "PublishPort",
    "RemapGid",
    "RemapUid",
    "RemapUidSize",
    "RemapUsers",
    "ServiceName",
    "SetWorkingDirectory",
    "UserNS",
    "Yaml",
];

const VOLUME: &[&str] = &[
    "ContainersConfModule",
    "Copy",
    "Device",
    "Driver",
    "GlobalArgs",
    "Group",
    "Image",
    "Label",
    "Options",
    "PodmanArgs",
    "ServiceName",
    "Type",
    "User",
    "VolumeName",
];

const NETWORK: &[&str] = &[
    "ContainersConfModule",
    "DNS",
    "DisableDNS",
    "Driver",
    "Gateway",
    "GlobalArgs",
    "IPAMDriver",
    "IPRange",
    "IPv6",
    "Internal",
    "Label",
    "NetworkDeleteOnStop",
    "NetworkName",
    "Options",
    "PodmanArgs",
    "ServiceName",
    "Subnet",
];

const IMAGE: &[&str] = &[
    "AllTags",
    "Arch",
    "AuthFile",
    "CertDir",
    "ContainersConfModule",
    "Creds",
    "DecryptionKey",
    "GlobalArgs",
    "Image",
    "ImageTag",
    "OS",
    "PodmanArgs",
    "Retry",
    "RetryDelay",
    "ServiceName",
    "TLSVerify",
    "Variant",
];

const BUILD: &[&str] = &[
    "Annotation",
    "Arch",
    "AuthFile",
    "ContainersConfModule",
    "DNS",
    "DNSOption",
    "DNSSearch",
    "Environment",
    "File",
    "ForceRM",
    "GlobalArgs",
    "GroupAdd",
    "ImageTag",
    "Label",
    "Network",
    "PodmanArgs",
    "Pull",
    "Retry",
    "RetryDelay",
    "Secret",
    "ServiceName",
    "SetWorkingDirectory",
    "TLSVerify",
    "Target",
    "Variant",
    "Volume",
];

/// The documented keys of one kind of unit.
pub(crate) fn known(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Container => CONTAINER,
        Kind::Pod => POD,
        Kind::Kube => KUBE,
        Kind::Volume => VOLUME,
        Kind::Network => NETWORK,
        Kind::Image => IMAGE,
        Kind::Build => BUILD,
    }
}

/// What a key was probably meant to be.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Lookup {
    /// A documented key, spelled as documented.
    Known,
    /// A documented key in another capitalisation.
    Miscapitalised(&'static str),
    /// Not in the table.
    Unknown,
}

pub(crate) fn lookup(kind: Kind, key: &str) -> Lookup {
    let keys = known(kind);
    if keys.contains(&key) {
        return Lookup::Known;
    }
    match keys.iter().find(|k| k.eq_ignore_ascii_case(key)) {
        Some(k) => Lookup::Miscapitalised(k),
        None => Lookup::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_is_known_miscapitalised_or_unknown() {
        assert_eq!(lookup(Kind::Container, "Image"), Lookup::Known);
        assert_eq!(
            lookup(Kind::Container, "image"),
            Lookup::Miscapitalised("Image")
        );
        assert_eq!(
            lookup(Kind::Container, "publishport"),
            Lookup::Miscapitalised("PublishPort")
        );
        assert_eq!(lookup(Kind::Container, "Imgae"), Lookup::Unknown);
        // A key belongs to its kind of unit and no other.
        assert_eq!(lookup(Kind::Volume, "PublishPort"), Lookup::Unknown);
    }

    /// The user guide lists every key, so it cannot fall behind this table.
    #[test]
    fn the_guide_lists_exactly_these_keys() {
        let guide = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../docs/targets/podman.md"
        ))
        .expect("the guide exists");
        let section = guide
            .split("### Known keys")
            .nth(1)
            .expect("the guide has a Known keys section");
        for kind in Kind::ALL {
            let prefix = format!("| `{}` | ", kind.as_str());
            let row = section
                .lines()
                .find(|line| line.starts_with(&prefix))
                .unwrap_or_else(|| panic!("no row for {}", kind.as_str()));
            let listed: Vec<&str> = row[prefix.len()..]
                .trim_end_matches(" |")
                .split(", ")
                .map(|key| key.trim_matches('`'))
                .collect();
            assert_eq!(listed, known(kind), "{}", kind.as_str());
        }
    }

    #[test]
    fn every_table_is_sorted_and_free_of_duplicates() {
        for kind in Kind::ALL {
            let keys = known(kind);
            let mut sorted = keys.to_vec();
            sorted.sort_unstable();
            sorted.dedup();
            assert_eq!(keys, sorted.as_slice(), "{}", kind.as_str());
        }
    }
}
