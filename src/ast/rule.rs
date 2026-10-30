use super::{Ident, Pragma, Span, Spanned, Type};

// ── Rule ──────────────────────────────────────────────────────────────────────

/// A single CDDL rule definition.
///
/// ```cddl
/// my-type = { field: uint }          ; AssignType
/// $socket /= tstr                    ; AssignExtend (type socket plug)
/// $$group //= ( extra: bool )        ; AssignExtend (group socket plug)
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    /// Pragmas that immediately precede this rule (attached pragmas).
    pub pragmas: Vec<Pragma>,

    /// Its documentation: the plain comment lines directly above it (no blank line in
    /// between), and a comment at the end of its last line (dgtlrift/cddlc#8).
    pub doc: Option<String>,

    /// Where it was written (dgtlrift/cddlc#9).
    pub source: Option<super::SourceLine>,

    pub name: Spanned<Ident>,

    /// Generic parameters, if any: `foo<T, U> = ...`
    pub generic_params: Vec<Spanned<Ident>>,

    pub kind: RuleKind,
}

#[derive(Debug, Clone, PartialEq)]
pub enum RuleKind {
    /// `name = type`  or  `name /= type`  (type rule, with optional extend)
    Type {
        ty:     Spanned<Type>,
        extend: bool,   // true if `/=`
    },

    /// `name = ( group )`  or  `name //= ( group )`  (group rule)
    Group {
        group:  Spanned<GroupEntry>,
        extend: bool,   // true if `//=`
    },
}

// ── Group entries ─────────────────────────────────────────────────────────────

/// A group is an ordered sequence of entries, possibly with choices between
/// them.  It appears inside `{...}` (map), `[...]` (array), or `(...)` (group
/// rule).
///
/// RFC 8610 §3.5
#[derive(Debug, Clone, PartialEq)]
pub struct GroupEntry {
    /// Alternatives separated by `//` (group choice).
    pub choices: Vec<GroupChoice>,
}

/// One alternative within a group choice.
#[derive(Debug, Clone, PartialEq)]
pub struct GroupChoice {
    pub entries: Vec<Spanned<MemberEntry>>,
}

/// A single member within a group choice.
#[derive(Debug, Clone, PartialEq)]
pub struct MemberEntry {
    pub occurrence: Option<Spanned<Occurrence>>,
    pub kind:       MemberKind,

    /// Pragmas attached to this specific member.
    pub pragmas: Vec<Pragma>,

    /// Its documentation: the comment lines directly above it, an `@doc` pragma among
    /// them, and a comment on its own line after it (dgtlrift/cddlc#8).
    pub doc: Option<String>,

    /// Where it was written (dgtlrift/cddlc#9).
    pub source: Option<super::SourceLine>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberKind {
    /// A key/value pair: `key: type` or `key => type`
    ///
    /// `key` may be a bare identifier (becomes tstr key), a type, or a value.
    KeyValue {
        key:   Spanned<MemberKey>,
        value: Spanned<Type>,
        /// `true` for `=>` (cut), `false` for `:`
        cut:   bool,
    },

    /// A bare type reference embedded in the group (unwrapped group member).
    Type(Spanned<Type>),
}

#[derive(Debug, Clone, PartialEq)]
pub enum MemberKey {
    /// Bare label as map key: `field-name: type` → key is tstr "field-name"
    Bareword(Ident),

    /// Type used as key: `tstr => uint`
    Type(Type),

    /// Literal value used as key: `1 => tstr`
    Value(Value),
}

// ── Occurrence indicators ─────────────────────────────────────────────────────

/// Occurrence indicator on a group member.
/// RFC 8610 §3.2
#[derive(Debug, Clone, PartialEq)]
pub enum Occurrence {
    /// `?`  — optional (0 or 1)
    Optional,

    /// `*`  — zero or more
    ZeroOrMore,

    /// `+`  — one or more
    OneOrMore,

    /// `n * m`  — exactly n to m occurrences
    ///
    /// Either bound may be absent:
    ///   `*`  = `0*`  = unbounded
    ///   `1*` = one or more
    ///   `*3` = zero to three
    Bounded { min: Option<u64>, max: Option<u64> },
}

impl Occurrence {
    pub fn min(&self) -> u64 {
        match self {
            Occurrence::Optional        => 0,
            Occurrence::ZeroOrMore      => 0,
            Occurrence::OneOrMore       => 1,
            Occurrence::Bounded { min, .. } => min.unwrap_or(0),
        }
    }

    pub fn max(&self) -> Option<u64> {
        match self {
            Occurrence::Optional        => Some(1),
            Occurrence::ZeroOrMore      => None,
            Occurrence::OneOrMore       => None,
            Occurrence::Bounded { max, .. } => *max,
        }
    }
}

// ── Literal values ────────────────────────────────────────────────────────────

/// A CDDL literal value.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// Unsigned integer literal: `42`
    Uint(u64),

    /// Signed (negative) integer literal: `-1`
    Int(i64),

    /// Floating-point literal: `3.14`
    Float(f64),

    /// Text string literal: `"hello"`
    Text(String),

    /// Byte string literal: `h'deadbeef'` or `b64'...'` or `'...'`
    Bytes(Vec<u8>),
}

impl Value {
    pub fn span_placeholder() -> Span {
        Span::new(0, 0)
    }
}
