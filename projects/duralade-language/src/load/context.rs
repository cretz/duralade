use std::collections::HashMap;
use std::sync::{Arc, LazyLock, RwLock};

use crate::model::Symbol;
use crate::parse::ParseError;

use super::Module;

/// Per-project-root settings that affect module loading.
/// Each project root (e.g. "myapp", "duralade", "test") can have its own settings.
#[derive(Clone, Default)]
pub struct RootSettings {
    pub stdlib_prelude: bool,
}

thread_local! {
    /// Tracks nested with_registry read locks to detect deadlocks in debug builds.
    /// A with_registry_mut call while this is > 0 would deadlock the RwLock.
    static REGISTRY_READ_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// An error encountered during loading (distinct from parse errors).
#[derive(Debug, Clone)]
pub struct LoadError {
    pub module: Symbol,
    pub message: String,
    /// Which file in the module. Use 0 for module-level errors.
    pub file_idx: usize,
    /// Byte range `(start, end)` where the error occurred, end-exclusive.
    /// `None` for module-level errors without a specific source location.
    pub range: Option<(usize, usize)>,
}

impl LoadError {
    /// Format this error with file path and line number when available.
    pub fn format_with_registry(&self, registry: &super::registry::Registry) -> String {
        if let (Some((start, _)), Some(module_idx)) =
            (self.range, registry.module_index(&self.module))
        {
            let module = registry.module(module_idx);
            if let Some(file) = module.files.get(self.file_idx) {
                let source_map = &file.source_file.source_map;
                if let Some(line) = source_map.byte_to_line(start) {
                    return format!("{}:{}: {}", file.source_file.file_path, line, self.message);
                }
            }
        }
        format!("{}: {}", self.module, self.message)
    }
}

// --- Module path utilities ---
// A module path is a dot-separated string like "duralade.str".
// The first segment is the project root, the rest are the module path.

/// Build a dot-separated module path Symbol from a parsed `Import` node.
pub fn import_to_path(import: &crate::model::Import) -> Symbol {
    import
        .path
        .iter()
        .map(|i| i.name.as_str())
        .collect::<Vec<_>>()
        .join(".")
        .into()
}

/// Extract the effective alias name from an import: explicit alias if
/// present, otherwise the last path segment.
pub fn import_alias(import: &crate::model::Import) -> Symbol {
    import
        .alias
        .as_ref()
        .map(|a| a.name.clone())
        .or_else(|| import.path.last().map(|s| s.name.clone()))
        .unwrap_or_else(Symbol::empty)
}

/// Get the project root from a dot-separated module path (first segment).
pub fn path_root(path: &str) -> &str {
    path.split('.').next().unwrap_or(path)
}

/// Get the default alias from a dot-separated module path (last segment).
/// E.g. "duralade.error" → "error".
pub fn path_alias(path: &str) -> Symbol {
    path.rsplit('.').next().unwrap_or(path).into()
}

/// Get the module segments after the root from a dot-separated module path.
/// E.g. "duralade.admin.roles" → ["admin", "roles"].
pub fn path_module_segments(path: &str) -> Vec<&str> {
    let mut segments: Vec<&str> = path.split('.').collect();
    if !segments.is_empty() {
        segments.remove(0);
    }
    segments
}

/// The result of a completed load operation - loaded modules and any errors.
pub struct LoadResult {
    pub registry: super::registry::Registry,
    pub parse_errors: Vec<LoadError>,
    pub load_errors: Vec<LoadError>,
    pub strict_violations: Vec<LoadError>,
}

impl LoadResult {
    pub fn has_errors(&self) -> bool {
        !self.parse_errors.is_empty() || !self.load_errors.is_empty()
    }
}

/// Default implicit modules available without explicit import.
/// TODO: allow projects to customize this list via duralade.toml (e.g. stdlib_prelude = false).
pub static DEFAULT_IMPLICIT_MODULES: LazyLock<Vec<Symbol>> = LazyLock::new(|| {
    vec![
        Symbol::mod_array(),
        Symbol::mod_bool(),
        Symbol::mod_error(),
        Symbol::mod_float(),
        Symbol::mod_int(),
        Symbol::mod_map(),
        Symbol::mod_str(),
    ]
});

#[derive(Default)]
struct LoadContextInner {
    modules: HashMap<Symbol, Arc<Module>>,
    registry: super::registry::Registry,
    parse_errors: Vec<LoadError>,
    load_errors: Vec<LoadError>,
    strict_violations: Vec<LoadError>,
}

/// Shared state that flows through the loading pipeline. Cheap to clone
/// (Arc internals). Each clone can carry per-context info like the current
/// loading stack for cycle detection.
#[derive(Clone)]
pub struct LoadContext {
    inner: Arc<RwLock<LoadContextInner>>,
    /// Current module-loading stack for cycle awareness.
    stack: Vec<Symbol>,
    /// When true, populate TypeTable with non-runtime-required expression types.
    /// Used by LSP; off by default to save memory and load time.
    pub populate_non_required_types: bool,
    /// Per-project-root settings. Shared across clones; cheap to clone (Arc).
    root_settings: Arc<HashMap<String, RootSettings>>,
}

impl LoadContext {
    pub fn new(root_settings: HashMap<String, RootSettings>) -> Self {
        Self {
            inner: Arc::default(),
            stack: Vec::new(),
            populate_non_required_types: false,
            root_settings: Arc::new(root_settings),
        }
    }

    /// Return implicit prelude modules for a given module path, based on its root's settings.
    pub fn implicit_modules_for(&self, module_path: &str) -> &[Symbol] {
        let root = path_root(module_path);
        if self
            .root_settings
            .get(root)
            .is_some_and(|s| s.stdlib_prelude)
        {
            &DEFAULT_IMPLICIT_MODULES
        } else {
            &[]
        }
    }

    pub fn is_on_stack(&self, path: &str) -> bool {
        self.stack.iter().any(|s| s.as_str() == path)
    }

    pub fn push_stack(&mut self, path: Symbol) {
        self.stack.push(path);
    }

    pub fn pop_stack(&mut self) {
        self.stack.pop();
    }

    pub fn add_parse_errors(&self, module: &Symbol, file_idx: usize, errors: Vec<ParseError>) {
        let mut inner = self.inner.write().unwrap();
        inner
            .parse_errors
            .extend(errors.into_iter().map(|e| LoadError {
                module: module.clone(),
                message: e.message,
                file_idx,
                range: Some(e.consumed_range),
            }));
    }

    pub fn add_strict_violations(
        &self,
        module: &Symbol,
        file_idx: usize,
        violations: Vec<ParseError>,
    ) {
        let mut inner = self.inner.write().unwrap();
        inner
            .strict_violations
            .extend(violations.into_iter().map(|e| LoadError {
                module: module.clone(),
                message: e.message,
                file_idx,
                range: Some(e.consumed_range),
            }));
    }

    pub fn add_strict_violation(&self, error: LoadError) {
        self.inner.write().unwrap().strict_violations.push(error);
    }

    pub fn add_load_error(&self, error: LoadError) {
        self.inner.write().unwrap().load_errors.push(error);
    }

    /// Report a load error and return error_type in one step.
    /// Prefer this over separate add_load_error + error_type calls.
    pub fn add_type_error(
        &self,
        module: Symbol,
        message: String,
        file_idx: usize,
        range: Option<(usize, usize)>,
    ) -> super::registry::TypeId {
        self.add_load_error(LoadError {
            module,
            message,
            file_idx,
            range,
        });
        self.error_type()
    }

    pub fn get_module(&self, path: &str) -> Option<Arc<Module>> {
        let inner = self.inner.read().unwrap();
        inner.modules.get(path).cloned()
    }

    pub fn insert_module(&self, path: Symbol, module: Arc<Module>) {
        let mut inner = self.inner.write().unwrap();
        inner.registry.register_module(path.clone(), module.clone());
        inner.modules.insert(path, module);
    }

    pub fn register_decl(
        &self,
        module_idx: crate::engine::ModuleIndex,
        name: &Symbol,
    ) -> super::registry::DeclId {
        self.inner
            .write()
            .unwrap()
            .registry
            .register_decl(module_idx, name)
    }

    pub fn with_registry<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&super::registry::Registry) -> R,
    {
        #[cfg(debug_assertions)]
        REGISTRY_READ_DEPTH.with(|c| c.set(c.get() + 1));
        let inner = self.inner.read().unwrap();
        let result = f(&inner.registry);
        #[cfg(debug_assertions)]
        REGISTRY_READ_DEPTH.with(|c| c.set(c.get() - 1));
        result
    }

    pub fn with_registry_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut super::registry::Registry) -> R,
    {
        #[cfg(debug_assertions)]
        assert!(
            REGISTRY_READ_DEPTH.with(|c| c.get()) == 0,
            "with_registry_mut called while with_registry read lock is held - this will deadlock"
        );
        let mut inner = self.inner.write().unwrap();
        f(&mut inner.registry)
    }

    /// Acquire the write lock and pass direct access to the registry, load errors,
    /// and strict violations. Used by body type-checking to avoid per-call lock overhead.
    pub fn with_inner_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut super::registry::Registry, &mut Vec<LoadError>, &mut Vec<LoadError>) -> R,
    {
        let mut inner = self.inner.write().unwrap();
        let inner = &mut *inner;
        f(
            &mut inner.registry,
            &mut inner.load_errors,
            &mut inner.strict_violations,
        )
    }

    pub fn decl_id_in_module(
        &self,
        module_idx: crate::engine::ModuleIndex,
        name: &str,
    ) -> Option<super::registry::DeclId> {
        self.with_registry(|r| r.decl_id_in_module(module_idx, name))
    }

    pub fn module_index(&self, path: &str) -> Option<crate::engine::ModuleIndex> {
        self.with_registry(|r| r.module_index(path))
    }

    pub fn array_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.array_decl)
    }
    pub fn bool_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.bool_decl)
    }
    pub fn float_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.float_decl)
    }
    pub fn int_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.int_decl)
    }
    pub fn map_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.map_decl)
    }
    pub fn str_decl(&self) -> super::registry::DeclId {
        self.with_registry(|r| r.str_decl)
    }

    pub fn any_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.any_type)
    }
    pub fn anylocal_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.anylocal_type)
    }
    pub fn array_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.array_type)
    }
    pub fn bool_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.bool_type)
    }
    pub fn error_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.error_type)
    }
    pub fn float_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.float_type)
    }
    pub fn int_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.int_type)
    }
    pub fn map_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.map_type)
    }
    pub fn str_type(&self) -> super::registry::TypeId {
        self.with_registry(|r| r.str_type)
    }

    pub fn intern_named_type(&self, decl: super::registry::DeclId) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_named(decl))
    }
    pub fn intern_named_type_with_args(
        &self,
        decl: super::registry::DeclId,
        type_args: Vec<super::registry::RegistryTypeArg>,
    ) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_named_with_args(decl, type_args))
    }
    pub fn intern_member_func_type(
        &self,
        decl: super::registry::DeclId,
        member: Symbol,
        type_args: Vec<super::registry::RegistryTypeArg>,
    ) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_member_func(decl, member, type_args))
    }
    pub fn intern_nilable_type(&self, inner: super::registry::TypeId) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_nilable(inner))
    }
    pub fn intern_entity_ref_type(
        &self,
        inner: super::registry::TypeId,
    ) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_entity_ref(inner))
    }
    pub fn intern_anonymous_type(&self, anon: super::TypeConstruct) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_anonymous(anon))
    }
    pub fn intern_type_param_type(&self, name: &Symbol) -> super::registry::TypeId {
        self.with_registry_mut(|r| r.intern_type_param(name))
    }

    /// Consume the LoadContext and return the LoadResult.
    /// Returns Err if other clones of this context still exist.
    pub fn into_result(self) -> Result<LoadResult, Self> {
        let stack = self.stack;
        let populate_non_required_types = self.populate_non_required_types;
        let root_settings = self.root_settings;
        match Arc::try_unwrap(self.inner) {
            Ok(rwlock) => {
                let inner = rwlock.into_inner().unwrap();
                Ok(LoadResult {
                    registry: inner.registry,
                    parse_errors: inner.parse_errors,
                    load_errors: inner.load_errors,
                    strict_violations: inner.strict_violations,
                })
            }
            Err(inner) => Err(LoadContext {
                inner,
                stack,
                populate_non_required_types,
                root_settings,
            }),
        }
    }
}
