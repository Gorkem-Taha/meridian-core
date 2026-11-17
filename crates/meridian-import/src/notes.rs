//! Collecting what an importer has to say, and naming what it imports.

use std::collections::BTreeSet;

use meridian_sdk::{ImportNote, NoteLevel};

/// The notes of one import, in the order they were raised.
///
/// An importer calls [`Notes::untranslated`] for anything the source says
/// that the document does not, [`Notes::changed`] for anything the document
/// says differently, and [`Notes::info`] for a faithful translation worth
/// explaining. Dropping something without one of the first two is the one
/// thing an importer must never do.
#[derive(Clone, Debug, Default)]
pub struct Notes {
    notes: Vec<ImportNote>,
    document: String,
}

impl Notes {
    pub fn new() -> Self {
        Notes::default()
    }

    /// Names the document the notes that follow belong to.
    pub fn in_document(&mut self, document: &str) {
        self.document = document.to_owned();
    }

    fn push(&mut self, level: NoteLevel, origin: &str, subject: &str, resource: &str, msg: String) {
        self.notes.push(ImportNote {
            level,
            origin: origin.to_owned(),
            subject: subject.to_owned(),
            msg,
            document: self.document.clone(),
            resource: resource.to_owned(),
        });
    }

    /// Something the source says that the document does not. `resource` is
    /// the id of the imported resource it belongs beside, or empty.
    pub fn untranslated(
        &mut self,
        origin: &str,
        subject: &str,
        resource: &str,
        msg: impl Into<String>,
    ) {
        self.push(NoteLevel::Untranslated, origin, subject, resource, msg.into());
    }

    /// Something the document says differently from the source.
    pub fn changed(&mut self, origin: &str, subject: &str, resource: &str, msg: impl Into<String>) {
        self.push(NoteLevel::Changed, origin, subject, resource, msg.into());
    }

    /// A faithful translation worth explaining.
    pub fn info(&mut self, origin: &str, subject: &str, resource: &str, msg: impl Into<String>) {
        self.push(NoteLevel::Info, origin, subject, resource, msg.into());
    }

    pub fn into_vec(self) -> Vec<ImportNote> {
        self.notes
    }
}

/// Turns any text into something usable as a resource id or a document name:
/// lower-case letters, digits and `separator`, starting with a letter.
pub fn slug(text: &str, separator: char) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in text.chars() {
        if c.is_ascii_alphanumeric() {
            if gap && !out.is_empty() {
                out.push(separator);
            }
            gap = false;
            out.push(c.to_ascii_lowercase());
        } else {
            gap = true;
        }
    }
    if out.is_empty() || !out.starts_with(|c: char| c.is_ascii_lowercase()) {
        out.insert_str(0, if out.is_empty() { "item" } else { "x" });
    }
    out
}

/// Returns `base` if nothing has taken it, and otherwise `base_2`, `base_3`
/// and so on. The name returned is recorded as taken.
pub fn unique_id(base: &str, taken: &mut BTreeSet<String>) -> String {
    if taken.insert(base.to_owned()) {
        return base.to_owned();
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}_{n}");
        if taken.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_slug_is_lower_case_words_joined_by_the_separator() {
        assert_eq!(slug("Install nginx (web)", '_'), "install_nginx_web");
        assert_eq!(slug("  web-server.conf ", '-'), "web-server-conf");
        assert_eq!(slug("80/tcp", '_'), "x80_tcp");
        assert_eq!(slug("!!", '_'), "item");
    }

    #[test]
    fn a_taken_id_gets_a_number() {
        let mut taken = BTreeSet::new();
        assert_eq!(unique_id("web", &mut taken), "web");
        assert_eq!(unique_id("web", &mut taken), "web_2");
        assert_eq!(unique_id("web", &mut taken), "web_3");
        assert_eq!(unique_id("db", &mut taken), "db");
    }
}
