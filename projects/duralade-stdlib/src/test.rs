use std::collections::BTreeMap;

use duralade_language::engine::native::NativeCallError;
use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::scope::FaultError;
use duralade_language::interpret::value::{HeapValue, Value};
use duralade_language::model::Symbol;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.test::on_extern_native", on_extern_native)?;

    Ok(())
}

/// Allocate a `duralade.error::simple` data value on the heap.
// TODO: provide a helper on NativeCallContext (e.g. ctx.make_error(message)) so native
// authors don't have to manually construct DeclRef + heap alloc for common error returns.
fn alloc_error(ctx: &mut NativeCallContext, message: String) -> Value {
    let mut fields = BTreeMap::new();
    fields.insert(Symbol::message(), Value::Str(message.into()));
    let id = ctx.eval_ctx.heap_mut().alloc(HeapValue::Data {
        decl: None,
        out_early_name: None,
        fields,
        type_args: BTreeMap::new(),
    });
    Value::Data(id)
}

fn on_extern_native<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let extern_name = match args.get("extern") {
            Some(Value::Str(name)) => name.to_string(),
            _ => {
                return Err(NativeCallError::Fault(FaultError {
                    message: "on_extern_native expected arg 'extern: str'".into(),
                    stack: Vec::new(),
                }));
            }
        };
        let func = match args.get("handler") {
            Some(v @ Value::Func(_)) => v.clone(),
            _ => {
                return Err(NativeCallError::Fault(FaultError {
                    message: "on_extern_native expected arg 'handler: func {}'".into(),
                    stack: Vec::new(),
                }));
            }
        };

        if let Err(message) = ctx.eval_ctx.set_extern_override(extern_name, func) {
            out.insert("error".into(), alloc_error(ctx, message));
        }
        Ok(())
    })
}
