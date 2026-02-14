use std::process::ExitCode;

use clap::{Args, Parser, Subcommand};

mod engine;
mod entity_commands;
mod project;
mod project_commands;
mod state;

#[derive(Parser)]
#[command(name = "duralade", version, about = "Duralade entity engine")]
struct Cli {
    /// Enable debug-level logging (or use RUST_LOG env var for fine-grained control)
    #[arg(short, long, global = true)]
    verbose: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Entity lifecycle commands
    #[command(subcommand)]
    Entity(Box<EntityCommand>),
    /// Project-level commands
    #[command(subcommand)]
    Project(ProjectCommand),
}

#[derive(Subcommand)]
enum ProjectCommand {
    /// Run project tests
    Test(ProjectTestArgs),
}

#[derive(Args)]
struct ProjectTestArgs {
    /// Code source directory
    #[arg(long, default_value = ".")]
    code: String,
    /// Only run tests whose qualified name contains this substring
    filter: Option<String>,
    /// Disable strict mode checks (ordering, formatting)
    #[arg(long)]
    no_strict: bool,
}

#[derive(Subcommand)]
enum EntityCommand {
    /// Create a new entity
    Spawn(EntitySpawnArgs),
    /// Execute entity until all coroutines yield
    Tick(EntityTickArgs),
    /// Show entity state and completion info
    Describe(EntityDescribeArgs),
    /// Read-only query (view function or out fields)
    View(EntityViewArgs),
    /// Replay entity from events (diagnostic, read-only)
    Replay(EntityReplayArgs),
    /// Display call stacks of active coroutines
    #[command(name = "inspect-stack")]
    InspectStack(EntityInspectStackArgs),
    /// Queue a function invocation for next tick
    #[command(name = "invoke-func")]
    InvokeFunc(EntityInvokeFuncArgs),
    /// Invoke noblock function, tick, and return result
    #[command(name = "invoke-func-noblock")]
    InvokeFuncNoblock(EntityInvokeFuncNoblockArgs),
    /// Complete a pending extern invocation
    #[command(name = "complete-extern")]
    CompleteExtern(EntityCompleteExternArgs),
    /// Request cooperative cancellation
    Cancel(EntityCancelArgs),
    /// Cancel an in-flight function invocation
    #[command(name = "cancel-func")]
    CancelFunc(EntityCancelFuncArgs),
    /// Checkpoint and compact event log
    Checkpoint(EntityCheckpointArgs),
    /// Force terminate entity
    Terminate(EntityTerminateArgs),
    /// Interactive entity shell
    Shell(EntityShellArgs),
}

// Shared entity arg groups

#[derive(Args)]
struct EntityGlobalArgs {
    /// Code source directory
    #[arg(long, default_value = ".")]
    code: String,
    /// Time override (epoch ms, default: wall clock)
    #[arg(long)]
    current_time: Option<u64>,
    /// State codec (accepted, not yet implemented)
    #[arg(long)]
    codec: Option<String>,
    /// Disable strict mode checks (ordering, formatting)
    #[arg(long)]
    no_strict: bool,
}

#[derive(Args)]
struct EntityStateArgs {
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Write to a different store
    #[arg(long)]
    state_out: Option<String>,
    /// Show what would happen without writing
    #[arg(long)]
    dry_run: bool,
}

#[derive(Args)]
struct EntityInputArgs {
    /// Input data as JSON string
    #[arg(long = "in", value_name = "DATA")]
    data: Option<String>,
    /// Read input data from file
    #[arg(long = "in-file")]
    file: Option<String>,
    /// Input format override
    #[arg(long = "in-format")]
    format: Option<String>,
}

#[derive(Args)]
struct EntityOutputArgs {
    /// Where to write output (default: stdio)
    #[arg(long, default_value = "stdio://")]
    out: String,
    /// Output format
    #[arg(long, default_value = "json")]
    out_format: String,
}

#[derive(Args)]
struct EntityResultArgs {
    /// Result data as JSON string
    #[arg(long)]
    result: Option<String>,
    /// Read result from file
    #[arg(long = "result-file")]
    result_file: Option<String>,
    /// Result format override
    #[arg(long = "result-format")]
    result_format: Option<String>,
}

// Per-entity-command args

#[derive(Args)]
struct EntitySpawnArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity type (e.g. my_project.user)
    #[arg(long)]
    entity: String,
    /// Entity identifier
    #[arg(long)]
    id: String,
    /// Skip initial tick
    #[arg(long)]
    no_tick: bool,
    #[command(flatten)]
    input: EntityInputArgs,
}

#[derive(Args)]
struct EntityTickArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity to tick (ticks all if omitted)
    #[arg(long)]
    id: Option<String>,
}

#[derive(Args)]
struct EntityDescribeArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    output: EntityOutputArgs,
    /// Include specific out field (repeatable)
    #[arg(long = "get-out")]
    get_out: Vec<String>,
    /// Include all out fields
    #[arg(long)]
    get_out_all: bool,
    /// Omit run result for completed entities
    #[arg(long)]
    no_result: bool,
}

#[derive(Args)]
struct EntityViewArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    output: EntityOutputArgs,
    /// View function to invoke
    #[arg(long, group = "view_mode")]
    func: Option<String>,
    /// Specific out field to read (repeatable)
    #[arg(long, group = "view_mode")]
    field: Vec<String>,
    /// Read all out fields
    #[arg(long, group = "view_mode")]
    field_all: bool,
    #[command(flatten)]
    input: EntityInputArgs,
    /// Truncate events to this number, then tick and inspect
    #[arg(long = "after-event-num")]
    after_event_num: Option<u64>,
}

#[derive(Args)]
struct EntityReplayArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Entity identifier
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct EntityInspectStackArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    output: EntityOutputArgs,
    /// Truncate events to this number, then tick and inspect
    #[arg(long = "after-event-num")]
    after_event_num: Option<u64>,
}

#[derive(Args)]
struct EntityInvokeFuncArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    input: EntityInputArgs,
    /// Function name to invoke
    #[arg(long)]
    func: String,
    /// Request identifier
    #[arg(long = "request-id")]
    request_id: Option<String>,
}

#[derive(Args)]
struct EntityInvokeFuncNoblockArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    input: EntityInputArgs,
    #[command(flatten)]
    output: EntityOutputArgs,
    /// Noblock function name to invoke
    #[arg(long)]
    func: String,
}

#[derive(Args)]
struct EntityCompleteExternArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
    #[command(flatten)]
    result: EntityResultArgs,
    /// Event number of the ExternInvoke to complete
    #[arg(long = "invoke-num")]
    invoke_num: u64,
}

#[derive(Args)]
struct EntityCancelArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct EntityCancelFuncArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
    /// Request ID of the FuncInvoke to cancel
    #[arg(long = "request-id")]
    request_id: String,
}

#[derive(Args)]
struct EntityCheckpointArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
    /// Keep all events instead of compacting
    #[arg(long)]
    no_compact: bool,
}

#[derive(Args)]
struct EntityTerminateArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    #[command(flatten)]
    store: EntityStateArgs,
    /// Entity identifier
    #[arg(long)]
    id: String,
}

#[derive(Args)]
struct EntityShellArgs {
    #[command(flatten)]
    global: EntityGlobalArgs,
    /// State store (default: ./duralade-state.json)
    #[arg(long, default_value = "./duralade-state.json")]
    state: String,
    /// Defer writes, use explicit persist command
    #[arg(long)]
    no_auto_persist: bool,
    /// Prevent any state modifications
    #[arg(long)]
    read_only: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    let filter = if cli.verbose {
        tracing_subscriber::EnvFilter::new("debug")
    } else {
        tracing_subscriber::EnvFilter::from_default_env()
    };
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();

    let result = match cli.command {
        Command::Entity(cmd) => match *cmd {
            EntityCommand::Spawn(args) => entity_commands::spawn(args),
            EntityCommand::Tick(args) => entity_commands::tick(args),
            EntityCommand::Describe(args) => entity_commands::describe(args),
            EntityCommand::View(args) => entity_commands::view(args),
            EntityCommand::Replay(args) => entity_commands::replay(args),
            EntityCommand::InspectStack(args) => entity_commands::inspect_stack(args),
            EntityCommand::InvokeFunc(args) => entity_commands::invoke_func(args),
            EntityCommand::InvokeFuncNoblock(args) => entity_commands::invoke_func_noblock(args),
            EntityCommand::CompleteExtern(args) => entity_commands::complete_extern(args),
            EntityCommand::Cancel(args) => entity_commands::cancel(args),
            EntityCommand::CancelFunc(args) => entity_commands::cancel_func(args),
            EntityCommand::Checkpoint(args) => entity_commands::checkpoint(args),
            EntityCommand::Terminate(args) => entity_commands::terminate(args),
            EntityCommand::Shell(args) => entity_commands::shell(args),
        },
        Command::Project(cmd) => match cmd {
            ProjectCommand::Test(args) => project_commands::test(args),
        },
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("error: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn resolve_time(current_time: Option<u64>) -> u64 {
    current_time.unwrap_or_else(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("system clock before epoch")
            .as_millis() as u64
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use duralade_runtime::state_store::StateStore;

    struct TestProject {
        dir: tempfile::TempDir,
    }

    impl TestProject {
        fn new(project_name: &str, module_name: &str, source: &str) -> Self {
            let dir = tempfile::tempdir().unwrap();
            let toml = format!("[project]\nname = \"{project_name}\"\n");
            std::fs::write(dir.path().join("duralade.toml"), toml).unwrap();
            let src_dir = dir.path().join("src");
            std::fs::create_dir_all(&src_dir).unwrap();
            std::fs::write(src_dir.join(format!("{module_name}.dl")), source).unwrap();
            Self { dir }
        }

        fn with_test(self, module_name: &str, source: &str) -> Self {
            let test_dir = self.dir.path().join("test");
            std::fs::create_dir_all(&test_dir).unwrap();
            std::fs::write(test_dir.join(format!("{module_name}.dl")), source).unwrap();
            self
        }

        fn code_path(&self) -> String {
            self.dir.path().to_str().unwrap().to_string()
        }

        fn state_path(&self) -> String {
            self.dir
                .path()
                .join("state.json")
                .to_str()
                .unwrap()
                .to_string()
        }

        fn out_path(&self, name: &str) -> String {
            self.dir.path().join(name).to_str().unwrap().to_string()
        }
    }

    fn global(code: &str) -> EntityGlobalArgs {
        EntityGlobalArgs {
            code: code.into(),
            current_time: Some(1000),
            codec: None,
            no_strict: true,
        }
    }

    fn store_args(state: &str) -> EntityStateArgs {
        EntityStateArgs {
            state: state.into(),
            state_out: None,
            dry_run: false,
        }
    }

    fn no_input() -> EntityInputArgs {
        EntityInputArgs {
            data: None,
            file: None,
            format: None,
        }
    }

    /// Read events for an entity from the store file.
    fn read_entity_events(
        state_path: &str,
        entity_id: &str,
    ) -> Vec<duralade_language::event::Event> {
        let url = state::parse_state_uri(state_path).unwrap();
        let store = state::Store::open(&url, None, false).unwrap();
        use entity_commands::BlockReady;
        store.read_events(entity_id).block().unwrap()
    }

    #[test]
    fn spawn_tick_extern_cycle() {
        let proj = TestProject::new(
            "myapp",
            "worker",
            "\
extern get_thing {
    out value: int
}

out entity worker {
    run {
        out result: int

        result = get_thing().value
    }
}
",
        );
        let state_file = proj.state_path();

        // 1. Spawn with --no-tick
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.worker".into(),
            id: "e1".into(),
            no_tick: true,
            input: no_input(),
        })
        .unwrap();

        // State should have 1 event (EntityInvoke).
        let events = read_entity_events(&state_file, "e1");
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0].event_type,
            duralade_language::event::EventType::EntityInvoke { entity, .. }
            if entity == "myapp.worker"
        ));

        // 2. Tick - should block on get_thing extern.
        entity_commands::tick(EntityTickArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: Some("e1".into()),
        })
        .unwrap();

        let events = read_entity_events(&state_file, "e1");
        assert!(events.len() >= 2);
        let extern_invoke = events.iter().find(|e| {
            matches!(
                &e.event_type,
                duralade_language::event::EventType::ExternInvoke { extern_name, .. }
                if extern_name == "myapp.worker::get_thing"
            )
        });
        assert!(extern_invoke.is_some(), "expected ExternInvoke event");
        let invoke_event_num = extern_invoke.unwrap().num;

        // 3. Complete the extern with result value 42.
        entity_commands::complete_extern(EntityCompleteExternArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: "e1".into(),
            result: EntityResultArgs {
                result: Some(r#"{"value": 42}"#.into()),
                result_file: None,
                result_format: None,
            },
            invoke_num: invoke_event_num,
        })
        .unwrap();

        // 4. Tick again - should complete.
        entity_commands::tick(EntityTickArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: Some("e1".into()),
        })
        .unwrap();

        let events = read_entity_events(&state_file, "e1");
        let complete = events.iter().rev().find(|e| {
            matches!(
                &e.event_type,
                duralade_language::event::EventType::EntityComplete { .. }
            )
        });
        assert!(complete.is_some(), "expected EntityComplete event");

        // 5. Describe - should show completed with result.
        let describe_out = proj.out_path("describe.json");
        entity_commands::describe(EntityDescribeArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: describe_out.clone(),
                out_format: "json".into(),
            },
            get_out: vec![],
            get_out_all: false,
            no_result: false,
        })
        .unwrap();

        let desc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&describe_out).unwrap()).unwrap();
        assert_eq!(desc["status"], "completed");
        assert_eq!(desc["result"]["result"], 42);
    }

    #[test]
    fn implicit_project_spawn_completes() {
        let dir = tempfile::tempdir().unwrap();
        // No duralade.toml - implicit project mode
        std::fs::write(
            dir.path().join("simple.dl"),
            "\
out entity simple {
    run {
        out result: int

        result = 99
    }
}
",
        )
        .unwrap();
        let code = dir.path().to_str().unwrap().to_string();
        let state_file = dir.path().join("state.json").to_str().unwrap().to_string();

        entity_commands::spawn(EntitySpawnArgs {
            global: global(&code),
            store: store_args(&state_file),
            entity: "simple".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        let events = read_entity_events(&state_file, "e1");

        // Entity type in events should be fully qualified with "_." prefix
        assert!(matches!(
            &events[0].event_type,
            duralade_language::event::EventType::EntityInvoke { entity, .. }
            if entity == "_.simple"
        ));

        // Should have completed
        let complete = events.iter().find(|e| {
            matches!(
                &e.event_type,
                duralade_language::event::EventType::EntityComplete { .. }
            )
        });
        assert!(complete.is_some(), "expected EntityComplete");

        // Describe should show result
        let describe_out = dir
            .path()
            .join("describe.json")
            .to_str()
            .unwrap()
            .to_string();
        entity_commands::describe(EntityDescribeArgs {
            global: global(&code),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: describe_out.clone(),
                out_format: "json".into(),
            },
            get_out: vec![],
            get_out_all: false,
            no_result: false,
        })
        .unwrap();

        let desc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&describe_out).unwrap()).unwrap();
        assert_eq!(desc["status"], "completed");
        assert_eq!(desc["result"]["result"], 99);
    }

    #[test]
    fn entity_run_completes() {
        let proj = TestProject::new(
            "myapp",
            "simple",
            "\
out entity simple {
    run {
        out result: int

        result = 42
    }
}
",
        );
        let state_file = proj.state_path();

        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.simple".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        let events = read_entity_events(&state_file, "e1");
        let complete = events.iter().find(|e| {
            matches!(
                &e.event_type,
                duralade_language::event::EventType::EntityComplete { .. }
            )
        });
        assert!(
            complete.is_some(),
            "expected EntityComplete after entity spawn with tick"
        );

        let describe_out = proj.out_path("describe.json");
        entity_commands::describe(EntityDescribeArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: describe_out.clone(),
                out_format: "json".into(),
            },
            get_out: vec![],
            get_out_all: false,
            no_result: false,
        })
        .unwrap();

        let desc: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&describe_out).unwrap()).unwrap();
        assert_eq!(desc["status"], "completed");
        assert_eq!(desc["result"]["result"], 42);
    }

    #[test]
    fn cancel_func_command() {
        let proj = TestProject::new(
            "myapp",
            "worker",
            "\
extern get_thing {
    out value: int
}

out entity worker {
    run {
        out result: int

        result = get_thing().value
    }

    out func do_work {
        out done: bool

        done = true
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn entity
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.worker".into(),
            id: "e1".into(),
            no_tick: true,
            input: no_input(),
        })
        .unwrap();

        // Invoke func with a request-id
        entity_commands::invoke_func(EntityInvokeFuncArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: "e1".into(),
            input: no_input(),
            func: "do_work".into(),
            request_id: Some("req-1".into()),
        })
        .unwrap();

        // Cancel the func
        entity_commands::cancel_func(EntityCancelFuncArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: "e1".into(),
            request_id: "req-1".into(),
        })
        .unwrap();

        // Verify FuncCancel event exists
        let events = read_entity_events(&state_file, "e1");
        let cancel = events.iter().find(|e| {
            matches!(
                &e.event_type,
                duralade_language::event::EventType::FuncCancel { .. }
            )
        });
        assert!(cancel.is_some(), "expected FuncCancel event");

        // Cancelling again should error
        let err = entity_commands::cancel_func(EntityCancelFuncArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            id: "e1".into(),
            request_id: "req-1".into(),
        });
        assert!(err.is_err(), "expected error on double cancel");
    }

    #[test]
    fn replay_completed_entity() {
        let proj = TestProject::new(
            "myapp",
            "simple",
            "\
out entity simple {
    run {
        out result: int

        result = 42
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn (auto-tick completes the entity)
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.simple".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        // Replay should succeed on completed entity
        entity_commands::replay(EntityReplayArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
        })
        .unwrap();
    }

    #[test]
    fn replay_blocked_entity() {
        let proj = TestProject::new(
            "myapp",
            "worker",
            "\
extern get_thing {
    out value: int
}

out entity worker {
    run {
        out result: int

        result = get_thing().value
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn + tick → blocks on extern
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.worker".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        // Replay should succeed (entity is blocked, not faulted)
        entity_commands::replay(EntityReplayArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
        })
        .unwrap();
    }

    #[test]
    fn view_field_all() {
        let proj = TestProject::new(
            "myapp",
            "counter",
            "\
extern get_thing {
    out value: int
}

out entity counter {
    out count: int = 0

    run {
        out result: int

        count = 10
        result = get_thing().value
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn + tick → blocks on extern, but count is set to 10
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.counter".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        let view_out = proj.out_path("view.json");
        entity_commands::view(EntityViewArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: view_out.clone(),
                out_format: "json".into(),
            },
            func: None,
            field: vec![],
            field_all: true,
            input: no_input(),
            after_event_num: None,
        })
        .unwrap();

        let val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&view_out).unwrap()).unwrap();
        assert_eq!(val["count"], 10);
    }

    #[test]
    fn view_func() {
        let proj = TestProject::new(
            "myapp",
            "store",
            "\
extern get_thing {
    out value: int
}

out entity store {
    out total: int = 0

    run {
        out result: int

        total = 5
        result = get_thing().value
    }

    view func summary {
        out amount: int

        amount = total * 2
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn + tick → blocks on extern, total is set to 5
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.store".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        let view_out = proj.out_path("view.json");
        entity_commands::view(EntityViewArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: view_out.clone(),
                out_format: "json".into(),
            },
            func: Some("summary".into()),
            field: vec![],
            field_all: false,
            input: no_input(),
            after_event_num: None,
        })
        .unwrap();

        let val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&view_out).unwrap()).unwrap();
        assert_eq!(val["amount"], 10);
    }

    #[test]
    fn inspect_stack_blocked_entity() {
        let proj = TestProject::new(
            "myapp",
            "worker",
            "\
extern get_thing {
    out value: int
}

out entity worker {
    run {
        out result: int

        result = get_thing().value
    }
}
",
        );
        let state_file = proj.state_path();

        // Spawn + tick → blocks on get_thing extern
        entity_commands::spawn(EntitySpawnArgs {
            global: global(&proj.code_path()),
            store: store_args(&state_file),
            entity: "myapp.worker".into(),
            id: "e1".into(),
            no_tick: false,
            input: no_input(),
        })
        .unwrap();

        let out_file = proj.out_path("stacks.json");
        entity_commands::inspect_stack(EntityInspectStackArgs {
            global: global(&proj.code_path()),
            state: state_file,
            id: "e1".into(),
            output: EntityOutputArgs {
                out: out_file.clone(),
                out_format: "json".into(),
            },
            after_event_num: None,
        })
        .unwrap();

        let val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out_file).unwrap()).unwrap();

        // Should have last_event_num and coroutines array
        assert!(val["last_event_num"].is_number());
        let coroutines = val["coroutines"].as_array().unwrap();
        assert!(!coroutines.is_empty(), "expected at least one coroutine");

        // The run coroutine should have a stack with frames
        let run_coroutine = &coroutines[0];
        assert!(run_coroutine["source_event"].is_null()); // run coroutine has no source event
        let stack = run_coroutine["stack"].as_array().unwrap();
        assert!(!stack.is_empty(), "expected non-empty call stack");

        // Bottom frame should be the entity entry point
        let bottom = &stack[0];
        assert_eq!(bottom["module"], "myapp.worker");
        assert_eq!(bottom["construct"], "worker");
        let file_str = bottom["file"].as_str().unwrap();
        assert!(
            file_str.ends_with("worker.dl"),
            "expected file to end with worker.dl, got: {file_str}"
        );
        assert!(bottom["line"].is_number(), "expected line number");

        // If there's a second frame (the get_thing call site), check it too
        if stack.len() > 1 {
            let call_frame = &stack[stack.len() - 1];
            assert_eq!(call_frame["module"], "myapp.worker");
            // get_thing() is on line 9 of the source (1-indexed)
            assert_eq!(call_frame["line"], 9);
        }
    }

    #[test]
    fn project_test_passes() {
        let proj = TestProject::new(
            "myapp",
            "math",
            "\
out func add {
    in a: int
    in b: int
    out result: int

    result = a + b
}
",
        )
        .with_test(
            "math_test",
            "\
import duralade.assert
import duralade.test
import myapp.math

@test
func add_works {
    out! :error?

    assert::equal(actual = math::add(a = 1, b = 2).result, expected = 3)!
}
",
        );

        project_commands::test(ProjectTestArgs {
            code: proj.code_path(),
            filter: None,
            no_strict: false,
        })
        .unwrap();
    }

    #[test]
    fn project_test_failure() {
        let proj = TestProject::new("myapp", "empty", "").with_test(
            "bad",
            "\
import duralade.assert
import duralade.test

@test
func always_fails {
    out! :error?

    assert::assert(in = false, message = \"intentional\")!
}
",
        );

        let result = project_commands::test(ProjectTestArgs {
            code: proj.code_path(),
            filter: None,
            no_strict: false,
        });
        assert!(result.is_err());
    }
}
