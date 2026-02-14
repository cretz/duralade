use std::future::Future;
use std::path::PathBuf;

/// A source file ready for parsing.
pub struct FetchedFile {
    /// Path for error reporting (may be virtual for non-disk sources).
    pub path: PathBuf,
    /// Source text.
    pub content: String,
}

pub struct FetchedModule {
    pub files: Vec<FetchedFile>,
    /// Whether `builtin` keyword is allowed (project-level setting).
    pub allow_builtin: bool,
}

/// Fetches the source files that make up a module. Given a dot-separated
/// module path (e.g. "duralade.str"), returns all `.dl` files contributing
/// to that module with their content. The fetcher internally maps canonical
/// roots to source locations. Callers don't care where modules live.
pub trait ModuleFetcher: Send + Sync {
    fn fetch(&self, path: &str) -> impl Future<Output = Result<FetchedModule, FetchError>> + Send;
}

#[derive(Debug)]
pub struct FetchError {
    pub message: String,
}
