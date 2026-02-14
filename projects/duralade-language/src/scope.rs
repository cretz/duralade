use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::load::registry::TypeId;
use crate::model::Symbol;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ScopeKind {
    Module,
    File,
    Function,
    Construct,
    #[default]
    Block,
    For,
    Implicitly,
}

pub type ScopeRef<Var, Construct> = Rc<RefCell<Scope<Var, Construct>>>;

/// A single scope level, parameterized over variable values and construct references.
///
/// Scopes form a tree via parent links. The parent always points to the
/// definition site — never the call site. This means:
/// - A named function's parent is its module or construct scope
/// - A closure's parent is the scope where it was created
/// - Visibility is purely structural: if it's in the parent chain, it's visible
///
/// Construct with `..Default::default()` and call `into_ref()` to wrap in Rc<RefCell>.
pub struct Scope<Var, Construct> {
    pub kind: ScopeKind,
    pub parent: Option<ScopeRef<Var, Construct>>,
    pub name: Option<Symbol>,
    pub vars: BTreeMap<Symbol, Var>,
    pub type_params: BTreeMap<Symbol, TypeId>,
    pub construct: Option<Construct>,
    /// Which field is the `out!` field (for `return!` resolution).
    pub out_early: Option<Symbol>,
    pub label: Option<Symbol>,
}

impl<Var, Construct> Default for Scope<Var, Construct> {
    fn default() -> Self {
        Self {
            kind: ScopeKind::default(),
            parent: None,
            name: None,
            vars: BTreeMap::new(),
            type_params: BTreeMap::new(),
            construct: None,
            out_early: None,
            label: None,
        }
    }
}

impl<Var, Construct> std::fmt::Debug for Scope<Var, Construct>
where
    Var: std::fmt::Debug,
    Construct: std::fmt::Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Scope")
            .field("kind", &self.kind)
            .field("name", &self.name)
            .field("vars", &self.vars.keys().collect::<Vec<_>>())
            .field("type_params", &self.type_params.keys().collect::<Vec<_>>())
            .field("construct", &self.construct)
            .field("out_early", &self.out_early)
            .field("label", &self.label)
            .field("parent", &self.parent.is_some())
            .finish()
    }
}

impl<Var, Construct> Scope<Var, Construct> {
    pub fn into_ref(self) -> ScopeRef<Var, Construct> {
        Rc::new(RefCell::new(self))
    }

    /// Declare a new variable in this scope. Returns false if the name
    /// already exists in this scope (redeclaration).
    pub fn declare_var(&mut self, name: Symbol, value: Var) -> bool {
        if self.vars.contains_key(&name) {
            return false;
        }
        self.vars.insert(name, value);
        true
    }

    /// Update an existing variable by walking up the scope chain.
    /// Checks vars, then construct-backed storage (via callback), then parent.
    pub fn set_var(
        &mut self,
        name: &Symbol,
        value: Var,
        construct_set: &impl Fn(&Construct, &Symbol, Var) -> bool,
    ) -> bool
    where
        Var: Clone,
    {
        if let Some(ref c) = self.construct
            && construct_set(c, name, value.clone())
        {
            return true;
        }
        if self.vars.contains_key(name) {
            self.vars.insert(name.clone(), value);
            return true;
        }
        self.parent
            .as_ref()
            .is_some_and(|p| p.borrow_mut().set_var(name, value, construct_set))
    }
}

impl<Var: Clone, Construct> Scope<Var, Construct> {
    /// Resolve a variable by walking up the scope chain.
    /// Checks vars, then construct-backed storage (via callback), then parent.
    /// No boundaries — visibility is purely structural from the parent chain.
    pub fn resolve_var(
        &self,
        name: &Symbol,
        construct_lookup: &impl Fn(&Construct, &Symbol) -> Option<Var>,
    ) -> Option<Var> {
        if let Some(v) = self.vars.get(name) {
            return Some(v.clone());
        }
        if let Some(ref c) = self.construct
            && let Some(v) = construct_lookup(c, name)
        {
            return Some(v);
        }
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().resolve_var(name, construct_lookup))
    }

    /// Resolve a type param by walking up the scope chain.
    pub fn resolve_type_param(&self, name: &Symbol) -> Option<TypeId> {
        if let Some(&ty) = self.type_params.get(name) {
            return Some(ty);
        }
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().resolve_type_param(name))
    }

    /// Find the `out!` field from the nearest enclosing Function scope.
    pub fn enclosing_out_early(&self) -> Option<(Symbol, Var)> {
        if self.kind == ScopeKind::Function {
            if let Some(ref name) = self.out_early {
                let ty = self.vars.get(name)?.clone();
                return Some((name.clone(), ty));
            }
            return None;
        }
        self.parent
            .as_ref()
            .and_then(|p| p.borrow().enclosing_out_early())
    }

    /// Set the `out!` field value in the nearest enclosing Function scope.
    pub fn set_enclosing_out_early(&mut self, value: Var) -> Option<Symbol> {
        if self.kind == ScopeKind::Function {
            if let Some(ref name) = self.out_early {
                let name = name.clone();
                self.vars.insert(name.clone(), value);
                return Some(name);
            }
            return None;
        }
        self.parent
            .as_ref()
            .and_then(|p| p.borrow_mut().set_enclosing_out_early(value))
    }

    /// Check if break/continue can reach an enclosing for loop.
    /// Function scopes block break/continue (can't break out of a function).
    pub fn can_break_or_continue(&self, target_label: Option<&Symbol>) -> bool {
        if self.kind == ScopeKind::For {
            match target_label {
                Some(target) => {
                    if self.label.as_ref() == Some(target) {
                        return true;
                    }
                }
                None => return true,
            }
        }
        if self.kind == ScopeKind::Function {
            return false;
        }
        self.parent
            .as_ref()
            .is_some_and(|p| p.borrow().can_break_or_continue(target_label))
    }
}
