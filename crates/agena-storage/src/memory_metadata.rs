//! Storage-specific write guards for domain memory metadata.

pub use agena_domain::{
    MemoryKind, MemoryMetadata, MemoryOrigin, MemoryRevision, MemoryScope, MemorySource,
    MemoryStatus,
};

/// Optional control fields retain their existing values on an update.
/// Source provenance always describes this write and is cleared when omitted.
#[derive(Debug, Clone, Default)]
pub struct MemoryWriteOptions {
    pub kind: Option<MemoryKind>,
    pub scope: Option<MemoryScope>,
    pub status: Option<MemoryStatus>,
    pub importance: Option<u8>,
    pub source: Option<MemorySource>,
    /// Every replaced note must be read, authorized and revision checked.
    pub supersedes: Vec<MemoryRevision>,
}

/// Capture before starting asynchronous extraction, then pass back unchanged.
#[derive(Debug, Clone)]
pub struct MemoryWriteSnapshot {
    pub name: String,
    pub generation: u64,
    pub expected_sha256: Option<String>,
    pub is_tombstoned: bool,
    pub(crate) store_key: String,
}

impl crate::MemoryRecord {
    /// History remains available through get/list even when it is inactive.
    pub fn is_active_for(&self, session_id: Option<i64>) -> bool {
        self.is_active_in_context(session_id, None)
    }

    pub fn is_active_in_context(&self, session_id: Option<i64>, instance_id: Option<&str>) -> bool {
        self.metadata.status == MemoryStatus::Active
            && !self.metadata.source_suppressed
            && self
                .metadata
                .scope
                .includes_context(session_id, instance_id)
    }
}
