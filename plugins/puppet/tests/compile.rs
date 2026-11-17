//! The compile half, without a process boundary.
//!
//! The golden files under `testdata` were recorded by the Go implementation
//! this plugin replaces. Matching them byte for byte is the pass mark for the
//! port.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{
    assert_golden, assert_golden_refusal, fixture, graph_from_text, without_positions,
};
use meridian_sdk::{CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_puppet::Puppet;

const FIXTURES: &[&str] = &[
    "accounts",
    "restart_chain",
    "runtime_condition",
    "web_stack",
];
/// The fixtures Puppet cannot express, and says so.
const REFUSED: &[&str] = &["runtime_condition", "web_stack"];

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    Puppet.emit(graph, &Data::new())
}

fn compile_named(name: &str, resources: &str) -> Result<Emitted, CompileError> {
    let text = format!(
        "apiVersion: meridian/v1\nkind: ResourceSet\nmetadata:\n  name: {name}\nspec:\n  target: puppet\n  targetHost:\n    from: inline\n    host: web01.example.com\n  resources:\n{resources}"
    );
    emit(&graph_from_text("site.yaml", &text, "puppet"))
}

fn compile(resources: &str) -> Result<Emitted, CompileError> {
    compile_named("web", resources)
}

fn refusal(resources: &str) -> CompileError {
    compile(resources).expect_err("the document should be refused")
}

fn manifest(resources: &str) -> String {
    compile(resources).unwrap().artifact.files["manifests/init.pp"].clone()
}

#[test]
fn every_fixture_compiles_or_is_refused_as_the_go_plugin_recorded() {
    for name in FIXTURES {
        let graph = without_positions(fixture(name, "puppet"));
        match emit(&graph) {
            Ok(emitted) => {
                assert!(!REFUSED.contains(name), "{name} should be refused");
                assert_golden(&golden(), name, &emitted);
            }
            Err(refusal) => {
                assert!(REFUSED.contains(name), "{name}: {refusal}");
                assert_golden_refusal(&golden(), name, &refusal);
            }
        }
    }
}

#[test]
fn a_refusal_of_a_notify_points_at_the_notifier() {
    let graph = fixture("web_stack", "puppet");
    let err = emit(&graph).unwrap_err();
    assert_eq!(err.resource, "nginx_site");
    assert_eq!(err.rule, "unsupported-action");
    assert_eq!(err.pos.file, "fixtures/web_stack.yaml");
    assert!(err.pos.line >= graph.resource("nginx_site").unwrap().pos.line);
}

#[test]
fn a_type_puppet_cannot_refresh_cannot_be_notified() {
    let err = refusal(
        "    - id: conf\n      type: file\n      params: {path: /a, content: a}\n      notifies:\n        - resource: pkg\n          action: restart\n    - id: pkg\n      type: package\n      params: {name: app}\n",
    );
    assert_eq!(err.rule, "unsupported-action");
    assert_eq!(
        err.msg,
        "notifies resource \"pkg\", which this target cannot refresh: Puppet refreshes \
         only the types that define a refresh, and package is not one of them"
    );
}

#[test]
fn an_action_outside_the_table_names_the_ones_supported() {
    let err = refusal(
        "    - id: conf\n      type: file\n      params: {path: /a, content: a}\n      notifies:\n        - resource: svc\n          action: bounce\n    - id: svc\n      type: service\n      params: {name: app}\n",
    );
    assert_eq!(
        err.msg,
        "notifies resource \"svc\" with action \"bounce\", which resource type \"service\" \
         does not support on this target (supported: refresh, restart)"
    );
}

#[test]
fn a_service_cannot_be_left_restarted() {
    let err = refusal(
        "    - id: svc\n      type: service\n      state: restarted\n      params: {name: app}\n",
    );
    assert_eq!(err.rule, "unsupported-state");
    assert_eq!(
        err.msg,
        "resource type \"service\" does not accept state \"restarted\" on this target \
         (accepted: running, started, stopped)"
    );
}

#[test]
fn two_resources_managing_one_thing_are_refused_here_not_on_the_node() {
    let err = refusal(
        "    - id: a\n      type: package\n      params: {name: nginx}\n    - id: b\n      type: package\n      state: latest\n      params: {name: nginx}\n",
    );
    assert_eq!(err.rule, "duplicate-namevar");
    assert_eq!(err.resource, "b");
    assert!(err
        .msg
        .starts_with("this package manages \"nginx\", which resource \"a\" already manages"));
}

#[test]
fn a_document_name_that_cannot_be_a_class_is_refused() {
    let err = compile_named(
        "web-server",
        "    - id: a\n      type: package\n      params: {name: nginx}\n",
    )
    .unwrap_err();
    assert_eq!(err.rule, "invalid-class-name");
    assert_eq!(
        err.msg,
        "document name \"web-server\" cannot become a Puppet class name: \"-\" is not allowed, \
         and a class name may hold only lower-case letters, digits and underscores"
    );
}

#[test]
fn a_notify_already_orders_so_no_require_repeats_it() {
    let text = manifest(
        "    - id: conf\n      type: file\n      params: {path: /a, content: a}\n      notifies:\n        - resource: svc\n          action: restart\n    - id: svc\n      type: service\n      params: {name: app}\n      dependsOn:\n        - resource: conf\n",
    );
    assert!(text.contains("notify  => Service['svc'],"), "{text}");
    assert!(!text.contains("require"), "{text}");
}

#[test]
fn strings_are_quoted_so_puppet_reads_what_was_written() {
    let text = manifest(
        "    - id: conf\n      type: file\n      params:\n        path: /etc/motd\n        content: \"it's $HOME\\n\"\n        owner: $user\n    - id: shell\n      type: exec\n      params:\n        command: echo hi | tee /tmp/x\n        shell: true\n        chdir: /tmp\n",
    );
    assert!(text.contains("content => \"it's \\$HOME\\n\","), "{text}");
    assert!(text.contains("owner   => '$user',"), "{text}");
    assert!(text.contains("provider => shell,"), "{text}");
    assert!(text.contains("cwd      => '/tmp',"), "{text}");
}

#[test]
fn numbers_lists_and_maps_are_written_as_puppet_source() {
    let text = manifest(
        "    - id: u\n      type: user\n      params:\n        name: app\n        uid: 1200.0\n        system: true\n        groups: [a, b]\n",
    );
    assert!(text.contains("uid    => 1200,"), "{text}");
    assert!(text.contains("system => true,"), "{text}");
    assert!(text.contains("groups => ['a', 'b'],"), "{text}");
}

#[test]
fn a_title_with_a_quote_is_refused() {
    let mut graph = fixture("accounts", "puppet");
    graph.resources[0].id = "it's".to_owned();
    graph.edges.clear();
    assert_eq!(emit(&graph).unwrap_err().rule, "quoted-title");
}
