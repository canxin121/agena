use std::future::Future;

pub use agena_async::AbortOnDrop;

/// Spawn a task with a runtime-owned abort-on-drop lifecycle guard.
pub fn spawn_abortable<F>(future: F) -> AbortOnDrop
where
    F: Future<Output = ()> + Send + 'static,
{
    let task = tokio::spawn(future);
    AbortOnDrop::new(&task)
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
