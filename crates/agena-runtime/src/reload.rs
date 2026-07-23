use std::path::PathBuf;

use chrono::{DateTime, Utc};

/// Why the current runtime snapshot is being rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuntimeReloadCause {
    Manual,
    WatchedPathsChanged { paths: Vec<PathBuf> },
}

/// Result metadata for a completed runtime snapshot reload.
#[derive(Debug, Clone)]
pub struct RuntimeReloadReport {
    pub cause: RuntimeReloadCause,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}
