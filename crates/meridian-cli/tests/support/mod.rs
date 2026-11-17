//! What the conformance tests share: every shipped target as an emitter, and
//! the segments they belong to.

#![allow(dead_code)]

use meridian_sdk::{Category, CompileError, Data, Emitted, Emitter};

/// Every target the workspace ships, sorted by name.
pub fn emitters() -> Vec<Box<dyn Emitter>> {
    let mut all: Vec<Box<dyn Emitter>> = vec![
        Box::new(meridian_target_ansible::Ansible),
        Box::new(meridian_target_arm::Arm),
        Box::new(meridian_target_bicep::Bicep),
        Box::new(meridian_target_chef::Chef),
        Box::new(meridian_target_cloud_init::CloudInit),
        Box::new(meridian_target_cloudformation::CloudFormation),
        Box::new(meridian_target_docker::Docker),
        Box::new(meridian_target_dsc::Dsc),
        Box::new(meridian_target_helm::Helm),
        Box::new(meridian_target_kubernetes::Kubernetes),
        Box::new(meridian_target_opentofu::OpenTofu),
        Box::new(meridian_target_openvox::OpenVox),
        Box::new(meridian_target_packer::Packer),
        Box::new(meridian_target_podman::Podman),
        Box::new(meridian_target_pulumi::Pulumi),
        Box::new(meridian_target_puppet::Puppet),
        Box::new(meridian_target_salt::Salt),
        Box::new(meridian_target_terraform::Terraform),
    ];
    all.sort_by(|a, b| a.name().cmp(b.name()));
    all
}

/// One target, by name.
pub fn emitter(name: &str) -> Box<dyn Emitter> {
    emitters()
        .into_iter()
        .find(|e| e.name() == name)
        .unwrap_or_else(|| panic!("no target is called {name}"))
}

/// The targets that share the six configuration management resource types.
pub const SHARED_DIALECT: &[&str] = &["ansible", "chef", "cloud-init", "openvox", "puppet", "salt"];

/// The targets of one segment.
pub fn segment(category: Category) -> Vec<Box<dyn Emitter>> {
    emitters()
        .into_iter()
        .filter(|e| e.category() == category)
        .collect()
}

/// How a compile came out, as one cell of a table: `ok`, `ok, warns`, or the
/// rule that refused it.
pub fn outcome(result: &Result<Emitted, CompileError>) -> String {
    match result {
        Ok(emitted) if emitted.warnings.is_empty() => "ok".to_owned(),
        Ok(_) => "ok, warns".to_owned(),
        Err(refusal) => format!("`{}`", refusal.rule),
    }
}

/// No data: what a command is given when it is run without `--data`.
pub fn no_data() -> Data {
    Data::new()
}
