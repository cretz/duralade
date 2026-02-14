use std::any::Any;
use std::collections::BTreeMap;

use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallError, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::native_collection::{MapKey, NativeMap};
use duralade_language::interpret::scope::{FaultError, InternalError};
use duralade_language::interpret::value::{HeapValue, Value};
use duralade_language::interpret::yielder::NativeYielder;
use duralade_language::model::Symbol;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.map::map.contains_key", map_contains_key)?;
    registry.register("duralade.map::map.get", map_get)?;
    registry.register("duralade.map::map.iter", map_iter)?;
    registry.register("duralade.map::map.len", map_len)?;
    registry.register("duralade.map::map.remove", map_remove)?;
    registry.register("duralade.map::map.set", map_set)?;
    Ok(())
}

/// Extract the NativeMap from a construct value (Data with `_items` = Native(NativeMap)).
fn with_native_map<F, R>(ctx: &NativeCallContext, method: &str, f: F) -> Result<R, NativeCallError>
where
    F: FnOnce(&NativeMap) -> R,
{
    let Some(Value::Data(data_id)) = ctx.construct_value() else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: expected Data construct value"
        ))));
    };
    let heap = ctx.eval_ctx.heap();
    let HeapValue::Data { fields, .. } = heap.get(data_id.id()) else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: construct is not a Data"
        ))));
    };
    let Some(Value::Native(native_id)) = fields.get("_items") else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: missing _items field"
        ))));
    };
    let HeapValue::Native(native) = heap.get(native_id.id()) else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: _items is not a Native"
        ))));
    };
    let Some(m) = (native.as_ref() as &dyn Any).downcast_ref::<NativeMap>() else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: _items is not a NativeMap"
        ))));
    };
    Ok(f(m))
}

fn require_arg<'a>(
    args: &'a NativeArgs,
    name: &str,
    method: &str,
) -> Result<&'a Value, NativeCallError> {
    args.get(name).ok_or_else(|| {
        NativeCallError::InternalError(InternalError::new(format!(
            "map.{method}: missing arg '{name}'"
        )))
    })
}

// map.contains_key(key: k) -> result: bool
fn map_contains_key<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let key = require_arg(args, "key", "contains_key")?;
        let heap = ctx.eval_ctx.heap();
        let lookup = MapKey::new(key.clone(), &heap).map_err(|msg| {
            NativeCallError::Fault(FaultError {
                message: format!("map.contains_key: {msg}"),
                stack: Vec::new(),
            })
        })?;
        let result = with_native_map(ctx, "contains_key", |m| m.0.borrow().contains_key(&lookup))?;
        out.insert("result".into(), Value::Bool(result));
        Ok(())
    })
}

// map.get(key: k) -> value: v?
fn map_get<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let key = require_arg(args, "key", "get")?;
        let heap = ctx.eval_ctx.heap();
        let lookup = MapKey::new(key.clone(), &heap).map_err(|msg| {
            NativeCallError::Fault(FaultError {
                message: format!("map.get: {msg}"),
                stack: Vec::new(),
            })
        })?;
        let value = with_native_map(ctx, "get", |m| {
            m.0.borrow().get(&lookup).cloned().unwrap_or(Value::Nil)
        })?;
        out.insert(Symbol::result(), value);
        Ok(())
    })
}

// map.len() -> count: int
fn map_len<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    _args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let len = with_native_map(ctx, "len", |m| m.0.borrow().len() as i64)?;
        out.insert(Symbol::result(), Value::Int(len));
        Ok(())
    })
}

// map.remove(key: k) -> value: v?
fn map_remove<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let key = require_arg(args, "key", "remove")?;
        let heap = ctx.eval_ctx.heap();
        let lookup = MapKey::new(key.clone(), &heap).map_err(|msg| {
            NativeCallError::Fault(FaultError {
                message: format!("map.remove: {msg}"),
                stack: Vec::new(),
            })
        })?;
        let value = with_native_map(ctx, "remove", |m| {
            m.0.borrow_mut().shift_remove(&lookup).unwrap_or(Value::Nil)
        })?;
        out.insert(Symbol::result(), value);
        Ok(())
    })
}

// map.set(key: k, value: v) -> old_value: v?
fn map_set<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let key = require_arg(args, "key", "set")?.clone();
        let value = require_arg(args, "value", "set")?.clone();
        let heap = ctx.eval_ctx.heap();
        let map_key = MapKey::new(key, &heap).map_err(|msg| {
            NativeCallError::Fault(FaultError {
                message: format!("map.set: {msg}"),
                stack: Vec::new(),
            })
        })?;
        let old = with_native_map(ctx, "set", |m| {
            m.0.borrow_mut()
                .insert(map_key, value)
                .unwrap_or(Value::Nil)
        })?;
        out.insert(Symbol::old_value(), old);
        Ok(())
    })
}

/// Extract the native_yielder channel id from a yielder entity value.
fn extract_native_yielder(
    ctx: &NativeCallContext,
    args: &NativeArgs,
) -> Result<Value, NativeCallError> {
    let Some(Value::Entity(eid)) = args.get("yielder") else {
        return Err(NativeCallError::InternalError(InternalError::new(
            "map.iter: missing or invalid 'yielder' arg",
        )));
    };
    let heap = ctx.eval_ctx.heap();
    let HeapValue::Entity { fields, .. } = heap.get(eid.id()) else {
        return Err(NativeCallError::InternalError(InternalError::new(
            "map.iter: yielder is not an Entity",
        )));
    };
    let Some(native_yielder) = fields.get("native_yielder") else {
        return Err(NativeCallError::InternalError(InternalError::new(
            "map.iter: yielder entity missing native_yielder field",
        )));
    };
    Ok(native_yielder.clone())
}

// map.iter(yielder: yielder[t = entry[k, v]])
fn map_iter<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    _out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        // Collect entries up front so we release the RefCell borrow.
        let entries: Vec<(Value, Value)> = with_native_map(ctx, "iter", |m| {
            m.0.borrow()
                .iter()
                .map(|(k, v)| (k.value().clone(), v.clone()))
                .collect()
        })?;

        // Look up the entry DeclId from the map module.
        let map_module_idx = ctx.eval_ctx.registry.module_index(&Symbol::mod_map());
        let entry_decl =
            map_module_idx.and_then(|idx| ctx.eval_ctx.registry.decl_id_in_module(idx, "entry"));

        // Yield each entry.
        for (key, value) in entries {
            // Extract and validate the yielder each iteration.
            let ny_value = extract_native_yielder(ctx, args)?;
            let Value::Native(ny_id) = ny_value else {
                return Err(NativeCallError::InternalError(InternalError::new(
                    "map.iter: native_yielder is not a Native value",
                )));
            };
            let Some(ny) = NativeYielder::validate(ctx.eval_ctx.heap().get(ny_id.id()), ny_id.id())
            else {
                return Err(NativeCallError::InternalError(InternalError::new(
                    "map.iter: native_yielder is not a valid YielderChannel",
                )));
            };

            // Build an entry Data value.
            let mut entry_fields = BTreeMap::new();
            entry_fields.insert(Symbol::key(), key);
            entry_fields.insert(Symbol::value(), value);
            let entry_value = Value::Data(ctx.eval_ctx.heap_mut().alloc(HeapValue::Data {
                decl: entry_decl,
                out_early_name: None,
                fields: entry_fields,
                type_args: BTreeMap::new(),
            }));

            match ny.exec_call(ctx.eval_ctx, entry_value).await {
                duralade_language::interpret::eval::EvalResult::Ok(Value::Bool(true)) => {}
                duralade_language::interpret::eval::EvalResult::Ok(_) => break,
                duralade_language::interpret::eval::EvalResult::Fault(e) => {
                    return Err(NativeCallError::Fault(e));
                }
                duralade_language::interpret::eval::EvalResult::InternalError(e) => {
                    return Err(NativeCallError::InternalError(e));
                }
                other => return Err(NativeCallError::ControlFlow(other)),
            }
        }

        Ok(())
    })
}
