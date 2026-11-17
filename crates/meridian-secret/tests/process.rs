//! What the process is made into before it reads a secret, what a program
//! it starts inherits, and what a panic prints.
//!
//! Hardening changes the whole process, so each check runs in a child: this
//! test binary started again with a variable that says which part to play.

#![cfg(unix)]

use std::process::{Command, Stdio};

use meridian_secret::{from_env, harden_process, hardening, SecretValue};

const ROLE: &str = "MERIDIAN_SECRET_TEST_ROLE";
const TOKEN: &str = "MERIDIAN_SECRET_TEST_TOKEN";
const VALUE: &str = "kq7Zp2-not-a-real-token-9fXw";

/// Runs this test binary again as `role`, running only `test`.
fn child(role: &str, test: &str) -> std::process::Output {
    Command::new(std::env::current_exe().unwrap())
        .args([test, "--exact", "--nocapture", "--test-threads=1"])
        .env(ROLE, role)
        .env(TOKEN, VALUE)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn role() -> Option<String> {
    std::env::var(ROLE).ok()
}

fn text(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn hardening_turns_off_core_dumps_and_makes_the_process_undumpable() {
    if role().as_deref() == Some("harden") {
        let achieved = harden_process();
        assert!(achieved.core_dumps_off, "{achieved:?}");
        // What was achieved is what the system now reports.
        assert_eq!(hardening(), achieved);
        if cfg!(target_os = "linux") {
            assert!(achieved.undumpable, "{achieved:?}");
            let status = std::fs::read_to_string("/proc/self/limits").unwrap();
            let core = status
                .lines()
                .find(|l| l.starts_with("Max core file size"))
                .unwrap()
                .to_owned();
            let soft = core.split_whitespace().nth(4).unwrap().to_owned();
            assert_eq!(soft, "0", "{core}");
        }
        println!("hardened");
        return;
    }
    let output = child(
        "harden",
        "hardening_turns_off_core_dumps_and_makes_the_process_undumpable",
    );
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("hardened"), "{}", text(&output));
}

#[test]
fn a_secret_read_from_the_environment_is_not_inherited_by_what_is_started_next() {
    if role().as_deref() == Some("env") {
        let token = from_env(TOKEN).unwrap();
        assert!(token.expose(|bytes| bytes == VALUE.as_bytes()));
        assert!(std::env::var_os(TOKEN).is_none(), "the variable is removed");
        // A program started after the secret was read: its whole
        // environment, as it sees it.
        let seen = Command::new("env").output().unwrap();
        let seen = String::from_utf8_lossy(&seen.stdout).into_owned();
        assert!(!seen.contains(VALUE), "the child was handed the secret");
        assert!(!seen.contains(TOKEN), "the child was handed the variable");
        println!("not inherited");
        return;
    }
    let output = child(
        "env",
        "a_secret_read_from_the_environment_is_not_inherited_by_what_is_started_next",
    );
    assert!(output.status.success(), "{}", text(&output));
    assert!(text(&output).contains("not inherited"), "{}", text(&output));
}

/// Something that holds a secret the way a run's structures do.
#[derive(Debug)]
#[allow(dead_code)]
struct Stage {
    name: String,
    password: SecretValue,
}

#[test]
// Unwrapping an error is the point: it is how a value usually reaches a panic.
#[allow(clippy::unnecessary_literal_unwrap)]
fn a_panic_while_a_secret_is_held_does_not_print_it() {
    if let Some(role) = role().filter(|r| r.starts_with("panic")) {
        let stage = Stage {
            name: "db".to_owned(),
            password: from_env(TOKEN).unwrap(),
        };
        match role.as_str() {
            // The ways a value usually reaches a panic message.
            "panic-debug" => panic!("stage failed: {stage:?}"),
            "panic-display" => panic!("bad password {}", stage.password),
            "panic-unwrap" => {
                let outcome: Result<(), &Stage> = Err(&stage);
                outcome.unwrap();
            }
            "panic-assert" => assert_eq!(format!("{:?}", stage.password), "x"),
            "panic-expect" => {
                let outcome: Result<(), SecretValue> = Err(stage.password);
                outcome.expect("the password was accepted");
            }
            _ => unreachable!(),
        }
        return;
    }
    for role in [
        "panic-debug",
        "panic-display",
        "panic-unwrap",
        "panic-assert",
        "panic-expect",
    ] {
        let output = child(role, "a_panic_while_a_secret_is_held_does_not_print_it");
        let printed = text(&output);
        assert!(!output.status.success(), "{role} was meant to panic");
        assert!(printed.contains("panicked"), "{role}: {printed}");
        assert!(printed.contains("(sensitive)"), "{role}: {printed}");
        assert!(
            !printed.contains(VALUE),
            "{role} printed the secret:\n{printed}"
        );
    }
}
