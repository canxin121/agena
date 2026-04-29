use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::RwLock;
use uuid::Uuid;

use crate::job::ScheduledJob;

#[async_trait::async_trait]
pub trait JobStore: Send + Sync {
    async fn put(&self, job: ScheduledJob);
    async fn remove(&self, id: Uuid) -> bool;
    async fn list(&self) -> Vec<ScheduledJob>;
    async fn get(&self, id: Uuid) -> Option<ScheduledJob>;
    async fn replace(&self, id: Uuid, job: ScheduledJob) -> bool;
}

#[derive(Default, Clone)]
pub struct InMemoryJobStore {
    inner: Arc<RwLock<HashMap<Uuid, ScheduledJob>>>,
}

impl InMemoryJobStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait::async_trait]
impl JobStore for InMemoryJobStore {
    async fn put(&self, job: ScheduledJob) {
        self.inner.write().insert(job.id, job);
    }
    async fn remove(&self, id: Uuid) -> bool {
        self.inner.write().remove(&id).is_some()
    }
    async fn list(&self) -> Vec<ScheduledJob> {
        self.inner.read().values().cloned().collect()
    }
    async fn get(&self, id: Uuid) -> Option<ScheduledJob> {
        self.inner.read().get(&id).cloned()
    }
    async fn replace(&self, id: Uuid, job: ScheduledJob) -> bool {
        let mut g = self.inner.write();
        if !g.contains_key(&id) {
            return false;
        }
        g.insert(id, job);
        true
    }
}
