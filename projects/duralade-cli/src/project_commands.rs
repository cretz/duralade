use duralade_runtime::testing::{self, TestEvent, TestOptions};

use crate::ProjectTestArgs;
use crate::project;

// TODO: --compact flag for condensed output (single line per test)
// TODO: --format junit and --format tap for machine-readable output

pub fn test(args: ProjectTestArgs) -> Result<(), String> {
    let project_dir = project::resolve_project_dir(&args.code)?;
    let stdlib_dir = project::find_stdlib()?;

    let mut failures: Vec<String> = Vec::new();

    // TODO: consider exposing --verify-gc CLI arg
    let summary = testing::load_and_run_tests(
        &project_dir,
        &stdlib_dir,
        args.filter.as_deref(),
        &TestOptions::default(),
        &mut |event| match event {
            TestEvent::RunStarted { count } => {
                eprintln!("running {count} test(s)\n");
            }
            TestEvent::TestStarted { qualified_name } => {
                eprintln!("=== {} ===", qualified_name);
            }
            TestEvent::TestCompleted { result } => {
                if result.passed {
                    eprintln!("PASS in {}ms\n", result.duration.as_millis());
                } else {
                    if let Some(err) = &result.error {
                        eprintln!("{err}");
                    }
                    eprintln!("FAIL in {}ms\n", result.duration.as_millis());
                    failures.push(result.qualified_name.clone());
                }
            }
        },
    )?;

    eprintln!(
        "{} passed, {} failed, {} skipped",
        summary.passed, summary.failed, summary.skipped
    );

    if !failures.is_empty() {
        eprintln!("  failures:");
        for name in &failures {
            eprintln!("    FAIL: {name}");
        }
    }

    if summary.failed > 0 {
        Err(format!("{} test(s) failed", summary.failed))
    } else {
        Ok(())
    }
}
