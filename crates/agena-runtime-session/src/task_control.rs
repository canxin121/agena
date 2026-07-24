use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

use crate::{AbortOnDrop, spawn_abortable};

/// Shutdown signal shared by session background tasks.
#[derive(Default)]
pub struct TaskControl {
    shutdown: AtomicBool,
    notify: Notify,
    guards: parking_lot::Mutex<Vec<Arc<AbortOnDrop>>>,
}

impl TaskControl {
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
        self.guards.lock().clear();
    }

    pub fn notify(&self) -> &Notify {
        &self.notify
    }

    pub fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        self.guards.lock().push(Arc::new(spawn_abortable(future)));
    }
}
