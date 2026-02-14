use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

use crate::interpret::native_collection::{MapKey, NativeArray, NativeMap};
use crate::interpret::value::{EntityId, Heap, HeapValue, Value, ValueId};
use crate::load::registry::{DeclId, Registry, TypeEntry, TypeId};
use crate::load::{FieldModifier, TypeConstructKind};
use crate::model::Symbol;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub num: u64,
    pub time: u64,
    #[serde(flatten)]
    pub event_type: EventType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum EventType {
    ExternInvoke {
        #[serde(rename = "extern")]
        extern_name: String,
        args: OwnedFields,
    },
    ExternComplete {
        invoke_num: u64,
        result: OwnedFields,
    },
    FuncInvoke {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        func: String,
        args: OwnedFields,
    },
    FuncComplete {
        invoke_num: u64,
        result: OwnedFields,
    },
    EntityInvoke {
        id: String,
        entity: String,
        args: OwnedFields,
    },
    EntityComplete {
        terminated: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<OwnedFields>,
    },
    EntityCancel {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    FuncCancel {
        invoke_num: u64,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
}

/// Named fields with owned values. Used in EventType args/results and OwnedValue::Data.
pub type OwnedFields = BTreeMap<String, OwnedValue>;

/// Fully-owned value for durable event storage. No heap references.
/// `Data` = string-keyed fields (serializes as JSON object).
/// `Map` = arbitrary-keyed entries (serializes as tagged array of pairs).
#[derive(Debug, Clone)]
pub enum OwnedValue {
    Int(i64),
    Float(f64),
    Str(String),
    Bool(bool),
    Nil,
    List(Vec<OwnedValue>),
    Data(OwnedFields),
    Map(Vec<(OwnedValue, OwnedValue)>),
    EntityRef(EntityId),
}

#[derive(Debug, Clone)]
pub struct OwnedValueError(pub String);

impl fmt::Display for OwnedValueError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl Serialize for OwnedValue {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        match self {
            OwnedValue::Int(n) => serializer.serialize_i64(*n),
            OwnedValue::Float(f) => serializer.serialize_f64(*f),
            OwnedValue::Str(s) => serializer.serialize_str(s),
            OwnedValue::Bool(b) => serializer.serialize_bool(*b),
            OwnedValue::Nil => serializer.serialize_unit(),
            OwnedValue::List(items) => items.serialize(serializer),
            OwnedValue::Data(entries) => entries.serialize(serializer),
            OwnedValue::Map(entries) => {
                let pairs: Vec<(&OwnedValue, &OwnedValue)> =
                    entries.iter().map(|(k, v)| (k, v)).collect();
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("$duralade.map", &pairs)?;
                map.end()
            }
            OwnedValue::EntityRef(id) => {
                let mut map = serializer.serialize_map(Some(1))?;
                map.serialize_entry("$duralade.entity_ref", &id.0)?;
                map.end()
            }
        }
    }
}

impl<'de> Deserialize<'de> for OwnedValue {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(OwnedValueVisitor)
    }
}

struct OwnedValueVisitor;

impl<'de> serde::de::Visitor<'de> for OwnedValueVisitor {
    type Value = OwnedValue;

    fn expecting(&self, formatter: &mut std::fmt::Formatter) -> std::fmt::Result {
        formatter.write_str("a JSON value")
    }

    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Bool(v))
    }

    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Int(v))
    }

    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Int(v as i64))
    }

    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Float(v))
    }

    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Str(v.to_string()))
    }

    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Str(v))
    }

    fn visit_none<E: serde::de::Error>(self) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Nil)
    }

    fn visit_unit<E: serde::de::Error>(self) -> Result<OwnedValue, E> {
        Ok(OwnedValue::Nil)
    }

    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<OwnedValue, A::Error> {
        let mut items = Vec::new();
        while let Some(item) = seq.next_element()? {
            items.push(item);
        }
        Ok(OwnedValue::List(items))
    }

    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<OwnedValue, A::Error> {
        let mut entries: OwnedFields = BTreeMap::new();
        while let Some((key, value)) = map.next_entry::<String, OwnedValue>()? {
            entries.insert(key, value);
        }
        if let Some(OwnedValue::Str(id)) = entries.remove("$duralade.entity_ref") {
            return Ok(OwnedValue::EntityRef(EntityId(id)));
        }
        if let Some(OwnedValue::List(pairs)) = entries.remove("$duralade.map") {
            let mut map_entries = Vec::with_capacity(pairs.len());
            for pair in pairs {
                let OwnedValue::List(mut kv) = pair else {
                    return Err(serde::de::Error::custom(
                        "$duralade.map entries must be [key, value] pairs",
                    ));
                };
                if kv.len() != 2 {
                    return Err(serde::de::Error::custom(
                        "$duralade.map entries must be [key, value] pairs",
                    ));
                }
                let v = kv.pop().unwrap();
                let k = kv.pop().unwrap();
                map_entries.push((k, v));
            }
            return Ok(OwnedValue::Map(map_entries));
        }
        Ok(OwnedValue::Data(entries))
    }
}

impl OwnedValue {
    /// Materialize a runtime Value into a fully-owned OwnedValue by deep-copying from the heap.
    pub fn from_value(value: &Value, heap: &Heap) -> Result<Self, OwnedValueError> {
        Ok(match value {
            Value::Int(n) => OwnedValue::Int(*n),
            Value::Float(f) => OwnedValue::Float(*f),
            Value::Str(s) => OwnedValue::Str(s.to_string()),
            Value::Bool(b) => OwnedValue::Bool(*b),
            Value::Nil => OwnedValue::Nil,
            Value::EntityRef(id) => OwnedValue::EntityRef(id.clone()),
            Value::Data(id) => Self::materialize_heap(id.id(), heap)?,
            Value::Func(_) => {
                return Err(OwnedValueError(
                    "functions cannot be converted to OwnedValue".into(),
                ));
            }
            Value::Entity(_) => {
                return Err(OwnedValueError(
                    "entities cannot be converted to OwnedValue".into(),
                ));
            }
            Value::Native(_) => {
                return Err(OwnedValueError(
                    "native objects cannot be converted to OwnedValue".into(),
                ));
            }
            Value::Type(_) => {
                return Err(OwnedValueError(
                    "type values cannot be converted to OwnedValue".into(),
                ));
            }
        })
    }

    fn materialize_heap(id: ValueId, heap: &Heap) -> Result<Self, OwnedValueError> {
        Ok(match heap.get(id) {
            HeapValue::Data { fields, .. } => {
                // Native-backed collections: extract _items
                if let Some(Value::Native(native_id)) = fields.get("_items")
                    && let HeapValue::Native(native) = heap.get(native_id.id())
                {
                    if let Some(arr) =
                        (native.as_ref() as &dyn std::any::Any).downcast_ref::<NativeArray>()
                    {
                        return Ok(OwnedValue::List(
                            arr.0
                                .borrow()
                                .iter()
                                .map(|v| OwnedValue::from_value(v, heap))
                                .collect::<Result<_, _>>()?,
                        ));
                    }
                    if let Some(m) =
                        (native.as_ref() as &dyn std::any::Any).downcast_ref::<NativeMap>()
                    {
                        return Ok(OwnedValue::Map(
                            m.0.borrow()
                                .iter()
                                .map(|(k, v)| {
                                    Ok((
                                        OwnedValue::from_value(k.value(), heap)?,
                                        OwnedValue::from_value(v, heap)?,
                                    ))
                                })
                                .collect::<Result<_, OwnedValueError>>()?,
                        ));
                    }
                }
                OwnedValue::Data(
                    fields
                        .iter()
                        .map(|(k, v)| Ok((k.to_string(), OwnedValue::from_value(v, heap)?)))
                        .collect::<Result<_, OwnedValueError>>()?,
                )
            }
            HeapValue::Entity { .. } => {
                return Err(OwnedValueError(
                    "entities cannot be converted to OwnedValue".into(),
                ));
            }
            HeapValue::Func { .. } => {
                return Err(OwnedValueError(
                    "functions cannot be converted to OwnedValue".into(),
                ));
            }
            HeapValue::Native(_) => {
                return Err(OwnedValueError(
                    "native objects cannot be converted to OwnedValue".into(),
                ));
            }
        })
    }

    fn owned_type_name(&self) -> &'static str {
        match self {
            OwnedValue::Int(_) => "int",
            OwnedValue::Float(_) => "float",
            OwnedValue::Str(_) => "str",
            OwnedValue::Bool(_) => "bool",
            OwnedValue::Nil => "nil",
            OwnedValue::List(_) => "list",
            OwnedValue::Data(_) => "data",
            OwnedValue::Map(_) => "map",
            OwnedValue::EntityRef(_) => "entity_ref",
        }
    }

    fn type_mismatch(&self, ty: TypeId, registry: &Registry) -> OwnedValueError {
        OwnedValueError(format!(
            "type mismatch: expected {}, got {}",
            registry.display_type(ty),
            self.owned_type_name()
        ))
    }

    /// Internalize an OwnedValue back into the heap, returning a runtime Value.
    /// Uses `ty` to decide whether a Map becomes `Value::Data` or `Value::Map`,
    /// and validates that the OwnedValue shape matches the expected type.
    pub fn into_value(
        self,
        ty: TypeId,
        registry: &Registry,
        heap: &mut Heap,
    ) -> Result<Value, OwnedValueError> {
        match registry.type_entry(ty) {
            TypeEntry::Any | TypeEntry::AnyLocal | TypeEntry::Error | TypeEntry::TypeParam(_) => {
                self.into_value_untyped(heap)
            }
            TypeEntry::Nilable(inner) => {
                if matches!(self, OwnedValue::Nil) {
                    return Ok(Value::Nil);
                }
                self.into_value(*inner, registry, heap)
            }
            TypeEntry::EntityRef(_) => match self {
                OwnedValue::EntityRef(id) => Ok(Value::EntityRef(id)),
                _ => Err(self.type_mismatch(ty, registry)),
            },
            TypeEntry::Named { decl, .. } if *decl == registry.int_decl => match self {
                OwnedValue::Int(v) => Ok(Value::Int(v)),
                _ => Err(self.type_mismatch(ty, registry)),
            },
            TypeEntry::Named { decl, .. } if *decl == registry.float_decl => match self {
                OwnedValue::Float(v) => Ok(Value::Float(v)),
                _ => Err(self.type_mismatch(ty, registry)),
            },
            TypeEntry::Named { decl, .. } if *decl == registry.str_decl => match self {
                OwnedValue::Str(s) => Ok(Value::Str(s.into())),
                _ => Err(self.type_mismatch(ty, registry)),
            },
            TypeEntry::Named { decl, .. } if *decl == registry.bool_decl => match self {
                OwnedValue::Bool(b) => Ok(Value::Bool(b)),
                _ => Err(self.type_mismatch(ty, registry)),
            },
            TypeEntry::Named { decl, .. } => self.into_value_named(*decl, registry, heap),
            TypeEntry::MemberFunc { .. } => Err(self.type_mismatch(ty, registry)),
            TypeEntry::Anonymous(anon) => {
                // Clone to release borrow on registry
                let anon = anon.clone();
                self.into_value_anonymous(&anon, registry, heap)
            }
        }
    }

    fn into_value_untyped(self, heap: &mut Heap) -> Result<Value, OwnedValueError> {
        match self {
            OwnedValue::Int(n) => Ok(Value::Int(n)),
            OwnedValue::Float(f) => Ok(Value::Float(f)),
            OwnedValue::Str(s) => Ok(Value::Str(s.into())),
            OwnedValue::Bool(b) => Ok(Value::Bool(b)),
            OwnedValue::Nil => Ok(Value::Nil),
            OwnedValue::EntityRef(id) => Ok(Value::EntityRef(id)),
            OwnedValue::List(items) => {
                let items = items
                    .into_iter()
                    .map(|v| v.into_value_untyped(heap))
                    .collect::<Result<Vec<_>, _>>()?;
                Ok(NativeArray::alloc(items, None, TypeId::ANY, heap))
            }
            OwnedValue::Data(entries) => {
                let fields = entries
                    .into_iter()
                    .map(|(k, v)| v.into_value_untyped(heap).map(|val| (k.into(), val)))
                    .collect::<Result<BTreeMap<Symbol, Value>, _>>()?;
                Ok(Value::Data(heap.alloc(HeapValue::Data {
                    decl: None,
                    out_early_name: None,
                    fields,
                    type_args: BTreeMap::new(),
                })))
            }
            OwnedValue::Map(entries) => {
                // Convert all entries to Values first (may allocate on heap),
                // then build the NativeMap.
                let mut converted = Vec::with_capacity(entries.len());
                for (k, v) in entries {
                    let key = k.into_value_untyped(heap)?;
                    let val = v.into_value_untyped(heap)?;
                    converted.push((key, val));
                }
                let map_value = NativeMap::alloc(None, TypeId::ANY, TypeId::ANY, heap);
                for (key, val) in converted {
                    let map_key = MapKey::new(key, heap)
                        .map_err(|msg| OwnedValueError(format!("map key not hashable: {msg}")))?;
                    let Value::Data(data_id) = &map_value else {
                        return Err(OwnedValueError("map alloc failed".into()));
                    };
                    let HeapValue::Data { fields, .. } = heap.get(data_id.id()) else {
                        return Err(OwnedValueError("map data missing".into()));
                    };
                    let Some(Value::Native(native_id)) = fields.get("_items") else {
                        return Err(OwnedValueError("map _items missing".into()));
                    };
                    let HeapValue::Native(native) = heap.get(native_id.id()) else {
                        return Err(OwnedValueError("map _items not native".into()));
                    };
                    let Some(nm) =
                        (native.as_ref() as &dyn std::any::Any).downcast_ref::<NativeMap>()
                    else {
                        return Err(OwnedValueError("map _items not NativeMap".into()));
                    };
                    nm.0.borrow_mut().insert(map_key, val);
                }
                Ok(map_value)
            }
        }
    }

    fn into_value_named(
        self,
        decl_id: DeclId,
        registry: &Registry,
        heap: &mut Heap,
    ) -> Result<Value, OwnedValueError> {
        let entry = registry.decl(decl_id);
        let decl_name = entry.name.clone();
        let module_idx = entry.module;
        let module = registry.module(module_idx);
        let Some(decl) = module.get_declaration(&decl_name) else {
            return Err(OwnedValueError(format!(
                "'{}' not found in module '{}'",
                decl_name,
                registry.module_path(module_idx)
            )));
        };
        match decl.kind {
            TypeConstructKind::Data | TypeConstructKind::Extern | TypeConstructKind::Native => {
                let OwnedValue::Data(entries) = self else {
                    return Err(OwnedValueError(format!(
                        "type mismatch: expected {}, got {}",
                        decl_name,
                        self.owned_type_name()
                    )));
                };
                convert_fields(entries, &decl.fields, &decl_name, registry, heap)
            }
            TypeConstructKind::Entity => Err(OwnedValueError(format!(
                "cannot deserialize into entity '{}' (use entity ref type instead)",
                decl_name
            ))),
            _ => Err(OwnedValueError(format!(
                "cannot deserialize into {:?} '{}'",
                decl.kind, decl_name
            ))),
        }
    }

    fn into_value_anonymous(
        self,
        anon: &crate::load::TypeConstruct,
        registry: &Registry,
        heap: &mut Heap,
    ) -> Result<Value, OwnedValueError> {
        use crate::load::TypeConstructKind;
        match anon.kind {
            TypeConstructKind::Data => {
                let OwnedValue::Data(entries) = self else {
                    return Err(OwnedValueError(format!(
                        "type mismatch: expected anonymous data, got {}",
                        self.owned_type_name()
                    )));
                };
                convert_anon_fields(entries, &anon.fields, registry, heap)
            }
            _ => Err(OwnedValueError(format!(
                "cannot deserialize into anonymous {:?}",
                anon.kind
            ))),
        }
    }
}

pub fn is_field_required(field: &crate::load::TypeField, registry: &Registry) -> bool {
    matches!(
        field.modifier,
        Some(FieldModifier::In) | Some(FieldModifier::Inout)
    ) && !field.has_default
        && !registry.is_nilable(field.ty)
}

fn convert_fields(
    mut entries: OwnedFields,
    decl_fields: &[crate::load::TypeField],
    type_name: &str,
    registry: &Registry,
    heap: &mut Heap,
) -> Result<Value, OwnedValueError> {
    let out_early_name = decl_fields.iter().find_map(|f| {
        if f.modifier == Some(FieldModifier::OutEarly) {
            Some(f.name.clone())
        } else {
            None
        }
    });
    let mut fields = BTreeMap::new();
    for decl_field in decl_fields {
        let field_name = &decl_field.name;
        if let Some(owned) = entries.remove(field_name.as_str()) {
            let val = owned.into_value(decl_field.ty, registry, heap)?;
            fields.insert(field_name.clone(), val);
        } else if is_field_required(decl_field, registry) {
            return Err(OwnedValueError(format!(
                "missing required field '{}' for type '{}'",
                field_name, type_name
            )));
        }
        // Non-required missing fields are omitted - caller fills defaults.
    }
    // Extra entries not in the declaration are dropped.
    Ok(Value::Data(heap.alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields,
        type_args: BTreeMap::new(),
    })))
}

fn is_anon_field_required(field: &crate::load::TypeField, registry: &Registry) -> bool {
    !field.has_default && !registry.is_nilable(field.ty)
}

fn convert_anon_fields(
    mut entries: OwnedFields,
    anon_fields: &[crate::load::TypeField],
    registry: &Registry,
    heap: &mut Heap,
) -> Result<Value, OwnedValueError> {
    let out_early_name = anon_fields.iter().find_map(|f| {
        if f.modifier == Some(FieldModifier::OutEarly) {
            Some(f.name.clone())
        } else {
            None
        }
    });
    let mut fields = BTreeMap::new();
    for anon_field in anon_fields {
        let name = &anon_field.name;
        if let Some(owned) = entries.remove(name.as_str()) {
            let val = owned.into_value(anon_field.ty, registry, heap)?;
            fields.insert(name.clone(), val);
        } else if is_anon_field_required(anon_field, registry) {
            return Err(OwnedValueError(format!(
                "missing required field '{}' in anonymous data",
                name
            )));
        }
    }
    Ok(Value::Data(heap.alloc(HeapValue::Data {
        decl: None,
        out_early_name,
        fields,
        type_args: BTreeMap::new(),
    })))
}

/// Result of checking the event log for an extern replay.
pub enum ExternReplayResult {
    /// ExternInvoke + ExternComplete found. Contains the result fields.
    Completed(OwnedFields),
    /// ExternInvoke found but no ExternComplete - extern still pending.
    /// Contains the invoke's event_num for later completion lookup.
    Pending(u64),
    /// No matching ExternInvoke at cursor - this is a new extern call.
    NotFound,
    /// ExternInvoke exists at cursor but name doesn't match - replay divergence.
    Diverged(String),
}

/// Event log shared between the interpreter future and the entity instance
/// via Rc<RefCell<EventLog>>.
#[derive(Debug, Default)]
pub struct EventLog {
    pub events: Vec<Event>,
    pub cursor: usize,
    pub next_event_num: u64,
    pub current_time_ms: u64,
}

impl EventLog {
    pub fn from_events(events: Vec<Event>, next_event_num: u64) -> Self {
        Self {
            events,
            cursor: 0,
            next_event_num,
            current_time_ms: 0,
        }
    }

    /// Check whether the current cursor position has a matching ExternInvoke+ExternComplete
    /// pair for replay. Advances the cursor accordingly.
    pub fn try_replay_extern(&mut self, name: &str) -> ExternReplayResult {
        tracing::trace!(name, cursor = self.cursor, "trying extern replay");
        if self.cursor >= self.events.len() {
            return ExternReplayResult::NotFound;
        }
        if let EventType::ExternInvoke {
            extern_name: ref n, ..
        } = self.events[self.cursor].event_type
        {
            if n != name {
                return ExternReplayResult::Diverged(format!(
                    "replay divergence: expected extern '{}', got '{}'",
                    n, name
                ));
            }
            let invoke_num = self.events[self.cursor].num;
            self.cursor += 1;

            if self.cursor < self.events.len()
                && let EventType::ExternComplete {
                    invoke_num: num,
                    ref result,
                } = self.events[self.cursor].event_type
                && num == invoke_num
            {
                let result = result.clone();
                self.cursor += 1;
                return ExternReplayResult::Completed(result);
            }
            return ExternReplayResult::Pending(invoke_num);
        }
        ExternReplayResult::NotFound
    }

    /// Check if a specific ExternInvoke has been completed.
    pub fn check_extern_complete(&self, invoke_num: u64) -> Option<OwnedFields> {
        self.events.iter().find_map(|e| {
            if let EventType::ExternComplete {
                invoke_num: n,
                ref result,
            } = e.event_type
                && n == invoke_num
            {
                return Some(result.clone());
            }
            None
        })
    }

    /// Append a new event. Returns the assigned event_num.
    /// Advances the cursor past the new event so `try_replay_extern` won't
    /// attempt to replay freshly-written events.
    pub fn append(&mut self, event_type: EventType) -> u64 {
        let num = self.next_event_num;
        self.next_event_num += 1;
        tracing::trace!(num, "event appended");
        self.events.push(Event {
            num,
            time: self.current_time_ms,
            event_type,
        });
        self.cursor = self.events.len();
        num
    }
}
