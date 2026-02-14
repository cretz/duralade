use std::collections::BTreeMap;

use crate::engine::ModuleIndex;
use crate::engine::native::{NativeCallContext, NativeCallError};
use crate::load::registry::DeclId;
use crate::load::{FieldModifier, Module, TypeConstructKind};
use crate::model::{ConstructKind, Entity, Field, FieldKind, FieldVarModifier, NodeId, Symbol};

use super::call;
use super::context::EvalContext;
use super::eval::{self, EvalResult};
use super::scope::{ConstructScope, FuncInput, ImplicitBinding, StackFrame};
use super::value::{HeapValue, Value, ValueId};
use crate::scope::Scope;

fn internal_error_result(ctx: &EvalContext, message: impl Into<String>) -> EvalResult {
    eval::make_internal_error(message.into(), ctx.snapshot_stack())
}

fn fault_result(ctx: &EvalContext, message: impl Into<String>) -> EvalResult {
    eval::make_fault(message.into(), ctx.snapshot_stack())
}

/// Like find_data_member_func but returns just the fields slice.
fn find_data_member_fields<'a>(
    module: &'a Module,
    data_name: &str,
    member_name: &str,
) -> Option<&'a [Field]> {
    module
        .get_data_member_ast(data_name, member_name)
        .map(|c| match &c.kind {
            ConstructKind::Native(n) => n.fields.as_slice(),
            ConstructKind::Func(f) => f.fields.as_slice(),
            _ => &[],
        })
}

/// Shared entity setup: push entity scope, bind entity-level fields, run init.
/// When `construct_value_id` is provided, fields are written to the heap entity
/// (used by engine-level entities with run blocks). Otherwise fields are bound
/// as scope vars (used by local entities without run blocks).
/// Caller must pop the entity scope when done.
async fn setup_entity_scope(
    ctx: &mut EvalContext,
    module: &Module,
    module_idx: ModuleIndex,
    entity: &Entity,
    entity_name: &str,
    args: BTreeMap<Symbol, Value>,
    construct_value_id: Option<ValueId>,
) -> EvalResult {
    let field_eval_order: Vec<usize> = module
        .get_declaration(entity_name)
        .map(|d| d.ast_field_eval_order.clone())
        .filter(|o| !o.is_empty())
        .unwrap_or_else(|| (0..entity.fields.len()).collect());

    let Some((def_file_idx, _)) = module.get_declaration_indexed(entity_name) else {
        return internal_error_result(
            ctx,
            format!("declaration not found for entity '{entity_name}'"),
        );
    };
    let def_scope = ctx.definition_scope(module_idx, def_file_idx);
    if let Some(vid) = construct_value_id {
        let decl_id = ctx
            .registry
            .decl_id_in_module(module_idx, entity_name)
            .expect("entity declaration not registered");
        ctx.push_exec_frame(
            Scope {
                kind: crate::scope::ScopeKind::Construct,
                parent: Some(def_scope),
                name: Some(entity_name.into()),
                construct: Some(ConstructScope {
                    decl: decl_id,
                    heap_id: Some(vid),
                    target: Value::Nil,
                }),
                ..Default::default()
            }
            .into_ref(),
        );
    } else {
        ctx.push_exec_frame(
            Scope {
                kind: crate::scope::ScopeKind::Block,
                parent: Some(def_scope),
                ..Default::default()
            }
            .into_ref(),
        );
    }

    for &field_idx in &field_eval_order {
        let field = &entity.fields[field_idx];
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            match fv.modifier {
                FieldVarModifier::In | FieldVarModifier::Inout => {
                    let val = if let Some(val) = args.get(name) {
                        val.clone()
                    } else if let Some(default_expr) = &fv.default {
                        match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Some(vid) = construct_value_id {
                        let mut h = ctx.heap_mut();
                        if let HeapValue::Entity { fields, .. } = h.get_mut(vid) {
                            fields.insert(name.clone(), val);
                        }
                    } else if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                FieldVarModifier::Value => {
                    let val = if let Some(default_expr) = &fv.default {
                        match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Some(vid) = construct_value_id {
                        let mut h = ctx.heap_mut();
                        if let HeapValue::Entity { fields, .. } = h.get_mut(vid) {
                            fields.insert(name.clone(), val);
                        }
                    } else if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                FieldVarModifier::Out | FieldVarModifier::OutEarly => {
                    let val = if let Some(default_expr) = &fv.default {
                        match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Some(vid) = construct_value_id {
                        let mut h = ctx.heap_mut();
                        if let HeapValue::Entity { fields, .. } = h.get_mut(vid) {
                            fields.insert(name.clone(), val);
                        }
                    } else if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                FieldVarModifier::Implicit => {
                    match call::bind_implicit_var(ctx, module, entity_name, None, fv).await {
                        EvalResult::Ok(_) => {}
                        err => {
                            ctx.pop_scope();
                            return err;
                        }
                    }
                }
            }
        }
    }

    // Run init block if present. Init is a member func - gets its own scope
    // so local vars don't land in the heap entity.
    if let Some(init) = &entity.init {
        ctx.push_exec_frame(
            Scope {
                kind: crate::scope::ScopeKind::Function,
                parent: Some(ctx.current_scope_ref()),
                name: Some(Symbol::init()),
                ..Default::default()
            }
            .into_ref(),
        );

        for field in &init.fields {
            if let FieldKind::Var(fv) = &field.kind
                && fv.modifier == FieldVarModifier::Implicit
            {
                match call::bind_implicit_var(ctx, module, entity_name, Some("init"), fv).await {
                    EvalResult::Ok(_) => {}
                    err => {
                        ctx.pop_scope();
                        ctx.pop_scope();
                        return err;
                    }
                }
            }
        }

        ctx.push_import_context(module_idx);
        let result = eval::exec_stmts(ctx, &init.stmts).await;
        ctx.pop_import_context();

        ctx.pop_scope();

        if !matches!(result, EvalResult::Ok(_)) {
            ctx.pop_scope();
            return result;
        }
    }

    EvalResult::Ok(Value::Nil)
}

/// Construct a local entity (no `run` block). Returns `Value::Entity(id)`.
pub(crate) async fn construct_local_entity(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    entity_name: &str,
    args: BTreeMap<Symbol, Value>,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();

    let entity = match module.get_entity_ast(entity_name) {
        Some(e) => e,
        None => {
            return internal_error_result(
                ctx,
                format!(
                    "entity '{}' not found in module {}",
                    entity_name,
                    ctx.registry.module_path(module_idx)
                ),
            );
        }
    };

    if entity.run.is_some() {
        return internal_error_result(
            ctx,
            format!(
                "entity '{}' has a run block and cannot be constructed locally - use spawn",
                entity_name
            ),
        );
    }

    match setup_entity_scope(ctx, &module, module_idx, entity, entity_name, args, None).await {
        EvalResult::Ok(_) => {}
        err => return err,
    }

    let mut fields = BTreeMap::new();
    for field in &entity.fields {
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            if let EvalResult::Ok(val) = ctx.lookup(name) {
                fields.insert(name.clone(), val);
            }
        }
    }

    ctx.pop_scope();

    let decl_id = ctx
        .registry
        .decl_id_in_module(module_idx, entity_name)
        .expect("entity declaration not registered");

    let id = ctx.heap_mut().alloc(HeapValue::Entity {
        decl: decl_id,
        fields,
    });

    EvalResult::Ok(Value::Entity(id))
}

/// Execute an entity's `run` block. The caller provides a pre-allocated
/// `entity_value_id` on the heap so entity fields are written there directly
/// and are accessible by member funcs via `construct_value_id`.
#[tracing::instrument(level = "debug", skip_all, fields(entity = %entity_name))]
pub(crate) async fn run_entity(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    entity_name: &str,
    args: BTreeMap<Symbol, Value>,
    entity_value_id: ValueId,
    initial_entry_implicits: Vec<ImplicitBinding>,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();

    let entity = match module.get_entity_ast(entity_name) {
        Some(e) => e,
        None => {
            return internal_error_result(
                ctx,
                format!(
                    "entity '{}' not found in module {}",
                    entity_name,
                    ctx.registry.module_path(module_idx)
                ),
            );
        }
    };

    let run = match module.get_entity_run_ast(entity_name) {
        Some(r) => r,
        None => {
            return internal_error_result(
                ctx,
                format!("entity '{}' has no run block", entity_name),
            );
        }
    };

    // out_early is needed before exec (for ScopeKind::Function).
    // out_field_names is deferred to after exec - re-derived from the run AST
    // (via the Arc<Module>) to avoid cloning Vec<String> on every call.
    let out_early = run.fields.iter().find_map(|field| {
        if let FieldKind::Var(fv) = &field.kind
            && fv.modifier == FieldVarModifier::OutEarly
        {
            return Some(fv.name.name.clone());
        }
        None
    });

    match setup_entity_scope(
        ctx,
        &module,
        module_idx,
        entity,
        entity_name,
        args,
        Some(entity_value_id),
    )
    .await
    {
        EvalResult::Ok(_) => {}
        err => return err,
    }

    let (file_idx, decl_node_id) = module
        .get_declaration_indexed(entity_name)
        .map(|(fi, d)| (fi, d.node_id))
        .unwrap_or((0, NodeId(0)));
    ctx.push_function_frame(
        Scope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(ctx.current_scope_ref()),
            name: Some(Symbol::run()),
            out_early: out_early.clone(),
            ..Default::default()
        }
        .into_ref(),
        StackFrame {
            module: module_idx,
            file_idx,
            call_node_id: None,
            decl_node_id,
        },
        initial_entry_implicits,
    );

    for field in &run.fields {
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            match fv.modifier {
                FieldVarModifier::Out | FieldVarModifier::OutEarly => {
                    let val = if let Some(default_expr) = &fv.default {
                        match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_scope();
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                FieldVarModifier::Implicit => {
                    match call::bind_implicit_var(ctx, &module, entity_name, Some("run"), fv).await
                    {
                        EvalResult::Ok(_) => {}
                        err => {
                            ctx.pop_scope();
                            ctx.pop_scope();
                            return err;
                        }
                    }
                }
                _ => {}
            }
        }
    }

    ctx.push_import_context(module_idx);
    let mut result = eval::exec_stmts(ctx, &run.stmts).await;
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
            for field in &run.fields {
                if let FieldKind::Var(fv) = &field.kind
                    && matches!(
                        fv.modifier,
                        FieldVarModifier::Out | FieldVarModifier::OutEarly
                    )
                    && let EvalResult::Ok(val) = ctx.lookup(&fv.name.name)
                {
                    fields.insert(fv.name.name.clone(), val);
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
    ctx.pop_scope();
    return_value
}

/// Dispatch a member function call on a local entity value.
pub(crate) async fn eval_local_entity_method(
    ctx: &mut EvalContext,
    entity_vid: ValueId,
    method: &str,
    args: BTreeMap<Symbol, Value>,
) -> EvalResult {
    let (entity_name, module_idx) = {
        let heap = ctx.heap();
        let hv = heap.get(entity_vid);
        match hv {
            HeapValue::Entity { decl, .. } => {
                let entry = ctx.registry.decl(*decl);
                (entry.name.clone(), entry.module)
            }
            _ => return internal_error_result(ctx, "expected Entity heap value"),
        }
    };

    let input = FuncInput {
        args,
        ..Default::default()
    };
    run_entity_member_func(
        ctx,
        module_idx,
        &entity_name,
        method,
        input,
        BTreeMap::new(),
        entity_vid,
    )
    .await
}

/// Execute an entity member function. Entity fields are accessible via
/// construct_value_id on the entity scope. Returns the func's out fields as a compound.
#[tracing::instrument(level = "debug", skip_all, fields(entity = %entity_name, func = %func_name))]
pub(crate) async fn run_entity_member_func(
    ctx: &mut EvalContext,
    module_idx: ModuleIndex,
    entity_name: &str,
    func_name: &str,
    input: FuncInput,
    member_type_args: BTreeMap<Symbol, crate::load::registry::TypeId>,
    entity_value_id: ValueId,
) -> EvalResult {
    let module = ctx.registry.module(module_idx).clone();

    let construct = match module.get_entity_member_ast(entity_name, func_name) {
        Some(f) => f,
        None => {
            return internal_error_result(
                ctx,
                format!("func '{}' not found on entity '{}'", func_name, entity_name),
            );
        }
    };
    let ConstructKind::Func(func) = &construct.kind else {
        return internal_error_result(
            ctx,
            format!("'{}' on entity '{}' is not a func", func_name, entity_name),
        );
    };
    let Some(member_decl) = module
        .get_declaration(entity_name)
        .and_then(|decl| decl.get_member(func_name))
    else {
        return internal_error_result(
            ctx,
            format!(
                "member declaration metadata not found for '{}.{}'",
                entity_name, func_name
            ),
        );
    };
    let out_early = member_decl.out_early_name.clone();

    let Some(decl_id) = ctx.registry.decl_id_in_module(module_idx, entity_name) else {
        return internal_error_result(
            ctx,
            format!("declaration not registered for entity '{entity_name}'"),
        );
    };
    let Some((def_file_idx, _)) = module.get_declaration_indexed(entity_name) else {
        return internal_error_result(
            ctx,
            format!("declaration not found for entity '{entity_name}'"),
        );
    };
    let def_scope = ctx.definition_scope(module_idx, def_file_idx);
    ctx.push_exec_frame(
        Scope {
            kind: crate::scope::ScopeKind::Construct,
            parent: Some(def_scope),
            name: Some(entity_name.into()),
            construct: Some(ConstructScope {
                decl: decl_id,
                heap_id: Some(entity_value_id),
                target: Value::Nil,
            }),
            ..Default::default()
        }
        .into_ref(),
    );

    let (file_idx, member_node_id) = module
        .get_declaration_indexed(entity_name)
        .and_then(|(fi, decl)| {
            decl.members
                .iter()
                .find(|m| m.name.as_deref() == Some(func_name))
                .map(|m| (fi, m.node_id))
        })
        .unwrap_or((0, NodeId(0)));
    ctx.push_function_frame(
        Scope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(ctx.current_scope_ref()),
            name: Some(func_name.into()),
            out_early: out_early.clone(),
            type_params: member_type_args,
            ..Default::default()
        }
        .into_ref(),
        StackFrame {
            module: module_idx,
            file_idx,
            call_node_id: None,
            decl_node_id: member_node_id,
        },
        input.implicits,
    );

    ctx.push_import_context(module_idx);

    for field in &func.fields {
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            match fv.modifier {
                FieldVarModifier::In | FieldVarModifier::Inout => {
                    if let Some(val) = input.args.get(name) {
                        if let Err(err) = ctx.declare_local(name.clone(), val.clone()) {
                            return err;
                        }
                    } else if let Some(default_expr) = &fv.default {
                        let val = match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_import_context();
                                ctx.pop_scope();
                                ctx.pop_scope();
                                return other;
                            }
                        };
                        if let Err(err) = ctx.declare_local(name.clone(), val) {
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
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                FieldVarModifier::Implicit => {
                    match call::bind_implicit_var(ctx, &module, entity_name, Some(func_name), fv)
                        .await
                    {
                        EvalResult::Ok(_) => {}
                        err => {
                            ctx.pop_import_context();
                            ctx.pop_scope();
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
            let mut fields: BTreeMap<Symbol, Value> = BTreeMap::new();
            let member_decl = module
                .get_declaration(entity_name)
                .and_then(|decl| decl.get_member(func_name));
            if let Some(md) = member_decl {
                for name in &md.out_field_names {
                    if let EvalResult::Ok(val) = ctx.lookup(name) {
                        fields.insert(name.clone(), val);
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
    ctx.pop_scope();
    return_value
}

/// Dispatch a member function call on a data value.
/// Looks up the data declaration via DeclId and dispatches
/// to the member - either a Native handler or a Func body.
pub(crate) async fn eval_data_member_call(
    ctx: &mut EvalContext,
    target: Value,
    decl_id: DeclId,
    member_name: &str,
    args: BTreeMap<Symbol, Value>,
    member_type_args: BTreeMap<Symbol, crate::load::registry::TypeId>,
) -> EvalResult {
    let entry = ctx.registry.decl(decl_id);
    let module_idx = entry.module;
    let data_name = entry.name.clone();
    let module = ctx.registry.module(module_idx).clone();

    let Some(decl) = module.get_declaration(&data_name) else {
        return internal_error_result(
            ctx,
            format!(
                "'{}' not found in module {}",
                data_name,
                ctx.registry.module_path(module_idx)
            ),
        );
    };

    let member_decl = match decl.get_member(member_name) {
        Some(m) => m,
        None => {
            return internal_error_result(
                ctx,
                format!("no member '{}' on data '{}'", member_name, data_name),
            );
        }
    };

    let input = FuncInput {
        args,
        ..Default::default()
    };

    match member_decl.kind {
        TypeConstructKind::Native => {
            eval_data_native_member(
                ctx,
                target,
                module_idx,
                &module,
                &data_name,
                member_name,
                member_decl,
                input,
            )
            .await
        }
        TypeConstructKind::Func => {
            eval_data_func_member(
                ctx,
                target,
                module_idx,
                &module,
                &data_name,
                member_name,
                input,
                member_type_args,
            )
            .await
        }
        _ => internal_error_result(
            ctx,
            format!(
                "member '{}' on '{}' has unexpected kind {:?}",
                member_name, data_name, member_decl.kind
            ),
        ),
    }
}

#[allow(clippy::too_many_arguments)]
async fn eval_data_native_member(
    ctx: &mut EvalContext,
    target: Value,
    module_idx: ModuleIndex,
    module: &Module,
    data_name: &str,
    member_name: &str,
    member_decl: &crate::load::TypeConstruct,
    input: FuncInput,
) -> EvalResult {
    let member_fields: &[Field] =
        find_data_member_fields(module, data_name, member_name).unwrap_or(&[]);

    let full_name = ctx
        .registry
        .qualify_decl(module_idx, &format!("{data_name}.{member_name}"));

    let (file_idx, decl_node_id) = module
        .get_declaration_indexed(data_name)
        .and_then(|(fi, decl)| {
            decl.members
                .iter()
                .find(|m| m.name.as_deref() == Some(member_name))
                .map(|m| (fi, m.node_id))
        })
        .unwrap_or((0, NodeId(0)));
    ctx.push_function_frame(
        Scope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(ctx.current_scope_ref()),
            name: Some(member_name.into()),
            ..Default::default()
        }
        .into_ref(),
        StackFrame {
            module: module_idx,
            file_idx,
            call_node_id: None,
            decl_node_id,
        },
        input.implicits,
    );

    let mut resolved_args = BTreeMap::new();
    for field in member_fields {
        let FieldKind::Var(var) = &field.kind else {
            continue;
        };
        if !matches!(var.modifier, FieldVarModifier::In | FieldVarModifier::Inout) {
            continue;
        }
        let name = &var.name.name;
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
            return internal_error_result(
                ctx,
                format!(
                    "native '{}.{}' missing required argument '{}'",
                    data_name, member_name, name
                ),
            );
        }
    }

    let Some(handler) = ctx.native_registry().get(&full_name) else {
        ctx.pop_scope();
        return fault_result(
            ctx,
            format!("native handler '{}' is not registered", full_name),
        );
    };

    let mut native_out: BTreeMap<Symbol, Value> = BTreeMap::new();
    let call_result = {
        let mut native_ctx = NativeCallContext::with_construct_value(ctx, target);
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

    let out_fields: Vec<_> = member_decl
        .fields
        .iter()
        .filter(|f| {
            f.modifier == Some(crate::load::FieldModifier::Out)
                || f.modifier == Some(crate::load::FieldModifier::OutEarly)
        })
        .collect();
    for field in &out_fields {
        let field_name = &field.name;
        if !native_out.contains_key(field_name) {
            if ctx.registry.is_nilable(field.ty) {
                native_out.insert(field_name.clone(), Value::Nil);
            } else {
                ctx.pop_scope();
                return fault_result(
                    ctx,
                    format!(
                        "native '{}.{}' result missing required field '{}'",
                        data_name, member_name, field_name
                    ),
                );
            }
        }
    }

    ctx.pop_scope();

    let out_early = member_decl
        .fields
        .iter()
        .find(|f| f.modifier == Some(FieldModifier::OutEarly))
        .map(|f| f.name.clone());
    let id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name: out_early,
        fields: native_out,
        type_args: BTreeMap::new(),
    });
    EvalResult::Ok(Value::Data(id))
}

/// Execute a Func member on a data value. Allocates a temporary Data heap
/// value so the func body can access the target via construct_value_id,
/// and sibling members resolve by bare name in the data scope.
#[allow(clippy::too_many_arguments)]
async fn eval_data_func_member(
    ctx: &mut EvalContext,
    target: Value,
    module_idx: ModuleIndex,
    module: &Module,
    data_name: &str,
    func_name: &str,
    input: FuncInput,
    member_type_args: BTreeMap<Symbol, crate::load::registry::TypeId>,
) -> EvalResult {
    let Some(construct) = module.get_data_member_ast(data_name, func_name) else {
        return internal_error_result(
            ctx,
            format!(
                "func '{}' not found on data '{}' in module {}",
                func_name,
                data_name,
                ctx.registry.module_path(module_idx)
            ),
        );
    };
    let ConstructKind::Func(func) = &construct.kind else {
        return internal_error_result(
            ctx,
            format!("'{}' on data '{}' is not a func", func_name, data_name),
        );
    };

    let Some(member_decl) = module
        .get_declaration(data_name)
        .and_then(|decl| decl.get_member(func_name))
    else {
        return internal_error_result(
            ctx,
            format!(
                "member declaration metadata not found for '{}.{}'",
                data_name, func_name
            ),
        );
    };
    let out_early = member_decl.out_early_name.clone();

    let heap_id = target.as_heap_id();
    let Some(decl_id) = ctx.registry.decl_id_in_module(module_idx, data_name) else {
        return internal_error_result(
            ctx,
            format!("declaration not registered for data '{data_name}'"),
        );
    };
    let Some((def_file_idx, _)) = module.get_declaration_indexed(data_name) else {
        return internal_error_result(ctx, format!("declaration not found for data '{data_name}'"));
    };
    let construct_type_params = heap_id
        .map(|vid| {
            let heap = ctx.heap();
            if let HeapValue::Data { type_args, .. } = heap.get(vid) {
                type_args.clone()
            } else {
                BTreeMap::new()
            }
        })
        .unwrap_or_default();
    let def_scope = ctx.definition_scope(module_idx, def_file_idx);
    ctx.push_exec_frame(
        Scope {
            kind: crate::scope::ScopeKind::Construct,
            parent: Some(def_scope),
            name: Some(data_name.into()),
            type_params: construct_type_params,
            construct: Some(ConstructScope {
                decl: decl_id,
                heap_id,
                target: target.clone(),
            }),
            ..Default::default()
        }
        .into_ref(),
    );

    let (file_idx, member_node_id) = module
        .get_declaration_indexed(data_name)
        .and_then(|(fi, decl)| {
            decl.members
                .iter()
                .find(|m| m.name.as_deref() == Some(func_name))
                .map(|m| (fi, m.node_id))
        })
        .unwrap_or((0, NodeId(0)));
    ctx.push_function_frame(
        Scope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(ctx.current_scope_ref()),
            name: Some(func_name.into()),
            out_early: out_early.clone(),
            type_params: member_type_args,
            ..Default::default()
        }
        .into_ref(),
        StackFrame {
            module: module_idx,
            file_idx,
            call_node_id: None,
            decl_node_id: member_node_id,
        },
        input.implicits,
    );

    ctx.push_import_context(module_idx);

    for field in &func.fields {
        if let FieldKind::Var(fv) = &field.kind {
            let name = &fv.name.name;
            match fv.modifier {
                FieldVarModifier::In | FieldVarModifier::Inout => {
                    if let Some(val) = input.args.get(name) {
                        if let Err(err) = ctx.declare_local(name.clone(), val.clone()) {
                            return err;
                        }
                    } else if let Some(default_expr) = &fv.default {
                        let val = match eval::eval_expr(ctx, default_expr).await {
                            EvalResult::Ok(v) => v,
                            other => {
                                ctx.pop_import_context();
                                ctx.pop_scope();
                                ctx.pop_scope();
                                return other;
                            }
                        };
                        if let Err(err) = ctx.declare_local(name.clone(), val) {
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
                                ctx.pop_scope();
                                return other;
                            }
                        }
                    } else {
                        Value::Nil
                    };
                    if let Err(err) = ctx.declare_local(name.clone(), val) {
                        return err;
                    }
                }
                _ => {}
            }
        }
    }

    let result = eval::exec_stmts(ctx, &func.stmts).await;
    ctx.pop_import_context();

    let return_value = match result {
        EvalResult::Ok(_) | EvalResult::Return => {
            let mut fields: BTreeMap<Symbol, Value> = BTreeMap::new();
            let member_decl = module
                .get_declaration(data_name)
                .and_then(|decl| decl.get_member(func_name));
            if let Some(md) = member_decl {
                for name in &md.out_field_names {
                    if let EvalResult::Ok(val) = ctx.lookup(name) {
                        fields.insert(name.clone(), val);
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
    ctx.pop_scope();
    return_value
}
