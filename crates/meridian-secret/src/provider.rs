//! The three providers that need nothing installed: the environment, a
//! file, and a command.
//!
//! Each reads straight into protected memory. None of them puts the value,
//! or anything read from the place the value lives, into an error: an error
//! names the provider and where it looked, and stops there.

use std::fmt;
use std::fs::File;
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::{SecretValue, DEFAULT_MAX_LEN};

/// Why a secret could not be fetched. It never holds the value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// The place the secret was to be read from has none.
    Missing(String),
    /// The place exists and was refused, or reading it failed.
    Failed(String),
}

impl fmt::Display for FetchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FetchError::Missing(why) | FetchError::Failed(why) => f.write_str(why),
        }
    }
}

impl std::error::Error for FetchError {}

fn failed(what: impl fmt::Display, e: std::io::Error) -> FetchError {
    FetchError::Failed(format!("{what}: {e}"))
}

/// Reads a secret from an environment variable of this process, then
/// removes the variable, so that no program Meridian starts inherits it.
///
/// This is the weakest provider and is documented as such: the kernel's own
/// record of the environment the process started with is not Meridian's to
/// wipe.
pub fn from_env(key: &str) -> Result<SecretValue, FetchError> {
    let Some(value) = std::env::var_os(key) else {
        return Err(FetchError::Missing(format!(
            "the environment variable {key} is not set"
        )));
    };
    std::env::remove_var(key);
    // The copy the standard library made is ordinary memory: move the bytes
    // to their home and wipe the copy.
    let mut bytes = value.into_encoded_bytes();
    SecretValue::take_from(&mut bytes)
        .map_err(|e| failed(format_args!("the environment variable {key}"), e))
}

/// How [`from_file`] reads.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FileOptions {
    /// Read the file even when its mode lets others than its owner read it.
    pub allow_wide_mode: bool,
    /// Keep a line ending at the end of the file as part of the secret.
    pub keep_trailing_newline: bool,
}

/// Reads a secret from a file.
///
/// A file others than its owner can read is refused unless the options say
/// otherwise: a secret anyone on the machine can read is rarely what was
/// meant, and saying so beats using it quietly.
pub fn from_file(path: &Path, options: FileOptions) -> Result<SecretValue, FetchError> {
    let shown = path.display();
    let file = match File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FetchError::Missing(format!(
                "the file {shown} does not exist"
            )))
        }
        Err(e) => return Err(failed(format_args!("the file {shown}"), e)),
    };
    let meta = file
        .metadata()
        .map_err(|e| failed(format_args!("the file {shown}"), e))?;
    if !meta.is_file() {
        return Err(FetchError::Failed(format!("{shown} is not a file")));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 && !options.allow_wide_mode {
            return Err(FetchError::Failed(format!(
                "the file {shown} has mode {mode:04o}, which lets others than its owner read \
                 it; make it 0600, or set allowWideMode: true on the secret if that is \
                 intended"
            )));
        }
    }
    let secret = SecretValue::from_reader(file, DEFAULT_MAX_LEN)
        .map_err(|e| failed(format_args!("the file {shown}"), e))?;
    if options.keep_trailing_newline {
        Ok(secret)
    } else {
        secret
            .without_trailing_newline()
            .map_err(|e| failed(format_args!("the file {shown}"), e))
    }
}

/// Reads a secret from the standard output of a command.
///
/// The command is run as given, without a shell. Its standard input is
/// closed, its standard error is discarded (a command that fails may print
/// the very thing it was asked for), and a line ending at the end of what it
/// prints is not part of the secret.
pub fn from_exec(command: &[String]) -> Result<SecretValue, FetchError> {
    let Some((program, args)) = command.split_first() else {
        return Err(FetchError::Failed(
            "the command is empty: give the program and its arguments as a list".to_owned(),
        ));
    };
    let mut child = match Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            return Err(FetchError::Failed(format!(
                "the command {program} was not found"
            )))
        }
        Err(e) => return Err(failed(format_args!("the command {program}"), e)),
    };
    let read = match child.stdout.take() {
        Some(stdout) => SecretValue::from_reader(stdout.take(u64::MAX), DEFAULT_MAX_LEN),
        None => Err(std::io::Error::other("its output could not be read")),
    };
    let status = child
        .wait()
        .map_err(|e| failed(format_args!("the command {program}"), e))?;
    // The status first: what a failed command printed is not a secret to
    // keep, and is dropped (and wiped) here.
    if !status.success() {
        let how = match status.code() {
            Some(code) => format!("exited with status {code}"),
            None => "was stopped by a signal".to_owned(),
        };
        return Err(FetchError::Failed(format!("the command {program} {how}")));
    }
    let secret = read.map_err(|e| failed(format_args!("the command {program}"), e))?;
    if secret.is_empty() {
        return Err(FetchError::Missing(format!(
            "the command {program} printed nothing"
        )));
    }
    secret
        .without_trailing_newline()
        .map_err(|e| failed(format_args!("the command {program}"), e))
}
