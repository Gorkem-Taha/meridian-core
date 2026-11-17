//! The `meridian` command, run as a process against a real plugin.

#![cfg(unix)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const WORKFLOW: &str = r#"apiVersion: meridian/v1
kind: Infrastructure
metadata:
  name: network
spec:
  target: fake
  resources:
    - id: vm
      type: file
      params:
        content: 10.0.0.5
    - id: key
      type: file
      params:
        content: hunter2-very-secret
  outputs:
    - name: address
      value: "${vm.content}"
    - name: secret_key
      value: "${key.content}"
---
apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: web
spec:
  target: fake
  targetHost:
    from: infrastructure
    ref: network
    output: address
  resources:
    - id: motd
      type: file
      params:
        content: "welcome\n"
"#;

/// A scratch directory holding the workflow file, per test.
fn scratch(test: &str, workflow: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("site.yaml"), workflow).unwrap();
    dir
}

fn meridian(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_meridian"))
        .args(args)
        .current_dir(dir)
        .env("MERIDIAN_PLUGIN_DIR", meridian_target_fake::plugin_dir())
        .env_remove("MERIDIAN_STATE_DIR")
        .output()
        .expect("meridian runs")
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn validate_accepts_good_documents_and_points_at_the_problem_in_bad_ones() {
    let dir = scratch("validate", WORKFLOW);
    let ok = meridian(&dir, &["validate", "site.yaml"]);
    assert!(ok.status.success(), "{}", stderr(&ok));
    assert_eq!(stdout(&ok), "2 documents valid\n");

    // A field that does not exist, reported at its own line.
    let typo = WORKFLOW.replace("    - id: motd\n", "    - id: motd\n      colour: blue\n");
    assert_ne!(typo, WORKFLOW);
    fs::write(dir.join("site.yaml"), typo).unwrap();
    let bad = meridian(&dir, &["validate", "site.yaml"]);
    assert_eq!(bad.status.code(), Some(1));
    assert_eq!(stdout(&bad), "");
    assert_eq!(
        stderr(&bad),
        "error: site.yaml[doc 1]:34:7: spec.resources[0]: unknown field \"colour\"\n"
    );

    // A document that parses but points at one that does not exist.
    let dangling = WORKFLOW.replace("ref: network", "ref: nowhere");
    fs::write(dir.join("site.yaml"), dangling).unwrap();
    let bad = meridian(&dir, &["validate", "site.yaml"]);
    assert_eq!(bad.status.code(), Some(1));
    assert!(stderr(&bad).contains("nowhere"), "{}", stderr(&bad));
}

#[test]
fn plugins_lists_what_each_plugin_is_and_can_do() {
    let dir = scratch("plugins", WORKFLOW);
    let out = meridian(&dir, &["plugins"]);
    assert!(out.status.success(), "{}", stderr(&out));
    let text = stdout(&out);
    let binary = meridian_target_fake::plugin_path();
    assert_eq!(
        text,
        format!(
            "fake\n  category:  configuration-management\n  can:       compile, plan, apply, outputs\n  \
             resources: file\n  sdk:       0.1.0 (protocol 2)\n  binary:    {}\n",
            binary.display()
        )
    );
}

#[test]
fn compile_writes_each_documents_files_and_applies_nothing() {
    let dir = scratch("compile", WORKFLOW);
    let out = meridian(&dir, &["compile", "site.yaml", "--out", "build"]);
    assert!(out.status.success(), "{}", stderr(&out));
    assert_eq!(
        stdout(&out),
        "network (Infrastructure, target fake)\n  compiled 4 files into build/network\n\
         web (ResourceSet, target fake)\n  deferred: needs the output \"address\" of Infrastructure \"network\", \
         which has not been applied yet\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("build/network/files/vm")).unwrap(),
        "10.0.0.5"
    );
    assert_eq!(
        fs::read_to_string(dir.join("build/network/outputs/address")).unwrap(),
        "${vm.content}"
    );
    assert!(!dir.join(".meridian/state.json").exists());
}

#[test]
fn plan_then_apply_then_plan_tells_the_whole_story() {
    let dir = scratch("lifecycle", WORKFLOW);

    let plan = meridian(&dir, &["plan", "site.yaml"]);
    assert!(plan.status.success(), "{}", stderr(&plan));
    assert_eq!(
        stdout(&plan),
        "network (Infrastructure, target fake)\n\
         \x20 + create files/key: 19 bytes\n\
         \x20 + create files/vm: 8 bytes\n\
         \x20 + create outputs/address: 13 bytes\n\
         \x20 + create outputs/secret_key: 14 bytes\n\
         \x20 4 changes to make\n\
         web (ResourceSet, target fake)\n\
         \x20 deferred: needs the output \"address\" of Infrastructure \"network\", which has not been applied yet\n"
    );

    let apply = meridian(&dir, &["apply", "site.yaml"]);
    assert!(apply.status.success(), "{}", stderr(&apply));
    let text = stdout(&apply);
    assert!(text.contains("  applied 4 changes\n"), "{text}");
    assert!(text.contains("  output address = \"10.0.0.5\"\n"), "{text}");
    assert!(
        text.contains("  output secret_key = (sensitive)\n"),
        "{text}"
    );
    assert!(text.contains("web (ResourceSet, target fake on 10.0.0.5)\n  + create files/motd: 8 bytes\n  applied 1 change\n"), "{text}");
    // The tool's own output is on stderr, and the secret is in neither.
    let log = stderr(&apply);
    assert!(log.contains("  network | create files/vm\n"), "{log}");
    assert!(log.contains("  web ! inputs: "), "{log}");
    assert!(
        !text.contains("hunter2") && !log.contains("hunter2"),
        "{text}{log}"
    );
    assert_eq!(
        fs::read_to_string(dir.join(".meridian/resourceset/web/fake/current/files/motd")).unwrap(),
        "welcome\n"
    );

    let again = meridian(&dir, &["plan", "site.yaml"]);
    assert!(again.status.success());
    assert_eq!(
        stdout(&again),
        "network (Infrastructure, target fake)\n  in sync, nothing to change\n\
         web (ResourceSet, target fake on 10.0.0.5)\n  in sync, nothing to change\n"
    );
}

#[test]
fn a_failed_apply_exits_non_zero_and_says_which_stage_and_why() {
    let dir = scratch(
        "failure",
        &WORKFLOW.replace("content: 10.0.0.5", "content: FAIL"),
    );
    let out = meridian(&dir, &["apply", "site.yaml", "--state-dir", "state"]);
    assert_eq!(out.status.code(), Some(1));
    let text = stdout(&out);
    assert!(
        text.contains(
            "  FAILED: could not write files/vm (ran `fake-tool apply --password=***`, exit status 3)\n    disk on fire\n"
        ),
        "{text}"
    );
    assert!(
        text.ends_with("web (ResourceSet, target fake)\n  not run, because Infrastructure \"network\" failed\n"),
        "{text}"
    );
    assert!(
        dir.join("state/state.json").exists(),
        "--state-dir was not used"
    );
}

#[test]
fn a_missing_plugin_is_a_clear_failure_and_usage_errors_exit_2() {
    let dir = scratch(
        "no-plugin",
        &WORKFLOW.replace("target: fake", "target: terraform"),
    );
    let out = meridian(&dir, &["plan", "site.yaml"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(
        stdout(&out)
            .contains("  FAILED: no plugin provides target \"terraform\" (loaded targets: fake)\n"),
        "{}",
        stdout(&out)
    );

    assert_eq!(meridian(&dir, &["plan"]).status.code(), Some(2));
    assert_eq!(meridian(&dir, &["frobnicate"]).status.code(), Some(2));
    let missing = meridian(&dir, &["validate", "nope.yaml"]);
    assert_eq!(missing.status.code(), Some(1));
    assert!(
        stderr(&missing).contains("nope.yaml"),
        "{}",
        stderr(&missing)
    );
}
