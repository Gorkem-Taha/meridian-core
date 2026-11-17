//! `meridian-policy-opa <policy>...`: see the library for what it does.

use std::io::Read;
use std::path::PathBuf;
use std::process::ExitCode;

use meridian_policy_opa::{evaluate, OPA_VARIABLE};

fn main() -> ExitCode {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let opa = std::env::var(OPA_VARIABLE)
        .ok()
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "opa".to_owned());
    let mut input = Vec::new();
    if let Err(e) = std::io::stdin().read_to_end(&mut input) {
        eprintln!("meridian-policy-opa: reading the input: {e}");
        return ExitCode::FAILURE;
    }
    match evaluate(&opa, &paths, &input) {
        Ok(answer) => {
            println!("{answer}");
            ExitCode::SUCCESS
        }
        Err(why) => {
            eprintln!("meridian-policy-opa: {why}");
            ExitCode::FAILURE
        }
    }
}
