use std::collections::HashMap;

use crate::event::{OwnedFields, OwnedValue};
use crate::model::{self, Symbol};

use super::registry::{DeclId, Registry, RegistryTypeArg, TypeId};
use super::type_check::CheckContext;
use super::{
    Annotation, FieldModifier, FuncModifiers, LoadError, Module, TypeConstruct, TypeConstructKind,
    TypeField, TypeIntype, TypedFile, Visibility,
};

/// Shared context for type resolution — carries registry and error collection,
/// replacing the closure-based `LoadContext` pattern with direct access.
pub struct TypeContext<'a> {
    pub registry: &'a mut Registry,
    pub errors: &'a mut Vec<LoadError>,
    pub strict_violations: &'a mut Vec<LoadError>,
}

/// Resolve all declaration signatures (field types, intypes) in a module,
/// then type-check all function bodies.
pub fn resolve_module_types(
    ctx: &mut TypeContext,
    module: &mut Module,
    populate_non_required_types: bool,
    full_path: &Symbol,
) {
    // Pass 1: resolve all field types across all files.
    for file_idx in 0..module.files.len() {
        let num_decls = module.files[file_idx].declarations.len();
        for decl_idx in 0..num_decls {
            resolve_declaration(
                ctx,
                decl_idx,
                file_idx,
                &mut module.files,
                &module.decl_index,
                full_path,
            );
        }
    }

    // Compute non_serializable for each declaration now that field types are
    // resolved. func/entity/extern/native are always non-serializable. Data is
    // non-serializable if any field type is non-serializable (transitive).
    // Also pre-populate the registry's serializable cache so that
    // is_type_serializable works during body type-checking (when the registry
    // only has a stub module without files).
    for file in &mut module.files {
        for decl in &mut file.declarations {
            let non_ser = match decl.kind {
                super::TypeConstructKind::Func
                | super::TypeConstructKind::Extern
                | super::TypeConstructKind::Native
                | super::TypeConstructKind::Entity => true,
                super::TypeConstructKind::Data | super::TypeConstructKind::TypeAlias => decl
                    .fields
                    .iter()
                    .any(|f| !ctx.registry.is_type_serializable(f.ty)),
            };
            decl.non_serializable = non_ser;
            if let Some(decl_id) = decl.decl_id {
                ctx.registry.cache_serializable(decl_id, !non_ser);
            }
        }
    }

    // Pass 2a: resolve annotations.
    for file_idx in 0..module.files.len() {
        let file_anns: Vec<model::Annotation> =
            module.files[file_idx].source_file.annotations.clone();
        let resolved_file_anns: Vec<_> = file_anns
            .iter()
            .filter_map(|ann| {
                let (decl, fields) = eval_const_invocation(
                    ctx,
                    &ann.invocation,
                    &module.files[file_idx],
                    full_path,
                )?;
                Some(Annotation { decl, fields })
            })
            .collect();
        module.files[file_idx].annotations = resolved_file_anns;

        for decl_idx in 0..module.files[file_idx].declarations.len() {
            let decl_name = module.files[file_idx].declarations[decl_idx]
                .name
                .as_deref();
            let anns: Vec<model::Annotation> = module.files[file_idx]
                .source_file
                .constructs
                .iter()
                .find(|c| decl_name == Some(c.name.name.as_str()))
                .map(|c| c.annotations.clone())
                .unwrap_or_default();
            let resolved: Vec<_> = anns
                .iter()
                .filter_map(|ann| {
                    let (decl, fields) = eval_const_invocation(
                        ctx,
                        &ann.invocation,
                        &module.files[file_idx],
                        full_path,
                    )?;
                    Some(Annotation { decl, fields })
                })
                .collect();
            module.files[file_idx].declarations[decl_idx].annotations = resolved;
        }
    }

    // Pass 2b: type-check function bodies.
    for file_idx in 0..module.files.len() {
        let node_types = {
            let file = &module.files[file_idx];
            let mut cc = CheckContext::new(
                ctx,
                &module.files,
                &module.decl_index,
                full_path.clone(),
                file_idx,
                &file.source_file.source_map,
                populate_non_required_types,
            );
            let valid_constructs: Vec<&model::Construct> = file
                .source_file
                .constructs
                .iter()
                .filter(|c| !matches!(c.kind, model::ConstructKind::Invalid(_)))
                .collect();
            for (decl, construct) in file.declarations.iter().zip(valid_constructs.iter()) {
                if let model::ConstructKind::Func(func) = &construct.kind {
                    super::type_check::check_func_body(&mut cc, func, decl);
                }
                if let model::ConstructKind::Data(data) = &construct.kind {
                    super::type_check::check_member_func_bodies(&mut cc, &data.funcs, decl);
                }
                if let model::ConstructKind::Entity(entity) = &construct.kind {
                    super::type_check::check_member_func_bodies(&mut cc, &entity.funcs, decl);
                }
                if let model::ConstructKind::Extern(_) = &construct.kind {
                    super::type_check::check_extern_serializability(&mut cc, decl);
                }
                super::type_check::check_type_arg_constraints(&mut cc, decl);
            }
            cc.node_types
        };
        module.files[file_idx]
            .type_table
            .node_types
            .extend(node_types);
    }
}

/// Get the nth valid (non-Invalid) construct from a source file.
fn valid_construct(source_file: &model::SourceFile, idx: usize) -> &model::Construct {
    source_file
        .constructs
        .iter()
        .filter(|c| !matches!(c.kind, model::ConstructKind::Invalid(_)))
        .nth(idx)
        .unwrap()
}

/// Extract the AST fields and member construct fields from a construct.
fn construct_fields(
    construct: &model::Construct,
) -> Option<(&[model::Field], Vec<&[model::Field]>)> {
    let (fields, member_constructs): (&[model::Field], &[model::Construct]) = match &construct.kind
    {
        model::ConstructKind::TypeAlias(_) => return Some((&[], vec![])),
        model::ConstructKind::Data(d) => (&d.fields, &d.funcs),
        model::ConstructKind::Entity(e) => (&e.fields, &e.funcs),
        model::ConstructKind::Func(f) => (&f.fields, &[]),
        model::ConstructKind::Extern(e) => (&e.fields, &[]),
        model::ConstructKind::Native(n) => (&n.fields, &[]),
        model::ConstructKind::Invalid(_) => return None,
    };
    let member_fields = member_constructs
        .iter()
        .filter_map(|c| match &c.kind {
            model::ConstructKind::Func(f) => Some(f.fields.as_slice()),
            model::ConstructKind::Native(n) => Some(n.fields.as_slice()),
            _ => None,
        })
        .collect();
    Some((fields, member_fields))
}

/// Resolve AST fields to owned TypeField/TypeIntype vecs.
/// `type_params` contains names of `intype` fields on the enclosing construct,
/// so that references like `t` in `in value: t` resolve to `Type::TypeParam("t")`.
/// `eval_order` determines the order in which var fields are resolved, so that
/// sibling references are in scope when their dependents are checked.
fn resolve_field_list<'a>(
    cc: &mut CheckContext<'a>,
    fields: &'a [model::Field],
    type_params: &[Symbol],
    eval_order: &[usize],
) -> (Vec<TypeField>, Vec<TypeIntype>) {
    cc.push_block_scope();

    let mut resolved_intypes = Vec::new();
    for field in fields {
        if let model::FieldKind::Intype(intype) = &field.kind {
            resolved_intypes.push(resolve_intype(cc, intype));
        }
    }

    let var_indices: Vec<usize> = fields
        .iter()
        .enumerate()
        .filter_map(|(i, f)| matches!(f.kind, model::FieldKind::Var(_)).then_some(i))
        .collect();

    let mut resolved_by_ast_idx: Vec<Option<TypeField>> = vec![None; fields.len()];

    for &ast_idx in eval_order {
        if let model::FieldKind::Var(var) = &fields[ast_idx].kind {
            let resolved = resolve_var_field(cc, var, type_params);
            cc.declare_var(var.name.name.clone(), resolved.ty);
            resolved_by_ast_idx[ast_idx] = Some(resolved);
        }
    }

    for &ast_idx in &var_indices {
        if resolved_by_ast_idx[ast_idx].is_none()
            && let model::FieldKind::Var(var) = &fields[ast_idx].kind
        {
            let resolved = resolve_var_field(cc, var, type_params);
            cc.declare_var(var.name.name.clone(), resolved.ty);
            resolved_by_ast_idx[ast_idx] = Some(resolved);
        }
    }

    // Collect in source order.
    let resolved_fields: Vec<TypeField> = var_indices
        .into_iter()
        .filter_map(|i| resolved_by_ast_idx[i].take())
        .collect();

    cc.pop_scope();
    (resolved_fields, resolved_intypes)
}

/// Resolve a single declaration's field types.
fn resolve_declaration(
    ctx: &mut TypeContext,
    decl_idx: usize,
    file_idx: usize,
    files: &mut [TypedFile],
    decl_index: &HashMap<Symbol, (usize, usize)>,
    module_path: &Symbol,
) {
    if !files[file_idx].declarations[decl_idx].fields.is_empty()
        || !files[file_idx].declarations[decl_idx].intypes.is_empty()
    {
        return; // already resolved
    }

    // Resolve all fields in an immutable borrow block, then write results.
    let source_map = &files[file_idx].source_file.source_map;
    let construct = valid_construct(&files[file_idx].source_file, decl_idx);

    let (fields, intypes, type_alias_target, eval_order, member_results) = match &construct.kind {
        model::ConstructKind::TypeAlias(ta) => {
            let type_params: Vec<Symbol> =
                ta.fields.iter().map(|it| it.name.name.clone()).collect();
            let mut cc = CheckContext::new(
                ctx,
                files,
                decl_index,
                module_path.clone(),
                file_idx,
                source_map,
                false,
            );
            let intypes = ta
                .fields
                .iter()
                .map(|it| resolve_intype(&mut cc, it))
                .collect();
            let target = resolve_ast_type(&mut cc, &ta.target, &type_params);
            (Vec::new(), intypes, Some(target), vec![], vec![])
        }
        _ => {
            let (ast_fields, member_field_lists) = construct_fields(construct).unwrap();

            let type_params: Vec<Symbol> = ast_fields
                .iter()
                .filter_map(|f| match &f.kind {
                    model::FieldKind::Intype(it) => Some(it.name.name.clone()),
                    _ => None,
                })
                .collect();

            let unresolved_members: Vec<usize> = (0..member_field_lists.len())
                .filter(|i| {
                    files[file_idx].declarations[decl_idx].members[*i]
                        .fields
                        .is_empty()
                        && files[file_idx].declarations[decl_idx].members[*i]
                            .intypes
                            .is_empty()
                })
                .collect();

            // CheckContext borrows files immutably — scope it to field resolution only.
            let mut cc = CheckContext::new(
                ctx,
                files,
                decl_index,
                module_path.clone(),
                file_idx,
                source_map,
                false,
            );

            let eval_order = super::field_deps::compute_ast_field_eval_order(&mut cc, ast_fields);
            let member_orders: Vec<_> = unresolved_members
                .iter()
                .map(|&i| {
                    super::field_deps::compute_ast_field_eval_order(&mut cc, member_field_lists[i])
                })
                .collect();

            let resolved = resolve_field_list(&mut cc, ast_fields, &type_params, &eval_order);

            // Push parent fields into scope so member funcs can see them.
            cc.push_block_scope();
            for field in &resolved.0 {
                cc.declare_var(field.name.clone(), field.ty);
            }

            let members: Vec<_> = unresolved_members
                .into_iter()
                .zip(member_orders)
                .map(|(i, order)| {
                    let resolved =
                        resolve_field_list(&mut cc, member_field_lists[i], &type_params, &order);
                    (i, resolved.0, resolved.1, order)
                })
                .collect();

            cc.pop_scope();
            (resolved.0, resolved.1, None, eval_order, members)
        }
    };

    // Write results.
    let decl = &mut files[file_idx].declarations[decl_idx];
    decl.fields = fields;
    decl.intypes = intypes;
    decl.type_alias_target = type_alias_target;
    decl.ast_field_eval_order = eval_order;
    for (member_idx, fields, intypes, order) in member_results {
        decl.members[member_idx].fields = fields;
        decl.members[member_idx].intypes = intypes;
        decl.members[member_idx].ast_field_eval_order = order;
        decl.members[member_idx].rebuild_metadata();
    }
    decl.rebuild_metadata();
}

fn resolve_intype(cc: &mut CheckContext, intype: &model::FieldIntype) -> TypeIntype {
    let constraint = intype
        .constraint
        .as_ref()
        .map(|t| resolve_ast_type(cc, t, &[]));
    let default = intype
        .default
        .as_ref()
        .map(|t| resolve_ast_type(cc, t, &[]));
    TypeIntype {
        name: intype.name.name.clone(),
        constraint,
        default,
        node_id: intype.node_id,
    }
}

fn resolve_var_field<'a>(
    cc: &mut CheckContext<'a>,
    var: &'a model::FieldVar,
    type_params: &[Symbol],
) -> TypeField {
    let modifier = match var.modifier {
        model::FieldVarModifier::In => FieldModifier::In,
        model::FieldVarModifier::Out => FieldModifier::Out,
        model::FieldVarModifier::OutEarly => FieldModifier::OutEarly,
        model::FieldVarModifier::Inout => FieldModifier::Inout,
        model::FieldVarModifier::Value => FieldModifier::Value,
        model::FieldVarModifier::Implicit => FieldModifier::Implicit,
    };

    let resolved_ty = var
        .ty
        .as_ref()
        .map(|ast_type| resolve_ast_type(cc, ast_type, type_params));

    // Infer type from default expression if present
    let prev = cc.expected_type.take();
    cc.expected_type = resolved_ty;
    let inferred = var
        .default
        .as_ref()
        .map(|default_expr| super::type_check::check_expr(cc, default_expr));
    cc.expected_type = prev;

    let ty = if let Some(resolved) = resolved_ty {
        super::type_check::check_redundant_annotation(
            cc,
            &var.name.name,
            resolved,
            inferred,
            var.ty
                .as_ref()
                .map(|t| t.node_id())
                .unwrap_or(crate::model::NodeId(0)),
        );
        resolved
    } else if let Some(inferred) = inferred {
        if let Some(default_expr) = &var.default
            && let Some(msg) = super::type_check::uninferrable_literal_message(default_expr)
        {
            cc.add_type_error(
                msg.to_string(),
                cc.source_map
                    .node_ranges
                    .get(&default_expr.node_id())
                    .copied(),
            );
        }
        inferred
    } else {
        cc.add_type_error(
            format!(
                "Field '{}' has no type annotation and no default",
                var.name.name
            ),
            cc.source_map.node_ranges.get(&var.node_id).copied(),
        )
    };

    TypeField {
        name: var.name.name.clone(),
        modifier: Some(modifier),
        ty,
        has_default: var.default.is_some(),
        node_id: var.node_id,
    }
}

/// Convert an AST type to a resolved type.
pub(super) fn resolve_ast_type(
    cc: &mut CheckContext,
    ast_type: &model::Type,
    type_params: &[Symbol],
) -> TypeId {
    match ast_type {
        model::Type::Named(named) => resolve_named_type(cc, named, type_params),
        model::Type::Nilable(nilable) => {
            let inner = resolve_ast_type(cc, &nilable.inner, type_params);
            cc.registry.intern_nilable(inner)
        }
        model::Type::EntityRef(entity_ref) => {
            let inner = resolve_ast_type(cc, &entity_ref.inner, type_params);
            cc.registry.intern_entity_ref(inner)
        }
        model::Type::Anon(anon) => {
            let (kind, func_modifiers) = match anon.construct {
                model::TypeAnonConstruct::Data => {
                    (TypeConstructKind::Data, FuncModifiers::default())
                }
                model::TypeAnonConstruct::Func => {
                    (TypeConstructKind::Func, FuncModifiers::default())
                }
                model::TypeAnonConstruct::FuncView => (
                    TypeConstructKind::Func,
                    FuncModifiers {
                        view: true,
                        noblock: false,
                    },
                ),
                model::TypeAnonConstruct::FuncNoblock => (
                    TypeConstructKind::Func,
                    FuncModifiers {
                        view: false,
                        noblock: true,
                    },
                ),
                model::TypeAnonConstruct::Entity => {
                    (TypeConstructKind::Entity, FuncModifiers::default())
                }
            };
            let fields = anon
                .fields
                .iter()
                .map(|f| TypeField {
                    modifier: f.modifier.map(|m| match m {
                        model::TypeAnonFieldModifier::In => FieldModifier::In,
                        model::TypeAnonFieldModifier::Inout => FieldModifier::Inout,
                        model::TypeAnonFieldModifier::Out => FieldModifier::Out,
                        model::TypeAnonFieldModifier::OutEarly => FieldModifier::OutEarly,
                        model::TypeAnonFieldModifier::Implicit => FieldModifier::Implicit,
                    }),
                    name: f.name.as_ref().map(|n| n.name.clone()).unwrap_or_default(),
                    ty: resolve_ast_type(cc, &f.ty, type_params),
                    has_default: f.has_default,
                    node_id: f.node_id,
                })
                .collect();
            cc.registry.intern_anonymous(TypeConstruct {
                name: None,
                kind,
                visibility: Visibility::Private,
                annotations: vec![],
                intypes: vec![],
                fields,
                out_early_name: None,
                out_field_names: vec![],
                func_modifiers,
                type_alias_target: None,
                members: vec![],
                member_index: HashMap::new(),
                is_builtin: false,
                non_serializable: false,
                decl_id: None,
                node_id: crate::model::NodeId(0),
                ast_field_eval_order: vec![],
            })
        }
        model::Type::Introspection(intro) => cc.add_type_error(
            "type introspection (@type, @typein, @typeout) is not yet supported".to_string(),
            cc.source_map.node_ranges.get(&intro.node_id).copied(),
        ),
        model::Type::Invalid(_) => cc.registry.error_type,
    }
}

/// Resolve a named type path to a fully qualified type.
fn resolve_named_type(
    cc: &mut CheckContext,
    named: &model::TypeNamed,
    type_params: &[Symbol],
) -> TypeId {
    if let Some(module_ident) = &named.module {
        let type_name = match named.path.first() {
            Some(ident) => &ident.name,
            None => {
                return cc.add_type_error(
                    "empty type path".to_string(),
                    cc.source_map.node_ranges.get(&named.node_id).copied(),
                );
            }
        };
        let alias = &module_ident.name;
        if let Some(import_path) = cc.files[cc.file_idx].import_map.get(alias)
            && let Some(decl_id) = resolve_in_module(cc.registry, type_name, import_path)
        {
            let type_args = resolve_type_args(cc, &named.type_args, type_params);
            return cc.registry.intern_named_with_args(decl_id, type_args);
        }
        return cc.add_type_error(
            format!("Unknown module '{}'", alias),
            cc.source_map.node_ranges.get(&named.node_id).copied(),
        );
    }

    if named.path.len() == 1 {
        let name = &named.path[0].name;
        // Type params shadow everything — check current construct's type_params first
        // (inner shadows outer), then scope (parent constructs), then builtins/imports
        if type_params.iter().any(|tp| tp == name) {
            return cc.registry.intern_type_param(name);
        }
        if let Some(ty) = cc.resolve_type_in_scope(name) {
            return ty;
        }
        match name.as_str() {
            "any" => return cc.registry.any_type,
            "anylocal" => return cc.registry.anylocal_type,
            _ => {}
        }
        if let Some(decl_id) = resolve_name(cc, name) {
            let type_args = resolve_type_args(cc, &named.type_args, type_params);
            return cc.registry.intern_named_with_args(decl_id, type_args);
        }
        return cc.add_type_error(
            format!("cannot resolve type '{}'", name),
            cc.source_map.node_ranges.get(&named.node_id).copied(),
        );
    }

    cc.add_type_error(
        format!(
            "Dot-separated type path '{}' not supported, use '::' for module access",
            named
                .path
                .iter()
                .map(|i| i.name.as_str())
                .collect::<Vec<_>>()
                .join(".")
        ),
        cc.source_map.node_ranges.get(&named.node_id).copied(),
    )
}

fn resolve_type_args(
    cc: &mut CheckContext,
    type_args: &[model::TypeArgument],
    type_params: &[Symbol],
) -> Vec<RegistryTypeArg> {
    type_args
        .iter()
        .map(|ta| RegistryTypeArg {
            name: ta.name.name.clone(),
            value: resolve_ast_type(cc, &ta.value, type_params),
        })
        .collect()
}

/// Resolve a single-segment name by walking the scope chain:
/// module declarations -> file imports -> implicit imports.
pub(super) fn resolve_name(cc: &mut CheckContext, name: &Symbol) -> Option<DeclId> {
    if cc.decl_index.contains_key(name) {
        let module_idx = cc.registry.module_index(&cc.module_path)?;
        return cc.registry.decl_id_in_module(module_idx, name);
    }
    for import_path in cc.files[cc.file_idx].import_map.values() {
        if let Some(decl_id) = resolve_in_module(cc.registry, name, import_path) {
            return Some(decl_id);
        }
    }
    None
}

pub(super) fn resolve_in_module(
    registry: &Registry,
    name: &Symbol,
    path: &Symbol,
) -> Option<DeclId> {
    let module_idx = registry.module_index(path)?;
    registry.decl_id_in_module(module_idx, name)
}

/// Evaluate an invocation as a compile-time data constructor.
/// Returns the resolved DeclId and field map.
fn eval_const_invocation(
    ctx: &mut TypeContext,
    inv: &model::ExprInvocation,
    file: &TypedFile,
    full_path: &Symbol,
) -> Option<(DeclId, OwnedFields)> {
    let decl_id = match &*inv.expr {
        model::Expr::Ident(ident) => file
            .import_map
            .values()
            .find_map(|import_path| resolve_in_module(ctx.registry, &ident.name, import_path)),
        model::Expr::ModuleAccess(ma) => {
            let import_path = file.import_map.get(&ma.module.name)?;
            resolve_in_module(ctx.registry, &ma.name.name, import_path)
        }
        _ => None,
    };
    let Some(decl_id) = decl_id else {
        let source_map = &file.source_file.source_map;
        let name = match &*inv.expr {
            model::Expr::Ident(ident) => ident.name.as_str(),
            model::Expr::ModuleAccess(ma) => ma.name.name.as_str(),
            _ => "<expression>",
        };
        ctx.errors.push(LoadError {
            module: full_path.clone(),
            message: format!("cannot resolve type '{name}'"),
            file_idx: 0,
            range: source_map.node_ranges.get(&inv.expr.node_id()).copied(),
        });
        return None;
    };
    let mut fields = OwnedFields::new();
    for arg in &inv.args {
        fields.insert(
            arg.name.name.to_string(),
            eval_const_expr(ctx, &arg.value, file, full_path)?,
        );
    }
    Some((decl_id, fields))
}

/// Evaluate a constant expression at load time.
/// Reports load errors for non-constant expressions and returns `None`.
fn eval_const_expr(
    ctx: &mut TypeContext,
    expr: &model::Expr,
    file: &TypedFile,
    full_path: &Symbol,
) -> Option<OwnedValue> {
    let source_map = &file.source_file.source_map;
    match expr {
        model::Expr::Literal(lit) => match &lit.kind {
            model::ExprLiteralKind::Int(n) => {
                Some(OwnedValue::Int(model::ExprLiteralKind::parse_int(n).ok()?))
            }
            model::ExprLiteralKind::Float(f) => Some(OwnedValue::Float(
                model::ExprLiteralKind::parse_float(f).ok()?,
            )),
            model::ExprLiteralKind::Str(s) => Some(OwnedValue::Str(s.clone())),
            model::ExprLiteralKind::Bool(b) => Some(OwnedValue::Bool(*b)),
            model::ExprLiteralKind::Nil => Some(OwnedValue::Nil),
            model::ExprLiteralKind::Array(items) => {
                let vals: Option<Vec<_>> = items
                    .iter()
                    .map(|e| eval_const_expr(ctx, e, file, full_path))
                    .collect();
                Some(OwnedValue::List(vals?))
            }
            model::ExprLiteralKind::Map(entries) => {
                let mut map = OwnedFields::new();
                for entry in entries {
                    let key = eval_const_expr(ctx, &entry.key, file, full_path)?;
                    let val = eval_const_expr(ctx, &entry.value, file, full_path)?;
                    match key {
                        OwnedValue::Str(s) => {
                            map.insert(s, val);
                        }
                        _ => {
                            ctx.errors.push(LoadError {
                                module: full_path.clone(),
                                message: "map key must be a string constant".to_string(),
                                file_idx: 0,
                                range: source_map.node_ranges.get(&entry.key.node_id()).copied(),
                            });
                            return None;
                        }
                    }
                }
                Some(OwnedValue::Data(map))
            }
        },
        model::Expr::Invocation(inv) => {
            let (_, fields) = eval_const_invocation(ctx, inv, file, full_path)?;
            Some(OwnedValue::Data(fields))
        }
        model::Expr::Unary(u) if u.op == model::ExprUnaryOp::Negate => {
            match eval_const_expr(ctx, &u.expr, file, full_path)? {
                OwnedValue::Int(n) => Some(OwnedValue::Int(-n)),
                OwnedValue::Float(f) => Some(OwnedValue::Float(-f)),
                _ => {
                    ctx.errors.push(LoadError {
                        module: full_path.clone(),
                        message: "negation requires a numeric constant".to_string(),
                        file_idx: 0,
                        range: source_map.node_ranges.get(&u.node_id).copied(),
                    });
                    None
                }
            }
        }
        _ => {
            ctx.errors.push(LoadError {
                module: full_path.clone(),
                message: "expression is not a compile-time constant".to_string(),
                file_idx: 0,
                range: source_map.node_ranges.get(&expr.node_id()).copied(),
            });
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::load::*;
    use crate::parse::parse_source_file;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn make_module(source: &str) -> (Module, LoadContext, Symbol) {
        let result = parse_source_file(source.to_string(), PathBuf::from("test.dl"), false);
        assert!(
            result.parse_errors.is_empty(),
            "parse errors: {:?}",
            result.parse_errors
        );

        let mut root_settings = std::collections::HashMap::new();
        root_settings.insert(
            "test".to_string(),
            RootSettings {
                stdlib_prelude: true,
            },
        );
        let ctx = LoadContext::new(root_settings);
        let path: Symbol = "test.test".into();

        let mut files = Vec::new();
        let mut decl_index = HashMap::new();
        let mut declarations = Vec::new();
        for construct in &result.source_file.constructs {
            if let Some(decl) = crate::load::loader::construct_to_declaration(construct)
                && let Some(name) = decl.name.clone()
            {
                decl_index.insert(name, (0, declarations.len()));
                declarations.push(decl);
            }
        }
        // Build import map with default implicit modules.
        let mut import_map = HashMap::new();
        for module_path in DEFAULT_IMPLICIT_MODULES.iter() {
            let alias = path_alias(module_path);
            import_map.insert(alias, module_path.clone());
        }

        files.push(TypedFile {
            source_file: result.source_file,
            type_table: TypeTable::default(),
            annotations: Vec::new(),
            declarations,
            import_map,
        });

        let module = Module {
            has_errors: false,
            files,
            decl_index,
        };

        // Register module and declarations in registry so resolve_name can return DeclIds
        ctx.insert_module(
            path.clone(),
            std::sync::Arc::new(Module {
                has_errors: false,
                files: Vec::new(),
                decl_index: module.decl_index.clone(),
            }),
        );
        if let Some(module_idx) = ctx.module_index(&path) {
            for decl_name in module.decl_index.keys() {
                ctx.register_decl(module_idx, decl_name);
            }
        }

        (module, ctx, path)
    }

    fn resolve(module: &mut Module, ctx: &LoadContext, path: &Symbol) {
        ctx.with_inner_mut(|registry, errors, strict_violations| {
            let mut type_ctx = TypeContext {
                registry,
                errors,
                strict_violations,
            };
            resolve_module_types(&mut type_ctx, module, false, path);
        });
    }

    #[test]
    fn resolve_explicit_builtin_types() {
        let (mut module, ctx, path) =
            make_module("entity foo {\n    inout x: int\n    out y: str\n    in z: bool\n}\n");
        resolve(&mut module, &ctx, &path);

        let decl = module.get_declaration("foo").unwrap();
        assert_eq!(decl.fields.len(), 3);
        assert_eq!(decl.fields[0].name, "x");
        assert_eq!(decl.fields[0].ty, ctx.int_type());
        assert_eq!(decl.fields[0].modifier, Some(FieldModifier::Inout));
        assert_eq!(decl.fields[1].name, "y");
        assert_eq!(decl.fields[1].ty, ctx.str_type());
        assert_eq!(decl.fields[2].name, "z");
        assert_eq!(decl.fields[2].ty, ctx.bool_type());
    }

    #[test]
    fn resolve_nilable_and_entity_ref() {
        let (mut module, ctx, path) =
            make_module("entity bar {\n    in x: int?\n    in y: &bar\n}\n");
        resolve(&mut module, &ctx, &path);

        let decl = module.get_declaration("bar").unwrap();
        assert_eq!(decl.fields[0].ty, ctx.intern_nilable_type(ctx.int_type()));
        ctx.with_registry(|r| {
            if let crate::load::registry::TypeEntry::EntityRef(inner) =
                r.type_entry(decl.fields[1].ty)
            {
                if let crate::load::registry::TypeEntry::Named { decl, .. } = r.type_entry(*inner) {
                    assert_eq!(r.decl(*decl).name, "bar");
                } else {
                    panic!("expected Named inside EntityRef");
                }
            } else {
                panic!("expected EntityRef");
            }
        });
    }

    #[test]
    fn resolve_named_type_local() {
        let (mut module, ctx, path) = make_module(
            "data point {\n    x: int\n    y: int\n}\nfunc make_point {\n    in p: point\n}\n",
        );
        resolve(&mut module, &ctx, &path);

        let decl = module.get_declaration("make_point").unwrap();
        assert_eq!(decl.fields.len(), 1);
        ctx.with_registry(|r| {
            if let crate::load::registry::TypeEntry::Named { decl: did, .. } =
                r.type_entry(decl.fields[0].ty)
            {
                assert_eq!(r.decl(*did).name, "point");
            } else {
                panic!("expected Named type");
            }
        });
    }

    #[test]
    fn infer_type_from_literal_default() {
        let (mut module, ctx, path) =
            make_module("func foo {\n    out x = 0\n    out y = false\n    out z = \"hi\"\n}\n");
        resolve(&mut module, &ctx, &path);

        let decl = module.get_declaration("foo").unwrap();
        assert_eq!(decl.fields[0].name, "x");
        assert_eq!(decl.fields[0].ty, ctx.int_type());
        assert_eq!(decl.fields[1].name, "y");
        assert_eq!(decl.fields[1].ty, ctx.bool_type());
        assert_eq!(decl.fields[2].name, "z");
        assert_eq!(decl.fields[2].ty, ctx.str_type());
    }

    #[test]
    fn resolve_member_func_fields() {
        let (mut module, ctx, path) = make_module(
            "entity sem {\n    inout permits: int\n\n    out noblock func release {\n        out done: bool\n    }\n}\n",
        );
        resolve(&mut module, &ctx, &path);

        let decl = module.get_declaration("sem").unwrap();
        assert_eq!(decl.fields[0].ty, ctx.int_type());
        assert_eq!(decl.members.len(), 1);
        assert_eq!(decl.members[0].fields[0].name, "done");
        assert_eq!(decl.members[0].fields[0].ty, ctx.bool_type());
    }
}
