//! The compile half, without a process boundary.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{assert_golden, graph_from_file, graph_from_text};
use meridian_sdk::{CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_dsc::{node_path, Dsc};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    Dsc.emit(graph, &Data::new())
}

const SERVICE: &str = "    - id: spooler\n      type: PSDscResources/Service\n      params:\n        Name: Spooler\n        State: Running\n";
const KEY: &str = "    - id: key\n      type: Microsoft.Windows/Registry\n      params:\n        keyPath: HKLM\\Software\\App\n";

fn document(host: &str, resources: &str) -> String {
    format!(
        "apiVersion: meridian/v1\nkind: ResourceSet\nmetadata:\n  name: web\nspec:\n  target: dsc\n  targetHost:\n    from: inline\n    host: {host}\n  resources:\n{resources}"
    )
}

fn compile(resources: &str) -> Result<Emitted, CompileError> {
    emit(&graph_from_text(
        "site.yaml",
        &document("localhost", resources),
        "dsc",
    ))
}

fn node(resources: &str) -> String {
    compile(resources).unwrap().artifact.files[&node_path("web")].clone()
}

fn refusal(resources: &str) -> CompileError {
    compile(resources).expect_err("the document should be refused")
}

#[test]
fn the_example_compiles_to_its_golden_files() {
    let graph = graph_from_file("examples/dsc/web-server.yaml", "dsc");
    assert_golden(&golden(), "web-server", &emit(&graph).unwrap());
}

#[test]
fn a_resource_is_a_name_a_type_and_its_properties() {
    assert_eq!(
        node(SERVICE),
        "parameters: {}\nvariables: {}\nresources:\n  - name: spooler\n    type: PSDscResources/Service\n    properties:\n      Name: Spooler\n      State: Running\n"
    );
}

#[test]
fn the_project_is_one_layer_and_carries_the_runners_settings() {
    let emitted = compile(&format!(
        "    - id: settings\n      type: runner\n      params: {{engine: dscv3}}\n{KEY}"
    ))
    .unwrap();
    let datum = &emitted.artifact.files["Datum.yml"];
    assert!(
        datum.contains("  - Projects\\$($Node.ProjectPresence)\\$($Node.Project)\n"),
        "{datum}"
    );
    assert!(datum.contains("  Engine: DscV3\n"), "{datum}");
    assert!(datum.contains("  Target: Local\n"), "{datum}");
    assert!(datum.contains("  ConfigurationVersion: 0.5\n"), "{datum}");
}

#[test]
fn dependencies_and_notifications_are_the_runners_own() {
    let text = node(&format!(
        "{KEY}      notifies:\n        - resource: spooler\n          action: refresh\n{SERVICE}      dependsOn:\n        - resource: key\n    - id: last\n      type: PSDscResources/Service\n      params: {{Name: W32Time}}\n      dependsOn:\n        - resource: spooler\n"
    ));
    assert!(
        text.contains("    notify:\n      - PSDscResources/Service/spooler\n"),
        "{text}"
    );
    assert!(
        text.contains("    dependsOn:\n      - PSDscResources/Service/spooler\n"),
        "{text}"
    );
    // The notify already orders the key first, so it is not said twice.
    assert!(!text.contains("- Microsoft.Windows/Registry/key"), "{text}");
}

#[test]
fn a_reference_reads_the_other_resource_and_orders_after_it() {
    let text = node(&format!(
        "{SERVICE}    - id: note\n      type: PSDscResources/Environment\n      params:\n        Name: SPOOLER\n        Value: state is ${{spooler.State}}, cost $5\n"
    ));
    assert!(
        text.contains("Value: state is $((reference 'spooler').State), cost `$5\n"),
        "{text}"
    );
    assert!(
        text.contains("    dependsOn:\n      - PSDscResources/Service/spooler\n"),
        "{text}"
    );

    let err = refusal(&format!(
        "{SERVICE}    - id: note\n      type: PSDscResources/Environment\n      params:\n        Value: ${{spooler}}\n"
    ));
    assert_eq!(err.rule, "invalid-reference");
    assert!(err.msg.contains("must read a property"), "{}", err.msg);

    let err = refusal(
        "    - id: note\n      type: PSDscResources/Environment\n      params:\n        Value: ${spoolr.State}\n",
    );
    assert!(
        err.msg.contains("not a resource in this document"),
        "{}",
        err.msg
    );
}

#[test]
fn a_runtime_condition_is_a_pre_condition() {
    let text = node(
        "    - id: key\n      type: Microsoft.Windows/Registry\n      params: {keyPath: HKLM\\Software\\App}\n      runtimeWhen: \"equals (nodeName) 'web'\"\n",
    );
    assert!(
        text.contains("    preCondition: equals (nodeName) 'web'\n"),
        "{text}"
    );
}

#[test]
fn another_machine_needs_to_be_told_how_it_is_reached() {
    let err = emit(&graph_from_text(
        "site.yaml",
        &document("web01.example.com", SERVICE),
        "dsc",
    ))
    .unwrap_err();
    assert_eq!(err.rule, "missing-param");
    assert!(
        err.msg.contains("target: WinRM or target: SSH"),
        "{}",
        err.msg
    );

    let emitted = emit(&graph_from_text(
        "site.yaml",
        &document(
            "web01.example.com",
            &format!(
                "    - id: settings\n      type: runner\n      params: {{target: ssh, credential: deploy}}\n{SERVICE}"
            ),
        ),
        "dsc",
    ))
    .unwrap();
    let text = &emitted.artifact.files[&node_path("web")];
    assert!(
        text.contains(
            "    target:\n      action: SSH\n      computerName: web01.example.com\n      credentialRef: deploy\n"
        ),
        "{text}"
    );
}

#[test]
fn what_dsc_cannot_take_is_refused() {
    let err = refusal("    - id: nginx\n      type: package\n      params: {name: nginx}\n");
    assert_eq!(err.rule, "unsupported-resource-type");
    assert!(err.msg.contains("PSDscResources/Service"), "{}", err.msg);

    let err = refusal(
        "    - id: key\n      type: Microsoft.Windows/Registry\n      state: absent\n      params: {keyPath: x}\n",
    );
    assert_eq!(err.rule, "unsupported-state");
    assert!(err.msg.contains("Ensure or _exist"), "{}", err.msg);

    let err = refusal(&format!(
        "{KEY}      notifies:\n        - resource: spooler\n          action: restart\n{SERVICE}"
    ));
    assert_eq!(err.rule, "unsupported-action");
    assert!(err.msg.contains("(supported: refresh)"), "{}", err.msg);

    let err = refusal(&format!(
        "    - id: settings\n      type: runner\n      params: {{engine: v9}}\n{KEY}"
    ));
    assert_eq!(err.rule, "invalid-param");
    assert!(
        err.msg.contains("accepted: Auto, DscV2, DscV3"),
        "{}",
        err.msg
    );

    let err = refusal("    - id: settings\n      type: runner\n      params: {engine: Auto}\n");
    assert_eq!(err.rule, "empty-configuration");
}
