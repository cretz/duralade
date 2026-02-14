use std::any::Any;

use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallError, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::native_collection::NativeArray;
use duralade_language::interpret::scope::{FaultError, InternalError};
use duralade_language::interpret::value::{HeapValue, Value};
use duralade_language::model::Symbol;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.array::array.get", array_get)?;
    registry.register("duralade.array::array.len", array_len)?;
    registry.register("duralade.array::array.push", array_push)?;
    registry.register("duralade.array::array.remove", array_remove)?;
    registry.register("duralade.array::array.set", array_set)?;
    Ok(())
}

/// Extract the NativeArray's inner Vec from a construct value.
/// The construct value is a Data whose `_items` field is a Native(NativeArray).
fn with_native_array<F, R>(
    ctx: &NativeCallContext,
    method: &str,
    f: F,
) -> Result<R, NativeCallError>
where
    F: FnOnce(&NativeArray) -> R,
{
    let Some(Value::Data(data_id)) = ctx.construct_value() else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: expected Data construct value"
        ))));
    };
    let heap = ctx.eval_ctx.heap();
    let HeapValue::Data { fields, .. } = heap.get(data_id.id()) else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: construct is not a Data"
        ))));
    };
    let Some(Value::Native(native_id)) = fields.get("_items") else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: missing _items field"
        ))));
    };
    let HeapValue::Native(native) = heap.get(native_id.id()) else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: _items is not a Native"
        ))));
    };
    let Some(arr) = (native.as_ref() as &dyn Any).downcast_ref::<NativeArray>() else {
        return Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: _items is not a NativeArray"
        ))));
    };
    Ok(f(arr))
}

fn require_int_arg(args: &NativeArgs, name: &str, method: &str) -> Result<i64, NativeCallError> {
    match args.get(name) {
        Some(Value::Int(n)) => Ok(*n),
        _ => Err(NativeCallError::InternalError(InternalError::new(format!(
            "array.{method}: missing or invalid arg '{name}'"
        )))),
    }
}

// array.get(index: int) -> value: t?
fn array_get<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let index = require_int_arg(args, "index", "get")?;
        let value = with_native_array(ctx, "get", |arr| {
            let items = arr.0.borrow();
            if index < 0 || (index as usize) >= items.len() {
                Value::Nil
            } else {
                items[index as usize].clone()
            }
        })?;
        out.insert(Symbol::result(), value);
        Ok(())
    })
}

// array.len() -> count: int
fn array_len<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    _args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let len = with_native_array(ctx, "len", |arr| arr.0.borrow().len() as i64)?;
        out.insert(Symbol::result(), Value::Int(len));
        Ok(())
    })
}

// array.push(value: t)
fn array_push<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    _out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let Some(value) = args.get("value") else {
            return Err(NativeCallError::InternalError(InternalError::new(
                "array.push: missing arg 'value'",
            )));
        };
        with_native_array(ctx, "push", |arr| {
            arr.0.borrow_mut().push(value.clone());
        })?;
        Ok(())
    })
}

// array.remove(index: int) -> value: t?
fn array_remove<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let index = require_int_arg(args, "index", "remove")?;
        let value = with_native_array(ctx, "remove", |arr| {
            let mut items = arr.0.borrow_mut();
            if index < 0 || (index as usize) >= items.len() {
                Value::Nil
            } else {
                items.remove(index as usize)
            }
        })?;
        out.insert(Symbol::result(), value);
        Ok(())
    })
}

// array.set(index: int, value: t) -> old_value: t
// Faults on out-of-bounds.
fn array_set<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let index = require_int_arg(args, "index", "set")?;
        let Some(new_value) = args.get("value") else {
            return Err(NativeCallError::InternalError(InternalError::new(
                "array.set: missing arg 'value'",
            )));
        };
        let result = with_native_array(ctx, "set", |arr| {
            let mut items = arr.0.borrow_mut();
            if index < 0 || (index as usize) >= items.len() {
                Err(format!(
                    "array.set: index {} out of bounds (len {})",
                    index,
                    items.len()
                ))
            } else {
                let old = std::mem::replace(&mut items[index as usize], new_value.clone());
                Ok(old)
            }
        })?;
        match result {
            Ok(old_value) => {
                out.insert(Symbol::old_value(), old_value);
                Ok(())
            }
            Err(msg) => Err(NativeCallError::Fault(FaultError {
                message: msg,
                stack: Vec::new(),
            })),
        }
    })
}
