use std::collections::HashSet;
use std::path::Path;

use serde::Deserialize;

use duralade_language::load::LoadResult;

use crate::disk_fetch::DiskFetcher;

/// Root name for implicit projects (directories without duralade.toml).
pub const IMPLICIT_ROOT: &str = "_";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct TomlRoot {
    project: ProjectConfig,
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectConfig {
    pub name: String,
    pub source_root: Option<String>,
    pub tests_root: Option<String>,
    #[serde(default)]
    pub allow_builtin: bool,
    #[serde(default = "default_true")]
    pub stdlib_prelude: bool,
}

impl ProjectConfig {
    pub fn parse_toml(toml_str: &str) -> Result<Self, String> {
        let root: TomlRoot =
            toml::from_str(toml_str).map_err(|e| format!("failed to parse duralade.toml: {e}"))?;
        Ok(root.project)
    }

    pub fn effective_source_root(&self) -> String {
        self.source_root
            .clone()
            .unwrap_or_else(|| "src".to_string())
    }

    pub fn effective_test_root(&self) -> String {
        self.tests_root
            .clone()
            .unwrap_or_else(|| "test".to_string())
    }
}

pub struct ProjectLoadResult {
    pub config: ProjectConfig,
    pub load_result: LoadResult,
    pub test_modules: Vec<String>,
}

/// Load a project from its directory, reading duralade.toml and discovering all modules.
/// TODO: stdlib_dir will be auto-discovered (bundled with CLI or env var) instead of passed explicitly.
pub fn load_project(
    project_dir: &Path,
    stdlib_dir: &Path,
    include_tests: bool,
) -> Result<ProjectLoadResult, String> {
    let toml_path = project_dir.join("duralade.toml");
    let config = if toml_path.is_file() {
        let toml_str = std::fs::read_to_string(&toml_path)
            .map_err(|e| format!("cannot read {}: {e}", toml_path.display()))?;
        ProjectConfig::parse_toml(&toml_str)?
    } else {
        ProjectConfig {
            name: IMPLICIT_ROOT.to_string(),
            source_root: Some(".".to_string()),
            tests_root: None,
            allow_builtin: false,
            stdlib_prelude: true,
        }
    };

    let source_dir = project_dir.join(config.effective_source_root());
    let test_dir = project_dir.join(config.effective_test_root());

    let mut fetcher = DiskFetcher::new();
    if source_dir.is_dir() {
        fetcher.add_root(&config.name, &source_dir);
    }
    if include_tests && test_dir.is_dir() {
        fetcher.add_root("test", &test_dir);
    }
    fetcher.add_root("duralade", stdlib_dir);

    // TODO: stdlib should be loaded as a proper dependency, not special-cased.
    // For now, derive its config from the parent directory's duralade.toml.
    if let Some(stdlib_project_dir) = stdlib_dir.parent() {
        let stdlib_toml = stdlib_project_dir.join("duralade.toml");
        if let Ok(toml_str) = std::fs::read_to_string(stdlib_toml)
            && let Ok(stdlib_config) = ProjectConfig::parse_toml(&toml_str)
            && stdlib_config.allow_builtin
        {
            fetcher.set_allow_builtin("duralade");
        }
    }

    let source_modules = if source_dir.is_dir() {
        discover_modules(&source_dir, &config.name)?
    } else {
        Vec::new()
    };
    let test_modules = if include_tests && test_dir.is_dir() {
        discover_modules(&test_dir, "test")?
    } else {
        Vec::new()
    };

    let all_modules: Vec<String> = source_modules
        .iter()
        .cloned()
        .chain(test_modules.iter().cloned())
        .collect();

    #[cfg(feature = "tokio")]
    {
        use crate::tokio_executor::TokioExecutor;
        use duralade_language::load::{self, LoadContext, RootSettings};

        let executor = TokioExecutor::new();
        let mut root_settings = std::collections::HashMap::new();
        let prelude = RootSettings {
            stdlib_prelude: config.stdlib_prelude,
        };
        root_settings.insert(config.name.clone(), prelude.clone());
        root_settings.insert("test".to_string(), prelude);
        let mut ctx = LoadContext::new(root_settings);

        let rt = tokio::runtime::Runtime::new().map_err(|e| format!("tokio error: {e}"))?;
        rt.block_on(async {
            for module_path in &all_modules {
                load::load_module(&mut ctx, &executor, &fetcher, module_path.clone().into()).await;
            }
        });

        let load_result = ctx
            .into_result()
            .map_err(|_| "LoadContext still has clones".to_string())?;

        Ok(ProjectLoadResult {
            config,
            load_result,
            test_modules,
        })
    }

    #[cfg(not(feature = "tokio"))]
    {
        let _ = all_modules;
        Err("load_project requires the 'tokio' feature".to_string())
    }
}

/// Walk a directory, find .dl files, return unique module paths.
fn discover_modules(dir: &Path, root: &str) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    let mut modules = Vec::new();
    walk_dir(dir, dir, root, &mut seen, &mut modules)?;
    Ok(modules)
}

fn walk_dir(
    base: &Path,
    dir: &Path,
    root: &str,
    seen: &mut HashSet<Vec<String>>,
    modules: &mut Vec<String>,
) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| format!("cannot read directory {}: {e}", dir.display()))?;

    for entry in entries {
        let entry = entry.map_err(|e| format!("directory entry error: {e}"))?;
        let path = entry.path();

        if path.is_dir() {
            walk_dir(base, &path, root, seen, modules)?;
        } else if path.extension().is_some_and(|ext| ext == "dl")
            && let Some(module_segments) = file_to_module_segments(base, &path)
            && seen.insert(module_segments.clone())
        {
            let module_path = std::iter::once(root)
                .chain(module_segments.iter().map(|s| s.as_str()))
                .collect::<Vec<_>>()
                .join(".");
            modules.push(module_path);
        }
    }

    Ok(())
}

/// Convert a .dl file path to module path segments relative to base.
/// `base/admin/roles.dl` → `["admin", "roles"]`
/// `base/user.types.dl` → `["user"]` (multi-file module, first dot-segment only)
fn file_to_module_segments(base: &Path, file: &Path) -> Option<Vec<String>> {
    let relative = file.strip_prefix(base).ok()?;
    let mut segments: Vec<String> = relative
        .parent()?
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect();

    let stem = file.file_stem()?.to_str()?;
    // For multi-file modules (user.types.dl), take only the first segment before '.'
    let module_name = stem.split('.').next()?;
    segments.push(module_name.to_string());

    Some(segments)
}
