/// CDDL parser built on `winnow`.
///
/// Entry points:
/// - [`parse_cddl`]  — parse a complete source string into a [`CddlModule`]
/// - [`parse_type`]  — parse a single type expression (for testing/tooling)
///
/// The parser is a hand-written recursive-descent parser using winnow
/// combinators.  It tracks byte offsets throughout so that every AST node
/// carries a [`Span`].
///
/// Pragma comments are parsed inline: whenever a `;` comment is encountered
/// the pragma module attempts to interpret it, and any resulting pragmas are
/// accumulated and attached to the next rule or member entry.

use winnow::{
    ascii::{digit1, hex_digit1, multispace0},
    combinator::{alt, cut_err, peek, separated},
    error::{ContextError, ErrMode, StrContext, StrContextValue},
    token::{any, literal, none_of, one_of, take_till},
    ModalResult, Parser,
};

use std::path::PathBuf;

use crate::ast::{
    CddlModule, CtlOp, GroupChoice, GroupEntry, Ident, MemberEntry, MemberKey,
    MemberKind, Occurrence, Pragma, PragmaValue, Rule, RuleKind, SourceLine, Span, Spanned,
    Type, Type1, Type2, TypeOp, Value,
};
use crate::error::{ParseError, ParseResult, ParseWarning, WarningKind};
use crate::pragma::parse_pragma;

// ── Public API ────────────────────────────────────────────────────────────────

/// Parse a complete CDDL source file.
pub fn parse_cddl(
    src:  &str,
    path: PathBuf,
) -> Result<ParseResult<CddlModule>, ParseError> {
    let mut state = ParseState::new(src, path.clone());
    state.parse_module(src)
}

/// Parse with rich diagnostic output on failure.
///
/// On success returns the module and prints nothing.
/// On failure prints a structured diagnostic to stderr and returns the error.
///
/// Diagnostic includes:
///   - File, line, column
///   - Source excerpt with caret pointer
///   - Remaining unparsed input excerpt
///   - Accumulated pending pragmas at the point of failure
///   - Token context from winnow
pub fn parse_cddl_debug(
    src:  &str,
    path: PathBuf,
) -> Result<ParseResult<CddlModule>, ParseError> {
    let mut state = ParseState::new(src, path.clone());
    match state.parse_module(src) {
        Ok(result) => {
            if !result.warnings.is_empty() {
                eprintln!("[cddlc-parse] {} warning(s):", result.warnings.len());
                for w in &result.warnings {
                    eprintln!("  {w}");
                }
            }
            Ok(result)
        }
        Err(e) => {
            eprintln!("\n{}", format_parse_diagnostic(src, &path, &e));
            Err(e)
        }
    }
}

/// Format a rich diagnostic message for a parse error.
pub fn format_parse_diagnostic(src: &str, path: &PathBuf, error: &ParseError) -> String {
    let mut out = String::new();

    out.push_str("━━━ cddlc parse error ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");

    match error {
        ParseError::Syntax { line, col, message, span, .. } => {
            out.push_str(&format!("  File : {}\n", path.display()));
            out.push_str(&format!("  Line : {line}, Col: {col}\n"));
            out.push_str(&format!("  Span : {}..{}\n", span.start, span.end));
            out.push('\n');

            // Source excerpt: 3 lines before + the failing line
            let lines: Vec<&str> = src.lines().collect();
            let line_idx = line.saturating_sub(1); // 0-based
            let start_line = line_idx.saturating_sub(3);
            out.push_str("  Source context:\n");
            for (i, l) in lines[start_line..=line_idx.min(lines.len().saturating_sub(1))].iter().enumerate() {
                let ln = start_line + i + 1;
                if ln == *line {
                    out.push_str(&format!("  {:>4} │ {}\n", ln, l));
                    // Caret pointer
                    let caret_pos = col.saturating_sub(1).min(l.len());
                    out.push_str(&format!("       │ {}^\n", " ".repeat(caret_pos)));
                } else {
                    out.push_str(&format!("  {:>4} │ {}\n", ln, l));
                }
            }
            out.push('\n');

            // Remaining input at the failure point
            let offset = span.start;
            if offset < src.len() {
                let remaining = &src[offset..];
                let excerpt: String = remaining.chars().take(120).collect();
                let excerpt_display = excerpt.replace('\n', "↵").replace('\t', "→");
                out.push_str(&format!("  Remaining input at offset {offset}:\n"));
                out.push_str(&format!("    ┌─ {:?}", excerpt_display));
                if remaining.len() > 120 {
                    out.push_str(" …");
                }
                out.push_str("\n\n");
            }

            // Parse the winnow error message for expected tokens
            out.push_str("  Parser context:\n");
            for line in message.lines() {
                out.push_str(&format!("    {line}\n"));
            }

            // Common hints based on error content
            out.push('\n');
            out.push_str("  Hints:\n");
            if message.contains("parse_ident") || message.contains("ident") {
                out.push_str("  • Expected an identifier — check for reserved words or invalid characters\n");
            }
            if message.contains("assignop") || message.contains("\"=\"") {
                out.push_str("  • Expected assignment operator = / /= / //=\n");
            }
            if message.contains("comment") || src[..offset.min(src.len())].contains(';') {
                out.push_str("  • Comments use `;` to end-of-line; pragmas use `; @keyword value`\n");
            }
            if message.contains("map") || message.contains("group") {
                out.push_str("  • Map/group entries use `key: type` or `key => type`\n");
            }
            if message.contains("type") {
                out.push_str("  • Valid primitive types: uint, int, float, float32, float64, tstr, bstr, bool, null, any\n");
            }
        }
        ParseError::ImportCycle { cycle } => {
            out.push_str(&format!("  Import cycle: {}\n", cycle.join(" → ")));
            out.push_str("  Hint: check your @import chain for circular dependencies\n");
        }
        other => {
            out.push_str(&format!("  {other}\n"));
        }
    }

    out.push_str("━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━\n");
    out
}

/// Parse a single type expression (useful for testing).
pub fn parse_type_str(src: &str) -> Result<Type, String> {
    let mut input = src.trim();
    parse_type(&mut input).map_err(|e| format!("{e:?}"))
}

// ── Parser state ──────────────────────────────────────────────────────────────

struct ParseState {
    path:     PathBuf,
    warnings: Vec<ParseWarning>,
    /// Pending pragmas accumulated from comment lines, to be attached to the
    /// next rule or group member.
    pending_pragmas: Vec<Pragma>,
    /// Plain comment lines since the last blank line, to document the next rule
    /// (dgtlrift/cddlc#8).
    pending_doc: Vec<String>,
    /// A comment on the same line as the end of the rule just parsed.
    trailing_doc: Option<String>,
    /// A rule has been parsed: a comment before any line break can trail it.
    seen_rule: bool,
}

impl ParseState {
    fn new(_src: &str, path: PathBuf) -> Self {
        Self {
            path,
            warnings: Vec::new(),
            pending_pragmas: Vec::new(),
            pending_doc: Vec::new(),
            trailing_doc: None,
            seen_rule: false,
        }
    }

    /// Consume and return all accumulated pending pragmas.
    fn take_pragmas(&mut self) -> Vec<Pragma> {
        std::mem::take(&mut self.pending_pragmas)
    }

    fn add_warning(&mut self, w: ParseWarning) {
        self.warnings.push(w);
    }

    // ── Top-level parse ───────────────────────────────────────────────────────

    fn parse_module(
        &mut self,
        src: &str,
    ) -> Result<ParseResult<CddlModule>, ParseError> {
        let trace = std::env::var("CDDLC_TRACE").is_ok();
        let mut input = src;
        let mut rules: Vec<Spanned<Rule>> = Vec::new();
        let mut file_pragmas: Vec<Pragma> = Vec::new();
        let mut rule_index = 0usize;
        SRC_LEN.with(|l| l.set(src.len()));

        loop {
            self.skip_ws_and_comments(&mut input, src);
            // A comment on the last line of the rule just parsed documents it.
            if let Some(t) = self.trailing_doc.take() {
                if let Some(last) = rules.last_mut() {
                    push_doc(&mut last.node.doc, &t);
                }
            }
            if input.is_empty() {
                break;
            }

            // Any @import pragmas accumulated since the last rule are
            // file-level; everything else is attached to the next rule.
            let pending = self.take_pragmas();
            for p in pending {
                if p.keyword == "import" {
                    if trace { eprintln!("[cddlc-trace] file pragma: @import {:?}", p.value); }
                    file_pragmas.push(p);
                } else {
                    if trace { eprintln!("[cddlc-trace] rule pragma pending: @{}", p.keyword); }
                    self.pending_pragmas.push(p);
                }
            }

            let start = byte_offset(src, input);
            let line  = line_of(src, start);
            let col   = col_of(src, start);
            if trace {
                let preview: String = input.chars().take(40).collect();
                let preview = preview.replace('\n', "↵");
                eprintln!("[cddlc-trace] rule[{rule_index}] at {line}:{col} — {:?}", preview);
            }

            let rule = self.parse_rule(&mut input, src)
                .map_err(|e| ParseError::Syntax {
                    path:    self.path.clone(),
                    line,
                    col,
                    message: format!("{e:?}"),
                    span:    Span::new(start, start),
                })?;

            if trace {
                eprintln!("[cddlc-trace] rule[{rule_index}] parsed: {:?}", rule.name.node.as_str());
            }
            rule_index += 1;

            let end = byte_offset(src, input);
            self.seen_rule = true;
            let mut rule = rule;
            rule.source = Some(source_line(src, &self.path, rule.name.span.start));
            match &mut rule.kind {
                RuleKind::Type { ty, .. } => sources_in_type(&mut ty.node, src, &self.path),
                RuleKind::Group { group, .. } => sources_in_group(&mut group.node, src, &self.path),
            }
            rules.push(Spanned::new(rule, Span::new(start, end)));
        }

        // Any trailing @import pragmas after the last rule
        for p in self.take_pragmas() {
            if p.keyword == "import" {
                file_pragmas.push(p);
            }
        }

        if trace {
            eprintln!("[cddlc-trace] parse complete: {} rules, {} file_pragmas, {} warnings",
                rules.len(), file_pragmas.len(), self.warnings.len());
        }

        let module = CddlModule {
            source_path: self.path.clone(),
            file_pragmas,
            rules,
        };

        Ok(ParseResult::with_warnings(module, std::mem::take(&mut self.warnings)))
    }

    // ── Whitespace + comment handling ─────────────────────────────────────────

    /// Skip whitespace and comment lines.  Pragma comments are parsed and
    /// pushed into `self.pending_pragmas`.
    fn skip_ws_and_comments(&mut self, input: &mut &str, src: &str) {
        let trace = std::env::var("CDDLC_TRACE").is_ok();
        // Line breaks crossed since the last token (the end of the previous rule); at the
        // start of the file, a comment trails nothing.
        let mut breaks = usize::from(!self.seen_rule);
        loop {
            // Skip whitespace; a blank line ends a comment block, which then documents
            // nothing (dgtlrift/cddlc#8).
            let ws = multispace0::<_, ()>.parse_next(input).unwrap_or("");
            let nl = ws.matches('\n').count();
            breaks += nl;
            if nl >= 2 {
                self.pending_doc.clear();
            }

            if input.starts_with(';') {
                let line_no = line_of(src, byte_offset(src, input));
                let start   = byte_offset(src, input);

                // Consume `;` and rest of line
                let _ = one_of::<_, _, ()>(';').parse_next(input);
                let comment_body = take_till::<_, _, ()>(0.., |c| c == '\n')
                    .parse_next(input)
                    .unwrap_or("");
                let end = byte_offset(src, input);
                let span = Span::new(start, end);

                if trace {
                    eprintln!("[cddlc-trace] comment line {line_no}: ;{comment_body}");
                }

                // Try to parse as pragma
                match parse_pragma(comment_body, span, &self.path, line_no) {
                    None => {
                        if trace {
                            eprintln!("[cddlc-trace]   → ordinary comment (no pragma)");
                        }
                        let text = comment_text(comment_body);
                        if breaks == 0 {
                            // On the line the previous rule ended on.
                            match &mut self.trailing_doc {
                                Some(t) => { t.push('\n'); t.push_str(&text); }
                                None => self.trailing_doc = Some(text),
                            }
                        } else {
                            self.pending_doc.push(text);
                        }
                    }
                    Some(Ok(pragma)) => {
                        if trace {
                            eprintln!("[cddlc-trace]   → pragma @{} = {:?}", pragma.keyword, pragma.value);
                        }
                        // Emit warning for Unknown pragmas
                        if let PragmaValue::Unknown(_) = &pragma.value {
                            self.add_warning(ParseWarning {
                                path:    self.path.clone(),
                                line:    line_no,
                                col:     0,
                                message: format!(
                                    "unknown pragma '@{}' — preserved for custom backends",
                                    pragma.keyword
                                ),
                                kind: WarningKind::UnknownPragma,
                            });
                        }
                        self.pending_pragmas.push(pragma);
                    }
                    Some(Err(warn)) => {
                        if trace {
                            eprintln!("[cddlc-trace]   → pragma parse warning: {}", warn.message);
                        }
                        self.add_warning(warn);
                    }
                }
            } else {
                break;
            }
        }
    }

    // ── Rule parsing ──────────────────────────────────────────────────────────

    fn parse_rule(
        &mut self,
        input: &mut &str,
        src:   &str,
    ) -> ModalResult<Rule> {
        let pragmas = self.take_pragmas();
        let doc = join_doc(&std::mem::take(&mut self.pending_doc));

        // name [generic-params] assignop type-or-group
        let name_start = byte_offset(src, input);
        let name_str   = parse_ident(input)?;
        let name_end   = byte_offset(src, input);
        let name = Spanned::new(
            Ident::new(name_str),
            Span::new(name_start, name_end),
        );

        // Optional generic parameters: `<T, U>`
        let generic_params = parse_generic_params(input, src)?;

        ws0(input);

        // Assignment operator: `=`, `/=`, `//=`
        let (extend_type, extend_group) = parse_assignop(input)?;

        ws0(input);

        // Determine if RHS is a group rule (starts with `(`) or type rule
        let kind = if !extend_group && peek_group_rule(input) {
            let gs = byte_offset(src, input);
            let group = parse_group_entry(input, src)?;
            let ge    = byte_offset(src, input);
            RuleKind::Group {
                group:  Spanned::new(group, Span::new(gs, ge)),
                extend: false,
            }
        } else {
            let ts = byte_offset(src, input);
            let ty = parse_type(input)?;
            let te = byte_offset(src, input);
            RuleKind::Type {
                ty:     Spanned::new(ty, Span::new(ts, te)),
                extend: extend_type || extend_group,
            }
        };

        Ok(Rule { pragmas, doc, source: None, name, generic_params, kind })
    }
}

// ── Generic parameter list ────────────────────────────────────────────────────

fn parse_generic_params(
    input: &mut &str,
    src:   &str,
) -> ModalResult<Vec<Spanned<Ident>>> {
    if !input.starts_with('<') {
        return Ok(vec![]);
    }
    let _ = literal::<_, _, ContextError>("<").parse_next(input)?;
    ws0(input);
    let params = separated(
        1..,
        |i: &mut &str| {
            ws0(i);
            let s = byte_offset(src, i);
            let name = parse_ident(i)?;
            let e = byte_offset(src, i);
            ws0(i);
            Ok(Spanned::new(Ident::new(name), Span::new(s, e)))
        },
        (ws0_fn, literal(","), ws0_fn),
    )
    .parse_next(input)?;
    ws0(input);
    cut_err(literal(">"))
        .context(StrContext::Expected(StrContextValue::Description("closing '>' for generic params")))
        .parse_next(input)?;
    Ok(params)
}

// ── Assignment operator ───────────────────────────────────────────────────────

/// Returns `(type_extend, group_extend)`.
fn parse_assignop(input: &mut &str) -> ModalResult<(bool, bool)> {
    alt((
        literal("//=").map(|_| (false, true)),
        literal("/=").map(|_| (true, false)),
        literal("=").map(|_| (false, false)),
    ))
    .context(StrContext::Expected(StrContextValue::Description("'=', '/=', or '//='")))
    .parse_next(input)
}

fn peek_group_rule(input: &str) -> bool {
    input.starts_with('(')
}

// ── Type parsing ──────────────────────────────────────────────────────────────

/// Parse a `type` expression: one or more `type1` alternatives separated by `/`.
pub(crate) fn parse_type(input: &mut &str) -> ModalResult<Type> {
    let choices = separated(
        1..,
        |i: &mut &str| {
            wsc0(i);
            // What follows is left for the separator, which skips comments only when a
            // `/` comes next: a comment after the type documents what holds it
            // (dgtlrift/cddlc#8).
            parse_type1(i)
        },
        (wsc0_fn, literal("/"), wsc0_fn, peek(none_of('/'))),
    )
    .parse_next(input)?;

    Ok(Type { choices })
}

/// Parse a `type1`: a `type2` optionally followed by one or more operators.
/// Range operators can appear once; control operators may be chained.
fn parse_type1(input: &mut &str) -> ModalResult<Spanned<Type1>> {
    ws0(input);
    let start = 0usize;

    let base = parse_type2_spanned(input)?;
    let mut ops: Vec<TypeOp> = Vec::new();

    loop {
        // Whitespace is skipped only if an operator follows, so the line breaks after the
        // type are still there for the comment handling (dgtlrift/cddlc#8).
        let before = *input;
        ws0(input);

        if input.starts_with("...") {
            let _ = literal::<_, _, ContextError>("...").parse_next(input)?;
            ws0(input);
            let upper = parse_type2_spanned(input)?;
            ops.push(TypeOp::Range { upper, inclusive: false });
            break; // range must be last / only operator
        } else if input.starts_with("..") {
            let _ = literal::<_, _, ContextError>("..").parse_next(input)?;
            ws0(input);
            let upper = parse_type2_spanned(input)?;
            ops.push(TypeOp::Range { upper, inclusive: true });
            break; // range must be last / only operator
        } else if input.starts_with('.') {
            // Control operator: `.keyword operand`
            // Peek ahead to make sure it's really `.ident` not `...` or `..`
            let _ = one_of::<_, _, ContextError>('.').parse_next(input)?;
            let kw = parse_ident(input)?;
            let op = CtlOp::from_str(kw);
            ws0(input);
            let operand = parse_type2_spanned(input)?;
            ops.push(TypeOp::Control {
                op:      Spanned::new(op, Span::new(0, 0)),
                operand,
            });
            // Loop — another control op may follow (e.g. `.ge -40 .le 85`)
        } else {
            *input = before;
            break;
        }
    }

    let span = Span::new(start, 0);
    Ok(Spanned::new(Type1 { base, ops }, span))
}

fn parse_type2_spanned(input: &mut &str) -> ModalResult<Spanned<Type2>> {
    wsc0(input);
    let t2 = parse_type2(input)?;
    Ok(Spanned::new(t2, Span::new(0, 0)))
}

/// Parse a `type2` atom.
fn parse_type2(input: &mut &str) -> ModalResult<Type2> {
    wsc0(input);

    alt((
        // Tagged: #6.NN(type)
        parse_tagged,
        // Major type: #N  or bare #
        parse_major_type,
        // Unwrapped group ref: ~ident
        parse_unwrap,
        // Enumeration: &(group) or &typename  (RFC 8610 §3.9)
        parse_enum_ref,
        // Map: { group }
        parse_map_type,
        // Array: [ group ]
        parse_array_type,
        // Parenthesised group: ( group )
        parse_paren_group,
        // Literal values
        parse_value_type,
        // Typename (possibly with generic args)
        parse_typename,
    ))
    .context(StrContext::Expected(StrContextValue::Description("type expression")))
    .parse_next(input)
}

fn parse_tagged(input: &mut &str) -> ModalResult<Type2> {
    literal("#6.").parse_next(input)?;
    let tag_num: u64 = parse_uint_literal(input)?;
    ws0(input);
    let _ = cut_err(literal("("))
        .context(StrContext::Expected(StrContextValue::Description("'(' after tag number")))
        .parse_next(input)?;
    ws0(input);
    let inner = parse_type(input)?;
    ws0(input);
    cut_err(literal(")"))
        .context(StrContext::Expected(StrContextValue::Description("')' to close tagged type")))
        .parse_next(input)?;
    Ok(Type2::Tagged {
        tag:   tag_num,
        inner: Box::new(Spanned::new(inner, Span::new(0, 0))),
    })
}

fn parse_major_type(input: &mut &str) -> ModalResult<Type2> {
    literal("#").parse_next(input)?;
    if let Ok(d) = one_of::<_, _, ()>(['0','1','2','3','4','5','6','7']).parse_next(input) {
        Ok(Type2::MajorType(d as u8 - b'0'))
    } else {
        Ok(Type2::Any)
    }
}

fn parse_unwrap(input: &mut &str) -> ModalResult<Type2> {
    literal("~").parse_next(input)?;
    ws0(input);
    let name = parse_ident(input)?;
    Ok(Type2::Unwrap(Spanned::new(Ident::new(name), Span::new(0, 0))))
}

/// Parse an enumeration expression: `&(group)` or `&typename` (RFC 8610 §3.9).
fn parse_enum_ref(input: &mut &str) -> ModalResult<Type2> {
    literal("&").parse_next(input)?;
    ws0(input);
    if input.starts_with('(') {
        // &( group )
        let _ = literal("(").parse_next(input)?;
        ws0(input);
        let group = parse_group_entry_inner(input, ')')?;
        ws0(input);
        cut_err(literal(")"))
            .context(StrContext::Expected(StrContextValue::Description("')' to close enum group")))
            .parse_next(input)?;
        Ok(Type2::EnumGroup(Spanned::new(group, Span::new(0, 0))))
    } else {
        // &typename [genericargs]
        let name = cut_err(parse_ident)
            .context(StrContext::Expected(StrContextValue::Description("identifier after '&'")))
            .parse_next(input)?;
        ws0(input);
        let args: Vec<Spanned<Type>> = if input.starts_with('<') {
            let _ = literal::<_, _, ContextError>("<").parse_next(input)?;
            let args: Vec<Type> = separated(
                1..,
                |i: &mut &str| { ws0(i); let t = parse_type(i)?; ws0(i); Ok(t) },
                (ws0_fn, literal(","), ws0_fn),
            )
            .parse_next(input)?;
            cut_err(literal(">"))
                .context(StrContext::Expected(StrContextValue::Description("'>' to close generic args")))
                .parse_next(input)?;
            args.into_iter().map(|t| Spanned::new(t, Span::new(0, 0))).collect()
        } else {
            vec![]
        };
        Ok(Type2::EnumTypename {
            name: Spanned::new(Ident::new(name), Span::new(0, 0)),
            args,
        })
    }
}

fn parse_map_type(input: &mut &str) -> ModalResult<Type2> {
    literal("{").parse_next(input)?;
    ws0(input);
    let group = parse_group_entry_inner(input, '}')?;
    ws0(input);
    cut_err(literal("}"))
        .context(StrContext::Expected(StrContextValue::Description("'}' to close map")))
        .parse_next(input)?;
    Ok(Type2::Map(Spanned::new(group, Span::new(0, 0))))
}

fn parse_array_type(input: &mut &str) -> ModalResult<Type2> {
    literal("[").parse_next(input)?;
    ws0(input);
    let group = parse_group_entry_inner(input, ']')?;
    ws0(input);
    cut_err(literal("]"))
        .context(StrContext::Expected(StrContextValue::Description("']' to close array")))
        .parse_next(input)?;
    Ok(Type2::Array(Spanned::new(group, Span::new(0, 0))))
}

fn parse_paren_group(input: &mut &str) -> ModalResult<Type2> {
    literal("(").parse_next(input)?;
    ws0(input);
    let group = parse_group_entry_inner(input, ')')?;
    ws0(input);
    cut_err(literal(")"))
        .context(StrContext::Expected(StrContextValue::Description("')' to close group")))
        .parse_next(input)?;
    Ok(Type2::Group(Spanned::new(group, Span::new(0, 0))))
}

fn parse_value_type(input: &mut &str) -> ModalResult<Type2> {
    parse_value(input).map(Type2::Value)
}

fn parse_typename(input: &mut &str) -> ModalResult<Type2> {
    let name = parse_ident(input)?;
    ws0(input);

    // Generic args: `<type, type>`
    let args: Vec<Spanned<Type>> = if input.starts_with('<') {
        let _ = literal::<_, _, ContextError>("<").parse_next(input)?;
        let args: Vec<Type> = separated(
            1..,
            |i: &mut &str| { ws0(i); let t = parse_type(i)?; ws0(i); Ok(t) },
            (ws0_fn, literal(","), ws0_fn),
        )
        .parse_next(input)?;
        cut_err(literal(">"))
            .context(StrContext::Expected(StrContextValue::Description("'>' to close generic args")))
            .parse_next(input)?;
        args.into_iter()
            .map(|t| Spanned::new(t, Span::new(0, 0)))
            .collect()
    } else {
        vec![]
    };

    Ok(Type2::Typename {
        name: Spanned::new(Ident::new(name), Span::new(0, 0)),
        args,
    })
}

// ── Group parsing ─────────────────────────────────────────────────────────────

pub(crate) fn parse_group_entry(
    input: &mut &str,
    _src:  &str,
) -> ModalResult<GroupEntry> {
    parse_group_entry_inner(input, '\0')
}

/// Parse the interior of a group (between `{}`/`[]`/`()`), producing a
/// `GroupEntry` (a list of group choices separated by `//`).
fn parse_group_entry_inner(
    input:    &mut &str,
    close_ch: char,
) -> ModalResult<GroupEntry> {
    let choices = separated(
        0..,
        |i: &mut &str| parse_group_choice(i, close_ch),
        (ws0_fn, literal("//"), ws0_fn),
    )
    .parse_next(input)?;

    Ok(GroupEntry { choices })
}

/// Parse one group choice: a sequence of member entries separated by `,`.
///
/// Comments between entries document them (dgtlrift/cddlc#8): the lines directly above
/// an entry (no blank line in between), an `@doc` pragma among them, and a comment on
/// the line an entry ends on, before or after its comma.
fn parse_group_choice(
    input:    &mut &str,
    close_ch: char,
) -> ModalResult<GroupChoice> {
    let mut entries: Vec<Spanned<MemberEntry>> = Vec::new();
    let mut above = wsc_collect(input);

    loop {
        // Stop at closing delimiter or `//` separator
        if input.is_empty()
            || (close_ch != '\0' && input.starts_with(close_ch))
            || input.starts_with("//")
        {
            break;
        }

        let start = here(input);
        let mut entry = parse_member_entry(input)?;
        let end = here(input);
        entry.doc = above.leading();

        let mut after = wsc_collect(input);  // after value, before comma
        let comma = input.starts_with(',');
        if comma {
            let _ = literal::<_, _, ContextError>(",").parse_next(input)?;
            after.extend(wsc_collect(input));  // `value, ; comment\n`
        }
        if let Some(t) = after.trailing() {
            push_doc(&mut entry.doc, &t);
        }
        entries.push(Spanned::new(entry, Span::new(start, end)));
        if !comma {
            break;
        }
        above = after;
    }

    Ok(GroupChoice { entries })
}

/// Parse a single member entry (occurrence? key: value  or  occurrence? type).
fn parse_member_entry(input: &mut &str) -> ModalResult<MemberEntry> {
    wsc0(input);  // handle inline comment before entry

    // Occurrence indicator
    let occurrence = parse_occurrence(input)?;

    wsc0(input);

    // Try key: value first, then bare type
    let kind = if let Some(kv) = try_parse_key_value(input)? {
        kv
    } else {
        let ty = parse_type(input)?;
        MemberKind::Type(Spanned::new(ty, Span::new(0, 0)))
    };

    Ok(MemberEntry {
        occurrence: occurrence.map(|o| Spanned::new(o, Span::new(0, 0))),
        kind,
        pragmas: vec![],  // member-level pragmas handled at rule level
        doc: None,
        source: None,
    })
}

/// Try to parse a key-value pair.  Returns `None` if the input doesn't look
/// like `key:` or `key=>`.
fn try_parse_key_value(input: &mut &str) -> ModalResult<Option<MemberKind>> {
    // We need to look ahead to distinguish `ident :` from a bare type ref.
    // Strategy: tentatively parse an ident, then check for `:` or `=>`.

    // Save position
    let saved = *input;

    // Attempt to read a memberkey
    let key_result: ModalResult<MemberKey> = (|| {
        ws0(input);
        // Numeric value as key: `1 =>` or `1 :`
        if let Ok(v) = parse_value(input) {
            return Ok(MemberKey::Value(v));
        }
        // Ident as bareword key: `field-name :`
        if let Ok(name) = parse_ident(input) {
            return Ok(MemberKey::Bareword(Ident::new(name)));
        }
        Err(ErrMode::Backtrack(ContextError::new()))
    })();

    let key = match key_result {
        Ok(k) => k,
        Err(_) => {
            *input = saved;
            return Ok(None);
        }
    };

    // Skip whitespace between key and operator — critical for `field : type`
    ws0(input);
    let (cut, found) = if input.starts_with("=>") {
        let _ = literal::<_, _, ContextError>("=>").parse_next(input)?;
        (true, true)
    } else if input.starts_with(':') && !input.starts_with("::") {
        let _ = literal::<_, _, ContextError>(":").parse_next(input)?;
        (false, true)
    } else {
        (false, false)
    };

    if !found {
        *input = saved;
        return Ok(None);
    }

    ws0(input);
    let value = parse_type(input)?;

    Ok(Some(MemberKind::KeyValue {
        key:   Spanned::new(key, Span::new(0, 0)),
        value: Spanned::new(value, Span::new(0, 0)),
        cut,
    }))
}

// ── Occurrence ────────────────────────────────────────────────────────────────

fn parse_occurrence(input: &mut &str) -> ModalResult<Option<Occurrence>> {
    ws0(input);

    if input.starts_with('?') {
        let _ = literal::<_, _, ContextError>("?").parse_next(input)?;
        return Ok(Some(Occurrence::Optional));
    }

    // Try n*m, *n, n*, *
    if let Ok(occ) = parse_count_occurrence(input) {
        return Ok(Some(occ));
    }

    Ok(None)
}

fn parse_count_occurrence(input: &mut &str) -> ModalResult<Occurrence> {
    // Forms: `*`, `+`, `n*m`, `n*`, `*m`, `n+`
    let saved = *input;

    // Leading number
    let min: Option<u64> = if input.chars().next().map_or(false, |c| c.is_ascii_digit()) {
        let s = digit1::<_, ContextError>.parse_next(input)?;
        Some(s.parse().unwrap())
    } else {
        None
    };

    if input.starts_with('+') {
        let _ = literal::<_, _, ContextError>("+").parse_next(input)?;
        return Ok(if min.is_none() {
            Occurrence::OneOrMore
        } else {
            Occurrence::Bounded { min, max: None }
        });
    }

    if input.starts_with('*') {
        let _ = literal::<_, _, ContextError>("*").parse_next(input)?;
        // Trailing number?
        let max: Option<u64> = if input.chars().next().map_or(false, |c| c.is_ascii_digit()) {
            let s = digit1::<_, ContextError>.parse_next(input)?;
            Some(s.parse().unwrap())
        } else {
            None
        };
        return Ok(if min.is_none() && max.is_none() {
            Occurrence::ZeroOrMore
        } else {
            Occurrence::Bounded { min, max }
        });
    }

    // If we consumed a number but found no operator, backtrack
    if min.is_some() {
        *input = saved;
    }

    Err(ErrMode::Backtrack(ContextError::new()))
}

// ── Value literals ────────────────────────────────────────────────────────────

fn parse_value(input: &mut &str) -> ModalResult<Value> {
    alt((
        parse_bytes_value,
        parse_text_value,
        parse_number_value,
    ))
    .parse_next(input)
}

fn parse_number_value(input: &mut &str) -> ModalResult<Value> {
    // Negative integer or float
    let negative = if input.starts_with('-') {
        let _ = literal::<_, _, ContextError>("-").parse_next(input)?;
        true
    } else {
        false
    };

    // 0x hex
    if input.starts_with("0x") || input.starts_with("0X") {
        let _ = literal::<_, _, ContextError>("0x").parse_next(input)?;
        let hex = hex_digit1::<_, ContextError>.parse_next(input)?;
        let n = u64::from_str_radix(hex, 16)
            .map_err(|_| ErrMode::Backtrack(ContextError::new()))?;
        return Ok(if negative {
            Value::Int(-(n as i64))
        } else {
            Value::Uint(n)
        });
    }

    let digits = digit1::<_, ContextError>.parse_next(input)?;

    // Float check — but NOT if the next char is another '.' (range operator ..)
    if (input.starts_with('.') && !input.starts_with(".."))
        || input.starts_with('e') || input.starts_with('E')
    {
        let mut num_str = if negative { format!("-{digits}") } else { digits.to_owned() };
        if input.starts_with('.') {
            let _ = literal::<_, _, ContextError>(".").parse_next(input)?;
            let frac = digit1::<_, ContextError>.parse_next(input)?;
            num_str.push('.');
            num_str.push_str(frac);
        }
        if input.starts_with('e') || input.starts_with('E') {
            let e = one_of::<_, _, ContextError>(['e','E']).parse_next(input)?;
            num_str.push(e);
            if input.starts_with('+') || input.starts_with('-') {
                let s = one_of::<_, _, ContextError>(['+','-']).parse_next(input)?;
                num_str.push(s);
            }
            let exp = digit1::<_, ContextError>.parse_next(input)?;
            num_str.push_str(exp);
        }
        let f: f64 = num_str.parse()
            .map_err(|_| ErrMode::Backtrack(ContextError::new()))?;
        return Ok(Value::Float(f));
    }

    let n: u64 = digits.parse()
        .map_err(|_| ErrMode::Backtrack(ContextError::new()))?;

    Ok(if negative {
        Value::Int(-(n as i64))
    } else {
        Value::Uint(n)
    })
}

fn parse_text_value(input: &mut &str) -> ModalResult<Value> {
    literal("\"").parse_next(input)?;
    let mut s = String::new();
    loop {
        match any::<_, ContextError>.parse_next(input)? {
            '"'  => break,
            '\\' => {
                let esc = any::<_, ContextError>.parse_next(input)?;
                match esc {
                    '"'  => s.push('"'),
                    '\\' => s.push('\\'),
                    'n'  => s.push('\n'),
                    'r'  => s.push('\r'),
                    't'  => s.push('\t'),
                    other => { s.push('\\'); s.push(other); }
                }
            }
            c => s.push(c),
        }
    }
    Ok(Value::Text(s))
}

fn parse_bytes_value(input: &mut &str) -> ModalResult<Value> {
    // h'...' hex bytes
    if input.starts_with("h'") {
        let _ = literal::<_, _, ContextError>("h'").parse_next(input)?;
        let hex = take_till::<_, _, ContextError>(0.., |c| c == '\'')
            .parse_next(input)?;
        let _ = literal::<_, _, ContextError>("'").parse_next(input)?;
        let hex_clean: String = hex.chars().filter(|c| !c.is_whitespace()).collect();
        let bytes = (0..hex_clean.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex_clean[i..i+2], 16))
            .collect::<Result<Vec<u8>, _>>()
            .map_err(|_| ErrMode::Backtrack(ContextError::new()))?;
        return Ok(Value::Bytes(bytes));
    }

    // b64'...' base64 bytes
    if input.starts_with("b64'") {
        let _ = literal::<_, _, ContextError>("b64'").parse_next(input)?;
        let b64 = take_till::<_, _, ContextError>(0.., |c| c == '\'')
            .parse_next(input)?;
        let _ = literal::<_, _, ContextError>("'").parse_next(input)?;
        // Minimal base64 decode (stdlib has none; produce raw for now)
        // In a real build, use the `base64` crate.
        return Ok(Value::Bytes(b64.as_bytes().to_vec()));
    }

    // '...' UTF-8 bytes
    if input.starts_with('\'') {
        let _ = one_of::<_, _, ContextError>('\'').parse_next(input)?;
        let content = take_till::<_, _, ContextError>(0.., |c| c == '\'')
            .parse_next(input)?;
        let _ = one_of::<_, _, ContextError>('\'').parse_next(input)?;
        return Ok(Value::Bytes(content.as_bytes().to_vec()));
    }

    Err(ErrMode::Backtrack(ContextError::new()))
}

// ── Identifier parsing ────────────────────────────────────────────────────────

/// Parse a CDDL identifier.
/// RFC 8610 §3.1: `id = ALPHA *(*(ALPHA / DIGIT / "-" / ".") (ALPHA / DIGIT))`
///
/// Also handles socket/plug prefixes `$` and `$$`.
pub(crate) fn parse_ident<'a>(input: &mut &'a str) -> ModalResult<&'a str> {
    let start = *input;

    // Optional socket prefix
    let prefix_len = if input.starts_with("$$") {
        2
    } else if input.starts_with('$') {
        1
    } else {
        0
    };
    *input = &input[prefix_len..];

    // Must start with ALPHA (or _ as common extension)
    match input.chars().next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => {
            *input = start;
            return Err(ErrMode::Backtrack(ContextError::new()));
        }
    }

    let len = input
        .char_indices()
        .take_while(|(i, c)| {
            *i == 0 && (c.is_ascii_alphabetic() || *c == '_')
                || *i > 0 && (c.is_ascii_alphanumeric() || *c == '-' || *c == '_' || *c == '.')
        })
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);

    if len == 0 {
        *input = start;
        return Err(ErrMode::Backtrack(ContextError::new()));
    }

    *input = &input[len..];
    // Adjust for prefix already consumed
    Ok(&start[..prefix_len + len])
}

fn parse_uint_literal(input: &mut &str) -> ModalResult<u64> {
    let s = digit1::<_, ContextError>.parse_next(input)?;
    s.parse().map_err(|_| ErrMode::Backtrack(ContextError::new()))
}

// ── Whitespace helpers ────────────────────────────────────────────────────────

fn ws0(input: &mut &str) {
    let _ = multispace0::<_, ()>.parse_next(input);
}

fn ws0_fn(input: &mut &str) -> ModalResult<()> {
    let _ = multispace0::<_, ()>.parse_next(input);
    Ok(())
}

/// Skip whitespace AND any `;`-to-end-of-line comments.
/// Used inside group/type parsing where inline comments can appear
/// after a value or comma on the same line.
fn wsc0(input: &mut &str) {
    loop {
        let _ = multispace0::<_, ()>.parse_next(input);
        if input.starts_with(';') {
            // Consume `;` and everything up to (but not including) the newline
            let _ = one_of::<_, _, ()>(';').parse_next(input);
            let _ = take_till::<_, _, ()>(0.., |c| c == '\n').parse_next(input);
            // The newline itself will be consumed by multispace0 on the next iteration
        } else {
            break;
        }
    }
}

fn wsc0_fn(input: &mut &str) -> ModalResult<()> {
    wsc0(input);
    Ok(())
}

// ── Offset helpers ────────────────────────────────────────────────────────────

fn byte_offset(src: &str, remaining: &str) -> usize {
    src.len() - remaining.len()
}

/// The 1-based line of `offset`. (`str::lines` counted one short at a line's start.)
fn line_of(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())].matches('\n').count() + 1
}

// ── Comments as documentation (dgtlrift/cddlc#8) ──────────────────────────────

thread_local! {
    /// The length of the source being parsed, so the stateless group parsers can tell
    /// an entry's byte offset from the input that remains.
    static SRC_LEN: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The byte offset of `input` in the source being parsed.
fn here(input: &str) -> usize {
    SRC_LEN.with(std::cell::Cell::get).saturating_sub(input.len())
}

/// A comment's text: after the `;`, with one leading space and trailing blanks removed.
fn comment_text(body: &str) -> String {
    body.strip_prefix(' ').unwrap_or(body).trim_end().to_owned()
}

/// Comment lines as one documentation string, blank first and last lines dropped.
fn join_doc(lines: &[String]) -> Option<String> {
    let start = lines.iter().position(|l| !l.is_empty())?;
    let end = lines.iter().rposition(|l| !l.is_empty())?;
    Some(lines[start..=end].join("\n"))
}

/// Appends `text` to a documentation string, on its own line.
fn push_doc(doc: &mut Option<String>, text: &str) {
    match doc {
        Some(d) => { d.push('\n'); d.push_str(text); }
        None => *doc = Some(text.to_owned()),
    }
}

/// Whitespace and comments skipped between group entries: each comment's text, and
/// the line breaks before it in this stretch.
#[derive(Default)]
struct Stretch {
    comments: Vec<(usize, String)>,
    breaks:   usize,
}

impl Stretch {
    fn extend(&mut self, other: Stretch) {
        let base = self.breaks;
        self.comments.extend(other.comments.into_iter().map(|(b, t)| (base + b, t)));
        self.breaks += other.breaks;
    }

    /// The comments on the line the stretch starts on: they end the previous entry.
    fn trailing(&self) -> Option<String> {
        let lines: Vec<String> = self.comments.iter()
            .filter(|(b, _)| *b == 0)
            .map(|(_, t)| t.clone())
            .collect();
        join_doc(&lines)
    }

    /// The block of comment lines directly above the next entry: consecutive lines,
    /// the last on the line before it.
    fn leading(&self) -> Option<String> {
        let mut block: Vec<String> = Vec::new();
        let mut next = self.breaks;  // the line the entry is on
        for (b, t) in self.comments.iter().rev() {
            if *b == 0 || *b + 1 != next {
                break;
            }
            block.push(t.clone());
            next = *b;
        }
        block.reverse();
        join_doc(&block)
    }
}

/// Skips whitespace and comments like [`wsc0`], keeping the comments: plain text, or
/// an `@doc` pragma's text. Other pragmas are skipped.
fn wsc_collect(input: &mut &str) -> Stretch {
    let mut out = Stretch::default();
    loop {
        let ws = multispace0::<_, ()>.parse_next(input).unwrap_or("");
        out.breaks += ws.matches('\n').count();
        if !input.starts_with(';') {
            break;
        }
        let _ = one_of::<_, _, ()>(';').parse_next(input);
        let body = take_till::<_, _, ()>(0.., |c| c == '\n').parse_next(input).unwrap_or("");
        let text = match parse_pragma(body, Span::new(0, 0), &PathBuf::new(), 0) {
            None => Some(comment_text(body)),
            Some(Ok(Pragma { value: PragmaValue::Doc(d), .. })) => Some(d),
            Some(_) => None,
        };
        if let Some(t) = text {
            out.comments.push((out.breaks, t));
        }
    }
    out
}

// ── Source lines (dgtlrift/cddlc#9) ───────────────────────────────────────────

/// The line holding `offset`: its number and text, trimmed.
fn source_line(src: &str, path: &std::path::Path, offset: usize) -> SourceLine {
    let offset = offset.min(src.len());
    let start = src[..offset].rfind('\n').map_or(0, |i| i + 1);
    let end = src[offset..].find('\n').map_or(src.len(), |i| offset + i);
    SourceLine {
        path: path.to_owned(),
        line: line_of(src, offset),
        text: src[start..end].trim().to_owned(),
    }
}

fn sources_in_group(g: &mut GroupEntry, src: &str, path: &std::path::Path) {
    for choice in &mut g.choices {
        for e in &mut choice.entries {
            e.node.source = Some(source_line(src, path, e.span.start));
            match &mut e.node.kind {
                MemberKind::KeyValue { value, .. } => sources_in_type(&mut value.node, src, path),
                MemberKind::Type(t) => sources_in_type(&mut t.node, src, path),
            }
        }
    }
}

fn sources_in_type(t: &mut Type, src: &str, path: &std::path::Path) {
    for c in &mut t.choices {
        match &mut c.node.base.node {
            Type2::Map(g) | Type2::Array(g) | Type2::Group(g) | Type2::EnumGroup(g) => {
                sources_in_group(&mut g.node, src, path);
            }
            Type2::Tagged { inner, .. } => sources_in_type(&mut inner.node, src, path),
            _ => {}
        }
    }
}

fn col_of(src: &str, offset: usize) -> usize {
    let slice = &src[..offset.min(src.len())];
    slice.rfind('\n').map_or(offset + 1, |pos| offset - pos)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Identifier ────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_ident_simple() {
        let mut s = "my-type";
        assert_eq!(parse_ident(&mut s).unwrap(), "my-type");
        assert!(s.is_empty());
    }

    #[test]
    fn test_parse_ident_socket() {
        let mut s = "$$extensible-group";
        let id = parse_ident(&mut s).unwrap();
        assert_eq!(id, "$$extensible-group");
    }

    #[test]
    fn test_parse_ident_stops_at_space() {
        let mut s = "my-type = uint";
        let id = parse_ident(&mut s).unwrap();
        assert_eq!(id, "my-type");
        assert_eq!(s, " = uint");
    }

    // ── Values ────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_uint() {
        let mut s = "42";
        assert_eq!(parse_value(&mut s).unwrap(), Value::Uint(42));
    }

    #[test]
    fn test_parse_negative_int() {
        let mut s = "-7";
        assert_eq!(parse_value(&mut s).unwrap(), Value::Int(-7));
    }

    #[test]
    fn test_parse_float() {
        let mut s = "3.14";
        assert!(matches!(parse_value(&mut s).unwrap(), Value::Float(_)));
    }

    #[test]
    fn test_parse_text() {
        let mut s = r#""hello world""#;
        assert_eq!(parse_value(&mut s).unwrap(), Value::Text("hello world".into()));
    }

    #[test]
    fn test_parse_hex_bytes() {
        let mut s = "h'deadbeef'";
        assert_eq!(
            parse_value(&mut s).unwrap(),
            Value::Bytes(vec![0xde, 0xad, 0xbe, 0xef])
        );
    }

    // ── Types ─────────────────────────────────────────────────────────────────

    #[test]
    fn test_parse_typename() {
        let ty = parse_type_str("uint").unwrap();
        assert_eq!(ty.choices.len(), 1);
    }

    #[test]
    fn test_parse_type_choice() {
        let ty = parse_type_str("uint / tstr / bool").unwrap();
        assert_eq!(ty.choices.len(), 3);
    }

    #[test]
    fn test_parse_range_op() {
        let ty = parse_type_str("0..255").unwrap();
        assert_eq!(ty.choices.len(), 1);
        let op = ty.choices[0].node.ops.first().unwrap();
        assert!(matches!(op, TypeOp::Range { inclusive: true, .. }));
    }

    #[test]
    fn test_parse_control_op() {
        let ty = parse_type_str("tstr .size 64").unwrap();
        let op = ty.choices[0].node.ops.first().unwrap();
        assert!(matches!(op, TypeOp::Control { op, .. } if op.node == CtlOp::Size));
    }

    #[test]
    fn test_parse_tagged_type() {
        let ty = parse_type_str("#6.1(tstr)").unwrap();
        let t2 = &ty.choices[0].node.base.node;
        assert!(matches!(t2, Type2::Tagged { tag: 1, .. }));
    }

    #[test]
    fn test_parse_map_type() {
        let ty = parse_type_str("{ field: uint }").unwrap();
        assert!(matches!(ty.choices[0].node.base.node, Type2::Map(_)));
    }

    #[test]
    fn test_parse_array_type() {
        let ty = parse_type_str("[ * uint ]").unwrap();
        assert!(matches!(ty.choices[0].node.base.node, Type2::Array(_)));
    }

    // ── Occurrence ────────────────────────────────────────────────────────────

    #[test]
    fn test_occurrence_optional() {
        let mut s = "?";
        let occ = parse_occurrence(&mut s).unwrap();
        assert_eq!(occ, Some(Occurrence::Optional));
    }

    #[test]
    fn test_occurrence_zero_or_more() {
        let mut s = "*";
        let occ = parse_occurrence(&mut s).unwrap();
        assert_eq!(occ, Some(Occurrence::ZeroOrMore));
    }

    #[test]
    fn test_occurrence_bounded() {
        let mut s = "2*5";
        let occ = parse_occurrence(&mut s).unwrap();
        assert_eq!(occ, Some(Occurrence::Bounded { min: Some(2), max: Some(5) }));
    }

    #[test]
    fn test_occurrence_one_or_more() {
        let mut s = "+";
        let occ = parse_occurrence(&mut s).unwrap();
        assert_eq!(occ, Some(Occurrence::OneOrMore));
    }

    // ── Full module parse ─────────────────────────────────────────────────────

    #[test]
    fn test_inline_comment_after_field() {
        // Inline comment after a field value — the original failing case
        let src = r#"TestFormat = (
    field_a: uint .size 4, ; This comment broke the parser
    field_b: tstr,         ; Another inline comment
)"#;
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        assert_eq!(result.value.rules.len(), 1);
    }

    #[test]
    fn test_inline_comment_after_last_field() {
        // Inline comment on the last field (no trailing comma)
        let src = "sensor = { id: uint, value: float32 } ; end comment";
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        assert_eq!(result.value.rules.len(), 1);
    }

    #[test]
    fn test_inline_comment_in_type_choice() {
        let src = "status = \"ok\" ; ok variant\n    / \"warn\" ; warn variant\n    / \"error\"";
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        assert_eq!(result.value.rules.len(), 1);
    }

    #[test]
    fn test_parse_simple_module() {
        let src = r#"
; @import "common.cddl"

; @capacity 16
; @doc "A sensor reading"
sensor-reading = {
    id:    uint,
    temp:  float32,
    label: tstr,
}
"#;
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        assert_eq!(result.value.file_pragmas.len(), 1);
        assert_eq!(result.value.rules.len(), 1);
        assert_eq!(result.value.rules[0].node.pragmas.len(), 2);
    }

    #[test]
    fn test_parse_generic_rule() {
        let src = "labeled<T> = { label: tstr, value: T }";
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        let rule = &result.value.rules[0].node;
        assert_eq!(rule.generic_params.len(), 1);
        assert_eq!(rule.generic_params[0].node.as_str(), "T");
    }

    #[test]
    fn test_parse_type_choice_rule() {
        let src = "status = \"ok\" / \"error\" / \"pending\"";
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        assert_eq!(result.value.rules.len(), 1);
    }

    #[test]
    fn test_parse_type_extend() {
        let src = "my-type /= tstr";
        let result = parse_cddl(src, PathBuf::from("test.cddl")).unwrap();
        let kind = &result.value.rules[0].node.kind;
        assert!(matches!(kind, RuleKind::Type { extend: true, .. }));
    }
}
