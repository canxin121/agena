use std::future::Future;

/// Aborts an asynchronous task when the guard is dropped.
pub struct AbortOnDrop(pub tokio::task::JoinHandle<()>);

/// Spawn a task with a runtime-owned abort-on-drop lifecycle guard.
pub fn spawn_abortable<F>(future: F) -> AbortOnDrop
where
    F: Future<Output = ()> + Send + 'static,
{
    AbortOnDrop(tokio::spawn(future))
}

/// Spawn a best-effort one-shot task whose result is intentionally detached.
///
/// This is reserved for short notifications that must not keep a runtime
/// service alive (for example, shutdown broadcasts). Long-lived work must use
/// [`spawn_abortable`] or a runtime task-control registry instead.
pub fn spawn_detached<F>(future: F)
where
    F: Future<Output = ()> + Send + 'static,
{
    tokio::spawn(future);
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
