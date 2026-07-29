//! Storage contracts shared by application services and concrete backends.
//!
//! This crate intentionally has no database, filesystem, or runtime-specific
//! implementation dependency. Concrete SQLite/SeaORM adapters implement these
//! traits in their infrastructure layer.

use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicI64, Ordering};

use agena_domain::MessageId;
use agena_domain::{EventEnvelope, EventFilter, KindMatcher, PermissionMode, PermissionScope};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod memory_store;
pub use memory_store::MemoryStore;

/// Unified database storage configuration.
///
/// Resolves the SQLite database URL from (in priority order): an explicit
/// URL, a filesystem path, or the conventional user-local database path.
/// This is storage policy rather than Core runtime composition, so every
/// process bootstrapper can use it without importing the legacy monolith.
#[derive(Debug, Clone, Default)]
pub struct StorageConfig {
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
}

#[derive(Debug, thiserror::Error)]
pub enum StorageConfigError {
    #[error("failed to prepare database directory: {0}")]
    Io(#[from] io::Error),
}

impl StorageConfig {
    /// Build from `AGENA_DATABASE_URL` / `AGENA_DATABASE_PATH`.
    pub fn from_env() -> Self {
        Self {
            database_url: std::env::var("AGENA_DATABASE_URL").ok(),
            database_path: std::env::var("AGENA_DATABASE_PATH").ok().map(PathBuf::from),
        }
    }

    pub fn resolve_url(&self) -> Result<String, StorageConfigError> {
        if let Some(url) = self.database_url.as_deref() {
            return Ok(url.to_owned());
        }
        let path = self
            .database_path
            .clone()
            .unwrap_or_else(Self::default_path);
        Ok(format!("sqlite://{}?mode=rwc", path.display()))
    }

    pub fn ensure_parent(url: &str) -> Result<(), StorageConfigError> {
        let Some(path) = sqlite_file_path(url) else {
            return Ok(());
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    pub fn default_path() -> PathBuf {
        let mut base = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.push("agena");
        base.push("agena.db");
        base
    }

    pub fn display_location(url: &str) -> String {
        sqlite_file_path(url)
            .map(|path| path.display().to_string())
            .unwrap_or_else(|| {
                if url.starts_with("sqlite:") {
                    url.to_owned()
                } else {
                    "<redacted>".to_owned()
                }
            })
    }
}

fn sqlite_file_path(url: &str) -> Option<PathBuf> {
    if url == "sqlite::memory:" {
        return None;
    }
    let raw = url.strip_prefix("sqlite://")?;
    let path = raw.split('?').next().unwrap_or(raw);
    (!path.is_empty() && path != ":memory:").then(|| Path::new(path).to_path_buf())
}

#[cfg(test)]
mod storage_config_tests {
    use std::path::PathBuf;

    use super::StorageConfig;

    #[test]
    fn explicit_database_url_wins_over_database_path() {
        let config = StorageConfig {
            database_url: Some("sqlite::memory:".to_string()),
            database_path: Some(PathBuf::from("ignored.db")),
        };

        assert_eq!(config.resolve_url().unwrap(), "sqlite::memory:");
    }

    #[test]
    fn database_path_becomes_a_sqlite_creation_url() {
        let config = StorageConfig {
            database_url: None,
            database_path: Some(PathBuf::from("state/agena.db")),
        };

        assert_eq!(
            config.resolve_url().unwrap(),
            "sqlite://state/agena.db?mode=rwc"
        );
    }

    #[test]
    fn display_location_preserves_sqlite_and_redacts_other_urls() {
        assert_eq!(
            StorageConfig::display_location("sqlite::memory:"),
            "sqlite::memory:"
        );
        assert_eq!(
            StorageConfig::display_location("postgres://secret@host/database"),
            "<redacted>"
        );
    }
}

type TransactionEffectFuture = Pin<Box<dyn Future<Output = ()> + Send>>;

/// Backend-neutral queue of effects that run only after the surrounding
/// transaction has committed. Database adapters own begin/commit/rollback;
/// this storage primitive owns the deferred-effect ordering contract.
pub struct TransactionEffects {
    effects: Vec<TransactionEffectFuture>,
}

impl TransactionEffects {
    pub fn new() -> Self {
        Self {
            effects: Vec::new(),
        }
    }

    pub fn push<F>(&mut self, effect: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        self.effects.push(Box::pin(effect));
    }

    pub async fn run(self) {
        for effect in self.effects {
            effect.await;
        }
    }
}

impl Default for TransactionEffects {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EventStoreError {
    #[error("event store backend error: {0}")]
    Backend(String),
    #[error("event with seq_global={0} already exists")]
    DuplicateSeq(i64),
    #[error("invalid range: {0}")]
    InvalidRange(String),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
}

/// Process-local monotonic allocator for persisted event sequence numbers.
///
/// The runtime initializes it from the event-store high watermark before
/// publishing resumes. Storage owns this primitive because its value is part
/// of the persistence contract, while the event publisher owns orchestration.
#[derive(Debug)]
pub struct SequenceAllocator {
    next: AtomicI64,
}

impl SequenceAllocator {
    pub fn new() -> Self {
        Self::from_high_watermark(0)
    }

    pub fn from_high_watermark(highest: i64) -> Self {
        Self {
            next: AtomicI64::new(Self::next_after(highest)),
        }
    }

    pub fn init_from(&self, highest: i64) {
        self.next.store(Self::next_after(highest), Ordering::SeqCst);
    }

    pub fn next(&self) -> i64 {
        self.next.fetch_add(1, Ordering::SeqCst)
    }

    pub fn peek(&self) -> i64 {
        self.next.load(Ordering::SeqCst)
    }

    fn next_after(highest: i64) -> i64 {
        highest.saturating_add(1).max(1)
    }
}

impl Default for SequenceAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Persisted permission rule shared by policy resolution and storage adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedPermissionRule {
    pub action_key: String,
    pub mode: PermissionMode,
    pub scope: PermissionScope,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub source: String,
    pub reason: Option<String>,
    pub operator: Option<String>,
    pub revoked_at_ms: Option<i64>,
    pub revoked_reason: Option<String>,
    pub revoked_by: Option<String>,
}

/// Classification stored in a persistent memory document's frontmatter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
    #[serde(other)]
    Other,
}

impl MemoryType {
    pub fn label(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct MemoryFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub r#type: Option<MemoryType>,
}

#[derive(Debug, Clone)]
pub struct MemoryRecord {
    pub file_name: String,
    pub path: PathBuf,
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MemoryError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("malformed memory file `{path}`: {message}")]
    Malformed { path: PathBuf, message: String },
    #[error("yaml frontmatter error: {0}")]
    Yaml(#[from] serde_yaml::Error),
    #[error("memory `{0}` not found")]
    NotFound(String),
}

pub type MemoryResult<T> = Result<T, MemoryError>;

/// Provider-independent memory persistence port.
pub trait MemoryRepository: Send + Sync {
    fn directory(&self) -> PathBuf;
    fn ensure_index(&self) -> MemoryResult<PathBuf>;
    fn list(&self) -> MemoryResult<Vec<MemoryRecord>>;
    fn get(&self, name: &str) -> MemoryResult<MemoryRecord>;
    fn index_lines(&self) -> MemoryResult<Vec<String>>;
    fn forget(&self, name: &str) -> MemoryResult<()>;
    fn save(&self, entry: NewMemory) -> MemoryResult<MemoryRecord>;
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceRepositoryError {
    #[error("invalid workspace path: {0}")]
    InvalidPath(String),
    #[error("workspace repository backend error: {0}")]
    Backend(String),
}

/// Provider-independent persistence port for workspace identity resolution.
#[async_trait]
pub trait WorkspaceRepository: Send + Sync {
    async fn create(&self, path: String) -> Result<WorkspaceRecord, WorkspaceRepositoryError>;

    async fn update_path(
        &self,
        workspace_id: i64,
        path: String,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError>;

    async fn delete(
        &self,
        workspace_id: i64,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError>;

    async fn get(
        &self,
        workspace_id: i64,
    ) -> Result<Option<WorkspaceRecord>, WorkspaceRepositoryError>;

    async fn list(
        &self,
        query: WorkspaceListQuery,
    ) -> Result<Vec<WorkspaceRecord>, WorkspaceRepositoryError>;

    async fn path_by_id(
        &self,
        workspace_id: i64,
    ) -> Result<Option<String>, WorkspaceRepositoryError>;

    async fn lookup_id(
        &self,
        workspace_path: &str,
    ) -> Result<Option<i64>, WorkspaceRepositoryError>;

    async fn ensure_id(&self, workspace_path: &str) -> Result<i64, WorkspaceRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub id: i64,
    pub path: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceListQuery {
    pub search: Option<String>,
    pub before_updated_at_ms: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: u64,
}

/// Raw persisted model-catalog cache value. The provider/domain-specific
/// document is intentionally opaque JSON at this storage boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelCatalogCacheRecord {
    pub fetched_at_unix_ms: i64,
    pub source: String,
    pub document: serde_json::Value,
}

#[derive(Debug, thiserror::Error)]
pub enum ModelCatalogRepositoryError {
    #[error("model catalog repository backend error: {0}")]
    Backend(String),
    #[error("model catalog cache serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

/// Persistence port for the model-catalog cache.
///
/// `write_cache` replaces one complete logical snapshot: implementations must
/// make the catalog entry rows and its snapshot metadata visible as one
/// atomic update. A failed write must not expose a partially replaced cache to
/// a concurrent reader. The port deliberately keeps the document opaque so
/// that this transaction guarantee does not leak SeaORM or core model types.
#[async_trait]
pub trait ModelCatalogRepository: Send + Sync {
    async fn read_cache(
        &self,
    ) -> Result<Option<ModelCatalogCacheRecord>, ModelCatalogRepositoryError>;

    async fn write_cache(
        &self,
        record: &ModelCatalogCacheRecord,
    ) -> Result<(), ModelCatalogRepositoryError>;
}

/// Stable permission-rule row exposed by persistence adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRuleRecord {
    pub id: i64,
    pub action_key: String,
    pub mode: String,
    pub scope: String,
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub source: String,
    pub reason: Option<String>,
    pub operator: Option<String>,
    pub revoked_at_ms: Option<i64>,
    pub revoked_reason: Option<String>,
    pub revoked_by: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
pub struct PermissionRuleListQuery {
    pub search: Option<String>,
    pub before_updated_at_ms: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum PermissionRuleRepositoryError {
    #[error("permission rule repository backend error: {0}")]
    Backend(String),
}

/// Writes permission rules inside a transaction supplied by the concrete
/// persistence adapter. The storage contract deliberately keeps the
/// transaction type generic so it does not expose a database implementation.
#[async_trait]
pub trait PermissionRuleTransactionWriter<Transaction>: Send + Sync {
    async fn upsert_in_transaction(
        &self,
        transaction: &Transaction,
        rule: &PersistedPermissionRule,
    ) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError>;
}

/// Read-only permission-rule persistence port for application queries.
#[async_trait]
pub trait PermissionRuleRepository: Send + Sync {
    async fn list(
        &self,
        query: PermissionRuleListQuery,
    ) -> Result<Vec<PermissionRuleRecord>, PermissionRuleRepositoryError>;

    async fn get(
        &self,
        rule_id: i64,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError>;

    async fn upsert(
        &self,
        rule: &PersistedPermissionRule,
    ) -> Result<(PermissionRuleRecord, bool), PermissionRuleRepositoryError>;

    async fn replace(
        &self,
        rule_id: i64,
        rule: &PersistedPermissionRule,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError>;

    async fn revoke(
        &self,
        rule_id: i64,
        revoked_reason: Option<String>,
        revoked_by: Option<String>,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError>;

    async fn delete(
        &self,
        rule_id: i64,
    ) -> Result<Option<PermissionRuleRecord>, PermissionRuleRepositoryError>;

    async fn resolve(
        &self,
        action_key: &str,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<Vec<PersistedPermissionRule>, PermissionRuleRepositoryError>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionEventStats {
    pub message_count: i64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionStatsRepositoryError {
    #[error("session stats repository backend error: {0}")]
    Backend(String),
}

/// Read-only session statistics needed by application-facing resources.
#[async_trait]
pub trait SessionStatsRepository: Send + Sync {
    async fn workspace_counts(
        &self,
        workspace_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, i64>, SessionStatsRepositoryError>;

    async fn event_stats(
        &self,
        session_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, SessionEventStats>, SessionStatsRepositoryError>;

    async fn child_counts(
        &self,
        parent_ids: &[i64],
    ) -> Result<std::collections::HashMap<i64, i64>, SessionStatsRepositoryError>;
}

/// Storage-neutral serialized provider usage. Keeping the complete JSON value
/// preserves new token, cache, cost-provenance, billable-unit, and attributed
/// request fields without coupling this crate to `agena-provider`.
#[derive(Debug, Clone, PartialEq)]
pub struct UsageSample {
    pub value: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageRecord {
    pub session_id: i64,
    pub session_title: String,
    pub is_subagent: bool,
    pub created_at_ms: i64,
    pub provider_id: String,
    pub model_id: String,
    pub usage: UsageSample,
}

#[derive(Debug, thiserror::Error)]
pub enum UsageRepositoryError {
    #[error("usage repository backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait UsageRepository: Send + Sync {
    async fn list(
        &self,
        workspace_id: i64,
        session_ids: &[i64],
        include_subagents: bool,
        from_ms: Option<i64>,
        to_ms: Option<i64>,
    ) -> Result<Vec<UsageRecord>, UsageRepositoryError>;
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionLookupRepositoryError {
    #[error("projection lookup repository backend error: {0}")]
    Backend(String),
}

#[async_trait]
pub trait ProjectionLookupRepository: Send + Sync {
    async fn session_id_for_message(
        &self,
        message_id: i64,
    ) -> Result<Option<i64>, ProjectionLookupRepositoryError>;

    async fn session_id_for_part(
        &self,
        part_id: i64,
    ) -> Result<Option<i64>, ProjectionLookupRepositoryError>;
}

/// Stable header fields for one materialized message projection. Metadata and
/// usage remain opaque JSON at the storage boundary because their concrete
/// Core aggregate types are intentionally not persistence contracts.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionHeaderRecord {
    pub message_id: i64,
    pub turn_id: Option<i64>,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at_ms: i64,
    pub metadata: serde_json::Value,
    pub provider_state: Option<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
    pub part_count: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionPartRecord {
    pub part_id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: agena_domain::ExecutionStatus,
    pub kind: agena_domain::PartKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub has_detail: bool,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum MessageProjectionRepositoryError {
    #[error("message projection repository backend error: {0}")]
    Backend(String),
}

/// Read-only access to already-materialized message projection headers.
/// Projection synchronization and Core aggregate decoding remain outside this
/// adapter boundary.
#[async_trait]
pub trait MessageProjectionRepository: Send + Sync {
    async fn list_headers(
        &self,
        session_id: i64,
    ) -> Result<Vec<MessageProjectionHeaderRecord>, MessageProjectionRepositoryError>;

    async fn list_headers_page(
        &self,
        session_id: i64,
        cursor: Option<(i64, i64)>,
        limit: u64,
    ) -> Result<
        (Vec<MessageProjectionHeaderRecord>, bool, Option<(i64, i64)>),
        MessageProjectionRepositoryError,
    >;

    async fn get_header(
        &self,
        session_id: i64,
        message_id: i64,
    ) -> Result<Option<MessageProjectionHeaderRecord>, MessageProjectionRepositoryError>;

    async fn list_parts(
        &self,
        message_ids: &[i64],
        include_content: bool,
    ) -> Result<Vec<MessageProjectionPartRecord>, MessageProjectionRepositoryError>;

    async fn get_part(
        &self,
        part_id: i64,
    ) -> Result<Option<MessageProjectionPartRecord>, MessageProjectionRepositoryError>;
}

/// Stable write request for one materialized message projection. JSON payloads
/// remain opaque at this persistence boundary while Core owns lifecycle
/// materialization and aggregate decoding.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionMessageWrite {
    pub message_id: i64,
    pub session_id: i64,
    pub turn_id: Option<i64>,
    pub execution_id: Option<String>,
    pub run_id: Option<String>,
    pub role: agena_domain::Role,
    pub state: agena_domain::ExecutionStatus,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub metadata: serde_json::Value,
    pub provider_state: Option<serde_json::Value>,
    pub usage: Option<serde_json::Value>,
    pub part_count: i64,
    pub is_hidden: bool,
}

/// Stable write request for one materialized message part. Content remains
/// opaque at this persistence boundary; Core owns lifecycle materialization
/// until the full transcript projection contract moves.
#[derive(Debug, Clone, PartialEq)]
pub struct MessageProjectionPartWrite {
    pub session_id: i64,
    pub part_id: i64,
    pub message_id: i64,
    pub part_index: i32,
    pub status: agena_domain::ExecutionStatus,
    pub kind: agena_domain::PartKind,
    pub name: Option<String>,
    pub summary: Option<String>,
    pub has_detail: bool,
    pub operation_id: Option<String>,
    pub created_at_ms: i64,
    pub content: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageProjectionOpenIdentity {
    RunId(String),
    ExecutionId(String),
}

/// Transaction-scoped writer for projection rows. The generic transaction
/// keeps backend implementation types out of the storage contract.
#[async_trait]
pub trait MessageProjectionTransactionWriter<Transaction>: Send + Sync {
    async fn terminalize_open_messages_in_transaction(
        &self,
        transaction: &Transaction,
        session_id: i64,
        identity: &MessageProjectionOpenIdentity,
        status: agena_domain::ExecutionStatus,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError>;

    async fn clear_session_projection_in_transaction(
        &self,
        transaction: &Transaction,
        session_id: i64,
    ) -> Result<(), MessageProjectionRepositoryError>;

    async fn upsert_projection_watermark_in_transaction(
        &self,
        transaction: &Transaction,
        session_id: i64,
        last_seq_global: i64,
        updated_at_ms: i64,
    ) -> Result<(), MessageProjectionRepositoryError>;

    async fn upsert_message_in_transaction(
        &self,
        transaction: &Transaction,
        message: &MessageProjectionMessageWrite,
    ) -> Result<(), MessageProjectionRepositoryError>;

    async fn upsert_part_in_transaction(
        &self,
        transaction: &Transaction,
        part: &MessageProjectionPartWrite,
    ) -> Result<(), MessageProjectionRepositoryError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionSummaryRecord {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub relation_kind: agena_domain::SessionRelationKind,
    pub lifecycle_state: agena_domain::SessionLifecycleState,
    pub source_cutoff_seq_global: Option<i64>,
    pub source_message_id: Option<i64>,
    pub task_id: Option<String>,
    pub subtask_access: Option<agena_domain::ExecutionAccess>,
    pub subtask_status: Option<agena_domain::SubtaskStatus>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionTreeRecord {
    pub summary: SessionSummaryRecord,
    pub message_count: i64,
    pub child_session_count: i64,
    pub last_message_at_ms: Option<i64>,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionSummaryRepositoryError {
    #[error("session summary repository backend error: {0}")]
    Backend(String),
}

/// Read-only session metadata without core-owned runtime JSON.
#[async_trait]
pub trait SessionSummaryRepository: Send + Sync {
    async fn get(
        &self,
        session_id: i64,
    ) -> Result<Option<SessionSummaryRecord>, SessionSummaryRepositoryError>;

    async fn list(
        &self,
        query: SessionSummaryListQuery,
    ) -> Result<Vec<SessionSummaryRecord>, SessionSummaryRepositoryError>;

    async fn get_subagent_by_task_id(
        &self,
        parent_session_id: i64,
        task_id: &str,
    ) -> Result<Option<SessionSummaryRecord>, SessionSummaryRepositoryError>;

    async fn list_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionTreeRecord>, SessionSummaryRepositoryError>;
}

#[derive(Debug, Clone, Default)]
pub struct SessionSummaryListQuery {
    pub workspace_id: Option<i64>,
    pub roots_only: bool,
    pub parent_id: Option<i64>,
    pub search: Option<String>,
    pub before_updated_at_ms: Option<i64>,
    pub before_id: Option<i64>,
    pub offset: u64,
    pub limit: u64,
    pub include_subagents: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum SessionMutationRepositoryError {
    #[error("session mutation repository backend error: {0}")]
    Backend(String),
}

/// Simple session mutations that do not own branch/runtime transaction state.
#[async_trait]
pub trait SessionMutationRepository: Send + Sync {
    async fn create(
        &self,
        workspace_id: i64,
        parent_id: Option<i64>,
        title: String,
    ) -> Result<SessionSummaryRecord, SessionMutationRepositoryError>;

    async fn rename(
        &self,
        session_id: i64,
        title: String,
    ) -> Result<Option<SessionSummaryRecord>, SessionMutationRepositoryError>;

    async fn delete(&self, session_id: i64) -> Result<u64, SessionMutationRepositoryError>;
}

/// Allocator port used by streaming session history buffers.
pub trait MessageIdAllocator {
    fn next_message_id(&mut self) -> MessageId;
}

#[derive(Debug, Default)]
pub struct SequentialIdAllocator {
    next: i64,
}

/// In-memory state shared by session message/part ID allocators.
#[derive(Debug, Default)]
pub struct GlobalIdAllocator {
    pub initialized: bool,
    pub next_message_id: i64,
    pub next_part_id: i64,
}

impl SequentialIdAllocator {
    pub fn starting_at(start: i64) -> Self {
        Self { next: start }
    }
}

impl MessageIdAllocator for SequentialIdAllocator {
    fn next_message_id(&mut self) -> MessageId {
        let id = self.next;
        self.next += 1;
        MessageId(id)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MessageIdAllocator, ModelCatalogCacheRecord, SequenceAllocator, SequentialIdAllocator,
    };

    #[test]
    fn sequence_allocator_resumes_after_high_watermark() {
        let allocator = SequenceAllocator::from_high_watermark(41);
        assert_eq!(allocator.peek(), 42);
        assert_eq!(allocator.next(), 42);
        assert_eq!(allocator.next(), 43);
        allocator.init_from(100);
        assert_eq!(allocator.next(), 101);
    }

    #[test]
    fn sequence_allocator_never_returns_zero_for_negative_watermark() {
        let allocator = SequenceAllocator::from_high_watermark(-1);
        assert_eq!(allocator.peek(), 1);
        assert_eq!(allocator.next(), 1);
    }

    #[test]
    fn sequential_allocator_preserves_requested_start() {
        let mut allocator = SequentialIdAllocator::starting_at(41);
        assert_eq!(allocator.next_message_id().0, 41);
        assert_eq!(allocator.next_message_id().0, 42);
    }

    #[test]
    fn model_catalog_cache_record_preserves_opaque_document_shape() {
        let record = ModelCatalogCacheRecord {
            fetched_at_unix_ms: 42,
            source: "generated".to_owned(),
            document: serde_json::json!({"models": {"demo": {"display_name": "Demo"}}}),
        };
        let encoded = serde_json::to_value(&record).unwrap();
        let decoded: ModelCatalogCacheRecord = serde_json::from_value(encoded).unwrap();
        assert_eq!(decoded, record);
    }
}

const MAX_MEMORY_PATH_LENGTH: usize = 200;

fn sanitize_memory_path(path: &str) -> String {
    let sanitized: String = path
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    if sanitized.len() <= MAX_MEMORY_PATH_LENGTH {
        return sanitized;
    }
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    format!(
        "{}-{:x}",
        &sanitized[..MAX_MEMORY_PATH_LENGTH],
        hasher.finish()
    )
}

fn memory_base_dir() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(home).join("agena")
}

/// Filesystem location contract for workspace-scoped memory documents.
pub struct MemoryDir {
    path: PathBuf,
}

impl MemoryDir {
    pub fn from_workspace(workspace_root: &std::path::Path) -> Self {
        let normalized = workspace_root.to_string_lossy().replace('\\', "/");
        Self {
            path: memory_base_dir()
                .join("projects")
                .join(sanitize_memory_path(&normalized))
                .join("memory"),
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn entrypoint(&self) -> PathBuf {
        self.path.join("MEMORY.md")
    }

    pub fn index_dir(&self) -> PathBuf {
        self.path.join(".index")
    }

    pub fn ensure_exists(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.path)
    }
}

#[derive(Debug, Clone)]
pub struct NewMemory {
    pub name: String,
    pub description: String,
    pub memory_type: Option<MemoryType>,
    pub body: String,
    pub index_line: Option<String>,
}

/// Inclusive lower bound, exclusive upper bound by `seq_global`.
#[derive(Debug, Clone, Copy)]
pub struct StoreRange {
    pub after_seq_global: i64,
    pub limit: usize,
}

/// Descending event-log range ending immediately before an optional global
/// sequence. This supports cursor pages that render oldest-to-newest after a
/// newest-first storage fetch without materializing an entire session log.
#[derive(Debug, Clone, Copy)]
pub struct ReverseStoreRange {
    pub before_seq_global: Option<i64>,
    pub limit: usize,
}

/// Persistent event-log contract.
#[async_trait]
pub trait EventStore<K>: Send + Sync
where
    K: KindMatcher + Send + Sync + Clone + 'static,
{
    async fn append_batch(&self, events: &[EventEnvelope<K>]) -> Result<(), EventStoreError>;

    async fn range(
        &self,
        filter: &EventFilter,
        range: StoreRange,
    ) -> Result<Vec<EventEnvelope<K>>, EventStoreError>;

    /// Returns matching events in descending `seq_global` order.
    async fn range_before(
        &self,
        filter: &EventFilter,
        range: ReverseStoreRange,
    ) -> Result<Vec<EventEnvelope<K>>, EventStoreError>;

    async fn high_watermark(&self) -> Result<Option<i64>, EventStoreError>;

    async fn session_high_watermark(&self, session_id: i64)
    -> Result<Option<i64>, EventStoreError>;
}
