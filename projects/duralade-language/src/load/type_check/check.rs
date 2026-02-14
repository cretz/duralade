// TODO: validate that out-field default expressions only contain view-only calls
// TODO: test view-only enforcement errors (loader conformance tests, not parser or runtime)
// TODO: detect circular in-field default references (load error)

use std::collections::HashSet;

use crate::model::{self, NodeId, Symbol};

use super::assignable::{check_assignable, check_assignable_extended};
use super::infer::{infer_type_args, intern_constructor_type, to_registry_type_args};
use super::{CheckContext, CheckScope, anon_type_construct};
use crate::load::{
    FieldModifier, FuncModifiers, LoadError, TypeConstruct, TypeConstructKind, TypeField,
    Visibility,
    registry::{DeclId, TypeEntry, TypeId},
};

fn project_root(module_path: &str) -> &str {
    module_path.split('.').next().unwrap_or(module_path)
}

fn is_cross_project_private(name: &str, from_module: &str, defining_module: &str) -> bool {
    name.starts_with('_') && project_root(from_module) != project_root(defining_module)
}

pub(in crate::load) fn check_expr<'a>(cc: &mut CheckContext<'a>, expr: &'a model::Expr) -> TypeId {
    check_expr_inner(cc, expr)
}

/// Type-check a function body given its resolved declaration.
pub(in crate::load) fn check_func_body<'a>(
    cc: &mut CheckContext<'a>,
    func: &'a model::Func,
    decl: &'a TypeConstruct,
) {
    let out_early = decl
        .fields
        .iter()
        .find(|f| f.modifier == Some(FieldModifier::OutEarly))
        .map(|f| f.name.clone());
    let vars = decl.fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
    let type_params = decl
        .intypes
        .iter()
        .map(|it| (it.name.clone(), cc.registry.intern_type_param(&it.name)))
        .collect();
    cc.set_scope(
        CheckScope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(cc.scope().clone()),
            name: decl.name.clone(),
            out_early,
            vars,
            type_params,
            ..Default::default()
        }
        .into_ref(),
    );
    check_stmts(cc, &func.stmts);
    cc.pop_scope();
}

/// Type-check member function bodies on a data or entity construct.
/// Parent construct fields are pushed into scope so member funcs can reference them.
pub(in crate::load) fn check_member_func_bodies<'a>(
    cc: &mut CheckContext<'a>,
    member_funcs: &'a [model::Construct],
    parent_decl: &'a TypeConstruct,
) {
    for member_construct in member_funcs {
        let model::ConstructKind::Func(func) = &member_construct.kind else {
            continue;
        };
        let Some(member_decl) = parent_decl.get_member(&member_construct.name.name) else {
            continue;
        };
        let decl_name = parent_decl.name.clone().unwrap_or_default();
        let fields = parent_decl
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty))
            .collect();
        let type_params = parent_decl
            .intypes
            .iter()
            .map(|it| (it.name.clone(), cc.registry.intern_type_param(&it.name)))
            .collect();
        cc.set_scope(
            CheckScope {
                kind: crate::scope::ScopeKind::Construct,
                parent: Some(cc.scope().clone()),
                name: Some(decl_name.clone()),
                construct: Some(decl_name),
                vars: fields,
                type_params,
                ..Default::default()
            }
            .into_ref(),
        );
        check_func_body(cc, func, member_decl);
        cc.pop_scope();
    }
}

fn check_stmts<'a>(cc: &mut CheckContext<'a>, stmts: &'a [model::Stmt]) {
    for stmt in stmts {
        check_stmt(cc, stmt);
    }
}

fn check_stmt<'a>(cc: &mut CheckContext<'a>, stmt: &'a model::Stmt) {
    match stmt {
        model::Stmt::Expr(expr) => {
            check_expr_inner(cc, expr);
        }
        model::Stmt::Var(v) => {
            // Resolve declared types first so we can set expected_type before
            // checking value expressions (gives array/map literals context).
            let declared_types: Vec<Option<TypeId>> = v
                .vars
                .iter()
                .map(|decl| {
                    decl.ty.as_ref().map(|ast_type| {
                        crate::load::type_resolve::resolve_ast_type(cc, ast_type, &[])
                    })
                })
                .collect();
            let value_types: Vec<TypeId> = v
                .values
                .iter()
                .enumerate()
                .map(|(i, expr)| {
                    let prev = cc.expected_type.take();
                    cc.expected_type = declared_types.get(i).copied().flatten();
                    let ty = check_expr_inner(cc, expr);
                    cc.expected_type = prev;
                    ty
                })
                .collect();
            for (i, decl) in v.vars.iter().enumerate() {
                let declared_ty = if let Some(ty) = declared_types[i] {
                    ty
                } else {
                    if let Some(msg) = v
                        .values
                        .get(i)
                        .and_then(|e| uninferrable_literal_message(e))
                    {
                        cc.add_type_error(
                            msg.to_string(),
                            cc.source_map
                                .node_ranges
                                .get(&v.values[i].node_id())
                                .copied(),
                        );
                    }
                    value_types
                        .get(i)
                        .copied()
                        .unwrap_or(cc.registry.error_type)
                };
                // Non-nilable var without initializer is an error
                if decl.ty.is_some()
                    && v.values.get(i).is_none()
                    && !cc.registry.is_nilable(declared_ty)
                {
                    cc.add_type_error(
                        format!(
                            "non-nilable variable '{}' must have an initializer",
                            decl.name.name
                        ),
                        cc.source_map.node_ranges.get(&decl.node_id).copied(),
                    );
                }
                if let Some(ast_type) = &decl.ty
                    && let Some(value_ty) = value_types.get(i)
                {
                    check_var_assign(cc, declared_ty, *value_ty, v.values[i].node_id());
                    check_redundant_annotation(
                        cc,
                        &decl.name.name,
                        declared_ty,
                        Some(*value_ty),
                        ast_type.node_id(),
                    );
                }
                cc.declare_var(decl.name.name.clone(), declared_ty);
            }
        }
        model::Stmt::Assign(a) => {
            let value_types: Vec<TypeId> = a
                .values
                .iter()
                .map(|expr| check_expr_inner(cc, expr))
                .collect();
            for (i, target) in a.vars.iter().enumerate() {
                let target_ty = check_expr_inner(cc, target);
                if let Some(value_ty) = value_types.get(i) {
                    check_var_assign(cc, target_ty, *value_ty, a.values[i].node_id());
                }
            }
        }
        model::Stmt::If(i) => {
            let bindings = check_if_condition(cc, &i.condition);
            cc.set_scope(
                CheckScope {
                    kind: crate::scope::ScopeKind::Block,
                    parent: Some(cc.scope().clone()),
                    ..Default::default()
                }
                .into_ref(),
            );
            for (name, ty) in bindings {
                cc.declare_var(name, ty);
            }
            check_stmts(cc, &i.then_block.stmts);
            cc.pop_scope();
            for else_if in &i.else_ifs {
                let bindings = check_if_condition(cc, &else_if.condition);
                cc.set_scope(
                    CheckScope {
                        kind: crate::scope::ScopeKind::Block,
                        parent: Some(cc.scope().clone()),
                        ..Default::default()
                    }
                    .into_ref(),
                );
                for (name, ty) in bindings {
                    cc.declare_var(name, ty);
                }
                check_stmts(cc, &else_if.then_block.stmts);
                cc.pop_scope();
            }
            if let Some(else_block) = &i.else_block {
                cc.push_block_scope();
                check_stmts(cc, &else_block.stmts);
                cc.pop_scope();
            }
        }
        model::Stmt::For(f) => {
            if let Some(model::StmtForClause::Condition(cond)) = &f.clause {
                check_expr_inner(cc, &cond.expr);
            }
            if let Some(model::StmtForClause::In(for_in)) = &f.clause {
                // TODO: verify expr resolves to a func accepting a single
                // `in :yielder[t]` (possibly nilable) with no other required params
                check_expr_inner(cc, &for_in.expr);
            }
            cc.set_scope(
                CheckScope {
                    kind: crate::scope::ScopeKind::For,
                    parent: Some(cc.scope().clone()),
                    label: f.label.as_ref().map(|l| l.name.clone()),
                    ..Default::default()
                }
                .into_ref(),
            );
            check_stmts(cc, &f.block.stmts);
            cc.pop_scope();
        }
        model::Stmt::Block(b) => {
            cc.push_block_scope();
            check_stmts(cc, &b.stmts);
            cc.pop_scope();
        }
        model::Stmt::Defer(d) => {
            cc.push_block_scope();
            check_stmts(cc, &d.block.stmts);
            cc.pop_scope();
        }
        model::Stmt::Return(_) => {}
        model::Stmt::ReturnEarly(r) => {
            check_expr_inner(cc, &r.expr);
            if cc.enclosing_out_early_type().is_none() {
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: "cannot use 'return!' without an out! field".to_string(),
                    file_idx: cc.file_idx,
                    range: cc
                        .source_map
                        .node_ranges
                        .get(&r.node_id)
                        .map(|(start, _)| (*start, *start + 7)),
                });
            }
        }
        model::Stmt::Patch(p) => match &p.kind {
            model::StmtPatchKind::Chain(c) => {
                for branch in &c.branches {
                    check_stmts(cc, &branch.stmts);
                }
            }
            model::StmtPatchKind::Complete(_) => {}
        },
        model::Stmt::Implicitly(imp) => {
            for expr in &imp.exprs {
                let ty = check_expr_inner(cc, expr);
                // Runtime requires implicitly expression types (eval.rs uses them
                // to key implicit bindings), so always store them.
                cc.node_types.insert(expr.node_id(), ty);
            }
            if let Some(block) = &imp.block {
                cc.set_scope(
                    CheckScope {
                        kind: crate::scope::ScopeKind::Implicitly,
                        parent: Some(cc.scope().clone()),
                        ..Default::default()
                    }
                    .into_ref(),
                );
                check_stmts(cc, &block.stmts);
                cc.pop_scope();
            }
        }
        model::Stmt::ForBreak(brk) => {
            let label = brk.label.as_ref().map(|l| &l.name);
            if !cc.can_break_or_continue(label) {
                let msg = if let Some(l) = label {
                    format!("'break:{l}' is not inside a for loop with label '{l}'")
                } else {
                    "'break' is not inside a for loop".to_string()
                };
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: msg,
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&brk.node_id).copied(),
                });
            }
        }
        model::Stmt::ForContinue(cont) => {
            let label = cont.label.as_ref().map(|l| &l.name);
            if !cc.can_break_or_continue(label) {
                let msg = if let Some(l) = label {
                    format!("'continue:{l}' is not inside a for loop with label '{l}'")
                } else {
                    "'continue' is not inside a for loop".to_string()
                };
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: msg,
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&cont.node_id).copied(),
                });
            }
        }
        model::Stmt::Invalid(_) => {}
    }
}

fn check_var_assign(
    cc: &mut CheckContext,
    target_ty: TypeId,
    value_ty: TypeId,
    value_node: NodeId,
) {
    let error_type = cc.registry.error_type;
    if value_ty == error_type || target_ty == error_type {
        return;
    }
    if check_assignable_extended(cc, value_ty, target_ty).is_err() {
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!(
                "cannot assign {} to variable of type {}",
                cc.registry.display_type(value_ty),
                cc.registry.display_type(target_ty),
            ),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&value_node).copied(),
        });
    }
}

pub(in crate::load) fn check_redundant_annotation(
    cc: &mut CheckContext,
    name: &str,
    resolved: TypeId,
    inferred: Option<TypeId>,
    type_node: NodeId,
) {
    let error_type = cc.registry.error_type;
    if let Some(inferred) = inferred.filter(|&t| t != error_type)
        && resolved != error_type
        && inferred == resolved
    {
        cc.strict_violations.push(LoadError {
            module: cc.module_path.clone(),
            message: format!(
                "redundant type annotation on '{}': type {} can be inferred",
                name,
                cc.registry.display_type(resolved),
            ),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&type_node).copied(),
        });
    }
}

/// Check an if-condition and return any narrowing bindings to inject into the
/// then-block scope.
fn check_if_condition<'a>(
    cc: &mut CheckContext<'a>,
    cond: &'a model::StmtIfCondition,
) -> Vec<(Symbol, TypeId)> {
    match cond {
        model::StmtIfCondition::Bool(b) => {
            check_expr_inner(cc, &b.expr);
            vec![]
        }
        model::StmtIfCondition::NarrowingNil(n) => {
            let mut bindings = Vec::with_capacity(n.bindings.len());
            for binding in &n.bindings {
                let expr_ty = check_expr_inner(cc, &binding.expr);
                let narrowed = cc.registry.nilable_inner(expr_ty).unwrap_or(expr_ty);
                cc.node_types.insert(binding.node_id, narrowed);
                bindings.push((binding.name.name.clone(), narrowed));
            }
            bindings
        }
        model::StmtIfCondition::NarrowingAs(n) => {
            let mut bindings = Vec::with_capacity(n.bindings.len());
            for binding in &n.bindings {
                check_expr_inner(cc, &binding.expr);
                let resolved = crate::load::type_resolve::resolve_ast_type(cc, &binding.ty, &[]);
                cc.node_types.insert(binding.node_id, resolved);
                bindings.push((binding.name.name.clone(), resolved));
            }
            bindings
        }
    }
}

fn check_narrowing<'a>(cc: &mut CheckContext<'a>, n: &'a model::ExprNarrowing) -> TypeId {
    let inner_ty = check_expr_inner(cc, &n.expr);
    match &n.kind {
        model::ExprNarrowingKind::Nil(nil) => {
            if let Some(else_expr) = &nil.else_expr {
                let else_ty = check_expr_inner(cc, else_expr);
                check_narrowing_else_type(cc, else_ty, else_expr.node_id());
            }
            cc.registry.nilable_inner(inner_ty).unwrap_or(inner_ty)
        }
        model::ExprNarrowingKind::As(as_narrow) => {
            if let Some(else_expr) = &as_narrow.else_expr {
                let else_ty = check_expr_inner(cc, else_expr);
                check_narrowing_else_type(cc, else_ty, else_expr.node_id());
            }
            let resolved = crate::load::type_resolve::resolve_ast_type(cc, &as_narrow.ty, &[]);
            cc.node_types.insert(as_narrow.node_id, resolved);
            resolved
        }
    }
}

/// Check that a narrowing `else!` expression type is assignable to the
/// enclosing function's `out!` field type.
fn check_narrowing_else_type(cc: &mut CheckContext, else_ty: TypeId, else_node: NodeId) {
    let error_type = cc.registry.error_type;
    if else_ty == error_type {
        return;
    }
    if let Some(out_early_ty) = cc.enclosing_out_early_type()
        && check_assignable_extended(cc, else_ty, out_early_ty).is_err()
    {
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!(
                "else! expression type {} is not assignable to out! type {}",
                cc.registry.display_type(else_ty),
                cc.registry.display_type(out_early_ty),
            ),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&else_node).copied(),
        });
    }
}

fn check_expr_inner<'a>(cc: &mut CheckContext<'a>, expr: &'a model::Expr) -> TypeId {
    let ty = match expr {
        model::Expr::Literal(lit) => check_literal(cc, lit),
        model::Expr::Paren(p) => check_expr_inner(cc, &p.expr),
        model::Expr::Unary(u) => check_unary(cc, u),
        model::Expr::Binary(b) => check_binary(cc, b),
        model::Expr::Ident(ident) => cc.lookup_var(&ident.name).unwrap_or(cc.registry.error_type),
        model::Expr::Access(acc) => check_access(cc, acc),
        model::Expr::ModuleAccess(_) => {
            // TODO: resolve module-qualified access
            cc.registry.error_type
        }
        model::Expr::Invocation(inv) => check_invocation(cc, inv),
        model::Expr::Narrowing(n) => check_narrowing(cc, n),
        model::Expr::Outer(_) | model::Expr::Wait(_) | model::Expr::Spawn(_) => {
            cc.registry.error_type
        }
        model::Expr::AnonData(data) => check_anon_data(cc, data),
        model::Expr::AnonFunc(func) => check_anon_func(cc, func),
        model::Expr::Invalid(_) => cc.registry.error_type,
    };
    let runtime_required = matches!(
        expr,
        model::Expr::AnonData(_)
            | model::Expr::AnonFunc(_)
            | model::Expr::Literal(model::ExprLiteral {
                kind: model::ExprLiteralKind::Array(_) | model::ExprLiteralKind::Map(_),
                ..
            })
            | model::Expr::Invocation(_)
    );
    if cc.populate_non_required_types || runtime_required {
        cc.node_types.insert(expr.node_id(), ty);
    }
    ty
}

fn check_anon_data<'a>(cc: &mut CheckContext<'a>, data: &'a model::Data) -> TypeId {
    let fields = data
        .fields
        .iter()
        .filter_map(|field| match &field.kind {
            model::FieldKind::Var(var) => {
                let inferred = var.default.as_ref().map(|d| check_expr_inner(cc, d));
                let ty = if let Some(ast_type) = &var.ty {
                    let resolved = crate::load::type_resolve::resolve_ast_type(cc, ast_type, &[]);
                    cc.node_types.insert(ast_type.node_id(), resolved);
                    check_redundant_annotation(
                        cc,
                        &var.name.name,
                        resolved,
                        inferred,
                        ast_type.node_id(),
                    );
                    resolved
                } else {
                    inferred.unwrap_or(cc.registry.error_type)
                };
                Some(TypeField {
                    modifier: None,
                    name: var.name.name.clone(),
                    ty,
                    has_default: var.default.is_some(),
                    node_id: var.node_id,
                })
            }
            model::FieldKind::Invalid(_) | model::FieldKind::Intype(_) => None,
        })
        .collect();

    let eval_order = crate::load::field_deps::compute_ast_field_eval_order(cc, &data.fields);

    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Data,
        FuncModifiers {
            view: false,
            noblock: false,
        },
        fields,
        eval_order,
    ))
}

fn check_anon_func<'a>(cc: &mut CheckContext<'a>, func: &'a model::Func) -> TypeId {
    let func_modifiers = FuncModifiers {
        view: func.view,
        noblock: func.noblock,
    };

    let fields: Vec<_> = func
        .fields
        .iter()
        .filter_map(|field| match &field.kind {
            model::FieldKind::Var(var) => {
                let inferred = var.default.as_ref().map(|d| check_expr_inner(cc, d));
                let ty = if let Some(ast_type) = &var.ty {
                    let resolved = crate::load::type_resolve::resolve_ast_type(cc, ast_type, &[]);
                    cc.node_types.insert(ast_type.node_id(), resolved);
                    check_redundant_annotation(
                        cc,
                        &var.name.name,
                        resolved,
                        inferred,
                        ast_type.node_id(),
                    );
                    resolved
                } else {
                    inferred.unwrap_or(cc.registry.error_type)
                };
                let modifier = match var.modifier {
                    model::FieldVarModifier::In => Some(FieldModifier::In),
                    model::FieldVarModifier::Out => Some(FieldModifier::Out),
                    model::FieldVarModifier::OutEarly => Some(FieldModifier::OutEarly),
                    model::FieldVarModifier::Inout => Some(FieldModifier::Inout),
                    model::FieldVarModifier::Value => Some(FieldModifier::Value),
                    model::FieldVarModifier::Implicit => Some(FieldModifier::Implicit),
                };
                Some(TypeField {
                    modifier,
                    name: var.name.name.clone(),
                    ty,
                    has_default: var.default.is_some(),
                    node_id: var.node_id,
                })
            }
            model::FieldKind::Invalid(_) | model::FieldKind::Intype(_) => None,
        })
        .collect();

    let out_early = fields
        .iter()
        .find(|f| f.modifier == Some(FieldModifier::OutEarly))
        .map(|f| f.name.clone());
    let vars = fields.iter().map(|f| (f.name.clone(), f.ty)).collect();
    let type_params = func
        .fields
        .iter()
        .filter_map(|f| match &f.kind {
            model::FieldKind::Intype(it) => {
                let name = it.name.name.clone();
                Some((name.clone(), cc.registry.intern_type_param(&name)))
            }
            _ => None,
        })
        .collect();
    cc.set_scope(
        CheckScope {
            kind: crate::scope::ScopeKind::Function,
            parent: Some(cc.scope().clone()),
            out_early,
            vars,
            type_params,
            ..Default::default()
        }
        .into_ref(),
    );
    check_stmts(cc, &func.stmts);
    cc.pop_scope();

    let eval_order = crate::load::field_deps::compute_ast_field_eval_order(cc, &func.fields);

    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Func,
        func_modifiers,
        fields,
        eval_order,
    ))
}

fn check_literal<'a>(cc: &mut CheckContext<'a>, lit: &'a model::ExprLiteral) -> TypeId {
    match &lit.kind {
        model::ExprLiteralKind::Int(_) => cc.registry.int_type,
        model::ExprLiteralKind::Float(_) => cc.registry.float_type,
        model::ExprLiteralKind::Str(_) => cc.registry.str_type,
        model::ExprLiteralKind::Bool(_) => cc.registry.bool_type,
        model::ExprLiteralKind::Nil => cc.registry.intern_nilable(cc.registry.any_type),
        model::ExprLiteralKind::Array(items) => {
            // If the expected type is array[t = X], use X as the element type.
            let expected_elem = cc
                .expected_type
                .and_then(|ty| match cc.registry.type_entry(ty) {
                    TypeEntry::Named { decl, type_args } if *decl == cc.registry.array_decl => {
                        type_args
                            .iter()
                            .find(|a| a.name == Symbol::t())
                            .map(|a| a.value)
                    }
                    _ => None,
                });
            let mut element_type = expected_elem.unwrap_or(cc.registry.any_type);
            for (i, item) in items.iter().enumerate() {
                let item_ty = check_expr_inner(cc, item);
                if item_ty == cc.registry.error_type {
                    continue;
                }
                if expected_elem.is_some() {
                    // Check each element against the declared element type.
                    if check_assignable(cc, item_ty, element_type).is_err() {
                        let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                        cc.errors.push(LoadError {
                            module: cc.module_path.clone(),
                            message: format!(
                                "array element type mismatch: expected {}, got {}",
                                fmt(element_type),
                                fmt(item_ty)
                            ),
                            file_idx: cc.file_idx,
                            range: cc.source_map.node_ranges.get(&item.node_id()).copied(),
                        });
                    }
                } else if i == 0 || element_type == cc.registry.any_type {
                    element_type = item_ty;
                } else if item_ty != element_type
                    && check_assignable(cc, item_ty, element_type).is_err()
                {
                    let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: format!(
                            "array element type mismatch: expected {}, got {}",
                            fmt(element_type),
                            fmt(item_ty)
                        ),
                        file_idx: cc.file_idx,
                        range: cc.source_map.node_ranges.get(&item.node_id()).copied(),
                    });
                }
            }
            let array_decl = cc.registry.array_decl;
            cc.registry.intern_named_with_args(
                array_decl,
                vec![crate::load::registry::RegistryTypeArg {
                    name: Symbol::t(),
                    value: element_type,
                }],
            )
        }
        model::ExprLiteralKind::Map(entries) => {
            // If the expected type is map[k = K, v = V], use K/V for element checks.
            let (expected_key, expected_val) = cc
                .expected_type
                .map(|ty| match cc.registry.type_entry(ty) {
                    TypeEntry::Named { decl, type_args } if *decl == cc.registry.map_decl => {
                        let k = type_args
                            .iter()
                            .find(|a| a.name == Symbol::k())
                            .map(|a| a.value);
                        let v = type_args
                            .iter()
                            .find(|a| a.name == Symbol::v())
                            .map(|a| a.value);
                        (k, v)
                    }
                    _ => (None, None),
                })
                .unwrap_or((None, None));
            let mut key_type = expected_key.unwrap_or(cc.registry.any_type);
            let mut value_type = expected_val.unwrap_or(cc.registry.any_type);
            for (i, entry) in entries.iter().enumerate() {
                let kt = check_expr_inner(cc, &entry.key);
                let vt = check_expr_inner(cc, &entry.value);
                if kt != cc.registry.error_type {
                    if expected_key.is_some() {
                        if check_assignable(cc, kt, key_type).is_err() {
                            let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                            cc.errors.push(LoadError {
                                module: cc.module_path.clone(),
                                message: format!(
                                    "map key type mismatch: expected {}, got {}",
                                    fmt(key_type),
                                    fmt(kt)
                                ),
                                file_idx: cc.file_idx,
                                range: cc.source_map.node_ranges.get(&entry.key.node_id()).copied(),
                            });
                        }
                    } else if i == 0 || key_type == cc.registry.any_type {
                        key_type = kt;
                    } else if kt != key_type && check_assignable(cc, kt, key_type).is_err() {
                        let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                        cc.errors.push(LoadError {
                            module: cc.module_path.clone(),
                            message: format!(
                                "map key type mismatch: expected {}, got {}",
                                fmt(key_type),
                                fmt(kt)
                            ),
                            file_idx: cc.file_idx,
                            range: cc.source_map.node_ranges.get(&entry.key.node_id()).copied(),
                        });
                    }
                }
                if vt != cc.registry.error_type {
                    if expected_val.is_some() {
                        if check_assignable(cc, vt, value_type).is_err() {
                            let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                            cc.errors.push(LoadError {
                                module: cc.module_path.clone(),
                                message: format!(
                                    "map value type mismatch: expected {}, got {}",
                                    fmt(value_type),
                                    fmt(vt)
                                ),
                                file_idx: cc.file_idx,
                                range: cc
                                    .source_map
                                    .node_ranges
                                    .get(&entry.value.node_id())
                                    .copied(),
                            });
                        }
                    } else if i == 0 || value_type == cc.registry.any_type {
                        value_type = vt;
                    } else if vt != value_type && check_assignable(cc, vt, value_type).is_err() {
                        let fmt = |ty: TypeId| -> String { cc.registry.display_type(ty) };
                        cc.errors.push(LoadError {
                            module: cc.module_path.clone(),
                            message: format!(
                                "map value type mismatch: expected {}, got {}",
                                fmt(value_type),
                                fmt(vt)
                            ),
                            file_idx: cc.file_idx,
                            range: cc
                                .source_map
                                .node_ranges
                                .get(&entry.value.node_id())
                                .copied(),
                        });
                    }
                }
            }
            let map_decl = cc.registry.map_decl;
            cc.registry.intern_named_with_args(
                map_decl,
                vec![
                    crate::load::registry::RegistryTypeArg {
                        name: Symbol::k(),
                        value: key_type,
                    },
                    crate::load::registry::RegistryTypeArg {
                        name: Symbol::v(),
                        value: value_type,
                    },
                ],
            )
        }
    }
}

/// Returns an error message if the expression is a literal whose type cannot be
/// inferred without an explicit type annotation (nil, empty array, empty map).
pub fn uninferrable_literal_message(expr: &model::Expr) -> Option<&'static str> {
    if let model::Expr::Literal(lit) = expr {
        match &lit.kind {
            model::ExprLiteralKind::Nil => {
                Some("cannot infer type of 'nil', add a type annotation (e.g. ': int?')")
            }
            model::ExprLiteralKind::Array(items) if items.is_empty() => Some(
                "cannot infer element type of empty array, add a type annotation (e.g. ': array[t = int]')",
            ),
            model::ExprLiteralKind::Map(entries) if entries.is_empty() => Some(
                "cannot infer key/value types of empty map, add a type annotation (e.g. ': map[k = str, v = int]')",
            ),
            _ => None,
        }
    } else {
        None
    }
}

fn ty_is_numeric(cc: &mut CheckContext, ty: TypeId) -> bool {
    ty == cc.registry.int_type || ty == cc.registry.float_type
}

fn check_unary<'a>(cc: &mut CheckContext<'a>, u: &'a model::ExprUnary) -> TypeId {
    let inner = check_expr_inner(cc, &u.expr);
    if inner == cc.registry.error_type {
        return cc.registry.error_type;
    }
    match u.op {
        model::ExprUnaryOp::Negate => {
            if ty_is_numeric(cc, inner) {
                inner
            } else {
                let ty_name = cc.registry.display_type(inner);
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: format!("cannot apply '-' to {ty_name}"),
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&u.node_id).copied(),
                });
                cc.registry.error_type
            }
        }
        model::ExprUnaryOp::Not => {
            if inner == cc.registry.bool_type {
                cc.registry.bool_type
            } else {
                let ty_name = cc.registry.display_type(inner);
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: format!("cannot apply '!' to {ty_name}"),
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&u.node_id).copied(),
                });
                cc.registry.error_type
            }
        }
        model::ExprUnaryOp::EarlyReturn => {
            match cc.enclosing_out_early_type() {
                None => {
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: "cannot use '!' without an out! field".to_string(),
                        file_idx: cc.file_idx,
                        range: cc
                            .source_map
                            .op_positions
                            .get(&u.node_id)
                            .map(|&pos| (pos, pos + 1)),
                    });
                }
                Some(enclosing_type) => {
                    if inner != cc.registry.error_type
                        && let Err(message) = check_out_early_field(cc, inner, enclosing_type)
                    {
                        cc.errors.push(LoadError {
                            module: cc.module_path.clone(),
                            message,
                            file_idx: cc.file_idx,
                            range: cc
                                .source_map
                                .op_positions
                                .get(&u.node_id)
                                .map(|&pos| (pos, pos + 1)),
                        });
                    }
                }
            }
            cc.registry.error_type
        }
    }
}

/// Check whether the inner type of `!` has an out! field assignable to the func's out! type.
/// Returns Ok if valid, Err with an error message if not.
fn check_out_early_field(
    cc: &mut CheckContext,
    ty: TypeId,
    func_out_type: TypeId,
) -> Result<(), String> {
    let err_msg = || "'!' requires a compound type with an out! field".to_string();

    let out_early_ty = match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(anon) => anon
            .fields
            .iter()
            .find(|f| f.modifier == Some(FieldModifier::OutEarly))
            .map(|f| f.ty),
        TypeEntry::Named { decl, .. } => {
            let entry = cc.registry.decl(*decl);
            let module = cc.registry.module(entry.module).clone();
            module.get_declaration(&entry.name).and_then(|decl| {
                decl.fields
                    .iter()
                    .find(|f| f.modifier == Some(FieldModifier::OutEarly))
                    .map(|f| f.ty)
            })
        }
        _ => None,
    };

    // For anonymous data values, allow `!` to use the enclosing out! field name.
    let named_match = if out_early_ty.is_none() {
        cc.enclosing_out_early_name().and_then(|name| {
            if let TypeEntry::Anonymous(anon) = cc.registry.type_entry(ty) {
                anon.fields.iter().find(|f| f.name == name).map(|f| f.ty)
            } else {
                None
            }
        })
    } else {
        None
    };

    let inner_ty = out_early_ty.or(named_match).ok_or_else(err_msg)?;
    check_assignable_extended(cc, inner_ty, func_out_type)
        .map_err(|e| format!("'!' out! type mismatch: {e}"))
}

fn check_access<'a>(cc: &mut CheckContext<'a>, acc: &'a model::ExprAccess) -> TypeId {
    let base_ty = check_expr_inner(cc, &acc.expr);
    let error_ty = cc.registry.error_type;
    if base_ty == error_ty {
        return error_ty;
    }
    match resolve_field_type(cc, base_ty, &acc.ident.name) {
        FieldLookup::Found(ty) => ty,
        FieldLookup::MemberFunc(decl_id, member_name) => {
            cc.registry.intern_member_func(decl_id, member_name, vec![])
        }
        FieldLookup::Private(construct, field) => {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!(
                    "field '{}' on '{}' is not accessible (only out/inout fields are)",
                    field, construct
                ),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&acc.ident.node_id).copied(),
            });
            error_ty
        }
        FieldLookup::ProjectPrivate(construct, field) => {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!(
                    "field '{}' on '{}' is project-private (underscore prefix)",
                    field, construct
                ),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&acc.ident.node_id).copied(),
            });
            error_ty
        }
        _ => error_ty,
    }
}

enum FieldLookup {
    /// Field found and accessible.
    Found(TypeId),
    /// Member func found - carries owning DeclId and member name.
    MemberFunc(crate::load::registry::DeclId, Symbol),
    /// Field exists but is not externally accessible on this entity.
    Private(Symbol, Symbol),
    /// Field is project-private (underscore prefix, cross-project access).
    ProjectPrivate(Symbol, Symbol),
    /// Field not found on the type.
    NotFound,
}

/// Look up a field on a declaration, enforcing entity field visibility.
fn lookup_field_on_decl(d: &TypeConstruct, field_name: &str) -> FieldLookup {
    let Some(field) = d.fields.iter().find(|f| f.name == field_name) else {
        // Check member funcs - accessing a member func as a value.
        if let Some(member) = d.get_member(field_name) {
            if member.visibility == Visibility::Private {
                return FieldLookup::Private(
                    d.name.clone().unwrap_or_else(|| "?".into()),
                    field_name.into(),
                );
            }
            if let Some(owner_decl_id) = d.decl_id {
                return FieldLookup::MemberFunc(owner_decl_id, field_name.into());
            }
        }
        return FieldLookup::NotFound;
    };
    if d.kind == TypeConstructKind::Entity
        && !matches!(
            field.modifier,
            Some(FieldModifier::Out) | Some(FieldModifier::Inout)
        )
    {
        return FieldLookup::Private(
            d.name.clone().unwrap_or_else(|| "?".into()),
            field.name.clone(),
        );
    }
    FieldLookup::Found(field.ty)
}

/// Look up a field by name on a type. Returns Found, Private, or NotFound.
fn resolve_field_type(cc: &mut CheckContext, ty: TypeId, field_name: &str) -> FieldLookup {
    enum Inner {
        Done(FieldLookup),
        Recurse(TypeId),
        Substitute(TypeId, Vec<(Symbol, TypeId)>),
        MemberWithArgs(DeclId, Symbol, Vec<(Symbol, TypeId)>),
        FallbackLocal(Symbol, Vec<(Symbol, TypeId)>),
    }
    let inner = match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(anon) => Inner::Done(
            anon.fields
                .iter()
                .find(|f| f.name == field_name)
                .map(|f| FieldLookup::Found(f.ty))
                .unwrap_or(FieldLookup::NotFound),
        ),
        TypeEntry::Named {
            decl,
            type_args: named_type_args,
        } => {
            let entry = cc.registry.decl(*decl);
            let type_module_path = cc.registry.module_path(entry.module);
            if field_name.starts_with('_')
                && project_root(&cc.module_path) != project_root(type_module_path)
            {
                let construct_name = entry.name.clone();
                Inner::Done(FieldLookup::ProjectPrivate(
                    construct_name,
                    field_name.into(),
                ))
            } else {
                let ta: Vec<(Symbol, TypeId)> = named_type_args
                    .iter()
                    .map(|a| (a.name.clone(), a.value))
                    .collect();
                let module = cc.registry.module(entry.module).clone();
                if let Some(d) = module.get_declaration(&entry.name) {
                    let result = lookup_field_on_decl(d, field_name);
                    if !ta.is_empty() {
                        match result {
                            FieldLookup::Found(field_ty) => Inner::Substitute(field_ty, ta),
                            FieldLookup::MemberFunc(decl_id, member_name) => {
                                Inner::MemberWithArgs(decl_id, member_name, ta)
                            }
                            _ => Inner::Done(result),
                        }
                    } else {
                        Inner::Done(result)
                    }
                } else {
                    Inner::FallbackLocal(entry.name.clone(), ta)
                }
            }
        }
        TypeEntry::Nilable(inner) => Inner::Recurse(*inner),
        _ => Inner::Done(FieldLookup::NotFound),
    };
    match inner {
        Inner::Done(result) => result,
        Inner::Recurse(inner_ty) => resolve_field_type(cc, inner_ty, field_name),
        Inner::Substitute(field_ty, ta) => {
            FieldLookup::Found(cc.registry.substitute_type_params(field_ty, &ta))
        }
        Inner::MemberWithArgs(decl_id, member_name, ta) => {
            let type_args = to_registry_type_args(ta);
            FieldLookup::Found(
                cc.registry
                    .intern_member_func(decl_id, member_name, type_args),
            )
        }
        Inner::FallbackLocal(name, ta) => {
            let result = cc
                .lookup_decl(&name)
                .map(|d| lookup_field_on_decl(d, field_name))
                .unwrap_or(FieldLookup::NotFound);
            match result {
                FieldLookup::Found(field_ty) if !ta.is_empty() => {
                    FieldLookup::Found(cc.registry.substitute_type_params(field_ty, &ta))
                }
                FieldLookup::MemberFunc(decl_id, member_name) if !ta.is_empty() => {
                    let type_args = to_registry_type_args(ta);
                    FieldLookup::Found(cc.registry.intern_member_func(
                        decl_id,
                        member_name,
                        type_args,
                    ))
                }
                _ => result,
            }
        }
    }
}

/// Resolve invocation type args to registry type args.
// TODO: optimize - TypeConstruct clones below are needed to release the immutable borrow on
// cc.files before passing &mut cc to check_invocation_args. Consider caching or splitting
// CheckContext so node_types is independently borrowable.
fn check_invocation<'a>(cc: &mut CheckContext<'a>, inv: &'a model::ExprInvocation) -> TypeId {
    match &*inv.expr {
        model::Expr::Ident(ident) => {
            // Resolve through scope (finds locals, same-module decls, and imports).
            let Some(var_ty) = cc.lookup_var(&ident.name) else {
                return cc.registry.error_type;
            };
            // If it resolved to a named declaration, type-check as a declaration invocation.
            if let TypeEntry::Named { decl: decl_id, .. } = cc.registry.type_entry(var_ty)
                && let Some(decl) = cc.lookup_construct(*decl_id).cloned()
            {
                let decl_id = *decl_id;
                let arg_types = check_invocation_args(cc, &decl.fields, &inv.args, inv.node_id);
                if matches!(
                    decl.kind,
                    TypeConstructKind::Data | TypeConstructKind::Entity
                ) {
                    if inv.type_args.is_empty()
                        && let Some(inferred) = infer_type_args(
                            cc,
                            &decl.intypes,
                            &decl.fields,
                            &arg_types,
                            inv.node_id,
                        )
                    {
                        return cc
                            .registry
                            .intern_named_with_args(decl_id, to_registry_type_args(inferred));
                    }
                    return intern_constructor_type(cc, decl_id, &inv.type_args);
                }
                if inv.type_args.is_empty()
                    && !decl.intypes.is_empty()
                    && let Some(inferred) =
                        infer_type_args(cc, &decl.intypes, &decl.fields, &arg_types, inv.node_id)
                {
                    return decl_return_type_substituted(cc, &decl, &inferred);
                }
                return decl_return_type(cc, &decl);
            }
            // Otherwise it's a scope variable with an anon func type.
            invocation_return_type(cc, var_ty)
        }
        model::Expr::ModuleAccess(ma) => {
            let import_path = cc.files[cc.file_idx]
                .import_map
                .get(&ma.module.name)
                .cloned();
            let resolved = import_path.as_ref().and_then(|import_path| {
                let decl_id = crate::load::type_resolve::resolve_in_module(
                    cc.registry,
                    &ma.name.name,
                    import_path,
                )?;
                let entry = {
                    let e = cc.registry.decl(decl_id);
                    (e.module, e.name.clone())
                };
                let module = cc.registry.module(entry.0).clone();
                let decl = module.get_declaration(&entry.1)?.clone();
                Some((decl_id, decl))
            });
            if let Some((decl_id, decl)) = resolved {
                let target_path = import_path.as_ref().map(|p| p.as_ref()).unwrap_or("");
                if decl.visibility == Visibility::Private {
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: format!(
                            "'{}' in '{}' is not out (private)",
                            ma.name.name, ma.module.name
                        ),
                        file_idx: cc.file_idx,
                        range: cc.source_map.node_ranges.get(&inv.node_id).copied(),
                    });
                    for arg in &inv.args {
                        check_expr_inner(cc, &arg.value);
                    }
                    return cc.registry.error_type;
                }
                if is_cross_project_private(&ma.name.name, &cc.module_path, target_path) {
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: format!(
                            "'{}' in '{}' is project-private (underscore prefix)",
                            ma.name.name, ma.module.name
                        ),
                        file_idx: cc.file_idx,
                        range: cc.source_map.node_ranges.get(&inv.node_id).copied(),
                    });
                    for arg in &inv.args {
                        check_expr_inner(cc, &arg.value);
                    }
                    return cc.registry.error_type;
                }
                let arg_types = check_invocation_args(cc, &decl.fields, &inv.args, inv.node_id);
                if matches!(
                    decl.kind,
                    TypeConstructKind::Data | TypeConstructKind::Entity
                ) {
                    if inv.type_args.is_empty()
                        && let Some(inferred) = infer_type_args(
                            cc,
                            &decl.intypes,
                            &decl.fields,
                            &arg_types,
                            inv.node_id,
                        )
                    {
                        return cc
                            .registry
                            .intern_named_with_args(decl_id, to_registry_type_args(inferred));
                    }
                    return intern_constructor_type(cc, decl_id, &inv.type_args);
                }
                if inv.type_args.is_empty()
                    && !decl.intypes.is_empty()
                    && let Some(inferred) =
                        infer_type_args(cc, &decl.intypes, &decl.fields, &arg_types, inv.node_id)
                {
                    return decl_return_type_substituted(cc, &decl, &inferred);
                }
                return decl_return_type(cc, &decl);
            }
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("'{}' not found in '{}'", ma.name.name, ma.module.name),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&inv.node_id).copied(),
            });
            for arg in &inv.args {
                check_expr_inner(cc, &arg.value);
            }
            cc.registry.error_type
        }
        model::Expr::Access(acc) => {
            let base_ty = check_expr_inner(cc, &acc.expr);
            let error_ty = cc.registry.error_type;
            if base_ty == error_ty {
                for arg in &inv.args {
                    check_expr_inner(cc, &arg.value);
                }
                return error_ty;
            }
            check_member_invocation(
                cc,
                base_ty,
                &acc.ident.name,
                &inv.args,
                &inv.type_args,
                inv.node_id,
            )
        }
        _ => {
            for arg in &inv.args {
                check_expr_inner(cc, &arg.value);
            }
            cc.registry.error_type
        }
    }
}

/// Type-check a member invocation like `receiver.method(args)`.
/// Resolves the member declaration, substitutes type params from the receiver,
/// checks arguments, and returns the substituted return type.
fn check_member_invocation<'a>(
    cc: &mut CheckContext<'a>,
    receiver_ty: TypeId,
    member_name: &str,
    args: &'a [model::ExprInvocationArg],
    member_type_args: &[model::TypeArgument],
    inv_node_id: NodeId,
) -> TypeId {
    let error_ty = cc.registry.error_type;

    // Unwrap nilable receiver (e.g. `x?.method()` - receiver type is T?)
    let base_ty = cc
        .registry
        .nilable_inner(receiver_ty)
        .unwrap_or(receiver_ty);

    // Extract decl info and type args from the receiver type.
    let named_info = match cc.registry.type_entry(base_ty) {
        TypeEntry::Named { decl, type_args } => {
            let entry = cc.registry.decl(*decl);
            let module_path = cc.registry.module_path(entry.module).clone();
            let ta: Vec<(Symbol, TypeId)> = type_args
                .iter()
                .map(|a| (a.name.clone(), a.value))
                .collect();
            Some((entry.module, entry.name.clone(), module_path, ta))
        }
        _ => None,
    };
    let Some((type_module_idx, data_name, module_path, mut type_args)) = named_info else {
        // Not a named type - can't look up members.
        let ty_name = cc.registry.display_type(base_ty);
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!("no member '{}' on type '{}'", member_name, ty_name),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
        });
        for arg in args {
            check_expr_inner(cc, &arg.value);
        }
        return error_ty;
    };

    let member = if module_path == cc.module_path {
        // Same module: use in-progress declarations (files in cc)
        cc.lookup_decl(&data_name)
            .and_then(|d| d.get_member(member_name))
            .cloned()
    } else {
        // Cross-module: look up the fully loaded module
        cc.registry.module_index(&module_path).and_then(|idx| {
            cc.registry
                .module(idx)
                .get_declaration(&data_name)
                .and_then(|decl| decl.get_member(member_name))
                .cloned()
        })
    };

    let Some(member) = member else {
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!("no member '{}' on '{}'", member_name, data_name),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
        });
        for arg in args {
            check_expr_inner(cc, &arg.value);
        }
        return error_ty;
    };

    // Non-out member funcs are private - cannot be called from outside the construct.
    if member.visibility == Visibility::Private {
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!(
                "member '{}' on '{}' is not out (private)",
                member_name, data_name
            ),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
        });
        for arg in args {
            check_expr_inner(cc, &arg.value);
        }
        return error_ty;
    }

    // Underscore-prefixed members are project-private.
    let type_module_path = cc.registry.module_path(type_module_idx).clone();
    if is_cross_project_private(member_name, &cc.module_path, &type_module_path) {
        cc.errors.push(LoadError {
            module: cc.module_path.clone(),
            message: format!(
                "member '{}' on '{}' is project-private (underscore prefix)",
                member_name, data_name
            ),
            file_idx: cc.file_idx,
            range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
        });
        for arg in args {
            check_expr_inner(cc, &arg.value);
        }
        return error_ty;
    }

    // Resolve member func's own type args (e.g. `obj.method[t = str](...)`)
    // Member type args shadow receiver type args with the same name.
    if !member_type_args.is_empty() {
        let mut resolved_member_args: Vec<(Symbol, TypeId)> = member_type_args
            .iter()
            .map(|ta| {
                let resolved = crate::load::type_resolve::resolve_ast_type(cc, &ta.value, &[]);
                // Store resolved type so runtime can look it up via node_id
                cc.node_types.insert(ta.value.node_id(), resolved);
                (ta.name.name.clone(), resolved)
            })
            .collect();
        // Remove shadowed receiver type args, then prepend member args (first match wins)
        type_args.retain(|(name, _)| !resolved_member_args.iter().any(|(n, _)| n == name));
        resolved_member_args.append(&mut type_args);
        type_args = resolved_member_args;
    }

    // Substitute type params in member fields if any type args exist.
    if type_args.is_empty() {
        check_invocation_args(cc, &member.fields, args, inv_node_id);
        return decl_return_type(cc, &member);
    }

    let substituted_fields: Vec<TypeField> = member
        .fields
        .iter()
        .map(|f| TypeField {
            name: f.name.clone(),
            modifier: f.modifier,
            ty: cc.registry.substitute_type_params(f.ty, &type_args),
            has_default: f.has_default,
            node_id: f.node_id,
        })
        .collect();

    check_invocation_args(cc, &substituted_fields, args, inv_node_id);

    // Build return type from substituted out fields.
    let out_fields: Vec<TypeField> = substituted_fields
        .iter()
        .filter(|f| {
            matches!(
                f.modifier,
                Some(FieldModifier::Out) | Some(FieldModifier::OutEarly)
            )
        })
        .cloned()
        .collect();
    if out_fields.is_empty() {
        return cc.registry.intern_nilable(cc.registry.any_type); // void-like
    }
    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Data,
        FuncModifiers {
            view: false,
            noblock: false,
        },
        out_fields,
        vec![],
    ))
}

fn check_invocation_args<'a>(
    cc: &mut CheckContext<'a>,
    fields: &[super::TypeField],
    args: &'a [model::ExprInvocationArg],
    inv_node_id: NodeId,
) -> Vec<(Symbol, TypeId)> {
    // Skip arg checking if fields haven't been resolved yet (stub module).
    if fields.is_empty() && !args.is_empty() {
        let mut arg_types = Vec::with_capacity(args.len());
        for arg in args {
            let ty = check_expr_inner(cc, &arg.value);
            arg_types.push((arg.name.name.clone(), ty));
        }
        return arg_types;
    }
    let input_fields: Vec<_> = fields
        .iter()
        .filter(|f| {
            matches!(
                f.modifier,
                Some(FieldModifier::In) | Some(FieldModifier::Inout)
            )
        })
        .collect();
    let mut seen: HashSet<&str> = HashSet::new();
    let mut arg_types = Vec::with_capacity(args.len());

    for arg in args {
        let expected = input_fields.iter().find(|f| f.name == arg.name.name);

        let Some(expected) = expected else {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("unknown argument '{}'", arg.name.name),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&arg.name.node_id).copied(),
            });
            continue;
        };
        seen.insert(&expected.name);

        let actual = check_expr_inner(cc, &arg.value);
        arg_types.push((arg.name.name.clone(), actual));

        // Skip checks for unknown/void-like inference until expression typing is richer.
        if actual == cc.registry.error_type {
            continue;
        }
        let nil_any = cc.registry.intern_nilable(cc.registry.any_type);
        if actual == nil_any {
            // Keep explicit nil strict: nil cannot flow into non-nilable params.
            let is_nil_literal = matches!(
                &arg.value,
                model::Expr::Literal(model::ExprLiteral {
                    kind: model::ExprLiteralKind::Nil,
                    ..
                })
            );
            let is_nilable_or_any = {
                matches!(cc.registry.type_entry(expected.ty), TypeEntry::Nilable(_))
                    || cc.registry.is_any(expected.ty)
            };
            if is_nil_literal && !is_nilable_or_any {
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: format!(
                        "argument '{}' type mismatch: cannot assign nil to non-nilable type {}",
                        arg.name.name,
                        cc.registry.display_type(expected.ty),
                    ),
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&arg.value.node_id()).copied(),
                });
            }
            continue;
        }

        if let Err(msg) = check_assignable_extended(cc, actual, expected.ty) {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("argument '{}' type mismatch: {}", arg.name.name, msg),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&arg.value.node_id()).copied(),
            });
        }
    }

    for field in input_fields {
        if !field.has_default && !seen.contains(field.name.as_str()) {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("missing required argument '{}'", field.name),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&inv_node_id).copied(),
            });
        }
    }

    arg_types
}

/// Build the return type of a declaration from its out/out! fields.
fn decl_return_type(cc: &mut CheckContext, decl: &TypeConstruct) -> TypeId {
    let out_fields: Vec<_> = decl
        .fields
        .iter()
        .filter(|f| {
            matches!(
                f.modifier,
                Some(FieldModifier::Out) | Some(FieldModifier::OutEarly)
            )
        })
        .collect();
    if out_fields.is_empty() {
        return cc.registry.intern_nilable(cc.registry.any_type); // void-like
    }
    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Data,
        FuncModifiers {
            view: false,
            noblock: false,
        },
        out_fields
            .iter()
            .map(|f| TypeField {
                modifier: f.modifier,
                name: f.name.clone(),
                ty: f.ty,
                has_default: f.has_default,
                node_id: f.node_id,
            })
            .collect(),
        vec![],
    ))
}

/// Like `decl_return_type` but substitutes type params in out field types.
fn decl_return_type_substituted(
    cc: &mut CheckContext,
    decl: &TypeConstruct,
    subs: &[(Symbol, TypeId)],
) -> TypeId {
    let out_fields: Vec<_> = decl
        .fields
        .iter()
        .filter(|f| {
            matches!(
                f.modifier,
                Some(FieldModifier::Out) | Some(FieldModifier::OutEarly)
            )
        })
        .map(|f| TypeField {
            modifier: f.modifier,
            name: f.name.clone(),
            ty: cc.registry.substitute_type_params(f.ty, subs),
            has_default: f.has_default,
            node_id: f.node_id,
        })
        .collect();
    if out_fields.is_empty() {
        return cc.registry.intern_nilable(cc.registry.any_type);
    }
    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Data,
        FuncModifiers {
            view: false,
            noblock: false,
        },
        out_fields,
        vec![],
    ))
}

/// Extract the return type when a value of `ty` is invoked.
/// Handles anonymous func types; returns error_type for non-callable types.
fn invocation_return_type(cc: &mut CheckContext, ty: TypeId) -> TypeId {
    let anon = match cc.registry.type_entry(ty) {
        TypeEntry::Anonymous(a) if a.kind.is_func() => Some(a.clone()),
        _ => None,
    };
    let Some(anon) = anon else {
        return cc.registry.error_type;
    };
    let out_fields: Vec<_> = anon
        .fields
        .iter()
        .filter(|f| {
            matches!(
                f.modifier,
                Some(FieldModifier::Out) | Some(FieldModifier::OutEarly)
            )
        })
        .cloned()
        .collect();
    if out_fields.is_empty() {
        return cc.registry.intern_nilable(cc.registry.any_type);
    }
    cc.registry.intern_anonymous(anon_type_construct(
        TypeConstructKind::Data,
        FuncModifiers {
            view: false,
            noblock: false,
        },
        out_fields,
        vec![],
    ))
}

fn check_binary<'a>(cc: &mut CheckContext<'a>, b: &'a model::ExprBinary) -> TypeId {
    let left = check_expr_inner(cc, &b.left);
    let right = check_expr_inner(cc, &b.right);
    let error_ty = cc.registry.error_type;
    if left == error_ty || right == error_ty {
        return error_ty;
    }
    let int_ty = cc.registry.int_type;
    let float_ty = cc.registry.float_type;
    let str_ty = cc.registry.str_type;
    let bool_ty = cc.registry.bool_type;
    let result = match b.op {
        model::ExprBinaryOp::Add => {
            if left == int_ty && right == int_ty {
                Some(int_ty)
            } else if left == float_ty && right == float_ty {
                Some(float_ty)
            } else if left == str_ty && right == str_ty {
                Some(str_ty)
            } else {
                None
            }
        }
        model::ExprBinaryOp::Sub
        | model::ExprBinaryOp::Mul
        | model::ExprBinaryOp::Div
        | model::ExprBinaryOp::Mod => {
            if left == int_ty && right == int_ty {
                Some(int_ty)
            } else if left == float_ty && right == float_ty {
                Some(float_ty)
            } else {
                None
            }
        }
        model::ExprBinaryOp::Eq
        | model::ExprBinaryOp::Ne
        | model::ExprBinaryOp::Lt
        | model::ExprBinaryOp::Le
        | model::ExprBinaryOp::Gt
        | model::ExprBinaryOp::Ge => Some(bool_ty),
        model::ExprBinaryOp::And | model::ExprBinaryOp::Or => {
            if left == bool_ty && right == bool_ty {
                Some(bool_ty)
            } else {
                None
            }
        }
        model::ExprBinaryOp::NilCoalesce => {
            let inner = cc.registry.nilable_inner(left).unwrap_or(left);
            if check_assignable_extended(cc, right, inner).is_err() {
                cc.errors.push(LoadError {
                    module: cc.module_path.clone(),
                    message: format!(
                        "?? fallback type {} is not assignable to {}",
                        cc.registry.display_type(right),
                        cc.registry.display_type(inner),
                    ),
                    file_idx: cc.file_idx,
                    range: cc.source_map.node_ranges.get(&b.right.node_id()).copied(),
                });
            }
            Some(inner)
        }
    };
    if let Some(ty) = result {
        return ty;
    }
    let left_name = cc.registry.display_type(left);
    let right_name = cc.registry.display_type(right);
    cc.errors.push(LoadError {
        module: cc.module_path.clone(),
        message: format!(
            "cannot apply '{}' to {left_name} and {right_name}",
            b.op.as_str()
        ),
        file_idx: cc.file_idx,
        range: cc.source_map.node_ranges.get(&b.node_id).copied(),
    });
    error_ty
}

/// Check if `from` type is assignable to `to` type.
/// Returns Ok(()) if assignable, Err with a description if not.
/// Naive for now - will be expanded as the type system matures.
pub(in crate::load) fn check_extern_serializability(cc: &mut CheckContext, decl: &TypeConstruct) {
    for field in &decl.fields {
        let is_ser = cc.registry.is_type_serializable(field.ty);
        if !is_ser {
            cc.errors.push(LoadError {
                module: cc.module_path.clone(),
                message: format!("extern field '{}' must be a serializable type", field.name),
                file_idx: cc.file_idx,
                range: cc.source_map.node_ranges.get(&field.node_id).copied(),
            });
        }
    }
}

/// Check that type args on all field types satisfy their intype constraints.
/// E.g. `array[t = func {...}]` is invalid when array has `intype t` (defaults to `any`)
/// because func is not serializable.
pub(in crate::load) fn check_type_arg_constraints(cc: &mut CheckContext, decl: &TypeConstruct) {
    for field in &decl.fields {
        check_type_in_field(cc, field.ty, &decl.intypes, field.node_id);
    }
    for member in &decl.members {
        for field in &member.fields {
            check_type_in_field(cc, field.ty, &decl.intypes, field.node_id);
        }
    }
}

/// Walk a type and check any Named type's type args against their intype constraints.
fn check_type_in_field(
    cc: &mut CheckContext,
    ty: TypeId,
    enclosing_intypes: &[crate::load::TypeIntype],
    node_id: NodeId,
) {
    // Extract what we need from the registry in one shot, then process outside.
    enum Action {
        CheckNamed {
            target_intypes: Vec<crate::load::TypeIntype>,
            type_args: Vec<(Symbol, TypeId)>,
        },
        Recurse(TypeId),
        None,
    }

    let action = match cc.registry.type_entry(ty) {
        TypeEntry::Named { decl, type_args } => {
            let entry = cc.registry.decl(*decl);
            let is_same_module = *cc.registry.module_path(entry.module) == cc.module_path;
            // Same-module: look in files (registry has stub during type checking).
            // Cross-module: look in registry's loaded module.
            let target_intypes = if is_same_module {
                if let Some(&(fi, di)) = cc.decl_index.get(&entry.name) {
                    cc.files[fi].declarations[di].intypes.clone()
                } else {
                    vec![]
                }
            } else {
                cc.registry.decl_intypes(*decl).to_vec()
            };
            Action::CheckNamed {
                target_intypes,
                type_args: type_args
                    .iter()
                    .map(|a| (a.name.clone(), a.value))
                    .collect(),
            }
        }
        TypeEntry::Nilable(inner) | TypeEntry::EntityRef(inner) => Action::Recurse(*inner),
        _ => Action::None,
    };

    match action {
        Action::CheckNamed {
            target_intypes,
            type_args,
        } => {
            for (arg_name, arg_value) in &type_args {
                let Some(intype) = target_intypes.iter().find(|it| &it.name == arg_name) else {
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: format!("unknown type argument '{}'", arg_name),
                        file_idx: cc.file_idx,
                        range: cc.source_map.node_ranges.get(&node_id).copied(),
                    });
                    continue;
                };
                // No constraint or anylocal → anything is fine.
                let constraint = intype.constraint.unwrap_or(cc.registry.any_type);
                if cc.registry.is_anylocal(constraint) {
                    continue;
                }
                // Constraint is `any` → type arg must be serializable.
                let ok = {
                    if let TypeEntry::TypeParam(param_name) = cc.registry.type_entry(*arg_value) {
                        enclosing_intypes
                            .iter()
                            .find(|it| &it.name == param_name)
                            .map(|it| {
                                !cc.registry
                                    .is_anylocal(it.constraint.unwrap_or(cc.registry.any_type))
                            })
                            .unwrap_or(true)
                    } else {
                        cc.registry.is_type_serializable(*arg_value)
                    }
                };
                if !ok {
                    cc.errors.push(LoadError {
                        module: cc.module_path.clone(),
                        message: format!(
                            "type argument '{}' must be serializable (intype constraint is 'any')",
                            arg_name
                        ),
                        file_idx: cc.file_idx,
                        range: cc.source_map.node_ranges.get(&node_id).copied(),
                    });
                }
            }
            // Recurse into type arg values to check nested type args.
            for (_, arg_value) in &type_args {
                check_type_in_field(cc, *arg_value, enclosing_intypes, node_id);
            }
        }
        Action::Recurse(inner) => {
            check_type_in_field(cc, inner, enclosing_intypes, node_id);
        }
        Action::None => {}
    }
}
