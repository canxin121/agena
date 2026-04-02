use std::sync::{
    Arc, RwLock,
    atomic::{AtomicBool, Ordering},
};

use tokio::sync::Notify;

use super::RuntimeSnapshot;

pub(crate) struct RuntimeSnapshotStore {
    current: RwLock<Arc<RuntimeSnapshot>>,
}

impl RuntimeSnapshotStore {
    pub(crate) fn new(initial: Arc<RuntimeSnapshot>) -> Self {
        Self {
            current: RwLock::new(initial),
        }
    }

    pub(crate) fn current(&self) -> Arc<RuntimeSnapshot> {
        self.current
            .read()
            .map(|guard| Arc::clone(&*guard))
            .unwrap_or_else(|_| {
                panic!("runtime snapshot store lock poisoned while reading current snapshot")
            })
    }

    pub(crate) fn swap(&self, next: Arc<RuntimeSnapshot>) -> Arc<RuntimeSnapshot> {
        let mut guard = self.current.write().unwrap_or_else(|_| {
            panic!("runtime snapshot store lock poisoned while swapping snapshot")
        });
        std::mem::replace(&mut *guard, next)
    }
}

#[derive(Default)]
pub(crate) struct TaskControl {
    shutdown: AtomicBool,
    notify: Notify,
}

impl TaskControl {
    pub(crate) fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    pub(crate) fn shutdown(&self) {
        self.shutdown.store(true, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    pub(crate) fn notify(&self) -> &Notify {
        &self.notify
    }
}
