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
    cancelled: tokio_util::sync::CancellationToken,
    guards: parking_lot::Mutex<Vec<Arc<crate::AbortOnDrop>>>,
    workers: tokio_util::task::TaskTracker,
}

impl TaskControl {
    pub fn is_shutdown(&self) -> bool {
        self.shutdown.load(Ordering::SeqCst)
    }

    /// Close control and return whether this caller performed the first
    /// transition. Notification ownership shares the publication mutex.
    pub fn shutdown(&self) -> bool {
        let (first_shutdown, guards) = {
            let mut guards = self.guards.lock();
            let first_shutdown = !self.shutdown.swap(true, Ordering::SeqCst);
            (first_shutdown, std::mem::take(&mut *guards))
        };
        self.workers.close();
        self.cancelled.cancel();
        self.notify.notify_waiters();
        // Dropping the guards aborts runtime-owned workers immediately even
        // when another component still holds an Arc<TaskControl>.
        drop(guards);
        first_shutdown
    }

    /// Wait for cancelled worker futures to actually release their resources.
    pub(crate) async fn shutdown_and_wait(&self) {
        self.shutdown();
        self.workers.wait().await;
    }

    /// Observe shutdown without losing a notification before the first poll.
    pub(crate) async fn cancelled(&self) {
        self.cancelled.cancelled().await;
    }

    /// Serialize a synchronous publication with shutdown and maintenance
    /// admission. The caller must not await or reenter this control while held.
    pub(crate) fn running_guard(&self) -> Option<impl Drop + '_> {
        let guard = self.guards.lock();
        if self.is_shutdown() {
            return None;
        }
        Some(guard)
    }

    pub fn notify(&self) -> &Notify {
        &self.notify
    }

    /// Spawn and retain a runtime worker until shutdown or control drop.
    pub fn spawn<F>(&self, future: F)
    where
        F: std::future::Future<Output = ()> + Send + 'static,
    {
        let mut guards = self.guards.lock();
        if self.is_shutdown() {
            // The rejected future may own resources with reentrant Drop code.
            drop(guards);
            return;
        }
        // Register under the same lock as shutdown, including before the
        // future's first poll. Aborting the worker releases its tracker token.
        guards.push(Arc::new(crate::spawn_abortable(
            self.workers.track_future(future),
        )));
    }
}

#[cfg(test)]
mod drain_tests {
    use super::*;

    #[tokio::test]
    async fn drain_joins_cancelled_worker_destructors_before_returning() {
        struct MarkDropped(Arc<AtomicBool>);
        impl Drop for MarkDropped {
            fn drop(&mut self) {
                self.0.store(true, Ordering::SeqCst);
            }
        }
        let control = TaskControl::default();
        let dropped = Arc::new(AtomicBool::new(false));
        let marker = MarkDropped(dropped.clone());
        let (started, running) = tokio::sync::oneshot::channel();
        control.spawn(async move {
            let _marker = marker;
            let _ = started.send(());
            std::future::pending::<()>().await;
        });
        running.await.unwrap();
        control.shutdown_and_wait().await;
        assert!(dropped.load(Ordering::SeqCst));
        control.shutdown_and_wait().await;
    }
}
