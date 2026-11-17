//! The compile half, without a process boundary.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{
    assert_golden, assert_golden_refusal, graph_from_file, graph_from_text,
};
use meridian_sdk::{Category, CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_compose::Compose;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    Compose.emit(graph, &Data::new())
}

/// Compiles an Infrastructure document with the given name and `spec` body.
fn compile_named(name: &str, spec: &str) -> Result<Emitted, CompileError> {
    let text = format!(
        "apiVersion: meridian/v1\nkind: Infrastructure\nmetadata:\n  name: {name}\nspec:\n  target: compose\n{spec}"
    );
    emit(&graph_from_text("stack.yaml", &text, "compose"))
}

fn compile(spec: &str) -> Emitted {
    compile_named("shop", spec).unwrap_or_else(|e| panic!("the document should compile: {e}"))
}

fn file(spec: &str) -> String {
    compile(spec).artifact.files["compose.yaml"].clone()
}

fn settings(spec: &str) -> String {
    compile(spec).artifact.files["project.json"].clone()
}

fn refusal(spec: &str) -> CompileError {
    compile_named("shop", spec).expect_err("the document should be refused")
}

/// A service with an image, and whatever else is given, indented to sit
/// under `resources:`.
fn service(id: &str, rest: &str) -> String {
    format!("    - id: {id}\n      type: service\n      params:\n        image: {id}:1\n{rest}")
}

fn resources(items: &[String]) -> String {
    format!("  resources:\n{}", items.concat())
}

// ---- golden files ------------------------------------------------------

#[test]
fn the_example_compiles_to_its_golden_files() {
    let graph = graph_from_file("examples/compose/shop.yaml", "compose");
    assert_golden(&golden(), "shop", &emit(&graph).unwrap());
}

#[test]
fn a_document_for_a_host_compiles_to_its_golden_files() {
    let graph = graph_from_file("examples/compose/monitoring.yaml", "compose");
    assert_golden(&golden(), "monitoring", &emit(&graph).unwrap());
}

#[test]
fn relative_paths_compile_with_a_warning_each() {
    let graph = graph_from_file(
        "plugins/compose/testdata/documents/relative-paths.yaml",
        "compose",
    );
    assert_golden(&golden(), "relative-paths", &emit(&graph).unwrap());
}

#[test]
fn a_refusal_points_at_the_line_the_author_wrote() {
    let graph = graph_from_file(
        "plugins/compose/testdata/documents/unknown-volume.yaml",
        "compose",
    );
    assert_golden_refusal(&golden(), "unknown-volume", &emit(&graph).unwrap_err());
}

#[test]
fn compiling_twice_gives_the_same_bytes() {
    let graph = graph_from_file("examples/compose/shop.yaml", "compose");
    assert_eq!(emit(&graph).unwrap(), emit(&graph).unwrap());
}

// ---- what the plugin says about itself ---------------------------------

#[test]
fn the_plugin_declares_what_it_is() {
    assert_eq!(Compose.name(), "compose");
    assert_eq!(Compose.category(), Category::Containerization);
    assert_eq!(
        Compose.supported_resource_types(),
        ["config", "network", "project", "secret", "service", "volume"]
    );
    let capabilities = Compose.capabilities();
    assert!(capabilities.native_notify);
    assert!(!capabilities.synthetic_notify);
    assert!(!capabilities.runtime_condition);
}

// ---- the shape of the file ---------------------------------------------

#[test]
fn sections_follow_composes_convention_and_empty_ones_are_left_out() {
    let text = file(&resources(&[
        "    - id: data\n      type: volume\n".to_owned(),
        service("web", ""),
    ]));
    assert_eq!(
        text,
        "# Compiled by Meridian from the document shop. Do not edit.\n\
         name: shop\n\
         services:\n  web:\n    image: web:1\n\
         volumes:\n  data: {}\n"
    );
}

#[test]
fn params_pass_through_as_compose_documents_them() {
    let text = file(&resources(&[service(
        "web",
        "        ports: [\"8080:80\", \"443\"]\n        environment:\n          DEBUG: false\n          WORKERS: 4\n        deploy:\n          resources:\n            limits: {cpus: \"0.5\", memory: 256M}\n        x-team: storefront\n",
    )]));
    assert!(
        text.contains("    ports:\n      - 8080:80\n      - \"443\"\n"),
        "{text}"
    );
    assert!(
        text.contains("    environment:\n      DEBUG: false\n      WORKERS: 4\n"),
        "{text}"
    );
    assert!(
        text.contains("          cpus: \"0.5\"\n          memory: 256M\n"),
        "{text}"
    );
    assert!(text.contains("    x-team: storefront\n"), "{text}");
}

// ---- references --------------------------------------------------------

#[test]
fn a_reference_becomes_the_name_compose_knows_the_resource_by() {
    let text = file(&resources(&[
        "    - id: data\n      type: volume\n".to_owned(),
        "    - id: backend\n      type: network\n".to_owned(),
        service("db", ""),
        service(
            "web",
            "        environment:\n          DATABASE_URL: postgres://${db}:5432/shop\n        volumes: [\"${data}:/var/lib/data\"]\n        networks: [\"${ backend }\"]\n",
        ),
    ]));
    assert!(
        text.contains("      DATABASE_URL: postgres://db:5432/shop\n"),
        "{text}"
    );
    assert!(text.contains("      - data:/var/lib/data\n"), "{text}");
    assert!(text.contains("    networks:\n      - backend\n"), "{text}");
}

#[test]
fn an_escaped_reference_reaches_compose_as_its_own_variable() {
    let text = file(&resources(&[service(
        "web",
        "        environment:\n          TAG: $${TAG:-latest}\n          PRICE: $$5\n",
    )]));
    assert!(text.contains("      TAG: ${TAG:-latest}\n"), "{text}");
    assert!(text.contains("      PRICE: $$5\n"), "{text}");
}

#[test]
fn a_reference_to_nothing_is_refused_and_says_how_to_write_a_variable() {
    let err = refusal(&resources(&[service(
        "web",
        "        environment:\n          TAG: ${TAG}\n",
    )]));
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(err.resource, "web");
    assert_eq!(
        err.msg,
        "param \"environment\": ${TAG} refers to \"TAG\", which is not a resource in this \
         document. For one of Compose's own variables, write $${TAG}"
    );
}

#[test]
fn a_reference_to_an_attribute_is_refused_in_a_param() {
    let err = refusal(&resources(&[
        service("db", ""),
        service(
            "web",
            "        environment:\n          DB_PORT: ${db.port.5432}\n",
        ),
    ]));
    assert_eq!(err.rule, "invalid-reference");
    assert!(err.msg.contains("Write ${db} for its name"), "{}", err.msg);
}

#[test]
fn an_unclosed_reference_is_refused() {
    let err = refusal(&resources(&[service(
        "web",
        "        command: echo ${oops\n",
    )]));
    assert_eq!(err.rule, "invalid-reference");
    assert!(err.msg.contains("never closed"), "{}", err.msg);
}

#[test]
fn the_project_can_be_referred_to_by_name() {
    let text = file(&resources(&[
        "    - id: deployment\n      type: project\n      params: {name: shop-prod}\n".to_owned(),
        service(
            "web",
            "        labels:\n          project: ${deployment}\n          again: ${deployment.name}\n",
        ),
    ]));
    assert!(text.starts_with(
        "# Compiled by Meridian from the document shop. Do not edit.\nname: shop-prod\n"
    ));
    assert!(text.contains("      project: shop-prod\n"), "{text}");
    assert!(text.contains("      again: shop-prod\n"), "{text}");
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
        let err = refusal(&resources(&[service("web", params)]));
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
    let directory =
        "    - id: deployment\n      type: project\n      params: {directory: /srv/shop}\n"
            .to_owned();
    let emitted = compile(&resources(&[
        directory,
        service(
            "web",
            "        networks: [default]\n        volumes:\n          - ./site:/usr/share/nginx/html:ro\n          - /var/log/nginx:/var/log/nginx\n          - /var/cache/nginx\n          - $${DATA_DIR}:/data\n          - {type: bind, source: /etc/ssl, target: /etc/ssl}\n          - {type: tmpfs, target: /tmp}\n",
        ),
    ]));
    assert!(emitted.warnings.is_empty(), "{:?}", emitted.warnings);
}

#[test]
fn a_name_used_as_the_wrong_kind_is_refused() {
    let err = refusal(&resources(&[
        "    - id: data\n      type: volume\n".to_owned(),
        service("web", "        networks: [\"${data}\"]\n"),
    ]));
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(
        err.msg,
        "uses \"data\" as a network, and the document declares it as a volume"
    );
}

// ---- ordering ----------------------------------------------------------

#[test]
fn a_dependency_waits_for_health_when_the_other_service_can_report_it() {
    let text = file(&resources(&[
        service("cache", ""),
        service(
            "db",
            "        healthcheck:\n          test: [CMD, pg_isready]\n",
        ),
        service(
            "web",
            "      dependsOn:\n        - resource: db\n        - resource: cache\n",
        ),
    ]));
    assert!(
        text.contains(
            "  web:\n    depends_on:\n      cache:\n        condition: service_started\n      db:\n        condition: service_healthy\n    image: web:1\n"
        ),
        "{text}"
    );
}

#[test]
fn a_disabled_healthcheck_is_not_one_to_wait_for() {
    let text = file(&resources(&[
        service("db", "        healthcheck: {disable: true}\n"),
        service("web", "      dependsOn:\n        - resource: db\n"),
    ]));
    assert!(text.contains("condition: service_started"), "{text}");
}

#[test]
fn a_dependency_on_a_network_or_volume_needs_nothing_written() {
    let text = file(&resources(&[
        "    - id: data\n      type: volume\n".to_owned(),
        service(
            "web",
            "        volumes: [\"data:/data\"]\n      dependsOn:\n        - resource: data\n",
        ),
    ]));
    assert!(!text.contains("depends_on"), "{text}");
}

#[test]
fn a_network_cannot_depend_on_anything() {
    let err = refusal(&resources(&[
        service("web", ""),
        "    - id: backend\n      type: network\n      dependsOn:\n        - resource: web\n"
            .to_owned(),
    ]));
    assert_eq!(err.rule, "unsupported-dependency");
    assert_eq!(err.resource, "backend");
    assert!(
        err.msg.starts_with("a network cannot depend on \"web\""),
        "{}",
        err.msg
    );
}

#[test]
fn a_dependency_on_the_projects_settings_is_refused() {
    let err = refusal(&resources(&[
        "    - id: deployment\n      type: project\n".to_owned(),
        service("web", "      dependsOn:\n        - resource: deployment\n"),
    ]));
    assert_eq!(err.rule, "unsupported-dependency");
}

#[test]
fn a_restart_notify_becomes_restart_true_on_the_notified_service() {
    let text = file(&resources(&[
        service(
            "api",
            "      notifies:\n        - resource: web\n          action: restart\n",
        ),
        service("web", ""),
    ]));
    assert!(
        text.contains(
            "  web:\n    depends_on:\n      api:\n        condition: service_started\n        restart: true\n"
        ),
        "{text}"
    );
}

#[test]
fn a_notify_and_a_dependency_between_the_same_services_are_one_entry() {
    let text = file(&resources(&[
        service(
            "api",
            "        healthcheck: {test: [CMD, \"true\"]}\n      notifies:\n        - resource: web\n          action: restart\n",
        ),
        service("web", "      dependsOn:\n        - resource: api\n"),
    ]));
    assert_eq!(text.matches("      api:\n").count(), 1, "{text}");
    assert!(
        text.contains("      api:\n        condition: service_healthy\n        restart: true\n"),
        "{text}"
    );
}

#[test]
fn any_other_notify_is_refused_because_nothing_could_carry_it() {
    let err = refusal(&resources(&[
        service(
            "api",
            "      notifies:\n        - resource: web\n          action: reload\n",
        ),
        service("web", ""),
    ]));
    assert_eq!(err.rule, "unsupported-action");
    assert_eq!(err.resource, "api");
    assert!(
        err.msg
            .starts_with("notifies service \"web\" with action \"reload\""),
        "{}",
        err.msg
    );

    let err = refusal(&resources(&[
        "    - id: site\n      type: config\n      params: {content: hello}\n      notifies:\n        - resource: web\n          action: restart\n".to_owned(),
        service("web", ""),
    ]));
    assert_eq!(err.rule, "unsupported-action");
    assert_eq!(err.resource, "site");
}

#[test]
fn a_depends_on_the_author_wrote_is_kept_and_checked() {
    let text = file(&resources(&[
        service("migrate", "        restart: \"no\"\n"),
        service("cache", ""),
        service(
            "web",
            "        depends_on:\n          migrate:\n            condition: service_completed_successfully\n          cache:\n            required: false\n",
        ),
    ]));
    assert!(
        text.contains(
            "    depends_on:\n      cache:\n        condition: service_started\n        required: false\n      migrate:\n        condition: service_completed_successfully\n"
        ),
        "{text}"
    );

    let short = file(&resources(&[
        service("cache", ""),
        service("web", "        depends_on: [cache]\n"),
    ]));
    assert!(
        short.contains("    depends_on:\n      cache:\n        condition: service_started\n"),
        "{short}"
    );

    let err = refusal(&resources(&[service(
        "web",
        "        depends_on: [cache]\n",
    )]));
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(
        err.msg,
        "depends_on names \"cache\", which is not a service in this document"
    );

    let err = refusal(&resources(&[
        service("cache", ""),
        service(
            "web",
            "        depends_on:\n          cache: {condition: service_ready}\n",
        ),
    ]));
    assert_eq!(err.rule, "invalid-param");
    assert!(err
        .msg
        .starts_with("\"service_ready\" is not something a dependency can wait for"));

    let err = refusal(&resources(&[
        service("cache", ""),
        service(
            "web",
            "        depends_on:\n          cache: {condition: service_healthy}\n",
        ),
    ]));
    assert_eq!(err.rule, "invalid-param");
    assert!(err.msg.contains("declares no healthcheck"), "{}", err.msg);
}

#[test]
fn a_document_dependency_adds_to_a_depends_on_without_overriding_its_condition() {
    let text = file(&resources(&[
        service("migrate", ""),
        service(
            "web",
            "        depends_on:\n          migrate: {condition: service_completed_successfully}\n      dependsOn:\n        - resource: migrate\n",
        ),
    ]));
    assert_eq!(text.matches("      migrate:\n").count(), 1, "{text}");
    assert!(
        text.contains("condition: service_completed_successfully"),
        "{text}"
    );
}

#[test]
fn services_that_wait_for_each_other_in_a_circle_are_refused() {
    // The document's own dependsOn cannot make a circle, the core refuses
    // that. A notify, or a depends_on written by hand, can.
    let err = refusal(&resources(&[
        service(
            "api",
            "      notifies:\n        - resource: web\n          action: restart\n      dependsOn:\n        - resource: web\n",
        ),
        service("web", ""),
    ]));
    assert_eq!(err.rule, "dependency-cycle");
    assert!(
        err.msg.contains("(web -> api -> web)") || err.msg.contains("(api -> web -> api)"),
        "{}",
        err.msg
    );

    let err = refusal(&resources(&[service("web", "        depends_on: [web]\n")]));
    assert_eq!(err.msg, "a service cannot depend on itself");
}

// ---- what is refused outright ------------------------------------------

#[test]
fn a_type_that_is_not_composes_is_refused() {
    let err = refusal(
        "  resources:\n    - id: nginx\n      type: package\n      params: {name: nginx}\n",
    );
    assert_eq!(err.rule, "unsupported-resource-type");
    assert_eq!(
        err.msg,
        "\"package\" is not something a Compose file declares (accepted: config, network, \
         project, secret, service, volume)"
    );
}

#[test]
fn a_misspelt_attribute_is_refused_with_the_one_that_was_meant() {
    let err = refusal(&resources(&[service(
        "web",
        "        enviroment: {A: b}\n",
    )]));
    assert_eq!(err.rule, "unsupported-param");
    assert_eq!(
        err.msg,
        "\"enviroment\" is not an attribute of a Compose service; did you mean \"environment\"?"
    );
    let err =
        refusal("  resources:\n    - id: data\n      type: volume\n      params: {size: 10G}\n");
    assert_eq!(err.rule, "unsupported-param");
}

#[test]
fn a_service_with_nothing_to_run_is_refused() {
    let err = refusal(
        "  resources:\n    - id: web\n      type: service\n      params: {ports: [\"80\"]}\n",
    );
    assert_eq!(err.rule, "missing-image");

    // A build is something to run.
    compile("  resources:\n    - id: web\n      type: service\n      params: {build: /srv/web}\n");
}

#[test]
fn a_project_with_no_service_is_refused() {
    let err = refusal("  resources:\n    - id: data\n      type: volume\n");
    assert_eq!(err.rule, "empty-project");
}

#[test]
fn an_id_compose_could_not_use_as_a_name_is_refused() {
    let err =
        refusal("  resources:\n    - id: _web\n      type: service\n      params: {image: web}\n");
    assert_eq!(err.rule, "invalid-name");
}

#[test]
fn a_state_or_condition_compose_cannot_express_is_refused() {
    let err = refusal(
        "  resources:\n    - id: web\n      type: service\n      state: absent\n      params: {image: web}\n",
    );
    assert_eq!(err.rule, "unsupported-state");
    assert!(err.msg.contains("remove the resource from the document"));

    let err = refusal(
        "  resources:\n    - id: web\n      type: service\n      state: stopped\n      params: {image: web}\n",
    );
    assert_eq!(err.rule, "unsupported-state");
    assert!(err.msg.contains("(accepted: present)"));
}

// ---- the project's settings --------------------------------------------

#[test]
fn the_project_is_named_by_the_document_unless_told() {
    assert!(settings(&resources(&[service("web", "")])).contains("\"name\": \"shop\""));

    let err = compile_named("Shop.Prod", &resources(&[service("web", "")])).unwrap_err();
    assert_eq!(err.rule, "invalid-project-name");
    assert!(err
        .msg
        .starts_with("\"Shop.Prod\" cannot be a Compose project name"));

    let named = compile_named(
        "Shop.Prod",
        &resources(&[
            "    - id: deployment\n      type: project\n      params: {name: shop-prod}\n"
                .to_owned(),
            service("web", ""),
        ]),
    )
    .unwrap();
    assert!(named.artifact.files["project.json"].contains("\"name\": \"shop-prod\""));
    assert!(!named.artifact.files["compose.yaml"].contains("deployment"));
}

#[test]
fn the_settings_travel_beside_the_file() {
    let text = settings(&resources(&[
        "    - id: deployment\n      type: project\n      params:\n        directory: /srv/shop\n        profiles: [debug, metrics]\n        wait: false\n        wait_timeout: 90\n".to_owned(),
        service("web", ""),
    ]));
    assert_eq!(
        text.trim_end(),
        "{\n  \"directory\": \"/srv/shop\",\n  \"name\": \"shop\",\n  \"outputs\": {},\n  \"profiles\": [\n    \"debug\",\n    \"metrics\"\n  ],\n  \"wait\": false,\n  \"wait_timeout\": 90\n}"
    );
}

#[test]
fn project_settings_are_checked() {
    for (params, rule) in [
        ("{directory: site}", "invalid-param"),
        ("{profiles: debug}", "invalid-param"),
        ("{profiles: [1]}", "invalid-param"),
        ("{wait: sometimes}", "invalid-param"),
        ("{wait_timeout: 0}", "invalid-param"),
        ("{wait_timeout: soon}", "invalid-param"),
        ("{name: 7}", "invalid-param"),
        ("{pull: always}", "unsupported-param"),
    ] {
        let err = refusal(&resources(&[
            format!("    - id: deployment\n      type: project\n      params: {params}\n"),
            service("web", ""),
        ]));
        assert_eq!(err.rule, rule, "{params}");
        assert_eq!(err.resource, "deployment");
    }

    let err = refusal(&resources(&[
        "    - id: one\n      type: project\n".to_owned(),
        "    - id: two\n      type: project\n".to_owned(),
        service("web", ""),
    ]));
    assert_eq!(err.rule, "duplicate-project");
}

// ---- outputs -----------------------------------------------------------

fn outputs(items: &[(&str, &str)]) -> String {
    let mut text = "  outputs:\n".to_owned();
    for (name, value) in items {
        text.push_str(&format!("    - name: {name}\n      value: \"{value}\"\n"));
    }
    text
}

#[test]
fn an_output_is_filled_in_as_far_as_a_compile_can() {
    let spec = format!(
        "{}{}",
        resources(&[
            "    - id: deployment\n      type: project\n      params: {name: shop-prod}\n"
                .to_owned(),
            "    - id: data\n      type: volume\n".to_owned(),
            "    - id: logs\n      type: volume\n      params: {name: shared-logs}\n".to_owned(),
            "    - id: edge\n      type: network\n      params: {external: true}\n".to_owned(),
            service("web", "        ports: [\"80\"]\n"),
        ]),
        outputs(&[
            ("service", "${web}"),
            ("project", "${deployment}"),
            ("volume", "${data}"),
            ("named_volume", "${logs}"),
            ("external_network", "${edge}"),
            ("url", "http://${web.address.80}/"),
            ("dns", "${web.port.53/udp}"),
            ("container", "${ web.id }"),
            ("literal", "costs $${5} at ${web}"),
        ]),
    );
    let text = settings(&spec);
    for line in [
        "\"service\": \"web\"",
        "\"project\": \"shop-prod\"",
        "\"volume\": \"shop-prod_data\"",
        "\"named_volume\": \"shared-logs\"",
        "\"external_network\": \"edge\"",
        "\"url\": \"http://${web.address.80}/\"",
        "\"dns\": \"${web.port.53/udp}\"",
        "\"container\": \"${web.id}\"",
        "\"literal\": \"costs $${5} at web\"",
    ] {
        assert!(text.contains(line), "{line} is not in:\n{text}");
    }
}

#[test]
fn an_output_the_runner_could_not_answer_is_refused_at_compile_time() {
    let base = resources(&[
        "    - id: data\n      type: volume\n".to_owned(),
        service("web", ""),
    ]);
    for (value, says) in [
        (
            "${api.port.80}",
            "refers to \"api\", which is not a resource in this document",
        ),
        (
            "${web.ip}",
            "is not something this target can read from a service",
        ),
        (
            "${web.port.http}",
            "is not something this target can read from a service",
        ),
        (
            "${web.port.0}",
            "is not something this target can read from a service",
        ),
        (
            "${data.mountpoint}",
            "of a volume, which has nothing to read but its name",
        ),
        ("${web", "never closed"),
    ] {
        let err = refusal(&format!("{base}{}", outputs(&[("out", value)])));
        assert_eq!(err.rule, "invalid-reference", "{value}");
        assert!(err.msg.starts_with("output \"out\": "), "{}", err.msg);
        assert!(err.msg.contains(says), "{value}: {}", err.msg);
    }
}
