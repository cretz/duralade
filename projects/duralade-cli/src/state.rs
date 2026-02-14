use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

use url::Url;

use duralade_language::event::{Event, OwnedValue};
use duralade_runtime::state_store::{
    JsonFileStore, JsonMemoryStore, ListEntitiesFilter, StateStore,
};

use crate::EntityInputArgs;
use crate::EntityResultArgs;

/// Parse a raw `--state` or `--state-out` string into a normalized URL.
/// - Bare paths are resolved to absolute `file://` URLs.
/// - URLs with a scheme are accepted directly.
/// - Every returned URL has `?format=` set: derived from file extension for
///   `file://`, defaulted to `json` for `stdio://`.
pub fn parse_state_uri(raw: &str) -> Result<Url, String> {
    let mut url = if raw.contains("://") {
        Url::parse(raw).map_err(|e| format!("invalid URL '{raw}': {e}"))?
    } else {
        let path = Path::new(raw);
        let abs = if path.is_absolute() {
            path.to_path_buf()
        } else {
            std::env::current_dir()
                .map_err(|e| format!("cannot resolve relative path: {e}"))?
                .join(path)
        };
        Url::from_file_path(&abs)
            .map_err(|_| format!("cannot convert path to URL: {}", abs.display()))?
    };

    if !url.query_pairs().any(|(k, _)| k == "format") {
        let format = match url.scheme() {
            "stdio" => "json".to_string(),
            "file" => {
                let path = url
                    .to_file_path()
                    .map_err(|_| format!("invalid file URL: {url}"))?;
                path.extension()
                    .and_then(|e| e.to_str())
                    .ok_or_else(|| {
                        format!(
                            "cannot derive format from '{}': no file extension",
                            path.display()
                        )
                    })?
                    .to_string()
            }
            _ => return Ok(url),
        };
        url.set_query(Some(&format!("format={format}")));
    }

    Ok(url)
}

pub enum Store {
    File(JsonFileStore),
    /// In-memory store with optional write target for finalize.
    /// `None` = dry-run (no write).
    Memory(JsonMemoryStore, Option<Url>),
}

fn require_format_json(url: &Url) -> Result<(), String> {
    match url.query_pairs().find(|(k, _)| k == "format") {
        Some((_, v)) if v == "json" => Ok(()),
        Some((_, v)) => Err(format!(
            "unsupported state format '{v}' (only 'json' is supported)"
        )),
        None => Err("state URL missing format (expected ?format=json)".into()),
    }
}

impl Store {
    /// Open a state store from parsed, normalized URLs.
    /// Validates scheme (stdio or file), format (must be json), and out_url compatibility.
    pub fn open(in_url: &Url, out_url: Option<&Url>, dry_run: bool) -> Result<Store, String> {
        require_format_json(in_url)?;
        if let Some(out) = out_url {
            require_format_json(out)?;
        }

        match in_url.scheme() {
            "stdio" => {
                if out_url.is_some() {
                    return Err("--state-out cannot be used with stdio://".into());
                }
                let mut json = String::new();
                std::io::stdin()
                    .read_to_string(&mut json)
                    .map_err(|e| format!("read stdin: {e}"))?;
                let store = if json.trim().is_empty() {
                    JsonMemoryStore::empty()
                } else {
                    JsonMemoryStore::deserialize(&json)?
                };
                let target = if dry_run { None } else { Some(in_url.clone()) };
                Ok(Store::Memory(store, target))
            }
            "file" => {
                let in_path = in_url
                    .to_file_path()
                    .map_err(|_| format!("invalid file URL: {in_url}"))?;

                if let Some(out) = out_url
                    && out.scheme() != "file"
                {
                    return Err(
                        "--state-out must be a file path when --state is a file path".into(),
                    );
                }

                if dry_run {
                    let store = read_file_to_memory_store(&in_path)?;
                    Ok(Store::Memory(store, None))
                } else {
                    let out_path = match out_url {
                        Some(out) => out
                            .to_file_path()
                            .map_err(|_| format!("invalid file URL: {out}"))?,
                        None => in_path.clone(),
                    };
                    Ok(Store::File(JsonFileStore::open(&in_path, &out_path)?))
                }
            }
            s => Err(format!("unsupported state URI scheme '{s}://'")),
        }
    }

    /// Finalize store after all operations. File-backed stores already wrote on every
    /// mutation. Memory stores serialize and write to their configured target (if any).
    fn finalize_impl(&self) -> Result<(), String> {
        match self {
            Store::File(_) => Ok(()),
            Store::Memory(_, None) => {
                eprintln!("[dry-run] would write store");
                Ok(())
            }
            Store::Memory(s, Some(url)) => {
                let json = s.serialize()?;
                match url.scheme() {
                    "stdio" => {
                        let mut out = std::io::stdout();
                        out.write_all(json.as_bytes())
                            .map_err(|e| format!("write error: {e}"))?;
                        out.write_all(b"\n")
                            .map_err(|e| format!("write error: {e}"))
                    }
                    "file" => {
                        let path = url
                            .to_file_path()
                            .map_err(|_| format!("invalid file URL: {url}"))?;
                        std::fs::write(&path, json)
                            .map_err(|e| format!("write {}: {e}", path.display()))
                    }
                    _ => unreachable!("validated in open"),
                }
            }
        }
    }
}

fn read_file_to_memory_store(path: &Path) -> Result<JsonMemoryStore, String> {
    if path.exists() {
        let json =
            std::fs::read_to_string(path).map_err(|e| format!("read {}: {e}", path.display()))?;
        if json.trim().is_empty() {
            Ok(JsonMemoryStore::empty())
        } else {
            JsonMemoryStore::deserialize(&json)
        }
    } else {
        Ok(JsonMemoryStore::empty())
    }
}

impl StateStore for Store {
    async fn create_entity(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        match self {
            Store::File(s) => s.create_entity(id, events).await,
            Store::Memory(s, _) => s.create_entity(id, events).await,
        }
    }

    async fn write_events(&mut self, id: &str, events: Vec<Event>) -> Result<(), String> {
        match self {
            Store::File(s) => s.write_events(id, events).await,
            Store::Memory(s, _) => s.write_events(id, events).await,
        }
    }

    async fn read_events(&self, id: &str) -> Result<Vec<Event>, String> {
        match self {
            Store::File(s) => s.read_events(id).await,
            Store::Memory(s, _) => s.read_events(id).await,
        }
    }

    async fn list_entities(&self, filter: &ListEntitiesFilter) -> Result<Vec<String>, String> {
        match self {
            Store::File(s) => s.list_entities(filter).await,
            Store::Memory(s, _) => s.list_entities(filter).await,
        }
    }

    async fn entity_exists(&self, id: &str) -> Result<bool, String> {
        match self {
            Store::File(s) => s.entity_exists(id).await,
            Store::Memory(s, _) => s.entity_exists(id).await,
        }
    }

    async fn finalize(&self) -> Result<(), String> {
        self.finalize_impl()
    }
}

pub fn write_output(target: &str, value: &serde_json::Value, format: &str) -> Result<(), String> {
    let json = match format {
        "json-compact" => serde_json::to_string(value),
        "json" => serde_json::to_string_pretty(value),
        other => {
            return Err(format!(
                "unsupported output format '{other}' (expected 'json' or 'json-compact')"
            ));
        }
    }
    .map_err(|e| format!("serialize error: {e}"))?;
    write_to_target(target, json.as_bytes())
}

fn write_to_target(target: &str, data: &[u8]) -> Result<(), String> {
    if target == "stdio://" {
        let mut out = std::io::stdout();
        out.write_all(data)
            .map_err(|e| format!("write error: {e}"))?;
        out.write_all(b"\n")
            .map_err(|e| format!("write error: {e}"))?;
    } else {
        std::fs::write(target, data).map_err(|e| format!("cannot write '{target}': {e}"))?;
    }
    Ok(())
}

pub fn parse_input(args: &EntityInputArgs) -> Result<BTreeMap<String, OwnedValue>, String> {
    let json_str = match (&args.data, &args.file) {
        (Some(data), None) => data.clone(),
        (None, Some(path)) => {
            std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?
        }
        (None, None) => return Ok(BTreeMap::new()),
        (Some(_), Some(_)) => return Err("--in and --in-file are mutually exclusive".into()),
    };
    json_to_owned_map(&json_str)
}

pub fn parse_result(args: &EntityResultArgs) -> Result<BTreeMap<String, OwnedValue>, String> {
    let json_str = match (&args.result, &args.result_file) {
        (Some(data), None) => data.clone(),
        (None, Some(path)) => {
            std::fs::read_to_string(path).map_err(|e| format!("cannot read '{path}': {e}"))?
        }
        (None, None) => return Ok(BTreeMap::new()),
        (Some(_), Some(_)) => {
            return Err("--result and --result-file are mutually exclusive".into());
        }
    };
    json_to_owned_map(&json_str)
}

fn json_to_owned_map(json: &str) -> Result<BTreeMap<String, OwnedValue>, String> {
    serde_json::from_str(json).map_err(|e| format!("invalid JSON: {e}"))
}

pub fn owned_to_json(v: &OwnedValue) -> serde_json::Value {
    serde_json::to_value(v).expect("OwnedValue serialization cannot fail")
}

pub fn owned_map_to_json(map: &BTreeMap<String, OwnedValue>) -> serde_json::Value {
    serde_json::to_value(map).expect("OwnedValue map serialization cannot fail")
}
