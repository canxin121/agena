use std::{collections::HashMap, path::PathBuf, sync::Arc};

use agena_tool::SnapshotBackend;
use parking_lot::RwLock;

/// Process-local state for a session-managed workspace snapshot.
#[derive(Debug, Clone)]
pub struct SnapshotSession {
    pub path: PathBuf,
    pub branch: String,
    pub original_workspace: PathBuf,
    pub backend: SnapshotBackend,
    /// True when this process created the snapshot and owns cleanup.
    pub created_here: bool,
}

pub type SnapshotRegistry = Arc<RwLock<HashMap<i64, SnapshotSession>>>;

pub fn snapshot_registry() -> SnapshotRegistry {
    Arc::new(RwLock::new(HashMap::new()))
}

pub fn snapshot_rift_binary() -> String {
    std::env::var("AGENA_RIFT_BIN")
        .ok()
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "rift".to_owned())
}

/// A sorted projection of one active session-managed snapshot.
#[derive(Debug, Clone)]
pub struct ActiveSnapshot {
    pub session_id: i64,
    pub path: PathBuf,
    pub branch: String,
    pub backend: SnapshotBackend,
    pub created_here: bool,
}

pub fn list_active_snapshots(registry: &SnapshotRegistry) -> Vec<ActiveSnapshot> {
    let mut snapshots = registry
        .read()
        .iter()
        .map(|(session_id, session)| ActiveSnapshot {
            session_id: *session_id,
            path: session.path.clone(),
            branch: session.branch.clone(),
            backend: session.backend,
            created_here: session.created_here,
        })
        .collect::<Vec<_>>();
    snapshots.sort_by_key(|entry| entry.session_id);
    snapshots
}
