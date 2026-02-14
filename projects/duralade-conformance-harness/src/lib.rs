use duralade_language::parse::parse_source_file;
use std::path::{Path, PathBuf};

/// Type of error expected in a test
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExpectedErrorType {
    ParseError,      // @@ERROR:
    StrictViolation, // @@STRICT:
}

/// An error expected in a test file
#[derive(Debug, Clone)]
pub struct ExpectedError {
    pub line: usize,      // 1-indexed (in cleaned source)
    pub col_start: usize, // 0-indexed column of first ^
    pub col_end: usize,   // 0-indexed exclusive column after last ^
    pub error_type: ExpectedErrorType,
    pub message_substring: String,
}

/// Result of parsing a test file
#[derive(Debug)]
pub struct TestFile {
    pub cleaned_source: String,
    pub expected_errors: Vec<ExpectedError>,
}

/// Result of running a test
#[derive(Debug)]
pub struct TestResult {
    pub passed: bool,
    pub diagnostics: Vec<String>,
}

/// Parse test file, extract expected errors and produce cleaned source.
///
/// Annotation lines are standalone lines containing `@@ERROR:` or `@@STRICT:`,
/// with `^` carets marking the column range on the preceding code line.
///
/// Example:
/// ```text
///     var a = [1, 2
///             ^ @@ERROR: Expected ']'
/// ```
pub fn parse_test_file(source: &str) -> Result<TestFile, String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut cleaned_lines: Vec<&str> = Vec::new();
    let mut expected_errors = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        let orig_line_num = line_idx + 1;

        if let Some(marker_pos) = line.find("@@") {
            // This is an annotation line — parse it and don't add to cleaned source.

            // Parse marker type and message
            let marker = &line[marker_pos + 2..];
            let (error_type, message) = if let Some(msg) = marker.strip_prefix("ERROR:") {
                (ExpectedErrorType::ParseError, msg.trim())
            } else if let Some(msg) = marker.strip_prefix("STRICT:") {
                (ExpectedErrorType::StrictViolation, msg.trim())
            } else {
                return Err(format!(
                    "Line {}: Invalid marker (expected @@ERROR: or @@STRICT:)",
                    orig_line_num
                ));
            };

            // Parse caret range from before @@
            let before_marker = &line[..marker_pos];
            // Strip trailing whitespace between carets and @@
            let before_trimmed = before_marker.trim_end();

            // Find caret range
            let first_caret = before_trimmed.find('^');
            let last_caret = before_trimmed.rfind('^');

            match (first_caret, last_caret) {
                (Some(first), Some(last)) => {
                    // Validate: everything before first caret is spaces
                    if before_trimmed[..first].chars().any(|c| c != ' ') {
                        return Err(format!(
                            "Line {}: Only spaces allowed before '^' carets",
                            orig_line_num
                        ));
                    }
                    // Validate: everything between first and last caret is carets
                    if before_trimmed[first..=last].chars().any(|c| c != '^') {
                        return Err(format!("Line {}: Carets must be contiguous", orig_line_num));
                    }

                    let col_start = first;
                    let col_end = last + 1;

                    // Target is the last cleaned line
                    if cleaned_lines.is_empty() {
                        return Err(format!(
                            "Line {}: Annotation has no preceding code line",
                            orig_line_num
                        ));
                    }
                    let target_line = cleaned_lines.len(); // 1-indexed

                    expected_errors.push(ExpectedError {
                        line: target_line,
                        col_start,
                        col_end,
                        error_type,
                        message_substring: message.to_string(),
                    });
                }
                _ => {
                    return Err(format!(
                        "Line {}: Annotation line must have '^' carets marking the error range",
                        orig_line_num
                    ));
                }
            }
        } else {
            cleaned_lines.push(line);
        }
    }

    let mut cleaned_source = cleaned_lines.join("\n");
    // Preserve trailing newline if original had one
    if source.ends_with('\n') {
        cleaned_source.push('\n');
    }

    Ok(TestFile {
        cleaned_source,
        expected_errors,
    })
}

/// Check if message substring appears anywhere in error's cause chain
fn error_contains_message(err: &duralade_language::parse::ParseError, substring: &str) -> bool {
    if err.message.contains(substring) {
        return true;
    }
    if let Some(cause) = &err.cause {
        return error_contains_message(cause, substring);
    }
    false
}

/// Get the display range for an error. Walks the cause chain to find the most
/// specific error_range, falling back to the outermost consumed_range.
fn error_display_range(err: &duralade_language::parse::ParseError) -> (usize, usize) {
    if let Some(range) = err.error_range {
        return range;
    }
    if let Some(cause) = &err.cause {
        let inner = error_display_range(cause);
        if inner != cause.consumed_range || cause.error_range.is_some() {
            return inner;
        }
    }
    err.consumed_range
}

/// Run test file through parser and validate results
pub fn run_test(test_file: TestFile, file_path: PathBuf) -> TestResult {
    let parse_result = parse_source_file(test_file.cleaned_source.clone(), file_path);

    let source = &test_file.cleaned_source;

    // Helper: byte position to 1-indexed line number
    let line_num = |pos: usize| {
        let clamped = pos.min(source.len());
        source[..clamped].chars().filter(|&c| c == '\n').count() + 1
    };

    // Helper: byte position to 0-indexed column.
    // If pos falls on a '\n', returns the column of the '\n' on the current line
    // (i.e. the line length), NOT column 0 of the next line.
    let col_num = |pos: usize| {
        let clamped = pos.min(source.len());
        let line_start = source[..clamped].rfind('\n').map(|p| p + 1).unwrap_or(0);
        clamped - line_start
    };

    // Helper: line and column range for an error's display range.
    // Clamps end to the same line as start to avoid cross-line column confusion.
    // For ranges that start on a newline (e.g. blank line errors), ensures
    // at least width 1 so the error is displayable.
    let line_col_range = |range: (usize, usize)| {
        let start_line = line_num(range.0);
        let start_col = col_num(range.0);
        // Find the end of the start line
        let line_end = source[range.0..]
            .find('\n')
            .map(|p| range.0 + p)
            .unwrap_or(source.len());
        let clamped_end = range.1.min(line_end);
        let mut end_col = col_num(clamped_end);
        // Ensure non-empty ranges don't collapse to zero width after clamping
        if end_col == start_col && range.0 != range.1 {
            end_col = start_col + 1;
        }
        (start_line, start_col, end_col)
    };

    // Collect actual errors with their display ranges
    struct ActualError<'a> {
        line: usize,
        col_start: usize,
        col_end: usize,
        error_type: ExpectedErrorType,
        err: &'a duralade_language::parse::ParseError,
    }

    let mut actual_errors: Vec<ActualError> = Vec::new();

    for err in &parse_result.parse_errors {
        let range = error_display_range(err);
        let (line, col_start, col_end) = line_col_range(range);
        actual_errors.push(ActualError {
            line,
            col_start,
            col_end,
            error_type: ExpectedErrorType::ParseError,
            err,
        });
    }

    for err in &parse_result.strict_mode_violations {
        let range = error_display_range(err);
        let (line, col_start, col_end) = line_col_range(range);
        actual_errors.push(ActualError {
            line,
            col_start,
            col_end,
            error_type: ExpectedErrorType::StrictViolation,
            err,
        });
    }

    // Match expected against actual
    let mut diagnostics = Vec::new();
    let mut matched = vec![false; actual_errors.len()];

    for expected in &test_file.expected_errors {
        let found = actual_errors.iter().enumerate().any(|(i, actual)| {
            !matched[i]
                && actual.line == expected.line
                && actual.col_start == expected.col_start
                && actual.col_end == expected.col_end
                && actual.error_type == expected.error_type
                && error_contains_message(actual.err, &expected.message_substring)
                && {
                    matched[i] = true;
                    true
                }
        });

        if !found {
            let typ = match expected.error_type {
                ExpectedErrorType::ParseError => "ERROR",
                ExpectedErrorType::StrictViolation => "STRICT",
            };
            diagnostics.push(format!(
                "Line {}:{}-{}: Expected {}: \"{}\" - NOT FOUND",
                expected.line,
                expected.col_start,
                expected.col_end,
                typ,
                expected.message_substring
            ));
        }
    }

    // Report unmatched actual errors
    for (i, actual) in actual_errors.iter().enumerate() {
        if !matched[i] {
            let typ_str = match actual.error_type {
                ExpectedErrorType::ParseError => "ERROR",
                ExpectedErrorType::StrictViolation => "STRICT",
            };
            diagnostics.push(format!(
                "Line {}:{}-{}: Unexpected {}: \"{}\"",
                actual.line, actual.col_start, actual.col_end, typ_str, actual.err.message
            ));
        }
    }

    TestResult {
        passed: diagnostics.is_empty(),
        diagnostics,
    }
}

/// Run test from file path
pub fn run_test_file(path: &Path) -> Result<TestResult, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let test_file = parse_test_file(&source)?;
    Ok(run_test(test_file, path.to_path_buf()))
}
