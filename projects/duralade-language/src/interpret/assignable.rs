use std::collections::BTreeMap;

use crate::engine::ModuleIndex;
use crate::load::registry::{DeclId, Registry, TypeEntry, TypeId};
use crate::load::{FieldModifier, TypeConstructKind, TypeField};
use crate::model::Symbol;

use super::value::{Heap, HeapValue, HeapValueFuncKind, Value, ValueId};

/// Check whether a runtime value is assignable to the given target type.
/// Used by `as` narrowing expressions at runtime.
pub fn runtime_is_assignable(
    value: &Value,
    target: TypeId,
    heap: &Heap,
    registry: &Registry,
) -> bool {
    // Exact primitive matches
    match value {
        Value::Int(_) if target == registry.int_type => return true,
        Value::Float(_) if target == registry.float_type => return true,
        Value::Str(_) if target == registry.str_type => return true,
        Value::Bool(_) if target == registry.bool_type => return true,
        _ => {}
    }

    // anylocal accepts everything
    if registry.is_anylocal(target) {
        return true;
    }

    // any accepts serializable values (no func/entity)
    if registry.is_any(target) {
        return is_value_serializable(value);
    }

    // Nilable target: nil matches, or inner type matches
    if let Some(inner) = registry.nilable_inner(target) {
        return matches!(value, Value::Nil) || runtime_is_assignable(value, inner, heap, registry);
    }

    // Nil only matches nilable targets (handled above) and nil itself
    if matches!(value, Value::Nil) {
        return false;
    }

    match value {
        Value::Data(id) => check_data_assignable(id.id(), target, heap, registry),
        Value::Entity(id) => check_entity_assignable(id.id(), target, heap, registry),
        Value::Func(id) => check_func_assignable(id.id(), target, heap, registry),
        Value::EntityRef(_) => {
            // EntityRef is assignable to entity ref types
            if let TypeEntry::EntityRef(_) = registry.type_entry(target) {
                return true;
            }
            false
        }
        Value::Native(_) => false,
        // Primitives not matched above — wrong target type
        _ => false,
    }
}

fn is_value_serializable(value: &Value) -> bool {
    match value {
        Value::Int(_) | Value::Float(_) | Value::Str(_) | Value::Bool(_) | Value::Nil => true,
        Value::Data(_) => true,
        Value::Func(_) | Value::Entity(_) | Value::EntityRef(_) | Value::Native(_) => false,
        Value::Type(_) => false,
    }
}

fn check_data_assignable(
    value_id: ValueId,
    target: TypeId,
    heap: &Heap,
    registry: &Registry,
) -> bool {
    let HeapValue::Data { decl, fields, .. } = heap.get(value_id) else {
        return false;
    };

    match registry.type_entry(target) {
        // Named target: nominal check — DeclIds must match
        TypeEntry::Named {
            decl: target_decl,
            type_args: _,
        } => {
            if let Some(value_decl) = decl
                && value_decl == target_decl
            {
                // Same decl — if target has type args, we'd need to check them too.
                // For now, nominal match on DeclId is sufficient since type args
                // are erased at runtime (phase 2 will add runtime type arg tracking).
                return true;
            }
            // Named data with a different DeclId: check if target is a type alias
            // that resolves to something structurally compatible.
            if let Some(target_fields) = resolve_named_to_fields(registry, *target_decl) {
                return check_fields_assignable(fields, &target_fields, heap, registry);
            }
            false
        }
        // Anonymous target: structural (duck typed)
        TypeEntry::Anonymous(target_construct) => {
            if target_construct.kind != TypeConstructKind::Data {
                return false;
            }
            check_fields_assignable(fields, &target_construct.fields, heap, registry)
        }
        _ => false,
    }
}

fn check_entity_assignable(
    value_id: ValueId,
    target: TypeId,
    heap: &Heap,
    registry: &Registry,
) -> bool {
    let HeapValue::Entity { decl, .. } = heap.get(value_id) else {
        return false;
    };

    match registry.type_entry(target) {
        // Named target: nominal check
        TypeEntry::Named {
            decl: target_decl, ..
        } => decl == target_decl,
        // Entities don't structurally match anonymous types
        _ => false,
    }
}

fn check_func_assignable(
    value_id: ValueId,
    target: TypeId,
    heap: &Heap,
    registry: &Registry,
) -> bool {
    let HeapValue::Func { module, kind, .. } = heap.get(value_id) else {
        return false;
    };

    // Get the target's fields for structural comparison
    let target_fields = match registry.type_entry(target) {
        TypeEntry::Anonymous(construct) => {
            if construct.kind != TypeConstructKind::Func {
                return false;
            }
            &construct.fields
        }
        TypeEntry::Named { decl, .. } => {
            // Named func type — look up the declaration's fields
            let entry = registry.decl(*decl);
            let module = registry.module(entry.module);
            let Some(declaration) = module.get_declaration(&entry.name) else {
                return false;
            };
            if declaration.kind != TypeConstructKind::Func {
                return false;
            }
            &declaration.fields
        }
        TypeEntry::MemberFunc {
            decl,
            member,
            type_args,
        } => {
            // Member func type — look up member fields and substitute type args
            return check_func_against_member(*module, kind, *decl, member, type_args, registry);
        }
        _ => return false,
    };

    check_func_fields_match(*module, kind, target_fields, registry)
}

/// Get the fields of the func value for comparison.
fn get_func_value_fields<'a>(
    module: ModuleIndex,
    kind: &HeapValueFuncKind,
    registry: &'a Registry,
) -> Option<Vec<&'a TypeField>> {
    match kind {
        HeapValueFuncKind::Closure { body, .. } => {
            // Look up the closure's resolved type from the type table
            let reg_module = registry.module(module);
            let type_id = reg_module
                .files
                .iter()
                .find_map(|f| f.type_table.node_types.get(&body.node_id).copied())?;
            match registry.type_entry(type_id) {
                TypeEntry::Anonymous(construct) => Some(construct.fields.iter().collect()),
                _ => None,
            }
        }
        HeapValueFuncKind::Decl { decl, name, .. } => {
            let entry = registry.decl(*decl);
            let reg_module = registry.module(entry.module);
            let declaration = reg_module.get_declaration(&entry.name)?;
            // If name matches the declaration name, it's a top-level func ref
            if *name == entry.name {
                Some(declaration.fields.iter().collect())
            } else {
                // Member func ref
                let member = declaration.get_member(name)?;
                Some(member.fields.iter().collect())
            }
        }
    }
}

fn check_func_fields_match(
    module: ModuleIndex,
    kind: &HeapValueFuncKind,
    target_fields: &[TypeField],
    registry: &Registry,
) -> bool {
    let Some(value_fields) = get_func_value_fields(module, kind, registry) else {
        return false;
    };

    // Check that target's fields are present on the value's func with compatible types.
    // Func assignability is structural: target's required fields must exist on the value.
    for target_field in target_fields {
        let Some(value_field) = value_fields.iter().find(|f| f.name == target_field.name) else {
            // Target requires a field the value func doesn't have
            if target_field.has_default {
                continue; // Optional field — ok to be missing
            }
            return false;
        };
        if !is_modifier_compatible(value_field.modifier, target_field.modifier) {
            return false;
        }
        // For in params: target type must be assignable to value type (contravariant)
        // For out params: value type must be assignable to target type (covariant)
        // For simplicity, check bidirectional compatibility for now
        if value_field.ty != target_field.ty {
            // Types differ — check assignability based on modifier direction
            match target_field.modifier {
                Some(FieldModifier::Out) | Some(FieldModifier::OutEarly) => {
                    if !is_type_assignable(value_field.ty, target_field.ty, registry) {
                        return false;
                    }
                }
                Some(FieldModifier::In) => {
                    if !is_type_assignable(target_field.ty, value_field.ty, registry) {
                        return false;
                    }
                }
                Some(FieldModifier::Inout) => {
                    // Invariant
                    return false;
                }
                _ => {
                    if !is_type_assignable(value_field.ty, target_field.ty, registry) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

fn check_func_against_member(
    module: ModuleIndex,
    kind: &HeapValueFuncKind,
    target_decl: DeclId,
    target_member: &Symbol,
    type_args: &[crate::load::registry::RegistryTypeArg],
    registry: &Registry,
) -> bool {
    let entry = registry.decl(target_decl);
    let reg_module = registry.module(entry.module);
    let Some(declaration) = reg_module.get_declaration(&entry.name) else {
        return false;
    };
    let Some(member) = declaration.get_member(target_member) else {
        return false;
    };

    if type_args.is_empty() {
        return check_func_fields_match(module, kind, &member.fields, registry);
    }

    // Substitute type args into member fields
    // We can't mutate the registry, so we need to check assignability
    // using the pre-substituted type args manually.
    // For now, fall back to structural checking without substitution
    // (phase 2 will handle this properly with runtime type args).
    check_func_fields_match(module, kind, &member.fields, registry)
}

/// Check type-level assignability (without a runtime value).
/// Simplified version for field type comparisons at runtime.
fn is_type_assignable(from: TypeId, to: TypeId, registry: &Registry) -> bool {
    if from == to {
        return true;
    }
    if registry.is_anylocal(to) {
        return true;
    }
    if registry.is_any(to) {
        return registry.is_type_serializable(from);
    }
    if let Some(inner) = registry.nilable_inner(to) {
        if registry.nilable_inner(from).is_some() {
            let from_inner = registry.nilable_inner(from).unwrap();
            return is_type_assignable(from_inner, inner, registry);
        }
        return is_type_assignable(from, inner, registry);
    }
    // Named types: same DeclId with compatible type args
    if let (
        TypeEntry::Named {
            decl: d1,
            type_args: ta1,
        },
        TypeEntry::Named {
            decl: d2,
            type_args: ta2,
        },
    ) = (registry.type_entry(from), registry.type_entry(to))
        && d1 == d2
        && ta1.len() == ta2.len()
    {
        return ta1
            .iter()
            .zip(ta2.iter())
            .all(|(a, b)| is_type_assignable(a.value, b.value, registry));
    }
    false
}

/// Check if value fields structurally match target fields (duck typing for data).
fn check_fields_assignable(
    value_fields: &BTreeMap<Symbol, Value>,
    target_fields: &[TypeField],
    heap: &Heap,
    registry: &Registry,
) -> bool {
    for target_field in target_fields {
        let Some(value) = value_fields.get(&target_field.name) else {
            if target_field.has_default {
                continue;
            }
            return false;
        };
        if !runtime_is_assignable(value, target_field.ty, heap, registry) {
            return false;
        }
    }
    true
}

/// Resolve a named type (possibly a type alias) to its structural fields.
/// Returns None if the type is not a data type or type alias to data.
fn resolve_named_to_fields(registry: &Registry, decl: DeclId) -> Option<Vec<TypeField>> {
    let entry = registry.decl(decl);
    let module = registry.module(entry.module);
    let declaration = module.get_declaration(&entry.name)?;
    match declaration.kind {
        TypeConstructKind::TypeAlias => {
            let target_id = declaration.type_alias_target?;
            match registry.type_entry(target_id) {
                TypeEntry::Anonymous(construct) if construct.kind == TypeConstructKind::Data => {
                    Some(construct.fields.clone())
                }
                _ => None,
            }
        }
        TypeConstructKind::Data => Some(declaration.fields.clone()),
        _ => None,
    }
}

fn is_modifier_compatible(from: Option<FieldModifier>, to: Option<FieldModifier>) -> bool {
    match (from, to) {
        (_, None) => true,
        (None, Some(_)) => false,
        (Some(f), Some(t)) => match t {
            FieldModifier::In => matches!(f, FieldModifier::In | FieldModifier::Inout),
            FieldModifier::Out => matches!(f, FieldModifier::Out | FieldModifier::Inout),
            FieldModifier::Inout => matches!(f, FieldModifier::Inout),
            FieldModifier::OutEarly => matches!(f, FieldModifier::OutEarly),
            FieldModifier::Value => matches!(f, FieldModifier::Value),
            FieldModifier::Implicit => matches!(f, FieldModifier::Implicit),
        },
    }
}
