//! `when` conditions, and what removing a resource does to its edges.

use meridian_ir::{parse, prune, Condition, Data, Document, Value};

fn data() -> Data {
    let Value::Map(map) = meridian_ir::parse_value(
        "data.yaml",
        b"env: prod\ncount: 3\nratio: 1.5\ninstall: true\nlegacy: false\nnothing: null\nregion:\n  name: eu-west-1\n  zones: 3\n",
    )
    .unwrap() else {
        panic!("the data is a mapping");
    };
    map
}

fn decide(condition: &str) -> Result<bool, String> {
    Condition::parse(condition)?.evaluate(&data())
}

#[test]
fn a_condition_reads_data_and_compares_it() {
    for (condition, want) in [
        ("true", true),
        ("false", false),
        ("data.install", true),
        ("!data.install", false),
        ("!!data.install", true),
        ("data.env == 'prod'", true),
        ("data.env == \"prod\"", true),
        ("data.env != 'prod'", false),
        ("data.count == 3", true),
        ("data.count == 3.0", true),
        ("data.ratio == 1.5", true),
        ("data.count != -3", true),
        ("data.region.name == 'eu-west-1'", true),
        ("data.region.zones == data.count", true),
        ("data.nothing == null", true),
        ("data.install && data.env == 'prod'", true),
        ("data.install && data.legacy", false),
        ("data.legacy || data.env == 'dev'", false),
        // && binds tighter than ||.
        ("data.legacy && data.legacy || data.install", true),
        ("data.legacy && (data.legacy || data.install)", false),
        ("defined(data.env)", true),
        ("defined(data.missing)", false),
        ("defined(data.region.name)", true),
        ("defined(data.region.missing)", false),
        // The right side is only read when the left leaves the answer open,
        // so a condition can guard its own lookups.
        ("!defined(data.missing) || data.missing", true),
        ("defined(data.missing) && data.missing == 'x'", false),
        ("defined(data.env) && data.env == 'prod'", true),
        ("  data.env=='prod'  ", true),
        ("data.env == 'it\\'s'", false),
    ] {
        assert_eq!(decide(condition), Ok(want), "{condition}");
    }
}

#[test]
fn a_condition_that_cannot_be_decided_is_an_error_and_never_a_guess() {
    for (condition, want) in [
        ("data.missing", "data.missing is not set"),
        ("data.env", "data.env is a string, not true or false"),
        ("data.count", "data.count is a number, not true or false"),
        ("!data.env", "not true or false"),
        ("data.env && true", "data.env is a string"),
        ("true && data.missing", "data.missing is not set"),
        ("false || data.env", "data.env is a string"),
        (
            "data.env.name == 'x'",
            "data.env is a string, so it has no key \"name\"",
        ),
        ("'text'", "not true or false"),
    ] {
        let err = decide(condition).expect_err(condition);
        assert!(err.contains(want), "{condition}: {err}");
    }
}

#[test]
fn a_condition_that_is_not_in_the_language_is_refused_when_it_is_read() {
    for (condition, want) in [
        ("", "the condition ends where a value was expected"),
        ("data", "data must be followed by a key"),
        ("data.", "a \".\" must be followed by a key"),
        ("data.a ==", "the condition ends where a value was expected"),
        (
            "data.a == == 1",
            "unexpected \"==\" where a value was expected",
        ),
        ("(data.a", "expected \")\", but the condition ends"),
        ("data.a data.b", "after the end of the condition"),
        ("data.a == 1 == 2", "after the end of the condition"),
        ("facts.os_family == 'Debian'", "facts are not available"),
        ("env == 'prod'", "unknown name \"env\""),
        ("defined(env)", "unknown name \"env\""),
        ("defined('x')", "defined(...) takes a data.<key> path"),
        ("data.a > 1", "unexpected \">\""),
        ("data.a = 1", "unexpected \"=\""),
        ("'open", "a quoted text is never closed"),
    ] {
        let err = Condition::parse(condition).expect_err(condition);
        assert!(err.contains(want), "{condition:?}: {err}");
    }
}

const DOCUMENT: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata: {name: web}
spec:
  target: ansible
  resources:
    - id: pkg
      type: package
      when: data.install
    - id: conf
      type: file
      dependsOn:
        - resource: pkg
          ifMissing: DEPENDS
      notifies:
        - resource: svc
          action: restart
          ifMissing: NOTIFIES
    - id: svc
      type: service
      when: "data.env == 'prod'"
      dependsOn:
        - resource: pkg
    - id: motd
      type: file
"#;

fn pruned(depends: &str, notifies: &str, data: &str) -> Result<Vec<meridian_ir::Resource>, String> {
    let text = DOCUMENT
        .replace("DEPENDS", depends)
        .replace("NOTIFIES", notifies);
    let docs = parse("site.yaml", text.as_bytes()).unwrap();
    let Value::Map(data) = meridian_ir::parse_value("data.yaml", data.as_bytes()).unwrap() else {
        panic!("the data is a mapping");
    };
    let Document::ResourceSet(doc) = &docs[0] else {
        panic!("a ResourceSet");
    };
    prune(&doc.spec.resources, &data).map_err(|e| e.to_string())
}

fn ids(resources: &[meridian_ir::Resource]) -> Vec<&str> {
    resources.iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn a_resource_whose_condition_holds_stays_and_becomes_unconditional() {
    let kept = pruned("error", "error", "{install: true, env: prod}").unwrap();
    assert_eq!(ids(&kept), ["pkg", "conf", "svc", "motd"]);
    assert!(kept.iter().all(|r| r.when.is_empty()));
    // Nothing was removed, so every edge is still there.
    assert_eq!(kept[1].depends_on.len(), 1);
    assert_eq!(kept[1].notifies.len(), 1);
}

#[test]
fn an_edge_to_a_removed_resource_is_refused_unless_it_says_skip() {
    // The default, and the explicit spelling of it.
    for policy in ["error", "\"\""] {
        let err = pruned("skip", policy, "{install: true, env: dev}").unwrap_err();
        assert_eq!(
            err,
            "site.yaml:16:11: spec.resources[1].notifies[0].resource: resource \"conf\" \
             notifies \"svc\", which its `when` removed from this run; set ifMissing: skip on \
             this entry if the resource should carry on without it"
        );
    }
    let err = pruned("error", "skip", "{install: false, env: dev}").unwrap_err();
    assert!(
        err.starts_with(
            "site.yaml:13:11: spec.resources[1].dependsOn[0].resource: resource \"conf\" \
             depends on \"pkg\", which its `when` removed"
        ),
        "{err}"
    );
}

#[test]
fn skip_drops_the_edge_and_keeps_the_resource() {
    let kept = pruned("skip", "skip", "{install: false, env: dev}").unwrap();
    assert_eq!(ids(&kept), ["conf", "motd"]);
    assert!(kept[0].depends_on.is_empty());
    assert!(kept[0].notifies.is_empty());

    // Only the edge to the removed resource goes; the other stays.
    let kept = pruned("skip", "skip", "{install: true, env: dev}").unwrap();
    assert_eq!(ids(&kept), ["pkg", "conf", "motd"]);
    assert_eq!(kept[1].depends_on[0].resource, "pkg");
    assert!(kept[1].notifies.is_empty());
}

#[test]
fn the_edges_of_a_removed_resource_go_with_it() {
    // svc depends on pkg with the default policy, but svc itself is removed,
    // so its edge to the removed pkg is nobody's problem.
    let kept = pruned("skip", "skip", "{install: false, env: dev}").unwrap();
    assert!(!ids(&kept).contains(&"svc"));
}

#[test]
fn a_condition_that_fails_names_the_resource_and_its_line() {
    let err = pruned("error", "error", "{env: prod}").unwrap_err();
    assert_eq!(
        err,
        "site.yaml:7:7: spec.resources[0].when: resource \"pkg\": data.install is not set \
         (give it with --data, or test for it with defined(data.install))"
    );
}
