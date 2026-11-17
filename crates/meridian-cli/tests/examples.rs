//! Every document under `examples/` compiles through the real command and
//! the real plugins, so an example the documentation shows cannot rot.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use meridian_plugin_testkit::{plugin_binary, repo_root, scratch};

/// Whether a file holds values for `--data`, not documents.
fn is_data(file: &Path) -> bool {
    file.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with(".data.yaml"))
}

fn examples() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for directory in fs::read_dir(repo_root().join("examples")).unwrap() {
        let directory = directory.unwrap().path();
        if !directory.is_dir() {
            continue;
        }
        for file in fs::read_dir(&directory).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_some_and(|e| e == "yaml") && !is_data(&file) {
                found.push(file);
            }
        }
    }
    found.sort();
    found
}

/// The data an example is compiled with, one list of `--data` files per run.
///
/// `site.yaml` is given `site.data.yaml` when that file exists. Each
/// `site.<name>.data.yaml` beside it is a variant, layered over the first as
/// a second `--data`, and compiled as a run of its own. An example with no
/// data file is compiled once, with no data.
fn data_sets(example: &Path) -> Vec<Vec<PathBuf>> {
    let stem = example.file_stem().unwrap().to_str().unwrap();
    let directory = example.parent().unwrap();
    let base = directory.join(format!("{stem}.data.yaml"));
    if !base.exists() {
        return vec![Vec::new()];
    }
    let mut sets = vec![vec![base.clone()]];
    let mut variants: Vec<PathBuf> = fs::read_dir(directory)
        .unwrap()
        .map(|file| file.unwrap().path())
        .filter(|file| {
            let name = file.file_name().unwrap().to_str().unwrap();
            is_data(file) && *file != base && name.starts_with(&format!("{stem}."))
        })
        .collect();
    variants.sort();
    for variant in variants {
        sets.push(vec![base.clone(), variant]);
    }
    sets
}

/// The targets a file's documents select.
fn targets(file: &Path) -> BTreeSet<String> {
    fs::read_to_string(file)
        .unwrap()
        .lines()
        .filter_map(|line| line.strip_prefix("  target: "))
        .map(|target| target.trim().to_owned())
        .collect()
}

/// The names of a file's documents.
fn names(file: &Path) -> Vec<String> {
    let text = fs::read_to_string(file).unwrap();
    let mut names = Vec::new();
    let mut in_metadata = false;
    for line in text.lines() {
        if line == "metadata:" {
            in_metadata = true;
        } else if !line.starts_with(' ') && !line.is_empty() {
            in_metadata = false;
        } else if in_metadata {
            if let Some(name) = line.strip_prefix("  name: ") {
                names.push(name.trim().to_owned());
            }
        }
    }
    names
}

/// A workflow with every `targetHost` that waits for an output given an
/// address instead, as it is once the stage it waits for has been applied.
fn given_hosts(text: &str) -> String {
    let mut out = String::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        if line == "    from: infrastructure" {
            out.push_str("    from: inline\n    host: 203.0.113.10\n");
            // `ref` and `output` belong to the form being replaced.
            while lines.peek().is_some_and(|next| {
                next.starts_with("    ref: ") || next.starts_with("    output: ")
            }) {
                lines.next();
            }
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

/// `meridian <command> <file>` with every plugin the file needs and the data
/// given.
fn meridian(command: &str, file: &Path, data: &[PathBuf]) -> Command {
    let mut meridian = Command::new(env!("CARGO_BIN_EXE_meridian"));
    meridian.arg(command).arg(file);
    if command != "validate" {
        for target in targets(file) {
            let binary = plugin_binary(&format!("meridian-target-{target}"));
            meridian.arg("--plugin-dir").arg(binary.parent().unwrap());
        }
        for data in data {
            meridian.arg("--data").arg(data);
        }
    }
    meridian
}

fn run(mut command: Command, what: &str) -> String {
    let output = command.output().unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.status.success(), "{what} failed:\n{text}");
    text
}

/// Every file `meridian compile --out` wrote for a workflow, by its path
/// under the output directory.
fn compiled(file: &Path, data: &[PathBuf], scratch_name: &str) -> BTreeMap<String, String> {
    let out = scratch(env!("CARGO_TARGET_TMPDIR"), scratch_name);
    let mut command = meridian("compile", file, data);
    command.arg("--out").arg(&out);
    run(command, &format!("compiling {}", file.display()));
    let mut files = BTreeMap::new();
    let mut pending = vec![out.clone()];
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else {
                let relative = path.strip_prefix(&out).unwrap().to_str().unwrap();
                files.insert(relative.to_owned(), fs::read_to_string(&path).unwrap());
            }
        }
    }
    files
}

#[test]
fn every_example_validates_and_compiles_with_the_real_plugins() {
    let examples = examples();
    assert!(!examples.is_empty(), "no examples were found");
    for file in examples {
        assert!(!targets(&file).is_empty(), "{}: no target", file.display());
        run(
            meridian("validate", &file, &[]),
            &format!("validating {}", file.display()),
        );
        for data in data_sets(&file) {
            run(
                meridian("compile", &file, &data),
                &format!("compiling {} with {data:?}", file.display()),
            );
        }
    }
}

/// A workflow is several documents, and each one a command can compile ends
/// up in a directory of its own, named after the document. A ResourceSet that
/// waits for an output of an earlier stage has no host yet, so it is reported
/// as deferred and nothing is written for it.
#[test]
fn every_workflow_compiles_each_document_or_defers_it() {
    let workflows: Vec<PathBuf> = examples()
        .into_iter()
        .filter(|file| file.parent().unwrap().ends_with("workflow"))
        .collect();
    assert!(
        workflows.len() >= 6,
        "examples/workflow holds {} workflows",
        workflows.len()
    );
    for file in workflows {
        let text = fs::read_to_string(&file).unwrap();
        let names = names(&file);
        assert!(
            names.len() >= 2,
            "{} is not a workflow of several documents",
            file.display()
        );
        // The documents that wait for an output, in the order the file
        // declares its documents.
        let waiting: Vec<bool> = text
            .split("\n---\n")
            .map(|document| document.contains("    from: infrastructure"))
            .collect();
        assert_eq!(waiting.len(), names.len(), "{}", file.display());

        let stem = file.file_stem().unwrap().to_str().unwrap();
        for (i, data) in data_sets(&file).iter().enumerate() {
            // A deferred document is not compiled, so on its own that run
            // proves nothing about it. Each one is compiled again here as it
            // will be once its stage has run: with the host filled in.
            let with_hosts = given_hosts(&text);
            assert!(!with_hosts.contains("from: infrastructure"));
            let scratch_dir = scratch(env!("CARGO_TARGET_TMPDIR"), &format!("hosts-{stem}-{i}"));
            let copy = scratch_dir.join(file.file_name().unwrap());
            fs::write(&copy, with_hosts).unwrap();
            let files = compiled(&copy, data, &format!("workflow-hosts-{stem}-{i}"));
            for name in &names {
                assert!(
                    files
                        .keys()
                        .any(|path| path.starts_with(&format!("{name}/"))),
                    "{}: document {name:?} compiled to nothing once it had a host; files: {:?}",
                    file.display(),
                    files.keys().collect::<Vec<_>>()
                );
            }

            let files = compiled(&file, data, &format!("workflow-{stem}-{i}"));
            for (name, waits) in names.iter().zip(&waiting) {
                let written = files
                    .keys()
                    .any(|path| path.starts_with(&format!("{name}/")));
                assert_eq!(
                    written,
                    !waits,
                    "{}: document {name:?} (waits for an output: {waits}); files: {:?}",
                    file.display(),
                    files.keys().collect::<Vec<_>>()
                );
            }
        }
    }
}

/// The same conditions decide the same way in every document of a workflow,
/// whatever tool each document compiles for.
#[test]
fn one_set_of_conditions_decides_a_whole_workflow() {
    let file = repo_root().join("examples/workflow/environments.yaml");
    let sets = data_sets(&file);
    assert_eq!(sets.len(), 2, "a development and a production data set");
    let dev = compiled(&file, &sets[0], "environments-dev");
    let prod = compiled(&file, &sets[1], "environments-prod");
    let all = |files: &BTreeMap<String, String>, document: &str| -> String {
        files
            .iter()
            .filter(|(path, _)| path.starts_with(&format!("{document}/")))
            .map(|(_, content)| content.as_str())
            .collect()
    };

    // Terraform: `data.env != 'prod'` keeps the bastion in development only,
    // and `data.env == 'prod'` keeps the address in production only.
    let (dev_machines, prod_machines) = (all(&dev, "machines"), all(&prod, "machines"));
    assert!(dev_machines.contains("bastion"), "{dev_machines}");
    assert!(!prod_machines.contains("bastion"), "{prod_machines}");
    assert!(!dev_machines.contains("aws_eip"), "{dev_machines}");
    assert!(prod_machines.contains("aws_eip"), "{prod_machines}");
    // `defined(data.audit) && data.audit.enabled`: the key is only in production.
    assert!(!dev_machines.contains("shop-audit-logs"));
    assert!(prod_machines.contains("shop-audit-logs"));
    // The resource that waited for the bastion carries on without it.
    assert!(prod_machines.contains("terraform_data"));

    // Kubernetes: the same `data.env != 'prod'`, the same answer.
    let (dev_jobs, prod_jobs) = (all(&dev, "jobs"), all(&prod, "jobs"));
    assert!(
        dev_jobs.contains("ghcr.io/example/load:0.4.0"),
        "{dev_jobs}"
    );
    assert!(
        !prod_jobs.contains("ghcr.io/example/load:0.4.0"),
        "{prod_jobs}"
    );
    assert!(!dev_jobs.contains("PodDisruptionBudget"));
    assert!(prod_jobs.contains("PodDisruptionBudget"));
    // What no condition touches is in both.
    for jobs in [&dev_jobs, &prod_jobs] {
        assert!(jobs.contains("CronJob"), "{jobs}");
    }
    // No condition reaches a tool: nothing compiled mentions `data.`.
    for (path, content) in dev.iter().chain(&prod) {
        assert!(!content.contains("data.env"), "{path} carries a condition");
    }

    // With no data the workflow has no answer, and says which key it needs.
    let output = meridian("compile", &file, &[]).output().unwrap();
    assert!(!output.status.success());
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(said.contains("data.env is not set"), "{said}");
}
