use duralade_language::engine::native::{
    NativeArgs, NativeCallContext, NativeCallError, NativeCallFuture, NativeOut, NativeRegistry,
};
use duralade_language::interpret::{scope::InternalError, value::Value};
use duralade_language::model::Symbol;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    registry.register("duralade.str::str.find", str_find)?;
    Ok(())
}

fn str_find<'a>(
    ctx: &'a mut NativeCallContext<'a>,
    args: &'a NativeArgs,
    out: &'a mut NativeOut,
) -> NativeCallFuture<'a> {
    Box::pin(async move {
        let Some(Value::Str(haystack)) = ctx.construct_value() else {
            return Err(NativeCallError::InternalError(InternalError::new(
                "str.find: expected str construct value",
            )));
        };

        let Some(Value::Str(substr)) = args.get("substr") else {
            return Err(NativeCallError::InternalError(InternalError::new(
                "str.find: missing or invalid arg 'substr'",
            )));
        };

        let bytes_mode = matches!(args.get("bytes"), Some(Value::Bool(true)));

        let index = if bytes_mode {
            haystack.find(&**substr).map(|i| i as i64)
        } else {
            // Character index: find the byte position, then count chars up to it.
            // TODO: O(n) char counting per call - consider a char-indexed string
            // representation or at least caching length metadata on Value::Str.
            haystack
                .find(&**substr)
                .map(|byte_idx| haystack[..byte_idx].chars().count() as i64)
        };

        match index {
            Some(i) => out.insert(Symbol::result(), Value::Int(i)),
            None => out.insert(Symbol::result(), Value::Nil),
        };

        Ok(())
    })
}
