use std::path::Path;
use std::sync::Arc;

use duralade_runtime::project::load_project;
use duralade_runtime::testing::{self, TestOptions};

pub struct ProjectTestResult {
    pub passed: bool,
    pub diagnostics: Vec<String>,
}

pub fn run_test_project(
    project_dir: &Path,
    stdlib_dir: &Path,
) -> Result<ProjectTestResult, String> {
    let result = load_project(project_dir, stdlib_dir, true)?;

    if result.load_result.has_errors() {
        let mut msgs = Vec::new();
        for err in &result.load_result.parse_errors {
            msgs.push(format!("parse error in {:?}: {}", err.module, err.message));
        }
        for err in &result.load_result.load_errors {
            msgs.push(format!("load error in {:?}: {}", err.module, err.message));
        }
        return Err(msgs.join("\n"));
    }

    if !result.load_result.strict_violations.is_empty() {
        let mut msgs = Vec::new();
        for err in &result.load_result.strict_violations {
            msgs.push(format!("strict: {:?}: {}", err.module, err.message));
        }
        return Err(msgs.join("\n"));
    }

    let registry = Arc::new(result.load_result.registry);

    let mut diagnostics = Vec::new();
    let mut all_passed = true;

    for target in &result.test_modules {
        let results = testing::run_tests(&registry, target, &TestOptions { verify_gc: true });
        for r in &results {
            if !r.passed {
                all_passed = false;
                let msg = match &r.error {
                    Some(e) => format!("{}: FAILED - {}", r.qualified_name, e),
                    None => format!("{}: FAILED", r.qualified_name),
                };
                diagnostics.push(msg);
            }
        }
    }

    Ok(ProjectTestResult {
        passed: all_passed,
        diagnostics,
    })
}
