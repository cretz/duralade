use duralade_conformance_harness::run_test_file;
use duralade_language::parse::parse_source_file;
use std::path::Path;

fn test_conformance_file(path: &Path) -> datatest_stable::Result<()> {
    let result = run_test_file(path)?;

    if !result.passed {
        let msg = format!("Test failed:\n{}", result.diagnostics.join("\n"));
        return Err(msg.into());
    }

    Ok(())
}

fn test_sample_parses(path: &Path) -> datatest_stable::Result<()> {
    let source = std::fs::read_to_string(path)?;
    let result = parse_source_file(source, path.to_path_buf());

    let mut errors: Vec<String> = Vec::new();
    for err in &result.parse_errors {
        errors.push(format!("ERROR: {}", err.message));
    }
    for err in &result.strict_mode_violations {
        errors.push(format!("STRICT: {}", err.message));
    }

    if !errors.is_empty() {
        return Err(format!("Sample should parse cleanly:\n{}", errors.join("\n")).into());
    }

    Ok(())
}

datatest_stable::harness! {
    {
        test = test_conformance_file,
        root = "../duralade-conformance/parser",
        pattern = r"\.dl$"
    },
    {
        test = test_sample_parses,
        root = "../../samples",
        pattern = r"\.dl$"
    },
}
