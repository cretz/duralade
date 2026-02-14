use std::cell::Cell;
use std::collections::BTreeMap;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use crate::engine::ModuleIndex;
use crate::event::{OwnedFields, OwnedValue};
use crate::model::*;

use super::call;
use super::context::{self, EvalContext, WaitEntry};
use super::member;
use super::scope::{FaultError, InternalError};
use super::value::{EntityId, HeapValue, HeapValueFuncKind, Value};
use crate::scope::Scope;

#[derive(Debug)]
pub enum EvalResult {
    Ok(Value),
    Return,
    Break(Option<Symbol>),
    Continue(Option<Symbol>),
    Fault(FaultError),
    InternalError(InternalError),
}

macro_rules! internal_error {
    ($ctx:expr, $($arg:tt)*) => {
        make_internal_error(format!($($arg)*), $ctx.snapshot_stack())
    }
}

/// Create an error data value: `data { out! error = data { message = msg } }`.
/// Used by `?!` and `as!` default narrowing forms.
fn alloc_narrowing_error(ctx: &mut EvalContext, message: &str) -> Value {
    let mut inner_fields = BTreeMap::new();
    inner_fields.insert(Symbol::message(), Value::Str(Rc::from(message)));
    let inner_id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name: None,
        fields: inner_fields,
        type_args: BTreeMap::new(),
    });
    Value::Data(inner_id)
}

pub fn make_fault(message: String, stack: Vec<super::scope::StackFrame>) -> EvalResult {
    EvalResult::Fault(FaultError { message, stack })
}

#[track_caller]
pub fn make_internal_error(message: String, stack: Vec<super::scope::StackFrame>) -> EvalResult {
    let loc = std::panic::Location::caller();
    EvalResult::InternalError(InternalError {
        message,
        rust_location: format!("{}:{}", loc.file(), loc.line()),
        backtrace: Box::new(std::backtrace::Backtrace::capture()),
        stack,
    })
}

pub(crate) fn eval_expr<'a>(
    ctx: &'a mut EvalContext,
    expr: &'a Expr,
) -> Pin<Box<dyn Future<Output = EvalResult> + 'a>> {
    Box::pin(async move {
        match expr {
            Expr::Paren(p) => eval_expr(ctx, &p.expr).await,
            Expr::Access(acc) => eval_access(ctx, acc).await,
            Expr::ModuleAccess(_) => {
                internal_error!(ctx, "module access as value not yet supported")
            }
            Expr::Ident(ident) => ctx.lookup(&ident.name),
            Expr::Unary(un) => eval_unary(ctx, un).await,
            Expr::Binary(bin) => eval_binary(ctx, bin).await,
            Expr::Invocation(inv) => eval_invocation(ctx, inv).await,
            Expr::Narrowing(n) => eval_narrowing(ctx, n).await,
            Expr::Literal(lit) => eval_literal(ctx, lit).await,
            Expr::Wait(wait) => eval_wait(ctx, wait).await,
            Expr::Spawn(spawn) => eval_spawn(ctx, spawn).await,
            Expr::AnonData(data) => eval_anon_data(ctx, data).await,
            Expr::AnonFunc(func) => eval_anon_func(ctx, func).await,
            _ => internal_error!(ctx, "unsupported expression type"),
        }
    })
}

async fn eval_anon_data(ctx: &mut EvalContext, data: &Data) -> EvalResult {
    let mut fields = BTreeMap::new();
    let mut out_early_name: Option<Symbol> = None;

    for field in &data.fields {
        let FieldKind::Var(var) = &field.kind else {
            return internal_error!(ctx, "anonymous data contains non-variable field");
        };

        if var.modifier == FieldVarModifier::OutEarly {
            out_early_name = Some(var.name.name.clone());
        }

        let Some(default_expr) = &var.default else {
            return internal_error!(
                ctx,
                "anonymous data field '{}' is missing a default expression",
                var.name.name
            );
        };

        let value = match eval_expr(ctx, default_expr).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        fields.insert(var.name.name.clone(), value);
    }

    let id = ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields,
        type_args: BTreeMap::new(),
    });
    EvalResult::Ok(Value::Data(id))
}

async fn eval_anon_func(ctx: &mut EvalContext, func: &Func) -> EvalResult {
    let module = ctx
        .current_module_index()
        .unwrap_or(crate::engine::ModuleIndex(0));

    // TODO: Type checker must resolve lexical local bindings for closure bodies.
    let id = ctx.heap_mut().alloc(HeapValue::Func {
        module,
        kind: HeapValueFuncKind::Closure {
            captures: BTreeMap::new(),
            captured_scope: ctx.current_scope_ref(),
            body: func.clone(),
        },
    });
    EvalResult::Ok(Value::Func(id))
}

async fn eval_literal(ctx: &mut EvalContext, lit: &ExprLiteral) -> EvalResult {
    match &lit.kind {
        ExprLiteralKind::Int(s) => match ExprLiteralKind::parse_int(s) {
            Ok(n) => EvalResult::Ok(Value::Int(n)),
            Err(msg) => internal_error!(ctx, "{}", msg),
        },
        ExprLiteralKind::Float(s) => match ExprLiteralKind::parse_float(s) {
            Ok(n) => EvalResult::Ok(Value::Float(n)),
            Err(msg) => internal_error!(ctx, "{}", msg),
        },
        ExprLiteralKind::Str(s) => EvalResult::Ok(Value::Str(s.as_str().into())),
        ExprLiteralKind::Bool(b) => EvalResult::Ok(Value::Bool(*b)),
        ExprLiteralKind::Nil => EvalResult::Ok(Value::Nil),
        ExprLiteralKind::Array(items) => {
            let mut vals = Vec::with_capacity(items.len());
            for item in items {
                match eval_expr(ctx, item).await {
                    EvalResult::Ok(v) => vals.push(v),
                    other => return other,
                }
            }
            let element_type = ctx
                .lookup_as_narrowing_type(lit.node_id)
                .and_then(|tid| match ctx.registry.type_entry(tid) {
                    crate::load::registry::TypeEntry::Named { type_args, .. } => type_args
                        .iter()
                        .find(|a| a.name == Symbol::t())
                        .map(|a| a.value),
                    _ => None,
                })
                .unwrap_or(ctx.registry.any_type);
            let value = super::native_collection::NativeArray::alloc(
                vals,
                Some(ctx.registry.array_decl),
                element_type,
                &mut ctx.heap_mut(),
            );
            EvalResult::Ok(value)
        }
        ExprLiteralKind::Map(entries) => {
            let (key_type, value_type) = ctx
                .lookup_as_narrowing_type(lit.node_id)
                .and_then(|tid| match ctx.registry.type_entry(tid) {
                    crate::load::registry::TypeEntry::Named { type_args, .. } => {
                        let k = type_args
                            .iter()
                            .find(|a| a.name == Symbol::k())
                            .map(|a| a.value);
                        let v = type_args
                            .iter()
                            .find(|a| a.name == Symbol::v())
                            .map(|a| a.value);
                        k.zip(v)
                    }
                    _ => None,
                })
                .unwrap_or((ctx.registry.any_type, ctx.registry.any_type));
            let map_value = super::native_collection::NativeMap::alloc(
                Some(ctx.registry.map_decl),
                key_type,
                value_type,
                &mut ctx.heap_mut(),
            );
            for entry in entries {
                let key = match eval_expr(ctx, &entry.key).await {
                    EvalResult::Ok(v) => v,
                    other => return other,
                };
                let val = match eval_expr(ctx, &entry.value).await {
                    EvalResult::Ok(v) => v,
                    other => return other,
                };
                // Hash the key and insert into the NativeMap.
                let heap = ctx.heap();
                let map_key = match super::native_collection::MapKey::new(key, &heap) {
                    Ok(k) => k,
                    Err(msg) => {
                        return make_fault(format!("map literal: {msg}"), ctx.snapshot_stack());
                    }
                };
                let Value::Data(data_id) = &map_value else {
                    return internal_error!(ctx, "map literal: alloc did not return Data");
                };
                let HeapValue::Data { fields, .. } = heap.get(data_id.id()) else {
                    return internal_error!(ctx, "map literal: not a Data");
                };
                let Some(Value::Native(native_id)) = fields.get("_items") else {
                    return internal_error!(ctx, "map literal: missing _items");
                };
                let HeapValue::Native(native) = heap.get(native_id.id()) else {
                    return internal_error!(ctx, "map literal: _items not Native");
                };
                let Some(nm) = (native.as_ref() as &dyn std::any::Any)
                    .downcast_ref::<super::native_collection::NativeMap>()
                else {
                    return internal_error!(ctx, "map literal: _items not NativeMap");
                };
                nm.0.borrow_mut().insert(map_key, val);
            }
            EvalResult::Ok(map_value)
        }
    }
}

async fn eval_narrowing(ctx: &mut EvalContext, narrowing: &ExprNarrowing) -> EvalResult {
    let value = match eval_expr(ctx, &narrowing.expr).await {
        EvalResult::Ok(v) => v,
        other => return other,
    };

    match &narrowing.kind {
        ExprNarrowingKind::Nil(nil_narrow) => match value {
            Value::Nil => {
                let v = if let Some(else_expr) = &nil_narrow.else_expr {
                    match eval_expr(ctx, else_expr).await {
                        EvalResult::Ok(v) => v,
                        other => return other,
                    }
                } else {
                    alloc_narrowing_error(ctx, "unexpected nil")
                };
                ctx.set_enclosing_out_early(v);
                EvalResult::Return
            }
            v => EvalResult::Ok(v),
        },
        ExprNarrowingKind::As(as_narrow) => {
            let Some(mut target_type) = ctx.lookup_as_narrowing_type(as_narrow.node_id) else {
                return internal_error!(ctx, "as narrowing has no resolved target type");
            };
            // Resolve type params to concrete types from scope
            if let crate::load::registry::TypeEntry::TypeParam(name) =
                ctx.registry.type_entry(target_type)
            {
                let name = name.clone();
                if let Some(concrete) = ctx.resolve_type_param(&name) {
                    target_type = concrete;
                }
            }
            let is_match = {
                let heap = ctx.heap();
                super::assignable::runtime_is_assignable(&value, target_type, &heap, &ctx.registry)
            };
            if is_match {
                EvalResult::Ok(value)
            } else {
                let v = if let Some(else_expr) = &as_narrow.else_expr {
                    match eval_expr(ctx, else_expr).await {
                        EvalResult::Ok(v) => v,
                        other => return other,
                    }
                } else {
                    alloc_narrowing_error(ctx, "type narrowing: value does not match expected type")
                };
                ctx.set_enclosing_out_early(v);
                EvalResult::Return
            }
        }
    }
}

async fn eval_binary(ctx: &mut EvalContext, bin: &ExprBinary) -> EvalResult {
    let left = match eval_expr(ctx, &bin.left).await {
        EvalResult::Ok(v) => v,
        other => return other,
    };
    let right = match eval_expr(ctx, &bin.right).await {
        EvalResult::Ok(v) => v,
        other => return other,
    };

    match bin.op {
        ExprBinaryOp::Add => numeric_op(ctx, &left, &right, "add", |a, b| a + b, |a, b| a + b),
        ExprBinaryOp::Sub => numeric_op(ctx, &left, &right, "subtract", |a, b| a - b, |a, b| a - b),
        ExprBinaryOp::Mul => numeric_op(ctx, &left, &right, "multiply", |a, b| a * b, |a, b| a * b),
        ExprBinaryOp::Div => numeric_op(ctx, &left, &right, "divide", |a, b| a / b, |a, b| a / b),
        ExprBinaryOp::Mod => numeric_op(ctx, &left, &right, "modulo", |a, b| a % b, |a, b| a % b),
        ExprBinaryOp::Eq => EvalResult::Ok(Value::Bool(value_eq(&left, &right))),
        ExprBinaryOp::Ne => EvalResult::Ok(Value::Bool(!value_eq(&left, &right))),
        ExprBinaryOp::Lt => compare_op(ctx, &left, &right, |o| o.is_lt()),
        ExprBinaryOp::Le => compare_op(ctx, &left, &right, |o| o.is_le()),
        ExprBinaryOp::Gt => compare_op(ctx, &left, &right, |o| o.is_gt()),
        ExprBinaryOp::Ge => compare_op(ctx, &left, &right, |o| o.is_ge()),
        ExprBinaryOp::And => match (&left, &right) {
            (Value::Bool(a), Value::Bool(b)) => EvalResult::Ok(Value::Bool(*a && *b)),
            _ => internal_error!(ctx, "'and' requires bool operands"),
        },
        ExprBinaryOp::Or => match (&left, &right) {
            (Value::Bool(a), Value::Bool(b)) => EvalResult::Ok(Value::Bool(*a || *b)),
            _ => internal_error!(ctx, "'or' requires bool operands"),
        },
        ExprBinaryOp::NilCoalesce => match left {
            Value::Nil => EvalResult::Ok(right),
            _ => EvalResult::Ok(left),
        },
    }
}

async fn eval_unary(ctx: &mut EvalContext, un: &ExprUnary) -> EvalResult {
    let val = match eval_expr(ctx, &un.expr).await {
        EvalResult::Ok(v) => v,
        other => return other,
    };
    match un.op {
        ExprUnaryOp::Negate => match val {
            Value::Int(n) => EvalResult::Ok(Value::Int(-n)),
            Value::Float(n) => EvalResult::Ok(Value::Float(-n)),
            _ => internal_error!(ctx, "negate requires numeric operand"),
        },
        ExprUnaryOp::Not => match val {
            Value::Bool(b) => EvalResult::Ok(Value::Bool(!b)),
            _ => internal_error!(ctx, "'not' requires bool operand"),
        },
        ExprUnaryOp::EarlyReturn => {
            let Value::Data(id) = val else {
                return internal_error!(ctx, "! on non-compound value");
            };
            // Extract the out! field name and value from the expression's data.
            let (field_name, early_value) = {
                let heap = ctx.heap();
                let HeapValue::Data {
                    out_early_name,
                    fields,
                    ..
                } = heap.get(id.id())
                else {
                    return internal_error!(ctx, "! on non-Data heap value");
                };
                let Some(field_name) = out_early_name else {
                    return internal_error!(ctx, "! on data which has no out! field");
                };
                let Some(field_value) = fields.get(field_name) else {
                    return internal_error!(ctx, "! compound missing field '{field_name}'");
                };
                (field_name.clone(), field_value.clone())
            };
            tracing::trace!(
                field_name = field_name.as_str(),
                "! operator checking field"
            );
            if let Value::Nil = early_value {
                tracing::trace!("! field is nil, continuing");
                EvalResult::Ok(Value::Data(id))
            } else {
                tracing::trace!("! field is non-nil, early return");
                ctx.set_enclosing_out_early(early_value);
                EvalResult::Return
            }
        }
    }
}

async fn eval_access(ctx: &mut EvalContext, acc: &ExprAccess) -> EvalResult {
    let val = match eval_expr(ctx, &acc.expr).await {
        EvalResult::Ok(v) => v,
        other => return other,
    };
    match acc.op {
        ExprAccessOp::Dot => match val {
            Value::Data(ref id) => {
                // Try stored field first.
                let heap = ctx.heap();
                let HeapValue::Data { fields, decl, .. } = heap.get(id.id()) else {
                    return internal_error!(ctx, "cannot access field on non-Data heap value");
                };
                if let Some(v) = fields.get(&acc.ident.name) {
                    return EvalResult::Ok(v.clone());
                }
                let Some(decl_id) = *decl else {
                    return internal_error!(ctx, "field '{}' not found on data", acc.ident.name);
                };
                drop(heap);

                // Fall back to member func → bound func reference.
                let entry = ctx.registry.decl(decl_id);
                let module_idx = entry.module;
                let data_name = entry.name.clone();
                let module = ctx.registry.module(module_idx).clone();
                let has_member = module
                    .get_declaration(&data_name)
                    .and_then(|d| d.get_member(&acc.ident.name))
                    .is_some();
                if !has_member {
                    return internal_error!(ctx, "field '{}' not found on data", acc.ident.name);
                }
                let func_id = ctx.heap_mut().alloc(HeapValue::Func {
                    module: module_idx,
                    kind: HeapValueFuncKind::Decl {
                        decl: decl_id,
                        name: acc.ident.name.clone(),
                        target: Some(val),
                    },
                });
                EvalResult::Ok(Value::Func(func_id))
            }
            Value::Entity(id) => match ctx.heap().get(id.id()) {
                HeapValue::Entity { fields, .. } => match fields.get(&acc.ident.name) {
                    Some(v) => EvalResult::Ok(v.clone()),
                    None => internal_error!(ctx, "field '{}' not found on entity", acc.ident.name),
                },
                _ => internal_error!(ctx, "expected Entity heap value"),
            },
            Value::EntityRef(eid) => match acc.ident.name.as_str() {
                "id" => EvalResult::Ok(Value::Str(eid.0.as_str().into())),
                _ => internal_error!(ctx, "unknown entity ref field '{}'", acc.ident.name),
            },
            _ => internal_error!(
                ctx,
                "cannot access field '{}' on non-data value",
                acc.ident.name
            ),
        },
        _ => internal_error!(ctx, "unsupported access operation"),
    }
}

#[tracing::instrument(level = "trace", skip_all)]
async fn eval_invocation(ctx: &mut EvalContext, inv: &ExprInvocation) -> EvalResult {
    let segments = resolve_access_segments(&inv.expr);

    let mut args = BTreeMap::new();
    for arg in &inv.args {
        let val = match eval_expr(ctx, &arg.value).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        args.insert(arg.name.name.clone(), val);
    }

    // Resolve type args from the type checker's result for this invocation.
    // For constructors: extract from the Named result type (handles inference too).
    // For member func calls: resolve AST type args via node_types.
    let type_args: BTreeMap<Symbol, crate::load::registry::TypeId> = if let Some(type_id) =
        ctx.lookup_as_narrowing_type(inv.node_id)
        && let crate::load::registry::TypeEntry::Named { type_args, .. } =
            ctx.registry.type_entry(type_id)
        && !type_args.is_empty()
    {
        type_args
            .iter()
            .map(|a| {
                // Resolve TypeParam values to concrete types from scope
                let ty = if let crate::load::registry::TypeEntry::TypeParam(name) =
                    ctx.registry.type_entry(a.value)
                {
                    let name = name.clone();
                    ctx.resolve_type_param(&name).unwrap_or(a.value)
                } else {
                    a.value
                };
                (a.name.clone(), ty)
            })
            .collect()
    } else if !inv.type_args.is_empty() {
        // Member func type args: resolve each AST type arg value via node_types
        inv.type_args
            .iter()
            .filter_map(|ta| {
                ctx.lookup_as_narrowing_type(ta.value.node_id())
                    .map(|tid| (ta.name.name.clone(), tid))
            })
            .collect()
    } else {
        BTreeMap::new()
    };

    // Arrow dispatch: ref->func(args) emits duralade.entity::call system extern.
    if let Expr::Access(acc) = inv.expr.as_ref()
        && acc.op == ExprAccessOp::EntityRef
    {
        let target = match eval_expr(ctx, &acc.expr).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        return match target {
            Value::EntityRef(eid) => eval_entity_ref_call(ctx, eid, &acc.ident.name, args).await,
            _ => internal_error!(ctx, "-> requires an entity ref"),
        };
    }

    if let Some(segs) = &segments {
        if segs.len() == 2 {
            // Check if the first segment is a local entity ref variable.
            if let EvalResult::Ok(Value::EntityRef(eid)) = ctx.lookup(&segs[0]) {
                return eval_entity_ref_method(ctx, eid, &segs[1], args).await;
            }
            // Check if the first segment is a local entity value.
            if let EvalResult::Ok(Value::Entity(vid)) = ctx.lookup(&segs[0]) {
                return member::eval_local_entity_method(ctx, vid.id(), &segs[1], args).await;
            }
            if let Some((idx, func_name)) = resolve_qualified_decl_target(ctx, &segs[0], &segs[1]) {
                let input = super::scope::FuncInput {
                    args,
                    type_args,
                    ..Default::default()
                };
                return call::call_or_construct(ctx, idx, &func_name, input, Some(inv.node_id))
                    .await;
            }
        }
        if segs.len() == 1 {
            let name = &segs[0];
            if let Some(Value::Func(func_id)) = ctx.resolve_name(name) {
                // If the func is a bound member and there are type_args,
                // call through the member path which handles type arg passing.
                if !type_args.is_empty() {
                    let member_info = {
                        let heap = ctx.heap();
                        if let HeapValue::Func {
                            kind:
                                HeapValueFuncKind::Decl {
                                    decl,
                                    name: member_name,
                                    target: Some(target),
                                },
                            ..
                        } = heap.get(func_id.id())
                        {
                            Some((*decl, member_name.clone(), target.clone()))
                        } else {
                            None
                        }
                    };
                    if let Some((decl, member_name, target)) = member_info {
                        return member::eval_data_member_call(
                            ctx,
                            target,
                            decl,
                            &member_name,
                            args,
                            type_args,
                        )
                        .await;
                    }
                }
                return call::call_func_value(ctx, func_id.id(), args, Some(inv.node_id)).await;
            }
            // Try import alias (e.g. `import foo.bar` makes `bar()` resolve
            // to `foo.bar::bar`), but only if the module has a same-named
            // declaration. Otherwise fall back to current module.
            if let Some((idx, name)) = resolve_simple_decl_target(ctx, name) {
                let input = super::scope::FuncInput {
                    args,
                    type_args,
                    ..Default::default()
                };
                return call::call_or_construct(ctx, idx, &name, input, Some(inv.node_id)).await;
            }
        }
    }

    // Dot-access invocation on a complex expression: eval the LHS and
    // dispatch based on the resulting value type (entity ref, entity, func).
    if let Expr::Access(acc) = inv.expr.as_ref()
        && acc.op == ExprAccessOp::Dot
    {
        let target = match eval_expr(ctx, &acc.expr).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        return match &target {
            Value::EntityRef(eid) => {
                eval_entity_ref_method(ctx, eid.clone(), &acc.ident.name, args).await
            }
            Value::Entity(vid) => {
                member::eval_local_entity_method(ctx, vid.id(), &acc.ident.name, args).await
            }
            _ => {
                if let Some(decl_id) = ctx.value_decl_id(&target) {
                    member::eval_data_member_call(
                        ctx,
                        target,
                        decl_id,
                        &acc.ident.name,
                        args,
                        type_args,
                    )
                    .await
                } else {
                    internal_error!(ctx, "cannot call '.{}()' on {:?}", acc.ident.name, target)
                }
            }
        };
    }

    // Callable value dispatch.
    if let EvalResult::Ok(Value::Func(func_id)) = eval_expr(ctx, &inv.expr).await {
        return call::call_func_value(ctx, func_id.id(), args, Some(inv.node_id)).await;
    }

    internal_error!(
        ctx,
        "unknown function: {}",
        segments
            .map(|s| s.iter().map(|s| s.as_str()).collect::<Vec<_>>().join("."))
            .unwrap_or_else(|| "<complex expr>".to_string())
    )
}

async fn eval_entity_ref_method(
    ctx: &mut EvalContext,
    entity_id: EntityId,
    method: &str,
    _args: BTreeMap<Symbol, Value>,
) -> EvalResult {
    match method {
        "result" => {
            let mut extern_args = OwnedFields::new();
            extern_args.insert("entity_id".into(), OwnedValue::EntityRef(entity_id));
            call::call_system_extern(ctx, "duralade.entity::result", extern_args).await
        }
        _ => internal_error!(ctx, "unknown entity ref method '{method}'"),
    }
}

async fn eval_entity_ref_call(
    ctx: &mut EvalContext,
    entity_id: EntityId,
    func_name: &str,
    args: BTreeMap<Symbol, Value>,
) -> EvalResult {
    let mut extern_args = OwnedFields::new();
    extern_args.insert("entity_id".into(), OwnedValue::EntityRef(entity_id));
    extern_args.insert("func".into(), OwnedValue::Str(func_name.to_string()));
    let mut owned_call_args = OwnedFields::new();
    for (k, v) in &args {
        match OwnedValue::from_value(v, &ctx.heap()) {
            Ok(ov) => {
                owned_call_args.insert(k.to_string(), ov);
            }
            Err(e) => return internal_error!(ctx, "entity call arg '{}': {}", k, e),
        }
    }
    extern_args.insert("args".into(), OwnedValue::Data(owned_call_args));
    call::call_system_extern(ctx, "duralade.entity::call", extern_args).await
}

#[tracing::instrument(level = "debug", skip_all)]
async fn eval_spawn(ctx: &mut EvalContext, spawn: &ExprSpawn) -> EvalResult {
    // The inner expression must be an Invocation
    let inv = match spawn.invocation.as_ref() {
        Expr::Invocation(inv) => inv,
        _ => {
            return internal_error!(ctx, "spawn requires an invocation expression");
        }
    };

    // Resolve entity type to fully-qualified name
    let segments = resolve_access_segments(&inv.expr);
    let (module_idx, entity_name) = match &segments {
        Some(segs) if segs.len() == 2 => {
            match resolve_qualified_decl_target(ctx, &segs[0], &segs[1]) {
                Some((idx, entity_name)) => (idx, entity_name),
                None => {
                    return internal_error!(ctx, "cannot resolve module '{}' for spawn", segs[0]);
                }
            }
        }
        Some(segs) if segs.len() == 1 => match resolve_simple_decl_target(ctx, &segs[0]) {
            Some((idx, entity_name)) => (idx, entity_name),
            None => {
                return internal_error!(ctx, "cannot determine current module for spawn");
            }
        },
        _ => {
            return internal_error!(ctx, "spawn target must be a simple entity reference");
        }
    };

    // Evaluate constructor args
    let mut constructor_args = OwnedFields::new();
    for arg in &inv.args {
        let val = match eval_expr(ctx, &arg.value).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        match OwnedValue::from_value(&val, &ctx.heap()) {
            Ok(ov) => {
                constructor_args.insert(arg.name.name.to_string(), ov);
            }
            Err(e) => return internal_error!(ctx, "spawn arg '{}': {}", arg.name.name, e),
        }
    }

    // Evaluate spawn id
    let spawn_id = if let Some(id_expr) = &spawn.id {
        match eval_expr(ctx, id_expr).await {
            EvalResult::Ok(Value::Str(s)) => Some(s),
            EvalResult::Ok(_) => {
                return internal_error!(ctx, "spawn 'id' must be a string");
            }
            other => return other,
        }
    } else {
        None
    };

    // Build fully-qualified entity type string using :: to separate module path from entity name.
    let entity_type = ctx.registry.qualify_decl(module_idx, &entity_name);

    // Package as system extern args
    let mut extern_args = OwnedFields::new();
    extern_args.insert("entity".into(), OwnedValue::Str(entity_type));
    extern_args.insert("args".into(), OwnedValue::Data(constructor_args));
    if let Some(id) = spawn_id {
        extern_args.insert("id".into(), OwnedValue::Str(id.to_string()));
    }

    call::call_system_extern(ctx, "duralade.entity::spawn", extern_args).await
}

async fn eval_wait(ctx: &mut EvalContext, wait: &ExprWait) -> EvalResult {
    let (trigger, flag) = context::wait_trigger();
    let resolved = Rc::new(Cell::new(false));

    ctx.wait_queue().borrow_mut().push(WaitEntry {
        trigger,
        resolved: resolved.clone(),
        coroutine_id: ctx.current_coroutine_id(),
    });

    loop {
        // Block until the scheduler explicitly wakes us to try our condition.
        context::wait_for_trigger(&flag).await;

        match eval_expr(ctx, &wait.condition).await {
            EvalResult::Ok(Value::Bool(true)) => {
                resolved.set(true);
                let mut fields = BTreeMap::new();
                fields.insert(Symbol::error(), Value::Nil);
                let id = ctx.heap_mut().alloc(HeapValue::Data {
                    decl: None,
                    out_early_name: Some(Symbol::error()),
                    fields,
                    type_args: BTreeMap::new(),
                });
                return EvalResult::Ok(Value::Data(id));
            }
            EvalResult::Ok(Value::Bool(false)) => continue,
            EvalResult::Ok(_) => {
                return internal_error!(ctx, "wait condition must be a bool");
            }
            other => return other,
        }
    }
}

pub(crate) fn resolve_access_segments(expr: &Expr) -> Option<Vec<Symbol>> {
    match expr {
        Expr::Ident(ident) => Some(vec![ident.name.clone()]),
        Expr::ModuleAccess(ma) => Some(vec![ma.module.name.clone(), ma.name.name.clone()]),
        Expr::Access(acc) if acc.op == ExprAccessOp::Dot => {
            let mut segs = resolve_access_segments(&acc.expr)?;
            segs.push(acc.ident.name.clone());
            Some(segs)
        }
        _ => None,
    }
}

fn resolve_qualified_decl_target(
    ctx: &EvalContext,
    module_alias: &Symbol,
    decl_name: &Symbol,
) -> Option<(ModuleIndex, Symbol)> {
    let module_idx = ctx.resolve_import(module_alias)?;
    Some((module_idx, decl_name.clone()))
}

fn resolve_simple_decl_target(ctx: &EvalContext, name: &Symbol) -> Option<(ModuleIndex, Symbol)> {
    ctx.resolve_import(name)
        .filter(|&idx| ctx.registry.module(idx).get_declaration(name).is_some())
        .or_else(|| ctx.current_module_index())
        .map(|idx| (idx, name.clone()))
}

fn numeric_op(
    ctx: &EvalContext,
    left: &Value,
    right: &Value,
    op_name: &str,
    int_op: fn(i64, i64) -> i64,
    float_op: fn(f64, f64) -> f64,
) -> EvalResult {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => EvalResult::Ok(Value::Int(int_op(*a, *b))),
        (Value::Float(a), Value::Float(b)) => EvalResult::Ok(Value::Float(float_op(*a, *b))),
        (Value::Int(a), Value::Float(b)) => EvalResult::Ok(Value::Float(float_op(*a as f64, *b))),
        (Value::Float(a), Value::Int(b)) => EvalResult::Ok(Value::Float(float_op(*a, *b as f64))),
        _ => internal_error!(ctx, "cannot {op_name} non-numeric values"),
    }
}

fn value_eq(left: &Value, right: &Value) -> bool {
    match (left, right) {
        (Value::Int(a), Value::Int(b)) => a == b,
        (Value::Float(a), Value::Float(b)) => a == b,
        (Value::Str(a), Value::Str(b)) => a == b,
        (Value::Bool(a), Value::Bool(b)) => a == b,
        (Value::Nil, Value::Nil) => true,
        _ => false,
    }
}

fn compare_op(
    ctx: &EvalContext,
    left: &Value,
    right: &Value,
    check: fn(std::cmp::Ordering) -> bool,
) -> EvalResult {
    let ord = match (left, right) {
        (Value::Int(a), Value::Int(b)) => a.cmp(b),
        (Value::Str(a), Value::Str(b)) => a.cmp(b),
        _ => return internal_error!(ctx, "cannot compare values"),
    };
    EvalResult::Ok(Value::Bool(check(ord)))
}

pub(crate) fn exec_stmts<'a>(
    ctx: &'a mut EvalContext,
    stmts: &'a [Stmt],
) -> Pin<Box<dyn Future<Output = EvalResult> + 'a>> {
    Box::pin(async move {
        let mut last = Value::Nil;
        for stmt in stmts {
            match exec_stmt(ctx, stmt).await {
                EvalResult::Ok(v) => last = v,
                other => return other,
            }
        }
        EvalResult::Ok(last)
    })
}

pub async fn exec_stmt(ctx: &mut EvalContext, stmt: &Stmt) -> EvalResult {
    match stmt {
        Stmt::Expr(expr) => eval_expr(ctx, expr).await,
        Stmt::Var(var) => exec_var(ctx, var).await,
        Stmt::Assign(assign) => exec_assign(ctx, assign).await,
        Stmt::If(if_stmt) => exec_if(ctx, if_stmt).await,
        Stmt::Implicitly(imp) => exec_implicitly(ctx, imp).await,
        Stmt::Return(_) => EvalResult::Return,
        Stmt::ReturnEarly(ret) => {
            let v = match eval_expr(ctx, &ret.expr).await {
                EvalResult::Ok(v) => v,
                other => return other,
            };
            ctx.set_enclosing_out_early(v);
            EvalResult::Return
        }
        Stmt::Block(block) => exec_stmts(ctx, &block.stmts).await,
        Stmt::For(for_stmt) => exec_for(ctx, for_stmt).await,
        Stmt::ForBreak(brk) => EvalResult::Break(brk.label.as_ref().map(|l| l.name.clone())),
        Stmt::ForContinue(cont) => {
            EvalResult::Continue(cont.label.as_ref().map(|l| l.name.clone()))
        }
        Stmt::Defer(defer_stmt) => exec_defer(ctx, defer_stmt),
        _ => internal_error!(ctx, "unsupported statement type"),
    }
}

fn exec_defer(ctx: &mut EvalContext, defer_stmt: &StmtDefer) -> EvalResult {
    let module = ctx.current_module_index().unwrap_or(ModuleIndex(0));

    let func = Func {
        node_id: defer_stmt.node_id,
        view: false,
        noblock: true,
        fields: Vec::new(),
        stmts: defer_stmt.block.stmts.clone(),
    };

    let id = ctx.heap_mut().alloc(HeapValue::Func {
        module,
        kind: HeapValueFuncKind::Closure {
            captures: BTreeMap::new(),
            captured_scope: ctx.current_scope_ref(),
            body: func,
        },
    });

    // Push onto the enclosing function scope's deferred list.
    ctx.push_deferred(id);
    EvalResult::Ok(Value::Nil)
}

/// Run deferred closures from the current function scope in LIFO order.
/// Only called on normal completion/return - NOT on fault/internal error (those halt).
/// If a defer block itself faults, we halt immediately.
pub fn run_deferred<'a>(
    ctx: &'a mut EvalContext,
) -> Pin<Box<dyn Future<Output = EvalResult> + 'a>> {
    let deferred = ctx.take_deferred();
    if deferred.is_empty() {
        return Box::pin(std::future::ready(EvalResult::Ok(Value::Nil)));
    }
    Box::pin(async move {
        for rooted in deferred.into_iter().rev() {
            let result = call::call_func_value(ctx, rooted.id(), BTreeMap::new(), None).await;
            match result {
                EvalResult::Fault(_) | EvalResult::InternalError(_) => return result,
                _ => {}
            }
        }
        EvalResult::Ok(Value::Nil)
    })
}

async fn exec_implicitly(ctx: &mut EvalContext, stmt: &StmtImplicitly) -> EvalResult {
    // Resolve each expression's type from the TypeTable (populated at load time).
    let module_idx = match ctx.current_module_index() {
        Some(idx) => idx,
        None => {
            return internal_error!(ctx, "no module context for implicitly");
        }
    };
    let module = ctx.registry.module(module_idx).clone();

    let mut bindings = Vec::with_capacity(stmt.exprs.len());
    for expr in &stmt.exprs {
        let value = match eval_expr(ctx, expr).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        let ty = match module
            .files
            .iter()
            .find_map(|f| f.type_table.node_types.get(&expr.node_id()))
        {
            Some(&t) => t,
            None => {
                return internal_error!(ctx, "implicitly expression has no resolved type");
            }
        };
        // Strip nilable - implicit context is keyed by base type.
        let ty = ctx.registry.nilable_inner(ty).unwrap_or(ty);
        bindings.push(super::scope::ImplicitBinding { ty, value });
    }

    if let Some(block) = &stmt.block {
        let scope = Scope {
            kind: crate::scope::ScopeKind::Implicitly,
            parent: Some(ctx.current_scope_ref()),
            ..Default::default()
        }
        .into_ref();
        ctx.push_exec_frame(scope);
        ctx.exec_stack.last_mut().unwrap().implicits = bindings;
        let result = exec_stmts(ctx, &block.stmts).await;
        ctx.pop_scope();
        result
    } else {
        ctx.exec_stack
            .last_mut()
            .unwrap()
            .implicits
            .extend(bindings);
        EvalResult::Ok(Value::Nil)
    }
}

async fn exec_var(ctx: &mut EvalContext, var: &StmtVar) -> EvalResult {
    for (i, decl) in var.vars.iter().enumerate() {
        let val = if let Some(expr) = var.values.get(i) {
            match eval_expr(ctx, expr).await {
                EvalResult::Ok(v) => v,
                other => return other,
            }
        } else {
            Value::Nil
        };
        // := always declares a new local (shadows any outer binding).
        if let Err(err) = ctx.declare_local(decl.name.name.clone(), val) {
            return err;
        }
    }
    EvalResult::Ok(Value::Nil)
}

async fn exec_assign(ctx: &mut EvalContext, assign: &StmtAssign) -> EvalResult {
    if assign.vars.len() == 1 && assign.values.len() == 1 {
        let val = match eval_expr(ctx, &assign.values[0]).await {
            EvalResult::Ok(v) => v,
            other => return other,
        };
        if let Expr::Ident(ident) = &assign.vars[0] {
            let final_val = if assign.op == StmtAssignOp::Assign {
                val
            } else {
                let old = match ctx.lookup(&ident.name) {
                    EvalResult::Ok(v) => v,
                    other => return other,
                };
                let result = match assign.op {
                    StmtAssignOp::AddAssign => {
                        numeric_op(ctx, &old, &val, "add", |a, b| a + b, |a, b| a + b)
                    }
                    StmtAssignOp::SubAssign => {
                        numeric_op(ctx, &old, &val, "subtract", |a, b| a - b, |a, b| a - b)
                    }
                    StmtAssignOp::MulAssign => {
                        numeric_op(ctx, &old, &val, "multiply", |a, b| a * b, |a, b| a * b)
                    }
                    StmtAssignOp::DivAssign => {
                        numeric_op(ctx, &old, &val, "divide", |a, b| a / b, |a, b| a / b)
                    }
                    StmtAssignOp::ModAssign => {
                        numeric_op(ctx, &old, &val, "modulo", |a, b| a % b, |a, b| a % b)
                    }
                    StmtAssignOp::Assign => unreachable!(),
                };
                match result {
                    EvalResult::Ok(v) => v,
                    other => return other,
                }
            };
            return ctx.set_existing(&ident.name, final_val);
        }
    }
    internal_error!(ctx, "unsupported assignment target")
}

fn exec_for<'a>(
    ctx: &'a mut EvalContext,
    for_stmt: &'a StmtFor,
) -> Pin<Box<dyn Future<Output = EvalResult> + 'a>> {
    Box::pin(async move {
        let label = for_stmt.label.as_ref().map(|l| &l.name);

        match &for_stmt.clause {
            // Infinite loop: `for { ... }`
            None => loop {
                ctx.push_exec_frame(
                    Scope {
                        kind: crate::scope::ScopeKind::For,
                        parent: Some(ctx.current_scope_ref()),
                        ..Default::default()
                    }
                    .into_ref(),
                );
                let result = exec_stmts(ctx, &for_stmt.block.stmts).await;
                ctx.pop_scope();
                match result {
                    EvalResult::Ok(_) => {}
                    EvalResult::Continue(ref l) if l.is_none() || l.as_ref() == label => {
                        continue;
                    }
                    EvalResult::Break(ref l) if l.is_none() || l.as_ref() == label => {
                        break EvalResult::Ok(Value::Nil);
                    }
                    other => break other,
                }
            },

            // Condition loop: `for expr { ... }`
            Some(StmtForClause::Condition(cond)) => loop {
                let val = match eval_expr(ctx, &cond.expr).await {
                    EvalResult::Ok(v) => v,
                    other => break other,
                };
                let Value::Bool(true) = val else {
                    break EvalResult::Ok(Value::Nil);
                };
                ctx.push_exec_frame(
                    Scope {
                        kind: crate::scope::ScopeKind::For,
                        parent: Some(ctx.current_scope_ref()),
                        ..Default::default()
                    }
                    .into_ref(),
                );
                let result = exec_stmts(ctx, &for_stmt.block.stmts).await;
                ctx.pop_scope();
                match result {
                    EvalResult::Ok(_) => {}
                    EvalResult::Continue(ref l) if l.is_none() || l.as_ref() == label => {
                        continue;
                    }
                    EvalResult::Break(ref l) if l.is_none() || l.as_ref() == label => {
                        break EvalResult::Ok(Value::Nil);
                    }
                    other => break other,
                }
            },

            // For-in loop: `for item in <expr> { ... }`
            // The expression evaluates to an iter func (a noblock func that
            // accepts a yielder). The runtime constructs the yielder and
            // calls the func with it.
            Some(StmtForClause::In(for_in)) => {
                use super::yielder::YielderChannel;
                use std::cell::RefCell;

                // Evaluate the iter expression. The result is either:
                // - a func directly (e.g. a local variable holding an iter func)
                // - a compound with an `iter` field (e.g. iter::range(end = 5))
                let iter_func = match eval_expr(ctx, &for_in.expr).await {
                    EvalResult::Ok(Value::Func(fid)) => fid,
                    EvalResult::Ok(Value::Data(did)) => {
                        let heap = ctx.heap();
                        match heap.get(did.id()) {
                            HeapValue::Data { fields, .. } => match fields.get("iter") {
                                Some(Value::Func(fid)) => fid.clone(),
                                Some(other) => {
                                    return internal_error!(
                                        ctx,
                                        "for-in: 'iter' field is not a func, got {:?}",
                                        other
                                    );
                                }
                                None => {
                                    return internal_error!(
                                        ctx,
                                        "for-in: compound result has no 'iter' field"
                                    );
                                }
                            },
                            _ => return internal_error!(ctx, "for-in: expected Data heap value"),
                        }
                    }
                    EvalResult::Ok(other) => {
                        return internal_error!(
                            ctx,
                            "for-in: expected a func or compound with 'iter' field, got {:?}",
                            other
                        );
                    }
                    other => return other,
                };

                // Create yielder channel + entity, then call the iter func.
                let scope_depth = ctx.exec_stack.len();
                let call_stack_depth = ctx.call_stack_len();

                let stmts: Rc<[Stmt]> = for_stmt.block.stmts.clone().into();
                let channel = YielderChannel::new(
                    for_in.var.name.clone(),
                    label.cloned(),
                    stmts,
                    scope_depth,
                    call_stack_depth,
                );
                let channel_id = ctx
                    .heap_mut()
                    .alloc(HeapValue::Native(Box::new(RefCell::new(channel))));
                let channel_value = Value::Native(channel_id.clone());

                // Construct a `duralade.iter::yielder` entity wrapping the channel.
                let iter_module_idx = match ctx.registry.module_index("duralade.iter") {
                    Some(idx) => idx,
                    None => return internal_error!(ctx, "for-in: duralade.iter module not loaded"),
                };

                let mut yielder_args = BTreeMap::new();
                yielder_args.insert("native_yielder".into(), channel_value);
                let yielder_value = match super::member::construct_local_entity(
                    ctx,
                    iter_module_idx,
                    "yielder",
                    yielder_args,
                )
                .await
                {
                    EvalResult::Ok(v) => v,
                    other => return other,
                };

                // Link the channel to the yielder entity so it can update `active`.
                if let Value::Entity(ref eid) = yielder_value {
                    let heap = ctx.heap();
                    if let Some(ch) = YielderChannel::downcast(heap.get(channel_id.id())) {
                        ch.borrow_mut().set_entity_id(eid.id());
                    }
                }

                // Call the iter func with the yielder as a regular arg.
                let mut iter_call_args = BTreeMap::new();
                iter_call_args.insert("yielder".into(), yielder_value);
                let result = call::call_func_value(
                    ctx,
                    iter_func.id(),
                    iter_call_args,
                    Some(for_in.node_id),
                )
                .await;

                // Close the channel so stale yielder calls are safe.
                {
                    let heap = ctx.heap();
                    if let Some(ch) = YielderChannel::downcast(heap.get(channel_id.id())) {
                        ch.borrow_mut().close();
                    }
                }

                // The iter func drove the loop via yielder callbacks.
                // Propagate errors; otherwise the loop completed normally.
                match result {
                    EvalResult::Ok(_) => EvalResult::Ok(Value::Nil),
                    other => other,
                }
            }
        }
    })
}

async fn exec_if(ctx: &mut EvalContext, if_stmt: &StmtIf) -> EvalResult {
    match eval_if_condition(ctx, &if_stmt.condition).await {
        ConditionResult::Matched(bindings) => {
            return exec_if_branch(ctx, &if_stmt.then_block.stmts, bindings).await;
        }
        ConditionResult::NotMatched => {}
        ConditionResult::Early(r) => return r,
    }
    for else_if in &if_stmt.else_ifs {
        match eval_if_condition(ctx, &else_if.condition).await {
            ConditionResult::Matched(bindings) => {
                return exec_if_branch(ctx, &else_if.then_block.stmts, bindings).await;
            }
            ConditionResult::NotMatched => {}
            ConditionResult::Early(r) => return r,
        }
    }
    if let Some(else_block) = &if_stmt.else_block {
        return exec_if_branch(ctx, &else_block.stmts, vec![]).await;
    }
    EvalResult::Ok(Value::Nil)
}

enum ConditionResult {
    Matched(Vec<(Symbol, Value)>),
    NotMatched,
    /// Expression evaluation produced a non-Ok EvalResult (Return, Break, etc.)
    Early(EvalResult),
}

async fn eval_if_condition(ctx: &mut EvalContext, cond: &StmtIfCondition) -> ConditionResult {
    match cond {
        StmtIfCondition::Bool(b) => {
            let val = match eval_expr(ctx, &b.expr).await {
                EvalResult::Ok(v) => v,
                other => return ConditionResult::Early(other),
            };
            if let Value::Bool(true) = val {
                ConditionResult::Matched(vec![])
            } else {
                ConditionResult::NotMatched
            }
        }
        StmtIfCondition::NarrowingNil(n) => {
            let mut bindings = Vec::with_capacity(n.bindings.len());
            for binding in &n.bindings {
                let val = match eval_expr(ctx, &binding.expr).await {
                    EvalResult::Ok(v) => v,
                    other => return ConditionResult::Early(other),
                };
                if matches!(val, Value::Nil) {
                    return ConditionResult::NotMatched;
                }
                bindings.push((binding.name.name.clone(), val));
            }
            ConditionResult::Matched(bindings)
        }
        StmtIfCondition::NarrowingAs(n) => {
            let mut bindings = Vec::with_capacity(n.bindings.len());
            for binding in &n.bindings {
                let val = match eval_expr(ctx, &binding.expr).await {
                    EvalResult::Ok(v) => v,
                    other => return ConditionResult::Early(other),
                };
                let Some(mut target_type) = ctx.lookup_as_narrowing_type(binding.node_id) else {
                    return ConditionResult::Early(internal_error!(
                        ctx,
                        "if-as narrowing has no resolved target type"
                    ));
                };
                if let crate::load::registry::TypeEntry::TypeParam(name) =
                    ctx.registry.type_entry(target_type)
                {
                    let name = name.clone();
                    if let Some(concrete) = ctx.resolve_type_param(&name) {
                        target_type = concrete;
                    }
                }
                let heap = ctx.heap();
                let is_match = super::assignable::runtime_is_assignable(
                    &val,
                    target_type,
                    &heap,
                    &ctx.registry,
                );
                drop(heap);
                if !is_match {
                    return ConditionResult::NotMatched;
                }
                bindings.push((binding.name.name.clone(), val));
            }
            ConditionResult::Matched(bindings)
        }
    }
}

/// Execute a then/else-if/else branch in its own block scope,
/// injecting any narrowing bindings before running the body.
async fn exec_if_branch(
    ctx: &mut EvalContext,
    stmts: &[Stmt],
    bindings: Vec<(Symbol, Value)>,
) -> EvalResult {
    ctx.push_exec_frame(
        Scope {
            kind: crate::scope::ScopeKind::Block,
            parent: Some(ctx.current_scope_ref()),
            ..Default::default()
        }
        .into_ref(),
    );
    for (name, value) in bindings {
        if let Err(err) = ctx.declare_local(name, value) {
            ctx.pop_scope();
            return err;
        }
    }
    let result = exec_stmts(ctx, stmts).await;
    ctx.pop_scope();
    result
}
