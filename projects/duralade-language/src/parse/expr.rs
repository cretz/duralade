use crate::model::*;

use super::basic::try_parse_ident;
use super::construct::{parse_data, parse_func, starts_with_keyword};
use super::types::{parse_type, parse_type_arguments};
use super::{ParseContext, ParseError};

/// Parse `else! expr`. Returns None if `else!` not found at current position.
fn parse_else_bang(ctx: &mut ParseContext) -> Option<Expr> {
    if !ctx.remaining().starts_with("else!") {
        return None;
    }
    ctx.advance(5);
    ctx.skip_spaces();
    let else_start = ctx.pos();
    Some(match parse_expr(ctx) {
        Some(e) => e,
        None => {
            ctx.add_parse_error(ParseError::new(
                (else_start, ctx.pos()),
                "Expected expression after 'else!'".to_string(),
            ));
            Expr::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(else_start, else_start),
            })
        }
    })
}

/// Binary operator precedence (higher = tighter binding)
fn binary_op_precedence(op: &ExprBinaryOp) -> u8 {
    match op {
        ExprBinaryOp::NilCoalesce => 1,
        ExprBinaryOp::Or => 2,
        ExprBinaryOp::And => 3,
        ExprBinaryOp::Eq | ExprBinaryOp::Ne => 4,
        ExprBinaryOp::Lt | ExprBinaryOp::Le | ExprBinaryOp::Gt | ExprBinaryOp::Ge => 5,
        ExprBinaryOp::Add | ExprBinaryOp::Sub => 6,
        ExprBinaryOp::Mul | ExprBinaryOp::Div | ExprBinaryOp::Mod => 7,
    }
}

/// Try to match a binary operator at the start of `remaining`
fn try_parse_binary_op(remaining: &str) -> Option<(ExprBinaryOp, usize)> {
    // Two-char operators first
    if remaining.starts_with("==") {
        Some((ExprBinaryOp::Eq, 2))
    } else if remaining.starts_with("!=") {
        Some((ExprBinaryOp::Ne, 2))
    } else if remaining.starts_with("<=") {
        Some((ExprBinaryOp::Le, 2))
    } else if remaining.starts_with(">=") {
        Some((ExprBinaryOp::Ge, 2))
    } else if remaining.starts_with("&&") {
        Some((ExprBinaryOp::And, 2))
    } else if remaining.starts_with("||") {
        Some((ExprBinaryOp::Or, 2))
    } else if remaining.starts_with("??") {
        Some((ExprBinaryOp::NilCoalesce, 2))
    } else if remaining.starts_with('+') && !remaining.starts_with("+=") {
        Some((ExprBinaryOp::Add, 1))
    } else if remaining.starts_with('-') && !remaining.starts_with("-=") {
        Some((ExprBinaryOp::Sub, 1))
    } else if remaining.starts_with('*') && !remaining.starts_with("*=") {
        Some((ExprBinaryOp::Mul, 1))
    } else if remaining.starts_with('/') && !remaining.starts_with("/=") {
        Some((ExprBinaryOp::Div, 1))
    } else if remaining.starts_with('%') && !remaining.starts_with("%=") {
        Some((ExprBinaryOp::Mod, 1))
    } else if remaining.starts_with('<') {
        Some((ExprBinaryOp::Lt, 1))
    } else if remaining.starts_with('>') {
        Some((ExprBinaryOp::Gt, 1))
    } else {
        None
    }
}

/// Parse an expression. Returns None if no expression found at current position.
pub(crate) fn parse_expr(ctx: &mut ParseContext) -> Option<Expr> {
    parse_binary(ctx, 0)
}

/// Pratt parser: parse binary expression with minimum precedence
fn parse_binary(ctx: &mut ParseContext, min_prec: u8) -> Option<Expr> {
    let start = ctx.pos();
    let mut left = parse_unary(ctx)?;

    loop {
        let before_spaces = ctx.pos();
        let spaces_before = ctx.skip_spaces();

        let Some((op, op_len)) = try_parse_binary_op(ctx.remaining()) else {
            ctx.set_pos(before_spaces);
            break;
        };

        if binary_op_precedence(&op) < min_prec {
            ctx.set_pos(before_spaces);
            break;
        }

        // Consume operator
        let op_pos = ctx.pos();
        ctx.advance(op_len);

        // Strict: exactly one space before operator
        if spaces_before != 1 {
            ctx.add_strict_violation(ParseError::new(
                (before_spaces, before_spaces + spaces_before),
                "Expected exactly one space before operator".to_string(),
            ));
        }
        // Allow line continuation after operator (may cross newlines,
        // comments, blank lines). Snapshot so we can roll back if no
        // right operand is found - the trivia crossing is speculative.
        let after_op = ctx.pos();
        let snap = ctx.snapshot();
        ctx.parse_trivia();

        // Right operand with higher min_prec (left-associative)
        let right = match parse_binary(ctx, binary_op_precedence(&op) + 1) {
            Some(expr) => expr,
            None => {
                // Undo speculative trivia (may have crossed lines, added
                // strict violations, collected comments, etc.)
                ctx.restore(snap);
                ctx.add_parse_error(ParseError::new(
                    (before_spaces, after_op),
                    "Expected expression after operator".to_string(),
                ));
                // Consume rest of the line for clean recovery
                ctx.skip_past_end_of_line();
                let node_id = ctx.alloc_node_id(start, ctx.pos());
                ctx.set_op_position(node_id, op_pos);
                left = Expr::Binary(ExprBinary {
                    node_id,
                    left: Box::new(left),
                    op,
                    right: Box::new(Expr::Invalid(InvalidNode {
                        node_id: ctx.alloc_node_id(after_op, after_op),
                    })),
                });
                break;
            }
        };

        let node_id = ctx.alloc_node_id(start, ctx.pos());
        ctx.set_op_position(node_id, op_pos);
        left = Expr::Binary(ExprBinary {
            node_id,
            left: Box::new(left),
            op,
            right: Box::new(right),
        });
    }

    Some(left)
}

/// Parse prefix unary: `-expr`, `!expr`
fn parse_unary(ctx: &mut ParseContext) -> Option<Expr> {
    let start = ctx.pos();
    let first = ctx.remaining().chars().next()?;

    if first == '-' || first == '!' {
        let op_pos = ctx.pos();
        let op = if first == '-' {
            ExprUnaryOp::Negate
        } else {
            ExprUnaryOp::Not
        };
        ctx.advance(1);
        let operand = match parse_unary(ctx) {
            Some(expr) => expr,
            None => {
                let err_pos = ctx.pos();
                ctx.add_parse_error(ParseError::new(
                    (start, err_pos),
                    format!("Expected expression after '{}'", first),
                ));
                Expr::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(err_pos, err_pos),
                })
            }
        };
        let node_id = ctx.alloc_node_id(start, ctx.pos());
        ctx.set_op_position(node_id, op_pos);
        return Some(Expr::Unary(ExprUnary {
            node_id,
            op,
            expr: Box::new(operand),
        }));
    }

    parse_postfix(ctx)
}

/// Parse postfix: access `.`/`?.`/`->`, early return `!`
fn parse_postfix(ctx: &mut ParseContext) -> Option<Expr> {
    let start = ctx.pos();
    let mut expr = parse_primary(ctx)?;

    loop {
        let remaining = ctx.remaining();

        // Access operators
        let access_op = if remaining.starts_with("?.") {
            Some((ExprAccessOp::NilSafe, 2))
        } else if remaining.starts_with("->") {
            Some((ExprAccessOp::EntityRef, 2))
        } else if remaining.starts_with('.')
            && !remaining[1..].starts_with(|c: char| c.is_ascii_digit())
        {
            Some((ExprAccessOp::Dot, 1))
        } else {
            None
        };

        if let Some((op, len)) = access_op {
            ctx.advance(len);
            // Allow line continuation after access operator
            ctx.parse_trivia();
            match try_parse_ident(ctx) {
                Ok(Some(ident)) => {
                    expr = Expr::Access(ExprAccess {
                        node_id: ctx.alloc_node_id(start, ctx.pos()),
                        expr: Box::new(expr),
                        op,
                        ident,
                    });
                    continue;
                }
                Ok(None) => {
                    ctx.add_parse_error(ParseError::new(
                        (start, ctx.pos()),
                        "Expected identifier after access operator".to_string(),
                    ));
                    break;
                }
                Err(err) => {
                    ctx.add_parse_error(err);
                    break;
                }
            }
        }

        // Nil narrowing: `?!` or `? else! expr` - before `!` early return check
        // In if-narrowing context, skip bare `?` so it's consumed as the condition marker
        if remaining.starts_with('?')
            && !remaining.starts_with("?.")
            && (!ctx.suppress_narrowing_postfix || remaining.starts_with("?!"))
        {
            ctx.advance(1); // consume `?`
            let narrow_start = ctx.pos();
            let else_expr = if ctx.remaining().starts_with('!') {
                if ctx.defer_depth > 0 {
                    ctx.add_parse_error(ParseError::new(
                        (ctx.pos(), ctx.pos() + 1),
                        "Early return '!' is not allowed inside defer blocks".to_string(),
                    ));
                }
                ctx.advance(1);
                None
            } else {
                ctx.skip_spaces();
                match parse_else_bang(ctx) {
                    Some(e) => Some(Box::new(e)),
                    None => {
                        ctx.add_parse_error(
                            ctx.error_here("Expected '!' or 'else!' after '?'".to_string()),
                        );
                        ctx.skip_past_end_of_line();
                        break;
                    }
                }
            };
            expr = Expr::Narrowing(ExprNarrowing {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
                expr: Box::new(expr),
                kind: ExprNarrowingKind::Nil(ExprNarrowingNil {
                    node_id: ctx.alloc_node_id(narrow_start, ctx.pos()),
                    else_expr,
                }),
            });
            continue;
        }

        // Postfix `!` (early return) - but not `!=`
        if remaining.starts_with('!') && !remaining.starts_with("!=") {
            if ctx.defer_depth > 0 {
                ctx.add_parse_error(ParseError::new(
                    (ctx.pos(), ctx.pos() + 1),
                    "Early return '!' is not allowed inside defer blocks".to_string(),
                ));
            }
            let op_pos = ctx.pos();
            ctx.advance(1);
            let node_id = ctx.alloc_node_id(start, ctx.pos());
            ctx.set_op_position(node_id, op_pos);
            expr = Expr::Unary(ExprUnary {
                node_id,
                op: ExprUnaryOp::EarlyReturn,
                expr: Box::new(expr),
            });
            continue;
        }

        // Invocation with optional type args: `expr[type_args](args)` or `expr(args)`
        if remaining.starts_with('[') || remaining.starts_with('(') {
            let type_args = if remaining.starts_with('[') {
                parse_type_arguments(ctx)
            } else {
                Vec::new()
            };
            if ctx.remaining().starts_with('(') {
                let opener_pos = ctx.pos();
                ctx.advance(1);
                ctx.indent(b'(');
                let (args, trailing_comma_pos) = parse_invocation_args(ctx, opener_pos);
                let (had_space, closer_on_own_line) = {
                    let t = ctx.parse_trivia();
                    (!t.is_empty() && !t.contains('\n'), t.contains('\n'))
                };
                ctx.dedent();
                if ctx.remaining().starts_with(')') {
                    if had_space {
                        ctx.add_strict_violation(ParseError::new(
                            (ctx.pos() - 1, ctx.pos()),
                            "No space allowed before ')'".to_string(),
                        ));
                    }
                    let multiline = ctx.is_multiline(opener_pos, ctx.pos());
                    // Only require trailing comma when `)` starts its own line.
                    // When `)` follows `}` on the same line (e.g. `handler = func { ... })`),
                    // there's no natural place for a trailing comma.
                    if multiline
                        && closer_on_own_line
                        && trailing_comma_pos.is_none()
                        && !args.is_empty()
                    {
                        ctx.add_strict_violation(ParseError::new(
                            (ctx.pos(), ctx.pos() + 1),
                            "Trailing comma required in multi-line form".to_string(),
                        ));
                    }
                    if !multiline && let Some(cp) = trailing_comma_pos {
                        ctx.add_strict_violation(ParseError::new(
                            (cp, cp + 1),
                            "Trailing comma not allowed in single-line form".to_string(),
                        ));
                    }
                    ctx.advance(1);
                } else {
                    ctx.add_parse_error(ParseError::new(
                        (start, ctx.pos()),
                        "Expected ')'".to_string(),
                    ));
                }
                expr = Expr::Invocation(ExprInvocation {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    expr: Box::new(expr),
                    type_args,
                    args,
                });
                continue;
            } else {
                // Type args without invocation parens
                ctx.add_parse_error(
                    ctx.error_here("Expected '(' after type arguments".to_string()),
                );
                break;
            }
        }

        // `as` narrowing: `as! type` or `as type else! expr`
        // Requires space before `as` keyword
        // In if-narrowing context, skip bare `as` so it's consumed as the condition keyword
        {
            let before_as = ctx.pos();
            let spaces = ctx.skip_spaces();
            if spaces > 0
                && ctx.remaining().starts_with("as")
                && !ctx.remaining()[2..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
                && (!ctx.suppress_narrowing_postfix || ctx.remaining()[2..].starts_with('!'))
            {
                ctx.advance(2); // consume `as`
                let narrow_start = before_as + spaces; // start of `as`
                let (ty, else_expr) = if ctx.remaining().starts_with('!') {
                    // as! type
                    if ctx.defer_depth > 0 {
                        ctx.add_parse_error(ParseError::new(
                            (ctx.pos(), ctx.pos() + 1),
                            "Early return '!' is not allowed inside defer blocks".to_string(),
                        ));
                    }
                    ctx.advance(1);
                    ctx.skip_spaces();
                    (parse_type(ctx), None)
                } else {
                    // as type else! expr
                    ctx.skip_spaces();
                    let ty = parse_type(ctx);
                    ctx.skip_spaces();
                    match parse_else_bang(ctx) {
                        Some(e) => (ty, Some(Box::new(e))),
                        None => {
                            ctx.add_parse_error(ctx.error_here(
                                "Expected 'else!' after type in 'as' narrowing".to_string(),
                            ));
                            ctx.skip_past_end_of_line();
                            break;
                        }
                    }
                };
                expr = Expr::Narrowing(ExprNarrowing {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    expr: Box::new(expr),
                    kind: ExprNarrowingKind::As(ExprNarrowingAs {
                        node_id: ctx.alloc_node_id(narrow_start, ctx.pos()),
                        ty,
                        else_expr,
                    }),
                });
                continue;
            } else {
                ctx.set_pos(before_as);
            }
        }

        break;
    }

    Some(expr)
}

/// Parse comma-separated invocation arguments inside `(...)`.
/// Assumes opening `(` already consumed. Stops before `)` or at EOF.
/// `opener_pos` is the byte position of `(` for multiline detection.
/// Returns (args, trailing_comma_pos).
fn parse_invocation_args(
    ctx: &mut ParseContext,
    opener_pos: usize,
) -> (Vec<ExprInvocationArg>, Option<usize>) {
    // Strict: no space after '('
    if ctx.remaining().starts_with(' ') {
        ctx.add_strict_violation(ParseError::new(
            (ctx.pos(), ctx.pos() + 1),
            "No space allowed after '('".to_string(),
        ));
    }
    ctx.parse_trivia();
    let mut args = Vec::new();
    let mut trailing_comma_pos: Option<usize> = None;
    let mut prev_item_start = ctx.pos();

    while !ctx.remaining().starts_with(')') && !ctx.remaining().is_empty() {
        let arg_start = ctx.pos();
        let Some(arg_expr) = parse_expr(ctx) else {
            ctx.add_parse_error(ParseError::new(
                (arg_start, ctx.pos()),
                "Expected argument expression".to_string(),
            ));
            break;
        };

        // Check for `=` (named arg) - but not `==`
        let before_eq = ctx.pos();
        let spaces_before_eq = ctx.skip_spaces();
        if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
            if spaces_before_eq != 1 {
                ctx.add_strict_violation(ctx.spacing_error(
                    before_eq,
                    "Expected exactly one space before '=' in argument".to_string(),
                ));
            }
            ctx.advance(1);
            let after_eq = ctx.pos();
            if ctx.skip_spaces() != 1 {
                ctx.add_strict_violation(ctx.spacing_error(
                    after_eq,
                    "Expected exactly one space after '=' in argument".to_string(),
                ));
            }
            ctx.parse_trivia();

            // Name must be a simple identifier
            let name = match &arg_expr {
                Expr::Ident(ident) => ident.clone(),
                _ => {
                    ctx.add_parse_error(ParseError::new(
                        (arg_start, before_eq),
                        "Argument name must be an identifier".to_string(),
                    ));
                    Ident {
                        node_id: ctx.alloc_node_id(arg_start, before_eq),
                        name: Symbol::unknown(),
                        is_raw: false,
                    }
                }
            };

            let value_start = ctx.pos();
            let Some(value) = parse_expr(ctx) else {
                ctx.add_parse_error(ParseError::new(
                    (value_start, ctx.pos()),
                    "Expected expression after '='".to_string(),
                ));
                break;
            };

            if let Expr::Ident(ref val_ident) = value
                && val_ident.name == name.name
            {
                ctx.add_strict_violation(ParseError::new(
                    (arg_start, ctx.pos()),
                    format!(
                        "Redundant argument name '{}', use shorthand form",
                        ctx.ident_str(&name)
                    ),
                ));
            }

            args.push(ExprInvocationArg {
                node_id: ctx.alloc_node_id(arg_start, ctx.pos()),
                name,
                value,
            });
        } else {
            // Shorthand arg - derive name from last identifier
            ctx.set_pos(before_eq);
            let name = match &arg_expr {
                Expr::Ident(ident) => ident.clone(),
                Expr::Access(access) => access.ident.clone(),
                _ => {
                    ctx.add_parse_error(ParseError::new(
                        (arg_start, before_eq),
                        "Argument name required for non-identifier expressions".to_string(),
                    ));
                    Ident {
                        node_id: ctx.alloc_node_id(arg_start, before_eq),
                        name: Symbol::unknown(),
                        is_raw: false,
                    }
                }
            };
            args.push(ExprInvocationArg {
                node_id: ctx.alloc_node_id(arg_start, ctx.pos()),
                name,
                value: arg_expr,
            });
        }

        // Same-line check (skip for first arg)
        if args.len() > 1
            && !ctx.is_multiline(prev_item_start, arg_start)
            && ctx.is_multiline(opener_pos, arg_start)
        {
            ctx.add_strict_violation(ParseError::new(
                (arg_start, arg_start + 1),
                "Only one item per line allowed in multi-line form".to_string(),
            ));
        }
        prev_item_start = arg_start;

        // Peek for `)` before parse_trivia to avoid false indentation violations
        // on the closing paren (same pattern as `}` in block parsing)
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.starts_with(')') || peek.is_empty() {
            break;
        }
        ctx.parse_trivia();
        if ctx.remaining().starts_with(',') {
            let comma_pos = ctx.pos();
            ctx.advance(1);
            // Peek again after comma - trailing comma if closer follows
            let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
            if peek.starts_with(')') || peek.is_empty() {
                trailing_comma_pos = Some(comma_pos);
                ctx.parse_trivia();
                break;
            }
            ctx.check_comma_spacing();
            ctx.parse_trivia();
        } else if !ctx.remaining().starts_with(')') && !ctx.remaining().is_empty() {
            ctx.add_parse_error(ctx.error_here("Expected ',' or ')' after argument".to_string()));
            break;
        }
    }

    (args, trailing_comma_pos)
}

/// Parse a primary (atomic) expression
/// After parsing an identifier, check for `::` to produce ModuleAccess or plain Ident.
fn parse_ident_or_module_access(ctx: &mut ParseContext, start: usize, ident: Ident) -> Expr {
    if ctx.remaining().starts_with("::") {
        ctx.advance(2);
        match try_parse_ident(ctx) {
            Ok(Some(name)) => Expr::ModuleAccess(ExprModuleAccess {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
                module: ident,
                name,
            }),
            Ok(None) => {
                ctx.add_parse_error(ctx.error_here("Expected identifier after '::'".to_string()));
                Expr::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                })
            }
            Err(err) => {
                ctx.add_parse_error(err);
                Expr::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                })
            }
        }
    } else {
        Expr::Ident(ident)
    }
}

fn parse_primary(ctx: &mut ParseContext) -> Option<Expr> {
    let start = ctx.pos();
    let first = ctx.remaining().chars().next()?;

    // Parenthesized expression
    if first == '(' {
        ctx.advance(1);
        ctx.indent(b'(');
        ctx.parse_trivia();
        let inner = match parse_expr(ctx) {
            Some(e) => e,
            None => {
                ctx.add_parse_error(ParseError::new(
                    (start, ctx.pos()),
                    "Expected expression after '('".to_string(),
                ));
                if let Some(close) = ctx.remaining().find(')') {
                    ctx.advance(close + 1);
                }
                ctx.dedent();
                return Some(Expr::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                }));
            }
        };
        ctx.parse_trivia();
        ctx.dedent();
        if ctx.remaining().starts_with(')') {
            ctx.advance(1);
        } else {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                "Expected ')'".to_string(),
            ));
        }
        return Some(Expr::Paren(ExprParen {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            expr: Box::new(inner),
        }));
    }

    // Number literal
    if first.is_ascii_digit() {
        return Some(parse_number(ctx));
    }

    // Quoted string
    if first == '"' {
        return Some(parse_string_quoted(ctx));
    }

    // Raw string (two+ backticks)
    if first == '`' && ctx.remaining().as_bytes().get(1) == Some(&b'`') {
        return Some(parse_string_raw(ctx));
    }

    // Array literal: `[expr, expr, ...]`
    if first == '[' {
        return Some(parse_array_literal(ctx));
    }

    // Map literal: `{ key = value, ... }`
    if first == '{' {
        return Some(parse_map_literal(ctx));
    }

    // Identifier or keyword (true, false, nil, outer)
    if first.is_alphabetic() || first == '_' {
        // Peek at the word to detect keywords
        let word_len = ctx
            .remaining()
            .find(|c: char| !c.is_alphanumeric() && c != '_')
            .unwrap_or(ctx.remaining().len());
        let word = &ctx.remaining()[..word_len];

        return match word {
            "true" => {
                ctx.advance(4);
                Some(Expr::Literal(ExprLiteral {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    kind: ExprLiteralKind::Bool(true),
                }))
            }
            "false" => {
                ctx.advance(5);
                Some(Expr::Literal(ExprLiteral {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    kind: ExprLiteralKind::Bool(false),
                }))
            }
            "nil" => {
                ctx.advance(3);
                Some(Expr::Literal(ExprLiteral {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    kind: ExprLiteralKind::Nil,
                }))
            }
            "outer" => {
                ctx.advance(5);
                let mut depth: usize = 1;
                while ctx.remaining().starts_with(".outer")
                    && !ctx.remaining()[6..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
                {
                    ctx.advance(6);
                    depth += 1;
                }
                Some(Expr::Outer(ExprOuter {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    depth,
                }))
            }
            "wait" => {
                ctx.advance(4);
                if !ctx.remaining().starts_with('(') {
                    ctx.add_parse_error(ctx.error_here("Expected '(' after 'wait'".to_string()));
                    return Some(Expr::Invalid(InvalidNode {
                        node_id: ctx.alloc_node_id(start, ctx.pos()),
                    }));
                }
                ctx.advance(1);
                ctx.parse_trivia();
                let condition = match parse_expr(ctx) {
                    Some(e) => e,
                    None => {
                        ctx.add_parse_error(ParseError::new(
                            (start, ctx.pos()),
                            "Expected expression in 'wait'".to_string(),
                        ));
                        Expr::Invalid(InvalidNode {
                            node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
                        })
                    }
                };
                ctx.parse_trivia();
                if ctx.remaining().starts_with(')') {
                    ctx.advance(1);
                } else {
                    ctx.add_parse_error(ParseError::new(
                        (start, ctx.pos()),
                        "Expected ')' after wait condition".to_string(),
                    ));
                }
                Some(Expr::Wait(ExprWait {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    condition: Box::new(condition),
                }))
            }
            "spawn" => {
                ctx.advance(5);
                if !ctx.remaining().starts_with('(') {
                    ctx.add_parse_error(ctx.error_here("Expected '(' after 'spawn'".to_string()));
                    return Some(Expr::Invalid(InvalidNode {
                        node_id: ctx.alloc_node_id(start, ctx.pos()),
                    }));
                }
                let opener_pos = ctx.pos();
                ctx.advance(1); // consume '('
                ctx.indent(b'(');
                ctx.parse_trivia();

                // First arg: the entity invocation expression
                let invocation = match parse_expr(ctx) {
                    Some(e) => e,
                    None => {
                        ctx.add_parse_error(ParseError::new(
                            (start, ctx.pos()),
                            "Expected invocation expression in 'spawn'".to_string(),
                        ));
                        Expr::Invalid(InvalidNode {
                            node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
                        })
                    }
                };

                if !matches!(&invocation, Expr::Invocation(_) | Expr::Invalid(_)) {
                    ctx.add_parse_error(ParseError::new(
                        (opener_pos + 1, ctx.pos()),
                        "First argument to 'spawn' must be an invocation".to_string(),
                    ));
                }
                ctx.parse_trivia();

                // Optional named args: only `id` is valid
                let mut spawn_id: Option<Box<Expr>> = None;
                if ctx.remaining().starts_with(',') {
                    ctx.advance(1); // consume ','
                    ctx.check_comma_spacing();
                    ctx.parse_trivia();
                    let (args, _trailing) = parse_invocation_args(ctx, opener_pos);
                    let id_sym = Symbol::id();
                    for arg in &args {
                        if arg.name.name == id_sym {
                            spawn_id = Some(Box::new(arg.value.clone()));
                        } else {
                            let span = ctx.node_span(arg.node_id);
                            ctx.add_parse_error(ParseError::new(
                                span,
                                format!(
                                    "Unknown spawn argument '{}', expected 'id'",
                                    ctx.ident_str(&arg.name)
                                ),
                            ));
                        }
                    }
                }

                ctx.parse_trivia();
                ctx.dedent();
                if ctx.remaining().starts_with(')') {
                    ctx.advance(1);
                } else {
                    ctx.add_parse_error(ParseError::new(
                        (start, ctx.pos()),
                        "Expected ')' after spawn arguments".to_string(),
                    ));
                }

                Some(Expr::Spawn(ExprSpawn {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    invocation: Box::new(invocation),
                    id: spawn_id,
                }))
            }
            "data" => {
                ctx.advance(4);
                let data = parse_data(ctx, start, true);
                for field in &data.fields {
                    if let FieldKind::Var(var) = &field.kind
                        && var.default.is_none()
                    {
                        let span = ctx.node_span(var.node_id);
                        ctx.add_parse_error(ParseError::new(
                            span,
                            "Anonymous data field must have '= expression'".to_string(),
                        ));
                    }
                }
                Some(Expr::AnonData(data))
            }
            "func" => {
                ctx.advance(4);
                Some(Expr::AnonFunc(parse_func(ctx, start, false, false)))
            }
            "view" | "noblock" => {
                let after_word = &ctx.remaining()[word_len..];
                let after_space = after_word.trim_start_matches(' ');
                if starts_with_keyword(after_space, "func") {
                    let is_view = word_len == 4; // "view" = 4, "noblock" = 7
                    let modifier_name = if is_view { "view" } else { "noblock" };
                    ctx.advance(word_len);
                    let after_modifier = ctx.pos();
                    if ctx.parse_trivia() != " " {
                        ctx.add_strict_violation(ParseError::new(
                            (after_modifier, ctx.pos()),
                            format!("Expected exactly one space after '{}'", modifier_name),
                        ));
                    }
                    ctx.advance(4); // "func"
                    Some(Expr::AnonFunc(parse_func(ctx, start, is_view, !is_view)))
                } else {
                    match try_parse_ident(ctx) {
                        Ok(Some(ident)) => Some(parse_ident_or_module_access(ctx, start, ident)),
                        Ok(None) => None,
                        Err(err) => {
                            ctx.add_parse_error(err);
                            Some(Expr::Invalid(InvalidNode {
                                node_id: ctx.alloc_node_id(start, ctx.pos()),
                            }))
                        }
                    }
                }
            }
            _ => match try_parse_ident(ctx) {
                Ok(Some(ident)) => Some(parse_ident_or_module_access(ctx, start, ident)),
                Ok(None) => None,
                Err(err) => {
                    ctx.add_parse_error(err);
                    Some(Expr::Invalid(InvalidNode {
                        node_id: ctx.alloc_node_id(start, ctx.pos()),
                    }))
                }
            },
        };
    }

    None
}

/// Parse int or float literal
fn parse_number(ctx: &mut ParseContext) -> Expr {
    let start = ctx.pos();
    let remaining = ctx.remaining();
    let mut len = 0;
    let mut is_float = false;

    for (i, ch) in remaining.char_indices() {
        if ch.is_ascii_digit() || ch == '_' {
            len = i + ch.len_utf8();
        } else if ch == '.' && !is_float {
            // Dot followed by digit = float, otherwise stop (access op)
            if remaining[i + 1..].starts_with(|c: char| c.is_ascii_digit()) {
                is_float = true;
                len = i + 1;
            } else {
                break;
            }
        } else {
            break;
        }
    }

    let text = remaining[..len].to_string();
    ctx.advance(len);

    // Strict: validate underscore formatting (Spec 10.7)
    validate_number_underscores(ctx, &text, start);

    let kind = if is_float {
        ExprLiteralKind::Float(text)
    } else {
        ExprLiteralKind::Int(text)
    };
    Expr::Literal(ExprLiteral {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind,
    })
}

/// Format digits with underscores (only called on error path).
fn format_with_underscores(digits: &str, from_right: bool) -> String {
    let len = digits.len();
    if len < 4 {
        return digits.to_string();
    }
    let mut result = String::with_capacity(len + len / 3);
    for (i, ch) in digits.char_indices() {
        if i > 0 {
            let sep = if from_right {
                (len - i).is_multiple_of(3)
            } else {
                i % 3 == 0
            };
            if sep {
                result.push('_');
            }
        }
        result.push(ch);
    }
    result
}

/// Validate number underscore formatting (Spec 10.7).
fn validate_number_underscores(ctx: &mut ParseContext, text: &str, start: usize) {
    let (int_text, frac_text) = match text.find('.') {
        Some(dot) => (&text[..dot], Some(&text[dot + 1..])),
        None => (text, None),
    };

    // Check integer part: step 4 bytes from right (3 digits + underscore)
    let int_bytes = int_text.as_bytes();
    let int_ok = if int_bytes.len() < 4 {
        !int_bytes.contains(&b'_')
    } else {
        let ok;
        let mut i = int_bytes.len();
        loop {
            if i < 4 {
                ok = i > 0 && !int_bytes[..i].contains(&b'_');
                break;
            }
            if int_bytes[i - 4] != b'_' {
                ok = false;
                break;
            }
            i -= 4;
        }
        ok
    };

    // Check fractional part: step 4 bytes from left (3 digits + underscore)
    let frac_ok = frac_text.is_none_or(|f| {
        let fb = f.as_bytes();
        if fb.len() < 4 {
            !fb.contains(&b'_')
        } else {
            let ok;
            let mut i = 0;
            loop {
                if i + 4 > fb.len() {
                    ok = i < fb.len() && !fb[i..].contains(&b'_');
                    break;
                }
                if fb[i + 3] != b'_' {
                    ok = false;
                    break;
                }
                i += 4;
            }
            ok
        }
    });

    if !int_ok || !frac_ok {
        let int_digits: String = int_text.chars().filter(|c| *c != '_').collect();
        let expected_int = format_with_underscores(&int_digits, true);
        let expected = match frac_text {
            Some(f) => {
                let frac_digits: String = f.chars().filter(|c| *c != '_').collect();
                format!(
                    "{}.{}",
                    expected_int,
                    format_with_underscores(&frac_digits, false)
                )
            }
            None => expected_int,
        };
        ctx.add_strict_violation(ParseError::new(
            (start, start + text.len()),
            format!("Number should be formatted as {}", expected),
        ));
    }
}

/// Parse a hex escape sequence of exactly `count` hex digits.
/// Assumes the `\x`, `\u`, or `\U` prefix has already been consumed.
/// `esc_start` is the position of the `\` for error reporting.
fn parse_hex_escape(ctx: &mut ParseContext, esc_start: usize, count: usize, value: &mut String) {
    let hex_start = ctx.pos();
    let remaining = ctx.remaining();
    let available = remaining.len().min(count);
    let hex_str = &remaining[..available];
    let actual_hex_len = hex_str
        .find(|c: char| !c.is_ascii_hexdigit())
        .unwrap_or(available);

    if actual_hex_len < count {
        ctx.add_parse_error(ParseError::new(
            (esc_start, hex_start + actual_hex_len.max(1)),
            format!("Expected {} hex digits in escape sequence", count),
        ));
        ctx.advance(actual_hex_len);
        return;
    }

    let hex = &remaining[..count];
    let has_uppercase = hex.bytes().any(|b| b.is_ascii_uppercase());
    let code = u32::from_str_radix(hex, 16).unwrap();
    ctx.advance(count);
    if has_uppercase {
        ctx.add_strict_violation(ParseError::new(
            (hex_start, hex_start + count),
            "Hex digits in escape sequences must be lowercase".to_string(),
        ));
    }

    match char::from_u32(code) {
        Some(c) => value.push(c),
        None => {
            ctx.add_parse_error(ParseError::new(
                (esc_start, ctx.pos()),
                format!("Invalid Unicode code point: U+{:04X}", code),
            ));
        }
    }
}

/// Parse quoted string `"..."`
fn parse_string_quoted(ctx: &mut ParseContext) -> Expr {
    let start = ctx.pos();
    ctx.advance(1); // opening "
    let mut value = String::new();
    let mut closed = false;

    loop {
        if ctx.remaining().is_empty() {
            break;
        }
        let ch = ctx.remaining().chars().next().unwrap();
        match ch {
            '"' => {
                ctx.advance(1);
                closed = true;
                break;
            }
            '\\' => {
                let esc_start = ctx.pos();
                ctx.advance(1);
                if let Some(esc) = ctx.remaining().chars().next() {
                    match esc {
                        'n' => {
                            value.push('\n');
                            ctx.advance(1);
                        }
                        'r' => {
                            value.push('\r');
                            ctx.advance(1);
                        }
                        't' => {
                            value.push('\t');
                            ctx.advance(1);
                        }
                        '\\' => {
                            value.push('\\');
                            ctx.advance(1);
                        }
                        '"' => {
                            value.push('"');
                            ctx.advance(1);
                        }
                        '0' => {
                            value.push('\0');
                            ctx.advance(1);
                        }
                        'x' => {
                            ctx.advance(1);
                            parse_hex_escape(ctx, esc_start, 2, &mut value);
                        }
                        'u' => {
                            ctx.advance(1);
                            parse_hex_escape(ctx, esc_start, 4, &mut value);
                        }
                        'U' => {
                            ctx.advance(1);
                            parse_hex_escape(ctx, esc_start, 8, &mut value);
                        }
                        _ => {
                            ctx.add_parse_error(ParseError::new(
                                (esc_start, ctx.pos() + esc.len_utf8()),
                                format!("Unknown escape sequence '\\{}'", esc),
                            ));
                            value.push(esc);
                            ctx.advance(esc.len_utf8());
                        }
                    }
                } else {
                    break;
                }
            }
            '\n' | '\r' => break,
            _ => {
                value.push(ch);
                ctx.advance(ch.len_utf8());
            }
        }
    }

    if !closed {
        ctx.add_parse_error(ParseError::with_range(
            (start, ctx.pos()),
            "Unterminated string literal".to_string(),
            (start, start + 1), // point at opening quote
        ));
    }

    Expr::Literal(ExprLiteral {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind: ExprLiteralKind::Str(value),
    })
}

/// Parse raw string literal: N backticks (N >= 2) open and close, no escaping.
/// Cursor must be at the first backtick.
fn parse_string_raw(ctx: &mut ParseContext) -> Expr {
    let start = ctx.pos();

    // Count opening backticks and find closing delimiter from remaining
    let remaining = ctx.remaining();
    let mut n = 0;
    while remaining.as_bytes().get(n) == Some(&b'`') {
        n += 1;
    }
    let closing = &remaining[..n];
    let after_open = &remaining[n..];

    let value;
    if let Some(end) = after_open.find(closing) {
        value = after_open[..end].to_string();
        ctx.advance(n + end + n); // opening + content + closing
        ctx.sync_line_start(); // raw strings can span lines
    } else {
        // Error recovery: consume only to end of line
        let eol = after_open.find('\n').unwrap_or(after_open.len());
        value = after_open[..eol].to_string();
        ctx.advance(n + eol);
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Unterminated raw string literal".to_string(),
        ));
    }

    Expr::Literal(ExprLiteral {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind: ExprLiteralKind::Str(value),
    })
}

/// Parse array literal: `[expr, expr, ...]` or `[]`
fn parse_array_literal(ctx: &mut ParseContext) -> Expr {
    let start = ctx.pos();
    ctx.advance(1); // "["
    ctx.indent(b'[');
    // Strict: no space after '['
    if ctx.remaining().starts_with(' ') {
        ctx.add_strict_violation(ParseError::new(
            (ctx.pos(), ctx.pos() + 1),
            "No space allowed after '['".to_string(),
        ));
    }
    ctx.parse_trivia();

    // Empty array
    if ctx.remaining().starts_with(']') {
        ctx.dedent();
        ctx.advance(1);
        return Expr::Literal(ExprLiteral {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            kind: ExprLiteralKind::Array(Vec::new()),
        });
    }

    let mut elements = Vec::new();
    let mut trailing_comma_pos: Option<usize> = None;
    let mut prev_item_start = ctx.pos();

    // First element
    match parse_expr(ctx) {
        Some(expr) => elements.push(expr),
        None => {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                "Expected expression or ']' in array literal".to_string(),
            ));
        }
    }

    // Subsequent elements: stay on same line looking for `,` or `]`.
    // Only cross lines after a `,` (comma signals continuation).
    let mut had_space_before_close = false;
    loop {
        let before = ctx.pos();
        ctx.skip_spaces();
        if ctx.remaining().starts_with(']') || ctx.remaining().is_empty() {
            had_space_before_close = ctx.pos() > before;
            break;
        }
        if !ctx.remaining().starts_with(',') {
            break;
        }
        let comma_pos = ctx.pos();
        ctx.advance(1); // ","
        // Peek: trailing comma if closer follows
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.starts_with(']') || peek.is_empty() {
            trailing_comma_pos = Some(comma_pos);
            ctx.parse_trivia();
            break;
        }
        ctx.check_comma_spacing();
        ctx.parse_trivia();
        let item_start = ctx.pos();
        match parse_expr(ctx) {
            Some(expr) => {
                if !ctx.is_multiline(prev_item_start, item_start)
                    && ctx.is_multiline(start, item_start)
                {
                    ctx.add_strict_violation(ParseError::new(
                        (item_start, item_start + 1),
                        "Only one item per line allowed in multi-line form".to_string(),
                    ));
                }
                prev_item_start = item_start;
                elements.push(expr);
            }
            None => {
                ctx.add_parse_error(
                    ctx.error_here("Expected expression after ',' in array literal".to_string()),
                );
                break;
            }
        }
    }

    ctx.parse_trivia();
    ctx.dedent();
    if ctx.remaining().starts_with(']') {
        let multiline = ctx.is_multiline(start, ctx.pos());
        if had_space_before_close {
            ctx.add_strict_violation(ParseError::new(
                (ctx.pos() - 1, ctx.pos()),
                "No space allowed before ']'".to_string(),
            ));
        }
        if multiline && trailing_comma_pos.is_none() && !elements.is_empty() {
            ctx.add_strict_violation(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Trailing comma required in multi-line form".to_string(),
            ));
        }
        if !multiline && let Some(cp) = trailing_comma_pos {
            ctx.add_strict_violation(ParseError::new(
                (cp, cp + 1),
                "Trailing comma not allowed in single-line form".to_string(),
            ));
        }
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ParseError::with_range(
            (start, ctx.pos()),
            "Expected ']'".to_string(),
            (start, start + 1), // point at opening '['
        ));
    }

    Expr::Literal(ExprLiteral {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind: ExprLiteralKind::Array(elements),
    })
}

/// Parse map literal: `{ key = value, ... }` or `{}`
fn parse_map_literal(ctx: &mut ParseContext) -> Expr {
    let start = ctx.pos();
    ctx.advance(1); // "{"
    ctx.indent(b'{');
    // Strict: space or newline required after '{'
    if !ctx.remaining().starts_with(' ')
        && !ctx.remaining().starts_with('\n')
        && !ctx.remaining().starts_with("\r\n")
    {
        ctx.add_strict_violation(
            ctx.spacing_error(ctx.pos(), "Expected space or newline after '{'".to_string()),
        );
    }
    ctx.parse_trivia();

    // Empty map
    if ctx.remaining().starts_with('}') {
        ctx.dedent();
        ctx.advance(1);
        return Expr::Literal(ExprLiteral {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            kind: ExprLiteralKind::Map(Vec::new()),
        });
    }

    let mut entries = Vec::new();
    let mut trailing_comma_pos: Option<usize> = None;
    let mut prev_item_start = ctx.pos();

    // First entry
    if let Some(entry) = parse_map_entry(ctx) {
        entries.push(entry);
    }

    // Subsequent entries: stay on same line looking for `,` or `}`.
    // Only cross lines after a `,` (comma signals continuation).
    loop {
        ctx.skip_spaces();
        if ctx.remaining().starts_with('}') || ctx.remaining().is_empty() {
            break;
        }
        if !ctx.remaining().starts_with(',') {
            break;
        }
        let comma_pos = ctx.pos();
        ctx.advance(1); // ","
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.starts_with('}') || peek.is_empty() {
            trailing_comma_pos = Some(comma_pos);
            ctx.parse_trivia();
            break;
        }
        ctx.check_comma_spacing();
        ctx.parse_trivia();
        let item_start = ctx.pos();
        if let Some(entry) = parse_map_entry(ctx) {
            if !ctx.is_multiline(prev_item_start, item_start) && ctx.is_multiline(start, item_start)
            {
                ctx.add_strict_violation(ParseError::new(
                    (item_start, item_start + 1),
                    "Only one item per line allowed in multi-line form".to_string(),
                ));
            }
            prev_item_start = item_start;
            entries.push(entry);
        } else {
            break;
        }
    }

    ctx.parse_trivia();
    ctx.dedent();
    if ctx.remaining().starts_with('}') {
        let multiline = ctx.is_multiline(start, ctx.pos());
        let prev = ctx.prev_byte();
        if prev != Some(b' ') && prev != Some(b'\n') {
            ctx.add_strict_violation(ctx.spacing_error(
                ctx.pos(),
                "Expected space or newline before '}'".to_string(),
            ));
        }
        if multiline && trailing_comma_pos.is_none() && !entries.is_empty() {
            ctx.add_strict_violation(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Trailing comma required in multi-line form".to_string(),
            ));
        }
        if !multiline && let Some(cp) = trailing_comma_pos {
            ctx.add_strict_violation(ParseError::new(
                (cp, cp + 1),
                "Trailing comma not allowed in single-line form".to_string(),
            ));
        }
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ParseError::with_range(
            (start, ctx.pos()),
            "Expected '}'".to_string(),
            (start, start + 1), // point at opening '{'
        ));
    }

    Expr::Literal(ExprLiteral {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind: ExprLiteralKind::Map(entries),
    })
}

/// Parse a single map entry: `key = value`
fn parse_map_entry(ctx: &mut ParseContext) -> Option<ExprLiteralMapEntry> {
    let entry_start = ctx.pos();
    let key = parse_expr(ctx)?;
    let after_key = ctx.pos();

    ctx.skip_spaces();
    if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
        ctx.advance(1); // "="
        ctx.skip_spaces();
    } else {
        ctx.add_parse_error(ParseError::new(
            (entry_start, after_key),
            "Expected '=' after map key".to_string(),
        ));
        return Some(ExprLiteralMapEntry {
            node_id: ctx.alloc_node_id(entry_start, ctx.pos()),
            key,
            value: Expr::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
            }),
        });
    }

    let value = match parse_expr(ctx) {
        Some(e) => e,
        None => {
            ctx.add_parse_error(ParseError::new(
                (entry_start, ctx.pos()),
                "Expected expression after '=' in map entry".to_string(),
            ));
            Expr::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
            })
        }
    };

    Some(ExprLiteralMapEntry {
        node_id: ctx.alloc_node_id(entry_start, ctx.pos()),
        key,
        value,
    })
}
