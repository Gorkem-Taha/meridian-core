//! The little shell this target has to speak: splitting a command line into
//! words the way `sh` would, and quoting a word so `sh` reads it back whole.

/// Splits a command line into words, honouring quotes and backslashes, with
/// no expansion of any kind. It is how a command that does not ask for a
/// shell is turned into the argument list cloud-init runs directly.
pub(crate) fn split(line: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    // A pair of empty quotes is a word, so "started" is tracked apart from
    // "has characters".
    let mut started = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            }
            '\'' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => word.push(c),
                        None => return Err("a single quote is never closed".to_owned()),
                    }
                }
            }
            '"' => {
                started = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            // Inside double quotes a backslash escapes only
                            // these; before anything else it is itself.
                            Some(c @ ('"' | '\\' | '$' | '`')) => word.push(c),
                            Some(c) => {
                                word.push('\\');
                                word.push(c);
                            }
                            None => return Err("a double quote is never closed".to_owned()),
                        },
                        Some(c) => word.push(c),
                        None => return Err("a double quote is never closed".to_owned()),
                    }
                }
            }
            '\\' => {
                started = true;
                match chars.next() {
                    Some(c) => word.push(c),
                    None => return Err("it ends with a backslash that escapes nothing".to_owned()),
                }
            }
            c => {
                started = true;
                word.push(c);
            }
        }
    }
    if started {
        words.push(word);
    }
    Ok(words)
}

/// A word as `sh` must be given it to read it back unchanged.
pub(crate) fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_./=:,@%+".contains(c));
    if plain {
        word.to_owned()
    } else {
        format!("'{}'", word.replace('\'', "'\\''"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn words_split_on_whitespace_and_quotes_keep_it() {
        assert_eq!(split("a  b\tc").unwrap(), ["a", "b", "c"]);
        assert_eq!(split("  ").unwrap(), Vec::<String>::new());
        assert_eq!(
            split("echo 'a b'  \"c  d\" e\\ f").unwrap(),
            ["echo", "a b", "c  d", "e f"]
        );
        assert_eq!(split("x '' \"\"").unwrap(), ["x", "", ""]);
        assert_eq!(split("a'b c'd").unwrap(), ["ab cd"]);
    }

    #[test]
    fn nothing_is_expanded_and_escapes_follow_the_shell() {
        assert_eq!(
            split("echo $HOME *.txt").unwrap(),
            ["echo", "$HOME", "*.txt"]
        );
        assert_eq!(
            split(r#""a\"b" "c\nd" '\'"#).unwrap(),
            ["a\"b", "c\\nd", "\\"]
        );
    }

    #[test]
    fn an_unfinished_quote_or_escape_is_an_error() {
        assert!(split("echo 'a").unwrap_err().contains("single quote"));
        assert!(split("echo \"a").unwrap_err().contains("double quote"));
        assert!(split("echo a\\").unwrap_err().contains("backslash"));
    }

    #[test]
    fn a_word_is_quoted_only_when_it_has_to_be() {
        assert_eq!(quote("/srv/app"), "/srv/app");
        assert_eq!(quote("a b"), "'a b'");
        assert_eq!(quote(""), "''");
        assert_eq!(quote("it's"), "'it'\\''s'");
        assert_eq!(quote("$(rm -rf /)"), "'$(rm -rf /)'");
    }

    #[test]
    fn what_is_quoted_splits_back_to_itself() {
        for word in ["plain", "a b", "it's", "", "\"x\"", "a\\b", "$HOME", "é ü"] {
            assert_eq!(split(&quote(word)).unwrap(), [word], "{word:?}");
        }
    }
}
