use crate::model::*;

use super::{ParseContext, ParseError};

/// Parse an identifier (normal or raw)
/// Spec 5 - identifier = identifier_normal | identifier_raw .
pub(crate) fn try_parse_ident(ctx: &mut ParseContext) -> Result<Option<Ident>, ParseError> {
    let start_pos = ctx.pos();
    let remaining = ctx.remaining();

    if remaining.is_empty() {
        return Ok(None);
    }

    let first_char = remaining.chars().next().unwrap();

    // Raw identifier (single backtick delimiter, `` escapes to literal backtick)
    if first_char == '`' {
        // Two+ backticks at start = raw string literal, not raw ident
        if remaining.as_bytes().get(1) == Some(&b'`') {
            return Ok(None);
        }
        ctx.advance(1); // opening `
        let mut name = String::new();
        let mut terminated = false;
        loop {
            // Copy everything up to the next backtick
            let chunk_end = match ctx.remaining().find('`') {
                Some(pos) => pos,
                None => {
                    name.push_str(ctx.remaining());
                    let len = ctx.remaining().len();
                    ctx.advance(len);
                    break;
                }
            };
            name.push_str(&ctx.remaining()[..chunk_end]);
            ctx.advance(chunk_end);

            // Count consecutive backticks: pairs are escapes, odd remainder closes
            let mut run = 0;
            while ctx.remaining().as_bytes().get(run) == Some(&b'`') {
                run += 1;
            }
            ctx.advance(run);
            for _ in 0..run / 2 {
                name.push('`');
            }
            if run % 2 == 1 {
                terminated = true;
                break;
            }
        }
        if !terminated {
            return Err(ParseError::with_range(
                (start_pos, ctx.pos()),
                "Unterminated raw identifier".to_string(),
                (start_pos, ctx.pos()),
            ));
        }
        if name.is_empty() {
            return Err(ParseError::with_range(
                (start_pos, ctx.pos()),
                "Raw identifier cannot be empty".to_string(),
                (start_pos, ctx.pos()),
            ));
        }
        let node_id = ctx.alloc_node_id(start_pos, ctx.pos());
        return Ok(Some(Ident {
            node_id,
            name,
            is_raw: true,
        }));
    }

    // Check if this looks like it might be an identifier
    if !first_char.is_alphanumeric() && first_char != '_' {
        return Ok(None); // Not an identifier at all
    }

    // Consume identifier-like characters (alphanumeric, underscore, hyphen)
    // Hyphens are consumed to give better errors for kebab-case attempts
    let mut len = 0;
    let mut first_invalid_pos = None;

    for ch in remaining.chars() {
        if ch.is_alphanumeric() || ch == '_' {
            // Check if this character is valid in Duralade identifiers
            if first_invalid_pos.is_none()
                && !ch.is_lowercase()
                && !ch.is_ascii_digit()
                && ch != '_'
            {
                first_invalid_pos = Some(start_pos + len);
            }
            len += ch.len_utf8();
        } else if ch == '-' && len > 0 {
            // Consume hyphen as part of the token for better diagnostics
            if first_invalid_pos.is_none() {
                first_invalid_pos = Some(start_pos + len);
            }
            len += 1;
        } else {
            break;
        }
    }

    if len == 0 {
        return Ok(None);
    }

    let name = remaining[..len].to_string();
    ctx.advance(len);

    // Check if identifier starts with a digit
    if first_char.is_ascii_digit() {
        return Err(ParseError::with_range(
            (start_pos, start_pos + len),
            format!("Identifier '{}' cannot start with a digit", name),
            (start_pos, start_pos + 1),
        ));
    }

    if let Some(invalid_pos) = first_invalid_pos {
        let invalid_char = name[invalid_pos - start_pos..].chars().next().unwrap();
        return Err(ParseError::with_range(
            (start_pos, start_pos + len),
            format!(
                "Unexpected character '{}' in identifier '{}' (must be lowercase, digits, or underscore)",
                invalid_char, name
            ),
            (invalid_pos, invalid_pos + invalid_char.len_utf8()),
        ));
    }

    let node_id = ctx.alloc_node_id(start_pos, start_pos + len);
    Ok(Some(Ident {
        node_id,
        name,
        is_raw: false,
    }))
}
