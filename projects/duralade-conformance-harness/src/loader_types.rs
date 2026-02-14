use std::path::Path;

use duralade_language::load::{self, LoadContext};
use duralade_runtime::disk_fetch::DiskFetcher;
use duralade_runtime::tokio_executor::TokioExecutor;

use crate::annotations;

#[derive(Debug)]
pub struct TestResult {
    pub passed: bool,
    pub diagnostics: Vec<String>,
}

fn run_test(test_file: annotations::TestFile, file_path: &Path, stdlib_dir: &Path) -> TestResult {
    let stem = file_path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("test");

    let module_path = format!("loader_test.{stem}");

    // Write cleaned source to an isolated temp dir so parallel tests don't interfere
    let temp_dir = std::env::temp_dir()
        .join("duralade_loader_type_tests")
        .join(stem);
    std::fs::create_dir_all(&temp_dir).unwrap();
    let temp_file = temp_dir.join(file_path.file_name().unwrap());
    std::fs::write(&temp_file, &test_file.cleaned_source).unwrap();

    let mut fetcher = DiskFetcher::new();
    fetcher.add_root("loader_test", &temp_dir);
    fetcher.add_root("duralade", stdlib_dir);
    fetcher.set_allow_builtin("duralade");

    let executor = TokioExecutor::new();
    let mut root_settings = std::collections::HashMap::new();
    root_settings.insert(
        "loader_test".to_string(),
        load::RootSettings {
            stdlib_prelude: true,
        },
    );
    let mut ctx = LoadContext::new(root_settings);
    ctx.populate_non_required_types = true;

    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(load::load_module(
        &mut ctx,
        &executor,
        &fetcher,
        module_path.clone().into(),
    ));

    let result = match ctx.into_result() {
        Ok(r) => r,
        Err(_) => panic!("LoadContext still has outstanding references"),
    };

    let mut diagnostics = Vec::new();

    // The file must load cleanly - any errors are test failures
    for err in &result.parse_errors {
        diagnostics.push(format!(
            "Unexpected parse error: \"{}\"",
            err.format_with_registry(&result.registry)
        ));
    }
    for err in &result.load_errors {
        diagnostics.push(format!(
            "Unexpected load error: \"{}\"",
            err.format_with_registry(&result.registry)
        ));
    }
    for err in &result.strict_violations {
        diagnostics.push(format!(
            "Strict violation: \"{}\"",
            err.format_with_registry(&result.registry)
        ));
    }

    if !diagnostics.is_empty() {
        let _ = std::fs::remove_dir_all(&temp_dir);
        return TestResult {
            passed: false,
            diagnostics,
        };
    }

    // Find the module and its first file's type table + source map
    let module_idx = result.registry.module_index(&module_path);
    let module = module_idx.map(|idx| result.registry.module(idx).clone());
    let module = match module {
        Some(m) => m,
        None => {
            diagnostics.push(format!("Module '{}' not found in registry", module_path));
            let _ = std::fs::remove_dir_all(&temp_dir);
            return TestResult {
                passed: false,
                diagnostics,
            };
        }
    };

    // The test module has exactly one file (the test .dl)
    let file = &module.files[0];
    let source = &test_file.cleaned_source;
    let source_map = &file.source_file.source_map;
    let type_table = &file.type_table;

    // For each @@TYPE annotation, find the matching node and check its type
    for expected in &test_file.annotations {
        // Convert line/col to byte offset range
        let start_byte = line_col_to_byte(source, expected.line, expected.col_start);
        let end_byte = line_col_to_byte(source, expected.line, expected.col_end);

        // Find a NodeId with an exact matching byte range
        let found_node = source_map
            .node_ranges
            .iter()
            .find(|&(_, &(s, e))| s == start_byte && e == end_byte);

        let Some((&node_id, _)) = found_node else {
            diagnostics.push(format!(
                "Line {}:{}-{}: No AST node found at this position for @@TYPE: \"{}\"",
                expected.line, expected.col_start, expected.col_end, expected.message_substring
            ));
            continue;
        };

        let Some(ty) = type_table.node_types.get(&node_id) else {
            diagnostics.push(format!(
                "Line {}:{}-{}: AST node found but no type recorded for @@TYPE: \"{}\"",
                expected.line, expected.col_start, expected.col_end, expected.message_substring
            ));
            continue;
        };

        let actual_display = result.registry.display_type_qualified(*ty);
        if actual_display != expected.message_substring {
            diagnostics.push(format!(
                "Line {}:{}-{}: Type mismatch: expected \"{}\", got \"{}\"",
                expected.line,
                expected.col_start,
                expected.col_end,
                expected.message_substring,
                actual_display
            ));
        }
    }

    // Clean up temp file
    let _ = std::fs::remove_dir_all(&temp_dir);

    TestResult {
        passed: diagnostics.is_empty(),
        diagnostics,
    }
}

/// Convert 1-indexed line and 0-indexed column to a byte offset.
fn line_col_to_byte(source: &str, line: usize, col: usize) -> usize {
    let mut current_line = 1;
    for (i, ch) in source.char_indices() {
        if current_line == line {
            return i + col;
        }
        if ch == '\n' {
            current_line += 1;
        }
    }
    source.len()
}

pub fn run_test_file(path: &Path, stdlib_dir: &Path) -> Result<TestResult, String> {
    let source = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let test_file = annotations::parse_test_file(&source, &["TYPE"])?;
    Ok(run_test(test_file, path, stdlib_dir))
}
