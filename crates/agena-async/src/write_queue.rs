//! Fair, bounded admission before a single-writer database checks out a connection.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use tokio::sync::{OwnedSemaphorePermit, Semaphore};

const MAX_ADMITTED_WRITES: usize = 128;
const ADMISSION_TIMEOUT: Duration = Duration::from_secs(15);

/// One queue per database file, shared even by independently opened pools.
/// Waiting writers own no database connection. Reads bypass this queue.
#[derive(Debug)]
pub struct WriteQueue {
    capacity: Arc<Semaphore>,
    writer: Arc<Semaphore>,
}

#[derive(Debug)]
pub enum WriteQueueError {
    Full,
    TimedOut,
}

impl std::fmt::Display for WriteQueueError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Full => f.write_str("database write queue is full"),
            Self::TimedOut => f.write_str("database write admission timed out"),
        }
    }
}

impl std::error::Error for WriteQueueError {}

/// Retain through commit/rollback, then release before notifications or other
/// post-commit work. Dropping a waiting future releases its queue capacity.
pub struct WritePermit {
    _writer: OwnedSemaphorePermit,
    _capacity: OwnedSemaphorePermit,
    // Drop the queue last, after returning both permits, so a concurrent
    // registry lookup cannot replace it while an old writer is still admitted.
    _queue: Arc<WriteQueue>,
}

impl WriteQueue {
    /// Canonicalize once during database composition/first use, outside a
    /// transaction. Callers should retain the resulting Arc for their pool's
    /// lifetime. Symlink and relative-path aliases share the same queue.
    pub async fn for_file(path: &Path) -> Result<Arc<Self>, std::io::Error> {
        let identity = tokio::fs::canonicalize(path).await?;
        Ok(Self::named(identity))
    }

    /// An already resolved identity, also suitable for named in-memory stores.
    pub fn named(identity: PathBuf) -> Arc<Self> {
        static QUEUES: OnceLock<Mutex<HashMap<PathBuf, Weak<WriteQueue>>>> = OnceLock::new();
        let mut queues = QUEUES
            .get_or_init(Mutex::default)
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        queues.retain(|_, queue| queue.strong_count() > 0);
        if let Some(queue) = queues.get(&identity).and_then(Weak::upgrade) {
            return queue;
        }
        let queue = Arc::new(Self {
            capacity: Arc::new(Semaphore::new(MAX_ADMITTED_WRITES)),
            writer: Arc::new(Semaphore::new(1)),
        });
        queues.insert(identity, Arc::downgrade(&queue));
        queue
    }

    pub async fn acquire(self: &Arc<Self>) -> Result<WritePermit, WriteQueueError> {
        let capacity = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| WriteQueueError::Full)?;
        // Tokio's semaphore admits writers in arrival order, avoiding repeated
        // lock races. The timeout also bounds retained request payloads.
        let writer =
            tokio::time::timeout(ADMISSION_TIMEOUT, Arc::clone(&self.writer).acquire_owned())
                .await
                .map_err(|_| WriteQueueError::TimedOut)?
                .expect("the private database write semaphore is never closed");
        Ok(WritePermit {
            _queue: Arc::clone(self),
            _writer: writer,
            _capacity: capacity,
        })
    }
}
