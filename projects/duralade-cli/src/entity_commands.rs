use duralade_language::engine::{Engine, EngineError, EntityStatus};
use duralade_language::event::EventType;
use duralade_language::interpret::tick::TickStatus;

use crate::engine::{EngineHandle, EngineOptions};
use crate::state;
use crate::{
    EntityCancelArgs, EntityCancelFuncArgs, EntityCheckpointArgs, EntityCompleteExternArgs,
    EntityDescribeArgs, EntityInspectStackArgs, EntityInvokeFuncArgs, EntityInvokeFuncNoblockArgs,
    EntityReplayArgs, EntityShellArgs, EntitySpawnArgs, EntityTerminateArgs, EntityTickArgs,
    EntityViewArgs,
};

fn format_engine_error(e: EngineError) -> String {
    match e {
        EngineError::Fault { error, .. } => format!("fault: {}", error.message),
        EngineError::InternalError { error, .. } => {
            let mut msg = format!("internal error: {}", error.message);
            if !error.rust_location.is_empty() {
                msg.push_str(&format!("\n  at {}", error.rust_location));
            }
            msg.push_str("\n(this is a bug in duralade)");
            msg
        }
        other => other.to_string(),
    }
}

pub fn spawn(args: EntitySpawnArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let input = state::parse_input(&args.input)?;
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;
    let entity_type = handle.qualify_entity(&args.entity);

    let result = handle
        .engine
        .spawn_entity(
            args.id,
            entity_type,
            input,
            Vec::new(),
            time_ms,
            args.no_tick,
        )
        .block()
        .map_err(format_engine_error)?;

    match &result.status {
        TickStatus::Completed => eprintln!("entity completed"),
        TickStatus::Blocked => {
            if args.no_tick {
                eprintln!("entity created (tick skipped)");
            } else {
                eprintln!("entity blocked (waiting on extern)");
            }
        }
    }

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn tick(args: EntityTickArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    if let Some(id) = &args.id {
        let result = handle
            .engine
            .tick_entity(id, time_ms)
            .block()
            .map_err(format_engine_error)?;
        match &result.status {
            TickStatus::Completed => {
                eprintln!("entity '{id}' completed");
                if let Some(result) = result.new_events.iter().find_map(|e| {
                    if let EventType::EntityComplete { result, .. } = &e.event_type {
                        result.as_ref()
                    } else {
                        None
                    }
                }) {
                    let json = state::owned_map_to_json(result);
                    eprintln!("result: {json}");
                }
            }
            TickStatus::Blocked => {
                let new_extern_invokes: Vec<_> = result
                    .new_events
                    .iter()
                    .filter_map(|e| match &e.event_type {
                        EventType::ExternInvoke { extern_name, .. } => {
                            Some(format!("  event {}: {extern_name}", e.num))
                        }
                        _ => None,
                    })
                    .collect();
                if new_extern_invokes.is_empty() {
                    eprintln!("entity '{id}' blocked");
                } else {
                    eprintln!("entity '{id}' blocked, pending externs:");
                    for line in &new_extern_invokes {
                        eprintln!("{line}");
                    }
                }
            }
        }
    } else {
        let results = handle
            .engine
            .tick_all(time_ms)
            .block()
            .map_err(format_engine_error)?;
        if results.is_empty() {
            eprintln!("no entities in store");
        } else {
            for result in &results {
                if let TickStatus::Completed = &result.status {
                    eprintln!("entity completed");
                }
            }
        }
    }

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn describe(args: EntityDescribeArgs) -> Result<(), String> {
    // Describe only reads raw events and derives status - no instance reconstruction needed.
    let handle = EngineHandle::new(EngineOptions {
        code: None,
        state: &args.state,
        state_out: None,
        dry_run: false,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    let events = handle
        .engine
        .entity_events(&args.id)
        .block()
        .map_err(format_engine_error)?;
    let status = handle
        .engine
        .entity_status(&args.id)
        .block()
        .map_err(format_engine_error)?;

    let (id, entity_type) = match events.first().map(|e| &e.event_type) {
        Some(EventType::EntityInvoke { id, entity, .. }) => (id.clone(), entity.clone()),
        _ => return Err("invalid state: first event must be EntityInvoke".into()),
    };

    let mut output = serde_json::json!({
        "id": id,
        "entity": entity_type,
    });

    match &status {
        EntityStatus::Running => {
            output["status"] = "running".into();
        }
        EntityStatus::Completed { result } => {
            output["status"] = "completed".into();
            if !args.no_result
                && let Some(result) = result
            {
                output["result"] = state::owned_map_to_json(result);
            }
        }
        EntityStatus::Terminated => {
            output["status"] = "terminated".into();
        }
    }

    state::write_output(&args.output.out, &output, &args.output.out_format)
}

pub fn view(args: EntityViewArgs) -> Result<(), String> {
    let handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.state,
        state_out: None,
        dry_run: false,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    if args.field_all {
        let result = handle
            .engine
            .view_out_fields(&args.id, args.after_event_num)
            .block()
            .map_err(format_engine_error)?;
        let json = state::owned_map_to_json(&result.value);
        state::write_output(&args.output.out, &json, &args.output.out_format)
    } else if !args.field.is_empty() {
        let mut out = serde_json::Map::new();
        for name in &args.field {
            let result = handle
                .engine
                .view_out_field(&args.id, name, args.after_event_num)
                .block()
                .map_err(format_engine_error)?;
            if let Some(val) = result.value {
                out.insert(name.clone(), state::owned_to_json(&val));
            }
        }
        let json = serde_json::Value::Object(out);
        state::write_output(&args.output.out, &json, &args.output.out_format)
    } else if let Some(func) = &args.func {
        let input = state::parse_input(&args.input)?;
        let result = handle
            .engine
            .invoke_view_func(&args.id, func, input, Vec::new(), args.after_event_num)
            .block()
            .map_err(format_engine_error)?;
        let json = state::owned_map_to_json(&result.value);
        state::write_output(&args.output.out, &json, &args.output.out_format)
    } else {
        Err("specify --field, --field-all, or --func".into())
    }
}

pub fn replay(args: EntityReplayArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.state,
        state_out: None,
        dry_run: false,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    let status = handle
        .engine
        .replay_entity(&args.id, time_ms)
        .block()
        .map_err(format_engine_error)?;

    match status {
        TickStatus::Completed => eprintln!("replay ok: entity completed"),
        TickStatus::Blocked => eprintln!("replay ok: entity blocked"),
    }
    Ok(())
}

pub fn inspect_stack(args: EntityInspectStackArgs) -> Result<(), String> {
    let handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.state,
        state_out: None,
        dry_run: false,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    let result = handle
        .engine
        .inspect_stacks(&args.id, args.after_event_num)
        .block()
        .map_err(format_engine_error)?;

    let json = serde_json::json!({
        "last_event_num": result.last_event_num,
        "coroutines": result.value,
    });
    state::write_output(&args.output.out, &json, &args.output.out_format)
}

pub fn invoke_func(args: EntityInvokeFuncArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let input = state::parse_input(&args.input)?;
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    handle
        .engine
        .invoke_func(&args.id, &args.func, args.request_id, input, time_ms)
        .block()
        .map_err(format_engine_error)?;

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn invoke_func_noblock(args: EntityInvokeFuncNoblockArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let input = state::parse_input(&args.input)?;
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    let func_invoke_num = handle
        .engine
        .invoke_func(&args.id, &args.func, None, input, time_ms)
        .block()
        .map_err(format_engine_error)?;

    let result = handle
        .engine
        .tick_entity(&args.id, time_ms)
        .block()
        .map_err(format_engine_error)?;

    let func_result = result.new_events.iter().find_map(|e| {
        if let EventType::FuncComplete { invoke_num, result } = &e.event_type
            && *invoke_num == func_invoke_num
        {
            return Some(result);
        }
        None
    });

    if let Some(result) = func_result {
        let json = state::owned_map_to_json(result);
        state::write_output(&args.output.out, &json, &args.output.out_format)?;
    }

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn complete_extern(args: EntityCompleteExternArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let result = state::parse_result(&args.result)?;
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    handle
        .engine
        .complete_extern(&args.id, args.invoke_num, result, time_ms)
        .block()
        .map_err(format_engine_error)?;

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn cancel(args: EntityCancelArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    handle
        .engine
        .cancel_entity(&args.id, None, time_ms)
        .block()
        .map_err(format_engine_error)?;

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn cancel_func(args: EntityCancelFuncArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    handle
        .engine
        .cancel_func(&args.id, &args.request_id, time_ms)
        .block()
        .map_err(format_engine_error)?;

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn checkpoint(_args: EntityCheckpointArgs) -> Result<(), String> {
    Err("checkpoint is not yet implemented".into())
}

pub fn terminate(args: EntityTerminateArgs) -> Result<(), String> {
    let time_ms = crate::resolve_time(args.global.current_time);
    let mut handle = EngineHandle::new(EngineOptions {
        code: Some(&args.global.code),
        state: &args.store.state,
        state_out: args.store.state_out.as_deref(),
        dry_run: args.store.dry_run,
        strict: !args.global.no_strict,
        disable_heap_collect: true,
    })?;

    handle
        .engine
        .terminate_entity(&args.id, time_ms)
        .block()
        .map_err(format_engine_error)?;

    handle
        .engine
        .finalize()
        .block()
        .map_err(format_engine_error)
}

pub fn shell(_args: EntityShellArgs) -> Result<(), String> {
    Err("shell is not yet implemented".into())
}

/// Extension trait to synchronously resolve futures that are immediately ready
/// (e.g. JsonFileStore's in-memory operations).
pub(crate) trait BlockReady {
    type Output;
    fn block(self) -> Self::Output;
}

impl<F: std::future::Future> BlockReady for F {
    type Output = F::Output;
    fn block(self) -> F::Output {
        let mut pinned = std::pin::pin!(self);
        let waker = std::task::Waker::noop();
        let mut cx = std::task::Context::from_waker(waker);
        match pinned.as_mut().poll(&mut cx) {
            std::task::Poll::Ready(v) => v,
            std::task::Poll::Pending => panic!("store future unexpectedly pending"),
        }
    }
}
