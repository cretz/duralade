use std::collections::HashSet;

use crate::model::*;

use super::construct::{
    ConstructModifiers, expect_open_brace, parse_construct_header, parse_field, parse_func,
    parse_modifiers, parse_native, starts_with_keyword,
};
use super::source;
use super::stmt::parse_stmt;
use super::{ParseContext, ParseError};

/// Contents parsed from a construct body `{ ... }`
#[derive(Default)]
pub(crate) struct BodyContents {
    pub fields: Vec<Field>,
    pub stmts: Vec<Stmt>,
    pub funcs: Vec<Construct>,
    pub init: Option<EntityInit>,
    pub run: Option<EntityRun>,
}

/// Configuration for what a construct body can contain.
/// Default is all false.
#[derive(Default)]
pub(crate) struct BodyOptions {
    pub fields: bool,
    pub data_fields: bool,
    pub stmts: bool,
    pub funcs: bool,
    pub init_run: bool,
}

fn field_name(field: &Field) -> Option<&Ident> {
    match &field.kind {
        FieldKind::Var(v) => Some(&v.name),
        FieldKind::Intype(i) => Some(&i.name),
        FieldKind::Invalid(_) => None,
    }
}

/// Peek past whitespace to check if '}' or EOF is next.
/// Does not consume anything or trigger indentation checks.
fn at_body_end(ctx: &ParseContext) -> bool {
    let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
    peek.is_empty() || peek.starts_with('}')
}

/// Parse annotations and check for blank-line violations.
fn parse_item_annotations(ctx: &mut ParseContext, target: &str) -> Vec<Annotation> {
    let groups = source::parse_annotations(ctx);
    if groups.len() > 1 {
        let ann = groups[1..]
            .iter()
            .flat_map(|v| v.iter())
            .next()
            .or_else(|| groups[0].last())
            .unwrap();
        ctx.add_strict_violation(ParseError::new(
            ctx.node_span(ann.node_id),
            format!("No blank lines allowed between annotations and {}", target),
        ));
    }
    groups.into_iter().flatten().collect()
}

/// Parse a member function with pre-parsed annotations and modifiers.
/// Caller has already determined next word is `func`.
fn parse_member_func(
    ctx: &mut ParseContext,
    start_pos: usize,
    annotations: Vec<Annotation>,
    modifiers: ConstructModifiers,
) -> Construct {
    let header = parse_construct_header(ctx, modifiers);

    debug_assert_eq!(header.keyword, "func");

    for err in header.errors {
        ctx.add_parse_error(err);
    }

    let builtin = header.modifiers.builtin.is_some();
    let func = if builtin {
        let native = parse_native(
            ctx,
            start_pos,
            header.modifiers.view.is_some(),
            header.modifiers.noblock.is_some(),
        );
        Func {
            node_id: native.node_id,
            view: native.view,
            noblock: native.noblock,
            fields: native.fields,
            stmts: Vec::new(),
        }
    } else {
        parse_func(
            ctx,
            start_pos,
            header.modifiers.view.is_some(),
            header.modifiers.noblock.is_some(),
        )
    };

    Construct {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        annotations,
        out: header.modifiers.out.is_some(),
        builtin,
        name: header.name,
        kind: ConstructKind::Func(func),
    }
}

/// Parse `init { fields stmts }` or `run { fields stmts }`.
/// Returns None if not at init/run keyword (position unchanged).
fn parse_init_or_run(ctx: &mut ParseContext) -> Option<InitOrRun> {
    let start_pos = ctx.pos();
    let r = ctx.remaining();

    let keyword = if starts_with_keyword(r, "init") {
        ctx.advance(4);
        "init"
    } else if starts_with_keyword(r, "run") {
        ctx.advance(3);
        "run"
    } else {
        return None;
    };

    if !expect_open_brace(ctx, keyword) {
        return Some(match keyword {
            "init" => InitOrRun::Init(EntityInit {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                fields: Vec::new(),
                stmts: Vec::new(),
            }),
            _ => InitOrRun::Run(EntityRun {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                fields: Vec::new(),
                stmts: Vec::new(),
            }),
        });
    }

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            stmts: true,
            ..Default::default()
        },
    );

    Some(match keyword {
        "init" => InitOrRun::Init(EntityInit {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields: body.fields,
            stmts: body.stmts,
        }),
        _ => InitOrRun::Run(EntityRun {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields: body.fields,
            stmts: body.stmts,
        }),
    })
}

enum InitOrRun {
    Init(EntityInit),
    Run(EntityRun),
}

/// Parse a construct body: `{ ... }`.
/// Caller must have verified '{' is at current position.
pub(crate) fn parse_body(ctx: &mut ParseContext, options: BodyOptions) -> BodyContents {
    debug_assert!(ctx.remaining().starts_with('{'));
    ctx.advance(1);
    ctx.indent(b'{');

    let mut contents = BodyContents::default();

    if options.funcs || options.init_run {
        // Data/entity bodies: single loop handling fields, funcs, init/run.
        // Annotations and modifiers are parsed as a shared prefix, then we
        // decide field vs func based on what follows.
        let mut field_phase = options.fields;
        let mut seen_names: HashSet<Symbol> = HashSet::new();

        loop {
            if at_body_end(ctx) {
                break;
            }

            let pos_before = ctx.pos();
            ctx.parse_trivia();
            if ctx.remaining().is_empty() {
                break;
            }

            // Check for init/run (entity bodies) - before annotations
            if options.init_run
                && (starts_with_keyword(ctx.remaining(), "init")
                    || starts_with_keyword(ctx.remaining(), "run"))
            {
                field_phase = false;
                if let Some(init_or_run) = parse_init_or_run(ctx) {
                    match init_or_run {
                        InitOrRun::Init(init) => {
                            if contents.init.is_some() {
                                let span = ctx.node_span(init.node_id);
                                ctx.add_parse_error(ParseError::new(
                                    (span.0, span.0 + "init".len()),
                                    "Duplicate 'init' block".to_string(),
                                ));
                            }
                            contents.init = Some(init);
                        }
                        InitOrRun::Run(run) => {
                            if contents.run.is_some() {
                                let span = ctx.node_span(run.node_id);
                                ctx.add_parse_error(ParseError::new(
                                    (span.0, span.0 + "run".len()),
                                    "Duplicate 'run' block".to_string(),
                                ));
                            }
                            contents.run = Some(run);
                        }
                    }
                    continue;
                }
            }

            // Shared prefix: annotations + modifiers
            let item_start = ctx.pos();
            let annotations = parse_item_annotations(ctx, "member");
            let modifiers = parse_modifiers(ctx);

            // If view/noblock found, or next word is 'func' → member func
            if modifiers.view.is_some()
                || modifiers.noblock.is_some()
                || starts_with_keyword(ctx.remaining(), "func")
            {
                field_phase = false;
                let func = parse_member_func(ctx, item_start, annotations, modifiers);
                if !seen_names.insert(func.name.name.clone()) {
                    ctx.add_parse_error(ParseError::new(
                        ctx.node_span(func.name.node_id),
                        format!("Duplicate member name '{}'", func.name.name),
                    ));
                }
                contents.funcs.push(func);
                continue;
            }

            // Try as field
            if field_phase
                && let Some(field) = parse_field(
                    ctx,
                    item_start,
                    annotations,
                    modifiers.out,
                    options.data_fields,
                )
            {
                if let Some(ident) = field_name(&field)
                    && !seen_names.insert(ident.name.clone())
                {
                    ctx.add_parse_error(ParseError::new(
                        ctx.node_span(ident.node_id),
                        format!("Duplicate member name '{}'", ident.name),
                    ));
                }
                contents.fields.push(field);
                continue;
            }

            // Unrecognized content
            ctx.add_parse_error(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Unrecognized content in body".to_string(),
            ));
            ctx.skip_past_end_of_line();
            if ctx.pos() == pos_before {
                break;
            }
        }
    } else {
        // Func/init/run bodies: two-phase (fields then stmts)

        // Phase 1: fields
        if options.fields {
            let mut seen_names: HashSet<Symbol> = HashSet::new();
            loop {
                if at_body_end(ctx) {
                    break;
                }

                ctx.parse_trivia();
                if ctx.remaining().is_empty() || at_body_end(ctx) {
                    break;
                }

                let item_start = ctx.pos();
                let annotations = parse_item_annotations(ctx, "field");
                if let Some(field) =
                    parse_field(ctx, item_start, annotations, None, options.data_fields)
                {
                    if let Some(ident) = field_name(&field)
                        && !seen_names.insert(ident.name.clone())
                    {
                        ctx.add_parse_error(ParseError::new(
                            ctx.node_span(ident.node_id),
                            format!("Duplicate field name '{}'", ident.name),
                        ));
                    }
                    contents.fields.push(field);
                } else {
                    break;
                }
            }
        }

        // Phase 2: stmts
        if options.stmts {
            loop {
                if at_body_end(ctx) {
                    break;
                }

                let pos_before = ctx.pos();
                ctx.parse_trivia();
                if ctx.remaining().is_empty() || at_body_end(ctx) {
                    break;
                }

                if let Some(stmt) = parse_stmt(ctx) {
                    ctx.check_stmt_termination();
                    contents.stmts.push(stmt);
                } else {
                    ctx.add_parse_error(ParseError::new(
                        (ctx.pos(), ctx.pos() + 1),
                        "Unrecognized content in body".to_string(),
                    ));
                    ctx.skip_past_end_of_line();
                    if ctx.pos() == pos_before {
                        break;
                    }
                }
            }
        }
    }

    // Strict: check field and member func ordering
    ctx.assert_field_ordering(&contents.fields);
    ctx.assert_ordering(&[], &contents.funcs);

    // Closing brace
    ctx.parse_trivia();
    ctx.dedent();
    if ctx.remaining().starts_with('}') {
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ctx.error_here("Expected '}'".to_string()));
    }

    contents
}
