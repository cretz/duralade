use std::collections::{BTreeMap, HashMap};

use super::EntityInstance;

struct CacheNode {
    instance: EntityInstance,
    order_key: u64,
}

pub struct EntityCache {
    map: HashMap<String, CacheNode>,
    order: BTreeMap<u64, String>,
    clock: u64,
    capacity: usize,
}

impl EntityCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            map: HashMap::new(),
            order: BTreeMap::new(),
            clock: 0,
            capacity,
        }
    }

    pub fn len(&self) -> usize {
        self.map.len()
    }

    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Borrow a cached instance without removing it. Does not update LRU order.
    pub fn get(&self, id: &str) -> Option<&EntityInstance> {
        self.map.get(id).map(|node| &node.instance)
    }

    /// Remove and return the instance. Used to temporarily extract for ticking,
    /// then re-insert via `insert`.
    pub fn take(&mut self, id: &str) -> Option<EntityInstance> {
        let node = self.map.remove(id)?;
        self.order.remove(&node.order_key);
        Some(node.instance)
    }

    /// Insert an instance at the MRU position. Evicts LRU if over capacity.
    pub fn insert(&mut self, instance: EntityInstance) {
        if self.capacity == 0 {
            return;
        }

        let id = instance.id().to_string();

        // Remove existing entry if present.
        if let Some(old) = self.map.remove(&id) {
            self.order.remove(&old.order_key);
        }

        // Evict LRU if at capacity.
        while self.map.len() >= self.capacity {
            let Some((&k, _)) = self.order.iter().next() else {
                break;
            };
            let evict_id = self.order.remove(&k).unwrap();
            self.map.remove(&evict_id);
        }

        self.clock += 1;
        self.order.insert(self.clock, id.clone());
        self.map.insert(
            id,
            CacheNode {
                instance,
                order_key: self.clock,
            },
        );
    }
}
