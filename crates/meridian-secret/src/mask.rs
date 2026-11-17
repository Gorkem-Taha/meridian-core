//! The output filter: every line a tool prints is searched for the secrets
//! of the run, and what is found is masked before the line is shown, logged
//! or sent anywhere.
//!
//! This is the second line of defence. The first is that Meridian's own
//! structures never hold a secret's bytes; this one is for a tool that
//! echoes what it was given.

use base64::engine::Config;
use base64::Engine;

use crate::{SecretValue, MASK};

/// A needle shorter than this is not searched for: masking every `a` in a
/// tool's output hides nothing and destroys the output.
const SHORTEST: usize = 4;

/// The secrets of a run, in every form a tool is likely to print one.
#[derive(Default)]
pub struct Masker {
    /// Each is held as a secret itself: an encoded form of a secret is one.
    needles: Vec<SecretValue>,
}

impl std::fmt::Debug for Masker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Masker({} forms)", self.needles.len())
    }
}

fn hex(from: &[u8], home: &mut [u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    for (i, byte) in from.iter().enumerate() {
        home[2 * i] = DIGITS[usize::from(byte >> 4)];
        home[2 * i + 1] = DIGITS[usize::from(byte & 15)];
    }
}

pub(crate) fn url_len(from: &[u8]) -> usize {
    from.iter()
        .map(|b| if unreserved(*b) { 1 } else { 3 })
        .sum()
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~')
}

pub(crate) fn url(from: &[u8], home: &mut [u8]) {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    let mut at = 0;
    for byte in from {
        if unreserved(*byte) {
            home[at] = *byte;
            at += 1;
        } else {
            home[at] = b'%';
            home[at + 1] = DIGITS[usize::from(byte >> 4)];
            home[at + 2] = DIGITS[usize::from(byte & 15)];
            at += 3;
        }
    }
}

impl Masker {
    pub fn new() -> Masker {
        Masker::default()
    }

    /// Adds a secret, and the forms of it a tool may print: base64 (both
    /// alphabets, with and without padding), hex, URL-encoded, and each of
    /// its lines when it has several, since a tool that echoes a key prints
    /// it line by line.
    ///
    /// Returns `false` when the secret is too short to search for, so that
    /// the caller can say so: such a value cannot be masked in what a tool
    /// prints.
    pub fn add(&mut self, secret: &SecretValue) -> std::io::Result<bool> {
        if secret.len() < SHORTEST {
            return Ok(false);
        }
        self.needles.push(secret.duplicate()?);

        for engine in [
            &base64::engine::general_purpose::STANDARD,
            &base64::engine::general_purpose::STANDARD_NO_PAD,
            &base64::engine::general_purpose::URL_SAFE,
            &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            let len =
                base64::encoded_len(secret.len(), engine.config().encode_padding()).unwrap_or(0);
            let encoded = secret.derive(len, |from, home| {
                // The length was computed for this engine, so it fits.
                let _ = engine.encode_slice(from, home);
            })?;
            self.push_new(encoded);
        }
        self.push_new(secret.derive(secret.len() * 2, hex)?);
        let encoded_len = secret.expose(url_len);
        if encoded_len != secret.len() {
            self.push_new(secret.derive(encoded_len, url)?);
        }

        // Each line of a secret that has several.
        let lines: Vec<(usize, usize)> = secret.expose(|bytes| {
            let mut found = Vec::new();
            let mut start = 0;
            for (i, byte) in bytes.iter().enumerate() {
                if *byte == b'\n' {
                    found.push((start, i));
                    start = i + 1;
                }
            }
            if start > 0 {
                found.push((start, bytes.len()));
            }
            found
        });
        for (start, end) in lines {
            let end = secret.expose(|bytes| {
                if end > start && bytes[end - 1] == b'\r' {
                    end - 1
                } else {
                    end
                }
            });
            if end - start >= SHORTEST {
                let line = secret.derive(end - start, |from, home| {
                    home.copy_from_slice(&from[start..end]);
                })?;
                self.push_new(line);
            }
        }
        // The longest first, so that a secret is masked whole before a
        // shorter form of it could match inside it.
        self.needles.sort_by_key(|n| std::cmp::Reverse(n.len()));
        Ok(true)
    }

    fn push_new(&mut self, needle: SecretValue) {
        if needle.len() >= SHORTEST && !self.needles.iter().any(|have| have.same(&needle)) {
            self.needles.push(needle);
        }
    }

    /// Whether nothing would be masked.
    pub fn is_empty(&self) -> bool {
        self.needles.is_empty()
    }

    /// `text` with every form of every secret replaced by the mask.
    pub fn mask(&self, text: &str) -> String {
        String::from_utf8_lossy(&self.mask_bytes(text.as_bytes())).into_owned()
    }

    /// [`Masker::mask`], for output that may not be text.
    pub fn mask_bytes(&self, text: &[u8]) -> Vec<u8> {
        // Which bytes of the text are part of a secret.
        let mut hidden = vec![false; text.len()];
        let mut any = false;
        for needle in &self.needles {
            needle.expose(|needle| {
                if needle.len() > text.len() {
                    return;
                }
                let mut at = 0;
                while at + needle.len() <= text.len() {
                    if &text[at..at + needle.len()] == needle {
                        hidden[at..at + needle.len()].fill(true);
                        any = true;
                        at += needle.len();
                    } else {
                        at += 1;
                    }
                }
            });
        }
        if !any {
            return text.to_vec();
        }
        let mut out = Vec::with_capacity(text.len());
        let mut i = 0;
        while i < text.len() {
            if hidden[i] {
                out.extend_from_slice(MASK.as_bytes());
                while i < text.len() && hidden[i] {
                    i += 1;
                }
            } else {
                out.push(text[i]);
                i += 1;
            }
        }
        out
    }

    /// Whether `text` holds any form of any secret.
    pub fn finds(&self, text: &[u8]) -> bool {
        self.needles.iter().any(|needle| {
            needle.expose(|needle| {
                needle.len() <= text.len() && text.windows(needle.len()).any(|w| w == needle)
            })
        })
    }
}
