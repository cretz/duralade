mod basic;
mod body;
mod construct;
mod expr;
mod source;
mod types;

mod stmt;

use std::collections::HashMap;
use std::path::PathBuf;

use crate::model::*;

const INDENT_WIDTH: usize = 4;

/// Tracks what delimiter opened the current indent level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum IndentOpener {
    Brace,   // {
    Bracket, // [
    Paren,   // (
}

/// Parse a Duralade source file
pub fn parse_source_file(source: String, file_path: PathBuf) -> ParseResult {
    ParseContext::new(source, file_path).parse()
}

/// Result of parsing a source file
#[derive(Debug, Clone)]
pub struct ParseResult {
    pub source_file: SourceFile,
    pub parse_errors: Vec<ParseError>,
    pub strict_mode_violations: Vec<ParseError>,
}

/// Error information for a failed parse.
///
/// All ranges are byte offsets as `(start, end)` where end is **exclusive**
/// (like Rust's `Range<usize>`). A range of `(5, 8)` covers bytes 5, 6, 7.
/// Ranges must never be zero-width for display purposes — always point at
/// at least one byte/character.
#[derive(Debug, Clone)]
pub struct ParseError {
    /// The range of source bytes consumed while trying to parse (start..end, end-exclusive).
    pub consumed_range: (usize, usize),

    /// The error message at this level.
    pub message: String,

    /// The specific range to underline for display (start..end, end-exclusive).
    /// If None, `consumed_range` is used for display.
    pub error_range: Option<(usize, usize)>,

    /// The underlying cause (for nested errors).
    pub cause: Option<Box<ParseError>>,
}

impl ParseError {
    /// Create a new error
    pub fn new(consumed_range: (usize, usize), message: String) -> Self {
        Self {
            consumed_range,
            message,
            error_range: None,
            cause: None,
        }
    }

    /// Create an error with a specific error range
    pub fn with_range(
        consumed_range: (usize, usize),
        message: String,
        error_range: (usize, usize),
    ) -> Self {
        Self {
            consumed_range,
            message,
            error_range: Some(error_range),
            cause: None,
        }
    }

    /// Wrap this error with additional context
    pub fn wrap(self, consumed_range: (usize, usize), message: String) -> Self {
        Self {
            consumed_range,
            message,
            error_range: None,
            cause: Some(Box::new(self)),
        }
    }
}

// Parsing conventions:
//
// parse_xxx(ctx) -> Option<Xxx>
//   Errors are added to ctx.parse_errors. Invalid variants returned on failure.
//   Some = parsed (may be Invalid), None = not this construct (no input consumed).
//
// try_parse_xxx(ctx) -> Result<Option<Xxx>, ParseError>
//   Ok(Some) = parsed, Ok(None) = not this construct, Err = recognized but invalid.

/// Context for parsing Duralade source code.
/// Tracks position, allocates NodeIds, collects diagnostics.
pub(crate) struct ParseContext {
    /// The source code being parsed
    source: String,

    /// Current byte position in source
    pos: usize,

    /// File path for diagnostics
    file_path: String,

    /// Counter for allocating NodeIds
    next_node_id: u32,

    /// Maps NodeIds to (start, end) byte positions
    source_map: HashMap<NodeId, (usize, usize)>,

    /// Comments collected while parsing trivia
    comments: Vec<Comment>,

    /// Parse errors collected during parsing
    parse_errors: Vec<ParseError>,

    /// Strict mode violations collected during parsing
    strict_mode_violations: Vec<ParseError>,

    /// Stack of indent openers — length * INDENT_WIDTH gives expected indent level
    indent_stack: Vec<IndentOpener>,

    /// When true, expression postfix parsing skips bare `as type` and bare `?`
    /// narrowing forms (but still allows `as!` and `?!`). Used inside
    /// if-narrowing conditions so `as`/`?` are consumed by the condition parser.
    pub(crate) suppress_narrowing_postfix: bool,

    /// Set by parse_trivia when a blank line is encountered during the most
    /// recent call. Reset to false at the start of each parse_trivia call.
    pub(crate) trivia_had_blank_line: bool,

    /// Byte offset of the start of the current line (for line length checks)
    line_start: usize,
}

/// Snapshot of parse state for speculative parsing.
/// Captures position and error/violation/comment list lengths so they can
/// be restored if the speculative parse fails.
pub(crate) struct ParseSnapshot {
    pos: usize,
    next_node_id: u32,
    errors_len: usize,
    strict_len: usize,
    comments_len: usize,
    line_start: usize,
}

impl ParseContext {
    /// Create a new parse context
    fn new(source: String, file_path: PathBuf) -> Self {
        Self {
            source,
            pos: 0,
            file_path: file_path.to_string_lossy().to_string(),
            next_node_id: 0,
            source_map: HashMap::new(),
            comments: Vec::new(),
            parse_errors: Vec::new(),
            strict_mode_violations: Vec::new(),
            indent_stack: Vec::new(),
            suppress_narrowing_postfix: false,
            trivia_had_blank_line: false,
            line_start: 0,
        }
    }

    /// Get current position
    pub fn prev_byte(&self) -> Option<u8> {
        if self.pos > 0 { Some(self.source.as_bytes()[self.pos - 1]) } else { None }
    }


    pub fn pos(&self) -> usize {
        self.pos
    }

    /// Get the remaining source from current position
    pub fn remaining(&self) -> &str {
        &self.source[self.pos..]
    }

    /// Byte length of the next character at current position (0 if at end).
    /// Useful for building end-exclusive ranges that cover one character.
    pub fn next_char_len(&self) -> usize {
        self.remaining().chars().next().map_or(0, |c| c.len_utf8())
    }

    /// Create a spacing strict violation. `space_start` is the position before
    /// `skip_spaces()`, and current position is after. If zero spaces were found,
    /// points at the next character (where the space should be).
    pub fn spacing_error(&self, space_start: usize, message: String) -> ParseError {
        if self.pos == space_start {
            // Zero spaces: point at the character that should have been a space
            ParseError::new((space_start, space_start + self.next_char_len()), message)
        } else {
            // Too many spaces: point at the extra whitespace
            ParseError::new((space_start, self.pos), message)
        }
    }

    /// Create a ParseError pointing at the current character (single char range).
    /// If at a newline or EOF, points at the last character on the current line.
    pub fn error_here(&self, message: String) -> ParseError {
        let at_visible = self.pos < self.source.len() && self.source.as_bytes()[self.pos] != b'\n';
        let (start, end) = if at_visible {
            (self.pos, self.pos + self.next_char_len())
        } else if self.pos > 0 {
            // Back up to the previous character
            let prev_start = self.source[..self.pos]
                .char_indices()
                .next_back()
                .map(|(i, _)| i)
                .unwrap_or(0);
            (prev_start, self.pos)
        } else {
            (0, 1)
        };
        ParseError::new((start, end), message)
    }

    /// Get the (start, end) byte span for a node ID.
    pub fn node_span(&self, id: NodeId) -> (usize, usize) {
        *self.source_map.get(&id).expect("NodeId not in source_map")
    }

    /// Advance position by n bytes
    pub fn advance(&mut self, n: usize) {
        self.pos += n;
    }

    /// Called just before consuming a `\n`. Checks line length and updates line_start.
    fn on_newline(&mut self) {
        let line_end =
            if self.pos > self.line_start && self.source.as_bytes()[self.pos - 1] == b'\r' {
                self.pos - 1
            } else {
                self.pos
            };
        if line_end - self.line_start > 120 {
            let line = &self.source[self.line_start..line_end];
            let content = line.trim_start_matches(' ');
            let content = content.strip_prefix("# ").unwrap_or(content);
            // Only warn if content has a space (i.e. a natural break point exists)
            if content.contains(' ') {
                self.add_strict_violation(ParseError::new(
                    (self.line_start + 120, self.line_start + 121),
                    "Line exceeds 120 characters".to_string(),
                ));
            }
        }
        self.line_start = self.pos + 1;
    }

    /// Update line_start after consuming a multi-line token (e.g. raw string).
    /// Sets line_start to the start of the last line within the consumed range.
    pub fn sync_line_start(&mut self) {
        if let Some(last_nl) = self.source[..self.pos].rfind('\n') {
            self.line_start = last_nl + 1;
        }
    }

    /// Set position (for rollback after failed speculative parse)
    pub fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Consume only spaces (not newlines or comments). Returns count consumed.
    pub fn skip_spaces(&mut self) -> usize {
        let count = self
            .remaining()
            .find(|c: char| c != ' ')
            .unwrap_or(self.remaining().len());
        self.advance(count);
        count
    }

    /// Allocate a NodeId with the given span
    pub fn alloc_node_id(&mut self, start: usize, end: usize) -> NodeId {
        let id = NodeId(self.next_node_id);
        self.next_node_id += 1;
        self.source_map.insert(id, (start, end));
        id
    }

    /// Add a parse error
    pub fn add_parse_error(&mut self, error: ParseError) {
        self.parse_errors.push(error);
    }

    /// Add a strict mode violation
    pub fn add_strict_violation(&mut self, error: ParseError) {
        self.strict_mode_violations.push(error);
    }

    /// Save a snapshot for speculative parsing (e.g. line continuation after
    /// a binary operator). If the speculation fails, call `restore` to undo
    /// position changes and any errors/violations/comments added since.
    pub fn snapshot(&self) -> ParseSnapshot {
        ParseSnapshot {
            pos: self.pos,
            next_node_id: self.next_node_id,
            errors_len: self.parse_errors.len(),
            strict_len: self.strict_mode_violations.len(),
            comments_len: self.comments.len(),
            line_start: self.line_start,
        }
    }

    /// Restore state from a snapshot, discarding everything added since.
    pub fn restore(&mut self, snap: ParseSnapshot) {
        self.pos = snap.pos;
        // Remove source_map entries for NodeIds allocated during speculation
        for id in snap.next_node_id..self.next_node_id {
            self.source_map.remove(&NodeId(id));
        }
        self.next_node_id = snap.next_node_id;
        self.parse_errors.truncate(snap.errors_len);
        self.strict_mode_violations.truncate(snap.strict_len);
        self.comments.truncate(snap.comments_len);
        self.line_start = snap.line_start;
    }

    /// Current expected indentation level (in spaces)
    fn current_indent(&self) -> usize {
        self.indent_stack.len() * INDENT_WIDTH
    }

    /// Increase expected indentation by one level
    pub fn indent(&mut self, opener: IndentOpener) {
        self.indent_stack.push(opener);
    }

    /// Decrease expected indentation by one level
    pub fn dedent(&mut self) {
        self.indent_stack.pop();
    }

    /// Check if we're at a newline (handles both \n and \r\n)
    pub fn is_at_newline(&self) -> bool {
        self.remaining().starts_with('\n') || self.remaining().starts_with("\r\n")
    }

    /// Strict: check that a space or newline follows after a comma.
    pub fn check_comma_spacing(&mut self) {
        if !self.remaining().starts_with(' ')
            && !self.remaining().starts_with('\n')
            && !self.remaining().starts_with("\r\n")
        {
            self.add_strict_violation(self.spacing_error(
                self.pos(),
                "Expected space or newline after ','".to_string(),
            ));
        }
    }

    /// Check whether source between two byte positions contains a newline.
    pub fn is_multiline(&self, from: usize, to: usize) -> bool {
        self.source[from..to].contains('\n')
    }

    /// Check if we're at the start of a line (beginning of file or just after a newline)
    pub fn is_at_line_start(&self) -> bool {
        self.pos == 0 || self.source.as_bytes().get(self.pos - 1) == Some(&b'\n')
    }

    /// Check that the current position is at a valid statement boundary (newline, `}`, or EOF).
    /// If not, report an error and skip to end of line.
    pub fn check_stmt_termination(&mut self) {
        if !self.remaining().is_empty() && !self.is_at_newline() && !self.is_at_line_start() {
            let trimmed = self.remaining().trim_start_matches(' ');
            if !trimmed.starts_with('}') && !trimmed.starts_with('#') {
                self.add_parse_error(
                    self.error_here("Expected newline or '}' after statement".to_string()),
                );
                self.skip_past_end_of_line();
            }
        }
    }

    /// Skip past end of current line (for error recovery). Does nothing if already past line end.
    pub fn skip_past_end_of_line(&mut self) {
        // If at start of a line (already consumed the newline) or EOF, don't skip further
        if self.is_at_line_start() || self.remaining().is_empty() {
            return;
        }
        // Skip past the newline
        if let Some(newline_pos) = self.remaining().find('\n') {
            self.advance(newline_pos);
            self.on_newline();
            self.advance(1);
        } else {
            // No newline found, skip to end
            self.pos = self.source.len();
        }
    }

    /// Parse a qualified identifier (dot-separated identifiers)
    /// Returns at least one identifier
    pub fn parse_qualified_ident(&mut self) -> Result<Vec<Ident>, ParseError> {
        let start_pos = self.pos();
        let mut idents = Vec::new();

        loop {
            let ident_start = self.pos();

            // Parse identifier
            match basic::try_parse_ident(self) {
                Ok(Some(ident)) => idents.push(ident),
                Ok(None) => {
                    let char_end = ident_start + self.next_char_len();
                    return Err(ParseError::with_range(
                        (start_pos, self.pos()),
                        "Expected identifier".to_string(),
                        (ident_start, char_end),
                    ));
                }
                Err(err) => {
                    return Err(err);
                }
            }

            // Check for '.'
            if !self.remaining().starts_with('.') {
                break;
            }
            self.advance(1);
        }

        Ok(idents)
    }

    /// Parse trivia (whitespace, newlines, comments) with strict validation
    /// Returns the slice of what was skipped
    pub fn parse_trivia(&mut self) -> &str {
        let trivia_start = self.pos;
        self.trivia_had_blank_line = false;
        let mut last_was_blank_line = false;
        let mut blank_line_error_reported = false;
        let mut last_line_was_comment = false;

        loop {
            // Check if we're at the start of a line
            let at_line_start =
                self.pos == 0 || self.source.as_bytes().get(self.pos - 1) == Some(&b'\n');

            // Count and consume spaces
            let space_start = self.pos;
            let space_count = self
                .remaining()
                .find(|c: char| c != ' ')
                .unwrap_or(self.remaining().len());
            self.advance(space_count);

            // Normalize Windows line endings: skip \r if followed by \n
            if self.remaining().starts_with("\r\n") {
                self.advance(1); // Skip \r, let match handle \n
            }

            // Handle what follows the spaces
            match self.remaining().chars().next() {
                // Line ending (newline)
                Some('\n') => {
                    // Check for trailing whitespace
                    if space_count > 0 {
                        self.add_strict_violation(ParseError::new(
                            (space_start, self.pos),
                            "Trailing whitespace".to_string(),
                        ));
                    }
                    // Track blank lines and end comment blocks
                    if at_line_start {
                        last_line_was_comment = false;
                        if last_was_blank_line && !blank_line_error_reported {
                            self.add_strict_violation(ParseError::new(
                                (self.pos, self.pos + 1),
                                "Multiple consecutive blank lines".to_string(),
                            ));
                            blank_line_error_reported = true;
                        }
                        last_was_blank_line = true;
                        self.trivia_had_blank_line = true;
                    } else {
                        last_was_blank_line = false;
                        blank_line_error_reported = false;
                    }
                    self.on_newline();
                    self.advance(1);
                }
                // Invalid whitespace (tabs, \r, etc.)
                Some(ch) if ch.is_whitespace() => {
                    // Consume all consecutive invalid whitespace (optimized)
                    let invalid_start = self.pos;
                    let invalid_count = self
                        .remaining()
                        .find(|c: char| !c.is_whitespace() || c == '\n')
                        .unwrap_or(self.remaining().len());
                    self.advance(invalid_count);
                    self.add_parse_error(ParseError::new(
                        (invalid_start, self.pos),
                        "Invalid whitespace characters".to_string(),
                    ));
                }
                // End of file
                None => {
                    // Check for trailing whitespace at EOF
                    if space_count > 0 {
                        self.add_strict_violation(ParseError::new(
                            (space_start, self.pos),
                            "Trailing whitespace at end of file".to_string(),
                        ));
                    }
                    break;
                }
                // Comment line
                Some('#') => {
                    // Validate comment position and indentation
                    if !at_line_start {
                        self.add_strict_violation(ParseError::new(
                            (self.pos, self.pos + 1),
                            "Comment must be on its own line".to_string(),
                        ));
                    }
                    if at_line_start && space_count != self.current_indent() {
                        let range = if space_count == 0 {
                            (self.pos, self.pos + 1)
                        } else {
                            (space_start, self.pos)
                        };
                        self.add_strict_violation(ParseError::new(
                            range,
                            format!(
                                "Expected {} spaces, found {}",
                                self.current_indent(),
                                space_count
                            ),
                        ));
                    }

                    let line_start = self.pos;
                    self.advance(1); // Consume '#'

                    // Check for space after '#' (unless empty comment)
                    if !self.remaining().is_empty() && !self.is_at_newline() {
                        if self.remaining().starts_with(' ') {
                            self.advance(1);
                        } else {
                            self.add_strict_violation(ParseError::new(
                                (line_start, line_start + 1),
                                "Space required after '#'".to_string(),
                            ));
                        }
                    }

                    // Read comment text until newline/EOF (find \n which exists in both Unix and Windows)
                    let text_len = self
                        .remaining()
                        .find('\n')
                        .unwrap_or(self.remaining().len());
                    let text_without_cr = self.remaining()[..text_len].trim_end_matches('\r');
                    let trimmed_len = text_without_cr.trim_end().len();
                    let text_without_cr_len = text_without_cr.len();
                    let trimmed_text = text_without_cr.trim_end().to_string();

                    // Check for trailing whitespace in comment
                    if trimmed_len < text_without_cr_len {
                        self.add_strict_violation(ParseError::new(
                            (self.pos + trimmed_len, self.pos + text_without_cr_len),
                            "Trailing whitespace".to_string(),
                        ));
                    }

                    self.advance(text_len);
                    let comment_line = CommentLine {
                        node_id: self.alloc_node_id(line_start, self.pos),
                        text: trimmed_text,
                    };

                    // Append to existing comment block or create new one
                    if last_line_was_comment {
                        if let Some(last_comment) = self.comments.last_mut() {
                            if let Some(span) = self.source_map.get_mut(&last_comment.node_id) {
                                span.1 = self.pos;
                            }
                            last_comment.lines.push(comment_line);
                        }
                    } else {
                        let comment_node_id = self.alloc_node_id(line_start, self.pos);
                        self.comments.push(Comment {
                            node_id: comment_node_id,
                            lines: vec![comment_line],
                        });
                    }

                    last_line_was_comment = true;
                    last_was_blank_line = false;
                    blank_line_error_reported = false;
                }
                // Non-trivia content
                _ => {
                    // Patch directives (`%`) have their own column-0 rule
                    if at_line_start && !self.remaining().starts_with('%') {
                        // Check if content is a matching closer for the current opener.
                        // A closer at current_indent is regular content (e.g. `}` for an
                        // if-block inside a func body). Anything NOT at current_indent
                        // that matches the stack top is a (possibly misindented) closer.
                        // A closer char that doesn't match the stack top (e.g. `}` when
                        // top is Bracket) means a missing closer — skip indent check.
                        let outer_indent = self.current_indent().saturating_sub(INDENT_WIDTH);
                        let (matches_top, is_closer_char) =
                            match self.remaining().as_bytes().first() {
                                Some(b'}') => (
                                    self.indent_stack.last() == Some(&IndentOpener::Brace),
                                    true,
                                ),
                                Some(b']') => (
                                    self.indent_stack.last() == Some(&IndentOpener::Bracket),
                                    true,
                                ),
                                Some(b')') => (
                                    self.indent_stack.last() == Some(&IndentOpener::Paren),
                                    true,
                                ),
                                _ => (false, false),
                            };
                        let is_matching_closer =
                            matches_top && space_count != self.current_indent();
                        let expected = if is_matching_closer {
                            outer_indent
                        } else {
                            self.current_indent()
                        };
                        // Don't report indent errors for mismatched closers (e.g. `}`
                        // when top is Bracket) — the caller will report the missing closer.
                        if space_count != expected && !(is_closer_char && !matches_top) {
                            let range = if space_count == 0 {
                                (self.pos, (self.pos + 1).min(self.source.len()))
                            } else {
                                (space_start, self.pos)
                            };
                            self.add_strict_violation(ParseError::new(
                                range,
                                format!("Expected {} spaces, found {}", expected, space_count),
                            ));
                        }
                    }
                    break;
                }
            }
        }

        &self.source[trivia_start..self.pos]
    }

    /// Strict mode: check ordering of imports and constructs.
    /// Pass `&[]` for imports when checking nested (member) constructs.
    pub fn assert_ordering(&mut self, imports: &[Import], constructs: &[Construct]) {
        // Imports must be alphabetical by module path (ASCII, segment-by-segment).
        // Track max seen; compare against max to catch all violations.
        let mut max_i = 0;
        for i in 1..imports.len() {
            if import_path_cmp(&imports[i], &imports[max_i]) == std::cmp::Ordering::Less {
                // Find the closest earlier import this one should appear before
                // (smallest key still greater than current)
                let target = imports[..i]
                    .iter()
                    .filter(|x| import_path_cmp(&imports[i], x) == std::cmp::Ordering::Less)
                    .min_by(|a, b| import_path_cmp(a, b))
                    .unwrap();
                self.add_strict_violation(ParseError::new(
                    self.node_span(imports[i].node_id),
                    format!(
                        "Import '{}' should appear before '{}'",
                        import_path_display(&imports[i]),
                        import_path_display(target),
                    ),
                ));
            } else {
                max_i = i;
            }
        }

        // Constructs must be ordered by (kind, out-first, name alphabetical).
        let mut max_ci: Option<usize> = None;
        for i in 0..constructs.len() {
            if matches!(constructs[i].kind, ConstructKind::Invalid(_)) {
                continue;
            }
            let key = construct_sort_key(&constructs[i]);
            if let Some(mi) = max_ci
                && key < construct_sort_key(&constructs[mi])
            {
                let target = constructs[..i]
                    .iter()
                    .filter(|x| !matches!(x.kind, ConstructKind::Invalid(_)))
                    .filter(|x| key < construct_sort_key(x))
                    .min_by(|a, b| construct_sort_key(a).cmp(&construct_sort_key(b)))
                    .unwrap();
                self.add_strict_violation(ParseError::new(
                    self.node_span(constructs[i].node_id),
                    format!(
                        "'{}' should appear before '{}'",
                        construct_label(&constructs[i]),
                        construct_label(target),
                    ),
                ));
            } else {
                max_ci = Some(i);
            }
        }
    }

    /// Strict mode: check field ordering within a construct body.
    /// Fields are ordered by (group_rank, name) where group_rank is:
    /// intype=0, in(no default)=1, in(with default)=2,
    /// inout(no default)=3, inout(with default)=4, out=5, out!=6, implicit=7, value=8.
    pub fn assert_field_ordering(&mut self, fields: &[Field]) {
        let mut max_fi: Option<usize> = None;
        for i in 0..fields.len() {
            if matches!(fields[i].kind, FieldKind::Invalid(_)) {
                continue;
            }
            let key = field_sort_key(&fields[i]);
            if let Some(mi) = max_fi
                && key < field_sort_key(&fields[mi])
            {
                let target = fields[..i]
                    .iter()
                    .filter(|x| !matches!(x.kind, FieldKind::Invalid(_)))
                    .filter(|x| key < field_sort_key(x))
                    .min_by(|a, b| field_sort_key(a).cmp(&field_sort_key(b)))
                    .unwrap();
                self.add_strict_violation(ParseError::new(
                    self.node_span(fields[i].node_id),
                    format!(
                        "'{}' should appear before '{}'",
                        field_label(&fields[i]),
                        field_label(target),
                    ),
                ));
            } else {
                max_fi = Some(i);
            }
        }
    }

    /// Parse the source file into an AST
    fn parse(mut self) -> ParseResult {
        let start_pos = self.pos();

        let mut ann_groups = source::parse_annotations(&mut self);
        let mut source_anns: Option<Vec<Vec<Annotation>>> = None;

        // Parse imports
        let mut imports = Vec::new();
        let mut had_blank_after_last_import = false;
        loop {
            self.parse_trivia();
            match source::try_parse_import(&mut self) {
                Ok(Some(import)) => {
                    imports.push(import);
                    // Capture blank-line flag now — the import parser's internal
                    // parse_trivia (for "as" check) may have consumed it.
                    had_blank_after_last_import = self.trivia_had_blank_line;
                }
                Ok(None) => break,
                Err(err) => self.add_parse_error(err),
            }
        }

        // If imports found, all pending annotations are module-level
        if !imports.is_empty() {
            source_anns = Some(std::mem::take(&mut ann_groups));
        }

        // Parse constructs
        let mut constructs = Vec::new();
        let mut first_construct = true;
        loop {
            self.parse_trivia();
            if self.remaining().is_empty() {
                break;
            }
            // Strict: blank line between imports and first construct (Spec 6.1)
            if first_construct && !imports.is_empty() && !had_blank_after_last_import {
                self.add_strict_violation(ParseError::new(
                    (self.pos(), self.pos() + 1),
                    "Required blank line between imports and first construct".to_string(),
                ));
            }
            // Strict: blank line between each file construct (Spec 6.1)
            if !first_construct && !self.trivia_had_blank_line {
                self.add_strict_violation(ParseError::new(
                    (self.pos(), self.pos() + 1),
                    "Required blank line between constructs".to_string(),
                ));
            }
            let already_parsed = if first_construct {
                // If no source-file annotations yet, split: source-level gets all but the
                // last non-empty group. A trailing empty group (blank-line signal) also goes
                // to source_anns so the blank-line-after check works.
                if source_anns.is_none() && ann_groups.len() > 1 {
                    let non_empty_count = ann_groups.iter().filter(|g| !g.is_empty()).count();
                    if non_empty_count > 1 {
                        // Multiple annotation groups: all but last non-empty go to source
                        let split = ann_groups.len()
                            - 1
                            - ann_groups.iter().rev().position(|g| !g.is_empty()).unwrap();
                        source_anns = Some(ann_groups.drain(..split).collect());
                    } else {
                        // Single annotation group + blank line signal: all go to source
                        source_anns = Some(std::mem::take(&mut ann_groups));
                    }
                }
                // Remaining groups go to first construct
                Some(
                    std::mem::take(&mut ann_groups)
                        .into_iter()
                        .flatten()
                        .collect(),
                )
            } else {
                None
            };
            constructs.push(construct::parse_construct(&mut self, already_parsed));
            first_construct = false;
        }

        // Annotation-only file: all remaining become source-level
        let source_anns = source_anns.unwrap_or(ann_groups);

        // Strict: module-level annotations must be a single contiguous group
        // (trailing empty group is the blank-line-after signal, not a violation)
        if source_anns.iter().filter(|g| !g.is_empty()).count() > 1 {
            let ann = source_anns
                .iter()
                .filter(|g| !g.is_empty())
                .nth(1)
                .and_then(|g| g.first())
                .unwrap();
            self.add_strict_violation(ParseError::new(
                self.node_span(ann.node_id),
                "No blank lines allowed between module-level annotations".to_string(),
            ));
        }

        // Strict: required blank line after module-level annotations (Spec 6.1)
        let has_trailing_blank = source_anns.last().is_some_and(|g| g.is_empty());
        let annotations: Vec<Annotation> = source_anns.into_iter().flatten().collect();
        if !annotations.is_empty() && !has_trailing_blank {
            self.add_strict_violation(ParseError::new(
                self.node_span(annotations.last().unwrap().node_id),
                "Required blank line after module-level annotations".to_string(),
            ));
        }

        self.assert_ordering(&imports, &constructs);

        // Check if file ends with newline (strict mode)
        if !self.source.is_empty() && !self.source.ends_with('\n') {
            let end = self.source.len();
            let char_len = self.source.chars().last().map_or(1, |c| c.len_utf8());
            self.add_strict_violation(ParseError::new(
                (end - char_len, end),
                "File must end with a newline".to_string(),
            ));
        }

        ParseResult {
            source_file: SourceFile {
                node_id: self.alloc_node_id(start_pos, self.pos()),
                file_path: self.file_path,
                source_map: self.source_map,
                comments: self.comments,
                annotations,
                imports,
                constructs,
            },
            parse_errors: self.parse_errors,
            strict_mode_violations: self.strict_mode_violations,
        }
    }
}

/// Compare two imports by module path (ASCII, segment-by-segment).
fn import_path_cmp(a: &Import, b: &Import) -> std::cmp::Ordering {
    a.path
        .iter()
        .zip(b.path.iter())
        .find_map(|(x, y)| {
            let cmp = x.name.cmp(&y.name);
            if cmp != std::cmp::Ordering::Equal {
                Some(cmp)
            } else {
                None
            }
        })
        .unwrap_or_else(|| a.path.len().cmp(&b.path.len()))
}

fn import_path_display(imp: &Import) -> String {
    imp.path
        .iter()
        .map(|i| i.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
}

fn construct_sort_key(c: &Construct) -> (u8, u8, &str) {
    let kind_rank = match &c.kind {
        ConstructKind::TypeAlias(_) => 0,
        ConstructKind::Data(_) => 1,
        ConstructKind::Entity(_) => 2,
        ConstructKind::Func(_) => 3,
        ConstructKind::Extern(_) => 4,
        ConstructKind::Native(_) => 5,
        ConstructKind::Invalid(_) => 255,
    };
    let out_rank = if c.out { 0 } else { 1 };
    (kind_rank, out_rank, &c.name.name)
}

fn construct_label(c: &Construct) -> String {
    let keyword = match &c.kind {
        ConstructKind::TypeAlias(_) => "type",
        ConstructKind::Data(_) => "data",
        ConstructKind::Entity(_) => "entity",
        ConstructKind::Func(_) => "func",
        ConstructKind::Extern(_) => "extern",
        ConstructKind::Native(_) => "native",
        ConstructKind::Invalid(_) => "?",
    };
    if c.out {
        format!("out {} {}", keyword, c.name.name)
    } else {
        format!("{} {}", keyword, c.name.name)
    }
}

/// Returns (group_rank, name) for field ordering. Caller must filter out invalid fields.
fn field_sort_key(f: &Field) -> (u8, &str) {
    match &f.kind {
        FieldKind::Intype(ft) => (0, &ft.name.name),
        FieldKind::Var(fv) => {
            let rank = match fv.modifier {
                FieldVarModifier::In => {
                    if fv.default.is_some() {
                        2
                    } else {
                        1
                    }
                }
                FieldVarModifier::Inout => {
                    if fv.default.is_some() {
                        4
                    } else {
                        3
                    }
                }
                FieldVarModifier::Out => 5,
                FieldVarModifier::OutEarly => 6,
                FieldVarModifier::Implicit => 7,
                FieldVarModifier::Value => 8,
            };
            (rank, &fv.name.name)
        }
        FieldKind::Invalid(_) => unreachable!(),
    }
}

fn field_label(f: &Field) -> String {
    match &f.kind {
        FieldKind::Intype(ft) => format!("intype {}", ft.name.name),
        FieldKind::Var(fv) => {
            let modifier = match fv.modifier {
                FieldVarModifier::In => "in",
                FieldVarModifier::Inout => "inout",
                FieldVarModifier::Out => "out",
                FieldVarModifier::OutEarly => "out!",
                FieldVarModifier::Implicit => "implicit",
                FieldVarModifier::Value => "value",
            };
            format!("{} {}", modifier, fv.name.name)
        }
        FieldKind::Invalid(_) => "?".to_string(),
    }
}
