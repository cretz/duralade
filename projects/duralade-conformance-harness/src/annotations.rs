/// Shared annotation parsing for conformance test harnesses.
///
/// Test files use `@@TAG: message` annotations with `^^^` carets to mark
/// expected error locations. This module extracts those annotations and
/// produces a "cleaned" source with annotation lines removed.

#[derive(Debug, Clone)]
pub struct Annotation {
    /// Tag string, e.g. "ERROR", "TYPE"
    pub tag: String,
    /// 1-indexed line in cleaned source
    pub line: usize,
    /// 0-indexed column of first ^
    pub col_start: usize,
    /// 0-indexed exclusive column after last ^
    pub col_end: usize,
    pub message_substring: String,
}

pub struct TestFile {
    pub cleaned_source: String,
    pub annotations: Vec<Annotation>,
}

/// Parse a test file, extracting `@@TAG: message` annotations.
///
/// `valid_tags` lists which tags are recognized (e.g. `["ERROR", "STRICT"]`).
/// Returns error if an unrecognized `@@` tag is encountered.
pub fn parse_test_file(source: &str, valid_tags: &[&str]) -> Result<TestFile, String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut cleaned_lines: Vec<&str> = Vec::new();
    let mut annotations = Vec::new();

    for (line_idx, line) in lines.iter().enumerate() {
        let orig_line_num = line_idx + 1;

        if let Some(marker_pos) = line.find("@@") {
            let marker = &line[marker_pos + 2..];

            let mut matched_tag = None;
            for &tag in valid_tags {
                let prefix = format!("{}:", tag);
                if let Some(rest) = marker.strip_prefix(prefix.as_str()) {
                    matched_tag = Some((tag.to_string(), rest.trim().to_string()));
                    break;
                }
            }

            let (tag, message) = match matched_tag {
                Some(t) => t,
                None => {
                    let expected = valid_tags
                        .iter()
                        .map(|t| format!("@@{}:", t))
                        .collect::<Vec<_>>()
                        .join(" or ");
                    return Err(format!(
                        "Line {}: Invalid marker (expected {})",
                        orig_line_num, expected
                    ));
                }
            };

            // Parse caret range from before @@
            let before_marker = &line[..marker_pos];
            let before_trimmed = before_marker.trim_end();

            let first_caret = before_trimmed.find('^');
            let last_caret = before_trimmed.rfind('^');

            match (first_caret, last_caret) {
                (Some(first), Some(last)) => {
                    if before_trimmed[..first].chars().any(|c| c != ' ') {
                        return Err(format!(
                            "Line {}: Only spaces allowed before '^' carets",
                            orig_line_num
                        ));
                    }
                    if before_trimmed[first..=last].chars().any(|c| c != '^') {
                        return Err(format!("Line {}: Carets must be contiguous", orig_line_num));
                    }

                    if cleaned_lines.is_empty() {
                        return Err(format!(
                            "Line {}: Annotation has no preceding code line",
                            orig_line_num
                        ));
                    }

                    annotations.push(Annotation {
                        tag,
                        line: cleaned_lines.len(), // 1-indexed
                        col_start: first,
                        col_end: last + 1,
                        message_substring: message,
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
    if source.ends_with('\n') {
        cleaned_source.push('\n');
    }

    Ok(TestFile {
        cleaned_source,
        annotations,
    })
}

/// Byte position to 1-indexed line number.
pub fn line_num(source: &str, pos: usize) -> usize {
    let clamped = pos.min(source.len());
    source[..clamped].chars().filter(|&c| c == '\n').count() + 1
}

/// Byte position to 0-indexed column.
pub fn col_num(source: &str, pos: usize) -> usize {
    let clamped = pos.min(source.len());
    let line_start = source[..clamped].rfind('\n').map(|p| p + 1).unwrap_or(0);
    clamped - line_start
}
