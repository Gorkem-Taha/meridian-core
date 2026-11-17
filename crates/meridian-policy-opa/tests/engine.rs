//! The engine program, run as Meridian runs it, against a stand-in for
//! `opa` that checks how it was called and answers as `opa eval` does.
//! `tests/real.rs` does the same against the real thing.

#![cfg(unix)]

use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

/// Answers like `opa eval --format json`: the arguments it was given are
/// kept beside it, and what it says is chosen by the input.
const OPA: &str = r#"#!/bin/sh
here=$(dirname "$0")
printf '%s\n' "$*" > "$here/args"
input=$(cat)
case "$input" in
  *undefined*) echo '{}' ;;
  *broken*)
    echo '{"errors":[{"message":"rego_parse_error: unexpected ident token","code":"rego_parse_error","location":{"file":"p.rego","row":3,"col":1}}]}'
    exit 2 ;;
  *silent*) exit 1 ;;
  *) echo '{"result":[{"expressions":[{"value":{"deny":["web: motd would be a delete in prod"],"warn":[],"allowed_modes":["0644"]},"text":"data.meridian","location":{"row":1,"col":1}}]}]}' ;;
esac
"#;

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("engine")
        .join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let opa = dir.join("opa");
    fs::write(&opa, OPA).unwrap();
    fs::set_permissions(&opa, fs::Permissions::from_mode(0o755)).unwrap();
    dir
}

fn engine(opa: &Path, policies: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_meridian-policy-opa"))
        .args(policies)
        .env("MERIDIAN_OPA", opa)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    child.wait_with_output().unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn the_package_is_evaluated_over_the_input_and_answered_as_deny_and_warn() {
    let dir = scratch("answer");
    let output = engine(
        &dir.join("opa"),
        &["policies", "more.rego"],
        r#"{"stage":{}}"#,
    );
    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "{\"deny\":[\"web: motd would be a delete in prod\"],\"warn\":[]}\n"
    );
    assert_eq!(
        fs::read_to_string(dir.join("args")).unwrap(),
        "eval --format json --stdin-input --data policies --data more.rego data.meridian\n"
    );
}

#[test]
fn anything_that_is_not_an_answer_is_a_failure_that_says_why() {
    let dir = scratch("failures");
    let opa = dir.join("opa");

    let undefined = engine(&opa, &["p"], "undefined");
    assert_eq!(undefined.status.code(), Some(1));
    assert!(undefined.stdout.is_empty());
    assert!(
        stderr(&undefined).contains("the policies define nothing in the package `meridian`"),
        "{}",
        stderr(&undefined)
    );

    let broken = engine(&opa, &["p"], "broken");
    assert_eq!(broken.status.code(), Some(1));
    assert!(
        stderr(&broken).contains("eval failed: p.rego:3: rego_parse_error: unexpected ident token"),
        "{}",
        stderr(&broken)
    );

    let silent = engine(&opa, &["p"], "silent");
    assert_eq!(silent.status.code(), Some(1));
    assert!(
        stderr(&silent).trim_end().ends_with("eval failed"),
        "{}",
        stderr(&silent)
    );

    let no_policy = engine(&opa, &[], "{}");
    assert_eq!(no_policy.status.code(), Some(1));
    assert!(
        stderr(&no_policy).contains("no policy was given"),
        "{}",
        stderr(&no_policy)
    );

    let no_opa = engine(&dir.join("absent"), &["p"], "{}");
    assert_eq!(no_opa.status.code(), Some(1));
    assert!(
        stderr(&no_opa).contains("Install Open Policy Agent, or name its program in MERIDIAN_OPA"),
        "{}",
        stderr(&no_opa)
    );
}
