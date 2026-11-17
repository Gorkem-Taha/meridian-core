//! The compile half, without a process boundary.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{assert_golden, graph_from_file, graph_from_text};
use meridian_sdk::{CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_docker::Docker;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    Docker.emit(graph, &Data::new())
}

const DB: &str = "    - id: db\n      type: service\n      params:\n        image: postgres:16\n";
const APP: &str = "    - id: app\n      type: service\n      params:\n        image: app:1\n";
const SITE: &str =
    "    - id: site\n      type: config\n      params:\n        content: \"listen 80;\"\n";

fn document(resources: &str) -> String {
    format!(
        "apiVersion: meridian/v1\nkind: Infrastructure\nmetadata:\n  name: shop\nspec:\n  target: docker\n  resources:\n{resources}"
    )
}

fn compile(resources: &str) -> Result<Emitted, CompileError> {
    emit(&graph_from_text(
        "infra.yaml",
        &document(resources),
        "docker",
    ))
}

fn compose(resources: &str) -> String {
    compile(resources).unwrap().artifact.files["compose.yaml"].clone()
}

fn refusal(resources: &str) -> CompileError {
    compile(resources).expect_err("the document should be refused")
}

#[test]
fn the_example_compiles_to_its_golden_file() {
    let graph = graph_from_file("examples/docker/web.yaml", "docker");
    assert_golden(&golden(), "web", &emit(&graph).unwrap());
}

#[test]
fn the_project_is_named_after_the_document_and_sections_come_in_order() {
    let text = compose(&format!(
        "    - id: data\n      type: volume\n      params: {{}}\n{DB}"
    ));
    assert_eq!(
        text,
        "name: shop\nservices:\n  db:\n    image: postgres:16\nvolumes:\n  data: {}\n"
    );
}

#[test]
fn a_dependency_between_services_is_composes_own() {
    let text = compose(&format!(
        "{DB}{APP}      dependsOn:\n        - resource: db\n"
    ));
    assert!(
        text.contains(
            "  app:\n    image: app:1\n    depends_on:\n      db:\n        condition: service_started\n"
        ),
        "{text}"
    );
}

#[test]
fn what_the_author_wrote_about_a_dependency_is_kept() {
    let text = compose(&format!(
        "{DB}    - id: app\n      type: service\n      params:\n        image: app:1\n        depends_on:\n          db:\n            condition: service_healthy\n      dependsOn:\n        - resource: db\n"
    ));
    assert!(text.contains("condition: service_healthy"), "{text}");
    assert!(!text.contains("service_started"), "{text}");
}

#[test]
fn a_service_notifying_a_service_is_a_native_restart() {
    let emitted = compile(&format!(
        "{DB}      notifies:\n        - resource: app\n          action: restart\n{APP}"
    ))
    .unwrap();
    let text = &emitted.artifact.files["compose.yaml"];
    assert!(
        text.contains("      db:\n        condition: service_started\n        restart: true\n"),
        "{text}"
    );
    assert!(emitted.warnings.is_empty());
}

#[test]
fn anything_else_notifying_a_service_is_a_checksum_label() {
    let notify = "      notifies:\n        - resource: app\n          action: restart\n";
    let first = compile(&format!("{SITE}{notify}{APP}")).unwrap();
    let text = &first.artifact.files["compose.yaml"];
    assert!(
        text.contains("    labels:\n      meridian.dev/restart-on-site: "),
        "{text}"
    );
    assert_eq!(first.warnings.len(), 1);
    assert!(
        first.warnings[0].msg.contains("being recreated"),
        "{}",
        first.warnings[0].msg
    );
    let changed = compose(&format!(
        "{}{notify}{APP}",
        SITE.replace("listen 80", "listen 81")
    ));
    let line = |text: &str| {
        text.lines()
            .find(|l| l.contains("restart-on-site"))
            .unwrap()
            .to_owned()
    };
    assert_ne!(line(text), line(&changed));
}

#[test]
fn a_notify_compose_cannot_act_on_is_refused() {
    let err = refusal(&format!(
        "{DB}      notifies:\n        - resource: app\n          action: reload\n{APP}"
    ));
    assert_eq!(err.rule, "unsupported-action");
    assert!(err.msg.contains("(supported: restart)"), "{}", err.msg);

    let err = refusal(&format!(
        "{APP}      notifies:\n        - resource: site\n          action: restart\n{SITE}"
    ));
    assert_eq!(err.rule, "unsupported-action");
    assert!(err.msg.contains("a config"), "{}", err.msg);
}

#[test]
fn a_reference_is_an_elements_name_and_a_dollar_is_kept_from_compose() {
    let text = compose(&format!(
        "{DB}    - id: app\n      type: service\n      params:\n        image: app:1\n        environment:\n          DB_HOST: ${{db}}\n          PRICE: \"$5 and $${{HOME}}\"\n"
    ));
    assert!(text.contains("DB_HOST: db\n"), "{text}");
    assert!(text.contains("PRICE: $$5 and $${HOME}\n"), "{text}");

    let err = refusal(&format!(
        "{DB}    - id: app\n      type: service\n      params:\n        image: app:1\n        environment:\n          DB_HOST: ${{db.State.Status}}\n"
    ));
    assert_eq!(err.rule, "invalid-reference");
    assert!(
        err.msg.contains("only known once the project is up"),
        "{}",
        err.msg
    );

    let err = refusal(&format!(
        "{DB}    - id: app\n      type: service\n      params:\n        image: app:1\n        networks: [\"${{backend}}\"]\n"
    ));
    assert_eq!(
        err.msg,
        "param \"networks\": ${backend} refers to \"backend\", which is not an element of this document"
    );
}

#[test]
fn the_sidecar_says_where_the_project_runs_and_how_to_read_outputs() {
    let text = format!(
        "{}  outputs:\n    - name: url\n      value: http://localhost:${{db.port.5432}}\n    - name: state\n      value: ${{db.State.Status}}\n",
        document(&format!(
            "    - id: target\n      type: project\n      params: {{name: store, host: web01}}\n{DB}"
        ))
    );
    let emitted = emit(&graph_from_text("infra.yaml", &text, "docker")).unwrap();
    let project = &emitted.artifact.files["project.json"];
    assert!(project.contains("\"name\": \"store\""), "{project}");
    assert!(project.contains("\"host\": \"ssh://web01\""), "{project}");
    assert!(project.contains("\"port\": \"5432\""), "{project}");
    assert!(
        project.contains("\"inspect\": \".State.Status\""),
        "{project}"
    );
    let compose = &emitted.artifact.files["compose.yaml"];
    assert!(compose.starts_with("name: store\n"), "{compose}");
    assert!(!compose.contains("web01"), "{compose}");
}

#[test]
fn an_output_must_read_from_a_service() {
    let text = format!(
        "{}  outputs:\n    - name: url\n      value: ${{site.port.80}}\n",
        document(&format!("{SITE}{DB}"))
    );
    let err = emit(&graph_from_text("infra.yaml", &text, "docker")).unwrap_err();
    assert_eq!(err.rule, "invalid-reference");
    assert!(err.msg.contains("not a service"), "{}", err.msg);
}

#[test]
fn types_states_and_missing_images_are_checked() {
    let err = refusal("    - id: nginx\n      type: package\n      params: {name: nginx}\n");
    assert_eq!(err.rule, "unsupported-resource-type");
    assert!(
        err.msg.contains("supported: config, network"),
        "{}",
        err.msg
    );

    let err = refusal("    - id: app\n      type: service\n      params: {command: run}\n");
    assert_eq!(err.rule, "missing-param");

    let err = refusal(SITE);
    assert_eq!(err.rule, "empty-project");

    let err = refusal(
        "    - id: db\n      type: service\n      state: absent\n      params: {image: x}\n",
    );
    assert_eq!(err.rule, "unsupported-state");

    let err = refusal(&format!(
        "    - id: target\n      type: project\n      params: {{name: My Shop}}\n{DB}"
    ));
    assert_eq!(err.rule, "invalid-name");
}
