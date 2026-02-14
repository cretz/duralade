use std::path::Path;

use duralade_runtime::project::load_project;
use duralade_runtime::testing::{self, TestOptions};

fn test_project(path: &Path) -> datatest_stable::Result<()> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");

    let project_dir = path.parent().expect("duralade.toml must have parent dir");
    let result = duralade_conformance_harness::project::run_test_project(project_dir, &stdlib_dir)?;

    if !result.passed {
        let msg = format!("Test failed:\n{}", result.diagnostics.join("\n"));
        return Err(msg.into());
    }

    Ok(())
}

fn test_stdlib(_path: &Path) -> datatest_stable::Result<()> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");
    let project_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib");

    let summary = testing::load_and_run_tests(
        &project_dir,
        &stdlib_dir,
        None,
        &TestOptions { verify_gc: true },
        &mut |_| {},
    )?;

    if summary.failed > 0 {
        let mut failures = Vec::new();
        for result in &summary.results {
            if !result.passed {
                let msg = match &result.error {
                    Some(e) => format!("{}: FAILED - {}", result.qualified_name, e),
                    None => format!("{}: FAILED", result.qualified_name),
                };
                failures.push(msg);
            }
        }
        return Err(format!("Stdlib test failures:\n{}", failures.join("\n")).into());
    }

    if summary.passed == 0 {
        return Err("No stdlib @test functions found".into());
    }

    Ok(())
}

fn test_parser_conformance(path: &Path) -> datatest_stable::Result<()> {
    let result = duralade_conformance_harness::parser::run_test_file(path)?;

    if !result.passed {
        let msg = format!("Test failed:\n{}", result.diagnostics.join("\n"));
        return Err(msg.into());
    }

    Ok(())
}

fn test_sample_loads(path: &Path) -> datatest_stable::Result<()> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");

    // Walk up from .dl file to find duralade.toml
    let mut dir = path.parent();
    let project_dir = loop {
        match dir {
            Some(d) if d.join("duralade.toml").exists() => break d,
            Some(d) => dir = d.parent(),
            None => return Err(format!("no duralade.toml found above {}", path.display()).into()),
        }
    };

    let result = load_project(project_dir, &stdlib_dir, false)?;

    let mut errors: Vec<String> = Vec::new();
    for err in &result.load_result.parse_errors {
        errors.push(format!(
            "  parse: {}",
            err.format_with_registry(&result.load_result.registry)
        ));
    }
    for err in &result.load_result.load_errors {
        errors.push(format!(
            "  load: {}",
            err.format_with_registry(&result.load_result.registry)
        ));
    }

    if !errors.is_empty() {
        return Err(format!("Sample should load cleanly:\n{}", errors.join("\n")).into());
    }

    Ok(())
}

fn test_loader_types(path: &Path) -> datatest_stable::Result<()> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");

    let result = duralade_conformance_harness::loader_types::run_test_file(path, &stdlib_dir)?;

    if !result.passed {
        let msg = format!("Test failed:\n{}", result.diagnostics.join("\n"));
        return Err(msg.into());
    }

    Ok(())
}

fn test_runtime(path: &Path) -> datatest_stable::Result<()> {
    let stdlib_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");

    let results = duralade_conformance_harness::runtime::run_test_file(path, &stdlib_dir)?;

    let mut failures = Vec::new();
    for result in &results {
        if !result.passed {
            let msg = match &result.error {
                Some(e) => format!("{}: FAILED - {}", result.qualified_name, e),
                None => format!("{}: FAILED", result.qualified_name),
            };
            failures.push(msg);
        }
    }

    if !failures.is_empty() {
        return Err(format!("Runtime test failures:\n{}", failures.join("\n")).into());
    }

    if results.is_empty() {
        return Err("No @test functions found".into());
    }

    Ok(())
}

datatest_stable::harness! {
    {
        test = test_parser_conformance,
        root = "../duralade-conformance/parser",
        pattern = r"\.dl$"
    },
    {
        test = test_sample_loads,
        root = "../../samples",
        pattern = r"\.dl$"
    },
    {
        test = test_loader_types,
        root = "../duralade-conformance/loader_types",
        pattern = r"\.dl$"
    },
    {
        test = test_runtime,
        root = "../duralade-conformance/runtime/test",
        pattern = r"\.dl$"
    },
    {
        test = test_project,
        root = "../duralade-conformance/project",
        pattern = r"duralade\.toml$"
    },
    {
        test = test_stdlib,
        root = "../duralade-stdlib",
        pattern = r"duralade\.toml$"
    },
}
