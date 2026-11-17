//! Every document under `examples/` compiles through the real command and
//! the real plugins, so an example the documentation shows cannot rot.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use meridian_plugin_testkit::{plugin_binary, repo_root};

fn examples() -> Vec<PathBuf> {
    let mut found = Vec::new();
    for directory in fs::read_dir(repo_root().join("examples")).unwrap() {
        let directory = directory.unwrap().path();
        if !directory.is_dir() {
            continue;
        }
        for file in fs::read_dir(&directory).unwrap() {
            let file = file.unwrap().path();
            if file.extension().is_some_and(|e| e == "yaml") {
                found.push(file);
            }
        }
    }
    found.sort();
    found
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

#[test]
fn every_example_compiles_with_the_real_plugins() {
    let examples = examples();
    assert!(!examples.is_empty(), "no examples were found");
    for file in examples {
        let targets = targets(&file);
        assert!(!targets.is_empty(), "{}: no target", file.display());

        let mut command = Command::new(env!("CARGO_BIN_EXE_meridian"));
        command.arg("compile").arg(&file);
        for target in &targets {
            let binary = plugin_binary(&format!("meridian-target-{target}"));
            command.arg("--plugin-dir").arg(binary.parent().unwrap());
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{} did not compile:\n{}{}",
            file.display(),
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}
