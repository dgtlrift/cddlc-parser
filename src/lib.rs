//! # cddlc-parser
//!
//! Parses CDDL source files (RFC 8610) into an AST, extracting structured
//! pragma comments for use by the `cddlc-ir` lowering stage.
//!
//! ## Usage
//!
//! ```rust,no_run
//! use cddlc_parser::parse_cddl;
//! use std::path::PathBuf;
//!
//! let src = r#"
//!     ; @capacity 8
//!     readings = [* float32]
//! "#;
//!
//! let result = parse_cddl(src, PathBuf::from("example.cddl")).unwrap();
//! let module = result.value;
//! println!("Rules: {}", module.rules.len());
//!
//! for warning in &result.warnings {
//!     eprintln!("{warning}");
//! }
//! ```

pub mod ast;
pub mod error;
pub mod pragma;
pub mod parser;

// Flat re-exports for convenience
pub use ast::{
    CddlModule, CtlOp, GroupChoice, GroupEntry, Ident, MemberEntry, MemberKind,
    MemberKey, Occurrence, Pragma, PragmaValue, Rule, RuleKind, Span, Spanned,
    Type, Type1, Type2, TypeOp, Value, SourceLine,
};
pub use error::{ParseError, ParseResult, ParseWarning, WarningKind};
pub use parser::{parse_cddl, parse_cddl_debug, parse_type_str, format_parse_diagnostic};

/// Prelude for consumers of this crate.
pub mod prelude {
    pub use crate::{
        parse_cddl, parse_type_str,
        CddlModule, Rule, RuleKind, Type, Type2, Pragma, PragmaValue,
        ParseError, ParseResult, ParseWarning,
    };
}
