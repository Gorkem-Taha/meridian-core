//! Secrets through the real command: a secret decides a `when`, and its
//! value is in nothing the run prints or leaves on disk. Not in the plan, not
//! in what the tool logged, not in an error, not in the state directory, not
//! in what `compile --out` writes.

#![cfg(unix)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const PASSWORD: &str = "kq7Zp2-not-a-real-password-9fXw";
const TOKEN: &str = "tok_Zx81-not-a-real-token";

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("secrets-{test}"));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs the command with `PASSWORD` in the environment variable the
/// documents read, and with any further variables.
fn meridian_with(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_meridian"));
    command
        .args(args)
        .current_dir(dir)
        .env("MERIDIAN_PLUGIN_DIR", meridian_target_fake::plugin_dir())
        .env_remove("MERIDIAN_STATE_DIR")
        .env_remove("SITE_DB_PASSWORD");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().expect("meridian runs")
}

fn meridian(dir: &Path, args: &[&str]) -> Output {
    meridian_with(dir, args, &[("SITE_DB_PASSWORD", PASSWORD)])
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Every file under `dir`, with its content.
fn files_under(dir: &Path) -> Vec<(PathBuf, Vec<u8>)> {
    let mut found = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return found;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            found.extend(files_under(&path));
        } else if let Ok(bytes) = fs::read(&path) {
            found.push((path, bytes));
        }
    }
    found
}

fn holds(bytes: &[u8], needle: &str) -> bool {
    bytes
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

/// Asserts no secret is in what the run printed or in any file under `dir`
/// other than the ones the test wrote there itself.
fn nothing_leaked(output: &Output, dir: &Path, except: &[&str]) {
    for secret in [PASSWORD, TOKEN] {
        assert!(
            !stdout(output).contains(secret),
            "stdout:\n{}",
            stdout(output)
        );
        assert!(
            !stderr(output).contains(secret),
            "stderr:\n{}",
            stderr(output)
        );
        for (path, bytes) in files_under(dir) {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if except.contains(&name.as_str()) {
                continue;
            }
            assert!(!holds(&bytes, secret), "{} holds a secret", path.display());
        }
    }
}

/// A document whose conditions read three secrets, one from each provider.
/// No param holds one, so every stage can run.
const BY_SECRET: &str = r#"apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: site
spec:
  target: fake
  targetHost:
    from: inline
    host: web01
  secrets:
    - name: db_password
      from: env
      key: SITE_DB_PASSWORD
    - name: token
      from: file
      path: token.txt
    - name: flag
      from: exec
      command: [printf, "%s", "on"]
    - name: extra
      from: env
      key: SITE_NOT_SET_ANYWHERE
      optional: true
  conditions:
    has_extra: secret.extra != null
    strong: secret.db_password != 'changeme' && secret.db_password != ''
  resources:
    - id: motd
      type: file
      params:
        content: "welcome\n"
    - id: strong
      type: file
      when: cond.strong
      params:
        content: "the password is not the default\n"
    - id: flagged
      type: file
      # The token is compared in an encoded form, so that this file does not
      # hold it and the tests can search every file for it.
      when: secret.flag == 'on' && base64Encode(secret.token) == 'dG9rX1p4ODEtbm90LWEtcmVhbC10b2tlbg=='
      params:
        content: "flag on\n"
    - id: extra
      type: file
      when: cond.has_extra
      params:
        content: "extra\n"
"#;

fn by_secret(test: &str) -> PathBuf {
    let dir = scratch(test);
    fs::write(dir.join("site.yaml"), BY_SECRET).unwrap();
    let token = dir.join("token.txt");
    fs::write(&token, format!("{TOKEN}\n")).unwrap();
    fs::set_permissions(&token, fs::Permissions::from_mode(0o600)).unwrap();
    dir
}

#[test]
fn a_secret_decides_a_when_and_is_in_nothing_the_run_prints_or_writes() {
    let dir = by_secret("decides");

    let plan = meridian(&dir, &["plan", "site.yaml"]);
    assert!(plan.status.success(), "{}{}", stdout(&plan), stderr(&plan));
    let text = stdout(&plan);
    assert!(text.contains("+ create files/motd"), "{text}");
    assert!(text.contains("+ create files/strong"), "{text}");
    assert!(text.contains("+ create files/flagged"), "{text}");
    assert!(!text.contains("files/extra"), "{text}");
    // What a condition removed is reported in the document's own words.
    assert!(text.contains("resource \"extra\""), "{text}");
    assert!(text.contains("cond.has_extra"), "{text}");
    nothing_leaked(&plan, &dir, &["token.txt"]);

    let apply = meridian(&dir, &["apply", "site.yaml"]);
    assert!(
        apply.status.success(),
        "{}{}",
        stdout(&apply),
        stderr(&apply)
    );
    assert!(
        stdout(&apply).contains("applied 3 changes"),
        "{}",
        stdout(&apply)
    );
    nothing_leaked(&apply, &dir, &["token.txt"]);
    // The run left a state directory behind, and it was searched.
    assert!(dir.join(".meridian/state.json").exists());
    assert!(files_under(&dir.join(".meridian")).len() >= 4);

    // The other answer, from another value of the same secret.
    let weak = meridian_with(
        &dir,
        &["plan", "site.yaml"],
        &[("SITE_DB_PASSWORD", "changeme")],
    );
    assert!(weak.status.success(), "{}", stderr(&weak));
    let text = stdout(&weak);
    assert!(text.contains("resource \"strong\""), "{text}");
    assert!(text.contains("cond.strong"), "{text}");
    assert!(!text.contains("changeme"), "{text}");
}

#[test]
fn a_secret_that_cannot_be_fetched_stops_the_stage_and_names_only_where_it_was_looked_for() {
    let dir = by_secret("missing");
    // The environment variable is not set.
    let out = meridian_with(&dir, &["apply", "site.yaml"], &[]);
    assert_eq!(out.status.code(), Some(1), "{}", stdout(&out));
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains(
            "spec.secrets: secret \"db_password\" (from: env): the environment variable \
             SITE_DB_PASSWORD is not set"
        ),
        "{text}"
    );
    assert!(!dir
        .join(".meridian/resourceset/site/fake/current/files/motd")
        .exists());

    // The file is readable by others.
    fs::set_permissions(dir.join("token.txt"), fs::Permissions::from_mode(0o644)).unwrap();
    let out = meridian(&dir, &["plan", "site.yaml"]);
    assert_eq!(out.status.code(), Some(1));
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains("secret \"token\" (from: file)") && text.contains("has mode 0644"),
        "{text}"
    );
    nothing_leaked(&out, &dir, &["token.txt"]);

    // The command fails, having printed something.
    fs::set_permissions(dir.join("token.txt"), fs::Permissions::from_mode(0o600)).unwrap();
    let failing = BY_SECRET.replace(
        "command: [printf, \"%s\", \"on\"]",
        "command: [sh, -c, \"echo tok_Zx81-not-a-real-token; exit 4\"]",
    );
    fs::write(dir.join("site.yaml"), failing).unwrap();
    let out = meridian(&dir, &["plan", "site.yaml"]);
    assert_eq!(out.status.code(), Some(1));
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains("secret \"flag\" (from: exec): the command sh exited with status 4"),
        "{text}"
    );
    // Neither what it printed nor its arguments are repeated.
    assert!(!text.contains("exit 4") && !text.contains(TOKEN), "{text}");
}

#[test]
fn validate_and_compile_never_fetch_a_secret() {
    let dir = by_secret("unfetched");
    // Nothing the document needs exists: no variable, no file.
    fs::remove_file(dir.join("token.txt")).unwrap();

    let out = meridian_with(&dir, &["validate", "site.yaml"], &[]);
    assert!(out.status.success(), "{}", stderr(&out));

    // The document's conditions read secrets, and compile does not fetch
    // one, so it says so instead of guessing.
    let out = meridian_with(&dir, &["compile", "site.yaml"], &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = format!("{}{}", stdout(&out), stderr(&out));
    assert!(
        text.contains(
            "is not fetched here: a secret is fetched by plan and apply, never by \
                       validate or compile"
        ),
        "{text}"
    );
}

/// A document that places a secret in a param.
const IN_A_PARAM: &str = concat!(
    r#"apiVersion: meridian/v1
kind: ResourceSet
metadata:
  name: site
spec:
  target: fake
  targetHost:
    from: inline
    host: web01
  secrets:
    - name: db_password
      from: env
      key: SITE_DB_PASSWORD
  resources:
    - id: conf
      type: file
      params:
        content: "password=$"#,
    r#"{{ secret.db_password }}\n"
    - id: url
      type: file
      params:
        content: "postgres://app:$"#,
    r#"{{ urlEncode(secret.db_password) }}@db/app"
"#
);

#[test]
fn compile_writes_the_name_of_a_secret_in_a_param_and_never_its_value() {
    let dir = scratch("placeholder");
    fs::write(dir.join("site.yaml"), IN_A_PARAM).unwrap();
    // The variable is set, and compile still does not read it.
    let out = meridian(&dir, &["compile", "site.yaml", "--out", "out"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    nothing_leaked(&out, &dir, &[]);
    let written: Vec<String> = files_under(&dir.join("out"))
        .into_iter()
        .map(|(_, bytes)| String::from_utf8_lossy(&bytes).into_owned())
        .collect();
    assert!(
        written
            .iter()
            .any(|text| text == "password=(secret db_password)\n"),
        "{written:#?}"
    );
    assert!(
        written
            .iter()
            .any(|text| text == "postgres://app:urlEncode((secret db_password))@db/app"),
        "{written:#?}"
    );
}

#[test]
fn plan_and_apply_stop_before_a_secret_in_a_param_would_be_written_to_a_file() {
    let dir = scratch("refused");
    fs::write(dir.join("site.yaml"), IN_A_PARAM).unwrap();
    for command in ["plan", "apply"] {
        let out = meridian(&dir, &[command, "site.yaml"]);
        assert_eq!(out.status.code(), Some(1), "{command}: {}", stdout(&out));
        let text = format!("{}{}", stdout(&out), stderr(&out));
        assert!(
            text.contains(
                "resource \"conf\": params.content holds a secret, and target \"fake\" cannot be \
                 handed a secret yet"
            ),
            "{command}: {text}"
        );
        nothing_leaked(&out, &dir, &[]);
        // Nothing was compiled into the work directory.
        for (path, bytes) in files_under(&dir.join(".meridian")) {
            assert!(!holds(&bytes, "password="), "{}", path.display());
        }
    }
}

#[test]
fn a_secret_read_from_the_environment_is_not_handed_to_the_plugin() {
    // The fake tool logs its inputs; a plugin is a child process, and the
    // variable was removed from the environment before it was started.
    let dir = by_secret("not-inherited");
    let probe = BY_SECRET.replace(
        "command: [printf, \"%s\", \"on\"]",
        "command: [sh, -c, \"env | grep -c SITE_DB_PASSWORD; true\"]",
    );
    // `flag` now holds how many times the variable appears in the
    // environment of a program started after `db_password` was fetched.
    let probe = probe.replace("secret.flag == 'on'", "trim(secret.flag) == '0'");
    fs::write(dir.join("site.yaml"), probe).unwrap();
    let out = meridian(&dir, &["plan", "site.yaml"]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert!(
        stdout(&out).contains("+ create files/flagged"),
        "the variable was still in the environment:\n{}",
        stdout(&out)
    );
    nothing_leaked(&out, &dir, &["token.txt"]);
}

#[test]
fn a_document_left_out_by_its_own_when_fetches_nothing() {
    let dir = scratch("left-out");
    let doc = BY_SECRET.replace(
        "  secrets:\n",
        "  when: data.enabled ?? false\n  secrets:\n",
    );
    fs::write(dir.join("site.yaml"), doc).unwrap();
    // No token file and no variable: fetching would fail.
    let out = meridian_with(&dir, &["plan", "site.yaml"], &[]);
    assert!(out.status.success(), "{}{}", stdout(&out), stderr(&out));
    assert!(stdout(&out).contains("skipped"), "{}", stdout(&out));
}
