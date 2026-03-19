use std::path::PathBuf;
use thiserror::Error;

use crate::ast::Span;

/// All errors that can be produced by `cddlc-parser`.
#[derive(Debug, Error)]
pub enum ParseError {
    #[error("{path}:{line}:{col}: {message}")]
    Syntax {
        path:    PathBuf,
        line:    usize,
        col:     usize,
        message: String,
        span:    Span,
    },

    #[error("{path}: unknown pragma '@{keyword}' — ignored")]
    UnknownPragma {
        path:    PathBuf,
        line:    usize,
        keyword: String,
    },

    #[error("{path}: pragma '@{keyword}' has invalid value: {reason}")]
    BadPragmaValue {
        path:    PathBuf,
        line:    usize,
        keyword: String,
        reason:  String,
    },

    #[error("I/O error reading '{path}': {source}")]
    Io {
        path:   PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("import cycle detected: {}", .cycle.join(" → "))]
    ImportCycle { cycle: Vec<String> },
}

/// Non-fatal diagnostic (warning-level).
#[derive(Debug, Clone)]
pub struct ParseWarning {
    pub path:    PathBuf,
    pub line:    usize,
    pub col:     usize,
    pub message: String,
    pub kind:    WarningKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WarningKind {
    UnknownPragma,
    DeprecatedRule,
    UnreachableChoice,
}

impl std::fmt::Display for ParseWarning {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "warning[{:?}] {}:{}:{}: {}",
               self.kind, self.path.display(), self.line, self.col, self.message)
    }
}

/// Accumulated result of parsing: the module plus any non-fatal warnings.
pub struct ParseResult<T> {
    pub value:    T,
    pub warnings: Vec<ParseWarning>,
}

impl<T> ParseResult<T> {
    pub fn new(value: T) -> Self {
        Self { value, warnings: Vec::new() }
    }

    pub fn with_warnings(value: T, warnings: Vec<ParseWarning>) -> Self {
        Self { value, warnings }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> ParseResult<U> {
        ParseResult { value: f(self.value), warnings: self.warnings }
    }
}
