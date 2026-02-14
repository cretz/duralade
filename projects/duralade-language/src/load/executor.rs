use std::future::Future;

/// Executor for running CPU-bound work concurrently. Implementors adapt
/// their runtime (tokio, rayon, threads, etc.) to this interface.
///
/// I/O concurrency (fetching files) uses async directly and doesn't need
/// the executor - just `join_all` on the futures.
pub trait Executor: Send + Sync + 'static {
    /// Run a batch of blocking closures concurrently, returning results
    /// in the same order as the input.
    fn run_all_blocking<T: Send + 'static>(
        &self,
        fns: Vec<Box<dyn FnOnce() -> T + Send>>,
    ) -> impl Future<Output = Vec<T>> + Send;

    /// Spawn a blocking closure in the background. The executor tracks it
    /// internally for later joining.
    fn spawn_background(&self, f: impl FnOnce() + Send + 'static);

    /// Wait for all background tasks to complete.
    fn join_all_background(&self) -> impl Future<Output = ()> + Send;
}
