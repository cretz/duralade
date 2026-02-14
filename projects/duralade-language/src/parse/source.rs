use crate::model::*;

use super::basic::try_parse_ident;
use super::expr::parse_expr;
use super::{ParseContext, ParseError};

/// Parse an import declaration
/// Spec 6.3 - import = "import" identifier_qualified [ import_alias ] .
pub(crate) fn try_parse_import(ctx: &mut ParseContext) -> Result<Option<Import>, ParseError> {
    let start_pos = ctx.pos();

    // Check if we see "import " (with space)
    if !ctx.remaining().starts_with("import ") {
        return Ok(None);
    }

    // Consume "import "
    ctx.advance(7);

    // Skip trivia and check for strict mode (should be no additional trivia)
    if ctx.parse_trivia() != "" {
        ctx.add_strict_violation(ParseError::new(
            (start_pos + 7, ctx.pos()),
            "Expected no additional whitespace after 'import '".to_string(),
        ));
    }

    // Parse qualified identifier path (track error but continue)
    let mut error = None;
    let path = match ctx.parse_qualified_ident() {
        Ok(path) => path,
        Err(err) => {
            error = Some(err);
            Vec::new() // Continue with empty path
        }
    };

    // Check for optional "as" alias
    let end_pos = ctx.pos();
    let before_as = ctx.pos();
    let skipped_was_space = ctx.parse_trivia() == " ";
    let alias = if ctx.remaining().starts_with("as ") {
        // Check spacing before "as" (strict mode)
        if !skipped_was_space {
            ctx.add_strict_violation(ParseError::new(
                (before_as, ctx.pos()),
                "Expected exactly one space before 'as'".to_string(),
            ));
        }

        // Consume "as "
        let after_as = ctx.pos() + 3;
        ctx.advance(3);

        // Skip trivia after "as " (strict mode check)
        if ctx.parse_trivia() != "" {
            ctx.add_strict_violation(ParseError::new(
                (after_as, ctx.pos()),
                "Expected no additional whitespace after 'as '".to_string(),
            ));
        }

        // Parse alias identifier
        match try_parse_ident(ctx) {
            Ok(Some(ident)) => Some(ident),
            Ok(None) => {
                if error.is_none() {
                    error = Some(ParseError::with_range(
                        (before_as, ctx.pos()),
                        "Expected identifier after 'as'".to_string(),
                        (ctx.pos(), ctx.pos() + ctx.next_char_len()),
                    ));
                }
                None
            }
            Err(err) => {
                if error.is_none() {
                    error = Some(err);
                }
                None
            }
        }
    } else {
        None
    };

    // If we had an error, skip past end of line for error recovery
    if let Some(err) = error {
        ctx.skip_past_end_of_line();
        return Err(err.wrap((start_pos, ctx.pos()), "Invalid import".to_string()));
    }

    // Verify we're at newline, start of next line, or EOF
    if !ctx.remaining().is_empty() && !ctx.is_at_newline() && !ctx.is_at_line_start() {
        let ch = ctx.remaining().chars().next().unwrap();
        ctx.skip_past_end_of_line();
        return Err(ParseError::with_range(
            (start_pos, ctx.pos()),
            "Unexpected characters after import".to_string(),
            (ctx.pos(), ctx.pos() + ch.len_utf8()),
        )
        .wrap((start_pos, ctx.pos()), "Invalid import".to_string()));
    }

    let span_end = if alias.is_some() { ctx.pos() } else { end_pos };
    Ok(Some(Import {
        node_id: ctx.alloc_node_id(start_pos, span_end),
        path,
        alias,
    }))
}

/// Parse annotations, splitting into groups at each blank line (Spec 6.4).
/// Consumes trailing trivia. Callers should NOT call parse_trivia() after.
pub(crate) fn parse_annotations(ctx: &mut ParseContext) -> Vec<Vec<Annotation>> {
    let mut groups: Vec<Vec<Annotation>> = Vec::new();
    let mut current: Vec<Annotation> = Vec::new();
    loop {
        ctx.parse_trivia();
        if ctx.trivia_had_blank_line && !current.is_empty() {
            groups.push(std::mem::take(&mut current));
        }
        match try_parse_annotation(ctx) {
            Ok(Some(ann)) => current.push(ann),
            Ok(None) => break,
            Err(err) => ctx.add_parse_error(err),
        }
    }
    if !current.is_empty() {
        groups.push(current);
    }
    // Signal trailing blank line between last annotation and target
    if ctx.trivia_had_blank_line && !groups.is_empty() {
        groups.push(Vec::new());
    }
    groups
}

/// Parse an annotation: `@` invocation
/// Spec 6.4 - annotation = "@" invocation .
pub(crate) fn try_parse_annotation(
    ctx: &mut ParseContext,
) -> Result<Option<Annotation>, ParseError> {
    let start_pos = ctx.pos();

    // Check if we see '@'
    if !ctx.remaining().starts_with('@') {
        return Ok(None); // Not an annotation
    }

    // Consume '@'
    ctx.advance(1);

    // Parse expression after '@' - should be ident, access, or invocation
    let Some(expr) = parse_expr(ctx) else {
        return Err(ParseError::new(
            (start_pos, ctx.pos()),
            "Expected expression after '@'".to_string(),
        ));
    };

    // If already an invocation, use it directly. Otherwise wrap as zero-arg invocation.
    let invocation = match expr {
        Expr::Invocation(inv) => inv,
        _ => ExprInvocation {
            node_id: expr.node_id(),
            expr: Box::new(expr),
            type_args: Vec::new(),
            args: Vec::new(),
        },
    };

    // Validate base is an identifier or dot-access chain (e.g. @foo, @foo.bar(x))
    fn is_ident_or_access(expr: &Expr) -> bool {
        match expr {
            Expr::Ident(_) => true,
            Expr::Access(a) => is_ident_or_access(&a.expr),
            _ => false,
        }
    }
    if !is_ident_or_access(&invocation.expr) {
        ctx.add_parse_error(ParseError::new(
            (start_pos, ctx.pos()),
            "Annotation must be an identifier or dot-access chain".to_string(),
        ));
    }

    Ok(Some(Annotation {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        invocation,
    }))
}
