//! The running half, against a stand-in for `docker`.

#![cfg(unix)]

use std::fs;
use std::path::Path;

use meridian_plugin_testkit::{artifact, scratch, stand_in, KeptLog};
use meridian_sdk::{Artifact, RunContext, Runner, Value};
use meridian_target_docker::DockerRunner;

const TMP: &str = env!("CARGO_TARGET_TMPDIR");

fn compiled(host: &str) -> Artifact {
    let project = r#"{"name":"shop","host":"HOST","context":"","services":["app","db"],
        "outputs":[
          {"name":"url","parts":[{"text":"http://web01:"},{"service":"app","port":"80"}]},
          {"name":"state","parts":[{"service":"db","inspect":".State.Status"}]},
          {"name":"restarts","parts":[{"service":"db","inspect":".RestartCount"}]}]}"#
        .replace("HOST", host);
    artifact(&[("compose.yaml", "name: shop\n"), ("project.json", &project)])
}

fn context(dir: &Path) -> RunContext {
    RunContext {
        document: "shop".into(),
        work_dir: dir.join("work"),
        ..RunContext::default()
    }
}

/// A stand-in that appends each call to `calls` and answers by what the call
/// holds.
fn runner(dir: &Path, cases: &str) -> DockerRunner {
    let program = stand_in(
        dir,
        "docker",
        &format!(
            "echo \"$@\" >> \"{dir}/calls\"\ncase \"$*\" in\n{cases}\nesac",
            dir = dir.display()
        ),
    );
    DockerRunner {
        program: program.to_str().unwrap().to_owned(),
        extra_args: Vec::new(),
    }
}

fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("calls"))
        .unwrap()
        .lines()
        .map(|l| {
            l.replace(
                "compose --project-name shop --file compose.yaml ",
                "compose ",
            )
        })
        .collect()
}

// Compose reports on standard error.
const DRY_RUN: &str = r#"*--dry-run*) cat >&2 <<'END'
DRY-RUN MODE -  Network shop_backend  Creating
DRY-RUN MODE -  Network shop_backend  Created
DRY-RUN MODE -  Container shop-db-1  Running
DRY-RUN MODE -  Container shop-app-1  Recreate
DRY-RUN MODE -  Container shop-app-1  Recreated
DRY-RUN MODE -  Container shop-app-1  Starting
DRY-RUN MODE -  Container shop-app-1  Started
DRY-RUN MODE -  Container shop-old-1  Removing
DRY-RUN MODE -  Container shop-old-1  Removed
END
;;"#;

const UP: &str = r#"*" up "*) cat >&2 <<'END'
 Volume shop_data  Creating
 Volume shop_data  Created
 Container shop-db-1  Creating
 Container shop-db-1  Created
 Container shop-db-1  Starting
 Container shop-db-1  Started
 Container shop-app-1  Running
END
;;"#;

const READS: &str = r#"*" port "*) echo '0.0.0.0:32768' ;;
*" ps "*) echo 4f1c9a ;;
*inspect*) echo '[{"Id":"4f1c9a","RestartCount":0,"State":{"Status":"running"}}]' ;;"#;

#[test]
fn a_plan_is_a_dry_run_of_up() {
    let dir = scratch(TMP, "plan");
    let log = KeptLog::default();
    let plan = runner(&dir, DRY_RUN)
        .plan(&context(&dir), &compiled(""), &log)
        .unwrap();
    assert_eq!(
        calls(&dir),
        ["compose --dry-run up --detach --remove-orphans"]
    );
    let shown: Vec<String> = plan.changes.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        shown,
        [
            "create backend: network shop_backend",
            "update app: container shop-app-1",
            // Not one of the document's services, so it keeps Compose's name.
            "delete shop-old-1: container shop-old-1",
        ]
    );
    assert!(!plan.unknown);
    assert_eq!(
        log.text()[0],
        "project shop: asking Compose what bringing it up would change"
    );
    assert!(dir.join("work/compose.yaml").exists());
}

#[test]
fn an_apply_brings_the_project_up_and_reads_the_outputs() {
    let dir = scratch(TMP, "apply");
    let mut docker = runner(&dir, &format!("{UP}\n{READS}"));
    docker.extra_args = vec!["--wait".into()];
    let applied = docker
        .apply(
            &context(&dir),
            &compiled("ssh://web01"),
            &KeptLog::default(),
        )
        .unwrap();
    assert_eq!(
        calls(&dir),
        [
            // The daemon is the document's host, reached over SSH.
            "--host ssh://web01 compose up --detach --remove-orphans --wait",
            "--host ssh://web01 compose port --protocol tcp app 80",
            "--host ssh://web01 compose ps --all --quiet db",
            "--host ssh://web01 inspect 4f1c9a",
            "--host ssh://web01 compose ps --all --quiet db",
            "--host ssh://web01 inspect 4f1c9a",
        ]
    );
    let shown: Vec<String> = applied.changes.iter().map(|c| c.to_string()).collect();
    assert_eq!(
        shown,
        [
            "create data: volume shop_data",
            "create db: container shop-db-1"
        ]
    );
    assert_eq!(
        applied.outputs[0].value,
        Value::String("http://web01:32768".into())
    );
    assert_eq!(applied.outputs[1].value, Value::String("running".into()));
    // One reference alone keeps the type Docker gave it.
    assert_eq!(applied.outputs[2].value, Value::Int(0));
}

#[test]
fn an_output_with_nothing_behind_it_is_an_error() {
    let dir = scratch(TMP, "unpublished");
    let cases = format!("{UP}\n*\" port \"*) echo ':0' ;;");
    let err = runner(&dir, &cases.replace("':0'", "''"))
        .apply(&context(&dir), &compiled(""), &KeptLog::default())
        .unwrap_err();
    assert_eq!(
        err.msg,
        "output \"url\": service \"app\" does not publish port 80"
    );
}

#[test]
fn a_failed_up_keeps_what_compose_said() {
    let dir = scratch(TMP, "failed");
    let cases =
        "*\" up \"*) echo 'Error response from daemon: pull access denied for app' >&2; exit 1 ;;";
    let err = runner(&dir, cases)
        .apply(&context(&dir), &compiled(""), &KeptLog::default())
        .unwrap_err();
    assert_eq!(err.msg, "docker compose could not bring the project up");
    assert!(err.output.contains("pull access denied"), "{}", err.output);
}

#[test]
fn outputs_are_read_from_the_running_project() {
    let dir = scratch(TMP, "outputs");
    let docker = runner(&dir, READS);
    assert!(docker.outputs(&context(&dir)).is_err());
    fs::create_dir_all(dir.join("work")).unwrap();
    fs::write(
        dir.join("work/project.json"),
        &compiled("").files["project.json"],
    )
    .unwrap();
    let outputs = docker.outputs(&context(&dir)).unwrap();
    assert_eq!(outputs.len(), 3);
    assert_eq!(outputs[0].name, "url");
}
