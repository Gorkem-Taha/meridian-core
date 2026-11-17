mod common;

use common::*;
use meridian_ir::{parse, validate, Bundle};

const HEAD: &str =
    "apiVersion: meridian/v1\nkind: ResourceSet\nmetadata: {name: a}\nspec:\n  target: ansible\n";

#[test]
fn accepts_good_documents() {
    for doc in [VALID_INFRASTRUCTURE, VALID_RESOURCE_SET] {
        validate(&parse_one(doc)).unwrap();
    }
}

#[test]
fn rejects_a_duplicate_resource_id() {
    let doc = format!("{HEAD}  resources:\n    - id: nginx\n      type: package\n    - id: nginx\n      type: service\n");
    let err = validate(&parse_one(&doc)).unwrap_err().to_string();
    assert!(err.contains(r#"duplicate resource id "nginx""#), "{err}");
    // The message points at the first declaration too, so the author can find
    // both halves of the collision.
    assert!(err.contains("first declared at doc.yaml:7:7"), "{err}");
}

#[test]
fn rejects_dangling_edges() {
    let doc = format!(
        "{HEAD}  resources:\n    - id: nginx_conf\n      type: file\n      dependsOn:\n        - resource: absent_pkg\n      notifies:\n        - resource: absent_svc\n          action: restart\n"
    );
    let err = validate(&parse_one(&doc)).unwrap_err().to_string();
    for want in [
        r#"depends on unknown resource "absent_pkg""#,
        r#"notifies unknown resource "absent_svc""#,
    ] {
        assert!(
            err.contains(want),
            "error = {err}, want it to contain {want:?}"
        );
    }
}

#[test]
fn rejects_self_reference() {
    let doc = format!("{HEAD}  resources:\n    - id: nginx\n      type: service\n      dependsOn:\n        - resource: nginx\n");
    let err = validate(&parse_one(&doc)).unwrap_err().to_string();
    assert!(err.contains("cannot depend on itself"), "{err}");
}

#[test]
fn rejects_missing_required_fields() {
    let v = "apiVersion: meridian/v1\nkind: ResourceSet\n";
    for (name, doc, want) in [
        (
            "no metadata.name",
            format!("{v}spec:\n  target: ansible\n  resources:\n    - {{id: a, type: package}}\n"),
            "metadata.name: missing required field",
        ),
        (
            "no spec.target",
            format!("{v}metadata: {{name: a}}\nspec:\n  resources:\n    - {{id: a, type: package}}\n"),
            "spec.target: missing required field",
        ),
        ("no resources", format!("{HEAD}  resources: []\n"), "at least one resource is required"),
        (
            "resource without id",
            format!("{HEAD}  resources:\n    - {{type: package}}\n"),
            "spec.resources[0].id: missing required field",
        ),
        (
            "resource without type",
            format!("{HEAD}  resources:\n    - {{id: a}}\n"),
            "spec.resources[0].type: missing required field",
        ),
        (
            "unknown ifMissing policy",
            format!(
                "{HEAD}  resources:\n    - {{id: a, type: package}}\n    - {{id: b, type: service, dependsOn: [{{resource: a, ifMissing: shrug}}]}}\n"
            ),
            r#"unknown policy "shrug" (expected "error" or "skip")"#,
        ),
        (
            "notify without action",
            format!(
                "{HEAD}  resources:\n    - {{id: a, type: package}}\n    - {{id: b, type: service, notifies: [{{resource: a}}]}}\n"
            ),
            "notifies[0].action: missing required field",
        ),
    ] {
        let err = validate(&parse_one(&doc)).expect_err(name).to_string();
        assert!(err.contains(want), "{name}: error = {err}, want it to contain {want:?}");
    }
}

#[test]
fn reports_every_problem_at_once() {
    let doc = format!(
        "{HEAD}  resources:\n    - id: a\n      type: package\n    - id: a\n      type: service\n      dependsOn:\n        - resource: ghost\n"
    );
    let err = validate(&parse_one(&doc)).unwrap_err();
    assert_eq!(err.len(), 2, "{err}");
    assert!(
        err.to_string().starts_with("2 errors:\n  doc.yaml:9:7: "),
        "{err}"
    );
}

#[test]
fn a_bundle_resolves_target_host_across_documents() {
    let bundle = Bundle::new(parse("bundle.yaml", bundle_text().as_bytes()).unwrap());
    bundle.validate().unwrap();
    let rs = bundle
        .resource_set("web-server-baseline")
        .expect("the ResourceSet");
    let (infra, out) = bundle.resolve_target_host(rs).unwrap();
    assert_eq!(infra.metadata.name, "web-vm");
    assert_eq!(
        (out.name.as_str(), out.value.as_str()),
        ("web_vm_ip", "${web_vm.public_ip}")
    );
}

#[test]
fn a_bundle_rejects_an_unresolvable_target_host() {
    for (name, from, to, want) in [
        (
            "unknown ref",
            "ref: web-vm",
            "ref: nope",
            "no Infrastructure document named",
        ),
        (
            "unknown output",
            "output: web_vm_ip",
            "output: nope",
            "declares no output",
        ),
    ] {
        let text = format!(
            "{VALID_INFRASTRUCTURE}---\n{}",
            VALID_RESOURCE_SET.replacen(from, to, 1)
        );
        let bundle = Bundle::new(parse("bundle.yaml", text.as_bytes()).unwrap());
        let err = bundle.validate().expect_err(name).to_string();
        assert!(
            err.contains(want),
            "{name}: error = {err}, want it to contain {want:?}"
        );
    }
}

#[test]
fn a_bundle_validates_target_host_from_inline() {
    let doc = format!(
        "{HEAD}  targetHost:\n    from: inline\n  resources:\n    - {{id: a, type: package}}\n"
    );
    let err = Bundle::new([parse_one(&doc)])
        .validate()
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("targetHost.host: missing required field"),
        "{err}"
    );
}

#[test]
fn a_bundle_rejects_duplicate_document_names() {
    let text = format!("{VALID_RESOURCE_SET}---\n{VALID_RESOURCE_SET}");
    let bundle = Bundle::new(parse("bundle.yaml", text.as_bytes()).unwrap());
    let err = bundle.validate().unwrap_err().to_string();
    assert!(err.contains("duplicate ResourceSet document name"), "{err}");
    assert!(err.contains("first declared at bundle.yaml:1:1"), "{err}");
}
