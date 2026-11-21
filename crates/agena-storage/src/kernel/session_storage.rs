use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::{Result, SessionId};
use crate::backend::kernel::KernelBackend;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct SessionStorageBytes {
    pub conversation: u64,
    pub recordings: u64,
    pub media: u64,
    pub files: u64,
    pub cache: u64,
    pub plugins: u64,
}

impl SessionStorageBytes {
    pub fn total(self) -> u64 {
        [
            self.conversation,
            self.recordings,
            self.media,
            self.files,
            self.cache,
            self.plugins,
        ]
        .into_iter()
        .fold(0, u64::saturating_add)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStorageSummary {
    pub session_id: SessionId,
    pub title: String,
    pub workspace_id: Option<i64>,
    pub workspace_name: Option<String>,
    pub bot_id: Option<String>,
    pub bot_name: Option<String>,
    pub surface: Option<String>,
    pub deleted: bool,
    pub busy: bool,
    pub busy_reason: Option<StorageProtection>,
    pub modified_at_ms: i64,
    pub bytes: SessionStorageBytes,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageProtection {
    Running,
    Shared,
    Referenced,
    Unmanaged,
    Reading,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionFileKind {
    Upload,
    Artifact,
    Cache,
    Plugin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStorageFile {
    pub id: String,
    pub name: String,
    pub kind: SessionFileKind,
    pub bytes: u64,
    pub protection: Option<StorageProtection>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStorageDetail {
    #[serde(flatten)]
    pub summary: SessionStorageSummary,
    pub shared_history: bool,
    pub files: Vec<SessionStorageFile>,
    pub file_count: usize,
    pub removable_recording_bytes: u64,
    pub operation: Option<StorageOperation>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SessionStorageQuery {
    pub offset: Option<usize>,
    pub limit: Option<usize>,
    pub bot: Option<String>,
    pub scope: Option<SessionStorageScope>,
    pub search: Option<String>,
    pub refresh: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize)]
pub struct SessionStorageDetailQuery {
    pub file_offset: Option<usize>,
    pub file_limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionStorageScope {
    Project,
    Bot,
    Deleted,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionStorageList {
    pub items: Vec<SessionStorageSummary>,
    pub total: usize,
    pub measured_at_ms: i64,
    pub bytes: u64,
    pub bots: Vec<StorageBot>,
    pub database: DatabaseStorage,
    pub shared: SharedStorage,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageBot {
    pub id: String,
    pub name: String,
    pub session_count: usize,
    pub session_bytes: u64,
    pub profile_bytes: u64,
    pub channel_bytes: u64,
    pub channel_files: u64,
}

#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct SharedStorage {
    pub workspace_files: u64,
    pub web: u64,
    pub memory: u64,
    pub logs: u64,
    pub plugins: u64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct DatabaseStorage {
    pub file_bytes: u64,
    pub wal_bytes: u64,
    pub reusable_bytes: u64,
    pub can_reclaim: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DatabaseReclaimResult {
    pub database: DatabaseStorage,
    pub released_bytes: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct StorageSelection {
    pub purge: bool,
    pub cache: bool,
    pub recordings: bool,
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct StorageCleanupPreview {
    pub token: String,
    pub file_bytes: u64,
    pub database_bytes: u64,
    pub file_count: usize,
    pub keeps_shared_history: bool,
    pub protection: Option<StorageProtection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ConfirmStorageCleanup {
    pub token: String,
    pub selection: StorageSelection,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StorageOperationState {
    Running,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StorageOperation {
    pub id: String,
    pub state: StorageOperationState,
    pub purge: bool,
    pub files_removed: usize,
    pub files_skipped: usize,
    pub file_bytes_removed: u64,
    pub database_bytes_removed: u64,
    pub keeps_shared_history: bool,
    pub detail: Option<String>,
}

#[derive(Clone)]
pub struct SessionStorage(pub(crate) Arc<dyn KernelBackend>);

impl SessionStorage {
    pub async fn list(&self, query: SessionStorageQuery) -> Result<SessionStorageList> {
        self.0.storage_list(query).await
    }

    pub async fn detail(
        &self,
        session: SessionId,
        query: SessionStorageDetailQuery,
    ) -> Result<SessionStorageDetail> {
        self.0.storage_detail(session, query).await
    }

    pub async fn operation(&self, session: SessionId) -> Result<Option<StorageOperation>> {
        self.0.storage_operation(session).await
    }

    pub async fn preview(
        &self,
        session: SessionId,
        selection: StorageSelection,
    ) -> Result<StorageCleanupPreview> {
        self.0.storage_preview(session, selection).await
    }

    pub async fn confirm(
        &self,
        session: SessionId,
        confirmation: ConfirmStorageCleanup,
    ) -> Result<StorageOperation> {
        self.0.storage_confirm(session, confirmation).await
    }

    pub async fn reclaim_database(&self) -> Result<DatabaseReclaimResult> {
        self.0.storage_reclaim_database().await
    }
}
