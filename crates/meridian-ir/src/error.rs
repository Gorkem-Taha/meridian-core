use std::fmt;

/// A point inside a source document. `line` and `column` are 1-based; a zero
/// `line` means the position is unknown.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Position {
    pub file: String,
    pub doc_idx: usize,
    pub line: usize,
    pub column: usize,
}

impl Position {
    pub(crate) fn document(file: &str, doc_idx: usize) -> Self {
        Position {
            file: file.to_owned(),
            doc_idx,
            line: 0,
            column: 0,
        }
    }
}

impl fmt::Display for Position {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(if self.file.is_empty() {
            "<input>"
        } else {
            &self.file
        })?;
        if self.doc_idx > 0 {
            write!(f, "[doc {}]", self.doc_idx)?;
        }
        if self.line > 0 {
            write!(f, ":{}", self.line)?;
            if self.column > 0 {
                write!(f, ":{}", self.column)?;
            }
        }
        Ok(())
    }
}

/// A single located diagnostic.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Error {
    pub pos: Position,
    /// Dotted path to the offending field, when known. Empty otherwise.
    pub path: String,
    pub msg: String,
}

impl Error {
    pub(crate) fn new(pos: Position, path: impl Into<String>, msg: impl Into<String>) -> Self {
        Error {
            pos,
            path: path.into(),
            msg: msg.into(),
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: ", self.pos)?;
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        f.write_str(&self.msg)
    }
}

impl std::error::Error for Error {}

/// Every diagnostic one parse or validation pass found, ordered by source
/// position so repeated runs over the same input report them identically.
///
/// A list returned as an `Err` is never empty.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ErrorList(Vec<Error>);

impl ErrorList {
    pub(crate) fn new() -> Self {
        ErrorList(Vec::new())
    }

    pub(crate) fn push(&mut self, e: Error) {
        self.0.push(e);
    }

    pub(crate) fn extend(&mut self, other: ErrorList) {
        self.0.extend(other.0);
    }

    /// `Ok(value)` when nothing was recorded, otherwise the sorted list.
    pub(crate) fn finish<T>(mut self, value: T) -> Result<T, ErrorList> {
        if self.0.is_empty() {
            return Ok(value);
        }
        // Stable, so diagnostics at one position keep the order they were found in.
        self.0.sort_by(|a, b| a.pos.cmp(&b.pos));
        Err(self)
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Error> {
        self.0.iter()
    }

    pub fn first(&self) -> Option<&Error> {
        self.0.first()
    }
}

impl From<Error> for ErrorList {
    fn from(e: Error) -> Self {
        ErrorList(vec![e])
    }
}

impl<'a> IntoIterator for &'a ErrorList {
    type Item = &'a Error;
    type IntoIter = std::slice::Iter<'a, Error>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

impl IntoIterator for ErrorList {
    type Item = Error;
    type IntoIter = std::vec::IntoIter<Error>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl fmt::Display for ErrorList {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_slice() {
            [] => f.write_str("no errors"),
            [one] => one.fmt(f),
            many => {
                write!(f, "{} errors:", many.len())?;
                for e in many {
                    write!(f, "\n  {e}")?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for ErrorList {}
