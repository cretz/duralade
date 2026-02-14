pub mod cache;
pub mod native;

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::Arc;
use std::task::{Context, Poll};

use crate::engine::native::NativeRegistry;
use crate::event::{Event, EventLog, EventType, OwnedFields, OwnedValue};
use crate::interpret::context::{EvalContext, WaitQueue};
use crate::interpret::scope::{
    FaultError, ImplicitInit, InternalError, SharedCallStack, StackFrame,
};
use crate::interpret::tick::{self, TickOutcome, TickStatus};
use crate::interpret::value::{Heap, RootedId, ValueId};
use crate::load::registry::Registry;
use crate::load::{FieldModifier, TypeConstructKind};
use crate::model::Symbol;

/// Shared map of extern overrides, propagated from parent to child on spawn.
pub type ExternOverrides = Rc<RefCell<BTreeMap<String, ExternOverride>>>;

/// Structured error from `EntityInstance::tick()`.
#[derive(Debug)]
pub enum TickError {
    /// Entity is completed or terminated - cannot tick.
    NotRunning,
    /// A previous tick faulted or hit an internal error. Reconstruct from
    /// events to retry.
    AlreadyFaulted,
    /// Environment/data mismatch - user can fix via code or event surgery.
    Fault {
        error: FaultError,
        /// Events emitted before the fault (for diagnostics - do not persist).
        new_event_count: usize,
    },
    /// Compiler/runtime bug.
    InternalError {
        error: InternalError,
        /// Events emitted before the error (for diagnostics - do not persist).
        new_event_count: usize,
    },
}

/// Error type for all `Engine` trait methods.
#[derive(Debug)]
pub enum EngineError {
    /// Entity with the given ID does not exist.
    EntityNotFound { id: String },
    /// Entity with the given ID already exists.
    EntityAlreadyExists { id: String },
    /// Entity is completed or terminated - cannot tick/complete/invoke.
    NotRunning { id: String },
    /// A previous tick faulted. Reconstruct from events to retry.
    AlreadyFaulted { id: String },
    /// No unmatched ExternInvoke to complete.
    NoPendingExtern { id: String },
    /// Environment/data mismatch - user can fix via code or event surgery.
    Fault {
        error: FaultError,
        new_events: Vec<Event>,
    },
    /// Compiler/runtime bug.
    InternalError {
        error: InternalError,
        new_events: Vec<Event>,
    },
    /// Implementation-specific error. Different engine implementations may
    /// use concrete types here for downcasting.
    Other(Box<dyn std::error::Error>),
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EngineError::EntityNotFound { id } => write!(f, "entity not found: {id}"),
            EngineError::EntityAlreadyExists { id } => {
                write!(f, "entity already exists: {id}")
            }
            EngineError::NotRunning { id } => write!(f, "entity is not running: {id}"),
            EngineError::AlreadyFaulted { id } => write!(f, "entity already faulted: {id}"),
            EngineError::NoPendingExtern { id } => {
                write!(f, "no pending extern invocation: {id}")
            }
            EngineError::Fault { error, .. } => write!(f, "fault: {}", error.message),
            EngineError::InternalError { error, .. } => {
                write!(f, "internal error: {}", error.message)
            }
            EngineError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl TickError {
    /// Convert to `EngineError`, materializing new events from the entity's event log.
    pub fn into_engine_error(self, events: &[Event]) -> EngineError {
        match self {
            TickError::NotRunning => EngineError::NotRunning { id: String::new() },
            TickError::AlreadyFaulted => EngineError::AlreadyFaulted { id: String::new() },
            TickError::Fault {
                error,
                new_event_count,
            } => EngineError::Fault {
                error,
                new_events: events[events.len() - new_event_count..].to_vec(),
            },
            TickError::InternalError {
                error,
                new_event_count,
            } => EngineError::InternalError {
                error,
                new_events: events[events.len() - new_event_count..].to_vec(),
            },
        }
    }
}

impl From<String> for EngineError {
    fn from(s: String) -> Self {
        EngineError::Other(s.into())
    }
}

impl From<&str> for EngineError {
    fn from(s: &str) -> Self {
        EngineError::Other(s.into())
    }
}

impl EngineError {
    pub fn other(msg: impl Into<Box<dyn std::error::Error>>) -> Self {
        EngineError::Other(msg.into())
    }
}

/// Dense index into the Registry's module list.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ModuleIndex(pub u32);

#[derive(Debug, Clone)]
pub enum EntityStatus {
    Running,
    Completed { result: Option<OwnedFields> },
    Terminated,
}

pub struct EntityTickResult {
    pub status: TickStatus,
    /// Number of new events appended to the entity's event log during this tick.
    /// Access via `entity.events()[entity.events().len() - new_event_count..]`.
    pub new_event_count: usize,
}

pub struct EngineTickResult {
    pub status: TickStatus,
    pub new_events: Vec<Event>,
    /// IDs of child entities created by system extern routing (e.g. spawn).
    pub spawned: Vec<String>,
}

/// A resolved stack frame with human-readable names.
#[derive(Debug, Clone, serde::Serialize)]
pub struct ResolvedFrame {
    pub module: String,
    pub construct: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub member: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

/// A coroutine's identity and resolved call stack.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CoroutineInfo {
    pub id: u64,
    pub source_event: Option<u64>,
    pub stack: Vec<ResolvedFrame>,
}

/// Wrapper for view/inspect results that includes the last event num replayed.
#[derive(Debug, Clone)]
pub struct ViewResult<T> {
    pub last_event_num: u64,
    pub value: T,
}

pub trait Engine {
    fn spawn_entity(
        &mut self,
        id: String,
        entity_type: String,
        args: OwnedFields,
        implicits: Vec<ImplicitInit>,
        time_ms: u64,
        no_tick: bool,
    ) -> impl Future<Output = Result<EngineTickResult, EngineError>>;

    fn tick_entity(
        &mut self,
        id: &str,
        time_ms: u64,
    ) -> impl Future<Output = Result<EngineTickResult, EngineError>>;

    fn tick_all(
        &mut self,
        time_ms: u64,
    ) -> impl Future<Output = Result<Vec<EngineTickResult>, EngineError>>;

    fn complete_extern(
        &mut self,
        id: &str,
        invoke_num: u64,
        result: OwnedFields,
        time_ms: u64,
    ) -> impl Future<Output = Result<(), EngineError>>;

    // TODO: discuss how implicits interact with event logs before adding them here
    fn invoke_func(
        &mut self,
        id: &str,
        func: &str,
        request_id: Option<String>,
        args: OwnedFields,
        time_ms: u64,
    ) -> impl Future<Output = Result<u64, EngineError>>;

    /// Invoke a view member function, tick, and return its result.
    /// Verifies the function has the `view` modifier. View functions have
    /// no side effects and are guaranteed to complete synchronously (no externs).
    fn invoke_view_func(
        &self,
        id: &str,
        func: &str,
        args: OwnedFields,
        implicits: Vec<ImplicitInit>,
        after_event_num: Option<u64>,
    ) -> impl Future<Output = Result<ViewResult<OwnedFields>, EngineError>>;

    fn cancel_entity(
        &mut self,
        id: &str,
        reason: Option<String>,
        time_ms: u64,
    ) -> impl Future<Output = Result<(), EngineError>>;

    fn cancel_func(
        &mut self,
        id: &str,
        request_id: &str,
        time_ms: u64,
    ) -> impl Future<Output = Result<(), EngineError>>;

    fn terminate_entity(
        &mut self,
        id: &str,
        time_ms: u64,
    ) -> impl Future<Output = Result<(), EngineError>>;

    fn entity_events(&self, id: &str) -> impl Future<Output = Result<Vec<Event>, EngineError>>;

    fn entity_ids(&self) -> impl Future<Output = Result<Vec<String>, EngineError>>;

    fn entity_exists(&self, id: &str) -> impl Future<Output = Result<bool, EngineError>>;

    fn entity_status(&self, id: &str) -> impl Future<Output = Result<EntityStatus, EngineError>>;

    /// Replay entity from events (read-only diagnostic). Returns the tick status
    /// after replay without persisting anything.
    fn replay_entity(
        &self,
        id: &str,
        time_ms: u64,
    ) -> impl Future<Output = Result<TickStatus, EngineError>>;

    /// Read all entity out fields after replaying to a given point.
    fn view_out_fields(
        &self,
        id: &str,
        after_event_num: Option<u64>,
    ) -> impl Future<Output = Result<ViewResult<OwnedFields>, EngineError>>;

    /// Read a single entity out field after replaying to a given point.
    fn view_out_field(
        &self,
        id: &str,
        field: &str,
        after_event_num: Option<u64>,
    ) -> impl Future<Output = Result<ViewResult<Option<OwnedValue>>, EngineError>>;

    /// Get resolved call stacks of active coroutines.
    /// With after_event_num: truncate events, tick, then inspect.
    /// Without: full tick, then inspect.
    fn inspect_stacks(
        &self,
        id: &str,
        after_event_num: Option<u64>,
    ) -> impl Future<Output = Result<ViewResult<Vec<CoroutineInfo>>, EngineError>>;

    /// Flush any buffered state. Must be called after mutating operations.
    fn finalize(&self) -> impl Future<Output = Result<(), EngineError>>;
}

struct Coroutine {
    id: u64,
    /// Links back to the event that spawned this coroutine (e.g. FuncInvoke).
    /// None for the run coroutine and internally-spawned coroutines.
    source_event_num: Option<u64>,
    future: Option<Pin<Box<dyn Future<Output = TickOutcome>>>>,
    call_stack: SharedCallStack,
}

struct HeapCollectGuard {
    heap: Rc<RefCell<Heap>>,
    disabled: bool,
}

impl Drop for HeapCollectGuard {
    fn drop(&mut self) {
        if !self.disabled {
            self.heap.borrow_mut().gc_collect();
        }
    }
}

pub struct EntitySpawnOptions {
    pub registry: Arc<Registry>,
    pub id: String,
    pub entity_type: String,
    pub args: OwnedFields,
    pub initial_entry_implicits: Vec<ImplicitInit>,
    pub native_registry: Rc<NativeRegistry>,
    pub extern_overrides: Option<ExternOverrides>,
    pub time_ms: u64,
    /// Set to true for single-use instances that will be dropped immediately
    /// after use (e.g. CLI `run` command). Skips heap collection after each
    /// externally-invoked call. Leave false for long-lived instances (shell,
    /// server) to reclaim memory between calls.
    pub disable_heap_collect: bool,
}

pub struct EntityFromEventsOptions {
    pub registry: Arc<Registry>,
    pub events: Vec<Event>,
    pub native_registry: Rc<NativeRegistry>,
    pub until_event_num: Option<u64>,
    /// Set to true for single-use instances that will be dropped immediately
    /// after use (e.g. CLI `run` command). Skips heap collection after each
    /// externally-invoked call. Leave false for long-lived instances (shell,
    /// server) to reclaim memory between calls.
    pub disable_heap_collect: bool,
}

/// In-memory handle for an entity's interpreter and event log.
/// All operations needed by a CLI or REPL shell (engine-spec §4.3).
pub struct EntityInstance {
    events: Vec<Event>,
    next_event_num: u64,
    next_coroutine_id: u64,
    id: String,
    entity_type: String,
    entry_module_idx: ModuleIndex,
    entity_name: Symbol,
    registry: Arc<Registry>,
    heap: Rc<RefCell<Heap>>,
    entity_value_id: RootedId,
    wait_queue: WaitQueue,
    event_log: Rc<RefCell<EventLog>>,
    native_registry: Rc<NativeRegistry>,
    extern_overrides: ExternOverrides,
    coroutines: Vec<Coroutine>,
    primary_id: u64,
    /// Number of execution events (ExternInvoke + ExternComplete) loaded from
    /// the persisted event log. Used for replay divergence checks and `is_replaying()`.
    /// Zero for freshly spawned entities.
    recorded_exec_count: usize,
    /// True if the event log contains an EntityComplete event.
    has_complete_event: bool,
    /// Set on Fault or InternalError. Prevents further tick() calls - the
    /// instance must be reconstructed from events to retry.
    faulted: bool,
    /// When true, tick returns immediately after consuming all replay events
    /// instead of continuing execution past the replay boundary.
    replay_only: bool,
    disable_heap_collect: bool,
}

impl EntityInstance {
    fn heap_collect_guard(&self) -> HeapCollectGuard {
        HeapCollectGuard {
            heap: self.heap.clone(),
            disabled: self.disable_heap_collect,
        }
    }

    /// Create a new entity and activate it immediately.
    #[tracing::instrument(level = "info", skip(opts), fields(id = %opts.id, entity_type = %opts.entity_type))]
    pub fn spawn(opts: EntitySpawnOptions) -> Result<EntityInstance, String> {
        let EntitySpawnOptions {
            registry,
            id,
            entity_type,
            args,
            initial_entry_implicits,
            native_registry,
            extern_overrides,
            time_ms,
            disable_heap_collect,
        } = opts;
        let (entry_module_path, entity_name) = parse_entity_type(&entity_type)?;
        let entry_module_idx = registry
            .module_index(&entry_module_path)
            .ok_or_else(|| format!("module not found: {entry_module_path}"))?;

        let event = Event {
            num: 0,
            time: time_ms,
            event_type: EventType::EntityInvoke {
                id: id.clone(),
                entity: entity_type.clone(),
                args: args.clone(),
            },
        };

        let heap = Rc::new(RefCell::new(Heap::default()));

        // Allocate entity on heap with empty fields - setup_entity_scope
        // will populate via construct_value_id.
        let entity_decl_id = registry
            .decl_id_in_module(entry_module_idx, &entity_name)
            .expect("entity declaration not registered");
        let entity_value_id = {
            let mut h = heap.borrow_mut();
            h.alloc(crate::interpret::value::HeapValue::Entity {
                decl: entity_decl_id,
                fields: BTreeMap::new(),
            })
        };

        let wait_queue: WaitQueue = Rc::new(RefCell::new(Vec::new()));
        let event_log = Rc::new(RefCell::new(EventLog::from_events(Vec::new(), 1)));
        event_log.borrow_mut().current_time_ms = time_ms;
        let extern_overrides =
            extern_overrides.unwrap_or_else(|| Rc::new(RefCell::new(BTreeMap::new())));
        let primary_id: u64 = 0;
        let call_stack: SharedCallStack = Rc::new(RefCell::new(Vec::new()));
        let future = build_primary(
            &registry,
            entry_module_idx,
            &entity_name,
            args,
            initial_entry_implicits,
            event_log.clone(),
            heap.clone(),
            entity_value_id.id(),
            wait_queue.clone(),
            call_stack.clone(),
            primary_id,
            native_registry.clone(),
            extern_overrides.clone(),
        )?;

        Ok(EntityInstance {
            events: vec![event],
            next_event_num: 1,
            next_coroutine_id: 1,
            id,
            entity_type,
            entry_module_idx,
            entity_name,
            registry,
            heap,
            entity_value_id,
            wait_queue,
            event_log,
            native_registry,
            extern_overrides,
            coroutines: vec![Coroutine {
                id: primary_id,
                source_event_num: None,
                future: Some(future),
                call_stack,
            }],
            primary_id,
            has_complete_event: false,
            recorded_exec_count: 0,
            faulted: false,
            replay_only: false,
            disable_heap_collect,
        })
    }

    /// Reconstruct from a persisted event log. Builds the interpreter future
    /// and loads execution events for replay, but does not poll - replay happens
    /// on the first `tick()` call.
    ///
    /// When `until_event_num` is `Some(n)`, only events with `num <= n` are
    /// included - the entity replays up to that point and stops, as if later
    /// events never happened.
    #[tracing::instrument(level = "debug", skip_all, fields(event_count = opts.events.len()))]
    pub fn from_events(opts: EntityFromEventsOptions) -> Result<EntityInstance, String> {
        let EntityFromEventsOptions {
            registry,
            mut events,
            native_registry,
            until_event_num,
            disable_heap_collect,
        } = opts;
        if events.is_empty() {
            return Err("event log is empty".into());
        }

        if let Some(until) = until_event_num {
            events.retain(|e| e.num <= until);
            if events.is_empty() {
                return Err("no events at or before the given event number".into());
            }
        }

        let (id, entity_type, entity_args) = match &events[0].event_type {
            EventType::EntityInvoke { id, entity, args } => {
                (id.clone(), entity.clone(), args.clone())
            }
            _ => return Err("first event must be EntityInvoke".into()),
        };
        let (entry_module_path, entity_name) = parse_entity_type(&entity_type)?;
        let next_event_num = events.last().unwrap().num + 1;

        let entry_module_idx = registry
            .module_index(&entry_module_path)
            .ok_or_else(|| format!("module not found: {entry_module_path}"))?;

        // Single pass: filter execution events for replay and check for EntityComplete.
        let mut exec_events: Vec<Event> = Vec::new();
        let mut has_complete_event = false;
        for event in &events {
            match &event.event_type {
                EventType::ExternInvoke { .. } | EventType::ExternComplete { .. } => {
                    exec_events.push(event.clone());
                }
                EventType::EntityComplete { .. } => {
                    has_complete_event = true;
                }
                _ => {}
            }
        }

        let recorded_exec_count = exec_events.len();
        let event_log = Rc::new(RefCell::new(EventLog::from_events(
            exec_events,
            next_event_num,
        )));

        let heap = Rc::new(RefCell::new(Heap::default()));
        let entity_decl_id = registry
            .decl_id_in_module(entry_module_idx, &entity_name)
            .expect("entity declaration not registered");
        let entity_value_id = {
            let mut h = heap.borrow_mut();
            h.alloc(crate::interpret::value::HeapValue::Entity {
                decl: entity_decl_id,
                fields: BTreeMap::new(),
            })
        };
        let wait_queue: WaitQueue = Rc::new(RefCell::new(Vec::new()));
        let extern_overrides = Rc::new(RefCell::new(BTreeMap::new()));
        let primary_id: u64 = 0;
        let next_coroutine_id: u64 = 1;
        let call_stack: SharedCallStack = Rc::new(RefCell::new(Vec::new()));

        // TODO: replay needs to restore the original initial_entry_implicits
        // (they are not currently persisted in the event log).
        let future = build_primary(
            &registry,
            entry_module_idx,
            &entity_name,
            entity_args,
            Vec::new(),
            event_log.clone(),
            heap.clone(),
            entity_value_id.id(),
            wait_queue.clone(),
            call_stack.clone(),
            primary_id,
            native_registry.clone(),
            extern_overrides.clone(),
        )?;

        Ok(EntityInstance {
            events,
            next_event_num,
            next_coroutine_id,
            id,
            entity_type,
            entry_module_idx,
            entity_name,
            registry,
            heap,
            entity_value_id,
            wait_queue,
            event_log,
            native_registry,
            extern_overrides,
            coroutines: vec![Coroutine {
                id: primary_id,
                source_event_num: None,
                future: Some(future),
                call_stack,
            }],
            primary_id,
            has_complete_event,
            recorded_exec_count,
            faulted: false,
            replay_only: false,
            disable_heap_collect,
        })
    }

    /// Set replay-only mode: tick returns after consuming all replay events
    /// instead of continuing execution past the replay boundary.
    pub fn set_replay_only(&mut self) {
        self.replay_only = true;
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn entity_type(&self) -> &str {
        &self.entity_type
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn into_events(mut self) -> Vec<Event> {
        std::mem::take(&mut self.events)
    }

    pub fn extern_overrides(&self) -> ExternOverrides {
        self.extern_overrides.clone()
    }

    pub fn last_event_num(&self) -> u64 {
        self.events.last().map_or(0, |e| e.num)
    }

    /// Clone all out field values (fields with `Out` modifier in the declaration).
    pub fn to_out_fields(&self) -> OwnedFields {
        let module = self.registry.module(self.entry_module_idx);
        let Some(decl) = module.get_declaration(&self.entity_name) else {
            return OwnedFields::new();
        };
        let heap = self.heap.borrow();
        let entity_fields = match heap.get(self.entity_value_id.id()) {
            crate::interpret::value::HeapValue::Entity { fields, .. } => fields,
            _ => return OwnedFields::new(),
        };
        decl.fields
            .iter()
            .filter(|f| f.modifier == Some(FieldModifier::Out))
            .filter_map(|f| {
                entity_fields
                    .get(&f.name)
                    .and_then(|v| OwnedValue::from_value(v, &heap).ok())
                    .map(|ov| (f.name.to_string(), ov))
            })
            .collect()
    }

    // TODO: Linear scan of declaration fields - cache or index if this becomes hot.
    /// Clone a single out field value by name. Returns None if the field
    /// doesn't exist or isn't an `Out` field.
    pub fn to_out_field(&self, name: &str) -> Option<OwnedValue> {
        let module = self.registry.module(self.entry_module_idx);
        let decl = module.get_declaration(&self.entity_name)?;
        let is_out = decl
            .fields
            .iter()
            .any(|f| f.name == name && f.modifier == Some(FieldModifier::Out));
        if !is_out {
            return None;
        }
        let heap = self.heap.borrow();
        match heap.get(self.entity_value_id.id()) {
            crate::interpret::value::HeapValue::Entity { fields, .. } => fields
                .get(name)
                .and_then(|v| OwnedValue::from_value(v, &heap).ok()),
            _ => None,
        }
    }

    /// Invoke a view member function and return its result synchronously.
    /// View functions have no side effects and complete in a single poll.
    pub fn invoke_view_func(
        &self,
        func_name: &str,
        args: OwnedFields,
        implicits: Vec<ImplicitInit>,
    ) -> Result<OwnedFields, EngineError> {
        let _guard = self.heap_collect_guard();
        let module = self.registry.module(self.entry_module_idx);
        let decl = module.get_declaration(&self.entity_name).ok_or_else(|| {
            EngineError::other(format!("entity '{}' not found", self.entity_name))
        })?;
        let member = decl
            .members
            .iter()
            .find(|m| m.name.as_deref() == Some(func_name))
            .ok_or_else(|| EngineError::other(format!("member '{func_name}' not found")))?;
        if !member.func_modifiers.view {
            return Err(EngineError::other(format!(
                "'{func_name}' is not a view function"
            )));
        }

        let call_stack: SharedCallStack = Rc::new(RefCell::new(Vec::new()));
        let event_log = Rc::new(RefCell::new(crate::event::EventLog::default()));
        let wait_queue: WaitQueue = Rc::new(RefCell::new(Vec::new()));

        let ctx = EvalContext::with_shared(
            self.registry.clone(),
            self.heap.clone(),
            event_log,
            wait_queue,
            call_stack,
            0,
            self.native_registry.clone(),
            self.extern_overrides.clone(),
        );

        let mut future: Pin<Box<dyn Future<Output = TickOutcome>>> =
            Box::pin(tick::run_member_func_entry(
                ctx,
                self.entry_module_idx,
                self.entity_name.clone(),
                func_name.into(),
                args,
                self.entity_value_id.id(),
                implicits,
            ));

        let waker = tick::noop_waker();
        let mut cx = Context::from_waker(&waker);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(TickOutcome::Ok(val)) => match val {
                OwnedValue::Data(fields) => Ok(fields),
                other => Err(EngineError::other(format!(
                    "view function '{func_name}' returned non-data value: {other:?}"
                ))),
            },
            Poll::Ready(TickOutcome::Fault(e)) => Err(EngineError::Fault {
                error: e,
                new_events: Vec::new(),
            }),
            Poll::Ready(TickOutcome::InternalError(e)) => Err(EngineError::InternalError {
                error: e,
                new_events: Vec::new(),
            }),
            Poll::Pending => Err(EngineError::other(format!(
                "view function '{func_name}' did not complete synchronously"
            ))),
        }
    }

    pub fn is_done(&self) -> bool {
        self.has_complete_event
    }

    /// True when the interpreter has not yet consumed all recorded execution
    /// events - i.e., tick is replaying history rather than executing new code.
    pub fn is_replaying(&self) -> bool {
        self.recorded_exec_count > 0 && self.event_log.borrow().cursor < self.recorded_exec_count
    }

    /// Snapshot each coroutine's call stack. Returns (coroutine_id, source_event_num, frames).
    pub fn coroutine_stacks(&self) -> Vec<(u64, Option<u64>, Vec<StackFrame>)> {
        self.coroutines
            .iter()
            .filter(|c| c.future.is_some())
            .map(|c| (c.id, c.source_event_num, c.call_stack.borrow().clone()))
            .collect()
    }

    /// Derive entity status from the event log.
    pub fn status(&self) -> EntityStatus {
        for event in self.events.iter().rev() {
            match &event.event_type {
                EventType::EntityComplete {
                    terminated: true, ..
                } => return EntityStatus::Terminated,
                EventType::EntityComplete {
                    terminated: false,
                    result,
                } => {
                    return EntityStatus::Completed {
                        result: result.clone(),
                    };
                }
                _ => {}
            }
        }
        EntityStatus::Running
    }

    /// Run a tick. Polls the interpreter future forward.
    ///
    /// Cooperative scheduler:
    /// - Phase 1: Poll all coroutines once. If any completed (Ready), restart
    ///   from Phase 1. When no coroutine makes progress, move to Phase 2.
    /// - Phase 2: Walk wait_queue FIFO. For each wait: wake its trigger, poll
    ///   ONLY the owning coroutine (by coroutine_id), check `resolved`. If
    ///   resolved → remove entry, restart from Phase 1. If nothing resolves →
    ///   Blocked.
    #[tracing::instrument(level = "debug", skip(self), fields(id = %self.id))]
    pub fn tick(&mut self, time_ms: u64) -> Result<EntityTickResult, TickError> {
        let _guard = self.heap_collect_guard();
        if self.faulted {
            return Err(TickError::AlreadyFaulted);
        }

        self.activate_pending_funcs();

        self.event_log.borrow_mut().current_time_ms = time_ms;
        let prev_log_count = self.event_log.borrow().events.len();
        let prev_self_count = self.events.len();

        let waker = tick::noop_waker();
        let mut cx = Context::from_waker(&waker);
        let mut primary_done: Option<TickOutcome> = None;
        let mut completed_funcs: Vec<(u64, TickOutcome)> = Vec::new();

        'tick: loop {
            // Phase 1: Poll all coroutines once. If any made progress,
            // restart the loop - only proceed to Phase 2 when all are settled.
            tracing::trace!("phase 1: polling coroutines");
            let mut made_progress = false;
            for c in self.coroutines.iter_mut() {
                let Some(fut) = c.future.as_mut() else {
                    continue;
                };
                if let Poll::Ready(outcome) = fut.as_mut().poll(&mut cx) {
                    c.future = None;
                    made_progress = true;
                    if matches!(
                        &outcome,
                        TickOutcome::Fault(_) | TickOutcome::InternalError(_)
                    ) {
                        self.faulted = true;
                        self.drain_event_log(prev_log_count);
                        let new_event_count = self.events.len() - prev_self_count;
                        return match outcome {
                            TickOutcome::Fault(e) => Err(TickError::Fault {
                                error: e,
                                new_event_count,
                            }),
                            TickOutcome::InternalError(e) => Err(TickError::InternalError {
                                error: e,
                                new_event_count,
                            }),
                            _ => unreachable!(),
                        };
                    }
                    if c.id == self.primary_id {
                        primary_done = Some(outcome);
                    } else if let Some(en) = c.source_event_num {
                        completed_funcs.push((en, outcome));
                    }
                }
            }

            if primary_done.is_some() {
                break 'tick;
            }

            if made_progress {
                continue 'tick;
            }

            // Phase 2: Try pending waits from wait_queue.
            tracing::trace!("phase 2: trying wait queue");
            let mut wait_resolved = false;
            let mut i = 0;
            loop {
                let len = self.wait_queue.borrow().len();
                if i >= len {
                    break;
                }

                let (trigger, resolved, coro_id) = {
                    let wq = self.wait_queue.borrow();
                    (
                        wq[i].trigger.clone(),
                        wq[i].resolved.clone(),
                        wq[i].coroutine_id,
                    )
                };

                trigger.wake();

                // We target the owning coroutine, but Rust futures don't
                // support polling a specific await point - the entire future
                // resumes, so it may complete outright. Handle that here.
                if let Some(c) = self.coroutines.iter_mut().find(|c| c.id == coro_id)
                    && let Some(fut) = c.future.as_mut()
                    && let Poll::Ready(outcome) = fut.as_mut().poll(&mut cx)
                {
                    c.future = None;
                    if matches!(
                        &outcome,
                        TickOutcome::Fault(_) | TickOutcome::InternalError(_)
                    ) {
                        self.faulted = true;
                        self.drain_event_log(prev_log_count);
                        let new_event_count = self.events.len() - prev_self_count;
                        return match outcome {
                            TickOutcome::Fault(e) => Err(TickError::Fault {
                                error: e,
                                new_event_count,
                            }),
                            TickOutcome::InternalError(e) => Err(TickError::InternalError {
                                error: e,
                                new_event_count,
                            }),
                            _ => unreachable!(),
                        };
                    }
                    if c.id == self.primary_id {
                        self.wait_queue.borrow_mut().remove(i);
                        primary_done = Some(outcome);
                        break;
                    } else if let Some(en) = c.source_event_num {
                        completed_funcs.push((en, outcome));
                    }
                }

                if resolved.get() {
                    self.wait_queue.borrow_mut().remove(i);
                    wait_resolved = true;
                    break;
                } else {
                    i += 1;
                }
            }

            if primary_done.is_some() {
                break 'tick;
            }

            if wait_resolved {
                continue 'tick;
            }

            break 'tick;
        }

        // Clean up completed coroutines.
        self.coroutines.retain(|c| c.future.is_some());

        // Validate replay: all recorded execution events must have been consumed.
        if self.recorded_exec_count > 0 {
            let cursor = self.event_log.borrow().cursor;
            // TODO: include structured divergence details (cursor position, expected extern, etc.)
            if cursor < self.recorded_exec_count {
                self.faulted = true;
                self.drain_event_log(prev_log_count);
                let new_event_count = self.events.len() - prev_self_count;
                return Err(TickError::Fault {
                    error: FaultError {
                        message: format!(
                            "replay divergence: only {}/{} execution events consumed",
                            cursor, self.recorded_exec_count
                        ),
                        stack: Vec::new(),
                    },
                    new_event_count,
                });
            }
            self.recorded_exec_count = 0;

            if self.replay_only {
                let status = match primary_done {
                    Some(TickOutcome::Ok(_)) => TickStatus::Completed,
                    _ => TickStatus::Blocked,
                };
                return Ok(EntityTickResult {
                    status,
                    new_event_count: 0,
                });
            }
        }

        // Move new events from event_log into self.events (no cloning).
        self.drain_event_log(prev_log_count);

        // Emit FuncComplete events for completed secondary coroutines.
        for (invoke_event_num, outcome) in completed_funcs {
            let val = match outcome {
                TickOutcome::Ok(val) => val,
                _ => unreachable!("faults handled earlier"),
            };
            let OwnedValue::Data(fields) = val else {
                self.faulted = true;
                let new_event_count = self.events.len() - prev_self_count;
                return Err(TickError::InternalError {
                    error: InternalError::new("func invocation returned non-Map value"),
                    new_event_count,
                });
            };
            let num = self.next_event_num;
            self.next_event_num += 1;
            self.events.push(Event {
                num,
                time: time_ms,
                event_type: EventType::FuncComplete {
                    invoke_num: invoke_event_num,
                    result: fields,
                },
            });
        }

        let status = if let Some(outcome) = primary_done {
            let val = match outcome {
                TickOutcome::Ok(val) => val,
                _ => unreachable!("faults handled earlier"),
            };
            let OwnedValue::Data(fields) = val else {
                self.faulted = true;
                let new_event_count = self.events.len() - prev_self_count;
                return Err(TickError::InternalError {
                    error: InternalError::new("entity primary returned non-Map value"),
                    new_event_count,
                });
            };
            self.append_event(
                EventType::EntityComplete {
                    terminated: false,
                    result: Some(fields),
                },
                time_ms,
            );
            self.has_complete_event = true;
            TickStatus::Completed
        } else {
            TickStatus::Blocked
        };

        tracing::debug!(?status, "tick complete");
        Ok(EntityTickResult {
            status,
            new_event_count: self.events.len() - prev_self_count,
        })
    }

    /// Complete a specific extern invocation by event number.
    #[tracing::instrument(level = "debug", skip(self, result), fields(id = %self.id, invoke_num))]
    pub fn complete_extern(
        &mut self,
        invoke_num: u64,
        result: OwnedFields,
        time_ms: u64,
    ) -> Result<(), EngineError> {
        let _guard = self.heap_collect_guard();
        if self.is_done() {
            return Err("entity is not running".into());
        }

        let mut found_invoke = false;
        let mut already_completed = false;
        for e in &self.events {
            if e.num == invoke_num && matches!(&e.event_type, EventType::ExternInvoke { .. }) {
                found_invoke = true;
            } else if matches!(&e.event_type, EventType::ExternComplete { invoke_num: r, .. } if *r == invoke_num)
            {
                already_completed = true;
            }
        }
        if !found_invoke {
            return Err(EngineError::NoPendingExtern {
                id: self.id.clone(),
            });
        } else if already_completed {
            return Err(format!("event {invoke_num} has already been completed").into());
        }

        let event_type = EventType::ExternComplete { invoke_num, result };
        let num = self.next_event_num;
        self.next_event_num += 1;
        let event = Event {
            num,
            time: time_ms,
            event_type: event_type.clone(),
        };
        self.events.push(event);

        // Add to shared EventLog so the future sees it on next poll.
        let mut log = self.event_log.borrow_mut();
        log.next_event_num = self.next_event_num;
        log.events.push(Event {
            num,
            time: time_ms,
            event_type,
        });
        log.cursor = log.events.len();

        Ok(())
    }

    /// Request cooperative cancellation.
    pub fn cancel(&mut self, reason: Option<String>, time_ms: u64) -> Result<(), EngineError> {
        if self.is_done() {
            return Err("entity is not running".into());
        }
        let already = self
            .events
            .iter()
            .any(|e| matches!(&e.event_type, EventType::EntityCancel { .. }));
        if already {
            return Err("entity already has a cancel request".into());
        }
        self.append_event(EventType::EntityCancel { reason }, time_ms);
        Ok(())
    }

    /// Force terminate without running code.
    pub fn terminate(&mut self, time_ms: u64) -> Result<(), EngineError> {
        if self.is_done() {
            return Err("entity is not running".into());
        }
        self.append_event(
            EventType::EntityComplete {
                terminated: true,
                result: None,
            },
            time_ms,
        );
        self.has_complete_event = true;
        Ok(())
    }

    /// Append a FuncInvoke event. Returns the event number.
    pub fn invoke_func(
        &mut self,
        func: String,
        request_id: Option<String>,
        args: OwnedFields,
        time_ms: u64,
    ) -> Result<u64, EngineError> {
        if self.is_done() {
            return Err("entity is already completed".into());
        }
        Ok(self.append_event(
            EventType::FuncInvoke {
                id: request_id,
                func,
                args,
            },
            time_ms,
        ))
    }

    /// Cancel a pending FuncInvoke by request ID.
    pub fn cancel_func(&mut self, request_id: &str, time_ms: u64) -> Result<(), EngineError> {
        let invoke_num = self
            .events
            .iter()
            .find_map(|e| match &e.event_type {
                EventType::FuncInvoke { id: Some(rid), .. } if rid == request_id => Some(e.num),
                _ => None,
            })
            .ok_or_else(|| format!("no FuncInvoke with request-id '{request_id}'"))?;

        let already_done = self.events.iter().any(|e| {
            matches!(
                &e.event_type,
                EventType::FuncComplete { invoke_num: n, .. }
                | EventType::FuncCancel { invoke_num: n, .. }
                if *n == invoke_num
            )
        });
        if already_done {
            return Err(
                format!("FuncInvoke '{request_id}' is already completed or cancelled").into(),
            );
        }

        self.append_event(
            EventType::FuncCancel {
                invoke_num,
                reason: None,
            },
            time_ms,
        );
        Ok(())
    }

    /// Move new events from the shared EventLog into self.events.
    fn drain_event_log(&mut self, prev_log_count: usize) {
        let mut log = self.event_log.borrow_mut();
        self.next_event_num = log.next_event_num;
        self.events.extend(log.events.drain(prev_log_count..));
    }

    fn append_event(&mut self, event_type: EventType, time_ms: u64) -> u64 {
        let n = self.next_event_num;
        self.next_event_num += 1;
        self.events.push(Event {
            num: n,
            time: time_ms,
            event_type,
        });
        n
    }

    /// Scan entity events for FuncInvoke without matching FuncComplete/FuncCancel,
    /// and create coroutines for any that aren't already active.
    // TODO: Linear scan of all events every tick - should track pending funcs incrementally.
    fn activate_pending_funcs(&mut self) {
        // Single pass: collect pending FuncInvokes, removing completed/cancelled ones.
        let mut pending = Vec::new();
        for e in &self.events {
            match &e.event_type {
                EventType::FuncInvoke { func, args, .. } => {
                    pending.push((e.num, func.clone(), args.clone()));
                }
                EventType::FuncComplete { invoke_num, .. }
                | EventType::FuncCancel { invoke_num, .. } => {
                    pending.retain(|(en, _, _): &(u64, _, _)| en != invoke_num);
                }
                _ => {}
            }
        }

        if pending.is_empty() {
            return;
        }

        // Filter out already-active coroutines.
        pending.retain(|(en, _, _)| {
            !self
                .coroutines
                .iter()
                .any(|c| c.source_event_num == Some(*en))
        });

        for (event_num, func_name, args) in pending {
            let coro_id = self.next_coroutine_id;
            self.next_coroutine_id += 1;
            let call_stack: SharedCallStack = Rc::new(RefCell::new(Vec::new()));

            let ctx = EvalContext::with_shared(
                self.registry.clone(),
                self.heap.clone(),
                self.event_log.clone(),
                self.wait_queue.clone(),
                call_stack.clone(),
                coro_id,
                self.native_registry.clone(),
                self.extern_overrides.clone(),
            );

            // TODO: replay needs to restore the original implicits
            // (they are not currently persisted in the event log).
            let future: Pin<Box<dyn Future<Output = TickOutcome>>> =
                Box::pin(tick::run_member_func_entry(
                    ctx,
                    self.entry_module_idx,
                    self.entity_name.clone(),
                    func_name.into(),
                    args,
                    self.entity_value_id.id(),
                    Vec::new(),
                ));

            self.coroutines.push(Coroutine {
                id: coro_id,
                source_event_num: Some(event_num),
                future: Some(future),
                call_stack,
            });
        }
    }

    /// Consume the entity instance, dropping all owned values, and return
    /// details for verifying the heap is clean.
    pub fn into_drop_details(self) -> DropDetails {
        let heap = self.heap.clone();
        let entity_id = self.id.clone();
        drop(self);
        DropDetails { heap, entity_id }
    }
}

impl Drop for EntityInstance {
    fn drop(&mut self) {
        // Remove any extern overrides registered by this entity. Each
        // ExternOverride holds a RootedId (func_value) on this entity's heap
        // plus an Rc back to the overrides map - creating an Rc cycle that
        // prevents the override (and its heap references) from being freed.
        self.extern_overrides
            .borrow_mut()
            .retain(|_, ovr| !Rc::ptr_eq(&ovr.heap, &self.heap));
    }
}

/// Drop details returned by `into_drop_details` for post-drop leak checking.
pub struct DropDetails {
    heap: Rc<RefCell<Heap>>,
    entity_id: String,
}

impl DropDetails {
    /// Assert that all heap entries were freed. Call this after the
    /// `EntityInstance` has been consumed by `into_drop_details`.
    pub fn assert_clean(&self) -> Result<(), String> {
        self.heap.borrow_mut().gc_collect();
        let heap = self.heap.borrow();
        let count = heap.live_count();
        if count > 0 {
            let report = heap.leak_report();
            Err(format!(
                "heap leak: {} live entries after entity '{}' teardown: {:?}",
                count, self.entity_id, report
            ))
        } else {
            Ok(())
        }
    }
}

/// Build the primary coroutine future for an entity (shared by spawn and from_events).
#[allow(clippy::too_many_arguments)]
fn build_primary(
    registry: &Arc<Registry>,
    module_idx: ModuleIndex,
    entity_name: &Symbol,
    args: OwnedFields,
    initial_entry_implicits: Vec<ImplicitInit>,
    event_log: Rc<RefCell<EventLog>>,
    heap: Rc<RefCell<Heap>>,
    entity_value_id: ValueId,
    wait_queue: WaitQueue,
    call_stack: SharedCallStack,
    coroutine_id: u64,
    native_registry: Rc<NativeRegistry>,
    extern_overrides: ExternOverrides,
) -> Result<Pin<Box<dyn Future<Output = TickOutcome>>>, String> {
    let ctx = EvalContext::with_shared(
        registry.clone(),
        heap,
        event_log.clone(),
        wait_queue,
        call_stack,
        coroutine_id,
        native_registry,
        extern_overrides,
    );
    let module = registry.module(module_idx);
    let use_func_entry = module
        .get_declaration(entity_name)
        .is_some_and(|d| d.kind == TypeConstructKind::Func);
    if use_func_entry {
        Ok(Box::pin(tick::run_entry(
            ctx,
            module_idx,
            entity_name.clone(),
            args,
            initial_entry_implicits,
        )))
    } else {
        Ok(Box::pin(tick::run_entity_entry(
            ctx,
            module_idx,
            entity_name.clone(),
            args,
            entity_value_id,
            initial_entry_implicits,
        )))
    }
}

pub struct SpawnExternInfo {
    pub invoke_num: u64,
    pub entity_type: String,
    pub args: OwnedFields,
    pub id: Option<String>,
}

/// Find a `duralade.entity::spawn` ExternInvoke in a list of events.
// TODO: This scans all events linearly. The tick/engine should track the pending
// extern directly (e.g. return it in the tick result) instead of re-scanning.
pub fn find_spawn_extern(events: &[Event]) -> Option<SpawnExternInfo> {
    for event in events {
        if let EventType::ExternInvoke { extern_name, args } = &event.event_type
            && extern_name == "duralade.entity::spawn"
        {
            let entity_type = match args.get("entity") {
                Some(OwnedValue::Str(s)) => s.clone(),
                _ => continue,
            };
            let constructor_args = match args.get("args") {
                Some(OwnedValue::Data(m)) => m.clone(),
                _ => OwnedFields::new(),
            };
            let id = match args.get("id") {
                Some(OwnedValue::Str(s)) => Some(s.clone()),
                _ => None,
            };
            return Some(SpawnExternInfo {
                invoke_num: event.num,
                entity_type,
                args: constructor_args,
                id,
            });
        }
    }
    None
}

pub struct ResultExternInfo {
    pub invoke_num: u64,
    pub entity_id: String,
}

/// Find a `duralade.entity::result` ExternInvoke in a list of events.
// TODO: Same linear scan issue as find_spawn_extern - tick/engine should track pending externs directly.
pub fn find_result_extern(events: &[Event]) -> Option<ResultExternInfo> {
    for event in events {
        if let EventType::ExternInvoke { extern_name, args } = &event.event_type
            && extern_name == "duralade.entity::result"
        {
            let entity_id = match args.get("entity_id") {
                Some(OwnedValue::EntityRef(eid)) => eid.0.clone(),
                _ => continue,
            };
            return Some(ResultExternInfo {
                invoke_num: event.num,
                entity_id,
            });
        }
    }
    None
}

pub struct CallExternInfo {
    pub invoke_num: u64,
    pub entity_id: String,
    pub func_name: String,
    pub args: OwnedFields,
}

/// Find a `duralade.entity::call` ExternInvoke in a list of events.
// TODO: Same linear scan issue as find_spawn_extern - tick/engine should track pending externs directly.
pub fn find_call_extern(events: &[Event]) -> Option<CallExternInfo> {
    for event in events {
        if let EventType::ExternInvoke { extern_name, args } = &event.event_type
            && extern_name == "duralade.entity::call"
        {
            let entity_id = match args.get("entity_id") {
                Some(OwnedValue::EntityRef(eid)) => eid.0.clone(),
                _ => continue,
            };
            let func_name = match args.get("func") {
                Some(OwnedValue::Str(s)) => s.clone(),
                _ => continue,
            };
            let call_args = match args.get("args") {
                Some(OwnedValue::Data(m)) => m.clone(),
                _ => OwnedFields::new(),
            };
            return Some(CallExternInfo {
                invoke_num: event.num,
                entity_id,
                func_name,
                args: call_args,
            });
        }
    }
    None
}

/// Parse an entity type string into (module path, entity name).
/// Format: `root.module.path` (entity name = last segment)
/// or `root.module.path::construct` (explicit entity name).
/// Parse entity type string into (module_path_str, entity_name).
/// Format: `root.module.path` (entity name = last segment)
/// or `root.module.path::construct` (explicit entity name).
fn parse_entity_type(entity_type: &str) -> Result<(Symbol, Symbol), String> {
    let (module_part, explicit_name) = match entity_type.split_once("::") {
        Some((module, name)) => (module, Some(name)),
        None => (entity_type, None),
    };

    let segments: Vec<&str> = module_part.split('.').collect();
    if segments.len() < 2 {
        return Err(format!(
            "entity type '{}' must have at least root.module",
            entity_type
        ));
    }

    let entity_name = explicit_name.unwrap_or_else(|| segments.last().unwrap());

    Ok((module_part.into(), entity_name.into()))
}

// --- Extern overrides (test-time feature) ---

/// An extern override: captures the registering entity's heap and func so the
/// engine can execute the override on demand, passing OwnedValues across heaps.
pub struct ExternOverride {
    pub heap: Rc<RefCell<Heap>>,
    /// Holds the RootedId that keeps the func alive on the heap.
    pub func_value: crate::interpret::value::Value,
    pub registry: Arc<Registry>,
    pub native_registry: Rc<NativeRegistry>,
    pub extern_overrides: ExternOverrides,
    /// The target extern's DeclId, used for type-directed arg conversion.
    pub target_decl: crate::load::registry::DeclId,
}
// No Drop impl needed - func_value's RootedId auto-dec_refs.

impl ExternOverride {
    pub fn execute(&self, owned_args: OwnedFields) -> Result<OwnedFields, String> {
        let decl_entry = self.registry.decl(self.target_decl);
        let extern_decl = self
            .registry
            .module(decl_entry.module)
            .get_declaration(&decl_entry.name)
            .ok_or_else(|| format!("cannot resolve extern declaration '{}'", decl_entry.name))?;

        let call_stack: SharedCallStack = Rc::new(RefCell::new(Vec::new()));
        let event_log = Rc::new(RefCell::new(crate::event::EventLog::default()));
        let wait_queue: WaitQueue = Rc::new(RefCell::new(Vec::new()));

        let ctx = EvalContext::with_shared(
            self.registry.clone(),
            self.heap.clone(),
            event_log,
            wait_queue,
            call_stack,
            0,
            self.native_registry.clone(),
            self.extern_overrides.clone(),
        );

        let runtime_args =
            tick::convert_args(owned_args, &extern_decl.fields, &decl_entry.name, &ctx)
                .map_err(|e| e.0)?;

        let func_id = self
            .func_value
            .as_heap_id()
            .expect("ExternOverride func_value must be a Func");
        let mut future: Pin<Box<dyn Future<Output = TickOutcome>>> =
            Box::pin(tick::run_override_func(ctx, func_id, runtime_args));

        let waker = tick::noop_waker();
        let mut cx = Context::from_waker(&waker);
        match future.as_mut().poll(&mut cx) {
            Poll::Ready(TickOutcome::Ok(val)) => match val {
                OwnedValue::Data(fields) => Ok(fields),
                OwnedValue::Nil => Ok(OwnedFields::new()),
                other => Err(format!("extern override returned non-map value: {other:?}")),
            },
            Poll::Ready(TickOutcome::Fault(e)) => {
                Err(format!("extern override faulted: {}", e.message))
            }
            Poll::Ready(TickOutcome::InternalError(e)) => {
                Err(format!("extern override internal error: {}", e.message))
            }
            Poll::Pending => Err(
                "extern override function cannot perform async operations (e.g. extern calls)"
                    .into(),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_registry() -> Arc<Registry> {
        Arc::new(Registry::new())
    }

    #[test]
    fn from_events_empty_log() {
        assert!(
            EntityInstance::from_events(EntityFromEventsOptions {
                registry: empty_registry(),
                events: vec![],
                native_registry: Rc::new(super::native::NativeRegistry::new()),
                until_event_num: None,
                disable_heap_collect: true,
            })
            .is_err()
        );
    }

    #[test]
    fn from_events_wrong_first_event() {
        let events = vec![Event {
            num: 0,
            time: 0,
            event_type: EventType::EntityCancel { reason: None },
        }];
        assert!(
            EntityInstance::from_events(EntityFromEventsOptions {
                registry: empty_registry(),
                events,
                native_registry: Rc::new(super::native::NativeRegistry::new()),
                until_event_num: None,
                disable_heap_collect: true,
            })
            .is_err()
        );
    }

    #[test]
    fn parse_entity_type_works() {
        let (path, name) = parse_entity_type("my_project.admin.users").unwrap();
        assert_eq!(path, "my_project.admin.users");
        assert_eq!(name, "users");
    }

    #[test]
    fn parse_entity_type_explicit_name() {
        let (path, name) = parse_entity_type("my_project.admin::user_entity").unwrap();
        assert_eq!(path, "my_project.admin");
        assert_eq!(name, "user_entity");
    }

    #[test]
    fn parse_entity_type_single_segment_fails() {
        assert!(parse_entity_type("root").is_err());
    }
}
