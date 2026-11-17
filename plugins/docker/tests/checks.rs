//! What the compile refuses, or warns about, before Docker ever sees the
//! file: misspelt attributes, things a service uses that the document does
//! not declare, dependencies that could never be met, and the project's
//! settings.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{
    assert_golden, assert_golden_refusal, graph_from_file, graph_from_text,
};
use meridian_sdk::{CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_docker::Docker;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    Docker.emit(graph, &Data::new())
}

fn compile_named(name: &str, resources: &[String]) -> Result<Emitted, CompileError> {
    let text = format!(
        "apiVersion: meridian/v1\nkind: Infrastructure\nmetadata:\n  name: {name}\nspec:\n  target: docker\n  resources:\n{}",
        resources.concat()
    );
    emit(&graph_from_text("stack.yaml", &text, "docker"))
}

fn compile(resources: &[String]) -> Emitted {
    compile_named("shop", resources).unwrap_or_else(|e| panic!("the document should compile: {e}"))
}

fn file(resources: &[String]) -> String {
    compile(resources).artifact.files["compose.yaml"].clone()
}

fn refusal(resources: &[String]) -> CompileError {
    compile_named("shop", resources).expect_err("the document should be refused")
}

/// A service with an image, and whatever else is given.
fn service(id: &str, rest: &str) -> String {
    format!("    - id: {id}\n      type: service\n      params:\n        image: {id}:1\n{rest}")
}

fn other(id: &str, kind: &str) -> String {
    format!("    - id: {id}\n      type: {kind}\n      params: {{}}\n")
}

fn project(params: &str) -> String {
    format!("    - id: target\n      type: project\n      params: {params}\n")
}

// ---- golden files ------------------------------------------------------

#[test]
fn a_document_for_a_host_compiles_to_its_golden_files() {
    let graph = graph_from_file("examples/docker/monitoring.yaml", "docker");
    assert_golden(&golden(), "monitoring", &emit(&graph).unwrap());
}

#[test]
fn relative_paths_compile_with_a_warning_each() {
    let graph = graph_from_file(
        "plugins/docker/testdata/documents/relative-paths.yaml",
        "docker",
    );
    assert_golden(&golden(), "relative-paths", &emit(&graph).unwrap());
}

#[test]
fn a_refusal_points_at_the_line_the_author_wrote() {
    let graph = graph_from_file(
        "plugins/docker/testdata/documents/unknown-volume.yaml",
        "docker",
    );
    assert_golden_refusal(&golden(), "unknown-volume", &emit(&graph).unwrap_err());
}

#[test]
fn a_project_directory_answers_the_relative_paths() {
    let emitted = compile(&[
        project("{directory: /srv/shop}"),
        service(
            "web",
            "        build: ./web\n        env_file: web.env\n        volumes: [\"./public:/usr/share/nginx/html\"]\n",
        ),
    ]);
    assert!(emitted.warnings.is_empty(), "{:?}", emitted.warnings);
    assert!(emitted.artifact.files["project.json"].contains("\"directory\": \"/srv/shop\""));
}

// ---- attributes --------------------------------------------------------

#[test]
fn a_misspelt_attribute_is_refused_with_the_one_that_was_meant() {
    let err = refusal(&[service("web", "        enviroment: {A: b}\n")]);
    assert_eq!(err.rule, "unsupported-param");
    assert_eq!(err.resource, "web");
    assert_eq!(
        err.msg,
        "\"enviroment\" is not an attribute of a Compose service; did you mean \"environment\"?"
    );

    let err = refusal(&[
        "    - id: data\n      type: volume\n      params: {size: 10G}\n".to_owned(),
        service("web", ""),
    ]);
    assert_eq!(err.rule, "unsupported-param");
    assert_eq!(
        err.msg,
        "\"size\" is not an attribute of a Compose volume; an extension field starts with \"x-\""
    );
}

#[test]
fn an_extension_field_is_the_authors_own() {
    let text = file(&[service("web", "        x-team: storefront\n")]);
    assert!(text.contains("    x-team: storefront\n"), "{text}");
}

// ---- what a service uses must be declared ------------------------------

#[test]
fn a_network_volume_config_or_secret_a_service_uses_must_be_declared() {
    for (params, what) in [
        ("        networks: [backend]\n", "network \"backend\""),
        (
            "        networks:\n          backend: {aliases: [www]}\n",
            "network \"backend\"",
        ),
        ("        volumes: [\"data:/data\"]\n", "volume \"data\""),
        (
            "        volumes:\n          - {type: volume, source: data, target: /data}\n",
            "volume \"data\"",
        ),
        ("        configs: [site]\n", "config \"site\""),
        (
            "        secrets:\n          - {source: token, target: api_token}\n",
            "secret \"token\"",
        ),
    ] {
        let err = refusal(&[service("web", params)]);
        assert_eq!(err.rule, "invalid-reference", "{params}");
        assert!(
            err.msg.starts_with(&format!(
                "uses the {what}, which the document does not declare"
            )),
            "{}",
            err.msg
        );
    }
}

#[test]
fn bind_mounts_anonymous_volumes_and_the_default_network_need_no_declaration() {
    let emitted = compile(&[service(
        "web",
        "        networks: [default]\n        volumes:\n          - /var/log/nginx:/var/log/nginx\n          - /var/cache/nginx\n          - ~/site:/usr/share/nginx/html:ro\n          - {type: bind, source: /etc/ssl, target: /etc/ssl}\n          - {type: tmpfs, target: /tmp}\n",
    )]);
    assert!(emitted.warnings.is_empty(), "{:?}", emitted.warnings);
}

#[test]
fn a_name_used_as_the_wrong_kind_is_refused() {
    let err = refusal(&[
        other("data", "volume"),
        service("web", "        networks: [\"${data}\"]\n"),
    ]);
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(
        err.msg,
        "uses \"data\" as a network, and the document declares it as a volume"
    );
}

// ---- dependencies ------------------------------------------------------

#[test]
fn a_dependency_waits_for_health_when_the_other_service_can_report_it() {
    let text = file(&[
        service("cache", ""),
        service(
            "db",
            "        healthcheck:\n          test: [CMD, pg_isready]\n",
        ),
        service(
            "web",
            "      dependsOn:\n        - resource: db\n        - resource: cache\n",
        ),
    ]);
    assert!(
        text.contains(
            "    depends_on:\n      cache:\n        condition: service_started\n      db:\n        condition: service_healthy\n"
        ),
        "{text}"
    );
}

#[test]
fn a_disabled_healthcheck_is_not_one_to_wait_for() {
    let text = file(&[
        service("db", "        healthcheck: {disable: true}\n"),
        service("web", "      dependsOn:\n        - resource: db\n"),
    ]);
    assert!(text.contains("condition: service_started"), "{text}");
}

#[test]
fn a_restart_notify_waits_for_health_too() {
    let text = file(&[
        service(
            "api",
            "        healthcheck: {test: [CMD, \"true\"]}\n      notifies:\n        - resource: web\n          action: restart\n",
        ),
        service("web", ""),
    ]);
    assert!(
        text.contains("      api:\n        condition: service_healthy\n        restart: true\n"),
        "{text}"
    );
}

#[test]
fn a_depends_on_the_author_wrote_is_checked() {
    // One that can be met compiles as written.
    let text = file(&[
        service("migrate", "        restart: \"no\"\n"),
        service(
            "web",
            "        depends_on:\n          migrate: {condition: service_completed_successfully}\n",
        ),
    ]);
    assert!(
        text.contains("condition: service_completed_successfully"),
        "{text}"
    );

    let err = refusal(&[service("web", "        depends_on: [cache]\n")]);
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(
        err.msg,
        "depends_on names \"cache\", which is not a service in this document"
    );

    let err = refusal(&[
        other("cache", "volume"),
        service("web", "        depends_on: [cache]\n"),
    ]);
    assert_eq!(
        err.msg,
        "depends_on names \"cache\", which is a volume and not a service"
    );

    let err = refusal(&[
        service("cache", ""),
        service(
            "web",
            "        depends_on:\n          cache: {condition: service_ready}\n",
        ),
    ]);
    assert_eq!(err.rule, "invalid-param");
    assert!(
        err.msg
            .starts_with("\"service_ready\" is not something a dependency can wait for"),
        "{}",
        err.msg
    );

    let err = refusal(&[
        service("cache", ""),
        service(
            "web",
            "        depends_on:\n          cache: {condition: service_healthy}\n",
        ),
    ]);
    assert_eq!(err.rule, "invalid-param");
    assert!(err.msg.contains("declares no healthcheck"), "{}", err.msg);

    let err = refusal(&[service("web", "        depends_on: [web]\n")]);
    assert_eq!(err.msg, "a service cannot depend on itself");
}

#[test]
fn services_that_wait_for_each_other_in_a_circle_are_refused() {
    // The document's own dependsOn cannot make a circle: the core refuses
    // that. A notify, or a depends_on written by hand, can.
    let err = refusal(&[
        service(
            "api",
            "      notifies:\n        - resource: web\n          action: restart\n      dependsOn:\n        - resource: web\n",
        ),
        service("web", ""),
    ]);
    assert_eq!(err.rule, "dependency-cycle");
    assert!(
        err.msg.contains("(web -> api -> web)") || err.msg.contains("(api -> web -> api)"),
        "{}",
        err.msg
    );

    let err = refusal(&[
        service("a", "        depends_on: [b]\n"),
        service("b", "        depends_on: [c]\n"),
        service("c", "        depends_on: [a]\n"),
    ]);
    assert_eq!(err.rule, "dependency-cycle");
    assert!(err.msg.contains("(a -> b -> c -> a)"), "{}", err.msg);
}

// ---- the project's settings --------------------------------------------

#[test]
fn a_project_waits_unless_told_not_to() {
    let default = compile(&[service("web", "")]).artifact.files["project.json"].clone();
    assert!(default.contains("\"wait\": true"), "{default}");
    assert!(default.contains("\"wait_timeout\": 0"), "{default}");
    assert!(default.contains("\"profiles\": []"), "{default}");

    let told = compile(&[
        project("{wait: false, wait_timeout: 90, profiles: [debug, metrics]}"),
        service("web", ""),
    ])
    .artifact
    .files["project.json"]
        .clone();
    assert!(told.contains("\"wait\": false"), "{told}");
    assert!(told.contains("\"wait_timeout\": 90"), "{told}");
    assert!(
        told.contains("\"profiles\": [\n    \"debug\",\n    \"metrics\"\n  ]"),
        "{told}"
    );
}

#[test]
fn project_settings_are_checked() {
    for (params, rule) in [
        ("{directory: site}", "invalid-param"),
        ("{directory: 7}", "invalid-param"),
        ("{profiles: debug}", "invalid-param"),
        ("{profiles: [1]}", "invalid-param"),
        ("{wait: sometimes}", "invalid-param"),
        ("{wait_timeout: 0}", "invalid-param"),
        ("{wait_timeout: soon}", "invalid-param"),
        ("{name: 7}", "invalid-param"),
        ("{pull: always}", "unsupported-param"),
    ] {
        let err = refusal(&[project(params), service("web", "")]);
        assert_eq!(err.rule, rule, "{params}");
        assert_eq!(err.resource, "target");
    }
}
