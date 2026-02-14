use crate::model::*;

use super::basic::try_parse_ident;
use super::body::{BodyOptions, parse_body};
use super::expr::parse_expr;
use super::source;
use super::types::parse_type;
use super::{ParseContext, ParseError};

/// Parsed modifiers: [out] [builtin] [view|noblock]
pub(crate) struct ConstructModifiers {
    /// Span of 'out' keyword if present (start..end, end-exclusive)
    pub out: Option<(usize, usize)>,
    /// Span of 'builtin' keyword if present
    pub builtin: Option<(usize, usize)>,
    /// Span of 'view' keyword if present
    pub view: Option<(usize, usize)>,
    /// Span of 'noblock' keyword if present
    pub noblock: Option<(usize, usize)>,
}

/// Parsed header of a construct: [out] [view|noblock] keyword name
pub(crate) struct ConstructHeader {
    pub modifiers: ConstructModifiers,
    pub keyword: String,
    pub keyword_span: (usize, usize),
    pub name: Ident,
    pub errors: Vec<ParseError>,
}

/// Expect `{` after spaces on the current line.
/// Returns true if `{` found. Reports strict violation if spacing != 1 space.
/// Reports parse error and skips line if `{` missing.
pub(crate) fn expect_open_brace(ctx: &mut ParseContext, construct_name: &str) -> bool {
    let after_name = ctx.pos();
    let spaces = ctx.skip_spaces();

    if ctx.remaining().starts_with('{') {
        if spaces != 1 {
            ctx.add_strict_violation(ParseError::new(
                (after_name, ctx.pos()),
                "Expected exactly one space before '{'".to_string(),
            ));
        }
        true
    } else {
        ctx.add_parse_error(ctx.error_here(format!("Expected '{{' in {}", construct_name)));
        ctx.skip_past_end_of_line();
        false
    }
}

/// Check if a keyword starts at current position, followed by space or EOF
pub(crate) fn starts_with_keyword(remaining: &str, keyword: &str) -> bool {
    remaining.starts_with(keyword)
        && remaining[keyword.len()..]
            .chars()
            .next()
            .is_none_or(|c| !c.is_alphanumeric() && c != '_')
}

/// Parse [out] [view|noblock] modifiers with strict spacing.
/// Caller must have already parsed trivia/annotations.
pub(crate) fn parse_modifiers(ctx: &mut ParseContext) -> ConstructModifiers {
    // Parse optional 'out' modifier (but not 'out!' which is a field modifier)
    let out =
        if starts_with_keyword(ctx.remaining(), "out") && !ctx.remaining()[3..].starts_with('!') {
            let start = ctx.pos();
            ctx.advance(3);
            let after_modifier = ctx.pos();
            if ctx.parse_trivia() != " " {
                ctx.add_strict_violation(ParseError::new(
                    (after_modifier, ctx.pos()),
                    "Expected exactly one space after 'out'".to_string(),
                ));
            }
            Some((start, start + 3))
        } else {
            None
        };

    // Parse optional 'builtin' modifier
    let builtin = if starts_with_keyword(ctx.remaining(), "builtin") {
        let start = ctx.pos();
        ctx.advance(7);
        if !ctx.allow_builtin {
            ctx.add_parse_error(ParseError::new(
                (start, start + 7),
                "'builtin' requires allow_builtin = true in duralade.toml".to_string(),
            ));
        }
        let after_modifier = ctx.pos();
        if ctx.parse_trivia() != " " {
            ctx.add_strict_violation(ParseError::new(
                (after_modifier, ctx.pos()),
                "Expected exactly one space after 'builtin'".to_string(),
            ));
        }
        Some((start, start + 7))
    } else {
        None
    };

    // Parse optional 'view' or 'noblock' modifier
    let (view, noblock) = if starts_with_keyword(ctx.remaining(), "view") {
        let start = ctx.pos();
        ctx.advance(4);
        let after_modifier = ctx.pos();
        if ctx.parse_trivia() != " " {
            ctx.add_strict_violation(ParseError::new(
                (after_modifier, ctx.pos()),
                "Expected exactly one space after 'view'".to_string(),
            ));
        }
        (Some((start, start + 4)), None)
    } else if starts_with_keyword(ctx.remaining(), "noblock") {
        let start = ctx.pos();
        ctx.advance(7);
        let after_modifier = ctx.pos();
        if ctx.parse_trivia() != " " {
            ctx.add_strict_violation(ParseError::new(
                (after_modifier, ctx.pos()),
                "Expected exactly one space after 'noblock'".to_string(),
            ));
        }
        (None, Some((start, start + 7)))
    } else {
        (None, None)
    };

    ConstructModifiers {
        out,
        builtin,
        view,
        noblock,
    }
}

/// Parse [out] [view|noblock] keyword name with strict spacing.
/// Caller must have already parsed trivia/annotations.
/// Accepts pre-parsed modifiers (e.g. from body disambiguation).
pub(crate) fn parse_construct_header(
    ctx: &mut ParseContext,
    modifiers: ConstructModifiers,
) -> ConstructHeader {
    let mut errors = Vec::new();

    // Parse keyword
    let keyword_start = ctx.pos();
    let keyword = ctx.remaining().split(' ').next().unwrap_or("").to_string();
    ctx.advance(keyword.len());
    let keyword_span = (keyword_start, ctx.pos());

    // Strict space after keyword
    let after_keyword = ctx.pos();
    if ctx.parse_trivia() != " " {
        ctx.add_strict_violation(ParseError::new(
            (after_keyword, ctx.pos()),
            format!("Expected exactly one space after '{}'", keyword),
        ));
    }

    // Parse identifier
    let name = match try_parse_ident(ctx) {
        Ok(Some(ident)) => ident,
        Ok(None) => {
            errors.push(ctx.error_here(format!("Expected identifier after '{}'", keyword)));
            Ident {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
                name: Symbol::unknown(),
                is_raw: false,
            }
        }
        Err(err) => {
            errors.push(err);
            Ident {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
                name: Symbol::unknown(),
                is_raw: false,
            }
        }
    };

    ConstructHeader {
        modifiers,
        keyword,
        keyword_span,
        name,
        errors,
    }
}

/// Parse a construct: [annotations] [out] [view|noblock] keyword name { body }
/// `extra` provides pre-parsed annotations (e.g. from file-level splitting).
/// When None, annotations are parsed here.
pub(crate) fn parse_construct(
    ctx: &mut ParseContext,
    already_parsed: Option<Vec<Annotation>>,
) -> Construct {
    let start_pos = ctx.pos();

    let annotations = already_parsed.unwrap_or_else(|| {
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
                "No blank lines allowed between annotations and construct".to_string(),
            ));
        }
        groups.into_iter().flatten().collect()
    });

    let modifiers = parse_modifiers(ctx);
    let header = parse_construct_header(ctx, modifiers);

    // Validate modifier combinations
    let view_noblock_span = header.modifiers.view.or(header.modifiers.noblock);
    if let Some(span) = view_noblock_span
        && matches!(
            header.keyword.as_str(),
            "type" | "data" | "entity" | "extern"
        )
    {
        ctx.add_parse_error(ParseError::new(
            span,
            format!(
                "'{}' construct cannot have 'view' or 'noblock' modifier",
                header.keyword
            ),
        ));
    }
    if let Some(span) = header.modifiers.out
        && matches!(header.keyword.as_str(), "extern" | "native")
    {
        ctx.add_parse_error(ParseError::new(
            span,
            format!("'{}' construct cannot have 'out' modifier", header.keyword),
        ));
    }
    if let Some(span) = header.modifiers.builtin
        && !matches!(header.keyword.as_str(), "data" | "entity" | "func")
    {
        ctx.add_parse_error(ParseError::new(
            span,
            format!(
                "'{}' construct cannot have 'builtin' modifier",
                header.keyword
            ),
        ));
    }
    if header.keyword == "native"
        && header.modifiers.view.is_none()
        && header.modifiers.noblock.is_none()
    {
        ctx.add_parse_error(ParseError::new(
            (start_pos, start_pos + "native".len()),
            "'native' construct must have 'view' or 'noblock' modifier".to_string(),
        ));
    }

    // Add header errors
    for err in header.errors {
        ctx.add_parse_error(err);
    }

    // Dispatch to individual construct parsers
    let kind = match header.keyword.as_str() {
        "type" => ConstructKind::TypeAlias(parse_type_alias(ctx, start_pos)),
        "data" => ConstructKind::Data(parse_data(ctx, start_pos, false)),
        "entity" => ConstructKind::Entity(parse_entity(ctx, start_pos)),
        "func" if header.modifiers.builtin.is_some() => {
            // builtin func: fields only, no body - parsed like native
            let native = parse_native(
                ctx,
                start_pos,
                header.modifiers.view.is_some(),
                header.modifiers.noblock.is_some(),
            );
            ConstructKind::Func(Func {
                node_id: native.node_id,
                view: native.view,
                noblock: native.noblock,
                fields: native.fields,
                stmts: Vec::new(),
            })
        }
        "func" => ConstructKind::Func(parse_func(
            ctx,
            start_pos,
            header.modifiers.view.is_some(),
            header.modifiers.noblock.is_some(),
        )),
        "extern" => ConstructKind::Extern(parse_extern(ctx, start_pos)),
        "native" => ConstructKind::Native(parse_native(
            ctx,
            start_pos,
            header.modifiers.view.is_some(),
            header.modifiers.noblock.is_some(),
        )),
        _ => {
            // Not a construct keyword - unrecognized
            ctx.skip_past_end_of_line();
            let mut err = ParseError::new(
                (start_pos, ctx.pos()),
                "Unrecognized construct (expected type, data, entity, func, extern, or native)"
                    .to_string(),
            );
            err.error_range = Some(header.keyword_span);
            ctx.add_parse_error(err);
            ConstructKind::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            })
        }
    };

    Construct {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        annotations,
        out: header.modifiers.out.is_some(),
        builtin: header.modifiers.builtin.is_some(),
        name: header.name,
        kind,
    }
}

/// Parse a field. Returns None if current position doesn't look like a field.
/// When `data_field` is true, fields without a modifier keyword are allowed
/// (data fields are implicitly inout with no modifier).
/// Parse a field. Annotations and optional `out` modifier (from ConstructModifiers)
/// are pre-parsed by the caller.
pub(crate) fn parse_field(
    ctx: &mut ParseContext,
    start_pos: usize,
    annotations: Vec<Annotation>,
    out_modifier: Option<(usize, usize)>,
    data_field: bool,
) -> Option<Field> {
    // If 'out' was already consumed by parse_modifiers, use it as field modifier
    let (var_modifier, is_intype, modifier_len) = if out_modifier.is_some() {
        (Some(FieldVarModifier::Out), false, 0)
    } else {
        let modifier_word = ctx.remaining().split(' ').next().unwrap_or("");
        match (data_field, modifier_word) {
            (_, "intype") => (None, true, 7),
            (_, "out!") => (Some(FieldVarModifier::OutEarly), false, 5),
            (false, "in") => (Some(FieldVarModifier::In), false, 3),
            (false, "out") => (Some(FieldVarModifier::Out), false, 4),
            (false, "inout") => (Some(FieldVarModifier::Inout), false, 6),
            (false, "value") => (Some(FieldVarModifier::Value), false, 6),
            (false, "implicit") => (Some(FieldVarModifier::Implicit), false, 9),
            (true, modifier_word @ ("in" | "out" | "inout" | "value" | "implicit"))
                if {
                    let after = ctx.remaining()[modifier_word.len()..].trim_start();
                    let ch = after.chars().next().unwrap_or('\0');
                    ch == '_' || ch.is_lowercase()
                } =>
            {
                let len = modifier_word.len();
                ctx.add_parse_error(ParseError::new(
                    (ctx.pos(), ctx.pos() + len),
                    format!(
                        "Data fields don't have modifiers (remove '{}')",
                        modifier_word
                    ),
                ));
                ctx.advance(len);
                ctx.parse_trivia();
                (None, false, 0)
            }
            (true, _) => {
                // Data fields have no modifier.
                let ch = ctx.remaining().chars().next().unwrap_or('\0');
                if ch == ':' || ch == '_' || ch.is_lowercase() {
                    (None, false, 0)
                } else if annotations.is_empty() {
                    return None;
                } else {
                    ctx.add_parse_error(ParseError::new(
                        (start_pos, ctx.pos()),
                        "Expected field after annotations".to_string(),
                    ));
                    return Some(Field {
                        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                        annotations,
                        kind: FieldKind::Invalid(InvalidNode {
                            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                        }),
                    });
                }
            }
            (false, _) => {
                if annotations.is_empty() {
                    return None;
                } else {
                    // Had annotations but no field modifier
                    ctx.add_parse_error(ParseError::new(
                        (start_pos, ctx.pos()),
                        "Expected field after annotations".to_string(),
                    ));
                    return Some(Field {
                        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                        annotations,
                        kind: FieldKind::Invalid(InvalidNode {
                            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                        }),
                    });
                }
            }
        }
    };
    if modifier_len > 0 {
        // A field modifier must be followed by an identifier or ':' (for shorthand type).
        // Peek past the modifier and whitespace - if the next token is '=' this is an
        // assignment statement (e.g. `value = expr`), not a modifier-prefixed field.
        let after = ctx.remaining()[modifier_len..].trim_start();
        if after.starts_with('=') && !after.starts_with("==") {
            return None;
        }

        ctx.advance(modifier_len);

        // Parse trivia after modifier - strict mode: no additional whitespace
        let after_modifier = ctx.pos();
        if ctx.parse_trivia() != "" {
            ctx.add_strict_violation(ParseError::new(
                (after_modifier, ctx.pos()),
                "Expected no additional whitespace after field modifier".to_string(),
            ));
        }
    }

    // Parse field body: [ identifier ] [ : type ] [ = type_or_expr ]
    let mut errors = Vec::new();
    let mut name_opt: Option<Ident> = None;

    // Try to parse identifier (unless starts with : for shorthand)
    if !ctx.remaining().starts_with(':') {
        match try_parse_ident(ctx) {
            Ok(Some(ident)) => {
                name_opt = Some(ident);
            }
            Ok(None) => {
                // Expected identifier but didn't find one
                if !ctx.remaining().starts_with('=') {
                    errors.push(ParseError::with_range(
                        (start_pos, ctx.pos()),
                        "Expected identifier, ':', or '=' after field modifier".to_string(),
                        (ctx.pos(), ctx.pos() + ctx.next_char_len()),
                    ));
                }
            }
            Err(err) => {
                errors.push(err);
            }
        }
    }
    let had_explicit_name = name_opt.is_some();

    // Parse optional ": type"
    let before_colon = ctx.pos();
    let spaces_before_colon = ctx.skip_spaces();
    let type_opt = if ctx.remaining().starts_with(':') {
        // Only check spacing if we had an explicit name
        if had_explicit_name && spaces_before_colon != 0 {
            ctx.add_strict_violation(ParseError::new(
                (before_colon, ctx.pos()),
                "Expected no space before ':'".to_string(),
            ));
        }

        ctx.advance(1);

        let after_colon = ctx.pos();
        let spaces_after_colon = ctx.skip_spaces();
        if had_explicit_name && spaces_after_colon != 1 {
            ctx.add_strict_violation(ctx.spacing_error(
                after_colon,
                "Expected exactly one space after ':'".to_string(),
            ));
        } else if !had_explicit_name && spaces_after_colon != 0 && errors.is_empty() {
            ctx.add_strict_violation(ctx.spacing_error(
                after_colon,
                "Expected no space after ':' in shorthand field".to_string(),
            ));
        }

        // Parse type - handles its own multi-line recovery
        Some(parse_type(ctx))
    } else {
        None
    };

    // Derive name from type for shorthand `:type` form
    let name = name_opt
        .or_else(|| type_opt.as_ref().and_then(derive_field_name))
        .unwrap_or_else(|| {
            errors.push(ParseError::new(
                (start_pos, ctx.pos()),
                "Could not derive field name".to_string(),
            ));
            Ident {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                name: Symbol::unknown(),
                is_raw: false,
            }
        });

    // Parse optional "= value" - type depends on modifier
    // If no colon was found, the spaces already consumed while looking for ':'
    // are actually the spaces before '=', so carry them forward.
    let (before_equals, spaces_before_equals) = if type_opt.is_some() {
        let pos = ctx.pos();
        (pos, ctx.skip_spaces())
    } else {
        (before_colon, spaces_before_colon)
    };
    let (default_type, default_expr) = if ctx.remaining().starts_with('=') {
        if spaces_before_equals != 1 {
            ctx.add_strict_violation(ParseError::new(
                (before_equals, ctx.pos()),
                "Expected exactly one space before '='".to_string(),
            ));
        }

        ctx.advance(1);

        let after_equals = ctx.pos();
        if ctx.skip_spaces() != 1 {
            ctx.add_strict_violation(ParseError::new(
                (after_equals, ctx.pos()),
                "Expected exactly one space after '='".to_string(),
            ));
        }

        // Parse type or expression based on whether this is intype
        if is_intype {
            // intype: parse type for default
            (Some(parse_type(ctx)), None)
        } else {
            // var field: parse expression for default
            let default_expr = parse_expr(ctx);
            if default_expr.is_none() {
                errors.push(ParseError::new(
                    (before_equals, ctx.pos()),
                    "Expected expression after '='".to_string(),
                ));
            }
            (None, default_expr)
        }
    } else {
        (None, None)
    };

    // If we had any errors, return Invalid field
    if !errors.is_empty() {
        for err in errors {
            ctx.add_parse_error(err);
        }

        return Some(Field {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            annotations,
            kind: FieldKind::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            }),
        });
    }

    // Construct the appropriate FieldKind
    let kind = if let Some(modifier) = var_modifier {
        // var field (with explicit modifier)
        FieldKind::Var(FieldVar {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            modifier,
            name: name.clone(),
            ty: type_opt,
            default: default_expr,
        })
    } else if is_intype {
        // intype field - validate: must have explicit identifier
        if had_explicit_name {
            FieldKind::Intype(FieldIntype {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
                name,
                constraint: type_opt,
                default: default_type,
            })
        } else {
            ctx.add_parse_error(ParseError::new(
                (start_pos, ctx.pos()),
                "intype field must have an identifier".to_string(),
            ));
            FieldKind::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            })
        }
    } else {
        // data field (no modifier, implicitly inout)
        FieldKind::Var(FieldVar {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            modifier: FieldVarModifier::Inout,
            name,
            ty: type_opt,
            default: default_expr,
        })
    };

    Some(Field {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        annotations,
        kind,
    })
}

/// Derive a field name from a type for shorthand `:type` fields.
/// Unwraps nilable and entity ref wrappers, then uses the last path segment of a named type.
pub(super) fn derive_field_name(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Named(named) => named.path.last().cloned(),
        Type::Nilable(n) => derive_field_name(&n.inner),
        Type::EntityRef(r) => derive_field_name(&r.inner),
        _ => None,
    }
}

fn parse_type_alias(ctx: &mut ParseContext, start_pos: usize) -> TypeAlias {
    let mut fields = Vec::new();

    let after_name = ctx.pos();
    let spaces_after_name = ctx.skip_spaces();

    let (before_equals, space_before_equals) = if ctx.remaining().starts_with('{') {
        if spaces_after_name != 1 {
            ctx.add_strict_violation(ParseError::new(
                (after_name, ctx.pos()),
                "Expected exactly one space before '{'".to_string(),
            ));
        }

        let body = parse_body(
            ctx,
            BodyOptions {
                fields: true,
                ..Default::default()
            },
        );
        for field in body.fields {
            match field.kind {
                FieldKind::Intype(intype) => fields.push(intype),
                FieldKind::Var(_) => {
                    ctx.add_parse_error(ParseError::new(
                        (start_pos, ctx.pos()),
                        "Only 'intype' fields are allowed in type alias".to_string(),
                    ));
                }
                FieldKind::Invalid(_) => {}
            }
        }

        let before_eq = ctx.pos();
        let space = ctx.skip_spaces() == 1;
        (before_eq, space)
    } else {
        (after_name, spaces_after_name == 1)
    };

    // Parse "="
    if !ctx.remaining().starts_with('=') {
        ctx.add_parse_error(ctx.error_here("Expected '=' in type alias".to_string()));
        ctx.skip_past_end_of_line();
        return TypeAlias {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields,
            target: Type::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
            }),
        };
    }
    if !space_before_equals {
        ctx.add_strict_violation(ParseError::new(
            (before_equals, ctx.pos()),
            "Expected exactly one space before '='".to_string(),
        ));
    }
    ctx.advance(1);

    let after_equals = ctx.pos();
    if ctx.parse_trivia() != " " {
        ctx.add_strict_violation(ParseError::new(
            (after_equals, ctx.pos()),
            "Expected exactly one space after '='".to_string(),
        ));
    }

    let target = parse_type(ctx);

    TypeAlias {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        fields,
        target,
    }
}

pub(crate) fn parse_data(ctx: &mut ParseContext, start_pos: usize, anonymous: bool) -> Data {
    if !expect_open_brace(ctx, "data") {
        return Data {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields: Vec::new(),
            funcs: Vec::new(),
        };
    }

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            data_fields: true,
            funcs: !anonymous,
            ..Default::default()
        },
    );

    for field in &body.fields {
        let FieldKind::Var(var) = &field.kind else {
            continue;
        };
        if var.modifier == FieldVarModifier::OutEarly && !anonymous {
            ctx.add_parse_error(ParseError::new(
                ctx.node_span(var.node_id),
                "'out!' field is not allowed in named data".to_string(),
            ));
        }
    }

    Data {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        fields: body.fields,
        funcs: body.funcs,
    }
}

fn parse_entity(ctx: &mut ParseContext, start_pos: usize) -> Entity {
    if !expect_open_brace(ctx, "entity") {
        return Entity {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields: Vec::new(),
            init: None,
            run: None,
            funcs: Vec::new(),
        };
    }

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            funcs: true,
            init_run: true,
            ..Default::default()
        },
    );

    Entity {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        fields: body.fields,
        init: body.init,
        run: body.run,
        funcs: body.funcs,
    }
}

pub(crate) fn parse_func(
    ctx: &mut ParseContext,
    start_pos: usize,
    view: bool,
    noblock: bool,
) -> Func {
    if !expect_open_brace(ctx, "func") {
        return Func {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            view,
            noblock,
            fields: Vec::new(),
            stmts: Vec::new(),
        };
    }

    let saved_defer_depth = ctx.defer_depth;
    ctx.defer_depth = 0;

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            stmts: true,
            ..Default::default()
        },
    );

    ctx.defer_depth = saved_defer_depth;

    Func {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        view,
        noblock,
        fields: body.fields,
        stmts: body.stmts,
    }
}

fn parse_extern(ctx: &mut ParseContext, start_pos: usize) -> Extern {
    if !expect_open_brace(ctx, "extern") {
        return Extern {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            fields: Vec::new(),
        };
    }

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            ..Default::default()
        },
    );

    validate_extern_or_native_fields(ctx, &body.fields, "extern");

    Extern {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        fields: body.fields,
    }
}

fn validate_extern_or_native_fields(
    ctx: &mut ParseContext,
    fields: &[Field],
    construct_name: &str,
) {
    for field in fields {
        let FieldKind::Var(var) = &field.kind else {
            continue;
        };

        match var.modifier {
            FieldVarModifier::Implicit => {
                ctx.add_parse_error(ParseError::new(
                    ctx.node_span(var.node_id),
                    format!("'implicit' field is not allowed in {construct_name}"),
                ));
            }
            FieldVarModifier::Value => {
                ctx.add_parse_error(ParseError::new(
                    ctx.node_span(var.node_id),
                    format!("'value' field is not allowed in {construct_name}"),
                ));
            }
            FieldVarModifier::Out => {
                if let Some(default_expr) = &var.default {
                    ctx.add_parse_error(ParseError::new(
                        ctx.node_span(default_expr.node_id()),
                        format!("'out' field in {construct_name} cannot have default expression"),
                    ));
                }
            }
            FieldVarModifier::OutEarly | FieldVarModifier::In | FieldVarModifier::Inout => {}
        }
    }
}

pub(crate) fn parse_native(
    ctx: &mut ParseContext,
    start_pos: usize,
    view: bool,
    noblock: bool,
) -> Native {
    if !expect_open_brace(ctx, "native") {
        return Native {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            view,
            noblock,
            fields: Vec::new(),
        };
    }

    let body = parse_body(
        ctx,
        BodyOptions {
            fields: true,
            ..Default::default()
        },
    );

    validate_extern_or_native_fields(ctx, &body.fields, "native");

    Native {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        view,
        noblock,
        fields: body.fields,
    }
}
