use duralade_language::engine::native::NativeRegistry;

mod array;
mod int;
mod iter;
mod map;
mod str;
mod test;

pub fn apply_to_native_registry(registry: &mut NativeRegistry) -> Result<(), String> {
    array::apply_to_native_registry(registry)?;
    int::apply_to_native_registry(registry)?;
    iter::apply_to_native_registry(registry)?;
    map::apply_to_native_registry(registry)?;
    str::apply_to_native_registry(registry)?;
    test::apply_to_native_registry(registry)?;
    Ok(())
}
