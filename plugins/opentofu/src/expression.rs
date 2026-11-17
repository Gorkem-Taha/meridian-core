//! Finding the document's own names inside an expression that is more than a
//! path.
//!
//! `${web_vm.public_ip}` is a path and is rewritten whole. But an author also
//! writes `${upper(web_vm.name)}` or `${enabled ? web_vm.id : null}`, and
//! there `web_vm` and `enabled` are just as much the document's resources.
//! Left alone they would reach OpenTofu as names it has never heard of, and
//! the author would learn so only at plan time.
//!
//! This is not a parser for OpenTofu's expression language. It reads just
//! enough to tell a name that stands for a value from one that does not: a
//! function being called, an attribute after a dot, a key of an object, a
//! name a `for` binds, and the text of a quoted string. A template inside a
//! quoted string is an expression again and is read the same way.

use std::collections::BTreeSet;

fn is_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_'
}

fn is_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '-'
}

/// One name found in an expression, by where it is.
struct Name {
    start: usize,
    end: usize,
}

/// Every name in the expression that stands for a value.
fn names(expr: &str) -> Vec<Name> {
    let mut found = Vec::new();
    let mut bound = BTreeSet::new();
    scan(expr, 0, &mut bound, &mut found);
    found
}

/// Scans one expression, which starts `base` bytes into the whole. `bound`
/// holds the names a `for <a>[, <b>] in` has bound so far; they shadow
/// anything outside, here and in any template nested in a string.
fn scan<'a>(expr: &'a str, base: usize, bound: &mut BTreeSet<&'a str>, found: &mut Vec<Name>) {
    let bytes: Vec<(usize, char)> = expr.char_indices().collect();
    let offset = |index: usize| bytes.get(index).map_or(expr.len(), |(at, _)| *at);
    let mut binding = false;
    // The last character that was not white space, to judge what a name
    // follows.
    let mut previous: Option<char> = None;
    let mut i = 0;
    while i < bytes.len() {
        let (at, c) = bytes[i];
        if c == '"' {
            // A quoted string, to its closing quote. Its text is not names,
            // but a template inside it is an expression again.
            i += 1;
            while i < bytes.len() && bytes[i].1 != '"' {
                let here = bytes[i].1;
                let next = bytes.get(i + 1).map(|(_, c)| *c);
                if here == '\\' {
                    i += 2;
                } else if here == '$' && next == Some('$') {
                    // A doubled dollar sign escapes the template after it.
                    i += 2;
                    if bytes.get(i).map(|(_, c)| *c) == Some('{') {
                        i += 1;
                    }
                } else if here == '$' && next == Some('{') {
                    let start = i + 2;
                    let mut depth = 1usize;
                    let mut end = start;
                    while end < bytes.len() {
                        match bytes[end].1 {
                            '{' => depth += 1,
                            '}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        end += 1;
                    }
                    scan(
                        &expr[offset(start)..offset(end)],
                        base + offset(start),
                        bound,
                        found,
                    );
                    i = end + 1;
                } else {
                    i += 1;
                }
            }
            i += 1;
            previous = Some('"');
            continue;
        }
        if !is_start(c) {
            if !c.is_whitespace() {
                previous = Some(c);
            }
            i += 1;
            continue;
        }
        let mut j = i;
        while j < bytes.len() && is_continue(bytes[j].1) {
            j += 1;
        }
        let end = offset(j);
        let word = &expr[at..end];
        // What follows the name, past any white space.
        let mut k = j;
        while k < bytes.len() && bytes[k].1.is_whitespace() {
            k += 1;
        }
        let next = bytes.get(k).map(|(_, c)| *c);
        let after_next = bytes.get(k + 1).map(|(_, c)| *c);

        let after_dot = previous == Some('.');
        let called = next == Some('(');
        let assigned = next == Some('=') && !matches!(after_next, Some('=' | '>'));
        let key = next == Some(':') && matches!(previous, Some('{' | ','));

        if word == "for" && !after_dot {
            binding = true;
        } else if word == "in" && binding {
            binding = false;
        } else if binding {
            bound.insert(word);
        } else if !(after_dot || called || assigned || key || bound.contains(word)) {
            found.push(Name {
                start: base + at,
                end: base + end,
            });
        }
        previous = Some('a');
        i = j;
    }
}

/// Rewrites each of the document's names in an expression through `address`,
/// which answers `Some` for a name it knows. Everything else is left exactly
/// as written.
pub(crate) fn rewrite_names(expr: &str, mut address: impl FnMut(&str) -> Option<String>) -> String {
    let mut out = String::with_capacity(expr.len());
    let mut copied = 0;
    for name in names(expr) {
        if let Some(replacement) = address(&expr[name.start..name.end]) {
            out.push_str(&expr[copied..name.start]);
            out.push_str(&replacement);
            copied = name.end;
        }
    }
    out.push_str(&expr[copied..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rewrite(expr: &str) -> String {
        rewrite_names(expr, |name| match name {
            "web_vm" => Some("aws_instance.web_vm".to_owned()),
            "region" => Some("var.region".to_owned()),
            "enabled" => Some("var.enabled".to_owned()),
            _ => None,
        })
    }

    #[test]
    fn a_name_inside_a_call_or_an_operator_is_rewritten() {
        for (written, want) in [
            ("upper(web_vm.name)", "upper(aws_instance.web_vm.name)"),
            (
                "enabled ? web_vm.id : null",
                "var.enabled ? aws_instance.web_vm.id : null",
            ),
            // A template inside a string is an expression again.
            ("\"${region}-a\"", "\"${var.region}-a\""),
            (
                "[for zone in [\"a\", \"b\"] : \"${region}${zone}\"]",
                "[for zone in [\"a\", \"b\"] : \"${var.region}${zone}\"]",
            ),
            (
                "\"${upper(\"${web_vm.name}\")}\"",
                "\"${upper(\"${aws_instance.web_vm.name}\")}\"",
            ),
            (
                "[region, web_vm.id]",
                "[var.region, aws_instance.web_vm.id]",
            ),
            ("region == \"us-east-1\"", "var.region == \"us-east-1\""),
            ("!enabled", "!var.enabled"),
            (
                "web_vm.tags[region]",
                "aws_instance.web_vm.tags[var.region]",
            ),
        ] {
            assert_eq!(rewrite(written), want, "{written}");
        }
    }

    #[test]
    fn a_name_that_does_not_stand_for_a_value_is_left_alone() {
        for written in [
            // A function of the same name.
            "region(1)",
            // An attribute of something else.
            "var.region",
            "module.net.region",
            "data.aws_region.current.region",
            // Keys of an object.
            "{region = 1}",
            "{region: 1}",
            "{a = 1, region: 2}",
            "merge({region = \"x\"}, local.tags)",
            // Inside a string.
            "\"region web_vm\"",
            // An escaped template is text.
            "\"$${region}\"",
            "\"say \\\"region\\\"\"",
            // Names OpenTofu knows and the document does not.
            "upper(var.image)",
            "length(local.zones) > 1",
        ] {
            assert_eq!(rewrite(written), written, "{written}");
        }
    }

    #[test]
    fn a_name_a_for_binds_shadows_the_documents() {
        assert_eq!(
            rewrite("[for region in var.regions : upper(region)]"),
            "[for region in var.regions : upper(region)]"
        );
        assert_eq!(
            rewrite("{for k, region in local.m : k => region if enabled}"),
            "{for k, region in local.m : k => region if var.enabled}"
        );
        // A value of an object, as opposed to its key, is still rewritten.
        assert_eq!(rewrite("{zone = region}"), "{zone = var.region}");
        assert_eq!(
            rewrite("enabled ? region : \"none\""),
            "var.enabled ? var.region : \"none\""
        );
    }
}
