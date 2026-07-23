use tokio::sync::{Mutex, MutexGuard};

/// Serializes runtime snapshot reload operations.
#[derive(Default)]
pub struct ReloadGate {
    lock: Mutex<()>,
}

impl ReloadGate {
    pub async fn acquire(&self) -> MutexGuard<'_, ()> {
        self.lock.lock().await
    }
}
