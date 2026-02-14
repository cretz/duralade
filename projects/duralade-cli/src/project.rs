use std::path::{Path, PathBuf};
use std::sync::Arc;

use duralade_language::load::Registry;
use duralade_runtime::project::{IMPLICIT_ROOT, load_project};

pub struct LoadedProject {
    pub registry: Arc<Registry>,
    pub is_implicit: bool,
}

pub fn load_code(code_source: &str, strict: bool) -> Result<LoadedProject, String> {
    let project_dir = clean_canonicalize(code_source)
        .map_err(|e| format!("cannot resolve code source '{code_source}': {e}"))?;
    let stdlib_dir = find_stdlib()?;

    let result = load_project(&project_dir, &stdlib_dir, false)?;
    let is_implicit = result.config.name == IMPLICIT_ROOT;

    let registry_ref = &result.load_result.registry;
    let mut msgs = Vec::new();
    for err in &result.load_result.parse_errors {
        msgs.push(format!(
            "  parse: {}",
            err.format_with_registry(registry_ref)
        ));
    }
    for err in &result.load_result.load_errors {
        msgs.push(format!("  {}", err.format_with_registry(registry_ref)));
    }
    if strict {
        for err in &result.load_result.strict_violations {
            msgs.push(format!(
                "  strict: {}",
                err.format_with_registry(registry_ref)
            ));
        }
    }
    if !msgs.is_empty() {
        return Err(format!("project has errors:\n{}", msgs.join("\n")));
    }

    let registry = Arc::new(result.load_result.registry);
    Ok(LoadedProject {
        registry,
        is_implicit,
    })
}

pub fn resolve_project_dir(code_source: &str) -> Result<PathBuf, String> {
    clean_canonicalize(code_source)
        .map_err(|e| format!("cannot resolve code source '{code_source}': {e}"))
}

/// Canonicalize a path, stripping the Windows `\\?\` extended-length prefix.
/// Rust's std::fs::canonicalize on Windows always adds this prefix, which makes
/// paths ugly in error messages. There's no stdlib function that gives a clean,
/// normalized absolute path - `std::path::absolute` doesn't resolve `..` segments,
/// and `canonicalize` adds the prefix. So we canonicalize then strip.
fn clean_canonicalize(path: impl AsRef<Path>) -> std::io::Result<PathBuf> {
    let canon = std::fs::canonicalize(path)?;
    let s = canon.to_string_lossy();
    Ok(PathBuf::from(
        s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    ))
}

pub fn find_stdlib() -> Result<PathBuf, String> {
    let dev_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../duralade-stdlib/src");
    if dev_path.is_dir() {
        return Ok(dev_path);
    }
    Err(
        "cannot find duralade stdlib (looked at ../duralade-stdlib/src relative to CLI crate)"
            .into(),
    )
}
