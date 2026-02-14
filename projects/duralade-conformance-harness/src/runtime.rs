use std::path::Path;

use duralade_runtime::testing::{self, TestOptions, TestResult};

pub fn run_test_file(path: &Path, stdlib_dir: &Path) -> Result<Vec<TestResult>, String> {
    let project_dir = find_project_dir(path)?;

    let summary = testing::load_and_run_tests(
        project_dir,
        stdlib_dir,
        None,
        &TestOptions { verify_gc: true },
        &mut |_| {},
    )?;

    Ok(summary.results)
}

fn find_project_dir(path: &Path) -> Result<&Path, String> {
    let mut dir = path.parent();
    while let Some(d) = dir {
        if d.join("duralade.toml").exists() {
            return Ok(d);
        }
        dir = d.parent();
    }
    Err(format!("no duralade.toml found above {}", path.display()))
}
