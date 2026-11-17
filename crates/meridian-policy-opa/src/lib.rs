//! The policy engine for Open Policy Agent.
//!
//! `meridian-policy-opa <policy>...` reads one stage's input on standard
//! input, hands it and the policies to `opa eval`, and answers with what the
//! package `meridian` denies and warns about:
//!
//! ```json
//! {"deny": ["..."], "warn": ["..."]}
//! ```
//!
//! `deny` and `warn` in the package `meridian` is the convention Conftest
//! uses, so a policy written for one reads the same for the other. Each is a
//! set of messages; a message may also be an object with a `msg`.
//!
//! The program is `opa` on the `PATH`, or whatever `MERIDIAN_OPA` names.
//! The input goes to `opa` on its standard input and is never written to
//! disk.
//!
//! Anything that is not an answer is an error, and the engine exits 1: `opa`
//! missing, a policy that does not parse, policies with no package
//! `meridian` in them. Meridian reads an engine's failure as a denial.

use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};

use meridian_value::{decode_value, Value};

/// The variable that names the `opa` program.
pub const OPA_VARIABLE: &str = "MERIDIAN_OPA";

/// The package a policy's `deny` and `warn` are read from.
pub const PACKAGE: &str = "meridian";

/// Asks `opa` what the policies under `paths` say about `input`, and
/// returns the answer as the JSON Meridian reads.
pub fn evaluate(opa: &str, paths: &[PathBuf], input: &[u8]) -> Result<String, String> {
    if paths.is_empty() {
        return Err("no policy was given: name at least one file or directory".to_owned());
    }
    let mut command = Command::new(opa);
    command.args(["eval", "--format", "json", "--stdin-input"]);
    for path in paths {
        command.arg("--data").arg(path);
    }
    command.arg(format!("data.{PACKAGE}"));
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| {
            format!(
                "starting {opa}: {e}. Install Open Policy Agent, or name its program in \
                 {OPA_VARIABLE}"
            )
        })?;

    let mut stdin = child.stdin.take().expect("stdin was piped");
    let document = input.to_vec();
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&document);
    });
    let mut stderr = child.stderr.take().expect("stderr was piped");
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        let _ = stderr.read_to_string(&mut text);
        text
    });
    let mut stdout = Vec::new();
    let read = child
        .stdout
        .take()
        .expect("stdout was piped")
        .read_to_end(&mut stdout);
    let status = child
        .wait()
        .map_err(|e| format!("waiting for {opa}: {e}"))?;
    let _ = writer.join();
    let errors = errors.join().unwrap_or_default();
    read.map_err(|e| format!("reading what {opa} said: {e}"))?;

    if !status.success() {
        return Err(format!(
            "{opa} eval failed{}",
            why(&stdout, &errors).map_or_else(String::new, |why| format!(": {why}"))
        ));
    }
    answer(&stdout)
}

/// Why `opa` failed, from what it printed. With `--format json` it reports
/// errors as JSON on standard output; before it gets that far, as text on
/// standard error.
fn why(stdout: &[u8], stderr: &str) -> Option<String> {
    let reported = decode_value(stdout).ok().and_then(|value| {
        let errors = value.as_map()?.get("errors")?.as_list()?.to_vec();
        let messages: Vec<String> = errors
            .iter()
            .filter_map(|error| {
                let error = error.as_map()?;
                let message = error.get("message")?.as_str()?;
                let at = error
                    .get("location")
                    .and_then(Value::as_map)
                    .and_then(|location| {
                        let file = location.get("file")?.as_str()?;
                        match location.get("row") {
                            Some(Value::Int(row)) => Some(format!("{file}:{row}: ")),
                            _ => Some(format!("{file}: ")),
                        }
                    })
                    .unwrap_or_default();
                Some(format!("{at}{message}"))
            })
            .collect();
        (!messages.is_empty()).then(|| messages.join("; "))
    });
    reported.or_else(|| {
        stderr
            .lines()
            .rev()
            .find(|line| !line.trim().is_empty())
            .map(|line| line.trim().to_owned())
    })
}

/// Turns what `opa eval` printed into Meridian's answer.
pub fn answer(stdout: &[u8]) -> Result<String, String> {
    let value = decode_value(stdout).map_err(|e| format!("opa did not answer in JSON: {e}"))?;
    let package = value
        .as_map()
        .and_then(|top| top.get("result"))
        .and_then(Value::as_list)
        .and_then(|results| results.first())
        .and_then(Value::as_map)
        .and_then(|result| result.get("expressions"))
        .and_then(Value::as_list)
        .and_then(|expressions| expressions.first())
        .and_then(Value::as_map)
        .and_then(|expression| expression.get("value"));
    // An undefined `data.meridian` is no result at all. Policies that say
    // nothing in the package are far more likely to be misnamed than to
    // mean "allow everything", so that is not read as a yes.
    let Some(package) = package.and_then(Value::as_map) else {
        return Err(format!(
            "the policies define nothing in the package `{PACKAGE}`; `deny` and `warn` are \
             read from `package {PACKAGE}`"
        ));
    };
    let messages = |rule: &str| -> Result<Value, String> {
        match package.get(rule) {
            None => Ok(Value::List(Vec::new())),
            Some(Value::List(messages)) => Ok(Value::List(messages.clone())),
            Some(other) => Err(format!(
                "`{rule}` in the package `{PACKAGE}` is {}, and it has to be a set of \
                 messages: write `{rule} contains msg if {{ ... }}`",
                other.describe()
            )),
        }
    };
    let answer: std::collections::BTreeMap<String, Value> = [
        ("deny".to_owned(), messages("deny")?),
        ("warn".to_owned(), messages("warn")?),
    ]
    .into_iter()
    .collect();
    Ok(Value::Map(answer).canonical())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_package_is_read_into_deny_and_warn() {
        let said = br#"{"result":[{"expressions":[{"value":{"deny":["no replace in prod"],"warn":[{"msg":"big"}],"helper":true},"text":"data.meridian"}]}]}"#;
        assert_eq!(
            answer(said).unwrap(),
            r#"{"deny":["no replace in prod"],"warn":[{"msg":"big"}]}"#
        );
        let only_warn = br#"{"result":[{"expressions":[{"value":{"warn":[]}}]}]}"#;
        assert_eq!(answer(only_warn).unwrap(), r#"{"deny":[],"warn":[]}"#);
    }

    #[test]
    fn no_package_and_a_rule_of_the_wrong_shape_are_errors() {
        let undefined = answer(b"{}").unwrap_err();
        assert!(
            undefined.contains("define nothing in the package `meridian`"),
            "{undefined}"
        );
        let boolean =
            answer(br#"{"result":[{"expressions":[{"value":{"deny":true}}]}]}"#).unwrap_err();
        assert!(boolean.contains("has to be a set of messages"), "{boolean}");
        let garbage = answer(b"not json").unwrap_err();
        assert!(garbage.contains("did not answer in JSON"), "{garbage}");
    }

    #[test]
    fn a_failure_is_explained_from_json_or_from_standard_error() {
        let json = br#"{"errors":[{"message":"rego_parse_error: unexpected token","code":"rego_parse_error","location":{"file":"p.rego","row":3,"col":1}}]}"#;
        assert_eq!(
            why(json, "").unwrap(),
            "p.rego:3: rego_parse_error: unexpected token"
        );
        assert_eq!(
            why(b"", "x\nerror: bad flag\n\n").unwrap(),
            "error: bad flag"
        );
        assert_eq!(why(b"", ""), None);
    }
}
