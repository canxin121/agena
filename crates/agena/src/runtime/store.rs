use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use arc_swap::ArcSwap;
use tokio::sync::Notify;

use super::RuntimeSnapshot;

pub(crate) struct RuntimeSnapshotStore {
    current: ArcSwap<RuntimeSnapshot>,
}

impl RuntimeSnapshotStore {
    pub(crate) fn new(initial: Arc<RuntimeSnapshot>) -> Self {
        Self {
            current: ArcSwap::from(initial),
        }
    }

    pub(crate) fn current(&self) -> Arc<RuntimeSnapshot> {
        self.current.load_full()
    }

    pub(crate) fn swap(&self, next: Arc<RuntimeSnapshot>) -> Arc<RuntimeSnapshot> {
        self.current.swap(next)
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
