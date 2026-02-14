use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::model::{self, ConstructKind, SourceFile, Symbol};
use crate::parse::parse_source_file;

use super::{
    Executor, FuncModifiers, LoadContext, LoadError, Module, ModuleFetcher, TypeConstruct,
    TypeConstructKind, TypeTable, TypedFile, Visibility, context, type_resolve,
};

/// Load a module by its dot-separated path (e.g. "duralade.str").
/// If the module is already loaded or in-progress, returns the existing entry.
pub async fn load_module(
    ctx: &mut LoadContext,
    executor: &impl Executor,
    fetcher: &impl ModuleFetcher,
    path: Symbol,
) -> Arc<Module> {
    if let Some(module) = ctx.get_module(&path) {
        return module;
    }

    if ctx.is_on_stack(&path) {
        // Circular import - return a placeholder with has_errors so downstream
        // can distinguish "not found" from "circular".
        let module = Arc::new(Module {
            has_errors: true,
            files: Vec::new(),
            decl_index: HashMap::new(),
        });
        ctx.insert_module(path, module.clone());
        return module;
    }

    ctx.push_stack(path.clone());

    // Fetch source files
    let fetched = match fetcher.fetch(&path).await {
        Ok(f) => f,
        Err(e) => {
            ctx.add_load_error(LoadError {
                module: path.clone(),
                message: e.message,
                file_idx: 0,
                range: None,
            });
            ctx.pop_stack();
            let module = Arc::new(Module {
                has_errors: true,
                files: Vec::new(),
                decl_index: HashMap::new(),
            });
            ctx.insert_module(path, module.clone());
            return module;
        }
    };
    let allow_builtin = fetched.allow_builtin;

    // Parse each file concurrently
    let parse_fns: Vec<Box<dyn FnOnce() -> crate::parse::ParseResult + Send>> = fetched
        .files
        .into_iter()
        .map(|file| {
            Box::new(move || parse_source_file(file.content, file.path, allow_builtin)) as _
        })
        .collect();
    let parse_results = executor.run_all_blocking(parse_fns).await;

    let mut source_files = Vec::new();
    let mut has_errors = false;
    for (file_idx, result) in parse_results.into_iter().enumerate() {
        if !result.parse_errors.is_empty() {
            has_errors = true;
            ctx.add_parse_errors(&path, file_idx, result.parse_errors);
        }
        if !result.strict_mode_violations.is_empty() {
            ctx.add_strict_violations(&path, file_idx, result.strict_mode_violations);
        }
        source_files.push(result.source_file);
    }

    // Build TypedFiles with stub declarations, checking for duplicate names.
    // This must happen BEFORE loading dependencies so that circular imports
    // can see this module's declarations via the registry.
    let implicit_modules = ctx.implicit_modules_for(&path);
    let (files, decl_index) = collect_declarations(source_files, ctx, &path, implicit_modules);

    let mut module = Module {
        has_errors,
        files,
        decl_index,
    };

    // Pre-register the module and its declarations in the registry.
    {
        let stub = Arc::new(Module {
            has_errors: module.has_errors,
            files: Vec::new(),
            decl_index: module.decl_index.clone(),
        });
        ctx.insert_module(path.clone(), stub);
        if let Some(module_idx) = ctx.with_registry(|r| r.module_index(&path)) {
            for (decl_name, &(fi, di)) in &module.decl_index {
                let decl_id = ctx.register_decl(module_idx, decl_name);
                module.files[fi].declarations[di].decl_id = Some(decl_id);
            }
        }
    }

    // Follow imports - recursively load imported modules before type resolution.
    // Declarations are already registered above, so circular imports can resolve
    // references to this module's types.
    let source_files: Vec<_> = module.files.iter().map(|f| f.source_file.clone()).collect();
    let mut import_paths = collect_import_paths(ctx, &path, &source_files);
    // Ensure implicit modules are loaded too.
    for implicit in ctx.implicit_modules_for(&path) {
        if !import_paths.iter().any(|p| p == implicit) {
            import_paths.push(implicit.clone());
        }
    }
    for import_path in import_paths {
        if import_path != path {
            Box::pin(load_module(ctx, executor, fetcher, import_path)).await;
        }
    }

    // Resolve field types on all declarations, then type-check function bodies.
    // Acquires the write lock once for the entire type resolution pipeline,
    // giving direct registry access instead of per-call lock overhead.
    let populate_non_required_types = ctx.populate_non_required_types;
    ctx.with_inner_mut(|registry, load_errors, strict_violations| {
        let mut type_ctx = type_resolve::TypeContext {
            registry,
            errors: load_errors,
            strict_violations,
        };
        type_resolve::resolve_module_types(
            &mut type_ctx,
            &mut module,
            populate_non_required_types,
            &path,
        );
    });

    // Replace the stub with the fully resolved module
    let module = Arc::new(module);
    ctx.insert_module(path, module.clone());

    ctx.pop_stack();
    module
}

/// Collect all unique import paths from parsed source files.
fn collect_import_paths(
    ctx: &LoadContext,
    module_path: &Symbol,
    source_files: &[SourceFile],
) -> Vec<Symbol> {
    let current_root = module_path.split('.').next().unwrap_or(module_path);
    let mut seen = HashSet::new();
    for (file_idx, file) in source_files.iter().enumerate() {
        for import in &file.imports {
            let import_path = context::import_to_path(import);
            let import_root = import_path.split('.').next().unwrap_or(&import_path);
            if current_root != import_root
                && import_path
                    .split('.')
                    .skip(1)
                    .any(|seg| seg.starts_with('_'))
            {
                ctx.add_load_error(LoadError {
                    module: module_path.clone(),
                    message: format!(
                        "cannot import '{}': underscore-prefixed modules are project-private",
                        import_path
                    ),
                    file_idx,
                    range: file.source_map.node_ranges.get(&import.node_id).copied(),
                });
            }
            seen.insert(import_path);
        }
    }
    seen.into_iter().collect()
}

/// Build TypedFiles from parsed source files, collecting stub declarations
/// and detecting duplicate names across files.
fn collect_declarations(
    source_files: Vec<SourceFile>,
    ctx: &LoadContext,
    path: &Symbol,
    implicit_modules: &[Symbol],
) -> (Vec<TypedFile>, HashMap<Symbol, (usize, usize)>) {
    let mut files: Vec<TypedFile> = Vec::new();
    let mut decl_index: HashMap<Symbol, (usize, usize)> = HashMap::new();
    for (file_idx, source_file) in source_files.into_iter().enumerate() {
        let mut declarations = Vec::new();
        for construct in &source_file.constructs {
            let Some(decl) = construct_to_declaration(construct) else {
                continue;
            };

            let Some(name) = decl.name.clone() else {
                continue;
            };
            match decl_index.entry(name) {
                std::collections::hash_map::Entry::Occupied(existing) => {
                    ctx.add_load_error(LoadError {
                        module: path.clone(),
                        message: format!("Duplicate declaration '{}'", existing.key()),
                        file_idx,
                        range: source_file
                            .source_map
                            .node_ranges
                            .get(&decl.node_id)
                            .copied(),
                    });
                }
                std::collections::hash_map::Entry::Vacant(e) => {
                    e.insert((file_idx, declarations.len()));
                    declarations.push(decl);
                }
            }
        }
        // Build import map: implicit modules first, explicit imports shadow.
        let mut import_map = HashMap::new();
        for module_path in implicit_modules {
            let alias = context::path_alias(module_path);
            import_map.insert(alias, module_path.clone());
        }
        for import in &source_file.imports {
            let alias = context::import_alias(import);
            let ipath = context::import_to_path(import);
            import_map.insert(alias, ipath);
        }

        files.push(TypedFile {
            source_file,
            type_table: TypeTable::default(),
            annotations: Vec::new(),
            declarations,
            import_map,
        });
    }
    (files, decl_index)
}

/// Convert an AST Construct into a TypeConstruct stub.
/// Fields and type info are left empty - they'll be filled during type resolution.
pub(super) fn construct_to_declaration(construct: &model::Construct) -> Option<TypeConstruct> {
    let (kind, func_modifiers, members) = match &construct.kind {
        ConstructKind::TypeAlias(_) => (
            TypeConstructKind::TypeAlias,
            FuncModifiers::default(),
            vec![],
        ),
        ConstructKind::Data(data) => {
            let members = data
                .funcs
                .iter()
                .filter_map(construct_to_declaration)
                .collect();
            (TypeConstructKind::Data, FuncModifiers::default(), members)
        }
        ConstructKind::Entity(entity) => {
            let members = entity
                .funcs
                .iter()
                .filter_map(construct_to_declaration)
                .collect();
            (TypeConstructKind::Entity, FuncModifiers::default(), members)
        }
        ConstructKind::Func(func) => {
            let mods = FuncModifiers {
                view: func.view,
                noblock: func.noblock,
            };
            let kind = if construct.builtin {
                TypeConstructKind::Native
            } else {
                TypeConstructKind::Func
            };
            (kind, mods, vec![])
        }
        ConstructKind::Extern(_) => (TypeConstructKind::Extern, FuncModifiers::default(), vec![]),
        ConstructKind::Native(native) => {
            let mods = FuncModifiers {
                view: native.view,
                noblock: native.noblock,
            };
            (TypeConstructKind::Native, mods, vec![])
        }
        ConstructKind::Invalid(_) => return None,
    };
    let visibility = if construct.out {
        Visibility::Public
    } else {
        Visibility::Private
    };
    Some(TypeConstruct {
        name: Some(construct.name.name.clone()),
        kind,
        visibility,
        annotations: vec![],
        intypes: vec![],
        fields: vec![],
        out_early_name: None,
        out_field_names: vec![],
        func_modifiers,
        type_alias_target: None,
        members,
        member_index: HashMap::new(),
        is_builtin: construct.builtin,
        non_serializable: false,
        decl_id: None,
        node_id: construct.node_id,
        ast_field_eval_order: vec![],
    })
    .map(|mut decl| {
        decl.rebuild_metadata();
        decl
    })
}
