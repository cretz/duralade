use std::path::{Path, PathBuf};

use duralade_language::load::{self, LoadContext};
use duralade_runtime::disk_fetch::DiskFetcher;
use duralade_runtime::project::ProjectConfig;
use duralade_runtime::tokio_executor::TokioExecutor;

use crate::annotations;

#[derive(Debug)]
pub struct TestResult {
    pub passed: bool,
    pub diagnostics: Vec<String>,
}

struct FileInfo {
    module_path: String,
    cleaned_source: String,
    annotations: Vec<annotations::Annotation>,
}

pub fn run_loader_tests(loader_dir: &Path, stdlib_dir: &Path) -> Result<TestResult, String> {
    // Read all .dl files, parse annotations, write cleaned sources to temp dir.
    let temp_dir = std::env::temp_dir().join("duralade_loader_tests");
    let _ = std::fs::remove_dir_all(&temp_dir);
    std::fs::create_dir_all(&temp_dir).map_err(|e| e.to_string())?;

    let mut files: Vec<FileInfo> = Vec::new();
    let mut entries: Vec<_> = std::fs::read_dir(loader_dir)
        .map_err(|e| e.to_string())?
        .flatten()
        .filter(|e| e.path().extension().and_then(|ext| ext.to_str()) == Some("dl"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in &entries {
        let path = entry.path();
        let stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .ok_or_else(|| format!("bad filename: {}", path.display()))?;
        let source = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
        let test_file = annotations::parse_test_file(&source, &["ERROR", "STRICT"])?;
        std::fs::write(
            temp_dir.join(path.file_name().unwrap()),
            &test_file.cleaned_source,
        )
        .map_err(|e| e.to_string())?;
        files.push(FileInfo {
            module_path: format!("loader.{stem}"),
            cleaned_source: test_file.cleaned_source,
            annotations: test_file.annotations,
        });
    }

    // Copy subdirectories into temp dir: `helpers/` for same-project modules,
    // `deps/` for external project roots (each subdir becomes a fetcher root).
    let mut dep_roots: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(loader_dir)
        .into_iter()
        .flatten()
        .flatten()
    {
        let path = entry.path();
        if path.is_dir() {
            let name = path.file_name().unwrap().to_str().unwrap_or("").to_string();
            if name == "deps" {
                for dep in std::fs::read_dir(&path).into_iter().flatten().flatten() {
                    let dep_path = dep.path();
                    let toml_path = dep_path.join("duralade.toml");
                    if let Ok(toml_str) = std::fs::read_to_string(&toml_path)
                        && let Ok(config) = ProjectConfig::parse_toml(&toml_str)
                    {
                        let src = dep_path.join(config.effective_source_root());
                        dep_roots.push((config.name, src));
                    }
                }
            } else {
                let dest = temp_dir.join(&name);
                copy_dir(&path, &dest).map_err(|e| e.to_string())?;
            }
        }
    }

    // Load all modules with a shared context.
    let mut fetcher = DiskFetcher::new();
    fetcher.add_root("loader", &temp_dir);
    fetcher.add_root("duralade", stdlib_dir);
    fetcher.set_allow_builtin("duralade");
    for (name, path) in &dep_roots {
        fetcher.add_root(name, path);
    }

    let executor = TokioExecutor::new();
    let mut root_settings = std::collections::HashMap::new();
    root_settings.insert(
        "loader".to_string(),
        load::RootSettings {
            stdlib_prelude: true,
        },
    );
    let mut ctx = LoadContext::new(root_settings);

    let rt = tokio::runtime::Runtime::new().map_err(|e| e.to_string())?;
    for file in &files {
        rt.block_on(load::load_module(
            &mut ctx,
            &executor,
            &fetcher,
            file.module_path.as_str().into(),
        ));
    }

    let result = match ctx.into_result() {
        Ok(r) => r,
        Err(_) => return Err("LoadContext still has outstanding references".to_string()),
    };

    // Match errors against annotations per-module.
    struct ActualError {
        module: String,
        line: usize,
        col_start: usize,
        col_end: usize,
        tag: &'static str,
        message: String,
    }

    let mut actual_errors: Vec<ActualError> = Vec::new();

    for err in &result.load_errors {
        let source = files
            .iter()
            .find(|f| f.module_path.as_str() == &*err.module)
            .map(|f| f.cleaned_source.as_str());
        if let Some((start, end)) = err.range
            && let Some(source) = source
        {
            actual_errors.push(ActualError {
                module: err.module.to_string(),
                line: annotations::line_num(source, start),
                col_start: annotations::col_num(source, start),
                col_end: annotations::col_num(source, end),
                tag: "ERROR",
                message: err.message.clone(),
            });
        }
    }

    for err in &result.strict_violations {
        let source = files
            .iter()
            .find(|f| f.module_path.as_str() == &*err.module)
            .map(|f| f.cleaned_source.as_str());
        if let Some((start, end)) = err.range
            && let Some(source) = source
        {
            actual_errors.push(ActualError {
                module: err.module.to_string(),
                line: annotations::line_num(source, start),
                col_start: annotations::col_num(source, start),
                col_end: annotations::col_num(source, end),
                tag: "STRICT",
                message: err.message.clone(),
            });
        }
    }

    let mut diagnostics = Vec::new();
    let mut matched = vec![false; actual_errors.len()];

    for file in &files {
        for expected in &file.annotations {
            let found = actual_errors.iter().enumerate().any(|(i, actual)| {
                !matched[i]
                    && actual.module == file.module_path
                    && actual.line == expected.line
                    && actual.col_start == expected.col_start
                    && actual.col_end == expected.col_end
                    && actual.tag == expected.tag
                    && actual.message.contains(&expected.message_substring)
                    && {
                        matched[i] = true;
                        true
                    }
            });

            if !found {
                diagnostics.push(format!(
                    "{} Line {}:{}-{}: Expected {}: \"{}\" - NOT FOUND",
                    file.module_path,
                    expected.line,
                    expected.col_start,
                    expected.col_end,
                    expected.tag,
                    expected.message_substring
                ));
            }
        }
    }

    for (i, actual) in actual_errors.iter().enumerate() {
        if !matched[i] {
            diagnostics.push(format!(
                "{} Line {}:{}-{}: Unexpected {}: \"{}\"",
                actual.module,
                actual.line,
                actual.col_start,
                actual.col_end,
                actual.tag,
                actual.message
            ));
        }
    }

    for err in &result.parse_errors {
        diagnostics.push(format!(
            "{}: Unexpected parse error: \"{}\"",
            err.module, err.message
        ));
    }

    if !diagnostics.is_empty() {
        let mut header = vec!["Actual errors:".to_string()];
        for actual in &actual_errors {
            header.push(format!(
                "  {} Line {}:{}-{}: {}: \"{}\"",
                actual.module,
                actual.line,
                actual.col_start,
                actual.col_end,
                actual.tag,
                actual.message
            ));
        }
        if actual_errors.is_empty() {
            header.push("  (none)".to_string());
        }
        header.push(String::new());
        diagnostics.splice(0..0, header);
    }

    let _ = std::fs::remove_dir_all(&temp_dir);

    Ok(TestResult {
        passed: diagnostics.is_empty(),
        diagnostics,
    })
}

fn copy_dir(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dest)?;
    for entry in std::fs::read_dir(src)? {
        let entry = entry?;
        let path = entry.path();
        let target = dest.join(path.file_name().unwrap());
        if path.is_dir() {
            copy_dir(&path, &target)?;
        } else {
            std::fs::copy(&path, &target)?;
        }
    }
    Ok(())
}
