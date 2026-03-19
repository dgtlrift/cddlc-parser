/// Pragma extraction from CDDL comment lines.
///
/// CDDL comments are `;` to end-of-line.  A pragma comment has the form:
///
/// ```text
/// ; @keyword [value]
/// ```
///
/// This module is responsible only for *recognising and parsing* pragma lines.
/// Attaching them to rules is done in the main parser.

use crate::ast::{Pragma, PragmaValue, Span};
use crate::error::{ParseWarning, WarningKind};
use std::path::PathBuf;

/// Attempt to parse a raw comment body (the text after `;`) as a pragma.
///
/// Returns `None` if the comment is not a pragma (doesn't start with `@`).
/// Returns `Some(Ok(pragma))` on success.
/// Returns `Some(Err(warning))` if the pragma keyword is known but the value
/// is malformed — we emit a warning and return `None` to the caller so parsing
/// continues.
pub fn parse_pragma(
    comment_body: &str,
    span:         Span,
    path:         &PathBuf,
    line:         usize,
) -> Option<Result<Pragma, ParseWarning>> {
    let body = comment_body.trim();
    if !body.starts_with('@') {
        return None;
    }

    // Split into keyword and optional value portion
    let rest = &body[1..];
    let (keyword, value_str) = match rest.find(|c: char| c.is_whitespace()) {
        Some(pos) => (&rest[..pos], rest[pos..].trim()),
        None      => (rest, ""),
    };

    let pragma_value = match keyword {
        "import" => {
            match parse_quoted_string(value_str) {
                Some(s) => PragmaValue::Import(s),
                None    => return Some(Err(bad_pragma(path, line, keyword,
                    "expected a quoted string, e.g. @import \"path/to/file.cddl\""))),
            }
        }

        "capacity" => {
            match value_str.parse::<u64>() {
                Ok(n)  => PragmaValue::Capacity(n),
                Err(_) => return Some(Err(bad_pragma(path, line, keyword,
                    "expected a non-negative integer, e.g. @capacity 32"))),
            }
        }

        "str-capacity" => {
            match value_str.parse::<u64>() {
                Ok(n)  => PragmaValue::StrCapacity(n),
                Err(_) => return Some(Err(bad_pragma(path, line, keyword,
                    "expected a non-negative integer, e.g. @str-capacity 128"))),
            }
        }

        "regex-hook" => {
            if value_str.is_empty() {
                return Some(Err(bad_pragma(path, line, keyword,
                    "expected a C/Rust identifier, e.g. @regex-hook validate_serial")));
            }
            PragmaValue::RegexHook(value_str.to_owned())
        }

        "doc" => {
            match parse_quoted_string(value_str) {
                Some(s) => PragmaValue::Doc(s),
                // Also accept unquoted trailing text as doc comment
                None    => PragmaValue::Doc(value_str.to_owned()),
            }
        }

        "deprecated"       => PragmaValue::Deprecated,
        "skip-validation"  => PragmaValue::SkipValidation,
        "opaque"           => PragmaValue::Opaque,

        "default" => {
            if value_str.is_empty() {
                return Some(Err(bad_pragma(path, line, keyword,
                    "expected a value expression, e.g. @default 0")));
            }
            PragmaValue::Default(value_str.to_owned())
        }

        other => {
            // Unknown pragma — preserve it and emit a warning so the caller
            // can decide whether to surface it.
            let warn = ParseWarning {
                path:    path.clone(),
                line,
                col:     0,
                message: format!("unknown pragma '@{other}' — it will be preserved but ignored by all built-in backends"),
                kind:    WarningKind::UnknownPragma,
            };
            // Still produce the pragma so user-defined backends can see it.
            let pragma = Pragma {
                span,
                keyword: other.to_owned(),
                value:   PragmaValue::Unknown(value_str.to_owned()),
            };
            // We return Ok here (not an error) but the warning is communicated
            // via the ParseResult warning list at the call site.
            // We encode this as a special path: return the pragma AND a warning
            // by returning Err with the warning, letting the caller add both.
            // Actually, simplest: return the pragma as Ok and the caller emits
            // the warning separately.
            let _ = warn; // will be handled by caller checking for Unknown variant
            return Some(Ok(pragma));
        }
    };

    Some(Ok(Pragma {
        span,
        keyword: keyword.to_owned(),
        value:   pragma_value,
    }))
}

fn parse_quoted_string(s: &str) -> Option<String> {
    let s = s.trim();
    if s.starts_with('"') && s.ends_with('"') && s.len() >= 2 {
        Some(s[1..s.len()-1].to_owned())
    } else {
        None
    }
}

fn bad_pragma(
    path:    &PathBuf,
    line:    usize,
    keyword: &str,
    reason:  &str,
) -> ParseWarning {
    ParseWarning {
        path:    path.clone(),
        line,
        col:     0,
        message: format!("pragma '@{keyword}': {reason}"),
        kind:    WarningKind::UnknownPragma,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span() -> Span { Span::new(0, 0) }
    fn path() -> PathBuf { PathBuf::from("test.cddl") }

    #[test]
    fn test_not_a_pragma() {
        assert!(parse_pragma("just a comment", span(), &path(), 1).is_none());
        assert!(parse_pragma("  regular comment  ", span(), &path(), 1).is_none());
    }

    #[test]
    fn test_import() {
        let result = parse_pragma(r#"@import "common/types.cddl""#, span(), &path(), 1);
        let pragma = result.unwrap().unwrap();
        assert_eq!(pragma.keyword, "import");
        assert_eq!(pragma.value, PragmaValue::Import("common/types.cddl".into()));
    }

    #[test]
    fn test_capacity() {
        let result = parse_pragma("@capacity 64", span(), &path(), 1);
        let pragma = result.unwrap().unwrap();
        assert_eq!(pragma.value, PragmaValue::Capacity(64));
    }

    #[test]
    fn test_str_capacity() {
        let result = parse_pragma("@str-capacity 256", span(), &path(), 1);
        let pragma = result.unwrap().unwrap();
        assert_eq!(pragma.value, PragmaValue::StrCapacity(256));
    }

    #[test]
    fn test_deprecated() {
        let result = parse_pragma("@deprecated", span(), &path(), 1);
        let pragma = result.unwrap().unwrap();
        assert_eq!(pragma.value, PragmaValue::Deprecated);
    }

    #[test]
    fn test_unknown_pragma_preserved() {
        let result = parse_pragma("@my-custom-pragma foobar", span(), &path(), 1);
        let pragma = result.unwrap().unwrap();
        assert_eq!(pragma.keyword, "my-custom-pragma");
        assert!(matches!(pragma.value, PragmaValue::Unknown(_)));
    }
}
