use chrono::{DateTime, Utc};

/// Immutable identity/timestamp metadata for one runtime snapshot.
#[derive(Debug, Clone)]
pub(crate) struct SnapshotMetadata {
    generation: u64,
    loaded_at: DateTime<Utc>,
}

impl SnapshotMetadata {
    pub(crate) fn new(generation: u64) -> Self {
        Self {
            generation,
            loaded_at: Utc::now(),
        }
    }

    pub(crate) fn generation(&self) -> u64 {
        self.generation
    }

    pub(crate) fn loaded_at(&self) -> DateTime<Utc> {
        self.loaded_at
    }
}
