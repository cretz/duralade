use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use crate::engine::ModuleIndex;
use crate::model::Symbol;

use super::Module;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct DeclId(u32);

impl DeclId {
    pub fn raw(self) -> u32 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TypeId(u32);

impl TypeId {
    pub const ANY: TypeId = TypeId(0);
    pub const ANY_LOCAL: TypeId = TypeId(1);
    pub const ERROR: TypeId = TypeId(2);
}

pub struct DeclEntry {
    pub module: ModuleIndex,
    pub name: Symbol,
    pub qualified: Symbol,
}

pub enum TypeEntry {
    Any,
    AnyLocal,
    Error,
    Named {
        decl: DeclId,
        type_args: Vec<RegistryTypeArg>,
    },
    /// A reference to a member func on a named construct.
    /// The type_args are the receiver's type args (substituted into the member's fields).
    MemberFunc {
        decl: DeclId,
        member: Symbol,
        type_args: Vec<RegistryTypeArg>,
    },
    Nilable(TypeId),
    EntityRef(TypeId),
    Anonymous(Box<super::TypeConstruct>),
    TypeParam(Symbol),
}

pub struct RegistryTypeArg {
    pub name: Symbol,
    pub value: TypeId,
}

pub struct Registry {
    // Module layer
    modules: Vec<Arc<Module>>,
    module_paths: Vec<Symbol>,
    module_by_path: HashMap<Symbol, ModuleIndex>,

    // TypeConstruct layer
    decls: Vec<DeclEntry>,
    decl_index: Vec<HashMap<Symbol, DeclId>>,

    // Type layer (interned)
    types: Vec<TypeEntry>,
    named_cache: HashMap<DeclId, TypeId>,
    nilable_cache: HashMap<TypeId, TypeId>,
    entity_ref_cache: HashMap<TypeId, TypeId>,
    type_param_cache: HashMap<Symbol, TypeId>,
    serializable_cache: Mutex<HashMap<TypeId, bool>>,

    // Well-known IDs
    pub any_type: TypeId,
    pub anylocal_type: TypeId,
    pub error_type: TypeId,
    pub int_decl: DeclId,
    pub int_type: TypeId,
    pub float_decl: DeclId,
    pub float_type: TypeId,
    pub str_decl: DeclId,
    pub str_type: TypeId,
    pub bool_decl: DeclId,
    pub bool_type: TypeId,
    pub array_decl: DeclId,
    pub array_type: TypeId,
    pub map_decl: DeclId,
    pub map_type: TypeId,
}

impl Default for Registry {
    fn default() -> Self {
        Self::new()
    }
}

impl Registry {
    /// Create a new registry with Any, Error types and primitive decl/type IDs
    /// pre-interned. Primitive modules are registered as placeholder entries so
    /// their DeclIds are stable from the start. The loader replaces placeholder
    /// modules with real data when loading stdlib.
    pub fn new() -> Self {
        let mut types = Vec::new();
        types.push(TypeEntry::Any);
        let any_type = TypeId::ANY;
        types.push(TypeEntry::AnyLocal);
        let anylocal_type = TypeId::ANY_LOCAL;
        types.push(TypeEntry::Error);
        let error_type = TypeId::ERROR;

        let mut reg = Registry {
            modules: Vec::new(),
            module_paths: Vec::new(),
            module_by_path: HashMap::new(),
            decls: Vec::new(),
            decl_index: Vec::new(),
            types,
            named_cache: HashMap::new(),
            nilable_cache: HashMap::new(),
            entity_ref_cache: HashMap::new(),
            type_param_cache: HashMap::new(),
            serializable_cache: Mutex::new(HashMap::new()),
            any_type,
            anylocal_type,
            error_type,
            // These get set immediately below
            int_decl: DeclId(u32::MAX),
            int_type: TypeId(u32::MAX),
            float_decl: DeclId(u32::MAX),
            float_type: TypeId(u32::MAX),
            str_decl: DeclId(u32::MAX),
            str_type: TypeId(u32::MAX),
            bool_decl: DeclId(u32::MAX),
            bool_type: TypeId(u32::MAX),
            array_decl: DeclId(u32::MAX),
            array_type: TypeId(u32::MAX),
            map_decl: DeclId(u32::MAX),
            map_type: TypeId(u32::MAX),
        };

        // Pre-register placeholder modules and declarations for primitives
        // so DeclIds are stable before any modules are loaded.
        let placeholder_module = Arc::new(super::Module {
            has_errors: false,
            files: Vec::new(),
            decl_index: HashMap::new(),
        });

        let int_idx = reg.register_module(Symbol::mod_int(), placeholder_module.clone());
        reg.int_decl = reg.register_decl(int_idx, &Symbol::int());
        reg.int_type = reg.intern_named(reg.int_decl);

        let float_idx = reg.register_module(Symbol::mod_float(), placeholder_module.clone());
        reg.float_decl = reg.register_decl(float_idx, &Symbol::float());
        reg.float_type = reg.intern_named(reg.float_decl);

        let str_idx = reg.register_module(Symbol::mod_str(), placeholder_module.clone());
        reg.str_decl = reg.register_decl(str_idx, &Symbol::str());
        reg.str_type = reg.intern_named(reg.str_decl);

        let bool_idx = reg.register_module(Symbol::mod_bool(), placeholder_module.clone());
        reg.bool_decl = reg.register_decl(bool_idx, &Symbol::bool());
        reg.bool_type = reg.intern_named(reg.bool_decl);

        let array_idx = reg.register_module(Symbol::mod_array(), placeholder_module.clone());
        reg.array_decl = reg.register_decl(array_idx, &Symbol::array());
        reg.array_type = reg.intern_named(reg.array_decl);

        let map_idx = reg.register_module(Symbol::mod_map(), placeholder_module);
        reg.map_decl = reg.register_decl(map_idx, &Symbol::map());
        reg.map_type = reg.intern_named(reg.map_decl);

        reg
    }

    // --- Module operations ---

    pub fn register_module(&mut self, path: Symbol, module: Arc<Module>) -> ModuleIndex {
        if let Some(&idx) = self.module_by_path.get(&path) {
            // Update existing module
            self.modules[idx.0 as usize] = module;
            return idx;
        }
        let idx = ModuleIndex(self.modules.len() as u32);
        self.modules.push(module);
        self.module_paths.push(path.clone());
        self.module_by_path.insert(path, idx);
        idx
    }

    pub fn module(&self, idx: ModuleIndex) -> &Arc<Module> {
        &self.modules[idx.0 as usize]
    }

    pub fn module_path(&self, idx: ModuleIndex) -> &Symbol {
        &self.module_paths[idx.0 as usize]
    }

    pub fn module_index(&self, path: &str) -> Option<ModuleIndex> {
        self.module_by_path.get(path).copied()
    }

    pub fn module_index_from_segments(&self, segments: &[&str]) -> Option<ModuleIndex> {
        let path: Symbol = segments.join(".").into();
        self.module_index(&path)
    }

    pub fn module_count(&self) -> usize {
        self.modules.len()
    }

    pub fn iter_modules(&self) -> impl Iterator<Item = (ModuleIndex, &Symbol, &Arc<Module>)> {
        self.modules
            .iter()
            .enumerate()
            .map(move |(i, m)| (ModuleIndex(i as u32), &self.module_paths[i], m))
    }

    // --- TypeConstruct operations ---

    pub fn register_decl(&mut self, module: ModuleIndex, name: &Symbol) -> DeclId {
        let mi = module.0 as usize;
        if mi >= self.decl_index.len() {
            self.decl_index.resize_with(mi + 1, HashMap::new);
        }
        if let Some(&id) = self.decl_index[mi].get(name) {
            return id;
        }
        let module_path = &self.module_paths[module.0 as usize];
        let qualified: Symbol = format!("{}::{}", module_path, name).into();
        let id = DeclId(self.decls.len() as u32);
        self.decl_index[mi].insert(name.clone(), id);
        self.decls.push(DeclEntry {
            module,
            name: name.clone(),
            qualified,
        });
        // Every declaration gets a named type interned automatically.
        self.intern_named(id);
        id
    }

    pub fn decl(&self, id: DeclId) -> &DeclEntry {
        &self.decls[id.0 as usize]
    }

    /// Look up the intypes for a declaration via the module it belongs to.
    /// Returns an empty slice if the module is a placeholder stub.
    pub fn decl_intypes(&self, id: DeclId) -> &[super::TypeIntype] {
        let entry = self.decl(id);
        let module = &self.modules[entry.module.0 as usize];
        match module.get_declaration(&entry.name) {
            Some(tc) => &tc.intypes,
            None => &[],
        }
    }

    pub fn decl_id_in_module(&self, module: ModuleIndex, name: &str) -> Option<DeclId> {
        self.decl_index.get(module.0 as usize)?.get(name).copied()
    }

    // --- Type operations ---

    pub fn intern_any(&self) -> TypeId {
        self.any_type
    }

    pub fn intern_error(&self) -> TypeId {
        self.error_type
    }

    /// Look up the TypeId for a named type that was previously interned.
    /// Returns None if the type hasn't been interned yet.
    pub fn named_type(&self, decl: DeclId) -> Option<TypeId> {
        self.named_cache.get(&decl).copied()
    }

    pub fn intern_named(&mut self, decl: DeclId) -> TypeId {
        if let Some(&id) = self.named_cache.get(&decl) {
            return id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::Named {
            decl,
            type_args: Vec::new(),
        });
        self.named_cache.insert(decl, id);
        id
    }

    pub fn intern_named_with_args(
        &mut self,
        decl: DeclId,
        type_args: Vec<RegistryTypeArg>,
    ) -> TypeId {
        if type_args.is_empty() {
            return self.intern_named(decl);
        }
        // Not deduped for now (rare)
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::Named { decl, type_args });
        id
    }

    pub fn intern_nilable(&mut self, inner: TypeId) -> TypeId {
        if let Some(&id) = self.nilable_cache.get(&inner) {
            return id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::Nilable(inner));
        self.nilable_cache.insert(inner, id);
        id
    }

    pub fn intern_entity_ref(&mut self, inner: TypeId) -> TypeId {
        if let Some(&id) = self.entity_ref_cache.get(&inner) {
            return id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::EntityRef(inner));
        self.entity_ref_cache.insert(inner, id);
        id
    }

    pub fn intern_member_func(
        &mut self,
        decl: DeclId,
        member: Symbol,
        type_args: Vec<RegistryTypeArg>,
    ) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::MemberFunc {
            decl,
            member,
            type_args,
        });
        id
    }

    pub fn intern_anonymous(&mut self, construct: super::TypeConstruct) -> TypeId {
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::Anonymous(Box::new(construct)));
        id
    }

    pub fn intern_type_param(&mut self, name: &Symbol) -> TypeId {
        if let Some(&id) = self.type_param_cache.get(name) {
            return id;
        }
        let id = TypeId(self.types.len() as u32);
        self.types.push(TypeEntry::TypeParam(name.clone()));
        self.type_param_cache.insert(name.clone(), id);
        id
    }

    pub fn type_entry(&self, id: TypeId) -> &TypeEntry {
        &self.types[id.0 as usize]
    }

    // --- Primitive helpers ---

    pub fn is_int(&self, ty: TypeId) -> bool {
        ty == self.int_type
    }

    pub fn is_float(&self, ty: TypeId) -> bool {
        ty == self.float_type
    }

    pub fn is_str(&self, ty: TypeId) -> bool {
        ty == self.str_type
    }

    pub fn is_bool(&self, ty: TypeId) -> bool {
        ty == self.bool_type
    }

    pub fn is_numeric(&self, ty: TypeId) -> bool {
        self.is_int(ty) || self.is_float(ty)
    }

    pub fn is_nilable(&self, ty: TypeId) -> bool {
        matches!(self.type_entry(ty), TypeEntry::Nilable(_))
    }

    pub fn nilable_inner(&self, ty: TypeId) -> Option<TypeId> {
        match self.type_entry(ty) {
            TypeEntry::Nilable(inner) => Some(*inner),
            _ => None,
        }
    }

    pub fn is_any(&self, ty: TypeId) -> bool {
        ty == self.any_type
    }

    pub fn is_anylocal(&self, ty: TypeId) -> bool {
        ty == self.anylocal_type
    }

    pub fn is_error(&self, ty: TypeId) -> bool {
        ty == self.error_type
    }

    /// Check whether a type is serializable (can cross extern/`->` boundaries).
    /// Serializable types: primitives, entity refs, `any`, `nil`, and data/anonymous
    /// Pre-populate the serializable cache for a declaration's named TypeId.
    /// Called after field type resolution so is_type_serializable can find
    /// results for same-module types during body type-checking.
    pub fn cache_serializable(&self, decl: DeclId, serializable: bool) {
        if let Some(type_id) = self.named_type(decl) {
            self.serializable_cache
                .lock()
                .unwrap()
                .insert(type_id, serializable);
        }
    }

    /// types whose fields are transitively serializable.
    /// Non-serializable: func, entity, anylocal, anonymous func types.
    pub fn is_type_serializable(&self, ty: TypeId) -> bool {
        if let Some(&cached) = self.serializable_cache.lock().unwrap().get(&ty) {
            return cached;
        }
        let result = match self.type_entry(ty) {
            TypeEntry::Any
            | TypeEntry::Error
            | TypeEntry::EntityRef(_)
            | TypeEntry::TypeParam(_) => true,
            TypeEntry::AnyLocal => false,
            TypeEntry::Nilable(inner) => self.is_type_serializable(*inner),
            TypeEntry::Anonymous(tc) => {
                if tc.kind == super::TypeConstructKind::Func {
                    false
                } else {
                    tc.fields.iter().all(|f| self.is_type_serializable(f.ty))
                }
            }
            TypeEntry::Named { decl, type_args } => {
                let entry = self.decl(*decl);
                let module = &self.modules[entry.module.0 as usize];
                let Some(tc) = module.get_declaration(&entry.name) else {
                    return true;
                };
                match tc.kind {
                    super::TypeConstructKind::Func
                    | super::TypeConstructKind::Extern
                    | super::TypeConstructKind::Native
                    | super::TypeConstructKind::Entity => false,
                    super::TypeConstructKind::Data | super::TypeConstructKind::TypeAlias => {
                        type_args.iter().all(|a| self.is_type_serializable(a.value))
                            && tc.fields.iter().all(|f| self.is_type_serializable(f.ty))
                    }
                }
            }
            TypeEntry::MemberFunc { .. } => false,
        };
        self.serializable_cache.lock().unwrap().insert(ty, result);
        result
    }

    pub fn named_decl(&self, ty: TypeId) -> Option<DeclId> {
        match self.type_entry(ty) {
            TypeEntry::Named { decl, .. } => Some(*decl),
            _ => None,
        }
    }

    /// Substitute type parameters in a type. E.g., if `ty` is TypeParam("t") and
    /// `type_args` contains `[("t", int)]`, returns `int`. Recurses into nilable
    /// and named type args.
    pub fn substitute_type_params(&mut self, ty: TypeId, type_args: &[(Symbol, TypeId)]) -> TypeId {
        match self.type_entry(ty) {
            TypeEntry::TypeParam(name) => {
                let name = name.clone();
                for (param_name, concrete_ty) in type_args {
                    if *param_name == name {
                        return *concrete_ty;
                    }
                }
                ty
            }
            TypeEntry::Nilable(inner) => {
                let inner = *inner;
                let substituted = self.substitute_type_params(inner, type_args);
                if substituted == inner {
                    ty
                } else {
                    self.intern_nilable(substituted)
                }
            }
            TypeEntry::Named {
                decl,
                type_args: named_args,
            } if !named_args.is_empty() => {
                let decl = *decl;
                let args: Vec<_> = named_args
                    .iter()
                    .map(|a| (a.name.clone(), a.value))
                    .collect();
                let mut changed = false;
                let new_args: Vec<_> = args
                    .into_iter()
                    .map(|(name, arg_ty)| {
                        let sub = self.substitute_type_params(arg_ty, type_args);
                        if sub != arg_ty {
                            changed = true;
                        }
                        RegistryTypeArg { name, value: sub }
                    })
                    .collect();
                if changed {
                    self.intern_named_with_args(decl, new_args)
                } else {
                    ty
                }
            }
            _ => ty,
        }
    }

    /// Qualify a declaration name with its module path: "duralade.str::find"
    pub fn qualify_decl(&self, module: ModuleIndex, name: &str) -> String {
        format!("{}::{}", self.module_path(module), name)
    }

    // --- Display ---

    pub fn display_type(&self, ty: TypeId) -> String {
        match self.type_entry(ty) {
            TypeEntry::Any => "any".to_string(),
            TypeEntry::AnyLocal => "anylocal".to_string(),
            TypeEntry::Error => "<error>".to_string(),
            TypeEntry::Named { decl, type_args } => {
                let entry = self.decl(*decl);
                let mut s = entry.name.to_string();
                if !type_args.is_empty() {
                    let args: Vec<String> = type_args
                        .iter()
                        .map(|a| format!("{} = {}", a.name, self.display_type(a.value)))
                        .collect();
                    s.push_str(&format!("[{}]", args.join(", ")));
                }
                s
            }
            TypeEntry::Nilable(inner) => format!("{}?", self.display_type(*inner)),
            TypeEntry::EntityRef(inner) => format!("&{}", self.display_type(*inner)),
            TypeEntry::MemberFunc {
                decl,
                member,
                type_args,
            } => {
                let entry = self.decl(*decl);
                let mut s = format!("{}.{}", entry.name, member);
                if !type_args.is_empty() {
                    let args: Vec<String> = type_args
                        .iter()
                        .map(|a| format!("{} = {}", a.name, self.display_type(a.value)))
                        .collect();
                    s.push_str(&format!("[{}]", args.join(", ")));
                }
                s
            }
            TypeEntry::Anonymous(_) => "{...}".to_string(),
            TypeEntry::TypeParam(name) => name.to_string(),
        }
    }

    pub fn display_type_qualified(&self, ty: TypeId) -> String {
        match self.type_entry(ty) {
            TypeEntry::Any => "any".to_string(),
            TypeEntry::AnyLocal => "anylocal".to_string(),
            TypeEntry::Error => "<error>".to_string(),
            TypeEntry::Named { decl, type_args } => {
                let entry = self.decl(*decl);
                let mut s = entry.qualified.to_string();
                if !type_args.is_empty() {
                    let args: Vec<String> = type_args
                        .iter()
                        .map(|a| format!("{} = {}", a.name, self.display_type_qualified(a.value)))
                        .collect();
                    s.push_str(&format!("[{}]", args.join(", ")));
                }
                s
            }
            TypeEntry::Nilable(inner) => {
                format!("{}?", self.display_type_qualified(*inner))
            }
            TypeEntry::EntityRef(inner) => {
                format!("&{}", self.display_type_qualified(*inner))
            }
            TypeEntry::Anonymous(tc) => {
                let kind = match tc.kind {
                    super::TypeConstructKind::Data => "data",
                    super::TypeConstructKind::Entity => "entity",
                    super::TypeConstructKind::Func if tc.func_modifiers.view => "view func",
                    super::TypeConstructKind::Func if tc.func_modifiers.noblock => "noblock func",
                    super::TypeConstructKind::Func => "func",
                    _ => "unknown",
                };
                let fields: Vec<String> = tc
                    .fields
                    .iter()
                    .map(|f| {
                        let modifier = f
                            .modifier
                            .map(|m| format!("{} ", m.as_str()))
                            .unwrap_or_default();
                        let name = &f.name;
                        let ty = self.display_type_qualified(f.ty);
                        format!("{modifier}{name}: {ty}")
                    })
                    .collect();
                format!("{kind} {{ {} }}", fields.join(", "))
            }
            TypeEntry::MemberFunc {
                decl,
                member,
                type_args,
            } => {
                let entry = self.decl(*decl);
                let mut s = format!(
                    "{}::{}.{}",
                    self.module_path(entry.module),
                    entry.name,
                    member
                );
                if !type_args.is_empty() {
                    let args: Vec<String> = type_args
                        .iter()
                        .map(|a| format!("{} = {}", a.name, self.display_type_qualified(a.value)))
                        .collect();
                    s.push_str(&format!("[{}]", args.join(", ")));
                }
                s
            }
            TypeEntry::TypeParam(name) => format!("[{name}]"),
        }
    }

    // --- Resolve helpers (for runtime) ---

    /// Resolve a frame to human-readable form. Delegates to module data.
    pub fn resolve_frame(
        &self,
        frame: &crate::interpret::scope::StackFrame,
    ) -> super::super::engine::ResolvedFrame {
        let module_path = self.module_path(frame.module).to_string();
        let module = self.module(frame.module);
        let file = &module.files[frame.file_idx as usize];
        let source_map = &file.source_file.source_map;

        let location_node_id = frame.call_node_id.unwrap_or(frame.decl_node_id);
        let line = source_map
            .node_ranges
            .get(&location_node_id)
            .and_then(|(start, _)| source_map.byte_to_line(*start));

        let file_path = Some(file.source_file.file_path.clone());

        for decl in &file.declarations {
            if decl.node_id == frame.decl_node_id {
                return super::super::engine::ResolvedFrame {
                    module: module_path,
                    construct: decl
                        .name
                        .as_ref()
                        .map(|n| n.to_string())
                        .unwrap_or_default(),
                    member: None,
                    file: file_path,
                    line,
                };
            }
            for member in &decl.members {
                if member.node_id == frame.decl_node_id {
                    return super::super::engine::ResolvedFrame {
                        module: module_path,
                        construct: decl
                            .name
                            .as_ref()
                            .map(|n| n.to_string())
                            .unwrap_or_default(),
                        member: member.name.as_ref().map(|n| n.to_string()),
                        file: file_path,
                        line,
                    };
                }
            }
        }

        super::super::engine::ResolvedFrame {
            module: module_path,
            construct: format!("<node {}>", frame.decl_node_id.0),
            member: None,
            file: file_path,
            line,
        }
    }

    pub fn resolve_stacks(
        &self,
        stacks: Vec<(u64, Option<u64>, Vec<crate::interpret::scope::StackFrame>)>,
    ) -> Vec<super::super::engine::CoroutineInfo> {
        stacks
            .into_iter()
            .map(
                |(id, source_event, stack)| super::super::engine::CoroutineInfo {
                    id,
                    source_event,
                    stack: stack.iter().map(|f| self.resolve_frame(f)).collect(),
                },
            )
            .collect()
    }
}

impl std::fmt::Debug for Registry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Registry")
            .field("modules", &self.modules.len())
            .field("decls", &self.decls.len())
            .field("types", &self.types.len())
            .finish()
    }
}
