use std::sync::Mutex;

use duralade_language::load::Executor;

/// A tokio-based executor that runs blocking work on the tokio blocking
/// thread pool and tracks background tasks via JoinHandles.
pub struct TokioExecutor {
    handles: Mutex<Vec<tokio::task::JoinHandle<()>>>,
}

impl Default for TokioExecutor {
    fn default() -> Self {
        Self::new()
    }
}

impl TokioExecutor {
    pub fn new() -> Self {
        Self {
            handles: Mutex::new(Vec::new()),
        }
    }
}

impl Executor for TokioExecutor {
    async fn run_all_blocking<T: Send + 'static>(
        &self,
        fns: Vec<Box<dyn FnOnce() -> T + Send>>,
    ) -> Vec<T> {
        let handles: Vec<_> = fns
            .into_iter()
            .map(|f| tokio::task::spawn_blocking(f))
            .collect();
        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(handle.await.expect("blocking task panicked"));
        }
        results
    }

    fn spawn_background(&self, f: impl FnOnce() + Send + 'static) {
        let handle = tokio::task::spawn_blocking(f);
        self.handles.lock().unwrap().push(handle);
    }

    async fn join_all_background(&self) {
        let handles: Vec<_> = {
            let mut lock = self.handles.lock().unwrap();
            std::mem::take(&mut *lock)
        };
        for handle in handles {
            handle.await.expect("background task panicked");
        }
    }
}
