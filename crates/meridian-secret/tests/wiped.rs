//! After a secret is dropped, its bytes are nowhere in the process.
//!
//! This reads the test process's own memory, every writable mapping of it,
//! and looks for the secret: first while it is held, to prove the search
//! can find one, then after it is dropped.
//!
//! It is alone in its file so that it is alone in its process.

#![cfg(target_os = "linux")]

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};

use meridian_secret::{Part, SecretFile, SecretValue};

/// A random secret of printable bytes, made in place. The test never holds
/// the value in ordinary memory: what it searches with is every byte of the
/// value inverted, so that the needle is not itself a match.
fn random_secret() -> (SecretValue, Vec<u8>) {
    let value = SecretValue::filled(48, |home| {
        File::open("/dev/urandom")?.read_exact(home)?;
        for byte in home.iter_mut() {
            *byte = b'a' + (*byte % 26);
        }
        Ok(())
    })
    .unwrap();
    let inverted = value.expose(|bytes| bytes.iter().map(|b| !b).collect());
    (value, inverted)
}

/// How many times the bytes whose inverse is `inverted` appear in this
/// process's writable memory.
fn occurrences(inverted: &[u8]) -> usize {
    let maps = std::fs::read_to_string("/proc/self/maps").unwrap();
    let mut memory = File::open("/proc/self/mem").unwrap();
    // One buffer, reused, and wiped at the end: it holds copies of memory,
    // and must not become the place a secret is found.
    let mut buffer = vec![0u8; 1 << 20];
    let own = buffer.as_ptr() as usize;
    let mut found = 0;
    for line in maps.lines() {
        let mut fields = line.split_whitespace();
        let (Some(range), Some(perms)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !perms.starts_with("rw") {
            continue;
        }
        let Some((start, end)) = range.split_once('-') else {
            continue;
        };
        let (Ok(start), Ok(end)) = (
            usize::from_str_radix(start, 16),
            usize::from_str_radix(end, 16),
        ) else {
            continue;
        };
        let mut at = start;
        while at < end {
            let want = (end - at).min(buffer.len());
            if memory.seek(SeekFrom::Start(at as u64)).is_err() {
                break;
            }
            let Ok(read) = memory.read(&mut buffer[..want]) else {
                break;
            };
            if read == 0 {
                break;
            }
            // Skip the buffer's own pages: they hold what was just read.
            let overlaps_buffer = at < own + buffer.len() && own < at + read;
            if !overlaps_buffer && read >= inverted.len() {
                for window in buffer[..read].windows(inverted.len()) {
                    if window[0] == !inverted[0]
                        && window.iter().zip(inverted).all(|(a, b)| *a == !*b)
                    {
                        found += 1;
                    }
                }
            }
            // Overlap the next read, so a value across a boundary is seen.
            at += read.saturating_sub(inverted.len() - 1).max(1);
            if read < want {
                break;
            }
        }
    }
    buffer.iter_mut().for_each(|b| *b = 0);
    found
}

#[test]
fn a_dropped_secret_is_nowhere_in_the_process() {
    let (value, inverted) = random_secret();

    // While it is held it is in exactly one place: its home.
    assert_eq!(
        occurrences(&inverted),
        1,
        "the search finds a held secret, once"
    );

    // Use it the ways a run does: copy it, derive from it, write it to a
    // file for a tool, mask output with it.
    {
        let copy = value.duplicate().unwrap();
        let derived = value
            .derive(value.len(), |from, home| home.copy_from_slice(from))
            .unwrap();
        assert!(copy.same(&derived));
        let file = SecretFile::create(
            "vars.json",
            &[
                Part::Text("{\"password\": \""),
                Part::Secret(&value),
                Part::Text("\"}"),
            ],
        )
        .unwrap();
        assert!(file.path().exists());
        let mut masker = meridian_secret::Masker::new();
        masker.add(&value).unwrap();
        assert_eq!(masker.mask("nothing to hide"), "nothing to hide");
        assert!(
            occurrences(&inverted) >= 3,
            "the copies are found while held"
        );
    }

    // The copies are gone; the original is still the only one.
    assert_eq!(occurrences(&inverted), 1);

    drop(value);
    assert_eq!(
        occurrences(&inverted),
        0,
        "a dropped secret left its bytes behind"
    );
}
