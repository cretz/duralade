use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use duralade_language::engine::{
    CallExternInfo, CoroutineInfo, Engine, EngineError, EngineTickResult, EntityFromEventsOptions,
    EntityInstance, EntitySpawnOptions, EntityStatus, ResultExternInfo, SpawnExternInfo,
    ViewResult, cache::EntityCache, find_call_extern, find_result_extern, find_spawn_extern,
    native::NativeRegistry,
};
use duralade_language::event::{Event, EventType, OwnedValue};
use duralade_language::interpret::scope::ImplicitInit;
use duralade_language::interpret::tick::TickStatus;
use duralade_language::interpret::value::EntityId;
use duralade_language::load::Registry;

use crate::state_store::{ListEntitiesFilter, StateStore};

pub struct LocalEngineOptions {
    pub disable_heap_collect: bool,
    pub cache_capacity: usize,
    /// When true, completed entities remain in cache instead of being dropped.
    /// Enables post-test GC verification via `into_drop_details().assert_clean()`.
    pub cache_completed: bool,
}

// TODO: Review read-modify-write patterns for atomicity when a real DB store is introduced.
pub struct LocalEngine<S: StateStore> {
    registry: Arc<Registry>,
    store: S,
    native_registry: Rc<NativeRegistry>,
    options: LocalEngineOptions,
    cache: RefCell<EntityCache>,
}

impl<S: StateStore> LocalEngine<S> {
    pub fn new(registry: Arc<Registry>, store: S, options: LocalEngineOptions) -> Self {
        let mut native_registry = NativeRegistry::new();
        duralade_stdlib::apply_to_native_registry(&mut native_registry)
            .expect("failed to register stdlib native handlers");

        Self::with_native_registry(registry, store, Rc::new(native_registry), options)
    }

    pub fn with_native_registry(
        registry: Arc<Registry>,
        store: S,
        native_registry: Rc<NativeRegistry>,
        options: LocalEngineOptions,
    ) -> Self {
        let cache = RefCell::new(EntityCache::new(options.cache_capacity));
        Self {
            registry,
            store,
            native_registry,
            options,
            cache,
        }
    }

    pub fn cache(&self) -> &RefCell<EntityCache> {
        &self.cache
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn store_mut(&mut self) -> &mut S {
        &mut self.store
    }

    pub fn into_store(self) -> S {
        self.store
    }

    fn should_cache(&self, inst: &EntityInstance) -> bool {
        !inst.is_done() || self.options.cache_completed
    }

    // TODO: Check dirtiness - the cached instance may be stale if external events
    // were appended to the store since the last cache write. A generation counter
    // or dirty flag on the cache entry could detect this.
    async fn load_instance(&self, id: &str) -> Result<EntityInstance, EngineError> {
        if let Some(cached) = self.cache.borrow_mut().take(id) {
            Ok(cached)
        } else {
            let events = self.store.read_events(id).await?;
            Ok(EntityInstance::from_events(EntityFromEventsOptions {
                registry: self.registry.clone(),
                events,
                native_registry: self.native_registry.clone(),
                until_event_num: None,
                disable_heap_collect: self.options.disable_heap_collect,
            })?)
        }
    }

    /// Like load_instance, but ticks to materialize state on cache miss.
    /// Use for read-only operations (view, inspect) that need state but won't persist events.
    /// When `after_event_num` is Some, bypasses cache and truncates events.
    async fn load_instance_for_view(
        &self,
        id: &str,
        after_event_num: Option<u64>,
    ) -> Result<EntityInstance, EngineError> {
        let mut inst = if after_event_num.is_none() {
            self.load_instance(id).await?
        } else {
            EntityInstance::from_events(EntityFromEventsOptions {
                registry: self.registry.clone(),
                events: self.store.read_events(id).await?,
                native_registry: self.native_registry.clone(),
                until_event_num: after_event_num,
                disable_heap_collect: self.options.disable_heap_collect,
            })?
        };
        if !inst.is_done() && inst.is_replaying() {
            inst.tick(0)
                .map_err(|e| e.into_engine_error(inst.events()))?;
        }
        Ok(inst)
    }

    async fn store_instance(&mut self, inst: EntityInstance) -> Result<(), EngineError> {
        let id = inst.id().to_string();
        self.store.write_events(&id, inst.events().to_vec()).await?;
        if self.should_cache(&inst) {
            self.cache.borrow_mut().insert(inst);
        }
        Ok(())
    }

    // TODO: tick_routed rescans all new_events with find_*_extern on every iteration.
    // The tick result should carry the pending extern directly instead of requiring
    // a linear search each time.
    #[tracing::instrument(level = "debug", skip(self, inst), fields(id = %inst.id()))]
    async fn tick_routed(
        &mut self,
        inst: &mut EntityInstance,
        time_ms: u64,
    ) -> Result<(TickStatus, Vec<String>), EngineError> {
        let mut spawned = Vec::new();

        loop {
            let result = inst
                .tick(time_ms)
                .map_err(|e| e.into_engine_error(inst.events()))?;

            if !matches!(result.status, TickStatus::Blocked) {
                return Ok((result.status, spawned));
            }

            let new_start = inst.events().len() - result.new_event_count;
            if let Some(info) = find_spawn_extern(&inst.events()[new_start..]) {
                tracing::trace!("routing spawn extern");
                let spawned_id = self.handle_spawn(inst, &info, time_ms).await?;
                spawned.push(spawned_id);
                continue;
            }
            if let Some(info) = find_result_extern(&inst.events()[new_start..])
                && self.handle_result(inst, &info, time_ms).await?
            {
                tracing::trace!("routing result extern");
                continue;
            }
            if let Some(info) = find_call_extern(&inst.events()[new_start..])
                && self.handle_call(inst, &info, time_ms).await?
            {
                tracing::trace!("routing call extern");
                continue;
            }
            // Check for overridden externs (test-time feature).
            let handled = self.handle_extern_override(inst, new_start, time_ms)?;
            if handled {
                tracing::trace!("routing overridden extern");
                continue;
            }

            tracing::trace!("no routable extern, returning blocked");
            return Ok((TickStatus::Blocked, spawned));
        }
    }

    #[tracing::instrument(level = "debug", skip_all, fields(entity_type = %info.entity_type))]
    async fn handle_spawn(
        &mut self,
        caller: &mut EntityInstance,
        info: &SpawnExternInfo,
        time_ms: u64,
    ) -> Result<String, EngineError> {
        let target_id = info
            .id
            .clone()
            .unwrap_or_else(|| format!("{}:spawn:{}", caller.id(), info.invoke_num));

        if !self.store.entity_exists(&target_id).await? {
            let mut inst = EntityInstance::spawn(EntitySpawnOptions {
                registry: self.registry.clone(),
                id: target_id.clone(),
                entity_type: info.entity_type.clone(),
                args: info.args.clone(),
                initial_entry_implicits: Vec::new(),
                native_registry: self.native_registry.clone(),
                extern_overrides: Some(caller.extern_overrides()),
                time_ms,
                disable_heap_collect: self.options.disable_heap_collect,
            })?;
            // Use tick_routed so child entities get override routing too.
            // Box::pin for recursive async (tick_routed → handle_spawn → tick_routed).
            Box::pin(self.tick_routed(&mut inst, time_ms)).await?;
            self.store
                .create_entity(&target_id, inst.events().to_vec())
                .await?;
            if self.should_cache(&inst) {
                self.cache.borrow_mut().insert(inst);
            }
        }

        let mut result = BTreeMap::new();
        result.insert(
            "entity".to_string(),
            OwnedValue::EntityRef(EntityId(target_id.clone())),
        );
        result.insert("error".to_string(), OwnedValue::Nil);
        caller.complete_extern(info.invoke_num, result, time_ms)?;

        Ok(target_id)
    }

    #[tracing::instrument(level = "debug", skip_all, fields(target_id = %info.entity_id, func = %info.func_name))]
    async fn handle_call(
        &mut self,
        caller: &mut EntityInstance,
        info: &CallExternInfo,
        time_ms: u64,
    ) -> Result<bool, EngineError> {
        let mut target = self.load_instance(&info.entity_id).await?;

        if target.is_done() {
            let mut result = BTreeMap::new();
            result.insert(
                "error".to_string(),
                OwnedValue::Str("target entity is completed".into()),
            );
            caller.complete_extern(info.invoke_num, result, time_ms)?;
            self.store_instance(target).await?;
            return Ok(true);
        }

        // Append FuncInvoke and tick.
        let func_event_num =
            target.invoke_func(info.func_name.clone(), None, info.args.clone(), time_ms)?;
        // Box::pin for recursive async (tick_routed → handle_call → tick_routed).
        Box::pin(self.tick_routed(&mut target, time_ms)).await?;

        // Check for FuncComplete matching our FuncInvoke.
        let func_result = target.events().iter().rev().find_map(|e| {
            if let EventType::FuncComplete { invoke_num, result } = &e.event_type
                && *invoke_num == func_event_num
            {
                Some(result.clone())
            } else {
                None
            }
        });

        self.store_instance(target).await?;

        if let Some(mut fields) = func_result {
            fields.entry("error".to_string()).or_insert(OwnedValue::Nil);
            caller.complete_extern(info.invoke_num, fields, time_ms)?;
            return Ok(true);
        }

        Ok(false)
    }

    #[tracing::instrument(level = "debug", skip_all, fields(target_id = %info.entity_id))]
    async fn handle_result(
        &mut self,
        caller: &mut EntityInstance,
        info: &ResultExternInfo,
        time_ms: u64,
    ) -> Result<bool, EngineError> {
        let status = if let Some(inst) = self.cache.borrow().get(&info.entity_id) {
            inst.status()
        } else {
            let events = self.store.read_events(&info.entity_id).await?;
            derive_status(&events)
        };
        match status {
            EntityStatus::Completed { result } => {
                let mut fields = result.unwrap_or_default();
                fields.insert("error".to_string(), OwnedValue::Nil);
                caller.complete_extern(info.invoke_num, fields, time_ms)?;
                Ok(true)
            }
            EntityStatus::Terminated => {
                let mut fields = BTreeMap::new();
                fields.insert(
                    "error".to_string(),
                    OwnedValue::Str("entity was terminated".into()),
                );
                caller.complete_extern(info.invoke_num, fields, time_ms)?;
                Ok(true)
            }
            EntityStatus::Running => Ok(false),
        }
    }

    fn handle_extern_override(
        &self,
        inst: &mut EntityInstance,
        new_events_start: usize,
        time_ms: u64,
    ) -> Result<bool, EngineError> {
        // Find the first overridable extern invoke in the new events.
        let found = inst.events()[new_events_start..].iter().find_map(|event| {
            if let EventType::ExternInvoke { extern_name, args } = &event.event_type {
                let taken = inst.extern_overrides().borrow_mut().remove(extern_name);
                taken.map(|ovr| (event.num, extern_name.clone(), args.clone(), ovr))
            } else {
                None
            }
        });
        if let Some((event_num, extern_name, args, ovr)) = found {
            let result = ovr.execute(args).map_err(EngineError::other)?;
            inst.extern_overrides()
                .borrow_mut()
                .insert(extern_name, ovr);
            inst.complete_extern(event_num, result, time_ms)?;
            return Ok(true);
        }
        Ok(false)
    }
}

impl<S: StateStore> Engine for LocalEngine<S> {
    async fn spawn_entity(
        &mut self,
        id: String,
        entity_type: String,
        args: BTreeMap<String, OwnedValue>,
        implicits: Vec<ImplicitInit>,
        time_ms: u64,
        no_tick: bool,
    ) -> Result<EngineTickResult, EngineError> {
        if self.store.entity_exists(&id).await? {
            return Err(EngineError::EntityAlreadyExists { id });
        }

        let mut inst = EntityInstance::spawn(EntitySpawnOptions {
            registry: self.registry.clone(),
            id: id.clone(),
            entity_type,
            args,
            initial_entry_implicits: implicits,
            native_registry: self.native_registry.clone(),
            extern_overrides: None,
            time_ms,
            disable_heap_collect: self.options.disable_heap_collect,
        })?;

        let (status, spawned) = if no_tick {
            (TickStatus::Blocked, Vec::new())
        } else {
            self.tick_routed(&mut inst, time_ms).await?
        };

        let new_events = if inst.events().len() > 1 {
            inst.events()[1..].to_vec()
        } else {
            Vec::new()
        };
        self.store
            .create_entity(&id, inst.events().to_vec())
            .await?;
        self.cache.borrow_mut().insert(inst);

        Ok(EngineTickResult {
            status,
            new_events,
            spawned,
        })
    }

    async fn tick_entity(
        &mut self,
        id: &str,
        time_ms: u64,
    ) -> Result<EngineTickResult, EngineError> {
        let mut inst = self.load_instance(id).await?;

        let initial_count = inst.events().len();
        let (status, spawned) = if inst.is_done() {
            (TickStatus::Blocked, Vec::new())
        } else {
            self.tick_routed(&mut inst, time_ms).await?
        };

        let new_events = inst.events()[initial_count..].to_vec();
        self.store_instance(inst).await?;

        Ok(EngineTickResult {
            status,
            new_events,
            spawned,
        })
    }

    async fn tick_all(&mut self, time_ms: u64) -> Result<Vec<EngineTickResult>, EngineError> {
        let mut results = Vec::new();

        loop {
            let ids = self
                .store
                .list_entities(&ListEntitiesFilter { running_only: true })
                .await?;
            if ids.is_empty() {
                break;
            }

            let mut made_progress = false;
            for id in &ids {
                let result = self.tick_entity(id, time_ms).await?;
                if !result.new_events.is_empty() || !result.spawned.is_empty() {
                    made_progress = true;
                }
                results.push(result);
            }

            if !made_progress {
                break;
            }
        }

        Ok(results)
    }

    async fn complete_extern(
        &mut self,
        id: &str,
        invoke_num: u64,
        result: BTreeMap<String, OwnedValue>,
        time_ms: u64,
    ) -> Result<(), EngineError> {
        let mut inst = self.load_instance(id).await?;
        inst.complete_extern(invoke_num, result, time_ms)?;
        self.store_instance(inst).await
    }

    async fn invoke_view_func(
        &self,
        id: &str,
        func: &str,
        args: BTreeMap<String, OwnedValue>,
        implicits: Vec<ImplicitInit>,
        after_event_num: Option<u64>,
    ) -> Result<ViewResult<BTreeMap<String, OwnedValue>>, EngineError> {
        let inst = self.load_instance_for_view(id, after_event_num).await?;
        let last_event_num = inst.last_event_num();
        let value = inst.invoke_view_func(func, args, implicits)?;
        if after_event_num.is_none() && self.should_cache(&inst) {
            self.cache.borrow_mut().insert(inst);
        }
        Ok(ViewResult {
            last_event_num,
            value,
        })
    }

    async fn invoke_func(
        &mut self,
        id: &str,
        func: &str,
        request_id: Option<String>,
        args: BTreeMap<String, OwnedValue>,
        time_ms: u64,
    ) -> Result<u64, EngineError> {
        let mut inst = self.load_instance(id).await?;
        let num = inst.invoke_func(func.to_string(), request_id, args, time_ms)?;
        self.store_instance(inst).await?;
        Ok(num)
    }

    async fn cancel_entity(
        &mut self,
        id: &str,
        reason: Option<String>,
        time_ms: u64,
    ) -> Result<(), EngineError> {
        let mut inst = self.load_instance(id).await?;
        inst.cancel(reason, time_ms)?;
        self.store_instance(inst).await
    }

    async fn cancel_func(
        &mut self,
        id: &str,
        request_id: &str,
        time_ms: u64,
    ) -> Result<(), EngineError> {
        let mut inst = self.load_instance(id).await?;
        inst.cancel_func(request_id, time_ms)?;
        self.store_instance(inst).await
    }

    async fn terminate_entity(&mut self, id: &str, time_ms: u64) -> Result<(), EngineError> {
        let mut inst = self.load_instance(id).await?;
        inst.terminate(time_ms)?;
        self.store_instance(inst).await
    }

    async fn entity_events(&self, id: &str) -> Result<Vec<Event>, EngineError> {
        Ok(self.store.read_events(id).await?)
    }

    async fn entity_ids(&self) -> Result<Vec<String>, EngineError> {
        Ok(self.store.list_entities(&Default::default()).await?)
    }

    async fn entity_exists(&self, id: &str) -> Result<bool, EngineError> {
        Ok(self.store.entity_exists(id).await?)
    }

    async fn entity_status(&self, id: &str) -> Result<EntityStatus, EngineError> {
        let events = self.store.read_events(id).await?;
        Ok(derive_status(&events))
    }

    // Intentionally bypasses cache - replay always reconstructs from store events.
    async fn replay_entity(&self, id: &str, time_ms: u64) -> Result<TickStatus, EngineError> {
        let events = self.store.read_events(id).await?;
        let mut inst = EntityInstance::from_events(EntityFromEventsOptions {
            registry: self.registry.clone(),
            events,
            native_registry: self.native_registry.clone(),
            until_event_num: None,
            disable_heap_collect: self.options.disable_heap_collect,
        })?;

        if inst.is_done() {
            return Ok(TickStatus::Completed);
        }

        inst.set_replay_only();
        let result = inst
            .tick(time_ms)
            .map_err(|e| e.into_engine_error(inst.events()))?;
        Ok(result.status)
    }

    async fn view_out_fields(
        &self,
        id: &str,
        after_event_num: Option<u64>,
    ) -> Result<ViewResult<BTreeMap<String, OwnedValue>>, EngineError> {
        let inst = self.load_instance_for_view(id, after_event_num).await?;
        let result = ViewResult {
            last_event_num: inst.last_event_num(),
            value: inst.to_out_fields(),
        };
        if after_event_num.is_none() && self.should_cache(&inst) {
            self.cache.borrow_mut().insert(inst);
        }
        Ok(result)
    }

    async fn view_out_field(
        &self,
        id: &str,
        field: &str,
        after_event_num: Option<u64>,
    ) -> Result<ViewResult<Option<OwnedValue>>, EngineError> {
        let inst = self.load_instance_for_view(id, after_event_num).await?;
        let result = ViewResult {
            last_event_num: inst.last_event_num(),
            value: inst.to_out_field(field),
        };
        if after_event_num.is_none() && self.should_cache(&inst) {
            self.cache.borrow_mut().insert(inst);
        }
        Ok(result)
    }

    async fn inspect_stacks(
        &self,
        id: &str,
        after_event_num: Option<u64>,
    ) -> Result<ViewResult<Vec<CoroutineInfo>>, EngineError> {
        let inst = self.load_instance_for_view(id, after_event_num).await?;
        let result = ViewResult {
            last_event_num: inst.last_event_num(),
            value: self.registry.resolve_stacks(inst.coroutine_stacks()),
        };
        if after_event_num.is_none() && self.should_cache(&inst) {
            self.cache.borrow_mut().insert(inst);
        }
        Ok(result)
    }

    async fn finalize(&self) -> Result<(), EngineError> {
        Ok(self.store.finalize().await?)
    }
}

fn derive_status(events: &[Event]) -> EntityStatus {
    for event in events.iter().rev() {
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
