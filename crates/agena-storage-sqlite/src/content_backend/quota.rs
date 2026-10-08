//! Retained-resource reservations are shared by concurrent commits. Transient
//! record/manifest staging is bounded by admitted producer batches. A cancelled
//! or failed attempt reconciles unpublished files before the next retry.

use super::*;

#[derive(Default)]
struct Usage {
    bytes: u64,
    dirty: bool,
}
#[derive(Default)]
struct Accounting {
    total: u64,
    resources: HashMap<ContentId, Usage>,
}

pub(super) struct StoreQuota {
    state: Mutex<Accounting>,
    limit: u64,
}
pub(super) struct Lease {
    quota: Arc<StoreQuota>,
    id: ContentId,
    previous: u64,
    reserved: u64,
    finished: bool,
}

async fn directory_bytes(path: &Path) -> Result<u64, StoreError> {
    let mut entries = match tokio::fs::read_dir(path).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(io_error(error)),
    };
    let mut bytes = 0u64;
    while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
        if entry.file_type().await.map_err(io_error)?.is_file() {
            bytes = bytes
                .checked_add(entry.metadata().await.map_err(io_error)?.len())
                .ok_or_else(|| {
                    StoreError::Constraint("content store byte count exhausted".into())
                })?;
        }
    }
    Ok(bytes)
}

pub(super) async fn resource_bytes(root: &Path, id: ContentId) -> Result<u64, StoreError> {
    Ok(directory_bytes(&root.join(id.to_string())).await?
        + directory_bytes(&root.join(format!(".import-{id}"))).await?)
}

impl StoreQuota {
    pub async fn scan(root: &Path, limit: u64) -> Result<Arc<Self>, StoreError> {
        let mut entries = match tokio::fs::read_dir(root).await {
            Ok(entries) => Some(entries),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(io_error(error)),
        };
        let mut state = Accounting::default();
        if let Some(entries) = &mut entries {
            while let Some(entry) = entries.next_entry().await.map_err(io_error)? {
                if !entry.file_type().await.map_err(io_error)?.is_dir() {
                    continue;
                }
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                let value = name.strip_prefix(".import-").unwrap_or(name);
                let Ok(id) =
                    serde_json::from_value::<ContentId>(serde_json::Value::String(value.into()))
                else {
                    continue;
                };
                let bytes = directory_bytes(&entry.path()).await?;
                state.total = state.total.checked_add(bytes).ok_or_else(|| {
                    StoreError::Constraint("content store byte count exhausted".into())
                })?;
                let usage = state.resources.entry(id).or_default();
                usage.bytes += bytes;
                // A previous process may have left an unpublished tail.
                usage.dirty = true;
            }
        }
        Ok(Arc::new(Self {
            state: Mutex::new(state),
            limit,
        }))
    }

    pub fn needs_reconciliation(&self, id: ContentId) -> bool {
        self.state
            .lock()
            .expect("content quota lock")
            .resources
            .get(&id)
            .is_some_and(|usage| usage.dirty)
    }

    pub fn mark_dirty(&self, id: ContentId) {
        self.state
            .lock()
            .expect("content quota lock")
            .resources
            .entry(id)
            .or_default()
            .dirty = true;
    }

    pub async fn reconcile(&self, id: ContentId, root: &Path) -> Result<(), StoreError> {
        let bytes = resource_bytes(root, id).await?;
        let mut state = self.state.lock().expect("content quota lock");
        let previous = state.resources.get(&id).map_or(0, |usage| usage.bytes);
        state.total = state.total - previous + bytes;
        state.resources.insert(
            id,
            Usage {
                bytes,
                dirty: false,
            },
        );
        Ok(())
    }

    /// Reserve the absolute retained size against actual accounting. A virtual
    /// initial manifest has no bytes on disk and cannot subsidize creation.
    pub async fn reserve(
        self: &Arc<Self>,
        id: ContentId,
        target: u64,
        root: &Path,
    ) -> Result<Lease, StoreError> {
        self.reserve_target(id, target, root, false).await
    }

    /// A bounded terminal descriptor must remain publishable when retained
    /// output has filled the store. This allowance never admits record bytes
    /// or creates a resource; subsequent growth still sees its actual charge.
    pub async fn reserve_seal(
        self: &Arc<Self>,
        id: ContentId,
        target: u64,
        root: &Path,
    ) -> Result<Lease, StoreError> {
        self.reserve_target(id, target, root, true).await
    }

    async fn reserve_target(
        self: &Arc<Self>,
        id: ContentId,
        target: u64,
        root: &Path,
        sealing: bool,
    ) -> Result<Lease, StoreError> {
        if self.needs_reconciliation(id) {
            self.reconcile(id, root).await?;
        }
        let mut state = self.state.lock().expect("content quota lock");
        let previous = state.resources.get(&id).map_or(0, |usage| usage.bytes);
        let extra = target.saturating_sub(previous);
        let total = state
            .total
            .checked_add(extra)
            .filter(|total| {
                extra == 0 || *total <= self.limit || (sealing && previous > 0 && extra <= 4096)
            })
            .ok_or_else(|| {
                StoreError::Constraint("content store exceeds its retained-byte budget".into())
            })?;
        state.total = total;
        Ok(Lease {
            quota: self.clone(),
            id,
            previous,
            reserved: extra,
            finished: false,
        })
    }

    pub fn deleted(&self, id: ContentId) {
        let mut state = self.state.lock().expect("content quota lock");
        if let Some(usage) = state.resources.remove(&id) {
            state.total -= usage.bytes;
        }
    }
}

impl Lease {
    pub fn finish(mut self, bytes: u64) {
        let mut state = self.quota.state.lock().expect("content quota lock");
        state.total = state.total - self.reserved - self.previous + bytes;
        // Failed retirement can temporarily retain more physical bytes than
        // planned. Charge them and reconcile before the next admitted growth.
        state.resources.insert(
            self.id,
            Usage {
                bytes,
                dirty: bytes > self.previous + self.reserved,
            },
        );
        self.finished = true;
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        if !self.finished {
            self.quota
                .state
                .lock()
                .expect("content quota lock")
                .resources
                .insert(
                    self.id,
                    Usage {
                        bytes: self.previous + self.reserved,
                        dirty: true,
                    },
                );
        }
    }
}
