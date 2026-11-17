//! `spec.imports`: values a document is handed from another document.

use meridian_ir::{parse, Bundle, Document, IMPORT_FROM_ARTIFACT, IMPORT_FROM_OUTPUT};

const BOOTSTRAP: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata: {name: bootstrap}
spec:
  target: cloud-init
  targetHost: {from: inline, host: web-1}
  resources:
    - id: motd
      type: file
"#;

const NETWORK: &str = r#"apiVersion: meridian/v1
kind: Infrastructure
metadata: {name: network}
spec:
  target: terraform
  resources:
    - id: vpc
      type: aws_vpc
  outputs:
    - name: vpc_id
      value: "${aws_vpc.vpc.id}"
"#;

/// A machine that imports whatever `imports` says.
fn machine(imports: &str) -> String {
    format!(
        "apiVersion: meridian/v1\nkind: Infrastructure\nmetadata: {{name: machine}}\nspec:\n  \
         target: terraform\n  imports:\n{imports}  resources:\n    - id: web\n      type: \
         aws_instance\n"
    )
}

const GOOD: &str = "    - name: user_data\n      from: artifact\n      ref: bootstrap\n      file: \
                    user-data\n      sensitive: true\n    - name: vpc_id\n      from: output\n      \
                    ref: network\n      output: vpc_id\n";

fn bundle(documents: &[&str]) -> Bundle {
    let text = documents.join("---\n");
    Bundle::new(parse("site.yaml", text.as_bytes()).unwrap_or_else(|e| panic!("parse: {e}")))
}

fn errors(documents: &[&str]) -> String {
    bundle(documents).validate().unwrap_err().to_string()
}

#[test]
fn imports_are_read_on_either_kind_of_document() {
    let bundle = bundle(&[BOOTSTRAP, NETWORK, &machine(GOOD)]);
    bundle.validate().unwrap();
    let machine = &bundle.documents[2];
    let imports = machine.imports();
    assert_eq!(imports.len(), 2);
    assert_eq!(imports[0].name, "user_data");
    assert_eq!(imports[0].from, IMPORT_FROM_ARTIFACT);
    assert_eq!(imports[0].r#ref, "bootstrap");
    assert_eq!(imports[0].file, "user-data");
    assert!(imports[0].sensitive);
    // Lines count through the whole file, not the document.
    assert_eq!(imports[0].pos.line, 29);
    assert_eq!(imports[1].from, IMPORT_FROM_OUTPUT);
    assert_eq!(imports[1].output, "vpc_id");
    assert!(!imports[1].sensitive);
    assert!(bundle.documents[0].imports().is_empty());

    // A ResourceSet may import too.
    let configured = BOOTSTRAP.replace(
        "  resources:",
        "  imports:\n    - name: vpc\n      from: output\n      ref: network\n      output: \
         vpc_id\n  resources:",
    );
    let bundle = self::bundle(&[&configured, NETWORK]);
    bundle.validate().unwrap();
    assert_eq!(bundle.documents[0].imports()[0].name, "vpc");

    // What a document waits on: its imports' documents, once each.
    let both = self::bundle(&[BOOTSTRAP, NETWORK, &self::machine(GOOD)]);
    let needs: Vec<&str> = both
        .needs(&both.documents[2])
        .iter()
        .map(|d| d.name())
        .collect();
    assert_eq!(needs, ["bootstrap", "network"]);
}

#[test]
fn a_malformed_import_is_a_parse_error_with_its_position() {
    let wrong_type = machine("    - name: a\n      from: artifact\n      ref: bootstrap\n      file: f\n      sensitive: maybe\n");
    let err = parse("site.yaml", wrong_type.as_bytes())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("spec.imports[0].sensitive: expected true or false"),
        "{err}"
    );
    assert!(err.contains("site.yaml:11:"), "{err}");

    let unknown =
        machine("    - name: a\n      from: artifact\n      ref: bootstrap\n      path: f\n");
    let err = parse("site.yaml", unknown.as_bytes())
        .unwrap_err()
        .to_string();
    assert!(
        err.contains(r#"spec.imports[0]: unknown field "path""#),
        "{err}"
    );

    let not_a_list = machine("    name: a\n");
    let err = parse("site.yaml", not_a_list.as_bytes())
        .unwrap_err()
        .to_string();
    assert!(err.contains("spec.imports: expected a list"), "{err}");
}

#[test]
fn every_way_an_import_can_be_wrong_is_said() {
    let cases: &[(&str, &str)] = &[
        (
            "    - from: artifact\n      ref: bootstrap\n      file: user-data\n",
            "spec.imports[0].name: missing required field",
        ),
        (
            "    - name: user-data\n      from: artifact\n      ref: bootstrap\n      file: user-data\n",
            r#"spec.imports[0].name: "user-data" cannot name an import"#,
        ),
        (
            "    - name: 1st\n      from: artifact\n      ref: bootstrap\n      file: user-data\n",
            "it must start with a letter or an underscore",
        ),
        (
            "    - name: a\n      from: artifact\n      ref: bootstrap\n      file: user-data\n    - name: a\n      from: output\n      ref: network\n      output: vpc_id\n",
            r#"spec.imports[1].name: duplicate import name "a" (first declared at site.yaml[doc 2]:29:7)"#,
        ),
        (
            "    - name: a\n      ref: bootstrap\n      file: user-data\n",
            "spec.imports[0].from: missing required field",
        ),
        (
            "    - name: a\n      from: state\n      ref: bootstrap\n",
            r#"spec.imports[0].from: unknown source "state" (expected "artifact" or "output")"#,
        ),
        (
            "    - name: a\n      from: artifact\n      ref: bootstrap\n",
            "spec.imports[0].file: missing required field for from: artifact",
        ),
        (
            "    - name: a\n      from: artifact\n      ref: bootstrap\n      file: user-data\n      output: x\n",
            "spec.imports[0].output: does not apply to from: artifact, which takes `file`",
        ),
        (
            "    - name: a\n      from: output\n      ref: network\n",
            "spec.imports[0].output: missing required field for from: output",
        ),
        (
            "    - name: a\n      from: output\n      ref: network\n      output: vpc_id\n      file: x\n",
            "spec.imports[0].file: does not apply to from: output, which takes `output`",
        ),
        (
            "    - name: a\n      from: artifact\n      file: user-data\n",
            "spec.imports[0].ref: missing required field",
        ),
        (
            "    - name: a\n      from: artifact\n      ref: absent\n      file: user-data\n",
            r#"spec.imports[0].ref: no document named "absent" is loaded"#,
        ),
        (
            "    - name: a\n      from: output\n      ref: absent\n      output: x\n",
            r#"spec.imports[0].ref: no Infrastructure document named "absent" is loaded"#,
        ),
        (
            "    - name: a\n      from: output\n      ref: bootstrap\n      output: x\n",
            r#"spec.imports[0].ref: "bootstrap" is a ResourceSet, and only an Infrastructure document declares outputs"#,
        ),
        (
            "    - name: a\n      from: output\n      ref: network\n      output: subnet\n",
            r#"spec.imports[0].output: Infrastructure "network" declares no output "subnet""#,
        ),
        (
            "    - name: a\n      from: artifact\n      ref: machine\n      file: main.tf.json\n",
            "spec.imports[0].ref: a document cannot import from itself",
        ),
    ];
    for (imports, want) in cases {
        let err = errors(&[BOOTSTRAP, NETWORK, &machine(imports)]);
        assert!(
            err.contains(want),
            "for:\n{imports}\nwant {want:?} in:\n{err}"
        );
    }
}

#[test]
fn a_name_two_documents_share_cannot_be_imported_from_as_an_artifact() {
    let twin = BOOTSTRAP.replace("name: bootstrap", "name: network");
    let imports = "    - name: a\n      from: artifact\n      ref: network\n      file: f\n";
    let err = errors(&[&twin, NETWORK, &machine(imports)]);
    assert!(
        err.contains(
            r#"both an Infrastructure and a ResourceSet document are named "network", so which one's artifact is meant cannot be told; rename one of them"#
        ),
        "{err}"
    );
    // An output can only be an Infrastructure document's, so that is not
    // ambiguous.
    let imports = "    - name: a\n      from: output\n      ref: network\n      output: vpc_id\n";
    bundle(&[&twin, NETWORK, &machine(imports)])
        .validate()
        .unwrap();
}

#[test]
fn documents_that_wait_on_each_other_are_refused() {
    // The machine is created with the bootstrap's user-data, and the
    // bootstrap's host is the machine's address: neither can go first.
    let bootstrap = BOOTSTRAP.replace(
        "targetHost: {from: inline, host: web-1}",
        "targetHost: {from: infrastructure, ref: machine, output: address}",
    );
    let machine = machine(
        "    - name: user_data\n      from: artifact\n      ref: bootstrap\n      file: user-data\n",
    ) + "  outputs:\n    - name: address\n      value: x\n";
    let err = errors(&[&bootstrap, &machine]);
    assert!(
        err.contains(
            "these documents wait on each other, so none of them can be applied first: \
             ResourceSet \"bootstrap\" needs Infrastructure \"machine\" needs ResourceSet \
             \"bootstrap\""
        ),
        "{err}"
    );
    assert!(err.contains("site.yaml:1:1"), "{err}");

    // Two Infrastructure documents that import each other's outputs.
    let a = NETWORK.replace(
        "  resources:",
        "  imports:\n    - name: peer\n      from: output\n      ref: other\n      output: id\n  \
         resources:",
    );
    let b = NETWORK
        .replace("name: network", "name: other")
        .replace("name: vpc_id", "name: id")
        .replace(
            "  resources:",
            "  imports:\n    - name: peer\n      from: output\n      ref: network\n      output: \
             vpc_id\n  resources:",
        );
    let err = errors(&[&a, &b]);
    assert!(err.contains("wait on each other"), "{err}");
    assert!(
        err.contains("Infrastructure \"network\" needs Infrastructure \"other\" needs Infrastructure \"network\""),
        "{err}"
    );
}

#[test]
fn a_document_without_imports_is_unchanged() {
    let bundle = bundle(&[NETWORK]);
    bundle.validate().unwrap();
    match &bundle.documents[0] {
        Document::Infrastructure(doc) => assert!(doc.spec.imports.is_empty()),
        Document::ResourceSet(_) => unreachable!(),
    }
}
