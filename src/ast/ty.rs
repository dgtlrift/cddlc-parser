use super::{GroupEntry, Ident, Spanned, Value};

// ── Type expressions ──────────────────────────────────────────────────────────

/// A CDDL type expression.
///
/// RFC 8610 §3.5.4 — `type = type1 / type1`
#[derive(Debug, Clone, PartialEq)]
pub struct Type {
    /// Alternatives separated by `/` (type choice).
    pub choices: Vec<Spanned<Type1>>,
}

impl Type {
    /// Convenience: a type with a single, non-choice alternative.
    pub fn single(t1: Spanned<Type1>) -> Self {
        Self { choices: vec![t1] }
    }
}

/// A single alternative within a type choice.
///
/// RFC 8610 §3.5.4 — `type1 = type2 [S rangeop S type2 / S ctlop S type2]`
///
/// Multiple control operators may be chained: `int .ge -40 .le 85`
/// In that case `ops` holds each operator in order.  A range operator
/// must be the only operator (ranges can't be chained).
#[derive(Debug, Clone, PartialEq)]
pub struct Type1 {
    pub base: Spanned<Type2>,
    /// Chained operators — usually 0 or 1, but may be more for control ops.
    pub ops:  Vec<TypeOp>,
}

impl Type1 {
    /// Convenience accessor for the single-operator case.
    pub fn op(&self) -> Option<&TypeOp> {
        self.ops.first()
    }
}

/// An operator applied to a base type.
#[derive(Debug, Clone, PartialEq)]
pub enum TypeOp {
    /// Range operator: `lo..hi`  (inclusive) or `lo...hi` (exclusive upper)
    Range {
        upper:     Spanned<Type2>,
        inclusive: bool,
    },

    /// Control operator: `.size`, `.regexp`, `.cbor`, etc.
    Control {
        op:      Spanned<CtlOp>,
        operand: Spanned<Type2>,
    },
}

/// Control operators (RFC 8610 §3.8 + common extensions).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum CtlOp {
    /// `.size`  — constrain byte/text/array length
    Size,

    /// `.bits`  — bit constraints on uint
    Bits,

    /// `.regexp` — constrain tstr with PCRE/ECMA regex
    Regexp,

    /// `.cbor`  — inner bstr must decode as this type
    Cbor,

    /// `.cborseq` — inner bstr must decode as CBOR sequence of this type
    CborSeq,

    /// `.within` — type must be within (subset of) this type
    Within,

    /// `.and`   — type must satisfy both this type AND the base
    And,

    /// `.lt`, `.le`, `.gt`, `.ge`, `.eq`, `.ne`
    Lt, Le, Gt, Ge, Eq, Ne,

    /// `.default` — default value if key is absent
    Default,

    /// Unknown / future extension
    Other(String),
}

impl CtlOp {
    pub fn from_str(s: &str) -> Self {
        match s {
            "size"    => CtlOp::Size,
            "bits"    => CtlOp::Bits,
            "regexp"  => CtlOp::Regexp,
            "cbor"    => CtlOp::Cbor,
            "cborseq" => CtlOp::CborSeq,
            "within"  => CtlOp::Within,
            "and"     => CtlOp::And,
            "lt"      => CtlOp::Lt,
            "le"      => CtlOp::Le,
            "gt"      => CtlOp::Gt,
            "ge"      => CtlOp::Ge,
            "eq"      => CtlOp::Eq,
            "ne"      => CtlOp::Ne,
            "default" => CtlOp::Default,
            other     => CtlOp::Other(other.to_owned()),
        }
    }
}

/// A base type — the "atom" of the type grammar.
///
/// RFC 8610 §3.5.4 — `type2`
#[derive(Debug, Clone, PartialEq)]
pub enum Type2 {
    /// A literal value used as a type: `1`, `"hello"`, `h'ff'`
    Value(Value),

    /// Reference to another rule, possibly with generic arguments:
    /// `my-type`, `labeled<uint>`
    Typename {
        name:    Spanned<Ident>,
        args:    Vec<Spanned<Type>>,   // empty if not generic
    },

    /// A map type: `{ group }`
    Map(Spanned<GroupEntry>),

    /// An array type: `[ group ]`
    Array(Spanned<GroupEntry>),

    /// An unwrapped group reference: `~my-group`
    Unwrap(Spanned<Ident>),

    /// An inline group (parenthesised): `( group )`
    Group(Spanned<GroupEntry>),

    /// A tagged type: `#6.NN(type)`
    Tagged {
        tag:    u64,
        inner:  Box<Spanned<Type>>,
    },

    /// Major-type shorthand: `#N` (e.g. `#1` = uint major, `#2` = bstr)
    MajorType(u8),

    /// The `#` catch-all (any CBOR data item)
    Any,
}

// ── Pragma ────────────────────────────────────────────────────────────────────

/// A structured comment pragma.
///
/// Pragmas are comment lines of the form:
/// ```text
/// ; @keyword value...
/// ```
///
/// They may appear:
/// - Before the first rule (file-level, used for `@import`)
/// - Immediately before a rule definition (rule-level)
/// - Immediately before a group member (member-level)
#[derive(Debug, Clone, PartialEq)]
pub struct Pragma {
    pub span:    super::Span,
    pub keyword: String,
    pub value:   PragmaValue,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PragmaValue {
    /// `@import "path/to/file.cddl"`
    Import(String),

    /// `@capacity N`
    Capacity(u64),

    /// `@str-capacity N`
    StrCapacity(u64),

    /// `@regex-hook identifier`
    RegexHook(String),

    /// `@doc "free text"`
    Doc(String),

    /// `@deprecated`
    Deprecated,

    /// `@skip-validation`
    SkipValidation,

    /// `@opaque`
    Opaque,

    /// `@default expr` — raw text, parsed by IR
    Default(String),

    /// Unknown pragma — keyword + raw trailing text.
    /// Preserved so backends can define their own pragmas.
    Unknown(String),
}
