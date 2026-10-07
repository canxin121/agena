//! Post-commit notifications and an interruptible scheduler wait.

use std::sync::Arc;

use crate::ScheduledJob;

/// An authoritative local mutation, published only after persistence succeeds.
#[derive(Debug, Clone)]
pub enum SchedulerChange {
    Upsert(ScheduledJob),
    Removed(ScheduledJob),
}

/// Called synchronously after commit. Observers must be cheap and must not
/// perform I/O, await, or call back into scheduler mutations.
pub type SchedulerChangeObserver = Arc<dyn Fn(&SchedulerChange) + Send + Sync>;

pub(super) struct SchedulerChanges {
    // Serialize just commits + notification, never sink work or reads. An
    // older commit must not publish after a newer edit/deletion of that job.
    pub(super) commit: tokio::sync::Mutex<()>,
    observer: parking_lot::RwLock<Option<SchedulerChangeObserver>>,
    pub(super) wake: tokio::sync::watch::Sender<u64>,
}

impl Default for SchedulerChanges {
    fn default() -> Self {
        Self {
            commit: tokio::sync::Mutex::new(()),
            observer: parking_lot::RwLock::new(None),
            wake: tokio::sync::watch::channel(0).0,
        }
    }
}

impl SchedulerChanges {
    pub(super) fn set_observer(&self, observer: SchedulerChangeObserver) {
        *self.observer.write() = Some(observer);
    }

    pub(super) fn publish(&self, change: SchedulerChange, wake: bool) {
        let observer = self.observer.read().clone();
        if let Some(observer) = observer
            && std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| observer(&change))).is_err()
        {
            tracing::error!(target: "agena_scheduler", "scheduler change observer panicked after commit");
        }
        if wake {
            self.wake
                .send_modify(|revision| *revision = revision.wrapping_add(1));
        }
    }
}
