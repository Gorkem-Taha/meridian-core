//! One run at a time.
//!
//! Two runs against the same state collide in ways no tool can untangle: two
//! `terraform apply`s in one directory, a plan read while an apply is half
//! done. So anything that runs a tool first takes the state's lock, and a
//! second run is refused, and told who holds it.
//!
//! Meridian never breaks a lock by itself. A process on another machine
//! cannot be seen to be dead, and guessing wrong is exactly the collision the
//! lock exists to prevent. A lock left by a run that died is removed by a
//! person, with [`force_unlock`], who has to name it.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{Duration, Instant};

use meridian_sdk::{canonical_map, decode_map, Value};

use crate::backend::Backend;

/// The key the lock is kept under.
pub const LOCK_KEY: &str = "lock";

/// Who holds the state, and for what.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LockInfo {
    /// What names this lock: the id of the run that took it.
    pub id: String,
    /// The operating system user, as far as it can be told.
    pub user: String,
    /// The machine.
    pub host: String,
    /// The process on that machine.
    pub pid: u32,
    /// What the run is doing, such as `apply site.yaml`.
    pub command: String,
    /// When it was taken, as RFC 3339 in UTC.
    pub since: String,
}

impl LockInfo {
    /// A lock for this process, named `id`.
    pub fn for_this_process(id: &str, command: &str) -> LockInfo {
        LockInfo {
            id: id.to_owned(),
            user: user(),
            host: host(),
            pid: std::process::id(),
            command: command.to_owned(),
            since: crate::time::now(),
        }
    }

    pub fn to_json(&self) -> String {
        let map: BTreeMap<String, Value> = [
            ("id", Value::from(self.id.as_str())),
            ("user", Value::from(self.user.as_str())),
            ("host", Value::from(self.host.as_str())),
            ("pid", Value::Uint(u64::from(self.pid))),
            ("command", Value::from(self.command.as_str())),
            ("since", Value::from(self.since.as_str())),
        ]
        .into_iter()
        .map(|(key, value)| (key.to_owned(), value))
        .collect();
        canonical_map(&map) + "\n"
    }

    /// Reads what [`LockInfo::to_json`] wrote. A lock that cannot be read is
    /// still a lock: it comes back with only the id `unreadable`, and is
    /// never treated as absent.
    pub fn from_json(bytes: &[u8]) -> LockInfo {
        let Ok(map) = decode_map(bytes) else {
            return LockInfo {
                id: "unreadable".to_owned(),
                ..LockInfo::default()
            };
        };
        let text = |name: &str| match map.get(name) {
            Some(Value::String(s)) => s.clone(),
            _ => String::new(),
        };
        let id = text("id");
        LockInfo {
            id: if id.is_empty() {
                "unreadable".to_owned()
            } else {
                id
            },
            user: text("user"),
            host: text("host"),
            pid: match map.get("pid") {
                Some(Value::Uint(n)) => u32::try_from(*n).unwrap_or(0),
                Some(Value::Int(n)) => u32::try_from(*n).unwrap_or(0),
                _ => 0,
            },
            command: text("command"),
            since: text("since"),
        }
    }
}

impl fmt::Display for LockInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "lock {}", self.id)?;
        if !self.command.is_empty() {
            write!(f, ", taken by `meridian {}`", self.command)?;
        }
        if !self.user.is_empty() || !self.host.is_empty() {
            write!(
                f,
                " ({}@{}, process {})",
                if self.user.is_empty() {
                    "?"
                } else {
                    &self.user
                },
                if self.host.is_empty() {
                    "?"
                } else {
                    &self.host
                },
                self.pid
            )?;
        }
        if !self.since.is_empty() {
            write!(f, " at {}", self.since)?;
        }
        Ok(())
    }
}

/// The operating system user, as far as the environment says.
pub fn user() -> String {
    ["USER", "LOGNAME", "USERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
        .unwrap_or_default()
}

/// This machine's name, as far as it can be told without asking the network.
pub fn host() -> String {
    ["HOSTNAME", "COMPUTERNAME"]
        .iter()
        .find_map(|name| std::env::var(name).ok().filter(|v| !v.is_empty()))
        .or_else(|| {
            std::fs::read_to_string("/etc/hostname")
                .ok()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty())
        })
        .unwrap_or_default()
}

/// Why a lock was not taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LockError {
    /// Another run holds it.
    Held(Box<LockInfo>),
    /// The store could not be asked.
    Backend(String),
}

impl fmt::Display for LockError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LockError::Held(holder) => write!(
                f,
                "the state is locked by another run: {holder}\n\
                 Wait for it to finish, or wait here with --lock-timeout. If that run is \
                 gone and will never release it, remove the lock with `meridian unlock {}`",
                holder.id
            ),
            LockError::Backend(why) => write!(f, "taking the state lock: {why}"),
        }
    }
}

/// Takes the lock, waiting up to `timeout` for another run to release it.
pub fn acquire(backend: &dyn Backend, info: &LockInfo, timeout: Duration) -> Result<(), LockError> {
    let deadline = Instant::now() + timeout;
    let mut pause = Duration::from_millis(100);
    loop {
        match backend.create(LOCK_KEY, info.to_json().as_bytes()) {
            Ok(true) => return Ok(()),
            Ok(false) => {}
            Err(why) => return Err(LockError::Backend(why)),
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(match holder(backend) {
                Ok(Some(holder)) => LockError::Held(Box::new(holder)),
                // Released between the two calls. The caller asked for no
                // more waiting, so say it was held and let them run again.
                Ok(None) => LockError::Held(Box::new(LockInfo {
                    id: "released-just-now".to_owned(),
                    ..LockInfo::default()
                })),
                Err(why) => LockError::Backend(why),
            });
        }
        std::thread::sleep(pause.min(deadline - now));
        pause = (pause * 2).min(Duration::from_secs(5));
    }
}

/// Who holds the lock, if anyone does.
pub fn holder(backend: &dyn Backend) -> Result<Option<LockInfo>, String> {
    Ok(backend
        .get(LOCK_KEY)?
        .map(|bytes| LockInfo::from_json(&bytes)))
}

/// Releases the lock `id` took. A lock that is someone else's by now is left
/// alone: it was removed by hand and taken again, and it is not ours.
pub fn release(backend: &dyn Backend, id: &str) -> Result<(), String> {
    match holder(backend)? {
        Some(held) if held.id == id => backend.delete(LOCK_KEY),
        _ => Ok(()),
    }
}

/// Removes a lock a run left behind. The id has to be given and has to
/// match, so the lock removed is the one the person looked at.
pub fn force_unlock(backend: &dyn Backend, id: &str) -> Result<LockInfo, String> {
    match holder(backend)? {
        None => Err("the state is not locked".to_owned()),
        Some(held) if held.id == id => {
            backend.delete(LOCK_KEY)?;
            Ok(held)
        }
        Some(held) => Err(format!("the state is locked, but not by {id:?}: {held}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::MemoryBackend;

    fn info(id: &str) -> LockInfo {
        LockInfo {
            id: id.to_owned(),
            user: "mike".to_owned(),
            host: "build-07".to_owned(),
            pid: 4242,
            command: "apply site.yaml".to_owned(),
            since: "2026-11-17T09:14:03.000Z".to_owned(),
        }
    }

    #[test]
    fn a_second_run_is_refused_and_told_who_holds_the_lock() {
        let backend = MemoryBackend::new();
        acquire(&backend, &info("first"), Duration::ZERO).unwrap();
        let err = acquire(&backend, &info("second"), Duration::ZERO).unwrap_err();
        assert_eq!(err, LockError::Held(Box::new(info("first"))));
        let said = err.to_string();
        assert!(said.contains("mike@build-07, process 4242"), "{said}");
        assert!(said.contains("`meridian apply site.yaml`"), "{said}");
        assert!(said.contains("meridian unlock first"), "{said}");

        release(&backend, "first").unwrap();
        acquire(&backend, &info("second"), Duration::ZERO).unwrap();
    }

    #[test]
    fn waiting_takes_the_lock_once_it_is_released() {
        let backend = std::sync::Arc::new(MemoryBackend::new());
        acquire(backend.as_ref(), &info("first"), Duration::ZERO).unwrap();
        let other = backend.clone();
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            release(other.as_ref(), "first").unwrap();
        });
        acquire(backend.as_ref(), &info("second"), Duration::from_secs(10)).unwrap();
        releaser.join().unwrap();
        assert_eq!(holder(backend.as_ref()).unwrap().unwrap().id, "second");

        let started = Instant::now();
        let err = acquire(backend.as_ref(), &info("third"), Duration::from_millis(250));
        assert!(matches!(err, Err(LockError::Held(h)) if h.id == "second"));
        assert!(started.elapsed() >= Duration::from_millis(250));
    }

    #[test]
    fn releasing_never_removes_another_runs_lock() {
        let backend = MemoryBackend::new();
        acquire(&backend, &info("first"), Duration::ZERO).unwrap();
        release(&backend, "someone-else").unwrap();
        assert_eq!(holder(&backend).unwrap().unwrap().id, "first");
    }

    #[test]
    fn a_lock_is_only_removed_by_hand_when_named() {
        let backend = MemoryBackend::new();
        assert_eq!(
            force_unlock(&backend, "first").unwrap_err(),
            "the state is not locked"
        );
        acquire(&backend, &info("first"), Duration::ZERO).unwrap();
        let err = force_unlock(&backend, "wrong").unwrap_err();
        assert!(err.contains("not by \"wrong\""), "{err}");
        assert!(holder(&backend).unwrap().is_some());
        assert_eq!(force_unlock(&backend, "first").unwrap(), info("first"));
        assert!(holder(&backend).unwrap().is_none());
    }

    #[test]
    fn a_lock_that_cannot_be_read_still_holds() {
        let backend = MemoryBackend::new();
        backend.put(LOCK_KEY, b"not json").unwrap();
        let err = acquire(&backend, &info("second"), Duration::ZERO).unwrap_err();
        assert!(matches!(err, LockError::Held(h) if h.id == "unreadable"));
        assert_eq!(
            LockInfo::from_json(info("x").to_json().as_bytes()),
            info("x")
        );
    }
}
