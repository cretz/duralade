use duralade_language::engine::native::NativeCallError;
use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::scope::FaultError;
use duralade_language::interpret::value::Value;
use duralade_language::interpret::yielder::NativeYielder;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.iter::native_yield", native_yield)?;
    Ok(())
}

fn native_yield<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let native_yielder = if let Some(Value::Native(id)) = args.get("native_yielder")
            && let Some(ny) = NativeYielder::validate(ctx.eval_ctx.heap().get(id.id()), id.id())
        {
            ny
        } else {
            return Err(NativeCallError::Fault(FaultError {
                message: "native_yield: invalid 'native_yielder' \
                          (did you construct a yielder entity manually?)"
                    .into(),
                stack: Vec::new(),
            }));
        };

        let yielded_value = args.get("value").cloned().unwrap_or(Value::Nil);

        match native_yielder.exec_call(ctx.eval_ctx, yielded_value).await {
            duralade_language::interpret::eval::EvalResult::Ok(Value::Bool(active)) => {
                out.insert("active".into(), Value::Bool(active));
                Ok(())
            }
            duralade_language::interpret::eval::EvalResult::Ok(_) => {
                out.insert("active".into(), Value::Bool(false));
                Ok(())
            }
            duralade_language::interpret::eval::EvalResult::Fault(e) => {
                Err(NativeCallError::Fault(e))
            }
            duralade_language::interpret::eval::EvalResult::InternalError(e) => {
                Err(NativeCallError::InternalError(e))
            }
            // Break/continue/return targeting an outer scope - propagate through.
            other => Err(NativeCallError::ControlFlow(other)),
        }
    })
}
