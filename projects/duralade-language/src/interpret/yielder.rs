use std::cell::RefCell;
use std::rc::Rc;

use crate::model::Symbol;

use super::context::EvalContext;
use super::eval::{self, EvalResult};
use super::value::{HeapValue, NativeValue, Value, ValueId};
use crate::scope::ScopeKind;

/// Shared state between `exec_for` (In arm) and the yielder call.
/// Allocated on the heap as `HeapValue::Native(Box::new(RefCell::new(YielderChannel { .. })))`.
///
/// `RefCell<YielderChannel>` implements `NativeValue` with a no-op `gc_walk`
/// since it holds no `Value` references.
pub(crate) struct YielderChannel {
    /// The for body's loop variable name.
    var_name: Symbol,

    /// The for-in loop's label (for break/continue matching).
    label: Option<Symbol>,

    /// The for body's AST statements (cloned once from the for_stmt, shared via Rc).
    stmts: Rc<[crate::model::Stmt]>,

    /// Depth of scope_stack at the for-loop level.
    scope_depth: usize,

    /// Depth of call_stack at the for-loop level.
    call_stack_depth: usize,

    /// Set to true when the for-in loop has ended. A stale yielder call
    /// (e.g. from an escaped closure) returns `{ active: false }` cleanly.
    closed: bool,

    /// The yielder entity's heap id, set after entity construction.
    /// Used to update the entity's `active` field directly when the loop breaks.
    entity_id: Option<ValueId>,
}

impl YielderChannel {
    pub fn new(
        var_name: Symbol,
        label: Option<Symbol>,
        stmts: Rc<[crate::model::Stmt]>,
        scope_depth: usize,
        call_stack_depth: usize,
    ) -> Self {
        Self {
            var_name,
            label,
            stmts,
            scope_depth,
            call_stack_depth,
            closed: false,
            entity_id: None,
        }
    }

    /// Set the yielder entity's heap id after construction.
    pub fn set_entity_id(&mut self, id: ValueId) {
        self.entity_id = Some(id);
    }

    /// Mark the channel as closed. Subsequent yielder calls return `{ active: false }`.
    pub fn close(&mut self) {
        self.closed = true;
    }

    pub(crate) fn downcast(heap_value: &HeapValue) -> Option<&RefCell<Self>> {
        if let HeapValue::Native(native) = heap_value {
            (native.as_ref() as &dyn std::any::Any).downcast_ref::<RefCell<Self>>()
        } else {
            None
        }
    }
}

/// A validated handle to a `YielderChannel` on the heap.
/// Created by `NativeYielder::validate`, which proves the `ValueId` points
/// to a real `YielderChannel`. Call `exec_call` to run the for body.
pub struct NativeYielder {
    channel_id: ValueId,
}

impl NativeYielder {
    /// Validate that a heap value is a `YielderChannel`. Returns `None` if
    /// the value is not a native or not a `YielderChannel` (e.g. someone
    /// constructed a yielder entity manually with a bogus `native_yielder`).
    pub fn validate(heap_value: &HeapValue, channel_id: ValueId) -> Option<Self> {
        YielderChannel::downcast(heap_value).map(|_| NativeYielder { channel_id })
    }

    /// Execute a yielder call: save the iterator's state, run the for body
    /// with the yielded value, then restore the iterator's state.
    /// When the loop breaks, sets the yielder entity's `active` field to `false`.
    pub async fn exec_call(self, ctx: &mut EvalContext, yielded_value: Value) -> EvalResult {
        // Read channel state. We must drop the heap borrow before mutating ctx.
        let (var_name, label, stmts, scope_depth, call_stack_depth, closed, entity_id) = {
            let heap = ctx.heap();
            let Some(channel) = YielderChannel::downcast(heap.get(self.channel_id)) else {
                return eval::make_internal_error(
                    "NativeYielder: validated channel_id no longer valid".into(),
                    ctx.snapshot_stack(),
                );
            };
            let ch = channel.borrow();
            (
                ch.var_name.clone(),
                ch.label.clone(),
                ch.stmts.clone(), // Rc clone - cheap
                ch.scope_depth,
                ch.call_stack_depth,
                ch.closed,
                ch.entity_id,
            )
        };

        // Stale yielder: loop is done, entity's active is already false.
        if closed {
            return EvalResult::Ok(Value::Bool(false));
        }

        // Save iterator's execution state (everything above the for-loop level).
        let saved_iter = ctx.save_and_truncate(scope_depth, call_stack_depth);

        // Push a For scope and bind the loop variable.
        ctx.push_exec_frame(
            crate::scope::Scope {
                kind: ScopeKind::For,
                parent: Some(ctx.current_scope_ref()),
                ..Default::default()
            }
            .into_ref(),
        );
        if let Err(err) = ctx.declare_local(var_name, yielded_value) {
            ctx.pop_scope();
            ctx.restore_state(saved_iter);
            return err;
        }

        // Run the for body.
        let body_result = eval::exec_stmts(ctx, &stmts).await;

        // Pop the for scope.
        ctx.pop_scope();

        // Determine whether the iterator should continue.
        let should_continue = match &body_result {
            EvalResult::Ok(_) => true,
            EvalResult::Continue(l) if l.is_none() || *l == label => true,
            EvalResult::Break(l) if l.is_none() || *l == label => false,
            // Errors, return, or labeled break/continue for outer loops: propagate
            // after restoring iterator state.
            _ => {
                ctx.restore_state(saved_iter);
                return body_result;
            }
        };

        // Restore the iterator's execution state.
        ctx.restore_state(saved_iter);

        // When the loop breaks, update the yielder entity's `active` field.
        if !should_continue && let Some(eid) = entity_id {
            let mut heap = ctx.heap_mut();
            if let HeapValue::Entity { fields, .. } = heap.get_mut(eid) {
                fields.insert("active".into(), Value::Bool(false));
            }
        }

        EvalResult::Ok(Value::Bool(should_continue))
    }
}

impl NativeValue for RefCell<YielderChannel> {
    fn gc_walk(&self, _visitor: &mut dyn FnMut(&Value)) {}
}
