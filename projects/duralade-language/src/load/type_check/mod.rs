mod assignable;
mod check;
mod infer;

use std::collections::HashMap;

use crate::model::{NodeId, SourceMap, Symbol};
use crate::scope;

use super::{
    FuncModifiers, LoadError, TypeConstruct, TypeConstructKind, TypeField, Visibility,
    registry::TypeId,
};

pub(super) use check::{
    check_expr, check_extern_serializability, check_func_body, check_member_func_bodies,
    check_redundant_annotation, check_type_arg_constraints, uninferrable_literal_message,
};

type CheckScope = scope::Scope<TypeId, Symbol>;
type CheckScopeRef = scope::ScopeRef<TypeId, Symbol>;

/// Build an anonymous `TypeConstruct` for use in type interning.
fn anon_type_construct(
    kind: TypeConstructKind,
    func_modifiers: FuncModifiers,
    fields: Vec<TypeField>,
    ast_field_eval_order: Vec<usize>,
) -> TypeConstruct {
    TypeConstruct {
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
        node_id: NodeId(0),
        ast_field_eval_order,
    }
}

pub(super) struct CheckContext<'a> {
    pub files: &'a [super::TypedFile],
    pub decl_index: &'a HashMap<Symbol, (usize, usize)>,
    pub module_path: Symbol,
    pub file_idx: usize,
    pub source_map: &'a SourceMap,
    pub registry: &'a mut super::registry::Registry,
    pub errors: &'a mut Vec<LoadError>,
    pub strict_violations: &'a mut Vec<LoadError>,
    pub populate_non_required_types: bool,
    scope: CheckScopeRef,
    /// Expression types resolved during checking, drained into TypeTable after.
    pub node_types: HashMap<NodeId, TypeId>,
    /// Target type for the current expression (set when a type annotation is present).
    /// Used by array/map literal checks to validate elements against the declared type
    /// instead of inferring from the first element.
    pub expected_type: Option<TypeId>,
}

impl<'a> CheckContext<'a> {
    pub fn new(
        ctx: &'a mut super::type_resolve::TypeContext<'_>,
        files: &'a [super::TypedFile],
        decl_index: &'a HashMap<Symbol, (usize, usize)>,
        module_path: Symbol,
        file_idx: usize,
        source_map: &'a SourceMap,
        populate_non_required_types: bool,
    ) -> Self {
        // Populate module scope with same-module declaration TypeIds.
        let mut module_vars = std::collections::BTreeMap::new();
        for (name, &(fi, di)) in decl_index {
            if let Some(decl_id) = files
                .get(fi)
                .and_then(|f| f.declarations.get(di))
                .and_then(|d| d.decl_id)
            {
                let type_id = ctx.registry.intern_named(decl_id);
                module_vars.insert(name.clone(), type_id);
            }
        }

        // Populate file scope with same-named declarations from imports.
        // `import foo.bar` makes `bar` available; if the module has a
        // declaration also called `bar`, it resolves as that declaration.
        let mut file_vars = std::collections::BTreeMap::new();
        for (alias, import_path) in &files[file_idx].import_map {
            if let Some(module_idx) = ctx.registry.module_index(import_path) {
                let decl_id = {
                    let module = ctx.registry.module(module_idx);
                    module.get_declaration(alias).and_then(|d| {
                        if d.visibility == super::Visibility::Public {
                            d.decl_id
                        } else {
                            None
                        }
                    })
                };
                if let Some(decl_id) = decl_id {
                    file_vars.insert(alias.clone(), ctx.registry.intern_named(decl_id));
                }
            }
        }

        let module_scope = CheckScope {
            kind: scope::ScopeKind::Module,
            name: Some(module_path.clone()),
            vars: module_vars,
            ..Default::default()
        }
        .into_ref();
        let file_scope = CheckScope {
            kind: scope::ScopeKind::File,
            parent: Some(module_scope),
            vars: file_vars,
            ..Default::default()
        }
        .into_ref();
        Self {
            files,
            decl_index,
            module_path,
            file_idx,
            source_map,
            registry: ctx.registry,
            errors: ctx.errors,
            strict_violations: ctx.strict_violations,
            populate_non_required_types,
            scope: file_scope,
            node_types: HashMap::new(),
            expected_type: None,
        }
    }

    /// Report a load error and return error_type in one step.
    pub(super) fn add_type_error(
        &mut self,
        message: String,
        range: Option<(usize, usize)>,
    ) -> TypeId {
        self.errors.push(LoadError {
            module: self.module_path.clone(),
            message,
            file_idx: self.file_idx,
            range,
        });
        self.registry.error_type
    }

    pub(super) fn lookup_decl(&self, name: &str) -> Option<&TypeConstruct> {
        let &(fi, di) = self.decl_index.get(name)?;
        self.files.get(fi)?.declarations.get(di)
    }

    /// Resolve a DeclId to its TypeConstruct. For same-module declarations
    /// uses `files` directly (registry holds a stub during type checking).
    /// For cross-module declarations uses the registry.
    pub(super) fn lookup_construct(
        &self,
        decl_id: super::registry::DeclId,
    ) -> Option<&TypeConstruct> {
        let entry = self.registry.decl(decl_id);
        if self.registry.module_index(&self.module_path) == Some(entry.module) {
            self.lookup_decl(&entry.name)
        } else {
            self.registry
                .module(entry.module)
                .get_declaration(&entry.name)
        }
    }

    pub(super) fn scope(&self) -> &CheckScopeRef {
        &self.scope
    }

    pub(super) fn set_scope(&mut self, scope: CheckScopeRef) {
        self.scope = scope;
    }

    pub(super) fn pop_scope(&mut self) {
        let parent = self.scope.borrow().parent.clone();
        if let Some(parent) = parent {
            self.scope = parent;
        }
    }

    pub(super) fn push_block_scope(&mut self) {
        self.scope = CheckScope {
            kind: scope::ScopeKind::Block,
            parent: Some(self.scope.clone()),
            ..Default::default()
        }
        .into_ref();
    }

    pub(super) fn declare_var(&mut self, name: Symbol, ty: TypeId) {
        if !self.scope.borrow_mut().declare_var(name.clone(), ty) {
            self.errors.push(LoadError {
                module: self.module_path.clone(),
                message: format!("variable '{name}' already declared in this scope"),
                file_idx: self.file_idx,
                range: None,
            });
        }
    }

    pub(super) fn lookup_var(&self, name: &str) -> Option<TypeId> {
        let sym: Symbol = name.into();
        self.scope
            .borrow()
            .resolve_var(&sym, &|decl_name, field_name| {
                let &(fi, di) = self.decl_index.get(decl_name.as_str())?;
                let decl = self.files.get(fi)?.declarations.get(di)?;
                decl.fields
                    .iter()
                    .find(|f| &f.name == field_name)
                    .map(|f| f.ty)
            })
    }

    pub(super) fn resolve_type_in_scope(&self, name: &str) -> Option<TypeId> {
        self.scope.borrow().resolve_type_param(&name.into())
    }

    pub(super) fn enclosing_out_early_type(&self) -> Option<TypeId> {
        self.scope.borrow().enclosing_out_early().map(|(_, ty)| ty)
    }

    pub(super) fn enclosing_out_early_name(&self) -> Option<Symbol> {
        self.scope
            .borrow()
            .enclosing_out_early()
            .map(|(name, _)| name)
    }

    pub(super) fn can_break_or_continue(&self, label: Option<&Symbol>) -> bool {
        self.scope.borrow().can_break_or_continue(label)
    }
}
