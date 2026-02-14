use std::collections::HashMap;

use crate::model::{self, NodeId, Symbol};

use super::CheckContext;
use crate::load::{
    LoadError, TypeField, TypeIntype,
    registry::{DeclId, TypeEntry, TypeId},
};

pub(super) fn resolve_invocation_type_args(
    cc: &mut CheckContext,
    type_args: &[model::TypeArgument],
    decl_id: crate::load::registry::DeclId,
) -> Vec<crate::load::registry::RegistryTypeArg> {
    type_args
        .iter()
        .map(|ta| {
            if !is_valid_type_arg_name(cc, decl_id, &ta.name.name) {
                cc.errors.push(crate::load::LoadError {
                    module: cc.module_path.clone(),
                    message: format!("unknown type argument '{}'", ta.name.name),
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&ta.name.node_id).copied(),
                });
            }
            crate::load::registry::RegistryTypeArg {
                name: ta.name.name.clone(),
                value: crate::load::type_resolve::resolve_ast_type(cc, &ta.value, &[]),
            }
        })
        .collect()
}

/// Check if a type arg name is valid for a given declaration.
/// Handles same-module lookups (where registry has a stub) via cc.files/decl_index.
fn is_valid_type_arg_name(cc: &mut CheckContext, decl_id: DeclId, name: &Symbol) -> bool {
    let entry = cc.registry.decl(decl_id);
    let is_same_module = *cc.registry.module_path(entry.module) == cc.module_path;
    if is_same_module && let Some(&(fi, di)) = cc.decl_index.get(&entry.name) {
        return cc.files[fi].declarations[di]
            .intypes
            .iter()
            .any(|i| &i.name == name);
    }
    let intypes = cc.registry.decl_intypes(decl_id);
    intypes.iter().any(|i| &i.name == name)
}

pub(super) fn to_registry_type_args(
    pairs: Vec<(Symbol, TypeId)>,
) -> Vec<crate::load::registry::RegistryTypeArg> {
    pairs
        .into_iter()
        .map(|(name, value)| crate::load::registry::RegistryTypeArg { name, value })
        .collect()
}

/// Infer type arguments from actual argument types.
///
/// For each intype on the decl, walks the field types looking for TypeParam references
/// and unifies against the actual argument types to solve for type params.
/// Returns inferred (name, type) pairs, or None if any intype is unresolvable.
/// When inference fails, reports errors for each unsolved type param.
pub(super) fn infer_type_args(
    cc: &mut CheckContext,
    intypes: &[TypeIntype],
    fields: &[TypeField],
    arg_types: &[(Symbol, TypeId)],
    inv_node_id: NodeId,
) -> Option<Vec<(Symbol, TypeId)>> {
    if intypes.is_empty() {
        return None;
    }

    let mut solved: HashMap<Symbol, TypeId> = HashMap::new();

    for (arg_name, actual_ty) in arg_types {
        let field = fields.iter().find(|f| &f.name == arg_name);
        if let Some(field) = field {
            unify_type(cc, *actual_ty, field.ty, &mut solved);
        }
    }

    let mut result = Vec::with_capacity(intypes.len());
    let mut failed = false;
    for intype in intypes {
        if let Some(&ty) = solved.get(&intype.name) {
            result.push((intype.name.clone(), ty));
        } else if let Some(default_ty) = intype.default {
            result.push((intype.name.clone(), default_ty));
        } else {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("cannot infer type argument '{}'", intype.name),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
            });
            failed = true;
        }
    }
    if failed { None } else { Some(result) }
}

enum UnifyTypeAction {
    Solved(Symbol),
    NamedArgs {
        expected: Vec<(Symbol, TypeId)>,
        actual: Vec<(Symbol, TypeId)>,
    },
    Nilable {
        inner_expected: TypeId,
        inner_actual: Option<TypeId>,
    },
    FuncFields {
        expected: Vec<(Symbol, TypeId)>,
        actual: Vec<(Symbol, TypeId)>,
    },
    /// Like FuncFields but actual fields need type param substitution first.
    FuncFieldsSubstitute {
        expected: Vec<(Symbol, TypeId)>,
        actual: Vec<(Symbol, TypeId)>,
        type_args: Vec<(Symbol, TypeId)>,
    },
    /// MemberFunc on an in-progress module - need local lookup.
    MemberFuncLocal {
        expected: Vec<(Symbol, TypeId)>,
        decl_name: Symbol,
        member_name: Symbol,
        type_args: Vec<(Symbol, TypeId)>,
    },
    None,
}

/// Recursively unify an actual type against an expected type to solve for type params.
fn unify_type(
    cc: &mut CheckContext,
    actual: TypeId,
    expected: TypeId,
    solved: &mut HashMap<Symbol, TypeId>,
) {
    if actual == cc.registry.error_type {
        return;
    }
    let action = match cc.registry.type_entry(expected) {
        TypeEntry::TypeParam(name) => UnifyTypeAction::Solved(name.clone()),
        TypeEntry::Named {
            decl: expected_decl,
            type_args: expected_args,
        } if !expected_args.is_empty() => {
            let expected_decl = *expected_decl;
            let exp: Vec<_> = expected_args
                .iter()
                .map(|a| (a.name.clone(), a.value))
                .collect();
            if let TypeEntry::Named {
                decl: actual_decl,
                type_args: actual_args,
            } = cc.registry.type_entry(actual)
                && *actual_decl == expected_decl
            {
                let act: Vec<_> = actual_args
                    .iter()
                    .map(|a| (a.name.clone(), a.value))
                    .collect();
                UnifyTypeAction::NamedArgs {
                    expected: exp,
                    actual: act,
                }
            } else {
                UnifyTypeAction::None
            }
        }
        TypeEntry::Nilable(inner_expected) => {
            let inner_expected = *inner_expected;
            let inner_actual = if let TypeEntry::Nilable(inner) = cc.registry.type_entry(actual) {
                Some(*inner)
            } else {
                Option::None
            };
            UnifyTypeAction::Nilable {
                inner_expected,
                inner_actual,
            }
        }
        TypeEntry::Anonymous(expected_construct) if expected_construct.kind.is_func() => {
            let exp: Vec<_> = expected_construct
                .fields
                .iter()
                .map(|f| (f.name.clone(), f.ty))
                .collect();
            match cc.registry.type_entry(actual) {
                TypeEntry::Anonymous(ac) if ac.kind.is_func() => {
                    let act: Vec<_> = ac.fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
                    UnifyTypeAction::FuncFields {
                        expected: exp,
                        actual: act,
                    }
                }
                TypeEntry::MemberFunc {
                    decl,
                    member,
                    type_args,
                } => {
                    let entry = cc.registry.decl(*decl);
                    let decl_name = entry.name.clone();
                    let member_name = member.clone();
                    let module = cc.registry.module(entry.module).clone();
                    let member_decl = module
                        .get_declaration(&decl_name)
                        .and_then(|d| d.get_member(&member_name));
                    if let Some(m) = member_decl {
                        let act: Vec<_> = m.fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
                        if type_args.is_empty() {
                            UnifyTypeAction::FuncFields {
                                expected: exp,
                                actual: act,
                            }
                        } else {
                            let ta: Vec<_> = type_args
                                .iter()
                                .map(|a| (a.name.clone(), a.value))
                                .collect();
                            UnifyTypeAction::FuncFieldsSubstitute {
                                expected: exp,
                                actual: act,
                                type_args: ta,
                            }
                        }
                    } else {
                        // Local fallback for in-progress module
                        let ta: Vec<_> = type_args
                            .iter()
                            .map(|a| (a.name.clone(), a.value))
                            .collect();
                        UnifyTypeAction::MemberFuncLocal {
                            expected: exp,
                            decl_name,
                            member_name,
                            type_args: ta,
                        }
                    }
                }
                _ => UnifyTypeAction::None,
            }
        }
        _ => UnifyTypeAction::None,
    };
    match action {
        UnifyTypeAction::Solved(name) => {
            solved.entry(name).or_insert(actual);
        }
        UnifyTypeAction::NamedArgs { expected, actual }
        | UnifyTypeAction::FuncFields { expected, actual } => {
            for (exp_name, exp_ty) in &expected {
                if let Some((_, act_ty)) = actual.iter().find(|(n, _)| n == exp_name) {
                    unify_type(cc, *act_ty, *exp_ty, solved);
                }
            }
        }
        UnifyTypeAction::Nilable {
            inner_expected,
            inner_actual,
        } => {
            let act = inner_actual.unwrap_or(actual);
            unify_type(cc, act, inner_expected, solved);
        }
        UnifyTypeAction::FuncFieldsSubstitute {
            expected,
            actual,
            type_args,
        } => {
            for (exp_name, exp_ty) in &expected {
                if let Some((_, act_ty)) = actual.iter().find(|(n, _)| n == exp_name) {
                    let substituted = cc.registry.substitute_type_params(*act_ty, &type_args);
                    unify_type(cc, substituted, *exp_ty, solved);
                }
            }
        }
        UnifyTypeAction::MemberFuncLocal {
            expected,
            decl_name,
            member_name,
            type_args,
        } => {
            let member_fields: Vec<(Symbol, TypeId)> = cc
                .lookup_decl(&decl_name)
                .and_then(|d| d.get_member(&member_name))
                .map(|m| m.fields.iter().map(|f| (f.name.clone(), f.ty)).collect())
                .unwrap_or_default();
            for (exp_name, exp_ty) in &expected {
                if let Some((_, field_ty)) = member_fields.iter().find(|(name, _)| name == exp_name)
                {
                    let act_ty = if type_args.is_empty() {
                        *field_ty
                    } else {
                        cc.registry.substitute_type_params(*field_ty, &type_args)
                    };
                    unify_type(cc, act_ty, *exp_ty, solved);
                }
            }
        }
        UnifyTypeAction::None => {}
    }
}

/// Intern a named type for a constructor invocation, incorporating explicit type args.
pub(super) fn intern_constructor_type(
    cc: &mut CheckContext,
    decl_id: crate::load::registry::DeclId,
    inv_type_args: &[model::TypeArgument],
) -> TypeId {
    if inv_type_args.is_empty() {
        cc.registry.intern_named(decl_id)
    } else {
        let resolved = resolve_invocation_type_args(cc, inv_type_args, decl_id);
        cc.registry.intern_named_with_args(decl_id, resolved)
    }
}
