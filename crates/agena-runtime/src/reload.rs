use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// Why the current runtime snapshot is being rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeReloadCause {
    Manual,
    WatchedPathsChanged { paths: Vec<PathBuf> },
}

/// Metadata for a completed reload, or a reload deferred until its plugin call exits.
#[derive(Debug, Clone)]
pub struct RuntimeReloadReport {
    pub cause: RuntimeReloadCause,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    /// When set, generation/loaded_at describe the current snapshot. The task
    /// owns the pending reload and exposes its eventual success or failure.
    pub deferred_task_id: Option<String>,
}

tokio::task_local! {
    static DEFERRED_RELOADS: std::sync::Arc<std::sync::Mutex<Vec<String>>>;
}

/// Keep pending lifecycle effects visible at the common API response boundary.
/// The scope retains the caller's other task-local callback authority.
pub async fn capture_deferred_reloads<F: std::future::Future>(
    future: F,
) -> (F::Output, Vec<String>) {
    let tasks = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let output = DEFERRED_RELOADS.scope(tasks.clone(), future).await;
    let ids = tasks
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .clone();
    (output, ids)
}

pub(crate) fn record_deferred_reload(task_id: &str) {
    let _ = DEFERRED_RELOADS.try_with(|tasks| {
        let mut tasks = tasks.lock().unwrap_or_else(|error| error.into_inner());
        if !tasks.iter().any(|id| id == task_id) {
            tasks.push(task_id.to_owned());
        }
    });
}
