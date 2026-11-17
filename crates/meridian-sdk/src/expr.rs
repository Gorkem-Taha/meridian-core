//! The one piece of expression syntax a document can carry into a plugin: a
//! reference to something only the tool can know, written `${resource.path}`.
//!
//! An infrastructure document uses it for outputs (`${web_vm.public_ip}`) and
//! for wiring one resource to another. Each tool spells such a reference
//! differently, so the plugin rewrites it; this module only finds them.
//!
//! `$${` is the escape for a literal `${`.

/// One piece of a string that may hold references.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Part {
    /// Literal text, with any `$${` already turned back into `${`.
    Text(String),
    /// What stood between `${` and its closing brace.
    Expr(String),
}

/// Splits a string into literal text and `${...}` expressions.
///
/// Braces inside an expression are balanced, so `${merge({a = 1}, x)}` is one
/// expression. An unclosed `${` is an error, since no tool would read it the
/// way the author meant.
pub fn parse(text: &str) -> Result<Vec<Part>, String> {
    let mut parts = Vec::new();
    let mut literal = String::new();
    let mut rest = text;
    while let Some(at) = rest.find("${") {
        // `$${` is a literal `${`.
        if rest[..at].ends_with('$') {
            literal.push_str(&rest[..at - 1]);
            literal.push_str("${");
            rest = &rest[at + 2..];
            continue;
        }
        literal.push_str(&rest[..at]);
        let body = &rest[at + 2..];
        let mut depth = 1usize;
        let mut end = None;
        for (i, c) in body.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = Some(i);
                        break;
                    }
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            return Err(format!("{text:?} opens a ${{ that is never closed"));
        };
        if !literal.is_empty() {
            parts.push(Part::Text(std::mem::take(&mut literal)));
        }
        parts.push(Part::Expr(body[..end].trim().to_owned()));
        rest = &body[end + 1..];
    }
    literal.push_str(rest);
    if !literal.is_empty() {
        parts.push(Part::Text(literal));
    }
    Ok(parts)
}

/// Whether a string holds any `${...}` expression at all.
pub fn has_expr(text: &str) -> bool {
    parse(text).is_ok_and(|parts| parts.iter().any(|p| matches!(p, Part::Expr(_))))
}

/// Splits an expression into its leading name and what follows it:
/// `web_vm.public_ip` is `("web_vm", ".public_ip")`, `subnets[0].id` is
/// `("subnets", "[0].id")`. The name is empty when the expression does not
/// start with one.
pub fn head(expr: &str) -> (&str, &str) {
    let end = expr
        .char_indices()
        .find(|(_, c)| !(c.is_ascii_alphanumeric() || *c == '_' || *c == '-'))
        .map_or(expr.len(), |(i, _)| i);
    expr.split_at(end)
}

/// Whether an expression is nothing but a path: a name followed by `.field`
/// and `[index]` steps. Anything else, such as a function call, is the tool's
/// own expression language.
pub fn is_path(expr: &str) -> bool {
    let (name, rest) = head(expr);
    !name.is_empty()
        && rest
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-[]\"'*".contains(c))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(s: &str) -> Part {
        Part::Text(s.to_owned())
    }
    fn expr(s: &str) -> Part {
        Part::Expr(s.to_owned())
    }

    #[test]
    fn references_are_found_among_literal_text() {
        assert_eq!(parse("plain").unwrap(), [text("plain")]);
        assert_eq!(parse("${vm.ip}").unwrap(), [expr("vm.ip")]);
        assert_eq!(
            parse("http://${ vm.ip }:${port}/x").unwrap(),
            [
                text("http://"),
                expr("vm.ip"),
                text(":"),
                expr("port"),
                text("/x")
            ]
        );
        assert_eq!(parse("").unwrap(), []);
    }

    #[test]
    fn a_doubled_dollar_is_a_literal_and_braces_balance() {
        assert_eq!(parse("say $${NAME} $5").unwrap(), [text("say ${NAME} $5")]);
        assert_eq!(
            parse("${merge({a = 1}, x)}").unwrap(),
            [expr("merge({a = 1}, x)")]
        );
        assert!(parse("${vm.ip").unwrap_err().contains("never closed"));
        assert!(has_expr("a ${b}"));
        assert!(!has_expr("a $${b}"));
    }

    #[test]
    fn an_expression_splits_into_its_name_and_path() {
        assert_eq!(head("web_vm.public_ip"), ("web_vm", ".public_ip"));
        assert_eq!(head("subnets[0].id"), ("subnets", "[0].id"));
        assert_eq!(head("vm"), ("vm", ""));
        assert_eq!(head("(1 + 2)"), ("", "(1 + 2)"));
        assert!(is_path("web_vm.public_ip"));
        assert!(is_path("lb.status.ingress[0].ip"));
        assert!(!is_path("upper(vm.name)"));
        assert!(!is_path("a ? b : c"));
    }
}
