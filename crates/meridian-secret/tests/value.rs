//! A secret value: it has no text form, it is reached only through
//! `expose`, it is never longer than it may be, and what it is built from is
//! wiped.

use std::io::Read;

use meridian_secret::{SecretValue, MASK};

fn held(text: &str) -> SecretValue {
    SecretValue::from_reader(text.as_bytes(), 1024).unwrap()
}

#[test]
fn a_secret_has_no_text_form() {
    let value = held("correct horse battery staple");
    for shown in [
        format!("{value}"),
        format!("{value:?}"),
        format!("{value:#?}"),
        format!("{value:>40}"),
        value.to_string(),
        format!("{:?}", Some(&value)),
        format!("{:?}", vec![&value]),
        format!("{:?}", ("db", &value)),
    ] {
        assert!(shown.contains(MASK), "{shown}");
        assert!(!shown.contains("horse"), "{shown}");
    }

    // Inside a structure that derives Debug, as it would be in a report.
    #[derive(Debug)]
    #[allow(dead_code)]
    struct Credential<'a> {
        user: &'a str,
        password: &'a SecretValue,
    }
    let shown = format!(
        "{:?}",
        Credential {
            user: "deploy",
            password: &value
        }
    );
    assert_eq!(
        shown,
        "Credential { user: \"deploy\", password: (sensitive) }"
    );
}

#[test]
fn the_bytes_are_reached_only_through_expose() {
    let value = held("s3cr3t");
    assert_eq!(value.len(), 6);
    assert!(!value.is_empty());
    assert!(value.expose(|bytes| bytes == b"s3cr3t"));
    // What the closure returns is the caller's business; a length is fine.
    assert_eq!(value.expose(<[u8]>::len), 6);
    assert!(held("").is_empty());
}

#[test]
fn a_reader_is_read_to_its_end_whatever_it_hands_over_at_a_time() {
    /// A reader that gives one byte per call, as a slow pipe may.
    struct Trickle<'a>(&'a [u8]);
    impl Read for Trickle<'_> {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            let Some((first, rest)) = self.0.split_first() else {
                return Ok(0);
            };
            buf[0] = *first;
            self.0 = rest;
            Ok(1)
        }
    }
    let text = "a value that arrives slowly, one byte at a time";
    let value = SecretValue::from_reader(Trickle(text.as_bytes()), 1024).unwrap();
    assert!(value.expose(|bytes| bytes == text.as_bytes()));

    // Binary is a secret too.
    let binary: Vec<u8> = (0..=255).collect();
    let value = SecretValue::from_reader(&binary[..], 1024).unwrap();
    assert!(value.expose(|bytes| bytes == &binary[..]));
}

#[test]
fn a_value_longer_than_allowed_is_refused_and_not_cut_short() {
    let exactly = SecretValue::from_reader(&[7u8; 64][..], 64).unwrap();
    assert_eq!(exactly.len(), 64);
    let err = SecretValue::from_reader(&[7u8; 65][..], 64).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(
        err.to_string().contains("longer than the 64 bytes"),
        "{err}"
    );
}

#[test]
fn a_reader_that_fails_gives_its_error_and_no_secret() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("the pipe broke"))
        }
    }
    let err = SecretValue::from_reader(Broken, 64).unwrap_err();
    assert_eq!(err.to_string(), "the pipe broke");
}

#[test]
fn taking_a_secret_from_ordinary_memory_wipes_that_memory() {
    let mut ordinary = b"left behind?".to_vec();
    let value = SecretValue::take_from(&mut ordinary).unwrap();
    assert!(ordinary.iter().all(|b| *b == 0), "{ordinary:?}");
    assert_eq!(ordinary.len(), 12);
    assert!(value.expose(|bytes| bytes == b"left behind?"));
}

#[test]
fn two_secrets_are_compared_by_name_and_never_with_equals() {
    assert!(held("same").same(&held("same")));
    assert!(!held("same").same(&held("sane")));
    assert!(!held("same").same(&held("same ")));
    assert!(held("").same(&held("")));
}

#[test]
fn a_copy_is_asked_for_and_stands_on_its_own() {
    let first = held("one of two");
    let second = first.duplicate().unwrap();
    assert_ne!(first.address_for_tests(), second.address_for_tests());
    drop(first);
    assert!(second.expose(|bytes| bytes == b"one of two"));
}

#[test]
fn a_secret_is_derived_into_protected_memory() {
    let value = held("abc");
    let doubled = value
        .derive(6, |from, home| {
            home[..3].copy_from_slice(from);
            home[3..].copy_from_slice(from);
        })
        .unwrap();
    assert!(doubled.expose(|bytes| bytes == b"abcabc"));
    assert_eq!(format!("{doubled:?}"), MASK);
}

#[test]
fn one_trailing_line_ending_is_not_part_of_the_secret() {
    for (given, want) in [
        ("token\n", "token"),
        ("token\r\n", "token"),
        ("token", "token"),
        ("token\n\n", "token\n"),
        ("two\nlines\n", "two\nlines"),
        ("\n", ""),
        ("", ""),
    ] {
        let value = held(given).without_trailing_newline().unwrap();
        assert!(value.expose(|bytes| bytes == want.as_bytes()), "{given:?}");
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::*;

    /// The lines of /proc/self/smaps for the mapping holding `address`.
    fn mapping_of(address: usize) -> Vec<String> {
        let smaps = std::fs::read_to_string("/proc/self/smaps").unwrap();
        let mut current: Vec<String> = Vec::new();
        let mut holds = false;
        for line in smaps.lines() {
            let range = line.split(' ').next().and_then(|r| r.split_once('-'));
            let parsed = range.and_then(|(a, b)| {
                Some((
                    usize::from_str_radix(a, 16).ok()?,
                    usize::from_str_radix(b, 16).ok()?,
                ))
            });
            if let Some((start, end)) = parsed {
                if holds {
                    return current;
                }
                current.clear();
                holds = (start..end).contains(&address);
            }
            current.push(line.to_owned());
        }
        assert!(holds, "no mapping holds {address:#x}");
        current
    }

    fn flags(mapping: &[String]) -> Vec<&str> {
        mapping
            .iter()
            .find_map(|l| l.strip_prefix("VmFlags:"))
            .map(|f| f.split_whitespace().collect())
            .unwrap_or_default()
    }

    #[test]
    fn the_memory_is_guarded_and_left_out_of_core_dumps() {
        let value = held("where does this live?");
        let protection = value.protection();
        assert!(protection.guarded);
        assert!(protection.undumpable);

        let mapping = mapping_of(value.address_for_tests());
        assert!(mapping[0].contains(" rw-p "), "{}", mapping[0]);
        let flags = flags(&mapping);
        assert!(flags.contains(&"dd"), "not marked do-not-dump: {flags:?}");
        assert!(flags.contains(&"wf"), "not marked wipe-on-fork: {flags:?}");

        // The byte after the secret's last one is in a page nothing can
        // touch: the next mapping starts where this one ends and has no
        // permissions.
        let end = value.address_for_tests() + value.len();
        let after = mapping_of(end);
        assert!(after[0].contains(" ---p "), "{}", after[0]);
    }

    #[test]
    fn the_memory_is_locked_when_the_system_allows_and_says_so_when_it_does_not() {
        let value = held("never to swap");
        let mapping = mapping_of(value.address_for_tests());
        let flags = flags(&mapping);
        // The claim is that the answer is truthful, in both directions: a
        // system with no room left to lock memory must not be reported as
        // having locked it.
        assert_eq!(
            value.protection().locked,
            flags.contains(&"lo"),
            "{flags:?}"
        );
        if value.protection().locked {
            let locked_kb = mapping
                .iter()
                .find_map(|l| l.strip_prefix("Locked:"))
                .and_then(|l| l.trim().trim_end_matches(" kB").trim().parse::<u64>().ok());
            assert!(locked_kb.is_some_and(|kb| kb > 0), "{mapping:#?}");
        }
    }
}
