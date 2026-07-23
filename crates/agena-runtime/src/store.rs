use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use arc_swap::ArcSwap;
use tokio::sync::Notify;

/// Lock-free holder for the current runtime snapshot.
pub struct SnapshotStore<T> {
    current: ArcSwap<T>,
}

impl<T> SnapshotStore<T> {
    pub fn new(initial: Arc<T>) -> Self {
        Self {
            current: ArcSwap::from(initial),
        }
    }

    pub fn current(&self) -> Arc<T> {
        self.current.load_full()
    }

    pub fn swap(&self, next: Arc<T>) -> Arc<T> {
        self.current.swap(next)
    }
}

/// Shutdown signal shared by runtime background tasks.
#[derive(Default)]
pub struct TaskControl {
    shutdown: AtomicBool,
    notify: Notify,
    guards: parking_lot::Mutex<Vec<Arc<crate::AbortOnDrop>>>,
}

impl TaskControl {
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        // Dropping the guards aborts runtime-owned workers immediately even
        // when another component still holds an Arc<TaskControl>.
        self.guards.lock().clear();
    }

    pub fn notify(&self) -> &Notify {
        &self.notify
    }

    /// Retain a snapshot/runtime task guard for the lifetime of this control.
    pub(crate) fn retain_guard(&self, guard: Arc<crate::AbortOnDrop>) {
        self.guards.lock().push(guard);
    }

    /// Spawn and retain a runtime worker until shutdown or control drop.
    pub fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.retain_guard(Arc::new(crate::spawn_abortable(future)));
    }
}
