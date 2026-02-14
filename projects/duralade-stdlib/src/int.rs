use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallError, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::{scope::InternalError, value::Value};
use duralade_language::model::Symbol;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.int::from_str", int_from_str)?;
    Ok(())
}

fn int_from_str<'a>(
    _ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let Some(Value::Str(s)) = args.get("str") else {
            return Err(NativeCallError::InternalError(InternalError::new(
                "int.from_str: missing or invalid arg 'str'",
            )));
        };

        let value = s
            .trim()
            .parse::<i64>()
            .ok()
            .map(Value::Int)
            .unwrap_or(Value::Nil);
        out.insert(Symbol::result(), value);

        Ok(())
    })
}
