/// AST for CDDL (RFC 8610).
///
/// This module defines the *syntactic* representation produced by the parser.
/// It is deliberately close to the grammar: no name resolution, no constraint
/// propagation, no generic instantiation.  All of that happens in `cddlc-ir`.
///
/// Span information is preserved on every node so that the IR and backends can
/// emit accurate diagnostics.
pub mod rule;
pub mod ty;

pub use rule::*;
pub use ty::*;

use smol_str::SmolStr;

// ── Source location ───────────────────────────────────────────────────────────

/// Byte-offset span within a single source file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Span {
    pub start: usize,
    pub end:   usize,
}

impl Span {
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    pub fn merge(self, other: Span) -> Self {
        Self {
            start: self.start.min(other.start),
            end:   self.end.max(other.end),
        }
    }
}

/// Where an item was written: its file, its 1-based line, and that line's text,
/// trimmed (dgtlrift/cddlc#9).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceLine {
    pub path: std::path::PathBuf,
    pub line: usize,
    pub text: String,
}

/// A value together with its source span.
#[derive(Debug, Clone, PartialEq)]
pub struct Spanned<T> {
    pub node: T,
    pub span: Span,
}

impl<T> Spanned<T> {
    pub fn new(node: T, span: Span) -> Self {
        Self { node, span }
    }

    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Spanned<U> {
        Spanned { node: f(self.node), span: self.span }
    }
}

// ── Identifiers ───────────────────────────────────────────────────────────────

/// A CDDL identifier (rule name, group name, etc.).
/// RFC 8610 §3.1: id = ALPHA *(*("-" / ".") (ALPHA / DIGIT))
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Ident(pub SmolStr);

impl Ident {
    pub fn new(s: impl Into<SmolStr>) -> Self {
        Self(s.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Ident {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

// ── Top-level module ──────────────────────────────────────────────────────────

/// A parsed CDDL source file.
#[derive(Debug, Clone, PartialEq)]
pub struct CddlModule {
    /// The file path this module was loaded from (for diagnostics and import
    /// resolution).
    pub source_path: std::path::PathBuf,

    /// Pragmas that appear *before* any rule (file-level pragmas).
    /// These include `@import` directives.
    pub file_pragmas: Vec<Pragma>,

    /// The ordered list of rule definitions in this file.
    pub rules: Vec<Spanned<Rule>>,
}
