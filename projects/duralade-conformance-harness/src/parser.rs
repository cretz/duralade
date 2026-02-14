use duralade_language::parse::parse_source_file;
use std::path::{Path, PathBuf};

use crate::annotations;

#[derive(Debug)]
pub struct TestResult {
    pub passed: bool,
    pub diagnostics: Vec<String>,
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

fn run_test(test_file: annotations::TestFile, file_path: PathBuf) -> TestResult {
    let parse_result = parse_source_file(test_file.cleaned_source.clone(), file_path, false);

    let source = &test_file.cleaned_source;

    // Clamp end to same line as start to avoid cross-line column confusion.
    // For ranges that start on a newline, ensures at least width 1.
    let line_col_range = |range: (usize, usize)| {
        let start_line = annotations::line_num(source, range.0);
        let start_col = annotations::col_num(source, range.0);
        let line_end = source[range.0..]
            .find('\n')
            .map(|p| range.0 + p)
            .unwrap_or(source.len());
        let clamped_end = range.1.min(line_end);
        let mut end_col = annotations::col_num(source, clamped_end);
        if end_col == start_col && range.0 != range.1 {
            end_col = start_col + 1;
        }
        (start_line, start_col, end_col)
    };

    struct ActualError<'a> {
        line: usize,
        col_start: usize,
        col_end: usize,
        tag: &'static str,
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
            tag: "ERROR",
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
            tag: "STRICT",
            err,
        });
    }

    // Match expected against actual
    let mut diagnostics = Vec::new();
    let mut matched = vec![false; actual_errors.len()];

    for expected in &test_file.annotations {
        let found = actual_errors.iter().enumerate().any(|(i, actual)| {
            !matched[i]
                && actual.line == expected.line
                && actual.col_start == expected.col_start
                && actual.col_end == expected.col_end
                && actual.tag == expected.tag
                && error_contains_message(actual.err, &expected.message_substring)
                && {
                    matched[i] = true;
                    true
                }
        });

        if !found {
            diagnostics.push(format!(
                "Line {}:{}-{}: Expected {}: \"{}\" - NOT FOUND",
                expected.line,
                expected.col_start,
                expected.col_end,
                expected.tag,
                expected.message_substring
            ));
        }
    }

    for (i, actual) in actual_errors.iter().enumerate() {
        if !matched[i] {
            diagnostics.push(format!(
                "Line {}:{}-{}: Unexpected {}: \"{}\"",
                actual.line, actual.col_start, actual.col_end, actual.tag, actual.err.message
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
    let test_file = annotations::parse_test_file(&source, &["ERROR", "STRICT"])?;
    Ok(run_test(test_file, path.to_path_buf()))
}
