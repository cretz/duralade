use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;

use crate::load::registry::{DeclId, Registry, TypeId};
use crate::model::{NodeId, Symbol};
use crate::scope;

use super::value::{Heap, HeapValue, HeapValueFuncKind, Value, ValueId};

pub type RuntimeScope = scope::Scope<Value, ConstructScope>;
pub type RuntimeScopeRef = scope::ScopeRef<Value, ConstructScope>;

/// Construct reference for runtime scope — entity/data whose fields and
/// members are accessible by name from within member functions.
#[derive(Debug)]
pub struct ConstructScope {
    pub decl: DeclId,
    pub heap_id: Option<ValueId>,
    pub target: Value,
}

impl ConstructScope {
    /// Resolve a name against this construct — heap fields first, then members.
    pub fn resolve(
        &self,
        name: &Symbol,
        registry: &Registry,
        heap: &Rc<RefCell<Heap>>,
    ) -> Option<Value> {
        if let Some(vid) = self.heap_id {
            let h = heap.borrow();
            match h.get(vid) {
                HeapValue::Entity { fields, .. } | HeapValue::Data { fields, .. } => {
                    if let Some(v) = fields.get(name) {
                        return Some(v.clone());
                    }
                }
                _ => {}
            }
        }
        let entry = registry.decl(self.decl);
        let declaration = registry.module(entry.module).get_declaration(&entry.name)?;
        if declaration
            .members
            .iter()
            .any(|m| m.name.as_deref() == Some(name.as_str()))
        {
            let func_id = heap.borrow_mut().alloc(HeapValue::Func {
                module: entry.module,
                kind: HeapValueFuncKind::Decl {
                    decl: self.decl,
                    name: name.clone(),
                    target: Some(self.target.clone()),
                },
            });
            return Some(Value::Func(func_id));
        }
        None
    }

    /// Set a field on this construct's heap value. Returns true if found.
    pub fn set(&self, name: &Symbol, value: Value, heap: &Rc<RefCell<Heap>>) -> bool {
        if let Some(vid) = self.heap_id {
            let mut h = heap.borrow_mut();
            match h.get_mut(vid) {
                HeapValue::Entity { fields, .. } | HeapValue::Data { fields, .. } => {
                    if fields.contains_key(name) {
                        fields.insert(name.clone(), value);
                        return true;
                    }
                }
                _ => {}
            }
        }
        false
    }
}

/// Execution frame — wraps a shared scope with runtime execution state
/// (statement index, implicits, deferred closures).
#[derive(Debug, Clone)]
pub struct ExecFrame {
    pub scope: RuntimeScopeRef,
    /// The scope that was active before this frame was pushed (for pop).
    pub restore_scope: RuntimeScopeRef,
    /// Next statement index to execute in this frame's block.
    pub stmt_index: usize,
    pub implicits: Vec<ImplicitBinding>,
    /// Deferred closure RootedIds collected in this frame, run LIFO on scope exit.
    pub deferred: Vec<super::value::RootedId>,
}

impl ExecFrame {
    pub fn bind_local(&mut self, name: Symbol, value: Value) -> bool {
        self.scope.borrow_mut().declare_var(name, value)
    }

    pub fn local_values(&self) -> Vec<Value> {
        self.scope.borrow().vars.values().cloned().collect()
    }
}

#[derive(Debug, Clone)]
pub struct ImplicitBinding {
    pub ty: TypeId,
    pub value: Value,
}

/// A spec for creating an implicit binding at an entry point.
/// Evaluated on the heap via `call_or_construct` when the entry context is available.
#[derive(Debug, Clone)]
pub struct ImplicitInit {
    pub ty: TypeId,
    pub decl: DeclId,
}

/// Arguments and implicit bindings passed to a function call.
#[derive(Debug, Clone, Default)]
pub struct FuncInput {
    pub args: BTreeMap<Symbol, Value>,
    pub type_args: BTreeMap<Symbol, TypeId>,
    pub implicits: Vec<ImplicitBinding>,
}

/// A single frame in the call stack. Resolution to human-readable names
/// happens via `Registry::module_path(idx)` only on fault.
#[derive(Debug, Clone, Copy)]
pub struct StackFrame {
    pub module: crate::engine::ModuleIndex,
    pub file_idx: u16,
    pub call_node_id: Option<NodeId>,
    pub decl_node_id: NodeId,
}

pub type SharedCallStack = Rc<RefCell<Vec<StackFrame>>>;

/// Environment/data mismatch - user can fix via code or event surgery.
#[derive(Debug)]
pub struct FaultError {
    pub message: String,
    pub stack: Vec<StackFrame>,
}

/// Compiler/runtime bug - should never happen with correct parser/loader.
#[derive(Debug)]
pub struct InternalError {
    pub message: String,
    pub rust_location: String,
    pub backtrace: Box<std::backtrace::Backtrace>,
    pub stack: Vec<StackFrame>,
}

impl InternalError {
    #[track_caller]
    pub fn new(message: impl Into<String>) -> Self {
        let loc = std::panic::Location::caller();
        Self {
            message: message.into(),
            rust_location: format!("{}:{}", loc.file(), loc.line()),
            backtrace: Box::new(std::backtrace::Backtrace::capture()),
            stack: Vec::new(),
        }
    }
}
