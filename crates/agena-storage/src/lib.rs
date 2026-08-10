//! # agena-storage
//!
//! Storage contracts shared by application services and concrete backends.
//!
//! This crate intentionally has no database, filesystem, or runtime-specific
//! implementation dependency. Concrete SQLite adapters
//! ([`agena_storage_sqlite`]) implement these contracts in their infrastructure
//! layer.
//!
//! ## Repositories
//!
//! - [`MemoryRepository`] — durable memory records.
//! - [`WorkspaceRepository`] — workspace metadata.
//! - [`ModelCatalogRepository`] — model catalog cache records.
//! - [`PermissionRuleRepository`] — permission rules and transactions.
//!
//! [`MemoryStore`] provides an in-memory implementation of most contracts for
//! tests and small deployments; [`StorageConfig`] carries store configuration.

use std::collections::hash_map::DefaultHasher;
use std::future::Future;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::pin::Pin;

use agena_domain::{PermissionMode, PermissionScope};
use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

mod memory_store;
pub use memory_store::MemoryStore;

pub mod store;

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
/// Error preparing storage configuration (for example creating the database directory).
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

/// Persisted permission rule shared by policy resolution and storage adapters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PersistedPermissionRule {
    pub id: Option<i64>,
    pub created_at_ms: Option<i64>,
    pub updated_at_ms: Option<i64>,
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
/// Parsed frontmatter of a memory document.
pub struct MemoryFrontmatter {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub r#type: Option<MemoryType>,
}

#[derive(Debug, Clone)]
/// A memory document as stored on disk.
pub struct MemoryRecord {
    pub file_name: String,
    pub path: PathBuf,
    pub frontmatter: MemoryFrontmatter,
    pub body: String,
}

#[derive(Debug, thiserror::Error)]
/// Error reading or writing memory records.
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

/// Result alias for memory operations.
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
/// Backend error from the workspace repository.
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
/// A stored workspace (id, path, timestamps).
pub struct WorkspaceRecord {
    pub id: i64,
    pub path: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

#[derive(Debug, Clone, Default)]
/// Query for listing workspaces.
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
/// Backend error from the model catalog repository.
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
/// Query for listing permission rules.
pub struct PermissionRuleListQuery {
    pub search: Option<String>,
    pub before_updated_at_ms: Option<i64>,
    pub before_id: Option<i64>,
    pub limit: u64,
}

#[derive(Debug, thiserror::Error)]
/// Backend error from the permission rule repository.
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

    /// Load every non-revoked rule visible to a session in one query.
    /// The session runtime caches this snapshot and invalidates it on writes.
    async fn resolve_snapshot(
        &self,
        session_id: Option<i64>,
        workspace_id: Option<i64>,
    ) -> Result<Vec<PersistedPermissionRule>, PermissionRuleRepositoryError>;
}

#[cfg(test)]
mod tests {
    use super::ModelCatalogCacheRecord;

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
/// Input for creating a new memory record.
pub struct NewMemory {
    pub name: String,
    pub description: String,
    pub memory_type: Option<MemoryType>,
    pub body: String,
    pub index_line: Option<String>,
}
