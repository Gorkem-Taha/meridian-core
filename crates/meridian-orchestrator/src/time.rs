//! Timestamps and run ids, without a calendar library.
//!
//! The record needs two things from a clock: a time a person and a log
//! system can both read, and an id that sorts in the order runs happened.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use sha2::{Digest, Sha256};

/// The civil date of a day counted from 1970-01-01, by Howard Hinnant's
/// `civil_from_days`.
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

struct Parts {
    year: i64,
    month: u32,
    day: u32,
    hour: u64,
    minute: u64,
    second: u64,
    millis: u32,
}

fn parts(at: SystemTime) -> Parts {
    let since = at.duration_since(UNIX_EPOCH).unwrap_or_default();
    let secs = since.as_secs();
    let (year, month, day) = civil((secs / 86_400) as i64);
    let of_day = secs % 86_400;
    Parts {
        year,
        month,
        day,
        hour: of_day / 3_600,
        minute: of_day % 3_600 / 60,
        second: of_day % 60,
        millis: since.subsec_millis(),
    }
}

/// `at` as RFC 3339 in UTC, to the millisecond: `2026-11-17T09:15:11.204Z`.
pub fn rfc3339(at: SystemTime) -> String {
    let p = parts(at);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        p.year, p.month, p.day, p.hour, p.minute, p.second, p.millis
    )
}

/// The time now, as [`rfc3339`].
pub fn now() -> String {
    rfc3339(SystemTime::now())
}

/// A new run id: the second it started, then the millisecond within that
/// second as three hexadecimal digits and one more digit to keep apart two
/// runs started in the same millisecond, as in `2026-11-17T09-14-03Z-0cc7`.
///
/// Ids sort in the order the runs started, which is what "the last run"
/// means, and hold no character a file name or an object key cannot. Runs
/// that keep a record hold the state's lock one at a time, so two of them
/// never start in the same millisecond.
pub fn run_id() -> String {
    // Within one process the clock is only ever read forwards, so two ids
    // made in the same millisecond still differ and still sort.
    static LAST: AtomicU64 = AtomicU64::new(0);
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let mut millis = wall;
    let _ = LAST.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |last| {
        millis = wall.max(last + 1);
        Some(millis)
    });
    let p = parts(UNIX_EPOCH + std::time::Duration::from_millis(millis));
    let mut hasher = Sha256::new();
    hasher.update(millis.to_be_bytes());
    hasher.update(std::process::id().to_be_bytes());
    let digest = hasher.finalize();
    format!(
        "{:04}-{:02}-{:02}T{:02}-{:02}-{:02}Z-{:03x}{:x}",
        p.year,
        p.month,
        p.day,
        p.hour,
        p.minute,
        p.second,
        p.millis,
        digest[0] & 0xf
    )
}

/// Whether `text` has the shape of a run id. Ids arrive from the command
/// line and become part of a key, so anything else is refused.
pub fn is_run_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn a_time_is_written_as_rfc_3339_in_utc() {
        let at = |secs: u64, millis: u64| {
            rfc3339(UNIX_EPOCH + Duration::from_secs(secs) + Duration::from_millis(millis))
        };
        assert_eq!(at(0, 0), "1970-01-01T00:00:00.000Z");
        assert_eq!(at(951_782_400, 7), "2000-02-29T00:00:00.007Z");
        assert_eq!(at(1_794_906_911, 204), "2026-11-17T09:15:11.204Z");
        assert_eq!(at(4_102_444_799, 999), "2099-12-31T23:59:59.999Z");
    }

    #[test]
    fn run_ids_are_distinct_sortable_and_safe_in_a_key() {
        let a = run_id();
        let b = run_id();
        assert_ne!(a, b);
        assert!(is_run_id(&a), "{a}");
        assert_eq!(a.len(), "2026-11-17T09-14-03Z-7f3a".len());
        // A later run sorts after an earlier one, within the same second too.
        let c = run_id();
        assert!(a < b && b < c, "{a} {b} {c}");
        assert!(!is_run_id("../state"));
        assert!(!is_run_id(""));
    }
}
