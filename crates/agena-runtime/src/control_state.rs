//! Runtime-owned coordination state for snapshot replacement and workers.

use std::sync::Arc;

use crate::{ReloadGate, RuntimeBackgroundTaskRegistry, SnapshotStore, TaskControl};

/// Concrete-runtime-independent holder for snapshot and worker coordination.
pub struct RuntimeControlState<S, E> {
    snapshot_store: SnapshotStore<S>,
    reload_gate: ReloadGate,
    background_tasks: RuntimeBackgroundTaskRegistry<E>,
    task_control: Arc<TaskControl>,
    tracing_reload_handle: Option<crate::TracingFilterReloadHandle>,
}

impl<S, E> RuntimeControlState<S, E>
where
    E: Send + Sync + 'static,
{
    pub fn new(
        initial_snapshot: Arc<S>,
        tracing_reload_handle: Option<crate::TracingFilterReloadHandle>,
    ) -> Self {
        Self {
            snapshot_store: SnapshotStore::new(initial_snapshot),
            reload_gate: ReloadGate::default(),
            background_tasks: RuntimeBackgroundTaskRegistry::default(),
            task_control: Arc::new(TaskControl::default()),
            tracing_reload_handle,
        }
    }

    pub fn current_snapshot(&self) -> Arc<S> {
        self.snapshot_store.current()
    }

    pub fn swap_snapshot(&self, next: Arc<S>) -> Arc<S> {
        self.snapshot_store.swap(next)
    }

    pub fn reload_gate(&self) -> &ReloadGate {
        &self.reload_gate
    }

    pub fn background_tasks(&self) -> &RuntimeBackgroundTaskRegistry<E> {
        &self.background_tasks
    }

    pub fn task_control(&self) -> &TaskControl {
        &self.task_control
    }

    pub fn task_control_handle(&self) -> Arc<TaskControl> {
        Arc::clone(&self.task_control)
    }

    /// Reload the active tracing filter when a reload handle was configured.
    /// Returns `false` when no handle exists or the subscriber rejected it.
    pub fn reload_tracing_filter(&self, filter: tracing_subscriber::EnvFilter) -> bool {
        self.tracing_reload_handle
            .as_ref()
            .is_some_and(|handle| handle.reload(filter).is_ok())
    }

    /// Cancel registered background operations and stop maintenance loops in
    /// lifecycle order.
    pub fn shutdown(&self) {
        self.background_tasks.cancel_all();
        self.task_control.shutdown();
    }
}

impl<S, E> Drop for RuntimeControlState<S, E> {
    fn drop(&mut self) {
        self.background_tasks.cancel_all();
        self.task_control.shutdown();
    }
}
