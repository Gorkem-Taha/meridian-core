//! The compile half, without a process boundary.

#![allow(clippy::result_large_err)]

use std::path::{Path, PathBuf};

use meridian_plugin_testkit::{assert_golden, graph_from_file, graph_from_text};
use meridian_sdk::{CompileError, Data, Emitted, Emitter, ResourceGraph};
use meridian_target_cloudformation::CloudFormation;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("testdata")
}

fn emit(graph: &ResourceGraph) -> Result<Emitted, CompileError> {
    CloudFormation.emit(graph, &Data::new())
}

fn compile_named(name: &str, spec: &str) -> Result<Emitted, CompileError> {
    let text = format!(
        "apiVersion: meridian/v1\nkind: Infrastructure\nmetadata:\n  name: {name}\nspec:\n  target: cloudformation\n{spec}"
    );
    emit(&graph_from_text("infra.yaml", &text, "cloudformation"))
}

fn template(spec: &str) -> String {
    compile_named("net", spec).unwrap().artifact.files["template.yaml"].clone()
}

fn refusal(spec: &str) -> CompileError {
    compile_named("net", spec).expect_err("the document should be refused")
}

const BUCKET: &str =
    "    - id: logs\n      type: AWS::S3::Bucket\n      params: {BucketName: logs}\n";

#[test]
fn the_example_compiles_to_its_golden_template() {
    let graph = graph_from_file("examples/cloudformation/web-vm.yaml", "cloudformation");
    assert_golden(&golden(), "web-vm", &emit(&graph).unwrap());
}

#[test]
fn references_become_ref_getatt_and_sub() {
    let text = template(&format!(
        "  resources:\n{BUCKET}    - id: reader\n      type: AWS::IAM::Policy\n      params:\n        Bucket: ${{logs}}\n        Arn: ${{logs.Arn}}\n        Resource: ${{logs.Arn}}/*\n        Literal: cost is $${{5}}\n        Region: ${{AWS::Region}}\n"
    ));
    assert!(text.contains("Bucket:\n        Ref: Logs\n"), "{text}");
    assert!(
        text.contains("Arn:\n        Fn::GetAtt:\n          - Logs\n          - Arn\n"),
        "{text}"
    );
    assert!(text.contains("Fn::Sub: ${Logs.Arn}/*\n"), "{text}");
    assert!(text.contains("Literal: cost is ${5}\n"), "{text}");
    assert!(
        text.contains("Region:\n        Ref: AWS::Region\n"),
        "{text}"
    );
    // Every one of those references already orders the policy after the bucket.
    assert!(!text.contains("DependsOn"), "{text}");
}

#[test]
fn a_literal_dollar_brace_inside_a_sub_is_escaped() {
    let text = template(&format!(
        "  resources:\n{BUCKET}    - id: fn\n      type: AWS::Lambda::Function\n      params:\n        Code: \"echo $${{HOME}} ${{logs}}\"\n"
    ));
    assert!(text.contains("Fn::Sub: echo ${!HOME} ${Logs}\n"), "{text}");
}

#[test]
fn a_reference_to_nothing_is_refused_at_compile_time() {
    let err = refusal(&format!(
        "  resources:\n{BUCKET}    - id: fn\n      type: AWS::Lambda::Function\n      params:\n        Bucket: ${{log.Arn}}\n"
    ));
    assert_eq!(err.rule, "invalid-reference");
    assert_eq!(
        err.msg,
        "param \"Bucket\": ${log.Arn} refers to \"log\", which is not a resource in this document"
    );
}

#[test]
fn a_notify_is_refused_because_nothing_could_carry_it() {
    let err = refusal(&format!(
        "  resources:\n{BUCKET}    - id: fn\n      type: AWS::Lambda::Function\n      params: {{Handler: x}}\n      notifies:\n        - resource: logs\n          action: restart\n"
    ));
    assert_eq!(err.rule, "unsupported-action");
    assert_eq!(err.resource, "fn");
}

#[test]
fn a_type_that_is_not_cloudformations_is_refused() {
    let err =
        refusal("  resources:\n    - id: vm\n      type: aws_instance\n      params: {ami: x}\n");
    assert_eq!(err.rule, "unsupported-resource-type");
}

#[test]
fn two_ids_that_collapse_to_one_logical_id_are_refused() {
    let err = refusal(
        "  resources:\n    - id: web_vm\n      type: AWS::EC2::Instance\n      params: {ImageId: a}\n    - id: web-vm\n      type: AWS::EC2::Instance\n      params: {ImageId: b}\n",
    );
    assert_eq!(err.rule, "duplicate-logical-id");
    assert_eq!(
        err.msg,
        "resource ids \"web_vm\" and \"web-vm\" both become the logical id \"WebVm\"; \
         CloudFormation's logical ids hold only letters and digits"
    );
}

#[test]
fn the_stack_is_named_by_the_document_unless_told() {
    let files = compile_named("web-vm", &format!("  resources:\n{BUCKET}"))
        .unwrap()
        .artifact
        .files;
    assert!(files["stack.json"].contains("\"name\": \"web-vm\""));

    let err = compile_named("web_vm", &format!("  resources:\n{BUCKET}")).unwrap_err();
    assert_eq!(err.rule, "invalid-stack-name");

    let files = compile_named(
        "web_vm",
        &format!("  resources:\n{BUCKET}    - id: s\n      type: stack\n      params: {{name: web-vm}}\n"),
    )
    .unwrap()
    .artifact
    .files;
    assert!(files["stack.json"].contains("\"name\": \"web-vm\""));
    assert!(!files["template.yaml"].contains("stack"));
}

#[test]
fn stack_settings_are_checked() {
    let err = refusal(&format!(
        "  resources:\n{BUCKET}    - id: s\n      type: stack\n      params: {{capabilities: [CAPABILITY_EVERYTHING]}}\n"
    ));
    assert_eq!(err.rule, "invalid-param");
    let err = refusal(&format!(
        "  resources:\n{BUCKET}    - id: s\n      type: stack\n      params: {{profile: prod}}\n"
    ));
    assert_eq!(err.rule, "unsupported-param");
}
