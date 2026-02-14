use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use crate::engine::ModuleIndex;
use crate::engine::native::{NativeCallContext, NativeCallError};
use crate::event::{EventType, ExternReplayResult, OwnedFields, OwnedValue};
use crate::load::registry::TypeEntry;
use crate::load::{FieldModifier, Module, TypeConstructKind};
use crate::model::*;

use super::context::EvalContext;
use super::eval::{self, EvalResult};
use super::member;
use super::scope::RuntimeScopeRef;
use super::value::{HeapValue, HeapValueFuncKind, Value};
use crate::scope::Scope;

macro_rules! fault {
    ($ctx:expr, $($arg:tt)*) => {
        eval::make_fault(format!($($arg)*), $ctx.snapshot_stack())
    }
}

macro_rules! internal_error {
    ($ctx:expr, $($arg:tt)*) => {
        eval::make_internal_error(format!($($arg)*), $ctx.snapshot_stack())
    }
}

/// Future that returns Pending once, then Ready on next poll.
/// Used to yield control back to the tick driver.
struct YieldOnce(bool);

impl Future for YieldOnce {
    type Output = ();
    fn poll(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
        if self.0 {
            Poll::Ready(())
        } else {
            self.0 = true;
            Poll::Pending
        }
    }
}

fn yield_once() -> YieldOnce {
    YieldOnce(false)
}

pub(crate) async fn call_or_construct(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    name: &str,
    input: super::scope::FuncInput,
    call_node_id: Option<NodeId>,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();

    let decl = match module.get_declaration(name) {
        Some(d) => d,
        None => {
            let path = ctx.registry.module_path(module_idx);
            return internal_error!(ctx, "'{}' not found in module {}", name, path);
        }
    };

    match decl.kind {
        TypeConstructKind::Data => {
            let decl_id = ctx.registry.decl_id_in_module(module_idx, name);
            let out_early_name = decl
                .fields
                .iter()
                .find(|f| f.modifier == Some(FieldModifier::OutEarly))
                .map(|f| f.name.clone());
            let id = ctx.heap_mut().alloc(HeapValue::Data {
                decl: decl_id,
                out_early_name,
                fields: input.args,
                type_args: input.type_args,
            });
            EvalResult::Ok(Value::Data(id))
        }
        TypeConstructKind::Func => {
            call_module_func(ctx, module_idx, name, input, call_node_id).await
        }
        TypeConstructKind::Extern => call_extern(ctx, module_idx, name, input).await,
        TypeConstructKind::Native => call_native(ctx, module_idx, name, input).await,
        TypeConstructKind::Entity => {
            member::construct_local_entity(ctx, module_idx, name, input.args).await
        }
        _ => internal_error!(ctx, "'{}' is not callable", name),
    }
}

#[tracing::instrument(level = "trace", skip_all, fields(func = %func_name))]
async fn call_module_func(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    func_name: &str,
    input: super::scope::FuncInput,
    call_node_id: Option<NodeId>,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();

    let func = match module.get_func_ast(func_name) {
        Some(f) => f,
        None => {
            return internal_error!(ctx, "AST not found for function '{func_name}'");
        }
    };
    call_func_body(
        ctx,
        &module,
        module_idx,
        func_name,
        func,
        input,
        call_node_id,
        None,
    )
    .await
}

#[tracing::instrument(level = "trace", skip_all, fields(func_name = %decl_name))]
#[allow(clippy::too_many_arguments)]
async fn call_func_body(
    ctx: &mut EvalContext,
    module: &Module,
    module_idx: ModuleIndex,
    decl_name: &str,
    func: &Func,
    input: super::scope::FuncInput,
    call_node_id: Option<NodeId>,
    parent_scope: Option<RuntimeScopeRef>,
) -> EvalResult {
    let out_early = match module.get_declaration(decl_name) {
        Some(decl) => decl.out_early_name.clone(),
        None => func.fields.iter().find_map(|field| {
            if let FieldKind::Var(fv) = &field.kind
                && fv.modifier == FieldVarModifier::OutEarly
            {
                return Some(fv.name.name.clone());
            }
            None
        }),
    };

    let (file_idx, func_decl_node_id) = module
        .get_declaration_indexed(decl_name)
        .map(|(fi, d)| (fi, d.node_id))
        .unwrap_or((0, NodeId(0)));
    let stack_frame = super::scope::StackFrame {
        module: module_idx,
        file_idx,
        call_node_id,
        decl_node_id: func_decl_node_id,
    };
    let parent = parent_scope.unwrap_or_else(|| ctx.definition_scope(module_idx, file_idx));
    let new_scope = Scope {
        kind: crate::scope::ScopeKind::Function,
        parent: Some(parent),
        name: Some(decl_name.into()),
        out_early: out_early.clone(),
        type_params: input.type_args,
        ..Default::default()
    }
    .into_ref();
    ctx.push_function_frame(new_scope, stack_frame, input.implicits);

    // Resolve field eval order: try type table (anon funcs), then declaration, then AST order.
    let field_eval_order = module
        .files
        .iter()
        .find_map(|f| {
            let &ty_id = f.type_table.node_types.get(&func.node_id)?;
            match ctx.registry.type_entry(ty_id) {
                TypeEntry::Anonymous(anon) if !anon.ast_field_eval_order.is_empty() => {
                    Some(anon.ast_field_eval_order.clone())
                }
                _ => None,
            }
        })
        .or_else(|| {
            let order = module
                .get_declaration(decl_name)
                .map(|d| d.ast_field_eval_order.clone())
                .unwrap_or_default();
            if order.is_empty() { None } else { Some(order) }
        })
        .unwrap_or_else(|| (0..func.fields.len()).collect());

    ctx.push_import_context(module_idx);

    for &field_idx in &field_eval_order {
        let field = &func.fields[field_idx];
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            match fv.modifier {
                FieldVarModifier::In | FieldVarModifier::Inout => {
                    if let Some(val) = input.args.get(name) {
                        if let Err(err) = ctx.declare_local(name.clone(), val.clone()) {
                            ctx.pop_import_context();
                            ctx.pop_scope();
                            return err;
                        }
                    } else if let Some(default_expr) = &fv.default {
                        let val = match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_import_context();
                                ctx.pop_scope();
                                return other;
                            }
                        };
                        if let Err(err) = ctx.declare_local(name.clone(), val) {
                            ctx.pop_import_context();
                            ctx.pop_scope();
                            return err;
                        }
                    }
                }
                FieldVarModifier::Out | FieldVarModifier::OutEarly => {
                    let val = if let Some(default_expr) = &fv.default {
                        match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_import_context();
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Err(err) = ctx.declare_local(name.clone(), val) {
                        ctx.pop_import_context();
                        ctx.pop_scope();
                        return err;
                    }
                }
                FieldVarModifier::Implicit => {
                    match bind_implicit_var(ctx, module, decl_name, None, fv).await {
                        EvalResult::Ok(_) => {}
                        err => {
                            ctx.pop_import_context();
                            ctx.pop_scope();
                            return err;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    let mut result = eval::exec_stmts(ctx, &func.stmts).await;

    if !matches!(result, EvalResult::Fault(_) | EvalResult::InternalError(_)) {
        let defer_result = eval::run_deferred(ctx).await;
        if matches!(
            defer_result,
            EvalResult::Fault(_) | EvalResult::InternalError(_)
        ) {
            result = defer_result;
        }
    }

    ctx.pop_import_context();

    let return_value = match result {
        EvalResult::Ok(_) | EvalResult::Return => {
            let mut fields = BTreeMap::new();
            if let Some(decl) = module.get_declaration(decl_name) {
                for name in &decl.out_field_names {
                    if let EvalResult::Ok(val) = ctx.lookup(name) {
                        fields.insert(name.clone(), val);
                    }
                }
            } else {
                // Anonymous func fallback - derive from AST fields.
                for field in &func.fields {
                    if let FieldKind::Var(fv) = &field.kind
                        && (fv.modifier == FieldVarModifier::Out
                            || fv.modifier == FieldVarModifier::Inout
                            || fv.modifier == FieldVarModifier::OutEarly)
                        && let EvalResult::Ok(val) = ctx.lookup(&fv.name.name)
                    {
                        fields.insert(fv.name.name.clone(), val);
                    }
                }
            }
            let id = ctx.heap_mut().alloc(HeapValue::Data {
                decl: None,
                out_early_name: out_early,
                fields,
                type_args: BTreeMap::new(),
            });
            EvalResult::Ok(Value::Data(id))
        }
        err => err,
    };

    ctx.pop_scope();
    return_value
}

#[tracing::instrument(level = "trace", skip_all)]
pub(crate) async fn call_func_value(
    ctx: &mut EvalContext,
    func_id: super::value::ValueId,
    args: BTreeMap<Symbol, Value>,
    call_node_id: Option<NodeId>,
) -> EvalResult {
    let (func_module, func_kind) = {
        match ctx.heap().get(func_id) {
            HeapValue::Func { module, kind } => (*module, kind.clone()),
            _ => return internal_error!(ctx, "attempted to call non-function value"),
        }
    };

    match func_kind {
        HeapValueFuncKind::Closure {
            captured_scope,
            body,
            ..
        } => {
            let module = ctx.registry.module(func_module).clone();
            let input = super::scope::FuncInput {
                args,
                ..Default::default()
            };
            call_func_body(
                ctx,
                &module,
                func_module,
                &Symbol::anon_func(),
                &body,
                input,
                call_node_id,
                Some(captured_scope),
            )
            .await
        }
        HeapValueFuncKind::Decl { decl, name, target } => {
            if let Some(target) = target {
                super::member::eval_data_member_call(
                    ctx,
                    target,
                    decl,
                    &name,
                    args,
                    BTreeMap::new(),
                )
                .await
            } else {
                let input = super::scope::FuncInput {
                    args,
                    ..Default::default()
                };
                call_or_construct(ctx, func_module, &name, input, call_node_id).await
            }
        }
    }
}

#[tracing::instrument(level = "debug", skip_all, fields(extern_name = %extern_name))]
async fn call_extern(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    extern_name: &str,
    input: super::scope::FuncInput,
) -> EvalResult {
    let full_name = ctx.registry.qualify_decl(module_idx, extern_name);

    let owned_args: OwnedFields = {
        let mut map = OwnedFields::new();
        for (k, v) in &input.args {
            match OwnedValue::from_value(v, &ctx.heap()) {
                Ok(ov) => {
                    map.insert(k.to_string(), ov);
                }
                Err(e) => {
                    return internal_error!(ctx, "extern '{}' arg '{}': {}", extern_name, k, e);
                }
            }
        }
        map
    };

    let event_log = ctx.event_log().clone();
    let replay_result = event_log.borrow_mut().try_replay_extern(&full_name);

    let invoke_event_num = match replay_result {
        ExternReplayResult::Completed(result_fields) => {
            tracing::trace!("extern replay: completed");
            return extern_result_to_value(ctx, module_idx, extern_name, result_fields);
        }
        ExternReplayResult::Pending(event_num) => {
            tracing::trace!("extern replay: pending");
            event_num
        }
        ExternReplayResult::Diverged(msg) => {
            // TODO: include structured divergence details (expected vs actual extern name, event num, args)
            return fault!(ctx, "{}", msg);
        }
        ExternReplayResult::NotFound => {
            tracing::trace!("extern replay: not found, emitting ExternInvoke");
            event_log.borrow_mut().append(EventType::ExternInvoke {
                extern_name: full_name,
                args: owned_args,
            })
        }
    };

    // Yield and recheck until the extern is completed.
    loop {
        yield_once().await;
        let completed = event_log.borrow().check_extern_complete(invoke_event_num);
        if let Some(result_fields) = completed {
            return extern_result_to_value(ctx, module_idx, extern_name, result_fields);
        }
    }
}

fn extern_result_to_value(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    extern_name: &str,
    mut result_fields: OwnedFields,
) -> EvalResult {
    tracing::trace!(
        extern_name = %extern_name,
        field_count = result_fields.len(),
        "extern result materialized"
    );

    // Look up the extern declaration to validate and fill result fields.
    let mut out_early_name: Option<Symbol> = None;
    let module = ctx.registry.module(module_idx);
    if let Some(decl) = module.get_declaration(extern_name) {
        let out_fields: Vec<_> = decl
            .fields
            .iter()
            .filter(|f| {
                f.modifier == Some(FieldModifier::Out)
                    || f.modifier == Some(FieldModifier::OutEarly)
            })
            .collect();

        out_early_name = out_fields
            .iter()
            .find(|f| f.modifier == Some(FieldModifier::OutEarly))
            .map(|f| f.name.clone());

        // Reject extra fields not declared on the extern.
        for key in result_fields.keys() {
            if !out_fields.iter().any(|f| f.name == key.as_str()) {
                return fault!(
                    ctx,
                    "extern '{}' result contains undeclared field '{}'",
                    extern_name,
                    key
                );
            }
        }

        // Fill missing fields: nil for nilable types, error for required.
        for field in &out_fields {
            let field_name = &field.name;
            if !result_fields.contains_key(field_name.as_str()) {
                if ctx.registry.is_nilable(field.ty) {
                    result_fields.insert(field_name.to_string(), OwnedValue::Nil);
                } else {
                    return fault!(
                        ctx,
                        "extern '{}' result missing required field '{}'",
                        extern_name,
                        field_name
                    );
                }
            }
        }
    }

    let decl_fields = module
        .get_declaration(extern_name)
        .map(|d| d.fields.as_slice())
        .unwrap_or(&[]);
    let any_type = ctx.registry.any_type;
    let mut fields = BTreeMap::new();
    for (k, v) in result_fields {
        let field_ty = decl_fields
            .iter()
            .find(|f| {
                f.name == *k
                    && matches!(
                        f.modifier,
                        Some(FieldModifier::Out)
                            | Some(FieldModifier::OutEarly)
                            | Some(FieldModifier::Inout)
                    )
            })
            .map(|f| f.ty)
            .unwrap_or(any_type);
        match v.into_value(field_ty, &ctx.registry, &mut ctx.heap_mut()) {
            Ok(val) => {
                fields.insert(k.into(), val);
            }
            Err(e) => {
                return internal_error!(
                    ctx,
                    "extern '{}' result field '{}': {}",
                    extern_name,
                    k,
                    e
                );
            }
        }
    }
    let id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields,
        type_args: BTreeMap::new(),
    });
    EvalResult::Ok(Value::Data(id))
}

#[tracing::instrument(level = "debug", skip_all, fields(native_name = %native_name))]
async fn call_native(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    native_name: &str,
    input: super::scope::FuncInput,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();
    // .dli files parse funcs as ConstructKind::Func; the loader converts the
    // TypeConstruct to TypeConstructKind::Native but leaves the AST unchanged.  Accept
    // either AST kind here so module-level .dli natives resolve correctly.
    let fields = module
        .files
        .iter()
        .flat_map(|f| f.source_file.constructs.iter())
        .find_map(|c| {
            if c.name.name != *native_name {
                return None;
            }
            match &c.kind {
                ConstructKind::Native(n) => Some(n.fields.as_slice()),
                ConstructKind::Func(f) => Some(f.fields.as_slice()),
                _ => None,
            }
        });
    let Some(fields) = fields else {
        return internal_error!(ctx, "AST not found for native '{native_name}'");
    };
    let Some(decl) = module.get_declaration(native_name) else {
        return internal_error!(ctx, "declaration not found for native '{native_name}'");
    };

    let full_name = ctx.registry.qualify_decl(module_idx, native_name);

    // Push a function scope so native calls participate in implicit resolution.
    let (file_idx, decl_node_id) = module
        .get_declaration_indexed(native_name)
        .map(|(fi, d)| (fi, d.node_id))
        .unwrap_or((0, NodeId(0)));
    let native_scope = Scope {
        kind: crate::scope::ScopeKind::Function,
        parent: Some(ctx.definition_scope(module_idx, file_idx)),
        name: Some(native_name.into()),
        ..Default::default()
    }
    .into_ref();
    ctx.push_function_frame(
        native_scope,
        super::scope::StackFrame {
            module: module_idx,
            file_idx,
            call_node_id: None,
            decl_node_id,
        },
        input.implicits,
    );

    let mut resolved_args = BTreeMap::new();
    let mut input_names = std::collections::BTreeSet::new();
    for field in fields {
        let FieldKind::Var(var) = &field.kind else {
            continue;
        };
        if !matches!(var.modifier, FieldVarModifier::In | FieldVarModifier::Inout) {
            continue;
        }

        let name = &var.name.name;
        input_names.insert(name.clone());
        if let Some(v) = input.args.get(name) {
            resolved_args.insert(name.clone(), v.clone());
        } else if let Some(default_expr) = &var.default {
            let value = match eval::eval_expr(ctx, default_expr).await {
                EvalResult::Ok(v) => v,
                other => {
                    ctx.pop_scope();
                    return other;
                }
            };
            resolved_args.insert(name.clone(), value);
        } else {
            ctx.pop_scope();
            return internal_error!(
                ctx,
                "native '{}' missing required argument '{}' (loader/type-check bug)",
                native_name,
                name
            );
        }
    }

    for key in input.args.keys() {
        if !input_names.contains(key) {
            ctx.pop_scope();
            return internal_error!(
                ctx,
                "native '{}' received undeclared argument '{}' (loader/type-check bug)",
                native_name,
                key
            );
        }
    }

    let Some(handler) = ctx.native_registry().get(&full_name) else {
        ctx.pop_scope();
        return fault!(ctx, "native handler '{}' is not registered", full_name);
    };

    let mut native_out: BTreeMap<Symbol, Value> = BTreeMap::new();
    let call_result = {
        let mut native_ctx = NativeCallContext::new(ctx);
        handler
            .call(&mut native_ctx, &resolved_args, &mut native_out)
            .await
    };

    match call_result {
        Ok(()) => {}
        Err(NativeCallError::Fault(mut err)) => {
            if err.stack.is_empty() {
                err.stack = ctx.snapshot_stack();
            }
            ctx.pop_scope();
            return EvalResult::Fault(err);
        }
        Err(NativeCallError::InternalError(mut err)) => {
            if err.stack.is_empty() {
                err.stack = ctx.snapshot_stack();
            }
            ctx.pop_scope();
            return EvalResult::InternalError(err);
        }
        Err(NativeCallError::ControlFlow(result)) => {
            ctx.pop_scope();
            return result;
        }
    }

    let out_fields: Vec<_> = decl
        .fields
        .iter()
        .filter(|f| {
            f.modifier == Some(crate::load::FieldModifier::Out)
                || f.modifier == Some(crate::load::FieldModifier::OutEarly)
        })
        .collect();
    for key in native_out.keys() {
        if !out_fields.iter().any(|f| f.name == key.as_str()) {
            ctx.pop_scope();
            return fault!(
                ctx,
                "native '{}' result contains undeclared field '{}'",
                native_name,
                key
            );
        }
    }
    for field in &out_fields {
        let field_name = &field.name;
        if !native_out.contains_key(field_name) {
            if ctx.registry.is_nilable(field.ty) {
                native_out.insert(field_name.clone(), Value::Nil);
            } else {
                ctx.pop_scope();
                return fault!(
                    ctx,
                    "native '{}' result missing required field '{}'",
                    native_name,
                    field_name
                );
            }
        }
    }

    ctx.pop_scope();

    let out_early_name = decl
        .fields
        .iter()
        .find(|f| f.modifier == Some(FieldModifier::OutEarly))
        .map(|f| f.name.clone());
    let id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields: native_out,
        type_args: BTreeMap::new(),
    });
    EvalResult::Ok(Value::Data(id))
}

#[tracing::instrument(level = "debug", skip_all, fields(name = %full_name))]
pub(crate) async fn call_system_extern(
    ctx: &mut EvalContext,
    full_name: &str,
    owned_args: OwnedFields,
) -> EvalResult {
    let event_log = ctx.event_log().clone();
    let replay_result = event_log.borrow_mut().try_replay_extern(full_name);

    let invoke_event_num = match replay_result {
        ExternReplayResult::Completed(result_fields) => {
            return system_extern_result_to_value(ctx, full_name, result_fields);
        }
        ExternReplayResult::Pending(event_num) => event_num,
        ExternReplayResult::Diverged(msg) => {
            // TODO: include structured divergence details (expected vs actual extern name, event num, args)
            return fault!(ctx, "{}", msg);
        }
        ExternReplayResult::NotFound => event_log.borrow_mut().append(EventType::ExternInvoke {
            extern_name: full_name.to_string(),
            args: owned_args,
        }),
    };

    loop {
        yield_once().await;
        let completed = event_log.borrow().check_extern_complete(invoke_event_num);
        if let Some(result_fields) = completed {
            return system_extern_result_to_value(ctx, full_name, result_fields);
        }
    }
}

fn system_extern_result_to_value(
    ctx: &mut EvalContext,
    full_name: &str,
    result_fields: OwnedFields,
) -> EvalResult {
    // System extern names use `<root>.<module...>::<name>` format.
    let (module_path, name) = full_name.rsplit_once("::").unwrap_or(("", full_name));
    // TODO: replace hardcoded system extern signatures with .dli files
    let out_early_name = ctx
        .registry
        .module_index(module_path)
        .and_then(|idx| ctx.registry.module(idx).get_declaration(name))
        .and_then(|d| {
            d.fields
                .iter()
                .find(|f| f.modifier == Some(FieldModifier::OutEarly))
                .map(|f| f.name.clone())
        })
        .or_else(|| {
            // Hardcoded system extern out! fields - replace with .dli files
            match full_name {
                "duralade.entity::spawn" | "duralade.entity::call" | "duralade.entity::result" => {
                    Some(Symbol::error())
                }
                _ => None,
            }
        });
    let decl_fields = ctx
        .registry
        .module_index(module_path)
        .and_then(|idx| ctx.registry.module(idx).get_declaration(name))
        .map(|d| d.fields.as_slice())
        .unwrap_or(&[]);
    let mut fields = BTreeMap::new();
    for (k, v) in result_fields {
        let field_ty = decl_fields
            .iter()
            .find(|f| {
                f.name == *k
                    && matches!(
                        f.modifier,
                        Some(FieldModifier::Out)
                            | Some(FieldModifier::OutEarly)
                            | Some(FieldModifier::Inout)
                    )
            })
            .map(|f| f.ty)
            .unwrap_or(ctx.registry.any_type);
        match v.into_value(field_ty, &ctx.registry, &mut ctx.heap_mut()) {
            Ok(val) => {
                fields.insert(k.into(), val);
            }
            Err(e) => {
                return internal_error!(
                    ctx,
                    "system extern '{}' result field '{}': {}",
                    full_name,
                    k,
                    e
                );
            }
        }
    }
    let id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields,
        type_args: BTreeMap::new(),
    });
    EvalResult::Ok(Value::Data(id))
}

/// Bind an implicit field variable from the implicit context.
/// Returns `EvalResult::Ok` on success (value bound in ctx), error/early-return otherwise.
pub(crate) async fn bind_implicit_var(
    ctx: &mut EvalContext,
    module: &Module,
    decl_name: &str,
    member_name: Option<&str>,
    fv: &FieldVar,
) -> EvalResult {
    let name = &fv.name.name;
    let Some(decl) = module.get_declaration(decl_name) else {
        return EvalResult::Ok(Value::Nil);
    };
    let target_decl = match member_name {
        Some(member_name) => decl.get_member(member_name),
        None => Some(decl),
    };
    let Some(ty) = target_decl.and_then(|decl| decl.get_implicit_field_type(name)) else {
        return EvalResult::Ok(Value::Nil);
    };
    if let Some(val) = ctx.lookup_implicit(ty) {
        if !ctx.registry.is_nilable(ty) && matches!(val, Value::Nil) {
            return fault!(ctx, "implicit value for '{name}' is nil");
        }
        if let Err(err) = ctx.declare_local(name.clone(), val) {
            return err;
        }
    } else if let Some(default_expr) = &fv.default {
        let val = match eval::eval_expr(ctx, default_expr).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        if let Err(err) = ctx.declare_local(name.clone(), val) {
            return err;
        }
    } else {
        return fault!(ctx, "no implicit value found for '{name}'");
    }
    EvalResult::Ok(Value::Nil)
}
