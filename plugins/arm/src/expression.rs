//! ARM's template expression language, as far as telling a well-formed
//! expression from a broken one.
//!
//! A string value in a template is an expression when it starts with `[` and
//! ends with `]`, unless it starts with `[[`, which is how a template writes
//! literal text that happens to look like one. Resource Manager only reports
//! a malformed expression when the template is deployed; this module reports
//! it at compile time, against the line the author wrote.
//!
//! The grammar checked is the one Azure documents: function calls (with an
//! optional namespace), string literals in single quotes with `''` for a
//! quote, integers, and property and index access on a function's result.
//! Function names and argument counts are not checked: Azure adds functions,
//! and a template may define its own.

/// The expression inside a template string, or `None` when the string is
/// literal text.
pub fn inner(text: &str) -> Option<&str> {
    if text.len() >= 2 && text.starts_with('[') && text.ends_with(']') && !text.starts_with("[[") {
        Some(&text[1..text.len() - 1])
    } else {
        None
    }
}

/// Checks that an expression, given without its square brackets, is well
/// formed. The error says what was expected and where.
pub fn check(expression: &str) -> Result<(), String> {
    let mut parser = Parser {
        text: expression,
        at: 0,
    };
    parser.expression()?;
    parser.space();
    match parser.peek() {
        None => Ok(()),
        Some(c) => Err(parser.fail(&format!("unexpected {c:?} after the end of the expression"))),
    }
}

/// A single quoted string literal for `text`.
pub fn quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.text[self.at..].chars().next()
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek()?;
        self.at += c.len_utf8();
        Some(c)
    }

    fn eat(&mut self, c: char) -> bool {
        if self.peek() == Some(c) {
            self.at += c.len_utf8();
            true
        } else {
            false
        }
    }

    fn space(&mut self) {
        while self.peek().is_some_and(char::is_whitespace) {
            self.bump();
        }
    }

    fn fail(&self, what: &str) -> String {
        format!("{what} (at character {})", self.at + 1)
    }

    fn expression(&mut self) -> Result<(), String> {
        self.space();
        match self.peek() {
            Some('\'') => self.string()?,
            Some(c) if c.is_ascii_digit() || c == '-' => self.number()?,
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.call()?,
            Some(c) => return Err(self.fail(&format!("expected a value, found {c:?}"))),
            None => return Err(self.fail("expected a value, found the end of the expression")),
        }
        self.accessors()
    }

    fn string(&mut self) -> Result<(), String> {
        let start = self.at;
        self.bump();
        loop {
            match self.bump() {
                // `''` is a quote inside the literal.
                Some('\'') if self.eat('\'') => {}
                Some('\'') => return Ok(()),
                Some(_) => {}
                None => {
                    self.at = start;
                    return Err(self.fail("a string literal is never closed"));
                }
            }
        }
    }

    fn number(&mut self) -> Result<(), String> {
        self.eat('-');
        let mut digits = 0;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.bump();
            digits += 1;
        }
        if digits == 0 {
            return Err(self.fail("expected a digit"));
        }
        Ok(())
    }

    fn identifier(&mut self) -> Result<&str, String> {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
        {
            self.bump();
        }
        if self.at == start {
            return Err(self.fail("expected a name"));
        }
        Ok(&self.text[start..self.at])
    }

    /// `name(arguments)` or `namespace.name(arguments)`.
    fn call(&mut self) -> Result<(), String> {
        let start = self.at;
        let mut name = self.identifier()?.to_owned();
        if self.eat('.') {
            name.push('.');
            name.push_str(self.identifier()?);
        }
        self.space();
        if !self.eat('(') {
            self.at = start;
            return Err(self.fail(&format!(
                "{name:?} is not a value: a function is called with parentheses, and text is \
                 written in single quotes"
            )));
        }
        self.space();
        if self.eat(')') {
            return Ok(());
        }
        loop {
            self.expression()?;
            self.space();
            match self.bump() {
                Some(',') => {}
                Some(')') => return Ok(()),
                Some(c) => {
                    self.at -= c.len_utf8();
                    return Err(self.fail(&format!(
                        "expected a comma or a closing parenthesis in the call to {name}, found {c:?}"
                    )));
                }
                None => {
                    return Err(self.fail(&format!("the call to {name} is never closed")));
                }
            }
        }
    }

    /// `.property`, `[index]` and their null-tolerant `?.` and `?[` forms.
    fn accessors(&mut self) -> Result<(), String> {
        loop {
            self.space();
            let before = self.at;
            let safe = self.eat('?');
            if self.eat('.') {
                self.identifier()?;
            } else if self.eat('[') {
                self.expression()?;
                self.space();
                if !self.eat(']') {
                    return Err(self.fail("an index is never closed with a square bracket"));
                }
            } else if safe {
                return Err(self.fail("expected a property or an index after the question mark"));
            } else {
                self.at = before;
                return Ok(());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bracketed_string_is_an_expression() {
        assert_eq!(
            inner("[resourceGroup().location]"),
            Some("resourceGroup().location")
        );
        assert_eq!(inner("[[not one]"), None);
        assert_eq!(inner("[half"), None);
        assert_eq!(inner("plain"), None);
        assert_eq!(inner("["), None);
        assert_eq!(inner("[]"), Some(""));
    }

    #[test]
    fn well_formed_expressions_pass() {
        for expression in [
            "resourceGroup().location",
            "concat('st', uniqueString(resourceGroup().id))",
            "parameters('names')[0]",
            "reference(resourceId('Microsoft.Network/publicIPAddresses', 'web-ip'), '2023-09-01').ipAddress",
            "listKeys(variables('id'), '2023-01-01').keys[0].value",
            "format('{0}-{1}', 'it''s', copyIndex(1))",
            "if(equals(parameters('env'), 'prod'), 3, -1)",
            "sys.concat('a', 'b')",
            "contoso.uniqueName( 'a' )",
            "reference('x', '2023-01-01', 'Full')?.identity?['principalId']",
            "split('a/b', '/')[add(0, 1)]",
            "createObject()",
            " null() ",
        ] {
            assert_eq!(check(expression), Ok(()), "{expression}");
        }
    }

    #[test]
    fn broken_expressions_say_what_is_wrong_and_where() {
        for (expression, want) in [
            ("", "expected a value, found the end of the expression (at character 1)"),
            ("concat('a', 'b'", "the call to concat is never closed (at character 16)"),
            ("concat('a' 'b')", "expected a comma or a closing parenthesis in the call to concat, found '\\'' (at character 12)"),
            ("concat('a)", "a string literal is never closed (at character 8)"),
            ("resourceGroup.location", "\"resourceGroup.location\" is not a value: a function is called with parentheses, and text is written in single quotes (at character 1)"),
            ("parameters('a'))", "unexpected ')' after the end of the expression (at character 16)"),
            ("parameters('a')[0", "an index is never closed with a square bracket (at character 18)"),
            ("parameters('a').", "expected a name (at character 17)"),
            ("add(1, )", "expected a value, found ')' (at character 8)"),
            ("-x", "expected a digit (at character 2)"),
            ("a()?", "expected a property or an index after the question mark (at character 5)"),
            ("\"double\"", "expected a value, found '\"' (at character 1)"),
        ] {
            assert_eq!(check(expression), Err(want.to_owned()), "{expression}");
        }
    }

    #[test]
    fn a_quote_in_a_literal_is_doubled() {
        assert_eq!(quote("it's"), "'it''s'");
        assert_eq!(check(&quote("it's 'quoted'")), Ok(()));
    }
}
