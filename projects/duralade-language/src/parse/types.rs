use crate::model::*;

use super::basic::try_parse_ident;
use super::construct::starts_with_keyword;
use super::{IndentOpener, ParseContext, ParseError};

/// Parse a type reference: type_inner followed by optional `?` for nilable.
pub(crate) fn parse_type(ctx: &mut ParseContext) -> Type {
    let start_pos = ctx.pos();
    let inner = parse_type_inner(ctx, start_pos);

    if ctx.remaining().starts_with('?') {
        ctx.advance(1);
        Type::Nilable(TypeNilable {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            inner: Box::new(inner),
        })
    } else {
        inner
    }
}

/// Parse the type without the trailing `?`.
/// Separated from parse_type so entity refs can recurse without consuming `?`.
fn parse_type_inner(ctx: &mut ParseContext, start_pos: usize) -> Type {
    // Entity ref: &type
    if ctx.remaining().starts_with('&') {
        ctx.advance(1);
        let inner_start = ctx.pos();
        let inner = parse_type_inner(ctx, inner_start);
        return Type::EntityRef(TypeEntityRef {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            inner: Box::new(inner),
        });
    }

    // Anonymous types: data { }, [view|noblock] func { }, entity { }
    if let Some(ty) = try_parse_type_anonymous(ctx, start_pos) {
        return ty;
    }

    // Named type: qualified path
    let path = match ctx.parse_qualified_ident() {
        Ok(path) => path,
        Err(err) => {
            ctx.add_parse_error(err);
            return Type::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            });
        }
    };

    // Optional type arguments: [arg, arg, ...]
    let type_args = if ctx.remaining().starts_with('[') {
        parse_type_arguments(ctx)
    } else {
        vec![]
    };

    let named = TypeNamed {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        path,
        type_args,
    };

    // Introspection: @type, @typein, @typeout
    let introspection_op = if ctx.remaining().starts_with("@typeout") {
        ctx.advance(8);
        Some(TypeIntrospectionOp::TypeOut)
    } else if ctx.remaining().starts_with("@typein") {
        ctx.advance(7);
        Some(TypeIntrospectionOp::TypeIn)
    } else if ctx.remaining().starts_with("@type") {
        ctx.advance(5);
        Some(TypeIntrospectionOp::Type)
    } else {
        None
    };

    if let Some(op) = introspection_op {
        Type::Introspection(TypeIntrospection {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            inner: named,
            op,
        })
    } else {
        Type::Named(named)
    }
}

/// Parse type arguments: `[arg, arg, ...]`
/// Assumes `[` is at current position.
pub(crate) fn parse_type_arguments(ctx: &mut ParseContext) -> Vec<TypeArgument> {
    let start = ctx.pos();
    ctx.advance(1); // "["
    ctx.parse_trivia();

    let mut args = Vec::new();

    // Empty: []
    if ctx.remaining().starts_with(']') {
        ctx.advance(1);
        return args;
    }

    // First argument
    if let Some(arg) = parse_type_argument(ctx) {
        args.push(arg);
    }

    // Subsequent arguments
    loop {
        ctx.skip_spaces();
        if ctx.remaining().starts_with(']') || ctx.remaining().is_empty() {
            break;
        }
        if !ctx.remaining().starts_with(',') {
            break;
        }
        ctx.advance(1); // ","
        ctx.parse_trivia();
        // Trailing comma
        if ctx.remaining().starts_with(']') || ctx.remaining().is_empty() {
            break;
        }
        if let Some(arg) = parse_type_argument(ctx) {
            args.push(arg);
        } else {
            break;
        }
    }

    if ctx.remaining().starts_with(']') {
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ParseError::with_range(
            (start, ctx.pos()),
            "Expected ']'".to_string(),
            (start, start + 1),
        ));
    }

    args
}

/// Parse a single type argument: `[ name "=" ] type`
/// If no `name =` prefix, the name is derived from the type's last path segment.
fn parse_type_argument(ctx: &mut ParseContext) -> Option<TypeArgument> {
    let arg_start = ctx.pos();

    let ty = parse_type(ctx);

    // Check for `= type` — the LHS type must be a bare identifier (named, no type args)
    let before_eq = ctx.pos();
    let spaces_before = ctx.skip_spaces();
    if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
        let name = match &ty {
            Type::Named(named) if named.type_args.is_empty() && named.path.len() == 1 => {
                named.path[0].clone()
            }
            _ => {
                ctx.add_parse_error(ParseError::new(
                    (arg_start, before_eq),
                    "Type argument name must be a simple identifier".to_string(),
                ));
                Ident {
                    node_id: ctx.alloc_node_id(arg_start, before_eq),
                    name: "<unknown>".to_string(),
                    is_raw: false,
                }
            }
        };

        if spaces_before != 1 {
            ctx.add_strict_violation(ctx.spacing_error(
                before_eq,
                "Expected exactly one space before '=' in type argument".to_string(),
            ));
        }
        ctx.advance(1); // "="
        let after_eq = ctx.pos();
        if ctx.skip_spaces() != 1 {
            ctx.add_strict_violation(ctx.spacing_error(
                after_eq,
                "Expected exactly one space after '=' in type argument".to_string(),
            ));
        }
        ctx.parse_trivia();

        let value = parse_type(ctx);

        Some(TypeArgument {
            node_id: ctx.alloc_node_id(arg_start, ctx.pos()),
            name,
            value,
        })
    } else {
        // Shorthand: derive name from type
        ctx.set_pos(before_eq);
        let name = match derive_type_arg_name(&ty) {
            Some(name) => name,
            None => {
                // If type is already invalid, errors were already reported
                if !matches!(ty, Type::Invalid(_)) {
                    ctx.add_parse_error(ParseError::new(
                        (arg_start, ctx.pos()),
                        "Could not derive name for type argument".to_string(),
                    ));
                }
                return None;
            }
        };

        Some(TypeArgument {
            node_id: ctx.alloc_node_id(arg_start, ctx.pos()),
            name,
            value: ty,
        })
    }
}

/// Derive a name from a type for shorthand type arguments.
fn derive_type_arg_name(ty: &Type) -> Option<Ident> {
    match ty {
        Type::Named(named) => named.path.last().cloned(),
        Type::Nilable(n) => derive_type_arg_name(&n.inner),
        Type::EntityRef(r) => derive_type_arg_name(&r.inner),
        _ => None,
    }
}

/// Check if the remaining input looks like the start of an anonymous type,
/// and if so, peek past the keyword(s) to verify `{` follows.
/// Returns None (position unchanged) if not an anonymous type.
fn try_parse_type_anonymous(ctx: &mut ParseContext, start_pos: usize) -> Option<Type> {
    let remaining = ctx.remaining();

    // Determine construct kind and total keyword length to skip
    let (construct, skip) = if starts_with_keyword(remaining, "data") {
        (TypeAnonConstruct::Data, 4)
    } else if starts_with_keyword(remaining, "entity") {
        (TypeAnonConstruct::Entity, 6)
    } else if starts_with_keyword(remaining, "func") {
        (TypeAnonConstruct::Func, 4)
    } else if starts_with_keyword(remaining, "view") {
        let after_mod = remaining[4..].trim_start_matches(' ');
        if starts_with_keyword(after_mod, "func") {
            let skip = remaining.len() - after_mod.len() + 4; // "view" + spaces + "func"
            (TypeAnonConstruct::FuncView, skip)
        } else {
            return None;
        }
    } else if starts_with_keyword(remaining, "noblock") {
        let after_mod = remaining[7..].trim_start_matches(' ');
        if starts_with_keyword(after_mod, "func") {
            let skip = remaining.len() - after_mod.len() + 4;
            (TypeAnonConstruct::FuncNoblock, skip)
        } else {
            return None;
        }
    } else {
        return None;
    };

    // Peek past keyword(s) and spaces — must see `{`
    let after_keyword = remaining[skip..].trim_start_matches(' ');
    if !after_keyword.starts_with('{') {
        return None;
    }

    // Commit: advance past keyword(s) and spaces to `{`
    ctx.advance(remaining.len() - after_keyword.len());

    Some(parse_type_anonymous_body(ctx, start_pos, construct))
}

/// Parse the body of an anonymous type: `{ fields }`.
/// Assumes `{` is at current position.
fn parse_type_anonymous_body(
    ctx: &mut ParseContext,
    start_pos: usize,
    construct: TypeAnonConstruct,
) -> Type {
    ctx.advance(1); // "{"
    ctx.indent(IndentOpener::Brace);

    let mut fields = Vec::new();
    loop {
        // Peek for `}` or EOF before parse_trivia
        let peek = ctx.remaining().trim_start_matches([' ', '\n', '\r']);
        if peek.is_empty() || peek.starts_with('}') {
            break;
        }
        ctx.parse_trivia();
        if ctx.remaining().starts_with('}') || ctx.remaining().is_empty() {
            break;
        }
        if let Some(field) = parse_type_anon_field(ctx) {
            fields.push(field);
        } else {
            ctx.add_parse_error(ParseError::new(
                (ctx.pos(), ctx.pos() + 1),
                "Unrecognized content in anonymous type".to_string(),
            ));
            ctx.skip_past_end_of_line();
        }
    }

    ctx.dedent();
    ctx.parse_trivia();
    if ctx.remaining().starts_with('}') {
        ctx.advance(1);
    } else {
        ctx.add_parse_error(ctx.error_here("Expected '}'".to_string()));
    }

    Type::Anon(TypeAnon {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        construct,
        fields,
    })
}

/// Parse a single anonymous type field: `[modifier] [identifier] ":" type ["=" "???"]`
fn parse_type_anon_field(ctx: &mut ParseContext) -> Option<TypeAnonField> {
    let start_pos = ctx.pos();
    let remaining = ctx.remaining();

    // Parse optional modifier
    let modifier_word = remaining.split(' ').next().unwrap_or("");
    let (modifier, modifier_len) = match modifier_word {
        "in" => (Some(TypeAnonFieldModifier::In), 3), // "in" + implicit space consumed below
        "out!" => (Some(TypeAnonFieldModifier::OutEarly), 5),
        "out" => (Some(TypeAnonFieldModifier::Out), 4),
        "inout" => (Some(TypeAnonFieldModifier::Inout), 6),
        _ => (None, 0),
    };

    if modifier_len > 0 {
        // Verify it's a keyword boundary
        if !starts_with_keyword(remaining, modifier_word) {
            return None;
        }
        ctx.advance(modifier_len);
        let after_modifier = ctx.pos();
        if ctx.parse_trivia() != "" {
            ctx.add_strict_violation(ParseError::new(
                (after_modifier, ctx.pos()),
                "Expected no additional whitespace after field modifier".to_string(),
            ));
        }
    }

    // Parse optional identifier (unless starts with `:`)
    let mut name_opt: Option<Ident> = None;
    if !ctx.remaining().starts_with(':') {
        match try_parse_ident(ctx) {
            Ok(Some(ident)) => name_opt = Some(ident),
            Ok(None) => {}
            Err(err) => {
                ctx.add_parse_error(err);
            }
        }
    }

    // Expect `:` type
    let before_colon = ctx.pos();
    let spaces_before_colon = ctx.skip_spaces();
    if !ctx.remaining().starts_with(':') {
        // No `:` — not a valid anonymous type field
        if modifier.is_none() && name_opt.is_none() {
            ctx.set_pos(start_pos);
            return None;
        }
        ctx.add_parse_error(ctx.error_here("Expected ':' in anonymous type field".to_string()));
        ctx.skip_past_end_of_line();
        return Some(TypeAnonField {
            node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
            modifier,
            name: name_opt,
            ty: Box::new(Type::Invalid(InvalidNode {
                node_id: ctx.alloc_node_id(ctx.pos(), ctx.pos()),
            })),
            has_default: false,
        });
    }

    // Spacing before `:`
    if name_opt.is_some() && spaces_before_colon != 0 {
        ctx.add_strict_violation(ParseError::new(
            (before_colon, ctx.pos()),
            "Expected no space before ':'".to_string(),
        ));
    }

    ctx.advance(1); // ":"
    let after_colon = ctx.pos();
    if ctx.skip_spaces() != 1 {
        ctx.add_strict_violation(ctx.spacing_error(
            after_colon,
            "Expected exactly one space after ':'".to_string(),
        ));
    }

    let ty = parse_type(ctx);

    // Derive name from type for shorthand `:type` form
    let name = name_opt.or_else(|| {
        if let Type::Named(named) = &ty {
            if named.type_args.is_empty() {
                named.path.last().cloned()
            } else {
                None
            }
        } else {
            None
        }
    });

    // Optional `= ???`
    let before_eq = ctx.pos();
    let spaces_before_eq = ctx.skip_spaces();
    let has_default = if ctx.remaining().starts_with('=') && !ctx.remaining().starts_with("==") {
        if spaces_before_eq != 1 {
            ctx.add_strict_violation(ParseError::new(
                (before_eq, ctx.pos()),
                "Expected exactly one space before '='".to_string(),
            ));
        }
        ctx.advance(1); // "="
        let after_eq = ctx.pos();
        if ctx.skip_spaces() != 1 {
            ctx.add_strict_violation(ParseError::new(
                (after_eq, ctx.pos()),
                "Expected exactly one space after '='".to_string(),
            ));
        }
        if ctx.remaining().starts_with("???") {
            ctx.advance(3);
        } else {
            ctx.add_parse_error(
                ctx.error_here("Expected '???' after '=' in anonymous type field".to_string()),
            );
            ctx.skip_past_end_of_line();
        }
        true
    } else {
        ctx.set_pos(before_eq);
        false
    };

    Some(TypeAnonField {
        node_id: ctx.alloc_node_id(start_pos, ctx.pos()),
        modifier,
        name,
        ty: Box::new(ty),
        has_default,
    })
}
