use std::collections::{HashMap, HashSet, VecDeque};

use crate::model::{self, Symbol};

/// Collect bare identifier references from an expression that could refer to
/// fields of the enclosing construct. Tracks shadowed names through inner scopes
/// (anon func/data params, var declarations, for-in bindings, narrowing bindings).
fn collect_field_refs(
    expr: &model::Expr,
    field_names: &HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    collect_field_refs_expr(expr, field_names, &mut HashSet::new(), out);
}

fn collect_field_refs_expr(
    expr: &model::Expr,
    field_names: &HashSet<Symbol>,
    shadowed: &mut HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    match expr {
        model::Expr::Ident(ident) => {
            if field_names.contains(&ident.name) && !shadowed.contains(&ident.name) {
                out.insert(ident.name.clone());
            }
        }
        model::Expr::Access(access) => {
            collect_field_refs_expr(&access.expr, field_names, shadowed, out);
        }
        model::Expr::Paren(paren) => {
            collect_field_refs_expr(&paren.expr, field_names, shadowed, out);
        }
        model::Expr::Unary(unary) => {
            collect_field_refs_expr(&unary.expr, field_names, shadowed, out);
        }
        model::Expr::Binary(binary) => {
            collect_field_refs_expr(&binary.left, field_names, shadowed, out);
            collect_field_refs_expr(&binary.right, field_names, shadowed, out);
        }
        model::Expr::Narrowing(narrowing) => {
            collect_field_refs_expr(&narrowing.expr, field_names, shadowed, out);
            match &narrowing.kind {
                model::ExprNarrowingKind::Nil(nil) => {
                    if let Some(else_expr) = &nil.else_expr {
                        collect_field_refs_expr(else_expr, field_names, shadowed, out);
                    }
                }
                model::ExprNarrowingKind::As(as_) => {
                    if let Some(else_expr) = &as_.else_expr {
                        collect_field_refs_expr(else_expr, field_names, shadowed, out);
                    }
                }
            }
        }
        model::Expr::Invocation(inv) => {
            collect_field_refs_expr(&inv.expr, field_names, shadowed, out);
            for arg in &inv.args {
                collect_field_refs_expr(&arg.value, field_names, shadowed, out);
            }
        }
        model::Expr::Wait(wait) => {
            collect_field_refs_expr(&wait.condition, field_names, shadowed, out);
        }
        model::Expr::Spawn(spawn) => {
            collect_field_refs_expr(&spawn.invocation, field_names, shadowed, out);
            if let Some(id) = &spawn.id {
                collect_field_refs_expr(id, field_names, shadowed, out);
            }
        }
        model::Expr::Literal(lit) => match &lit.kind {
            model::ExprLiteralKind::Array(items) => {
                for item in items {
                    collect_field_refs_expr(item, field_names, shadowed, out);
                }
            }
            model::ExprLiteralKind::Map(entries) => {
                for entry in entries {
                    collect_field_refs_expr(&entry.key, field_names, shadowed, out);
                    collect_field_refs_expr(&entry.value, field_names, shadowed, out);
                }
            }
            _ => {}
        },
        model::Expr::AnonFunc(func) => {
            let mut inner_shadowed = shadowed.clone();
            for field in &func.fields {
                if let model::FieldKind::Var(fv) = &field.kind {
                    inner_shadowed.insert(fv.name.name.clone());
                }
            }
            // Walk field defaults - they can reference outer fields
            for field in &func.fields {
                if let model::FieldKind::Var(fv) = &field.kind
                    && let Some(default_expr) = &fv.default
                {
                    collect_field_refs_expr(default_expr, field_names, &mut inner_shadowed, out);
                }
            }
            collect_field_refs_stmts(&func.stmts, field_names, &mut inner_shadowed, out);
        }
        model::Expr::AnonData(data) => {
            let mut inner_shadowed = shadowed.clone();
            for field in &data.fields {
                if let model::FieldKind::Var(fv) = &field.kind {
                    inner_shadowed.insert(fv.name.name.clone());
                    if let Some(default_expr) = &fv.default {
                        collect_field_refs_expr(
                            default_expr,
                            field_names,
                            &mut inner_shadowed,
                            out,
                        );
                    }
                }
            }
        }
        model::Expr::ModuleAccess(_) | model::Expr::Outer(_) | model::Expr::Invalid(_) => {}
    }
}

fn collect_field_refs_stmts(
    stmts: &[model::Stmt],
    field_names: &HashSet<Symbol>,
    shadowed: &mut HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    for stmt in stmts {
        collect_field_refs_stmt(stmt, field_names, shadowed, out);
    }
}

fn collect_field_refs_stmt(
    stmt: &model::Stmt,
    field_names: &HashSet<Symbol>,
    shadowed: &mut HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    match stmt {
        model::Stmt::Expr(expr) => {
            collect_field_refs_expr(expr, field_names, shadowed, out);
        }
        model::Stmt::Var(var) => {
            // Evaluate RHS before shadowing (RHS can reference fields)
            for value in &var.values {
                collect_field_refs_expr(value, field_names, shadowed, out);
            }
            // Then shadow the declared names
            for decl in &var.vars {
                shadowed.insert(decl.name.name.clone());
            }
        }
        model::Stmt::Assign(assign) => {
            for value in &assign.values {
                collect_field_refs_expr(value, field_names, shadowed, out);
            }
            for target in &assign.vars {
                collect_field_refs_expr(target, field_names, shadowed, out);
            }
        }
        model::Stmt::Block(block) => {
            let mut inner_shadowed = shadowed.clone();
            collect_field_refs_stmts(&block.stmts, field_names, &mut inner_shadowed, out);
        }
        model::Stmt::If(if_stmt) => {
            collect_field_refs_if_condition(&if_stmt.condition, field_names, shadowed, out);
            let mut then_shadowed = shadowed.clone();
            add_if_condition_bindings(&if_stmt.condition, &mut then_shadowed);
            collect_field_refs_stmts(
                &if_stmt.then_block.stmts,
                field_names,
                &mut then_shadowed,
                out,
            );

            for else_if in &if_stmt.else_ifs {
                collect_field_refs_if_condition(&else_if.condition, field_names, shadowed, out);
                let mut ei_shadowed = shadowed.clone();
                add_if_condition_bindings(&else_if.condition, &mut ei_shadowed);
                collect_field_refs_stmts(
                    &else_if.then_block.stmts,
                    field_names,
                    &mut ei_shadowed,
                    out,
                );
            }
            if let Some(else_block) = &if_stmt.else_block {
                let mut else_shadowed = shadowed.clone();
                collect_field_refs_stmts(&else_block.stmts, field_names, &mut else_shadowed, out);
            }
        }
        model::Stmt::For(for_stmt) => {
            let mut inner_shadowed = shadowed.clone();
            if let Some(clause) = &for_stmt.clause {
                match clause {
                    model::StmtForClause::Condition(cond) => {
                        collect_field_refs_expr(&cond.expr, field_names, shadowed, out);
                    }
                    model::StmtForClause::In(for_in) => {
                        collect_field_refs_expr(&for_in.expr, field_names, shadowed, out);
                        inner_shadowed.insert(for_in.var.name.clone());
                    }
                }
            }
            collect_field_refs_stmts(&for_stmt.block.stmts, field_names, &mut inner_shadowed, out);
        }
        model::Stmt::Defer(defer) => {
            let mut inner_shadowed = shadowed.clone();
            collect_field_refs_stmts(&defer.block.stmts, field_names, &mut inner_shadowed, out);
        }
        model::Stmt::Return(_) | model::Stmt::ForBreak(_) | model::Stmt::ForContinue(_) => {}
        model::Stmt::ReturnEarly(ret) => {
            collect_field_refs_expr(&ret.expr, field_names, shadowed, out);
        }
        model::Stmt::Implicitly(imp) => {
            for expr in &imp.exprs {
                collect_field_refs_expr(expr, field_names, shadowed, out);
            }
            if let Some(block) = &imp.block {
                let mut inner_shadowed = shadowed.clone();
                collect_field_refs_stmts(&block.stmts, field_names, &mut inner_shadowed, out);
            }
        }
        model::Stmt::Patch(patch) => match &patch.kind {
            model::StmtPatchKind::Chain(chain) => {
                for branch in &chain.branches {
                    let mut inner_shadowed = shadowed.clone();
                    collect_field_refs_stmts(&branch.stmts, field_names, &mut inner_shadowed, out);
                }
            }
            model::StmtPatchKind::Complete(_) => {}
        },
        model::Stmt::Invalid(_) => {}
    }
}

fn collect_field_refs_if_condition(
    condition: &model::StmtIfCondition,
    field_names: &HashSet<Symbol>,
    shadowed: &mut HashSet<Symbol>,
    out: &mut HashSet<Symbol>,
) {
    match condition {
        model::StmtIfCondition::Bool(cond) => {
            if let Some(init) = &cond.init {
                collect_field_refs_stmt(init, field_names, shadowed, out);
            }
            collect_field_refs_expr(&cond.expr, field_names, shadowed, out);
        }
        model::StmtIfCondition::NarrowingAs(nar) => {
            for binding in &nar.bindings {
                collect_field_refs_expr(&binding.expr, field_names, shadowed, out);
            }
        }
        model::StmtIfCondition::NarrowingNil(nar) => {
            for binding in &nar.bindings {
                collect_field_refs_expr(&binding.expr, field_names, shadowed, out);
            }
        }
    }
}

fn add_if_condition_bindings(condition: &model::StmtIfCondition, shadowed: &mut HashSet<Symbol>) {
    match condition {
        model::StmtIfCondition::Bool(cond) => {
            if let Some(init) = &cond.init
                && let model::Stmt::Var(var) = init.as_ref()
            {
                for decl in &var.vars {
                    shadowed.insert(decl.name.name.clone());
                }
            }
        }
        model::StmtIfCondition::NarrowingAs(nar) => {
            for binding in &nar.bindings {
                shadowed.insert(binding.name.name.clone());
            }
        }
        model::StmtIfCondition::NarrowingNil(nar) => {
            for binding in &nar.bindings {
                shadowed.insert(binding.name.name.clone());
            }
        }
    }
}

/// Compute a topologically sorted evaluation order for AST fields.
/// Returns indices into the AST `construct.fields` array (Var fields only).
/// Detects circular references and reports them as load errors.
pub(super) fn compute_ast_field_eval_order(
    cc: &mut super::type_check::CheckContext,
    fields: &[model::Field],
) -> Vec<usize> {
    let module_path = &cc.module_path;
    let file_idx = cc.file_idx;
    let source_map = cc.source_map;
    // Build field_names set for the collect_field_refs filter
    let mut field_names: HashSet<Symbol> = HashSet::new();
    let mut var_indices: Vec<usize> = Vec::new();
    for (i, field) in fields.iter().enumerate() {
        if let model::FieldKind::Var(fv) = &field.kind {
            field_names.insert(fv.name.name.clone());
            var_indices.push(i);
        }
    }

    if var_indices.is_empty() {
        return vec![];
    }

    // Map field name → AST index
    let mut name_to_idx: HashMap<Symbol, usize> = HashMap::new();
    for &i in &var_indices {
        if let model::FieldKind::Var(fv) = &fields[i].kind {
            name_to_idx.insert(fv.name.name.clone(), i);
        }
    }

    // Build dependency graph
    let mut in_degree: HashMap<usize, usize> = HashMap::new();
    let mut reverse_adj: HashMap<usize, Vec<usize>> = HashMap::new();

    for &i in &var_indices {
        in_degree.entry(i).or_insert(0);

        if let model::FieldKind::Var(fv) = &fields[i].kind
            && let Some(default_expr) = &fv.default
        {
            let mut refs = HashSet::new();
            collect_field_refs(default_expr, &field_names, &mut refs);

            // A field's own name can't refer to itself - it isn't in
            // scope yet when its default is evaluated.  The bare ident
            // will resolve from an outer scope or fail at runtime.
            // TODO: add runtime tests for `in foo = foo` when foo is/isn't in outer scope
            refs.remove(&fv.name.name);

            for ref_name in &refs {
                if let Some(&dep_idx) = name_to_idx.get(ref_name) {
                    reverse_adj.entry(dep_idx).or_default().push(i);
                    *in_degree.entry(i).or_insert(0) += 1;
                }
            }
        }
    }

    // Sort adjacency lists so dependents are always enqueued in source order
    for list in reverse_adj.values_mut() {
        list.sort();
    }

    // Kahn's algorithm - seed with zero-degree nodes in source order for determinism
    let mut queue: VecDeque<usize> = VecDeque::new();
    for &idx in &var_indices {
        if in_degree.get(&idx) == Some(&0) {
            queue.push_back(idx);
        }
    }

    let mut order: Vec<usize> = Vec::new();
    while let Some(idx) = queue.pop_front() {
        order.push(idx);
        if let Some(dependents) = reverse_adj.get(&idx) {
            for &dep in dependents {
                let deg = in_degree.get_mut(&dep).unwrap();
                *deg -= 1;
                if *deg == 0 {
                    queue.push_back(dep);
                }
            }
        }
    }

    // Cycle detection
    if order.len() != in_degree.len() {
        let mut cycle_participants: Vec<usize> = in_degree
            .iter()
            .filter(|(_, deg)| **deg > 0)
            .map(|(idx, _)| *idx)
            .collect();
        cycle_participants.sort();
        let cycle_fields: Vec<&str> = cycle_participants
            .iter()
            .filter_map(|idx| {
                if let model::FieldKind::Var(fv) = &fields[*idx].kind {
                    Some(fv.name.name.as_str())
                } else {
                    None
                }
            })
            .collect();
        let first_range = cycle_participants
            .first()
            .and_then(|idx| source_map.node_ranges.get(&fields[*idx].node_id).copied());
        cc.errors.push(super::LoadError {
            module: module_path.clone(),
            message: format!(
                "Circular field default references among: {}",
                cycle_fields.join(", ")
            ),
            file_idx,
            range: first_range,
        });
        return var_indices;
    }

    order
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_field_names(names: &[&str]) -> HashSet<Symbol> {
        names.iter().map(|&s| s.into()).collect()
    }

    fn make_ident(name: &str) -> model::Ident {
        model::Ident {
            node_id: model::NodeId(0),
            name: name.into(),
            is_raw: false,
        }
    }

    fn ident_expr(name: &str) -> model::Expr {
        model::Expr::Ident(make_ident(name))
    }

    #[test]
    fn bare_ident_collected() {
        let field_names = make_field_names(&["x", "y"]);
        let mut refs = HashSet::new();
        collect_field_refs(&ident_expr("x"), &field_names, &mut refs);
        assert!(refs.contains("x"));
        assert!(!refs.contains("y"));
    }

    #[test]
    fn non_field_ident_ignored() {
        let field_names = make_field_names(&["x"]);
        let mut refs = HashSet::new();
        collect_field_refs(&ident_expr("z"), &field_names, &mut refs);
        assert!(refs.is_empty());
    }

    #[test]
    fn shadowed_by_anon_func_param() {
        let field_names = make_field_names(&["x"]);
        let mut refs = HashSet::new();
        // func(in x: ...) { x } - x is shadowed
        let anon = model::Expr::AnonFunc(model::Func {
            node_id: model::NodeId(0),
            view: false,
            noblock: false,
            fields: vec![model::Field {
                node_id: model::NodeId(0),
                annotations: vec![],
                kind: model::FieldKind::Var(model::FieldVar {
                    node_id: model::NodeId(0),
                    modifier: model::FieldVarModifier::In,
                    name: make_ident("x"),
                    ty: None,
                    default: None,
                }),
            }],
            stmts: vec![model::Stmt::Expr(ident_expr("x"))],
        });
        collect_field_refs(&anon, &field_names, &mut refs);
        assert!(refs.is_empty());
    }

    #[test]
    fn unshadowed_in_anon_func_body() {
        let field_names = make_field_names(&["x", "y"]);
        let mut refs = HashSet::new();
        // func(in x: ...) { y } - y is NOT shadowed
        let anon = model::Expr::AnonFunc(model::Func {
            node_id: model::NodeId(0),
            view: false,
            noblock: false,
            fields: vec![model::Field {
                node_id: model::NodeId(0),
                annotations: vec![],
                kind: model::FieldKind::Var(model::FieldVar {
                    node_id: model::NodeId(0),
                    modifier: model::FieldVarModifier::In,
                    name: make_ident("x"),
                    ty: None,
                    default: None,
                }),
            }],
            stmts: vec![model::Stmt::Expr(ident_expr("y"))],
        });
        collect_field_refs(&anon, &field_names, &mut refs);
        assert!(refs.contains("y"));
        assert!(!refs.contains("x"));
    }

    #[test]
    fn shadowed_by_var_decl_in_anon_func() {
        let field_names = make_field_names(&["x"]);
        let mut refs = HashSet::new();
        // func { x := 1; x } - x is shadowed after var decl
        let anon = model::Expr::AnonFunc(model::Func {
            node_id: model::NodeId(0),
            view: false,
            noblock: false,
            fields: vec![],
            stmts: vec![
                model::Stmt::Var(model::StmtVar {
                    node_id: model::NodeId(0),
                    vars: vec![model::StmtVarDecl {
                        node_id: model::NodeId(0),
                        name: make_ident("x"),
                        ty: None,
                    }],
                    values: vec![model::Expr::Literal(model::ExprLiteral {
                        node_id: model::NodeId(0),
                        kind: model::ExprLiteralKind::Int("1".to_string()),
                    })],
                }),
                model::Stmt::Expr(ident_expr("x")),
            ],
        });
        collect_field_refs(&anon, &field_names, &mut refs);
        assert!(refs.is_empty());
    }

    #[test]
    fn anon_data_default_refs_outer_field() {
        let field_names = make_field_names(&["x"]);
        let mut refs = HashSet::new();
        // data { in y = x } - x references outer field
        let anon = model::Expr::AnonData(model::Data {
            node_id: model::NodeId(0),
            fields: vec![model::Field {
                node_id: model::NodeId(0),
                annotations: vec![],
                kind: model::FieldKind::Var(model::FieldVar {
                    node_id: model::NodeId(0),
                    modifier: model::FieldVarModifier::In,
                    name: make_ident("y"),
                    ty: None,
                    default: Some(ident_expr("x")),
                }),
            }],
            funcs: vec![],
        });
        collect_field_refs(&anon, &field_names, &mut refs);
        assert!(refs.contains("x"));
    }

    #[test]
    fn anon_data_field_shadows_outer() {
        let field_names = make_field_names(&["x"]);
        let mut refs = HashSet::new();
        // data { in x: int; in y = x } - inner x shadows outer x
        let anon = model::Expr::AnonData(model::Data {
            node_id: model::NodeId(0),
            fields: vec![
                model::Field {
                    node_id: model::NodeId(0),
                    annotations: vec![],
                    kind: model::FieldKind::Var(model::FieldVar {
                        node_id: model::NodeId(0),
                        modifier: model::FieldVarModifier::In,
                        name: make_ident("x"),
                        ty: None,
                        default: None,
                    }),
                },
                model::Field {
                    node_id: model::NodeId(0),
                    annotations: vec![],
                    kind: model::FieldKind::Var(model::FieldVar {
                        node_id: model::NodeId(0),
                        modifier: model::FieldVarModifier::In,
                        name: make_ident("y"),
                        ty: None,
                        default: Some(ident_expr("x")),
                    }),
                },
            ],
            funcs: vec![],
        });
        collect_field_refs(&anon, &field_names, &mut refs);
        assert!(refs.is_empty());
    }
}
