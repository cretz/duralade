use std::collections::BTreeMap;
use std::future::Future;
use std::path::{Path, PathBuf};

use duralade_language::event::{Event, EventType};

#[derive(Default)]
pub struct ListEntitiesFilter {
    pub running_only: bool,
}

/// A collective state store holding multiple entities' event logs.
pub trait StateStore {
    /// Create a new entity with its initial events. Errors if entity already exists.
    fn create_entity(
        &mut self,
        id: &str,
        events: Vec<Event>,
    ) -> impl Future<Output = Result<(), String>> + Send;

    /// Replace all events for an existing entity. Errors if entity doesn't exist.
    fn write_events(
        &mut self,
        id: &str,
        events: Vec<Event>,
    ) -> impl Future<Output = Result<(), String>> + Send;

    /// Read all events for an entity. Errors if entity doesn't exist.
    // TODO: Consider streaming/pagination for large event logs (e.g. Stream<Item = Event>).
    fn read_events(&self, id: &str) -> impl Future<Output = Result<Vec<Event>, String>> + Send;

    /// List entity IDs, optionally filtered.
    fn list_entities(
        &self,
        filter: &ListEntitiesFilter,
    ) -> impl Future<Output = Result<Vec<String>, String>> + Send;

    /// Check if an entity exists.
    fn entity_exists(&self, id: &str) -> impl Future<Output = Result<bool, String>> + Send;

    /// Flush any buffered state. Default no-op for stores that write eagerly.
    fn finalize(&self) -> impl Future<Output = Result<(), String>> + Send {
        async { Ok(()) }
    }
}

/// File-backed state store. Reads from one path on open, writes the entire file to
/// another path on every mutation (read and write paths may be the same).
// TODO: Option to defer writes (batch mode) for callers that want explicit save control.
pub struct JsonFileStore {
    write_path: PathBuf,
    entities: BTreeMap<String, Vec<Event>>,
}

impl JsonFileStore {
    /// Open a JSON state file. Reads from `read_path`, writes to `write_path` on every mutation.
    pub fn open(read_path: &Path, write_path: &Path) -> Result<Self, String> {
        let entities = if read_path.exists() {
            let json = std::fs::read_to_string(read_path)
                .map_err(|e| format!("read {}: {e}", read_path.display()))?;
            serde_json::from_str(&json)
                .map_err(|e| format!("parse {}: {e}", read_path.display()))?
        } else {
            BTreeMap::new()
        };
        Ok(Self {
            write_path: write_path.to_path_buf(),
            entities,
        })
    }

    fn write_file(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.entities)
            .map_err(|e| format!("serialize error: {e}"))?;
        std::fs::write(&self.write_path, json)
            .map_err(|e| format!("write {}: {e}", self.write_path.display()))
    }
}

impl StateStore for JsonFileStore {
    async fn create_entity(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        if self.entities.contains_key(id) {
            return Err(format!("entity '{id}' already exists"));
        }
        self.entities.insert(id.to_string(), events);
        self.write_file()
    }

    async fn write_events(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        if !self.entities.contains_key(id) {
            return Err(format!("entity '{id}' does not exist"));
        }
        self.entities.insert(id.to_string(), events);
        self.write_file()
    }

    async fn read_events(&self, id: &str) -> Result<Vec<Event>, String> {
        self.entities
            .get(id)
            .cloned()
            .ok_or_else(|| format!("entity '{id}' not found"))
    }

    async fn list_entities(&self, filter: &ListEntitiesFilter) -> Result<Vec<String>, String> {
        Ok(filter_entity_list(&self.entities, filter))
    }

    async fn entity_exists(&self, id: &str) -> Result<bool, String> {
        Ok(self.entities.contains_key(id))
    }
}

/// In-memory state store. Use `deserialize`/`serialize` for JSON string I/O.
pub struct JsonMemoryStore {
    entities: BTreeMap<String, Vec<Event>>,
}

impl JsonMemoryStore {
    pub fn empty() -> Self {
        Self {
            entities: BTreeMap::new(),
        }
    }

    pub fn deserialize(json: &str) -> Result<Self, String> {
        let entities: BTreeMap<String, Vec<Event>> =
            serde_json::from_str(json).map_err(|e| format!("invalid state JSON: {e}"))?;
        Ok(Self { entities })
    }

    pub fn serialize(&self) -> Result<String, String> {
        serde_json::to_string_pretty(&self.entities).map_err(|e| format!("serialize error: {e}"))
    }

    pub fn into_entities(self) -> BTreeMap<String, Vec<Event>> {
        self.entities
    }

    pub fn from_entities(entities: BTreeMap<String, Vec<Event>>) -> Self {
        Self { entities }
    }
}

impl StateStore for JsonMemoryStore {
    async fn create_entity(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        if self.entities.contains_key(id) {
            return Err(format!("entity '{id}' already exists"));
        }
        self.entities.insert(id.to_string(), events);
        Ok(())
    }

    async fn write_events(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        if !self.entities.contains_key(id) {
            return Err(format!("entity '{id}' does not exist"));
        }
        self.entities.insert(id.to_string(), events);
        Ok(())
    }

    async fn read_events(&self, id: &str) -> Result<Vec<Event>, String> {
        self.entities
            .get(id)
            .cloned()
            .ok_or_else(|| format!("entity '{id}' not found"))
    }

    async fn list_entities(&self, filter: &ListEntitiesFilter) -> Result<Vec<String>, String> {
        Ok(filter_entity_list(&self.entities, filter))
    }

    async fn entity_exists(&self, id: &str) -> Result<bool, String> {
        Ok(self.entities.contains_key(id))
    }
}

fn is_completed(events: &[Event]) -> bool {
    events
        .iter()
        .rev()
        .any(|e| matches!(&e.event_type, EventType::EntityComplete { .. }))
}

fn filter_entity_list(
    entities: &BTreeMap<String, Vec<Event>>,
    filter: &ListEntitiesFilter,
) -> Vec<String> {
    entities
        .iter()
        .filter(|(_, events)| !filter.running_only || !is_completed(events))
        .map(|(id, _)| id.clone())
        .collect()
}
