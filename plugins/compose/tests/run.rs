//! The running half, against a stand-in for `docker`.

#![cfg(unix)]

use std::fs;
use std::path::Path;

use meridian_plugin_testkit::{artifact, scratch, stand_in, KeptLog};
use meridian_sdk::{Artifact, RunContext, Runner, Value};
use meridian_target_compose::ComposeRunner;

const TMP: &str = env!("CARGO_TARGET_TMPDIR");

fn project(settings: &str) -> Artifact {
    artifact(&[
        ("compose.yaml", "name: shop\nservices: {}\n"),
        ("project.json", settings),
    ])
}

fn compiled() -> Artifact {
    project(
        r#"{"name":"shop","directory":"","profiles":[],"wait":true,"wait_timeout":0,"outputs":{}}"#,
    )
}

fn context(dir: &Path) -> RunContext {
    RunContext {
        document: "shop".into(),
        work_dir: dir.join("work"),
        ..RunContext::default()
    }
}

/// A stand-in for both `docker` and `docker compose`. It appends each call
/// to `calls` and the `DOCKER_HOST` it saw to `hosts`, and answers by what
/// the whole command line looks like.
fn runner(dir: &Path, cases: &str) -> ComposeRunner {
    let program = stand_in(
        dir,
        "docker",
        &format!(
            "echo \"$@\" >> \"{dir}/calls\"\necho \"${{DOCKER_HOST:-local}}\" >> \"{dir}/hosts\"\ncase \"$*\" in\n{cases}\nesac",
            dir = dir.display()
        ),
    );
    let program = program.to_str().unwrap().to_owned();
    ComposeRunner {
        compose: vec![program.clone(), "compose".to_owned()],
        docker: program,
        extra_args: Vec::new(),
    }
}

/// The calls so far, with what every one of them repeats taken out.
fn calls(dir: &Path) -> Vec<String> {
    fs::read_to_string(dir.join("calls"))
        .unwrap_or_default()
        .lines()
        .map(|l| {
            l.replace("compose --ansi never --project-name shop --file compose.yaml", "compose")
                .replace(" --filter label=com.docker.compose.project=shop", "")
                .replace(
                    " --format {{.Label \"com.docker.compose.service\"}}|{{.Label \"com.docker.compose.config-hash\"}}|{{.State}}|{{.Names}}|{{.Label \"com.docker.compose.oneoff\"}}",
                    " --format <containers>",
                )
        })
        .collect()
}

/// The file as Compose reads it: two services, the default network and a volume.
const CONFIG: &str = r#"*" config --format json") echo '{"name":"shop","services":{"db":{"image":"postgres:17"},"web":{"image":"nginx:1.27"}},"networks":{"default":{"name":"shop_default"}},"volumes":{"data":{"name":"shop_data"}}}' ;;
*" config --hash "*) echo 'db h-db'; echo 'web h-web' ;;"#;

/// Docker has nothing of the project.
const NOTHING: &str = "";

/// Docker has everything, as the file describes it.
const RUNNING: &str = r#""ps --all "*) echo 'db|h-db|running|shop-db-1|False'; echo 'web|h-web|running|shop-web-1|False' ;;
"network ls "*) echo shop_default ;;
"volume ls "*) echo shop_data ;;"#;

const SURVEY: [&str; 5] = [
    "compose config --format json",
    "compose config --hash *",
    "ps --all --format <containers>",
    "network ls --format {{.Name}}",
    "volume ls --format {{.Name}}",
];

#[test]
fn a_plan_compares_the_file_with_docker_and_changes_nothing() {
    let dir = scratch(TMP, "plan");
    let log = KeptLog::default();
    let plan = runner(&dir, &format!("{CONFIG}\n{NOTHING}"))
        .plan(&context(&dir), &compiled(), &log)
        .unwrap();
    assert_eq!(calls(&dir), SURVEY);
    let shown: Vec<String> = plan.changes.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "create default: network shop_default",
            "create data: volume shop_data",
            "create db: container, image postgres:17",
            "create web: container, image nginx:1.27",
        ]
    );
    assert!(!plan.unknown);
    assert_eq!(
        log.text(),
        ["project shop: comparing the file with what Docker is running"]
    );
    // The compiled files are where Compose was told to read them.
    assert!(dir.join("work/compose.yaml").is_file());
    assert!(dir.join("work/project.json").is_file());
}

#[test]
fn a_project_already_as_the_file_describes_is_in_sync() {
    let dir = scratch(TMP, "in-sync");
    let plan = runner(&dir, &format!("{CONFIG}\n{RUNNING}"))
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap();
    assert!(plan.in_sync(), "{:?}", plan.changes);
}

#[test]
fn a_plan_reports_a_changed_service_a_stopped_one_and_an_orphan() {
    let dir = scratch(TMP, "plan-drift");
    let cases = format!(
        "{CONFIG}\n\"ps --all \"*) echo 'db|h-old|running|shop-db-1|False'; echo 'web|h-web|exited|shop-web-1|False'; echo 'cache|h-c|running|shop-cache-1|False'; echo 'web|h-web|running|shop-web-run-1|True' ;;\n\"network ls \"*) echo shop_default ;;\n\"volume ls \"*) echo shop_data ;;"
    );
    let plan = runner(&dir, &cases)
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap();
    let shown: Vec<String> = plan.changes.iter().map(ToString::to_string).collect();
    assert_eq!(
        shown,
        [
            "replace db: its configuration changed, so its container is recreated",
            "update web: started; container shop-web-1 is exited",
            "delete cache: no longer in the document, so its containers are removed",
        ]
    );
}

#[test]
fn an_apply_brings_the_project_up_waits_and_reports_what_it_planned() {
    let dir = scratch(TMP, "apply");
    let log = KeptLog::default();
    let cases =
        format!("{CONFIG}\n{NOTHING}\n*\" up \"*) echo ' Container shop-web-1 Started' >&2 ;;");
    let applied = runner(&dir, &cases)
        .apply(&context(&dir), &compiled(), &log)
        .unwrap();
    assert_eq!(
        calls(&dir)[SURVEY.len()..],
        ["compose up --detach --remove-orphans --wait"]
    );
    assert_eq!(applied.changes.len(), 4);
    assert!(applied.outputs.is_empty());
    // What Compose printed reached the operator as it happened.
    assert_eq!(log.text().last().unwrap(), " Container shop-web-1 Started");
}

#[test]
fn the_projects_settings_shape_every_call() {
    let dir = scratch(TMP, "settings");
    let settings = r#"{"name":"shop","directory":"/srv/shop","profiles":["debug","metrics"],"wait":true,"wait_timeout":90,"outputs":{}}"#;
    let mut docker = runner(&dir, &format!("{CONFIG}\n{RUNNING}"));
    docker.extra_args = vec!["--env-file".into(), "/etc/shop.env".into()];
    docker
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap();
    let all = calls(&dir);
    let shared = "compose --project-directory /srv/shop --profile debug --profile metrics --env-file /etc/shop.env";
    assert_eq!(all[0], format!("{shared} config --format json"));
    assert_eq!(
        all.last().unwrap(),
        &format!("{shared} up --detach --remove-orphans --wait --wait-timeout 90")
    );
}

#[test]
fn a_project_that_does_not_wait_is_brought_up_without_waiting() {
    let dir = scratch(TMP, "no-wait");
    let settings = r#"{"name":"shop","wait":false,"outputs":{}}"#;
    runner(&dir, &format!("{CONFIG}\n{RUNNING}"))
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap();
    assert_eq!(
        calls(&dir).last().unwrap(),
        "compose up --detach --remove-orphans"
    );
}

#[test]
fn a_service_built_from_source_is_rebuilt_and_makes_the_plan_uncertain() {
    let dir = scratch(TMP, "build");
    let cases = r#"*" config --format json") echo '{"services":{"api":{"build":{"context":"/srv/api"}}}}' ;;
*" config --hash "*) echo 'api h-api' ;;
"ps --all "*) echo 'api|h-api|running|shop-api-1|False' ;;"#;
    let docker = runner(&dir, cases);
    let plan = docker
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap();
    assert!(plan.changes.is_empty());
    assert!(
        plan.unknown,
        "a build's result cannot be known without building"
    );

    docker
        .apply(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap();
    assert_eq!(
        calls(&dir).last().unwrap(),
        "compose up --detach --remove-orphans --build --wait"
    );
}

#[test]
fn outputs_are_answered_by_asking_docker() {
    let dir = scratch(TMP, "outputs");
    let settings = r#"{"name":"shop","outputs":{
        "url":"http://${web.address.80}/",
        "port":"${web.port.80}",
        "dns":"${web.port.53/udp}",
        "container":"${web.id}",
        "volume":"shop_data",
        "literal":"costs $${5}"}}"#;
    let cases = format!(
        "{CONFIG}\n{RUNNING}\n*\" port --protocol udp web 53\") echo '0.0.0.0:5353' ;;\n*\" port web 80\") echo '0.0.0.0:32768'; echo '[::]:32768' ;;\n*\" ps --quiet web\") echo 93674f554175; echo 0a1b2c3d4e5f ;;"
    );
    let docker = runner(&dir, &cases);
    let applied = docker
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap();
    let read = |name: &str| {
        let output = applied
            .outputs
            .iter()
            .find(|o| o.name == name)
            .unwrap_or_else(|| panic!("no output {name}"));
        assert!(!output.sensitive);
        match &output.value {
            Value::String(s) => s.clone(),
            other => panic!("{name} is {other:?}"),
        }
    };
    assert_eq!(read("url"), "http://localhost:32768/");
    assert_eq!(read("port"), "32768");
    assert_eq!(read("dns"), "5353");
    assert_eq!(read("container"), "93674f554175");
    assert_eq!(read("volume"), "shop_data");
    assert_eq!(read("literal"), "costs ${5}");

    // Later, without applying anything, from the settings the apply left.
    let again = docker.outputs(&context(&dir)).unwrap();
    assert_eq!(again, applied.outputs);
}

#[test]
fn outputs_need_an_earlier_apply() {
    let dir = scratch(TMP, "outputs-none");
    let err = runner(&dir, "").outputs(&context(&dir)).unwrap_err();
    assert_eq!(
        err.msg,
        "nothing has been applied for this document yet, so it has no outputs"
    );
    assert!(calls(&dir).is_empty());
}

#[test]
fn an_output_for_a_port_the_service_does_not_publish_says_so() {
    let dir = scratch(TMP, "outputs-unpublished");
    let settings = r#"{"name":"shop","outputs":{"port":"${web.port.80}"}}"#;
    let cases = format!(
        "{CONFIG}\n{RUNNING}\n*\" port web 80\") echo 'no port 80/tcp for container shop-web-1' >&2; exit 1 ;;"
    );
    let err = runner(&dir, &cases)
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap_err();
    assert_eq!(
        err.msg,
        "output \"port\" reads the host port for port 80 of service \"web\", and the running \
         service publishes none. List it under the service's \"ports\""
    );
    assert_eq!(err.exit_code, Some(1));

    // The same when Compose answers with a port of zero and no error.
    let dir = scratch(TMP, "outputs-zero");
    let cases = format!("{CONFIG}\n{RUNNING}\n*\" port web 80\") echo ':0' ;;");
    let err = runner(&dir, &cases)
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap_err();
    assert!(err.msg.contains("publishes none"), "{}", err.msg);
}

#[test]
fn an_output_for_a_service_with_no_container_says_so() {
    let dir = scratch(TMP, "outputs-no-container");
    let settings = r#"{"name":"shop","outputs":{"container":"${web.id}"}}"#;
    let err = runner(&dir, &format!("{CONFIG}\n{RUNNING}"))
        .apply(&context(&dir), &project(settings), &KeptLog::default())
        .unwrap_err();
    assert_eq!(
        err.msg,
        "output \"container\" reads the container of service \"web\", and it has none running"
    );
}

#[test]
fn a_documents_host_is_where_docker_is_reached() {
    let dir = scratch(TMP, "host");
    let docker = runner(&dir, &format!("{CONFIG}\n{RUNNING}"));
    let mut remote = context(&dir);
    remote.host = "metrics01.example.com".into();
    docker
        .apply(&remote, &compiled(), &KeptLog::default())
        .unwrap();
    let hosts = fs::read_to_string(dir.join("hosts")).unwrap();
    assert_eq!(hosts.lines().count(), SURVEY.len() + 1);
    assert!(
        hosts.lines().all(|h| h == "ssh://metrics01.example.com"),
        "every call, Compose's and Docker's, goes to the host:\n{hosts}"
    );

    let dir = scratch(TMP, "host-local");
    let docker = runner(&dir, &format!("{CONFIG}\n{RUNNING}"));
    let mut local = context(&dir);
    local.host = "localhost".into();
    docker
        .plan(&local, &compiled(), &KeptLog::default())
        .unwrap();
    let hosts = fs::read_to_string(dir.join("hosts")).unwrap();
    assert!(hosts.lines().all(|h| h == "local"), "{hosts}");
}

#[test]
fn a_file_compose_rejects_is_an_error_with_what_compose_said() {
    let dir = scratch(TMP, "rejected");
    let cases =
        "*\" config --format json\") echo 'services.web.ports must be a list' >&2; exit 15 ;;";
    let log = KeptLog::default();
    let err = runner(&dir, cases)
        .plan(&context(&dir), &compiled(), &log)
        .unwrap_err();
    assert_eq!(err.msg, "docker compose rejected the compiled file");
    assert_eq!(err.exit_code, Some(15));
    assert_eq!(err.output, "services.web.ports must be a list\n");
    assert!(
        err.command.ends_with("config --format json"),
        "{}",
        err.command
    );
    assert!(log
        .text()
        .contains(&"services.web.ports must be a list".to_owned()));
    // Nothing further was asked once the file was refused.
    assert_eq!(calls(&dir).len(), 1);
}

#[test]
fn a_project_that_fails_to_come_up_is_an_error_with_where_to_look() {
    let dir = scratch(TMP, "up-failed");
    let cases = format!(
        "{CONFIG}\n{NOTHING}\n*\" up \"*) echo 'container shop-db-1 is unhealthy' >&2; exit 1 ;;"
    );
    let err = runner(&dir, &cases)
        .apply(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap_err();
    assert!(
        err.msg
            .starts_with("docker compose could not bring the project up"),
        "{}",
        err.msg
    );
    assert_eq!(err.exit_code, Some(1));
    assert!(err.output.contains("container shop-db-1 is unhealthy"));
}

#[test]
fn docker_being_unreachable_is_an_error_not_an_empty_plan() {
    let dir = scratch(TMP, "daemon-down");
    let cases = format!(
        "{CONFIG}\n\"ps --all \"*) echo 'Cannot connect to the Docker daemon' >&2; exit 1 ;;"
    );
    let err = runner(&dir, &cases)
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap_err();
    assert_eq!(
        err.msg,
        "docker could not list what the project has running"
    );
    assert!(err.output.contains("Cannot connect to the Docker daemon"));
}

#[test]
fn docker_not_being_installed_is_named() {
    let dir = scratch(TMP, "not-installed");
    let docker = ComposeRunner {
        docker: "meridian-no-such-docker".into(),
        compose: vec!["meridian-no-such-docker".into(), "compose".into()],
        extra_args: Vec::new(),
    };
    let err = docker
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap_err();
    assert!(
        err.msg
            .starts_with("could not run meridian-no-such-docker: it is not installed"),
        "{}",
        err.msg
    );
}

#[test]
fn an_artifact_this_plugin_did_not_compile_is_refused() {
    let dir = scratch(TMP, "foreign");
    let docker = runner(&dir, CONFIG);
    let err = docker
        .plan(
            &context(&dir),
            &artifact(&[("playbook.yml", "- hosts: all\n")]),
            &KeptLog::default(),
        )
        .unwrap_err();
    assert_eq!(
        err.msg,
        "the artifact has no compose.yaml and project.json; it was not compiled by this plugin"
    );
    let err = docker
        .plan(&context(&dir), &project("[]"), &KeptLog::default())
        .unwrap_err();
    assert_eq!(err.msg, "project.json is not one this plugin compiled");
    assert!(
        calls(&dir).is_empty(),
        "nothing is run for a foreign artifact"
    );
}

#[test]
fn a_standalone_compose_program_replaces_the_subcommand() {
    let dir = scratch(TMP, "standalone");
    let mut docker = runner(&dir, &format!("{CONFIG}\n{RUNNING}"));
    // `docker-compose`, or `podman compose`: the words that select Compose
    // are the operator's to choose.
    docker.compose = vec![docker.docker.clone()];
    docker
        .plan(&context(&dir), &compiled(), &KeptLog::default())
        .unwrap();
    assert_eq!(
        fs::read_to_string(dir.join("calls"))
            .unwrap()
            .lines()
            .next()
            .unwrap(),
        "--ansi never --project-name shop --file compose.yaml config --format json"
    );
}

#[test]
fn the_environment_names_the_programs_and_extra_arguments() {
    // The one test that touches these variables, so it cannot race another.
    use meridian_target_compose::{ARGS_VARIABLE, COMPOSE_VARIABLE, DOCKER_VARIABLE};
    for variable in [DOCKER_VARIABLE, COMPOSE_VARIABLE, ARGS_VARIABLE] {
        std::env::remove_var(variable);
    }
    let default = ComposeRunner::from_env();
    assert_eq!(default.docker, "docker");
    assert_eq!(default.compose, ["docker", "compose"]);
    assert!(default.extra_args.is_empty());

    std::env::set_var(DOCKER_VARIABLE, "/opt/bin/podman");
    std::env::set_var(ARGS_VARIABLE, " --env-file  /etc/shop.env ");
    let podman = ComposeRunner::from_env();
    assert_eq!(podman.docker, "/opt/bin/podman");
    assert_eq!(podman.compose, ["/opt/bin/podman", "compose"]);
    assert_eq!(podman.extra_args, ["--env-file", "/etc/shop.env"]);

    std::env::set_var(COMPOSE_VARIABLE, "docker-compose");
    std::env::set_var(DOCKER_VARIABLE, "  ");
    let standalone = ComposeRunner::from_env();
    assert_eq!(standalone.docker, "docker");
    assert_eq!(standalone.compose, ["docker-compose"]);

    for variable in [DOCKER_VARIABLE, COMPOSE_VARIABLE, ARGS_VARIABLE] {
        std::env::remove_var(variable);
    }
}
