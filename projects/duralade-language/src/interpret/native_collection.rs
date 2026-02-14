use std::cell::RefCell;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hasher};

use indexmap::IndexMap;

use crate::load::registry::DeclId;
use crate::model::Symbol;

use super::value::{Heap, HeapValue, NativeValue, Value, hash_value};

/// Native backing for array values. Stored on the heap as
/// `HeapValue::Native(Box::new(NativeArray(...)))` and referenced from
/// a Data's `_items` field as `Value::Native(rooted_id)`.
pub struct NativeArray(pub RefCell<Vec<Value>>);

impl NativeArray {
    /// Allocate a new array Data on the heap from a Vec of Values.
    /// Returns a `Value::Data` with `decl` and `_items` native backing.
    pub fn alloc(
        items: Vec<Value>,
        decl: Option<crate::load::registry::DeclId>,
        element_type: crate::load::registry::TypeId,
        heap: &mut Heap,
    ) -> Value {
        let native = NativeArray(RefCell::new(items));
        let native_id = heap.alloc(HeapValue::Native(Box::new(native)));
        let mut fields = BTreeMap::new();
        fields.insert(Symbol::underscore_items(), Value::Native(native_id));
        let mut type_args = BTreeMap::new();
        type_args.insert(Symbol::t(), element_type);
        Value::Data(heap.alloc(HeapValue::Data {
            decl,
            out_early_name: None,
            fields,
            type_args,
        }))
    }
}

/// Key wrapper for NativeMap. Stores a pre-computed hash alongside the Value
/// so IndexMap can hash/eq without heap access. Equality is hash-based only  -
/// collisions are astronomically unlikely with 64-bit hashes, and we don't
/// have heap access inside Eq. If this ever matters, we can switch to a
/// custom hash table that threads heap access through lookups.
///
/// TODO: when exposing hashing to users, revisit whether hash-only equality
/// is acceptable or if we need structural equality with heap access.
#[derive(Debug, Clone)]
pub struct MapKey {
    value: Value,
    cached_hash: u64,
}

impl std::hash::Hash for MapKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.cached_hash.hash(state);
    }
}

impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.cached_hash == other.cached_hash
    }
}

impl Eq for MapKey {}

impl MapKey {
    pub fn new(value: Value, heap: &Heap) -> Result<Self, String> {
        let cached_hash = hash_value(&value, heap)?;
        Ok(MapKey { value, cached_hash })
    }

    pub fn value(&self) -> &Value {
        &self.value
    }
}

/// Native backing for map values. Insertion-ordered via IndexMap.
/// Stored on the heap as `HeapValue::Native(Box::new(NativeMap(RefCell::new(IndexMap))))`
/// and referenced from a Data's `_items` field as `Value::Native(rooted_id)`.
pub struct NativeMap(pub RefCell<IndexMap<MapKey, Value>>);

impl Default for NativeMap {
    fn default() -> Self {
        NativeMap(RefCell::new(IndexMap::new()))
    }
}

impl NativeMap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate a new map Data on the heap.
    /// Returns a `Value::Data` with `decl = map_decl` and `_items` native backing.
    pub fn alloc(
        decl: Option<DeclId>,
        key_type: crate::load::registry::TypeId,
        value_type: crate::load::registry::TypeId,
        heap: &mut Heap,
    ) -> Value {
        let native = NativeMap::new();
        let native_id = heap.alloc(HeapValue::Native(Box::new(native)));
        let mut fields = BTreeMap::new();
        fields.insert(Symbol::underscore_items(), Value::Native(native_id));
        let mut type_args = BTreeMap::new();
        type_args.insert(Symbol::k(), key_type);
        type_args.insert(Symbol::v(), value_type);
        Value::Data(heap.alloc(HeapValue::Data {
            decl,
            out_early_name: None,
            fields,
            type_args,
        }))
    }
}

impl NativeValue for NativeMap {
    fn gc_walk(&self, visitor: &mut dyn FnMut(&Value)) {
        for (k, v) in self.0.borrow().iter() {
            visitor(&k.value);
            visitor(v);
        }
    }

    fn native_hash(&self, heap: &Heap) -> Result<u64, String> {
        let map = self.0.borrow();
        let mut hasher = DefaultHasher::new();
        std::hash::Hash::hash(&map.len(), &mut hasher);
        // Order-independent: XOR entry hashes together.
        let mut combined: u64 = 0;
        for (k, v) in map.iter() {
            let mut entry_hasher = DefaultHasher::new();
            std::hash::Hash::hash(&k.cached_hash, &mut entry_hasher);
            std::hash::Hash::hash(&hash_value(v, heap)?, &mut entry_hasher);
            combined ^= entry_hasher.finish();
        }
        std::hash::Hash::hash(&combined, &mut hasher);
        Ok(hasher.finish())
    }
}

impl NativeValue for NativeArray {
    fn gc_walk(&self, visitor: &mut dyn FnMut(&Value)) {
        for v in self.0.borrow().iter() {
            visitor(v);
        }
    }

    fn native_hash(&self, heap: &Heap) -> Result<u64, String> {
        let mut hasher = DefaultHasher::new();
        let items = self.0.borrow();
        std::hash::Hash::hash(&items.len(), &mut hasher);
        for v in items.iter() {
            std::hash::Hash::hash(&hash_value(v, heap)?, &mut hasher);
        }
        Ok(hasher.finish())
    }
}
