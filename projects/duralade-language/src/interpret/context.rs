use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap};
use std::rc::Rc;

use std::sync::Arc;

use crate::engine::{ExternOverride, ExternOverrides, ModuleIndex, native::NativeRegistry};
use crate::event::EventLog;
use crate::load::registry::TypeId;
use crate::load::registry::{DeclId, Registry};
use crate::load::{FieldModifier, TypeConstructKind};
use crate::model::{FieldKind, FieldVarModifier, NodeId, Symbol};

use super::scope::{ExecFrame, ImplicitBinding, RuntimeScopeRef, SharedCallStack, StackFrame};
use super::value::{Heap, HeapValue, HeapValueFuncKind, Value};
use crate::scope;

/// Saved execution state for scope/call-stack swapping.
/// Used by the yielder to park the iterator's frames while running the for body.
pub(crate) struct SavedExecState {
    pub frames: Vec<ExecFrame>,
    pub scope: RuntimeScopeRef,
    pub import_ctx: Vec<ModuleIndex>,
    pub call_stack: Vec<StackFrame>,
}

/// Scheduler-side handle to wake a waiting coroutine. Calling `wake()` causes
/// the coroutine's `WaitForTrigger` future to resolve on its next poll.
#[derive(Debug, Clone)]
pub struct WaitTrigger(Rc<Cell<bool>>);

impl WaitTrigger {
    pub fn wake(&self) {
        self.0.set(true);
    }
}

/// Future that resolves only when the paired `WaitTrigger::wake()` is called.
/// Reusable: after resolving, resets so a new `.await` blocks again until
/// the next `wake()`.
pub struct WaitForTrigger(Rc<Cell<bool>>);

impl std::future::Future for WaitForTrigger {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        _cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<()> {
        if self.0.get() {
            self.0.set(false);
            std::task::Poll::Ready(())
        } else {
            std::task::Poll::Pending
        }
    }
}

/// Create a linked trigger/future pair.
pub fn wait_trigger() -> (WaitTrigger, Rc<Cell<bool>>) {
    let flag = Rc::new(Cell::new(false));
    (WaitTrigger(flag.clone()), flag)
}

/// Construct a `WaitForTrigger` future from the shared flag.
/// Each `.await` on a new instance blocks until the next `wake()`.
pub fn wait_for_trigger(flag: &Rc<Cell<bool>>) -> WaitForTrigger {
    WaitForTrigger(flag.clone())
}

/// A registered wait entry pushed by `eval_wait`, drained by the scheduler.
#[derive(Debug)]
pub struct WaitEntry {
    /// Scheduler uses this to signal "evaluate your condition now".
    pub trigger: WaitTrigger,
    /// Coroutine sets this to true when the condition was met.
    pub resolved: Rc<Cell<bool>>,
    /// Identifies the owning coroutine so the scheduler can poll just that one.
    pub coroutine_id: u64,
}

/// Shared queue where eval_wait pushes new entries. The scheduler drains this
/// after each poll to discover which coroutine registered new waits.
pub type WaitQueue = Rc<RefCell<Vec<WaitEntry>>>;

pub struct EvalContext {
    /// The loaded program - static, not part of per-tick state.
    pub registry: Arc<Registry>,

    /// Value heap for all non-primitive values (shared across coroutines).
    heap: Rc<RefCell<Heap>>,

    /// The execution stack. Bottom = outermost function call.
    pub exec_stack: Vec<ExecFrame>,
    scope: RuntimeScopeRef,

    /// Shared event log for replay and new event recording.
    /// Rebuilt from scratch each tick via replay.
    event_log: Rc<RefCell<EventLog>>,

    /// Stack of module indices whose imports are currently active.
    import_context_stack: Vec<ModuleIndex>,

    /// Shared wait queue for registering new waits. The cooperative scheduler
    /// drains this after each poll to discover new wait entries.
    wait_queue: WaitQueue,

    // TODO: call_stack and current_coroutine_id are per-coroutine state that
    // lives here temporarily because each coroutine has its own EvalContext.
    // When contexts are shared, extract both into a proper per-coroutine
    // context threaded through eval functions.
    /// Duralade call stack - pushed/popped on Function scope entry/exit.
    /// Shared with the Coroutine so the engine can inspect stacks.
    call_stack: SharedCallStack,

    current_coroutine_id: u64,

    native_registry: Rc<NativeRegistry>,
    extern_overrides: ExternOverrides,
    /// Cached module and file scopes, keyed by module index.
    /// File scopes parent to their module scope.
    module_scopes: HashMap<ModuleIndex, RuntimeScopeRef>,
    file_scopes: HashMap<(ModuleIndex, u16), RuntimeScopeRef>,
}

impl EvalContext {
    pub fn new(registry: Arc<Registry>) -> Self {
        let root = scope::Scope {
            kind: scope::ScopeKind::Module,
            ..Default::default()
        }
        .into_ref();
        Self {
            registry,
            heap: Rc::new(RefCell::new(Heap::default())),
            exec_stack: Vec::new(),
            scope: root,
            event_log: Rc::new(RefCell::new(EventLog::default())),
            import_context_stack: Vec::new(),
            wait_queue: Rc::new(RefCell::new(Vec::new())),
            call_stack: Rc::new(RefCell::new(Vec::new())),
            current_coroutine_id: 0,
            native_registry: Rc::new(NativeRegistry::new()),
            extern_overrides: Rc::new(RefCell::new(BTreeMap::new())),
            module_scopes: HashMap::new(),
            file_scopes: HashMap::new(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn with_shared(
        registry: Arc<Registry>,
        heap: Rc<RefCell<Heap>>,
        event_log: Rc<RefCell<EventLog>>,
        wait_queue: WaitQueue,
        call_stack: SharedCallStack,
        coroutine_id: u64,
        native_registry: Rc<NativeRegistry>,
        extern_overrides: ExternOverrides,
    ) -> Self {
        let root = scope::Scope {
            kind: scope::ScopeKind::Module,
            ..Default::default()
        }
        .into_ref();
        Self {
            registry,
            heap,
            exec_stack: Vec::new(),
            scope: root,
            event_log,
            import_context_stack: Vec::new(),
            wait_queue,
            call_stack,
            current_coroutine_id: coroutine_id,
            native_registry,
            extern_overrides,
            module_scopes: HashMap::new(),
            file_scopes: HashMap::new(),
        }
    }

    /// Get or create the file scope where a declaration lives.
    /// File scope parents to its module scope.
    pub fn definition_scope(&mut self, module_idx: ModuleIndex, file_idx: u16) -> RuntimeScopeRef {
        let key = (module_idx, file_idx);
        if let Some(s) = self.file_scopes.get(&key) {
            return s.clone();
        }
        let module_scope = self
            .module_scopes
            .entry(module_idx)
            .or_insert_with(|| {
                scope::Scope {
                    kind: scope::ScopeKind::Module,
                    name: Some(self.registry.module_path(module_idx).clone()),
                    ..Default::default()
                }
                .into_ref()
            })
            .clone();
        let file_scope = scope::Scope {
            kind: scope::ScopeKind::File,
            parent: Some(module_scope),
            ..Default::default()
        }
        .into_ref();
        self.file_scopes.insert(key, file_scope.clone());
        file_scope
    }

    pub fn heap(&self) -> std::cell::Ref<'_, Heap> {
        self.heap.borrow()
    }

    pub fn heap_mut(&self) -> std::cell::RefMut<'_, Heap> {
        self.heap.borrow_mut()
    }

    /// Resolve any value to its declaration's DeclId.
    /// Works for heap-backed values (Entity) via their stored DeclId,
    /// and for scalars via well-known registry fields.
    pub fn value_decl_id(&self, value: &Value) -> Option<DeclId> {
        match value {
            Value::Data(id) => {
                let heap = self.heap();
                if let HeapValue::Data { decl, .. } = heap.get(id.id()) {
                    *decl
                } else {
                    None
                }
            }
            Value::Entity(id) => {
                let heap = self.heap();
                if let HeapValue::Entity { decl, .. } = heap.get(id.id()) {
                    Some(*decl)
                } else {
                    None
                }
            }
            Value::Int(_) => Some(self.registry.int_decl),
            Value::Float(_) => Some(self.registry.float_decl),
            Value::Str(_) => Some(self.registry.str_decl),
            Value::Bool(_) => Some(self.registry.bool_decl),
            _ => None,
        }
    }

    pub fn heap_rc(&self) -> &Rc<RefCell<Heap>> {
        &self.heap
    }

    /// Get a reference to the shared event log Rc (for tick driver to clone).
    pub fn event_log(&self) -> &Rc<RefCell<EventLog>> {
        &self.event_log
    }

    pub fn wait_queue(&self) -> &WaitQueue {
        &self.wait_queue
    }

    pub fn current_coroutine_id(&self) -> u64 {
        self.current_coroutine_id
    }

    pub fn lookup(&self, name: &str) -> super::eval::EvalResult {
        let sym: Symbol = name.into();
        match self.resolve_name(&sym) {
            Some(v) => super::eval::EvalResult::Ok(v),
            None => super::eval::make_internal_error(
                format!("undefined variable: {name}"),
                self.snapshot_stack(),
            ),
        }
    }

    /// Declare a new local variable in the current scope.
    /// Shadows any outer variable with the same name.
    /// Returns Err if the name already exists in the current scope.
    pub fn declare_local(&self, name: Symbol, value: Value) -> Result<(), super::eval::EvalResult> {
        if self.scope.borrow_mut().declare_var(name.clone(), value) {
            Ok(())
        } else {
            Err(super::eval::make_internal_error(
                format!("variable '{name}' already declared in this scope"),
                self.snapshot_stack(),
            ))
        }
    }

    pub fn resolve_name(&self, name: &Symbol) -> Option<Value> {
        let heap = &self.heap;
        let registry = &self.registry;
        self.scope
            .borrow()
            .resolve_var(name, &|cs, n| cs.resolve(n, registry, heap))
    }

    /// Look up an implicit value by exact type match, walking scopes top-down.
    /// For nilable types (`foo?`), searches for the inner type (`foo`) and
    /// returns `Some(Nil)` when not found (nilable implicits are always satisfied).
    pub fn lookup_implicit(&self, ty: TypeId) -> Option<Value> {
        let (search_ty, nilable) = self
            .registry
            .nilable_inner(ty)
            .map_or((ty, false), |inner| (inner, true));
        for frame in self.exec_stack.iter().rev() {
            for binding in frame.implicits.iter().rev() {
                if binding.ty == search_ty {
                    return Some(binding.value.clone());
                }
            }
        }
        if nilable { Some(Value::Nil) } else { None }
    }

    /// Set an existing variable (`=` assignment). Walks the scope chain.
    /// Returns an error if the variable doesn't exist.
    pub fn set_existing(&mut self, name: &Symbol, value: Value) -> super::eval::EvalResult {
        let heap = &self.heap;
        if self
            .scope
            .borrow_mut()
            .set_var(name, value, &|cs, n, v| cs.set(n, v, heap))
        {
            super::eval::EvalResult::Ok(Value::Nil)
        } else {
            super::eval::make_internal_error(
                format!("undefined variable: {name}"),
                self.snapshot_stack(),
            )
        }
    }

    /// Push an execution frame for the given scope.
    pub fn push_exec_frame(&mut self, new_scope: RuntimeScopeRef) {
        let restore = self.scope.clone();
        self.scope = new_scope.clone();
        self.exec_stack.push(ExecFrame {
            scope: new_scope,
            restore_scope: restore,
            stmt_index: 0,
            implicits: Vec::new(),
            deferred: Vec::new(),
        });
    }

    /// Push a Function scope and its corresponding call stack frame.
    pub fn push_function_frame(
        &mut self,
        new_scope: RuntimeScopeRef,
        frame: StackFrame,
        implicits: Vec<ImplicitBinding>,
    ) {
        self.push_exec_frame(new_scope);
        if let Some(f) = self.exec_stack.last_mut() {
            f.implicits = implicits;
        }
        self.call_stack.borrow_mut().push(frame);
    }

    /// Pop the topmost execution frame.
    /// If it's a Function/Closure scope, also pops the call stack.
    pub fn pop_scope(&mut self) {
        if let Some(frame) = self.exec_stack.pop() {
            let kind = frame.scope.borrow().kind;
            if kind == crate::scope::ScopeKind::Function {
                self.call_stack.borrow_mut().pop();
            }
            self.scope = frame.restore_scope;
        }
    }

    /// Save all execution state above the given depths and truncate.
    /// Returns the saved state for later restoration via `restore_state`.
    /// Used by the yielder to park the iterator's frames while running the for body.
    pub(crate) fn save_and_truncate(
        &mut self,
        scope_depth: usize,
        call_stack_depth: usize,
    ) -> SavedExecState {
        let saved_frames = self.exec_stack.split_off(scope_depth);
        let saved_scope = self.scope.clone();

        if let Some(frame) = self.exec_stack.last() {
            self.scope = frame.scope.clone();
        }

        let saved_import_ctx = self
            .import_context_stack
            .split_off(self.import_context_stack.len().min(scope_depth));

        let mut cs = self.call_stack.borrow_mut();
        let saved_call_stack = cs.split_off(call_stack_depth);
        drop(cs);

        SavedExecState {
            frames: saved_frames,
            scope: saved_scope,
            import_ctx: saved_import_ctx,
            call_stack: saved_call_stack,
        }
    }

    pub(crate) fn restore_state(&mut self, saved: SavedExecState) {
        self.exec_stack.extend(saved.frames);
        self.scope = saved.scope;
        self.import_context_stack.extend(saved.import_ctx);
        self.call_stack.borrow_mut().extend(saved.call_stack);
    }

    /// Clone the current call stack for fault/error capture.
    pub fn snapshot_stack(&self) -> Vec<StackFrame> {
        self.call_stack.borrow().clone()
    }

    pub fn call_stack_rc(&self) -> &SharedCallStack {
        &self.call_stack
    }

    pub(crate) fn call_stack_len(&self) -> usize {
        self.call_stack.borrow().len()
    }

    /// Resolve an import alias (e.g. "assert") to a ModuleIndex using the
    /// per-file import map. The current file is determined from the call stack.
    pub fn resolve_import(&self, alias: &str) -> Option<ModuleIndex> {
        let &current_idx = self.import_context_stack.last()?;
        let module = self.registry.module(current_idx);
        let file_idx = self
            .call_stack
            .borrow()
            .last()
            .map(|f| f.file_idx as usize)
            .unwrap_or(0);
        let file = module.files.get(file_idx)?;
        let path = file.import_map.get(alias)?;
        self.registry.module_index(path)
    }

    pub fn enclosing_out_early_name(&self) -> Option<Symbol> {
        self.scope
            .borrow()
            .enclosing_out_early()
            .map(|(name, _)| name)
    }

    pub fn set_enclosing_out_early(&self, value: Value) -> Option<Symbol> {
        self.scope.borrow_mut().set_enclosing_out_early(value)
    }

    pub fn current_module_index(&self) -> Option<ModuleIndex> {
        self.import_context_stack.last().copied()
    }

    pub fn push_import_context(&mut self, idx: ModuleIndex) {
        self.import_context_stack.push(idx);
    }

    pub fn pop_import_context(&mut self) {
        self.import_context_stack.pop();
    }

    pub fn current_scope_ref(&self) -> RuntimeScopeRef {
        self.scope.clone()
    }

    /// Look up the resolved target type for an `as` narrowing expression.
    /// The implementation is a generic node_types lookup, but not all expression
    /// types are stored at runtime — only runtime-required ones. Named narrowly
    /// to discourage callers from assuming arbitrary node types are available.
    pub fn lookup_as_narrowing_type(&self, node_id: NodeId) -> Option<TypeId> {
        let module_idx = self.current_module_index()?;
        let module = self.registry.module(module_idx);
        module
            .files
            .iter()
            .find_map(|f| f.type_table.node_types.get(&node_id).copied())
    }

    pub fn resolve_type_param(&self, name: &Symbol) -> Option<TypeId> {
        self.scope.borrow().resolve_type_param(name)
    }

    pub fn push_deferred(&mut self, func_id: super::value::RootedId) {
        for frame in self.exec_stack.iter_mut().rev() {
            if frame.scope.borrow().kind == crate::scope::ScopeKind::Function {
                frame.deferred.push(func_id);
                return;
            }
        }
    }

    pub fn take_deferred(&mut self) -> Vec<super::value::RootedId> {
        if let Some(frame) = self.exec_stack.last_mut()
            && frame.scope.borrow().kind == crate::scope::ScopeKind::Function
        {
            return std::mem::take(&mut frame.deferred);
        }
        Vec::new()
    }

    pub fn native_registry(&self) -> &Rc<NativeRegistry> {
        &self.native_registry
    }

    pub fn set_extern_override(&self, full_name: String, func: Value) -> Result<(), String> {
        let Value::Func(ref rooted) = func else {
            return Err("extern override must be a function value".into());
        };
        let func_id = rooted.id();

        let Some((module_part, decl_name_str)) = full_name.split_once("::") else {
            return Err(format!("invalid extern name '{}'", full_name));
        };
        let Some(extern_module_idx) = self.registry.module_index(module_part) else {
            return Err(format!(
                "override target extern '{}' does not exist",
                full_name
            ));
        };
        let extern_module = self.registry.module(extern_module_idx);
        let Some(extern_decl) = extern_module.get_declaration(decl_name_str) else {
            return Err(format!(
                "override target extern '{}' does not exist",
                full_name
            ));
        };
        if extern_decl.kind != TypeConstructKind::Extern {
            return Err(format!("override target '{}' is not an extern", full_name));
        }

        let (anon_func, override_module_idx) = {
            let heap = self.heap.borrow();
            match heap.get(func_id) {
                HeapValue::Func {
                    module,
                    kind: HeapValueFuncKind::Closure { body, .. },
                } => (body.clone(), *module),
                HeapValue::Func { .. } => {
                    return Err(
                        "extern override currently requires anonymous function literal".into(),
                    );
                }
                _ => return Err("extern override must be a function value".into()),
            }
        };
        let override_module = self.registry.module(override_module_idx);

        let mut actual_fields: Vec<(Symbol, FieldModifier, TypeId)> = Vec::new();
        for field in &anon_func.fields {
            let FieldKind::Var(var) = &field.kind else {
                continue;
            };
            let modifier = match var.modifier {
                FieldVarModifier::In => FieldModifier::In,
                FieldVarModifier::Out => FieldModifier::Out,
                FieldVarModifier::OutEarly => FieldModifier::OutEarly,
                FieldVarModifier::Inout => FieldModifier::Inout,
                FieldVarModifier::Value | FieldVarModifier::Implicit => {
                    return Err(format!(
                        "override function field '{}' has unsupported modifier",
                        var.name.name
                    ));
                }
            };
            let Some(ty_node) = &var.ty else {
                return Err(format!(
                    "override function field '{}' must have explicit type",
                    var.name.name
                ));
            };
            let Some(resolved_ty) = override_module
                .files
                .iter()
                .find_map(|f| f.type_table.node_types.get(&ty_node.node_id()).cloned())
            else {
                return Err(format!(
                    "cannot resolve type for override function field '{}'",
                    var.name.name
                ));
            };
            actual_fields.push((var.name.name.clone(), modifier, resolved_ty));
        }

        let expected_fields: Vec<(Symbol, FieldModifier, TypeId)> = extern_decl
            .fields
            .iter()
            .map(|f| {
                (
                    f.name.clone(),
                    f.modifier.unwrap_or(FieldModifier::In),
                    f.ty,
                )
            })
            .collect();

        // Compare by name+modifier+type regardless of field order.
        let mut actual_sorted = actual_fields.clone();
        let mut expected_sorted = expected_fields.clone();
        actual_sorted.sort_by(|a, b| a.0.cmp(&b.0));
        expected_sorted.sort_by(|a, b| a.0.cmp(&b.0));
        if actual_sorted != expected_sorted {
            let expected = expected_fields
                .iter()
                .map(|(name, modifier, ty)| {
                    format!("{modifier:?} {name}: {}", self.registry.display_type(*ty))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let actual = actual_fields
                .iter()
                .map(|(name, modifier, ty)| {
                    format!("{modifier:?} {name}: {}", self.registry.display_type(*ty))
                })
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "override function fields must exactly match extern '{}': expected [{}], got [{}]",
                full_name, expected, actual
            ));
        }

        let target_decl = self
            .registry
            .decl_id_in_module(extern_module_idx, decl_name_str)
            .expect("extern declaration not registered");

        self.extern_overrides.borrow_mut().insert(
            full_name,
            ExternOverride {
                heap: self.heap.clone(),
                func_value: func,
                registry: self.registry.clone(),
                native_registry: self.native_registry.clone(),
                extern_overrides: self.extern_overrides.clone(),
                target_decl,
            },
        );
        Ok(())
    }
}
