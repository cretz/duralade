use crate::model::*;

use super::basic::try_parse_ident;
use super::construct::starts_with_keyword;
use super::expr::parse_expr;
use super::types::parse_type;
use super::{ParseContext, ParseError};

/// Parse an optional `:label` suffix (used by `for`, `break`, `continue`).
/// Must appear immediately after the keyword with no space before `:`.
fn parse_label(ctx: &mut ParseContext) -> Option<Ident> {
    let before = ctx.pos();

    // Detect space before colon: `break :label` → strict violation, still parse it
    let spaces = ctx.skip_spaces();
    if !ctx.remaining().starts_with(':') {
        ctx.set_pos(before);
        return None;
    }
    if spaces > 0 {
        ctx.add_strict_violation(ParseError::new(
            (before, ctx.pos() + 1),
            "No space allowed before ':' in label".to_string(),
        ));
    }

    let colon_pos = ctx.pos();
    ctx.advance(1); // ":"

    // Detect space after colon: `break: label` → strict violation (only if label follows)
    let spaces_after = ctx.skip_spaces();

    match try_parse_ident(ctx) {
        Ok(Some(ident)) => {
            if spaces_after > 0 {
                ctx.add_strict_violation(ParseError::new(
                    (colon_pos, colon_pos + 1 + spaces_after),
                    "No space allowed after ':' in label".to_string(),
                ));
            }
            Some(ident)
        }
        Ok(None) => {
            ctx.add_parse_error(ParseError::new(
                (colon_pos, colon_pos + 1),
                "Expected label name after ':'".to_string(),
            ));
            None
        }
        Err(err) => {
            ctx.add_parse_error(err);
            None
        }
    }
}

/// Parse a statement. Returns None if no statement found at current position.
/// Handles: var, :=, return, return!, expression statements, assignments.
pub(crate) fn parse_stmt(ctx: &mut ParseContext) -> Option<Stmt> {
    let start = ctx.pos();
    let remaining = ctx.remaining();

    // `var` declaration
    if remaining.starts_with("var") && remaining[3..].starts_with([' ', '\n', '\r']) {
        return Some(parse_var(ctx));
    }

    // `return!` (must check before `return` due to prefix)
    if remaining.starts_with("return!")
        && !remaining[7..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    {
        if ctx.defer_depth > 0 {
            ctx.add_parse_error(ParseError::new(
                (start, start + 7),
                "Early return 'return!' is not allowed inside defer blocks".to_string(),
            ));
        }
        ctx.advance(7); // "return!"
        ctx.skip_spaces();
        let expr = match parse_expr(ctx) {
            Some(e) => e,
            None => {
                ctx.add_parse_error(ParseError::new(
                    (start, ctx.pos()),
                    "Expected expression after 'return!'".to_string(),
                ));
                return Some(Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                }));
            }
        };
        return Some(Stmt::ReturnEarly(StmtReturnEarly {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            expr,
        }));
    }

    // `return`
    if remaining.starts_with("return")
        && !remaining[6..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    {
        ctx.advance(6);
        return Some(Stmt::Return(StmtReturn {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
        }));
    }

    // `break` / `break:label`
    if starts_with_keyword(remaining, "break") {
        ctx.advance(5);
        let label = parse_label(ctx);
        return Some(Stmt::ForBreak(StmtForBreak {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            label,
        }));
    }

    // `continue` / `continue:label`
    if starts_with_keyword(remaining, "continue") {
        ctx.advance(8);
        let label = parse_label(ctx);
        return Some(Stmt::ForContinue(StmtForContinue {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            label,
        }));
    }

    // `if`
    if starts_with_keyword(remaining, "if") {
        return Some(parse_if(ctx));
    }

    // `for`
    if starts_with_keyword(remaining, "for") {
        return Some(parse_for(ctx));
    }

    // `defer`
    if starts_with_keyword(remaining, "defer") {
        return Some(parse_defer(ctx));
    }

    // `implicitly`
    if starts_with_keyword(remaining, "implicitly") {
        return Some(parse_implicitly(ctx));
    }

    // Patch: `% version {{ ... % }}`
    if remaining.starts_with('%') && !remaining.starts_with("%=") {
        return Some(parse_patch(ctx));
    }

    // Standalone block: `{ stmts }`
    if remaining.starts_with('{') {
        let block = parse_stmt_block(ctx);
        return Some(Stmt::Block(block));
    }

    // Try expression - then check for `:=` or assignment operators after it
    let expr = parse_expr(ctx)?;

    // Collect comma-separated LHS for potential multi-var (:= or assignment)
    let before_op = ctx.pos();
    ctx.skip_spaces();
    let mut lhs_exprs = vec![expr];
    let mut lhs_end = before_op; // position right after last LHS expression
    while ctx.remaining().starts_with(',') {
        ctx.advance(1); // ","
        ctx.skip_spaces();
        match parse_expr(ctx) {
            Some(e) => {
                lhs_exprs.push(e);
                lhs_end = ctx.pos();
                ctx.skip_spaces();
            }
            None => {
                ctx.add_parse_error(ParseError::new(
                    (ctx.pos() - 1, ctx.pos()),
                    "Expected expression after ','".to_string(),
                ));
                break;
            }
        }
    }

    // Check for `:=` (var_assignment shorthand)
    if ctx.remaining().starts_with(":=") {
        ctx.advance(2); // ":="
        ctx.skip_spaces();

        // All LHS must be identifiers
        let mut vars = Vec::new();
        let mut had_error = false;
        for lhs in &lhs_exprs {
            match lhs {
                Expr::Ident(ident) => vars.push(StmtVarDecl {
                    node_id: lhs.node_id(),
                    name: ident.clone(),
                    ty: None,
                }),
                _ => {
                    if !had_error {
                        ctx.add_parse_error(ParseError::new(
                            (start, lhs_end),
                            "Left side of ':=' must be an identifier".to_string(),
                        ));
                        had_error = true;
                    }
                }
            }
        }

        // Parse comma-separated RHS values
        let mut values = Vec::new();
        match parse_expr(ctx) {
            Some(e) => values.push(e),
            None => {
                ctx.add_parse_error(ParseError::new(
                    (start, ctx.pos()),
                    "Expected expression after ':='".to_string(),
                ));
                return Some(Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                }));
            }
        }
        while ctx.remaining().starts_with(',') {
            ctx.advance(1); // ","
            ctx.skip_spaces();
            match parse_expr(ctx) {
                Some(e) => values.push(e),
                None => {
                    ctx.add_parse_error(ParseError::new(
                        (ctx.pos() - 1, ctx.pos()),
                        "Expected expression after ','".to_string(),
                    ));
                    break;
                }
            }
        }

        if had_error {
            return Some(Stmt::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
            }));
        }

        if vars.len() != values.len() {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                format!(
                    "Expected {} values after ':=', found {}",
                    vars.len(),
                    values.len()
                ),
            ));
        }

        return Some(Stmt::Var(StmtVar {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            vars,
            values,
        }));
    }

    // Check for assignment operators: =, +=, -=, *=, /=, %=
    let assign_op = if ctx.remaining().starts_with("+=") {
        Some((StmtAssignOp::AddAssign, 2))
    } else if ctx.remaining().starts_with("-=") {
        Some((StmtAssignOp::SubAssign, 2))
    } else if ctx.remaining().starts_with("*=") {
        Some((StmtAssignOp::MulAssign, 2))
    } else if ctx.remaining().starts_with("/=") {
        Some((StmtAssignOp::DivAssign, 2))
    } else if ctx.remaining().starts_with("%=") {
        Some((StmtAssignOp::ModAssign, 2))
    } else if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
        Some((StmtAssignOp::Assign, 1))
    } else {
        None
    };

    if let Some((op, op_len)) = assign_op {
        ctx.advance(op_len);
        ctx.skip_spaces();

        // Parse comma-separated RHS values
        let mut values = Vec::new();
        match parse_expr(ctx) {
            Some(e) => values.push(e),
            None => {
                ctx.add_parse_error(ParseError::new(
                    (start, ctx.pos()),
                    "Expected expression after assignment operator".to_string(),
                ));
                return Some(Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                }));
            }
        }
        while ctx.remaining().starts_with(',') {
            ctx.advance(1); // ","
            ctx.skip_spaces();
            match parse_expr(ctx) {
                Some(e) => values.push(e),
                None => {
                    ctx.add_parse_error(ParseError::new(
                        (ctx.pos() - 1, ctx.pos()),
                        "Expected expression after ','".to_string(),
                    ));
                    break;
                }
            }
        }

        return Some(Stmt::Assign(StmtAssign {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            vars: lhs_exprs,
            op,
            values,
        }));
    }

    // No operator - if multi-expr, that's invalid
    if lhs_exprs.len() > 1 {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Expected ':=' or '=' after comma-separated expressions".to_string(),
        ));
        return Some(Stmt::Invalid(InvalidNode {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
        }));
    }

    // Expression statement
    ctx.set_pos(before_op);
    let expr = lhs_exprs.into_iter().next().unwrap();
    match &expr {
        Expr::Paren(_) => {
            ctx.add_parse_error(ParseError::new(
                (start, before_op),
                "Parenthesized expression cannot be a standalone statement".to_string(),
            ));
        }
        Expr::Unary(u) if u.op != ExprUnaryOp::EarlyReturn => {
            ctx.add_parse_error(ParseError::new(
                (start, before_op),
                "Prefix unary expression cannot be a standalone statement".to_string(),
            ));
        }
        _ => {}
    }
    Some(Stmt::Expr(expr))
}

/// Parse a statement block: `{ stmts }`.
/// Assumes caller has NOT consumed the `{`.
pub(crate) fn parse_stmt_block(ctx: &mut ParseContext) -> StmtBlock {
    let start = ctx.pos();
    if ctx.remaining().starts_with('{') {
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Expected '{'".to_string(),
        ));
        return StmtBlock {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            stmts: Vec::new(),
        };
    }

    ctx.indent(b'{');
    let mut stmts = Vec::new();
    loop {
        // Peek for `}` at outer indent level before parse_trivia to avoid
        // false indentation violations on the closing brace
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.is_empty() || peek.starts_with('}') {
            break;
        }
        let pos_before = ctx.pos();
        ctx.parse_trivia();
        if ctx.remaining().starts_with('}') || ctx.remaining().is_empty() {
            break;
        }
        if let Some(stmt) = parse_stmt(ctx) {
            ctx.check_stmt_termination();
            stmts.push(stmt);
        } else {
            ctx.add_parse_error(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Unrecognized content in block".to_string(),
            ));
            ctx.skip_past_end_of_line();
            if ctx.pos() == pos_before {
                break;
            }
        }
    }
    // Consume trivia, then dedent so closer gets correct indent check
    ctx.parse_trivia();
    ctx.dedent();
    if ctx.remaining().starts_with('}') {
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Expected '}'".to_string(),
        ));
    }

    StmtBlock {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        stmts,
    }
}

/// Parse an if condition: bool, narrowing-as, or narrowing-nil.
/// Returns true if remaining starts with `?` used as a narrowing-nil marker
/// (not `?.`, `??`, or `?!` which are other operators).
fn is_nil_narrowing_marker(remaining: &str) -> bool {
    remaining.starts_with('?')
        && !remaining.starts_with("?.")
        && !remaining.starts_with("??")
        && !remaining.starts_with("?!")
}

/// Grouped syntax: `name[, name]* := expr (as type | ?)[, expr (as type | ?)]* `
/// Returns None if no condition could be parsed at all.
fn parse_if_condition(ctx: &mut ParseContext) -> Option<StmtIfCondition> {
    let cond_start = ctx.pos();

    // Speculatively try narrowing: `names := exprs`
    // The one snapshot here is unavoidable - if it's not narrowing we must
    // re-parse as a bool condition.
    let snap = ctx.snapshot();
    let mut names: Vec<Ident> = Vec::new();
    if let Ok(Some(ident)) = try_parse_ident(ctx) {
        names.push(ident);

        // Collect more comma-separated names (outer snap covers us on bail-out)
        loop {
            ctx.skip_spaces();
            if !ctx.remaining().starts_with(',') {
                break;
            }
            ctx.advance(1); // ","
            ctx.skip_spaces();
            match try_parse_ident(ctx) {
                Ok(Some(id)) => names.push(id),
                _ => break,
            }
        }

        ctx.skip_spaces();
        if ctx.remaining().starts_with(":=") {
            ctx.advance(2); // ":="
            ctx.skip_spaces();

            // Committed to narrowing - suppress bare `as`/`?` in expr parser
            ctx.suppress_narrowing_postfix = true;
            if let Some(first_expr) = parse_expr(ctx) {
                ctx.skip_spaces();

                let result = if starts_with_keyword(ctx.remaining(), "as") {
                    Some(parse_narrowing_as(ctx, cond_start, names, first_expr))
                } else if is_nil_narrowing_marker(ctx.remaining()) {
                    Some(parse_narrowing_nil(ctx, cond_start, names, first_expr))
                } else {
                    None
                };

                if let Some(cond) = result {
                    ctx.suppress_narrowing_postfix = false;
                    return Some(cond);
                }
            }
            // Had `names :=` but no `as`/`?` - not a narrowing form.
        }
    }
    ctx.suppress_narrowing_postfix = false;
    ctx.restore(snap);

    // Bool condition form: [ init ";" ] expression
    // Speculatively try: parse a var/assignment statement followed by ";".
    let mut init: Option<Box<Stmt>> = None;
    let init_snap = ctx.snapshot();
    if let Some(init_stmt) = parse_stmt(ctx) {
        ctx.skip_spaces();
        if ctx.remaining().starts_with(';') {
            if !matches!(&init_stmt, Stmt::Var(_) | Stmt::Assign(_)) {
                ctx.add_parse_error(ParseError::new(
                    (cond_start, ctx.pos()),
                    "If init must be a variable declaration or assignment".to_string(),
                ));
            } else {
                init = Some(Box::new(init_stmt));
            }
            ctx.advance(1); // ";"
            ctx.skip_spaces();
        } else {
            ctx.restore(init_snap);
        }
    } else {
        ctx.restore(init_snap);
    }

    // `{` here is the block, not a map literal - condition is missing
    if ctx.remaining().starts_with('{') {
        return None;
    }

    // Parse condition expression
    parse_expr(ctx).map(|expr| {
        StmtIfCondition::Bool(StmtIfConditionBool {
            node_id: ctx.alloc_node_id(cond_start, ctx.pos()),
            init,
            expr,
        })
    })
}

/// Helper: parse remaining `as type[, expr as type]*` after first expr is already parsed.
/// Assumes `ctx` is positioned at the first `as` keyword.
fn parse_narrowing_as(
    ctx: &mut ParseContext,
    cond_start: usize,
    names: Vec<Ident>,
    first_expr: Expr,
) -> StmtIfCondition {
    ctx.advance(2); // "as"
    ctx.skip_spaces();
    let ty = parse_type(ctx);
    let mut exprs = vec![(first_expr, ty)];

    while ctx.remaining().starts_with(',') {
        ctx.advance(1);
        ctx.skip_spaces();
        let e_start = ctx.pos();
        // Guard: `{` after comma is the if-body, not another expression
        if ctx.remaining().starts_with('{')
            || ctx.remaining().starts_with('\n')
            || ctx.remaining().starts_with('\r')
        {
            ctx.add_parse_error(ParseError::new(
                (e_start, e_start + 1),
                "Expected expression in narrowing binding".to_string(),
            ));
            break;
        }
        let e = match parse_expr(ctx) {
            Some(e) => e,
            None => {
                ctx.add_parse_error(ParseError::new(
                    (e_start, ctx.pos().max(e_start + 1)),
                    "Expected expression in narrowing binding".to_string(),
                ));
                break;
            }
        };
        ctx.skip_spaces();
        if !starts_with_keyword(ctx.remaining(), "as") {
            ctx.add_parse_error(ParseError::new(
                (e_start, ctx.pos().max(e_start + 1)),
                "Expected 'as' after expression in narrowing binding".to_string(),
            ));
            break;
        }
        ctx.advance(2); // "as"
        ctx.skip_spaces();
        exprs.push((e, parse_type(ctx)));
    }

    if names.len() != exprs.len() {
        ctx.add_parse_error(ParseError::new(
            (cond_start, ctx.pos()),
            format!(
                "Expected {} expressions after ':=', found {}",
                names.len(),
                exprs.len()
            ),
        ));
    }

    let bindings = names
        .into_iter()
        .zip(exprs)
        .map(|(name, (expr, ty))| StmtIfNarrowingAsBinding {
            node_id: ctx.alloc_node_id(cond_start, ctx.pos()),
            name,
            expr,
            ty,
        })
        .collect();

    StmtIfCondition::NarrowingAs(StmtIfNarrowingAs {
        node_id: ctx.alloc_node_id(cond_start, ctx.pos()),
        bindings,
    })
}

/// Helper: parse remaining `?[, expr?]*` after first expr is already parsed.
/// Assumes `ctx` is positioned at the first `?`.
fn parse_narrowing_nil(
    ctx: &mut ParseContext,
    cond_start: usize,
    names: Vec<Ident>,
    first_expr: Expr,
) -> StmtIfCondition {
    ctx.advance(1); // "?"
    let mut exprs = vec![first_expr];

    while ctx.remaining().starts_with(',') {
        ctx.advance(1);
        ctx.skip_spaces();
        let e_start = ctx.pos();
        if ctx.remaining().starts_with('{')
            || ctx.remaining().starts_with('\n')
            || ctx.remaining().starts_with('\r')
        {
            ctx.add_parse_error(ParseError::new(
                (e_start, e_start + 1),
                "Expected expression in narrowing binding".to_string(),
            ));
            break;
        }
        let e = match parse_expr(ctx) {
            Some(e) => e,
            None => {
                ctx.add_parse_error(ParseError::new(
                    (e_start, ctx.pos().max(e_start + 1)),
                    "Expected expression in narrowing binding".to_string(),
                ));
                break;
            }
        };
        ctx.skip_spaces();
        if !is_nil_narrowing_marker(ctx.remaining()) {
            ctx.add_parse_error(ParseError::new(
                (e_start, ctx.pos().max(e_start + 1)),
                "Expected '?' after expression in narrowing binding".to_string(),
            ));
            break;
        }
        ctx.advance(1); // "?"
        exprs.push(e);
    }

    if names.len() != exprs.len() {
        ctx.add_parse_error(ParseError::new(
            (cond_start, ctx.pos()),
            format!(
                "Expected {} expressions after ':=', found {}",
                names.len(),
                exprs.len()
            ),
        ));
    }

    let bindings = names
        .into_iter()
        .zip(exprs)
        .map(|(name, expr)| StmtIfNarrowingNilBinding {
            node_id: ctx.alloc_node_id(cond_start, ctx.pos()),
            name,
            expr,
        })
        .collect();

    StmtIfCondition::NarrowingNil(StmtIfNarrowingNil {
        node_id: ctx.alloc_node_id(cond_start, ctx.pos()),
        bindings,
    })
}

/// Parse `if` statement: `if condition { stmts } [else if condition { stmts }]* [else { stmts }]`
/// Condition can be bool, narrowing-as (`x := expr as type`), or narrowing-nil (`x := expr?`).
fn parse_if(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();
    ctx.advance(2); // "if"
    ctx.skip_spaces();

    let condition = match parse_if_condition(ctx) {
        Some(c) => c,
        None => {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos().max(start + 1)),
                "Expected condition after 'if'".to_string(),
            ));
            // If `{` is ahead, still parse the block for recovery
            ctx.skip_spaces();
            if !ctx.remaining().starts_with('{') {
                ctx.skip_past_end_of_line();
                return Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                });
            }
            StmtIfCondition::Bool(StmtIfConditionBool {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
                init: None,
                expr: Expr::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
                }),
            })
        }
    };

    ctx.skip_spaces();
    let then_block = parse_stmt_block(ctx);

    // Parse else if / else chains
    let mut else_ifs = Vec::new();
    let mut else_block = None;

    loop {
        let before_else = ctx.pos();
        ctx.skip_spaces();
        if !starts_with_keyword(ctx.remaining(), "else") {
            ctx.set_pos(before_else);
            break;
        }
        ctx.advance(4); // "else"
        ctx.skip_spaces();

        if starts_with_keyword(ctx.remaining(), "if") {
            // else if
            let ei_start = before_else;
            ctx.advance(2); // "if"
            ctx.skip_spaces();

            let ei_condition = match parse_if_condition(ctx) {
                Some(c) => c,
                None => {
                    ctx.add_parse_error(ParseError::new(
                        (ei_start, ctx.pos()),
                        "Expected condition after 'else if'".to_string(),
                    ));
                    break;
                }
            };

            ctx.skip_spaces();
            let ei_block = parse_stmt_block(ctx);

            else_ifs.push(StmtIfElseIf {
                node_id: ctx.alloc_node_id(ei_start, ctx.pos()),
                condition: ei_condition,
                then_block: ei_block,
            });
        } else {
            // else
            else_block = Some(parse_stmt_block(ctx));
            break;
        }
    }

    Stmt::If(StmtIf {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        condition,
        then_block,
        else_ifs,
        else_block,
    })
}

/// Parse `for` statement: `for[:label] { stmts }`, `for[:label] expr { stmts }`,
/// `for[:label] name in expr { stmts }`
fn parse_for(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();
    ctx.advance(3); // "for"
    let label = parse_label(ctx);
    ctx.skip_spaces();

    // Check for infinite loop: `for {`
    if ctx.remaining().starts_with('{') {
        let block = parse_stmt_block(ctx);
        return Stmt::For(StmtFor {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            label,
            clause: None,
            block,
        });
    }

    // Parse expression (could be condition or for-in variable)
    let expr_start = ctx.pos();
    let Some(expr) = parse_expr(ctx) else {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Expected expression or '{' after 'for'".to_string(),
        ));
        ctx.skip_past_end_of_line();
        return Stmt::Invalid(InvalidNode {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
        });
    };

    ctx.skip_spaces();

    // Check for `in` keyword → for-in loop
    if starts_with_keyword(ctx.remaining(), "in") {
        ctx.advance(2); // "in"
        ctx.skip_spaces();

        // The expression before `in` must be an identifier
        let name = match &expr {
            Expr::Ident(ident) => ident.clone(),
            _ => {
                ctx.add_parse_error(ParseError::new(
                    (expr_start, ctx.pos()),
                    "For-in variable must be an identifier".to_string(),
                ));
                Ident {
                    node_id: expr.node_id(),
                    name: Symbol::empty(),
                    is_raw: false,
                }
            }
        };

        let Some(iter_expr) = parse_expr(ctx) else {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                "Expected expression after 'in'".to_string(),
            ));
            ctx.skip_past_end_of_line();
            return Stmt::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
            });
        };

        ctx.skip_spaces();
        let block = parse_stmt_block(ctx);
        return Stmt::For(StmtFor {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            label,
            clause: Some(StmtForClause::In(StmtForIn {
                node_id: ctx.alloc_node_id(expr_start, ctx.pos()),
                var: name,
                expr: iter_expr,
            })),
            block,
        });
    }

    // Condition loop: `for expr { stmts }`
    let block = parse_stmt_block(ctx);
    Stmt::For(StmtFor {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        label,
        clause: Some(StmtForClause::Condition(StmtForCondition {
            node_id: ctx.alloc_node_id(expr_start, ctx.pos()),
            expr,
        })),
        block,
    })
}

/// Parse `defer` statement: `defer { stmts }`
fn parse_defer(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();
    ctx.advance(5); // "defer"
    let spaces = ctx.skip_spaces();

    if spaces > 1 {
        ctx.add_strict_violation(ParseError::new(
            (start + 5, ctx.pos()),
            "Expected exactly one space after 'defer'".to_string(),
        ));
    }

    ctx.defer_depth += 1;
    let block = parse_stmt_block(ctx);
    ctx.defer_depth -= 1;
    Stmt::Defer(StmtDefer {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        block,
    })
}

/// Parse `implicitly` statement:
/// Block form: `implicitly expr1, expr2 { stmts }`
/// Rest-of-block form: `implicitly expr1, expr2`
fn parse_implicitly(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();
    ctx.advance(10); // "implicitly"
    let spaces = ctx.skip_spaces();

    if spaces > 1 {
        ctx.add_strict_violation(ParseError::new(
            (start + 10, ctx.pos()),
            "Expected exactly one space after 'implicitly'".to_string(),
        ));
    }

    // Parse comma-separated expressions
    let mut exprs = Vec::new();
    match parse_expr(ctx) {
        Some(expr) => exprs.push(expr),
        None => {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                "Expected expression after 'implicitly'".to_string(),
            ));
            return Stmt::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
            });
        }
    }

    loop {
        let before_comma = ctx.pos();
        ctx.skip_spaces();
        if !ctx.remaining().starts_with(',') {
            ctx.set_pos(before_comma);
            break;
        }
        ctx.advance(1); // ","
        ctx.skip_spaces();
        // Trailing comma: next is `{` or newline/end
        if ctx.remaining().starts_with('{') || ctx.is_at_newline() || ctx.remaining().is_empty() {
            break;
        }
        match parse_expr(ctx) {
            Some(expr) => exprs.push(expr),
            None => {
                ctx.add_parse_error(ParseError::new(
                    (before_comma, ctx.pos()),
                    "Expected expression after ','".to_string(),
                ));
                break;
            }
        }
    }

    // Check for block form
    ctx.skip_spaces();
    let block = if ctx.remaining().starts_with('{') {
        Some(parse_stmt_block(ctx))
    } else {
        None
    };

    Stmt::Implicitly(StmtImplicitly {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        exprs,
        block,
    })
}

/// Parse `var` declaration: `var name [: type] [= expr]`
fn parse_var(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();
    ctx.advance(3); // "var"
    ctx.skip_spaces();

    // Parse first declaration: either shorthand `:type` or `name [: type]`
    let decl_start = ctx.pos();
    let (name, ty) = if ctx.remaining().starts_with(':') {
        // Shorthand: `var :type` - derive name from type
        ctx.advance(1); // ':'
        let parsed_ty = parse_type(ctx);
        let derived = super::construct::derive_field_name(&parsed_ty).unwrap_or_else(|| {
            ctx.add_parse_error(ParseError::new(
                (decl_start, ctx.pos()),
                "Could not derive variable name from type".to_string(),
            ));
            Ident {
                node_id: ctx.alloc_node_id(decl_start, ctx.pos()),
                name: Symbol::unknown(),
                is_raw: false,
            }
        });
        (derived, Some(parsed_ty))
    } else {
        let name = match try_parse_ident(ctx) {
            Ok(Some(ident)) => ident,
            Ok(None) => {
                ctx.add_parse_error(ParseError::new(
                    (start, ctx.pos()),
                    "Expected identifier after 'var'".to_string(),
                ));
                ctx.skip_past_end_of_line();
                return Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                });
            }
            Err(err) => {
                ctx.add_parse_error(err);
                ctx.skip_past_end_of_line();
                return Stmt::Invalid(InvalidNode {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                });
            }
        };
        let ty = parse_var_optional_type(ctx);
        (name, ty)
    };
    let mut vars = vec![StmtVarDecl {
        node_id: ctx.alloc_node_id(decl_start, ctx.pos()),
        name,
        ty,
    }];

    // Additional comma-separated declarations
    loop {
        let before_comma = ctx.pos();
        ctx.skip_spaces();
        if !ctx.remaining().starts_with(',') {
            ctx.set_pos(before_comma);
            break;
        }
        ctx.advance(1); // ","
        ctx.skip_spaces();

        let decl_start = ctx.pos();
        match try_parse_ident(ctx) {
            Ok(Some(name)) => {
                let ty = parse_var_optional_type(ctx);
                vars.push(StmtVarDecl {
                    node_id: ctx.alloc_node_id(decl_start, ctx.pos()),
                    name,
                    ty,
                });
            }
            Ok(None) => {
                ctx.add_parse_error(ParseError::new(
                    (before_comma + 1, ctx.pos().max(before_comma + 2)),
                    "Expected identifier after ','".to_string(),
                ));
                break;
            }
            Err(err) => {
                ctx.add_parse_error(err);
                break;
            }
        }
    }

    // Optional `= expr, expr, ...`
    let before_eq = ctx.pos();
    ctx.skip_spaces();
    let values = if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
        ctx.advance(1);
        ctx.skip_spaces();
        let mut vals = Vec::new();
        match parse_expr(ctx) {
            Some(e) => vals.push(e),
            None => {
                ctx.add_parse_error(ParseError::new(
                    (before_eq, ctx.pos()),
                    "Expected expression after '='".to_string(),
                ));
                return Stmt::Var(StmtVar {
                    node_id: ctx.alloc_node_id(start, ctx.pos()),
                    vars,
                    values: vals,
                });
            }
        }
        // Additional comma-separated values
        loop {
            let before_comma = ctx.pos();
            ctx.skip_spaces();
            if !ctx.remaining().starts_with(',') {
                ctx.set_pos(before_comma);
                break;
            }
            ctx.advance(1); // ","
            ctx.skip_spaces();
            match parse_expr(ctx) {
                Some(e) => vals.push(e),
                None => {
                    ctx.add_parse_error(ParseError::new(
                        (before_comma + 1, ctx.pos().max(before_comma + 2)),
                        "Expected expression after ','".to_string(),
                    ));
                    break;
                }
            }
        }
        vals
    } else {
        ctx.set_pos(before_eq);
        Vec::new()
    };

    if !values.is_empty() && vars.len() != values.len() {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            format!(
                "Expected {} values after '=', found {}",
                vars.len(),
                values.len()
            ),
        ));
    }

    Stmt::Var(StmtVar {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        vars,
        values,
    })
}

/// Parse optional `: type` after a var declaration name.
fn parse_var_optional_type(ctx: &mut ParseContext) -> Option<Type> {
    let before_colon = ctx.pos();
    let spaces_before_colon = ctx.skip_spaces();
    if ctx.remaining().starts_with(':') {
        if spaces_before_colon != 0 {
            ctx.add_strict_violation(ParseError::new(
                (before_colon, ctx.pos()),
                "Expected no space before ':'".to_string(),
            ));
        }
        ctx.advance(1);
        let after_colon = ctx.pos();
        if ctx.skip_spaces() != 1 {
            ctx.add_strict_violation(ctx.spacing_error(
                after_colon,
                "Expected exactly one space after ':'".to_string(),
            ));
        }
        Some(parse_type(ctx))
    } else {
        ctx.set_pos(before_colon);
        None
    }
}

/// Parse a patch version: qualified identifier or `@default`.
fn parse_patch_version(ctx: &mut ParseContext) -> Vec<Ident> {
    if ctx.remaining().starts_with("@default")
        && !ctx.remaining()[8..].starts_with(|c: char| c.is_alphanumeric() || c == '_')
    {
        let start = ctx.pos();
        ctx.advance(8); // "@default"
        vec![Ident {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            name: Symbol::default_patch(),
            is_raw: false,
        }]
    } else {
        match ctx.parse_qualified_ident() {
            Ok(idents) => idents,
            Err(err) => {
                ctx.add_parse_error(err);
                Vec::new()
            }
        }
    }
}

/// Parse statements inside a patch branch.
/// Stops when it sees `% }}` (branch end/continuation) or `}` / EOF.
fn parse_patch_body(ctx: &mut ParseContext) -> Vec<Stmt> {
    let mut stmts = Vec::new();
    loop {
        // Peek past whitespace/newlines without consuming
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.is_empty() || peek.starts_with('}') {
            break;
        }
        // Check for `% }}` - branch end/continuation (not a nested patch)
        if let Some(stripped) = peek.strip_prefix('%') {
            let after_pct = stripped.trim_start_matches(' ');
            if after_pct.starts_with("}}") {
                break;
            }
        }

        let pos_before = ctx.pos();
        ctx.parse_trivia();
        if ctx.remaining().is_empty() {
            break;
        }

        if let Some(stmt) = parse_stmt(ctx) {
            ctx.check_stmt_termination();
            stmts.push(stmt);
        } else {
            ctx.add_parse_error(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Unrecognized content in patch branch".to_string(),
            ));
            ctx.skip_past_end_of_line();
            if ctx.pos() == pos_before {
                break;
            }
        }
    }
    stmts
}

/// Parse a patch statement: chain or complete form.
/// Assumes `%` is at current position (and not `%=`).
fn parse_patch(ctx: &mut ParseContext) -> Stmt {
    let start = ctx.pos();

    if !ctx.is_at_line_start() {
        ctx.add_strict_violation(ParseError::new(
            (start, start + 1),
            "Patch '%' should start at column 0".to_string(),
        ));
    }

    ctx.advance(1); // "%"
    ctx.skip_spaces();

    // Parse version
    let version = parse_patch_version(ctx);
    if version.is_empty() {
        ctx.skip_past_end_of_line();
        return Stmt::Invalid(InvalidNode {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
        });
    }
    ctx.skip_spaces();

    // Check for `complete` form
    if starts_with_keyword(ctx.remaining(), "complete") {
        ctx.advance(8); // "complete"
        return Stmt::Patch(StmtPatch {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            kind: StmtPatchKind::Complete(StmtPatchComplete {
                node_id: ctx.alloc_node_id(start, ctx.pos()),
                version,
            }),
        });
    }

    // Must be `{{` for chain start
    if !ctx.remaining().starts_with("{{") {
        ctx.add_parse_error(ParseError::new(
            (start, ctx.pos()),
            "Expected '{{' or 'complete' after patch version".to_string(),
        ));
        ctx.skip_past_end_of_line();
        return Stmt::Invalid(InvalidNode {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
        });
    }
    ctx.advance(2); // "{{"

    // First branch
    let branch_start = start;
    let stmts = parse_patch_body(ctx);
    let mut branches = vec![StmtPatchBranch {
        node_id: ctx.alloc_node_id(branch_start, ctx.pos()),
        version,
        stmts,
    }];

    // Continuation loop: `% }} version {{` or `% }}` (end)
    // Skip whitespace manually (not parse_trivia) to avoid indentation violations
    // on `%` delimiter lines which have special column-0 rules.
    loop {
        let skip = ctx
            .remaining()
            .find(|c: char| c != ' ' && c != '\n' && c != '\r')
            .unwrap_or(ctx.remaining().len());
        ctx.advance(skip);

        if !ctx.remaining().starts_with('%') {
            ctx.add_parse_error(ParseError::new(
                (start, ctx.pos()),
                "Unterminated patch chain, expected '% }}'".to_string(),
            ));
            break;
        }

        let cont_start = ctx.pos();

        if !ctx.is_at_line_start() {
            ctx.add_strict_violation(ParseError::new(
                (cont_start, cont_start + 1),
                "Patch '%' should start at column 0".to_string(),
            ));
        }

        ctx.advance(1); // "%"
        ctx.skip_spaces();

        // Must see `}}`
        if !ctx.remaining().starts_with("}}") {
            ctx.add_parse_error(ParseError::new(
                (cont_start, ctx.pos()),
                "Expected '}}' in patch continuation".to_string(),
            ));
            break;
        }
        ctx.advance(2); // "}}"
        ctx.skip_spaces();

        // End of chain or continuation?
        if ctx.remaining().is_empty()
            || ctx.remaining().starts_with('\n')
            || ctx.remaining().starts_with("\r\n")
        {
            // End of chain
            break;
        }

        // Parse continuation version
        let version = parse_patch_version(ctx);
        if version.is_empty() {
            ctx.skip_past_end_of_line();
            break;
        }
        ctx.skip_spaces();

        // Must be `{{`
        if !ctx.remaining().starts_with("{{") {
            ctx.add_parse_error(ParseError::new(
                (cont_start, ctx.pos()),
                "Expected '{{' after patch version".to_string(),
            ));
            ctx.skip_past_end_of_line();
            break;
        }
        ctx.advance(2); // "{{"

        // Parse branch body
        let stmts = parse_patch_body(ctx);
        branches.push(StmtPatchBranch {
            node_id: ctx.alloc_node_id(cont_start, ctx.pos()),
            version,
            stmts,
        });
    }

    Stmt::Patch(StmtPatch {
        node_id: ctx.alloc_node_id(start, ctx.pos()),
        kind: StmtPatchKind::Chain(StmtPatchChain {
            node_id: ctx.alloc_node_id(start, ctx.pos()),
            branches,
        }),
    })
}
