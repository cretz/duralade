use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use serde::{Deserialize, Serialize};

use crate::engine::ModuleIndex;
use crate::load::registry::DeclId;
use crate::model::{Func, Symbol};

use super::scope::RuntimeScopeRef;

/// Trait for Rust objects stored on the Duralade heap as `HeapValue::Native`.
/// Downcast via `(&*native as &dyn Any)`. Implement `gc_walk` so the
/// garbage collector can walk Value references held inside the native object.
pub trait NativeValue: Any {
    /// Call `visitor` for each `Value` this native object holds.
    fn gc_walk(&self, visitor: &mut dyn FnMut(&Value));

    /// Compute a deterministic hash for this native value.
    /// Returns Err if the value is not hashable.
    fn native_hash(&self, _heap: &Heap) -> Result<u64, String> {
        Err("native value is not hashable".into())
    }
}

impl fmt::Debug for dyn NativeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NativeValue")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ValueId(u32);

/// Separated refcount storage. Shared (via Rc) between the Heap and all
/// RootedIds so that inc/dec never needs to borrow the heap.
pub struct RefCounts {
    counts: RefCell<Vec<Cell<u32>>>,
    pending_frees: RefCell<Vec<ValueId>>,
}

impl RefCounts {
    fn new() -> Self {
        RefCounts {
            counts: RefCell::new(Vec::new()),
            pending_frees: RefCell::new(Vec::new()),
        }
    }

    /// Push a new slot (called by Heap::alloc). Returns the index.
    fn push(&self, initial: u32) -> u32 {
        let mut counts = self.counts.borrow_mut();
        let idx = counts.len() as u32;
        counts.push(Cell::new(initial));
        idx
    }

    /// Reuse a freed slot (called by Heap::alloc when reusing from free_list).
    fn reset(&self, idx: u32, initial: u32) {
        self.counts.borrow()[idx as usize].set(initial);
    }

    fn inc(&self, idx: u32) {
        let counts = self.counts.borrow();
        let c = &counts[idx as usize];
        c.set(c.get() + 1);
    }

    fn dec(&self, idx: u32) -> u32 {
        let counts = self.counts.borrow();
        let c = &counts[idx as usize];
        let n = c.get();
        debug_assert!(n > 0, "RootedId dec on refcount 0 - double free?");
        let n = n - 1;
        c.set(n);
        n
    }

    pub fn get(&self, id: ValueId) -> u32 {
        self.counts.borrow()[id.0 as usize].get()
    }
}

impl fmt::Debug for RefCounts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RefCounts")
            .field("len", &self.counts.borrow().len())
            .field("pending", &self.pending_frees.borrow().len())
            .finish()
    }
}

/// RAII handle to a heap-allocated value. Clone increments the refcount,
/// Drop decrements it. When the refcount hits zero, the ValueId is pushed
/// to a pending-frees queue (no heap borrow needed).
pub struct RootedId {
    id: ValueId,
    rc: Rc<RefCounts>,
}

impl RootedId {
    pub fn id(&self) -> ValueId {
        self.id
    }
}

impl Clone for RootedId {
    fn clone(&self) -> Self {
        self.rc.inc(self.id.0);
        RootedId {
            id: self.id,
            rc: self.rc.clone(),
        }
    }
}

impl Drop for RootedId {
    fn drop(&mut self) {
        let n = self.rc.dec(self.id.0);
        if n == 0 {
            self.rc.pending_frees.borrow_mut().push(self.id);
        }
    }
}

impl fmt::Debug for RootedId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RootedId({}, rc={})", self.id.0, self.rc.get(self.id))
    }
}

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Float(f64),
    Str(Rc<str>),
    Bool(bool),
    Nil,
    Data(RootedId),
    Func(RootedId),
    Entity(RootedId),
    EntityRef(EntityId),
    /// Opaque Rust object on the heap. Used by native code to pass
    /// arbitrary state through Duralade values (e.g. yielder channels).
    Native(RootedId),
    /// Reified type, used for type param bindings at runtime (e.g. `intype t`).
    Type(crate::load::registry::TypeId),
}

#[derive(Debug, Clone)]
pub enum HeapValueFuncKind {
    /// Anonymous func closure with AST body + captured scope.
    Closure {
        captures: BTreeMap<Symbol, Value>,
        captured_scope: RuntimeScopeRef,
        body: Func,
    },
    /// Reference to a named declaration (module-level or member func).
    Decl {
        decl: DeclId,
        name: Symbol,
        /// When Some, this is a bound member on a data/entity value.
        target: Option<Value>,
    },
}

#[derive(Debug)]
pub enum HeapValue {
    Data {
        decl: Option<DeclId>,
        out_early_name: Option<Symbol>,
        fields: BTreeMap<Symbol, Value>,
        type_args: BTreeMap<Symbol, crate::load::registry::TypeId>,
    },
    Func {
        module: ModuleIndex,
        kind: HeapValueFuncKind,
    },
    Entity {
        decl: DeclId,
        fields: BTreeMap<Symbol, Value>,
    },
    /// Rust object on the heap implementing `NativeValue`. Downcast via
    /// `(&*native as &dyn Any)` in the handler that knows the concrete type.
    Native(Box<dyn NativeValue>),
}

impl HeapValue {
    /// Count heap-internal references originating from this HeapValue.
    /// For each child Value with a heap ID, increments that ID's count in
    /// `internal_refs`. For Func entries, also walks the captured BindingsFrame
    /// parent chain (deduplicating shared frames via `visited_frames`).
    fn gc_collect_count_internal_refs(
        &self,
        internal_refs: &mut HashMap<ValueId, u32>,
        visited_frames: &mut HashSet<*const RefCell<super::scope::RuntimeScope>>,
    ) {
        match self {
            HeapValue::Data { fields, .. } | HeapValue::Entity { fields, .. } => {
                for v in fields.values() {
                    if let Some(id) = v.as_heap_id() {
                        *internal_refs.entry(id).or_default() += 1;
                    }
                }
            }
            HeapValue::Func { kind, .. } => match kind {
                HeapValueFuncKind::Closure {
                    captures,
                    captured_scope,
                    ..
                } => {
                    for v in captures.values() {
                        if let Some(id) = v.as_heap_id() {
                            *internal_refs.entry(id).or_default() += 1;
                        }
                    }
                    Heap::gc_collect_count_bindings_refs(
                        captured_scope,
                        internal_refs,
                        visited_frames,
                    );
                }
                HeapValueFuncKind::Decl { target, .. } => {
                    if let Some(v) = target
                        && let Some(id) = v.as_heap_id()
                    {
                        *internal_refs.entry(id).or_default() += 1;
                    }
                }
            },
            HeapValue::Native(native) => {
                native.gc_walk(&mut |v| {
                    if let Some(id) = v.as_heap_id() {
                        *internal_refs.entry(id).or_default() += 1;
                    }
                });
            }
        }
    }

    /// Push all heap-referencing children onto a BFS worklist.
    /// For Func entries, also walks the captured BindingsFrame chain.
    fn gc_collect_push_children(
        &self,
        worklist: &mut VecDeque<ValueId>,
        visited_frames: &mut HashSet<*const RefCell<super::scope::RuntimeScope>>,
    ) {
        match self {
            HeapValue::Data { fields, .. } | HeapValue::Entity { fields, .. } => {
                for v in fields.values() {
                    if let Some(id) = v.as_heap_id() {
                        worklist.push_back(id);
                    }
                }
            }
            HeapValue::Func { kind, .. } => match kind {
                HeapValueFuncKind::Closure {
                    captures,
                    captured_scope,
                    ..
                } => {
                    for v in captures.values() {
                        if let Some(id) = v.as_heap_id() {
                            worklist.push_back(id);
                        }
                    }
                    Heap::gc_collect_push_bindings_children(
                        captured_scope,
                        worklist,
                        visited_frames,
                    );
                }
                HeapValueFuncKind::Decl { target, .. } => {
                    if let Some(v) = target
                        && let Some(id) = v.as_heap_id()
                    {
                        worklist.push_back(id);
                    }
                }
            },
            HeapValue::Native(native) => {
                native.gc_walk(&mut |v| {
                    if let Some(id) = v.as_heap_id() {
                        worklist.push_back(id);
                    }
                });
            }
        }
    }
}

#[derive(Debug)]
pub struct Heap {
    entries: Vec<Option<HeapValue>>,
    free_list: Vec<u32>,
    rc: Rc<RefCounts>,
}

impl Default for Heap {
    fn default() -> Self {
        Heap {
            entries: Vec::new(),
            free_list: Vec::new(),
            rc: Rc::new(RefCounts::new()),
        }
    }
}

impl Heap {
    /// Allocate a value on the heap. Returns a RootedId with refcount 1.
    /// Child values inside the HeapValue are moved in - their RootedIds
    /// carry their own refcounts (no extra inc needed).
    pub fn alloc(&mut self, value: HeapValue) -> RootedId {
        let idx = if let Some(idx) = self.free_list.pop() {
            self.entries[idx as usize] = Some(value);
            self.rc.reset(idx, 1);
            idx
        } else {
            let idx = self.rc.push(1);
            self.entries.push(Some(value));
            idx
        };
        RootedId {
            id: ValueId(idx),
            rc: self.rc.clone(),
        }
    }

    pub fn get(&self, id: ValueId) -> &HeapValue {
        self.entries[id.0 as usize]
            .as_ref()
            .expect("dangling ValueId")
    }

    pub fn get_mut(&mut self, id: ValueId) -> &mut HeapValue {
        self.entries[id.0 as usize]
            .as_mut()
            .expect("dangling ValueId")
    }

    /// Return a RootedId reference to the shared RefCounts, for passing to
    /// code that needs to create Values from raw ValueIds (e.g. into_value).
    pub fn ref_counts(&self) -> Rc<RefCounts> {
        self.rc.clone()
    }

    /// Free a heap entry. The HeapValue is dropped, which drops child Values,
    /// which drops their RootedIds - cascading dec_ref to pending_frees.
    fn free(&mut self, id: ValueId) {
        let _entry = self.entries[id.0 as usize].take();
        self.free_list.push(id.0);
        // _entry drops here → HeapValue drops → child RootedIds drop →
        // any that hit refcount 0 push to pending_frees
    }

    /// Collect garbage: drain zero-refcount entries, then detect and free
    /// unreachable reference cycles via trial deletion.
    ///
    /// Trial deletion (similar to CPython's cycle collector):
    /// 1. Count "internal" refs - references from within the heap (including
    ///    BindingsFrame chains on Func entries) to each heap entry.
    /// 2. Entries where refcount > internal refs have external holders
    ///    (coroutine scopes, entity_value_id, etc.) - these are roots.
    /// 3. BFS from roots to mark all reachable entries.
    /// 4. Free everything not marked - these are unreachable cycles.
    pub fn gc_collect(&mut self) {
        self.gc_collect_drain_pending_frees();

        if self.live_count() == 0 {
            return;
        }

        // Step 1: Count internal references for each heap entry.
        let mut internal_refs: HashMap<ValueId, u32> = HashMap::new();
        let mut visited_frames: HashSet<*const RefCell<super::scope::RuntimeScope>> =
            HashSet::new();
        for entry in self.entries.iter().flatten() {
            entry.gc_collect_count_internal_refs(&mut internal_refs, &mut visited_frames);
        }

        // Step 2: Seed BFS worklist with roots - entries that have at least
        // one reference from outside the heap (refcount > internal count).
        let mut reachable: HashSet<ValueId> = HashSet::new();
        let mut worklist: VecDeque<ValueId> = VecDeque::new();
        for (i, entry) in self.entries.iter().enumerate() {
            if entry.is_none() {
                continue;
            }
            let id = ValueId(i as u32);
            let external = self.rc.get(id) - internal_refs.get(&id).copied().unwrap_or(0);
            if external > 0 {
                worklist.push_back(id);
            }
        }

        // Step 3: BFS - mark everything reachable from roots.
        let mut mark_visited_frames: HashSet<*const RefCell<super::scope::RuntimeScope>> =
            HashSet::new();
        while let Some(id) = worklist.pop_front() {
            if !reachable.insert(id) {
                continue;
            }
            if let Some(hv) = &self.entries[id.0 as usize] {
                hv.gc_collect_push_children(&mut worklist, &mut mark_visited_frames);
            }
        }

        // Step 4: Sweep - free every live entry not marked reachable.
        for i in 0..self.entries.len() {
            let id = ValueId(i as u32);
            if self.entries[i].is_some() && !reachable.contains(&id) {
                self.free(id);
            }
        }
        // Freeing cycle members drops their children's RootedIds, which may
        // cascade more zero-refcount frees.
        self.gc_collect_drain_pending_frees();
    }

    fn gc_collect_drain_pending_frees(&mut self) {
        loop {
            let id = self.rc.pending_frees.borrow_mut().pop();
            let Some(id) = id else { break };
            if self.entries[id.0 as usize].is_some() {
                self.free(id);
            }
        }
    }

    /// Walk a BindingsFrame parent chain, counting heap references from each
    /// frame's stack values. Deduplicates via Rc pointer identity so shared
    /// frames are only counted once.
    fn gc_collect_count_bindings_refs(
        frame: &RuntimeScopeRef,
        internal_refs: &mut HashMap<ValueId, u32>,
        visited: &mut HashSet<*const RefCell<super::scope::RuntimeScope>>,
    ) {
        if !visited.insert(Rc::as_ptr(frame)) {
            return;
        }
        let borrowed = frame.borrow();
        for v in borrowed.vars.values() {
            if let Some(id) = v.as_heap_id() {
                *internal_refs.entry(id).or_default() += 1;
            }
        }
        if let Some(parent) = &borrowed.parent {
            Self::gc_collect_count_bindings_refs(parent, internal_refs, visited);
        }
    }

    /// Walk a BindingsFrame parent chain, pushing heap references from each
    /// frame's stack values onto the BFS worklist.
    fn gc_collect_push_bindings_children(
        frame: &RuntimeScopeRef,
        worklist: &mut VecDeque<ValueId>,
        visited: &mut HashSet<*const RefCell<super::scope::RuntimeScope>>,
    ) {
        if !visited.insert(Rc::as_ptr(frame)) {
            return;
        }
        let borrowed = frame.borrow();
        for v in borrowed.vars.values() {
            if let Some(id) = v.as_heap_id() {
                worklist.push_back(id);
            }
        }
        if let Some(parent) = &borrowed.parent {
            Self::gc_collect_push_bindings_children(parent, worklist, visited);
        }
    }

    /// Count of live (non-freed) heap entries.
    pub fn live_count(&self) -> usize {
        self.entries.iter().filter(|e| e.is_some()).count()
    }

    /// Return diagnostic info for each live entry: (id, refcount, type tag).
    pub fn leak_report(&self) -> Vec<(ValueId, u32, &str)> {
        self.entries
            .iter()
            .enumerate()
            .filter_map(|(i, entry)| {
                let e = entry.as_ref()?;
                let rc = self.rc.get(ValueId(i as u32));
                let tag = match e {
                    HeapValue::Data { .. } => "Data",
                    HeapValue::Func { .. } => "Func",
                    HeapValue::Entity { .. } => "Entity",
                    HeapValue::Native(_) => "Native",
                };
                Some((ValueId(i as u32), rc, tag))
            })
            .collect()
    }

    /// Create a RootedId for a ValueId that is already allocated on this heap
    /// (e.g. entity_value_id). Increments the refcount.
    pub fn root(&self, id: ValueId) -> RootedId {
        self.rc.inc(id.0);
        RootedId {
            id,
            rc: self.rc.clone(),
        }
    }
}

impl Value {
    /// Returns the ValueId if this is a heap-allocated variant.
    pub fn as_heap_id(&self) -> Option<ValueId> {
        match self {
            Value::Data(r) | Value::Func(r) | Value::Entity(r) | Value::Native(r) => Some(r.id()),
            _ => None,
        }
    }
}

/// Compute a deterministic hash for a Value.
///
/// Hashable types: int, float, str, bool, nil, Data (structural),
/// EntityRef, Native (if it implements `native_hash`).
/// Returns Err for unhashable types (func, entity).
///
/// TODO: when exposing to users, replace DefaultHasher with a spec'd algorithm
/// (e.g. FNV-1a with documented constants) so the hash is replicatable across
/// implementations.
pub fn hash_value(value: &Value, heap: &Heap) -> Result<u64, String> {
    let mut hasher = DefaultHasher::new();
    hash_value_into(value, heap, &mut hasher)?;
    Ok(hasher.finish())
}

fn hash_value_into(value: &Value, heap: &Heap, hasher: &mut DefaultHasher) -> Result<(), String> {
    // Mix in a type tag so e.g. int(1) and bool(true) don't collide.
    match value {
        Value::Int(n) => {
            0u8.hash(hasher);
            n.hash(hasher);
        }
        Value::Float(f) => {
            1u8.hash(hasher);
            f.to_bits().hash(hasher);
        }
        Value::Str(s) => {
            2u8.hash(hasher);
            s.hash(hasher);
        }
        Value::Bool(b) => {
            3u8.hash(hasher);
            b.hash(hasher);
        }
        Value::Nil => {
            4u8.hash(hasher);
        }
        Value::Data(id) => {
            5u8.hash(hasher);
            let HeapValue::Data { decl, fields, .. } = heap.get(id.id()) else {
                return Err("hash_value: Data RootedId points to non-Data".into());
            };
            decl.hash(hasher);
            // BTreeMap iterates in sorted key order - deterministic.
            for (k, v) in fields {
                k.hash(hasher);
                hash_value_into(v, heap, hasher)?;
            }
        }
        Value::Func(_) => return Err("hash_value: func is not hashable".into()),
        Value::Entity(_) => return Err("hash_value: entity is not hashable".into()),
        Value::EntityRef(eid) => {
            6u8.hash(hasher);
            eid.hash(hasher);
        }
        Value::Native(id) => {
            let HeapValue::Native(native) = heap.get(id.id()) else {
                return Err("hash_value: Native RootedId points to non-Native".into());
            };
            7u8.hash(hasher);
            // Discriminate by concrete type so different NativeValue impls
            // with the same internal hash don't collide.
            (native.as_ref() as &dyn Any).type_id().hash(hasher);
            native.native_hash(heap)?.hash(hasher);
        }
        Value::Type(_) => return Err("hash_value: type is not hashable".into()),
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EntityId(pub String);
