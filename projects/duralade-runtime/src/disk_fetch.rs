use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use duralade_language::load::{
    FetchError, FetchedFile, FetchedModule, ModuleFetcher, path_module_segments, path_root,
};

/// A ModuleFetcher that reads .dl files from disk. Maps canonical project
/// roots to source directories.
///
/// Module path resolution: all segments except the last are directories.
/// The last segment matches files named `<segment>.dl` or `<segment>.<any>.dl`
/// (allowing a module to be split across multiple files for organization).
pub struct DiskFetcher {
    roots: HashMap<String, PathBuf>,
    /// Roots where `builtin` keyword is allowed (stdlib only).
    builtin_roots: HashSet<String>,
}

impl Default for DiskFetcher {
    fn default() -> Self {
        Self::new()
    }
}

impl DiskFetcher {
    pub fn new() -> Self {
        Self {
            roots: HashMap::new(),
            builtin_roots: HashSet::new(),
        }
    }

    pub fn add_root(&mut self, name: impl Into<String>, path: impl Into<PathBuf>) {
        self.roots.insert(name.into(), path.into());
    }

    pub fn set_allow_builtin(&mut self, name: &str) {
        self.builtin_roots.insert(name.to_string());
    }
}

impl ModuleFetcher for DiskFetcher {
    async fn fetch(&self, path: &str) -> Result<FetchedModule, FetchError> {
        let root = path_root(path);
        let segments = path_module_segments(path);

        let base = self.roots.get(root).ok_or_else(|| FetchError {
            message: format!("unknown project root: '{}'", root),
        })?;

        if segments.is_empty() {
            return Err(FetchError {
                message: "empty module path".to_string(),
            });
        }

        // Navigate directories for all segments except the last.
        let mut dir = base.clone();
        for segment in &segments[..segments.len() - 1] {
            dir.push(segment);
        }

        // Last segment: find matching .dl files.
        let module_name = segments[segments.len() - 1];
        let allow_builtin = self.builtin_roots.contains(root);
        let entries = std::fs::read_dir(&dir).map_err(|e| FetchError {
            message: format!("cannot read directory {}: {}", dir.display(), e),
        })?;

        let mut files = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|e| FetchError {
                message: format!("directory entry error: {e}"),
            })?;
            let file_path = entry.path();
            if file_path.extension().is_some_and(|ext| ext == "dl")
                && let Some(stem) = file_path.file_stem().and_then(|s| s.to_str())
            {
                // Match "error.dl" (stem == "error") or "error.types.dl" (stem starts with "error.")
                if stem == module_name || stem.starts_with(&format!("{module_name}.")) {
                    let content = std::fs::read_to_string(&file_path).map_err(|e| FetchError {
                        message: format!("cannot read {}: {}", file_path.display(), e),
                    })?;
                    files.push(FetchedFile {
                        path: file_path,
                        content,
                    });
                }
            }
        }

        if files.is_empty() {
            return Err(FetchError {
                message: format!(
                    "module not found: '{}' (looked for {}/{}.dl in {})",
                    segments.join("."),
                    dir.display(),
                    module_name,
                    base.display(),
                ),
            });
        }

        Ok(FetchedModule {
            files,
            allow_builtin,
        })
    }
}
