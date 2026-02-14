use crate::model::Symbol;

use super::{CheckContext, anon_type_construct};
use crate::load::{
    FieldModifier, FuncModifiers, TypeConstructKind, TypeField,
    registry::{DeclId, TypeEntry, TypeId},
};

pub(super) fn check_assignable(
    cc: &mut CheckContext,
    from: TypeId,
    to: TypeId,
) -> Result<(), String> {
    if from == to {
        return Ok(());
    }
    let error_type = cc.registry.error_type;
    // Error types are permissive (avoid cascading errors)
    if from == error_type || to == error_type {
        return Ok(());
    }
    // Extract structure before recursing to avoid deadlock on RwLock
    let (
        to_is_any,
        to_is_anylocal,
        to_nilable_inner,
        from_nilable_inner,
        to_is_type_param,
        from_is_type_param,
    ) = {
        (
            cc.registry.is_any(to),
            cc.registry.is_anylocal(to),
            cc.registry.nilable_inner(to),
            cc.registry.nilable_inner(from),
            matches!(cc.registry.type_entry(to), TypeEntry::TypeParam(_)),
            matches!(cc.registry.type_entry(from), TypeEntry::TypeParam(_)),
        )
    };
    let (from_is_any, from_is_anylocal) = (cc.registry.is_any(from), cc.registry.is_anylocal(from));
    // anylocal: true top type, everything is assignable
    if to_is_anylocal || from_is_anylocal {
        return Ok(());
    }
    // any: serializable top type - only serializable types are assignable
    if to_is_any {
        let is_ser = cc.registry.is_type_serializable(from);
        if is_ser {
            return Ok(());
        }
        return Err(format!(
            "cannot assign non-serializable type {} to any",
            cc.registry.display_type(from)
        ));
    }
    if from_is_any {
        return Ok(());
    }
    // Type params are permissive (like any) - constraint checking is future work.
    if to_is_type_param || from_is_type_param {
        return Ok(());
    }
    // Both nilable: check inner types are assignable
    if let Some(to_inner) = to_nilable_inner
        && let Some(from_inner) = from_nilable_inner
    {
        return check_assignable(cc, from_inner, to_inner);
    }
    // Any non-nil type is assignable to its nilable form
    if let Some(inner) = to_nilable_inner {
        return check_assignable(cc, from, inner);
    }
    // Nilable(Error) is also permissive
    if let Some(inner) = from_nilable_inner
        && inner == error_type
    {
        return Ok(());
    }
    // Same named type with different type args: check args pairwise.
    let named_match = {
        if let (
            TypeEntry::Named {
                decl: d1,
                type_args: ta1,
            },
            TypeEntry::Named {
                decl: d2,
                type_args: ta2,
            },
        ) = (cc.registry.type_entry(from), cc.registry.type_entry(to))
        {
            if d1 == d2 && ta1.len() == ta2.len() {
                Some(
                    ta1.iter()
                        .zip(ta2.iter())
                        .map(|(a, b)| (a.value, b.value))
                        .collect::<Vec<_>>(),
                )
            } else {
                None
            }
        } else {
            None
        }
    };
    if let Some(pairs) = named_match {
        for (from_arg, to_arg) in pairs {
            check_assignable_extended(cc, from_arg, to_arg)?;
        }
        return Ok(());
    }
    Err(format!(
        "expected {}, got {}",
        cc.registry.display_type(to),
        cc.registry.display_type(from)
    ))
}

pub(super) fn check_assignable_extended(
    cc: &mut CheckContext,
    from: TypeId,
    to: TypeId,
) -> Result<(), String> {
    if let Ok(()) = check_assignable(cc, from, to) {
        return Ok(());
    }
    let from_name = cc.registry.display_type(from);
    let to_name = cc.registry.display_type(to);
    let from_entry_kind = match cc.registry.type_entry(from) {
        TypeEntry::Nilable(inner) => EntryKind::Nilable(*inner),
        TypeEntry::Anonymous(_) | TypeEntry::MemberFunc { .. } => EntryKind::Anonymous,
        TypeEntry::Named { decl, .. } => EntryKind::Named(*decl),
        _ => EntryKind::Other,
    };
    let to_entry_kind = match cc.registry.type_entry(to) {
        TypeEntry::Nilable(inner) => EntryKind::Nilable(*inner),
        TypeEntry::Anonymous(_) | TypeEntry::MemberFunc { .. } => EntryKind::Anonymous,
        TypeEntry::Named { decl, .. } => EntryKind::Named(*decl),
        _ => EntryKind::Other,
    };
    match (from_entry_kind, to_entry_kind) {
        (_, EntryKind::Nilable(inner)) => check_assignable_extended(cc, from, inner),
        (EntryKind::Anonymous, EntryKind::Anonymous) => check_structural_compatible(cc, from, to),
        (EntryKind::Anonymous, EntryKind::Named(decl)) => {
            let target = resolve_named_assign_target(cc, decl)
                .ok_or_else(|| format!("expected {to_name}, got anonymous type"))?;
            check_assignable_extended(cc, from, target)
        }
        (EntryKind::Named(decl), EntryKind::Anonymous) => {
            let target = resolve_named_assign_target(cc, decl)
                .ok_or_else(|| format!("expected {to_name}, got {from_name}"))?;
            check_assignable_extended(cc, target, to)
        }
        (EntryKind::Named(from_decl), EntryKind::Named(to_decl)) => {
            let from_unwrapped = resolve_type_alias_target(cc, from_decl);
            let to_unwrapped = resolve_type_alias_target(cc, to_decl);
            match (from_unwrapped, to_unwrapped) {
                (Some(ft), Some(tt)) => check_assignable_extended(cc, ft, tt),
                (Some(ft), None) => check_assignable_extended(cc, ft, to),
                (None, Some(tt)) => check_assignable_extended(cc, from, tt),
                (None, None) => Err(format!("expected {to_name}, got {from_name}")),
            }
        }
        _ => Err(format!("expected {to_name}, got {from_name}")),
    }
}

/// Lightweight tag for dispatching assignability without holding borrows.
enum EntryKind {
    Nilable(TypeId),
    Anonymous,
    Named(DeclId),
    Other,
}

fn resolve_named_assign_target(cc: &mut CheckContext, decl: DeclId) -> Option<TypeId> {
    let entry = {
        let e = cc.registry.decl(decl);
        (e.module, e.name.clone())
    };
    let module = cc.registry.module(entry.0).clone();
    let decl = module
        .get_declaration(&entry.1)
        .or_else(|| cc.lookup_decl(&entry.1))?;
    match decl.kind {
        TypeConstructKind::TypeAlias => decl.type_alias_target,
        TypeConstructKind::Data => Some(
            cc.registry.intern_anonymous(anon_type_construct(
                TypeConstructKind::Data,
                FuncModifiers {
                    view: false,
                    noblock: false,
                },
                decl.fields
                    .iter()
                    .map(|f| TypeField {
                        modifier: None,
                        name: f.name.clone(),
                        ty: f.ty,
                        has_default: f.has_default,
                        node_id: f.node_id,
                    })
                    .collect(),
                decl.ast_field_eval_order.clone(),
            )),
        ),
        _ => None,
    }
}

fn resolve_type_alias_target(cc: &mut CheckContext, decl: DeclId) -> Option<TypeId> {
    let entry = {
        let e = cc.registry.decl(decl);
        (e.module, e.name.clone())
    };
    let module = cc.registry.module(entry.0).clone();
    let decl = module
        .get_declaration(&entry.1)
        .or_else(|| cc.lookup_decl(&entry.1))?;
    if decl.kind == TypeConstructKind::TypeAlias {
        decl.type_alias_target
    } else {
        None
    }
}

/// Look up a field's type and modifier on a structural type (Anonymous or MemberFunc).
/// For MemberFunc, substitutes receiver type args into the field type.
fn structural_field(
    cc: &mut CheckContext,
    ty: TypeId,
    field_name: &str,
) -> Option<(Option<FieldModifier>, TypeId, bool)> {
    enum LookupResult {
        Found(Option<FieldModifier>, TypeId, bool),
        FallbackLocal(Symbol, Symbol),
    }
    let result = match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(a) => a
            .fields
            .iter()
            .find(|f| f.name == field_name)
            .map(|f| LookupResult::Found(f.modifier, f.ty, f.has_default)),
        TypeEntry::MemberFunc { decl, member, .. } => {
            let entry = cc.registry.decl(*decl);
            let module = cc.registry.module(entry.module).clone();
            if let Some(f) = module
                .get_declaration(&entry.name)
                .and_then(|d| d.get_member(member))
                .and_then(|m| m.fields.iter().find(|f| f.name == field_name))
            {
                Some(LookupResult::Found(f.modifier, f.ty, f.has_default))
            } else if module.files.is_empty() {
                Some(LookupResult::FallbackLocal(
                    entry.name.clone(),
                    member.clone(),
                ))
            } else {
                None
            }
        }
        _ => None,
    };
    let (modifier, field_ty, has_default) = match result? {
        LookupResult::Found(m, ty, hd) => (m, ty, hd),
        LookupResult::FallbackLocal(decl_name, member_name) => {
            let f = cc
                .lookup_decl(&decl_name)
                .and_then(|d| d.get_member(&member_name))
                .and_then(|m| m.fields.iter().find(|f| f.name == field_name))?;
            (f.modifier, f.ty, f.has_default)
        }
    };
    // Substitute type args for MemberFunc
    let ta: Option<Vec<_>> = if let TypeEntry::MemberFunc { type_args, .. } =
        cc.registry.type_entry(ty)
        && !type_args.is_empty()
    {
        Some(
            type_args
                .iter()
                .map(|a| (a.name.clone(), a.value))
                .collect(),
        )
    } else {
        None
    };
    let field_ty = if let Some(ta) = ta {
        cc.registry.substitute_type_params(field_ty, &ta)
    } else {
        field_ty
    };
    Some((modifier, field_ty, has_default))
}

fn structural_kind(cc: &mut CheckContext, ty: TypeId) -> Option<TypeConstructKind> {
    let info = match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(a) => Some((a.kind, None)),
        TypeEntry::MemberFunc { decl, member, .. } => {
            let entry = cc.registry.decl(*decl);
            let module = cc.registry.module(entry.module).clone();
            if let Some(m) = module
                .get_declaration(&entry.name)
                .and_then(|d| d.get_member(member))
            {
                Some((m.kind, None))
            } else {
                // Stub module - need local fallback
                Some((
                    TypeConstructKind::Func,
                    Some((entry.name.clone(), member.clone())),
                ))
            }
        }
        _ => None,
    };
    match info {
        Some((kind, None)) => Some(kind),
        Some((_, Some((decl_name, member_name)))) => cc
            .lookup_decl(&decl_name)
            .and_then(|d| d.get_member(&member_name))
            .map(|m| m.kind),
        None => None,
    }
}

/// Get field names for the "to" side of a structural comparison.
fn structural_field_names(
    cc: &mut CheckContext,
    ty: TypeId,
) -> Vec<(Symbol, Option<FieldModifier>, TypeId, bool)> {
    match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(a) => a
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.modifier, f.ty, f.has_default))
            .collect(),
        TypeEntry::MemberFunc { decl, member, .. } => {
            let entry = cc.registry.decl(*decl);
            let decl_name = entry.name.clone();
            let member_name = member.clone();
            let module = cc.registry.module(entry.module).clone();
            module
                .get_declaration(&decl_name)
                .and_then(|d| d.get_member(&member_name))
                .or_else(|| {
                    // Local fallback for in-progress module
                    cc.lookup_decl(&decl_name)
                        .and_then(|d| d.get_member(&member_name))
                })
                .map(|m| {
                    m.fields
                        .iter()
                        .map(|f| (f.name.clone(), f.modifier, f.ty, f.has_default))
                        .collect()
                })
                .unwrap_or_default()
        }
        _ => vec![],
    }
}

fn check_structural_compatible(
    cc: &mut CheckContext,
    from: TypeId,
    to: TypeId,
) -> Result<(), String> {
    let from_kind = structural_kind(cc, from);
    let to_kind = structural_kind(cc, to);
    if from_kind != to_kind {
        return Err("anonymous type kind mismatch".to_string());
    }
    for (name, to_modifier, to_ty, to_has_default) in structural_field_names(cc, to) {
        let Some((from_modifier, from_ty, from_has_default)) = structural_field(cc, from, &name)
        else {
            return Err(format!("missing field '{name}'"));
        };
        if !is_modifier_compatible(from_modifier, to_modifier) {
            return Err(format!(
                "field '{}' modifier mismatch: expected {}, got {}",
                name,
                to_modifier.map(|m| m.as_str()).unwrap_or("<none>"),
                from_modifier.map(|m| m.as_str()).unwrap_or("<none>")
            ));
        }
        if to_has_default && !from_has_default {
            return Err(format!("field '{}' missing required default", name));
        }
        check_assignable_extended(cc, from_ty, to_ty)?;
    }
    Ok(())
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
