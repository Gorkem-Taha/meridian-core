//! Runs the shell script that stands in for a tool, on a system where a
//! script cannot be started as a program.
//!
//! `meridian_plugin_testkit::stand_in` copies this program to `<tool>.exe`
//! and writes the script to `<tool>.sh` beside it. Started as the tool, it
//! hands the script, its own arguments, its standard streams and its working
//! directory to `sh`, and exits with the script's status.

use std::path::PathBuf;
use std::process::{Command, ExitCode};

/// The `sh` to run a stand-in with: the one named by `MERIDIAN_TEST_SH`, the
/// one Git for Windows installs, or whatever `sh` is on the path.
fn shell() -> PathBuf {
    if let Some(named) = std::env::var_os("MERIDIAN_TEST_SH") {
        return PathBuf::from(named);
    }
    ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|base| PathBuf::from(base).join("Git").join("bin").join("sh.exe"))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("sh"))
}

fn main() -> ExitCode {
    let program = match std::env::current_exe() {
        Ok(program) => program,
        Err(err) => {
            eprintln!("meridian-stand-in: cannot tell which tool this stands in for: {err}");
            return ExitCode::from(127);
        }
    };
    let script = program.with_extension("sh");
    let shell = shell();
    let status = Command::new(&shell)
        .arg(&script)
        .args(std::env::args_os().skip(1))
        // An argument is the tool's argument, not a pattern for the shell's
        // runtime to expand before the script sees it.
        .env("MSYS", "noglob")
        .status();
    match status {
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(0, 255) as u8),
        Err(err) => {
            eprintln!(
                "meridian-stand-in: could not run {} with {}: {err}. The tests' stand-ins \
                 are shell scripts; install Git for Windows or name an sh in MERIDIAN_TEST_SH",
                script.display(),
                shell.display()
            );
            ExitCode::from(127)
        }
    }
}
