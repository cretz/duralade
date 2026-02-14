use std::sync::Arc;
use std::time::{Duration, Instant};

use duralade_language::engine::{Engine, EntityStatus};
use duralade_language::event::{EventType, OwnedFields, OwnedValue};
use duralade_language::interpret::scope::ImplicitInit;
use duralade_language::interpret::tick::TickStatus;
use duralade_language::load::{Annotation, Module, Registry};

use crate::local_engine::{LocalEngine, LocalEngineOptions};
use crate::project::load_project;
use crate::state_store::JsonMemoryStore;

#[derive(Debug)]
pub struct TestResult {
    pub qualified_name: String,
    pub passed: bool,
    pub error: Option<String>,
    pub duration: Duration,
}

fn is_test_annotation(ann: &Annotation, registry: &Registry) -> bool {
    registry
        .module_index("duralade.test")
        .and_then(|mi| registry.decl_id_in_module(mi, "test"))
        .is_some_and(|id| ann.decl == id)
}

/// Find all declarations that have a resolved @test annotation.
/// If `filter` is provided, only includes those whose qualified name contains the substring.
/// Returns (decl_name, qualified_name) pairs.
fn discover_test_funcs(
    module: &Module,
    module_path: &str,
    filter: Option<&str>,
    registry: &Registry,
) -> Vec<(String, String)> {
    let mut refs = Vec::new();
    for file in &module.files {
        for decl in &file.declarations {
            if decl
                .annotations
                .iter()
                .any(|a| is_test_annotation(a, registry))
            {
                let Some(name) = &decl.name else { continue };
                let qualified = format!("{}::{}", module_path, name);
                if filter.is_none_or(|f| qualified.contains(f)) {
                    refs.push((name.to_string(), qualified));
                }
            }
        }
    }
    refs
}

pub enum TestEvent<'a> {
    RunStarted { count: usize },
    TestStarted { qualified_name: &'a str },
    TestCompleted { result: &'a TestResult },
}

#[derive(Default)]
pub struct TestOptions {
    pub verify_gc: bool,
}

pub struct ProjectTestSummary {
    pub passed: usize,
    pub failed: usize,
    pub skipped: usize,
    pub results: Vec<TestResult>,
}

/// Run @test funcs across test modules with optional substring filter and event callbacks.
pub fn run_project_tests(
    registry: &Arc<Registry>,
    test_modules: &[String],
    filter: Option<&str>,
    options: &TestOptions,
    on_event: &mut dyn FnMut(TestEvent),
) -> ProjectTestSummary {
    let mut plan: Vec<Vec<(String, String)>> = Vec::new();
    let mut total = 0;
    for target in test_modules {
        let Some(module_idx) = registry.module_index(target) else {
            continue;
        };
        let module = registry.module(module_idx);
        let refs = discover_test_funcs(module, target, filter, registry);
        if !refs.is_empty() {
            total += refs.len();
            plan.push(refs);
        }
    }

    on_event(TestEvent::RunStarted { count: total });

    let mut passed = 0;
    let mut failed = 0;
    let mut results = Vec::new();

    for refs in &plan {
        for (decl_name, qualified_name) in refs {
            on_event(TestEvent::TestStarted { qualified_name });
            let start = Instant::now();
            let mut result = run_single_test(registry.clone(), decl_name, qualified_name, options);
            result.duration = start.elapsed();
            if result.passed {
                passed += 1;
            } else {
                failed += 1;
            }
            on_event(TestEvent::TestCompleted { result: &result });
            results.push(result);
        }
    }

    ProjectTestSummary {
        passed,
        failed,
        skipped: 0,
        results,
    }
}

/// Run all @test funcs in a single module (used by conformance harness).
pub fn run_tests(registry: &Arc<Registry>, target: &str, options: &TestOptions) -> Vec<TestResult> {
    let Some(module_idx) = registry.module_index(target) else {
        return vec![];
    };
    let module = registry.module(module_idx);

    let test_refs = discover_test_funcs(module, target, None, registry);
    tracing::info!(test_count = test_refs.len(), "running tests");
    let mut results = Vec::new();

    for (decl_name, qualified_name) in &test_refs {
        tracing::debug!(test = %decl_name, "running test");
        let start = Instant::now();
        let mut result = run_single_test(registry.clone(), decl_name, qualified_name, options);
        result.duration = start.elapsed();
        results.push(result);
    }

    results
}

/// Synchronously resolve an immediately-ready future (same as CLI's BlockReady).
fn block<F: std::future::Future>(f: F) -> F::Output {
    let mut pinned = std::pin::pin!(f);
    let waker = std::task::Waker::noop();
    let mut cx = std::task::Context::from_waker(waker);
    match pinned.as_mut().poll(&mut cx) {
        std::task::Poll::Ready(v) => v,
        std::task::Poll::Pending => panic!("future unexpectedly pending"),
    }
}

fn test_implicits(registry: &Registry) -> Vec<ImplicitInit> {
    let test_module = registry
        .module_index("duralade.test")
        .expect("duralade.test module not registered");
    let decl_id = registry
        .decl_id_in_module(test_module, "context")
        .expect("duralade.test::context not registered");
    let ty = registry
        .named_type(decl_id)
        .expect("duralade.test::context type not interned");
    vec![ImplicitInit { ty, decl: decl_id }]
}

fn run_single_test(
    registry: Arc<Registry>,
    decl_name: &str,
    qualified_name: &str,
    options: &TestOptions,
) -> TestResult {
    let store = JsonMemoryStore::empty();
    let implicits = test_implicits(&registry);
    let mut engine = LocalEngine::new(
        registry,
        store,
        LocalEngineOptions {
            disable_heap_collect: !options.verify_gc,
            cache_capacity: 64,
            cache_completed: options.verify_gc,
        },
    );

    let entity_type = qualified_name.to_string();
    let id = format!("test:{}", decl_name);

    let spawn_result = block(engine.spawn_entity(
        id.clone(),
        entity_type,
        OwnedFields::new(),
        implicits,
        0,
        false,
    ));

    let make_fail = |error: String| TestResult {
        qualified_name: qualified_name.to_string(),
        passed: false,
        error: Some(error),
        duration: Duration::ZERO,
    };

    let test_result = match spawn_result {
        Err(e) => return make_fail(e.to_string()),
        Ok(result) => match &result.status {
            TickStatus::Completed => {
                let fields = result.new_events.iter().find_map(|e| {
                    if let EventType::EntityComplete { result, .. } = &e.event_type {
                        result.as_ref()
                    } else {
                        None
                    }
                });
                interpret_test_fields(qualified_name, fields)
            }
            TickStatus::Blocked => {
                let _ = block(engine.tick_all(0));
                match block(engine.entity_status(&id)) {
                    Ok(EntityStatus::Completed { result }) => {
                        interpret_test_fields(qualified_name, result.as_ref())
                    }
                    Ok(_) => return make_fail("test blocked on unresolvable extern".into()),
                    Err(e) => return make_fail(e.to_string()),
                }
            }
        },
    };

    if options.verify_gc
        && test_result.passed
        && let Some(inst) = engine.cache().borrow_mut().take(&id)
        && let Err(msg) = inst.into_drop_details().assert_clean()
    {
        return make_fail(msg);
    }

    test_result
}

fn interpret_test_fields(qualified_name: &str, fields: Option<&OwnedFields>) -> TestResult {
    let Some(fields) = fields else {
        return TestResult {
            qualified_name: qualified_name.to_string(),
            passed: true,
            error: None,
            duration: Duration::ZERO,
        };
    };

    // Check for error field (from `out! :error?`).
    if let Some(error_val) = fields.get("error") {
        match error_val {
            OwnedValue::Nil => {}
            OwnedValue::Data(error_fields) => {
                let msg = error_fields
                    .get("message")
                    .and_then(|v| {
                        if let OwnedValue::Str(s) = v {
                            Some(s.clone())
                        } else {
                            None
                        }
                    })
                    .unwrap_or_else(|| "test failed with error".to_string());
                return TestResult {
                    qualified_name: qualified_name.to_string(),
                    passed: false,
                    error: Some(msg),
                    duration: Duration::ZERO,
                };
            }
            _ => {
                return TestResult {
                    qualified_name: qualified_name.to_string(),
                    passed: false,
                    error: Some(format!("test failed with non-data error: {:?}", error_val)),
                    duration: Duration::ZERO,
                };
            }
        }
    }

    TestResult {
        qualified_name: qualified_name.to_string(),
        passed: true,
        error: None,
        duration: Duration::ZERO,
    }
}

/// Load a project and run its @test functions. Shared entry point for
/// Rust #[test]s, CLI, and conformance harness.
pub fn load_and_run_tests(
    project_dir: &std::path::Path,
    stdlib_dir: &std::path::Path,
    filter: Option<&str>,
    options: &TestOptions,
    on_event: &mut dyn FnMut(TestEvent),
) -> Result<ProjectTestSummary, String> {
    let result = load_project(project_dir, stdlib_dir, true)?;

    let mut msgs = Vec::new();
    for err in &result.load_result.parse_errors {
        msgs.push(format!(
            "parse: {}",
            err.format_with_registry(&result.load_result.registry)
        ));
    }
    for err in &result.load_result.load_errors {
        msgs.push(err.format_with_registry(&result.load_result.registry));
    }
    for err in &result.load_result.strict_violations {
        msgs.push(format!(
            "strict: {}",
            err.format_with_registry(&result.load_result.registry)
        ));
    }
    if !msgs.is_empty() {
        return Err(format!("project has errors:\n  {}", msgs.join("\n  ")));
    }

    let registry = Arc::new(result.load_result.registry);
    let summary = run_project_tests(&registry, &result.test_modules, filter, options, on_event);
    Ok(summary)
}
