//! Admission control for synchronous I/O and CPU work called by async services.

use tokio::sync::Semaphore;
use tokio::task::{AbortHandle, JoinError};

mod write_queue;
pub use write_queue::{WritePermit, WriteQueue, WriteQueueError};

/// A process-wide limit for a class of blocking operations. Admission waits
/// asynchronously, before creating a blocking task. Keep separate pools for
/// long waits, filesystem operations, credentials, and CPU-heavy work to
/// bound each class independently. They still share Tokio's blocking executor.
///
/// Cancellation aborts work still queued in Tokio's blocking pool. Once a
/// closure starts, Rust cannot interrupt it: it retains its permit until it
/// actually finishes, even if the caller disappears. Operations that need
/// prompt cancellation must also check their own cancellation token.
pub struct BlockingPool {
    permits: Semaphore,
}

impl BlockingPool {
    pub const fn new(limit: usize) -> Self {
        assert!(limit > 0, "a blocking pool needs at least one slot");
        Self {
            permits: Semaphore::const_new(limit),
        }
    }

    pub async fn run<T, F>(&'static self, operation: F) -> Result<T, JoinError>
    where
        T: Send + 'static,
        F: FnOnce() -> T + Send + 'static,
    {
        let permit = self
            .permits
            .acquire()
            .await
            .expect("the private blocking admission semaphore is never closed");
        let worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operation()
        });
        let _abort = AbortQueuedWork(worker.abort_handle());
        worker.await
    }
}

struct AbortQueuedWork(AbortHandle);

impl Drop for AbortQueuedWork {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Tie a spawned async child to its owner, including early returns and
/// cancellation. Keep the JoinHandle separately when orderly joining is needed.
pub struct AbortOnDrop(AbortHandle);

impl AbortOnDrop {
    pub fn new<T>(task: &tokio::task::JoinHandle<T>) -> Self {
        Self(task.abort_handle())
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}
