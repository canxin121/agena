use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Serialize, de::DeserializeOwned};
use serde_json::Value;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

const DB_BUSY_TIMEOUT_MS: u64 = 15000;
const DB_POOL_MAX_CONNECTIONS: u32 = 16;
const DB_POOL_ACQUIRE_TIMEOUT_MS: u64 = 1500;
const DB_POOL_IDLE_TIMEOUT_SECS: u64 = 120;

pub(crate) const KV_KEY_SETTINGS: &str = "settings";
pub(crate) const KV_KEY_TERMINAL_SESSION_REGISTRY: &str = "terminal.sessionRegistry";
pub(crate) const KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE: &str = "workspacePreview.state.server";

#[derive(Debug, Clone)]
pub(crate) struct ServerStateDb {
    path: PathBuf,
    pool: SqlitePool,
}

impl ServerStateDb {
    pub(crate) async fn open() -> Result<Self, String> {
        Self::open_at_path(crate::server::persistence::paths::server_state_db_path()).await
    }

    pub(crate) async fn open_at_path(path: PathBuf) -> Result<Self, String> {
        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|err| err.to_string())?;
        }

        let options = SqliteConnectOptions::new()
            .filename(&path)
            .create_if_missing(true)
            .busy_timeout(Duration::from_millis(DB_BUSY_TIMEOUT_MS))
            .pragma("journal_mode", "WAL")
            .pragma("synchronous", "NORMAL")
            .pragma("foreign_keys", "ON")
            .pragma("temp_store", "MEMORY");

        let pool = SqlitePoolOptions::new()
            .max_connections(DB_POOL_MAX_CONNECTIONS)
            .acquire_timeout(Duration::from_millis(DB_POOL_ACQUIRE_TIMEOUT_MS))
            .idle_timeout(Some(Duration::from_secs(DB_POOL_IDLE_TIMEOUT_SECS)))
            .connect_with(options)
            .await
            .map_err(|err| err.to_string())?;

        initialize_schema(&pool).await?;

        Ok(Self { path, pool })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub(crate) async fn get_value(&self, key: &str) -> Result<Option<Value>, String> {
        let key = normalize_kv_key(key)?;
        let raw = sqlx::query_scalar::<_, String>(
            "SELECT value_json FROM server_kv WHERE key = ? LIMIT 1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|err| err.to_string())?;

        let Some(raw) = raw else {
            return Ok(None);
        };
        serde_json::from_str::<Value>(&raw)
            .map(Some)
            .map_err(|err| err.to_string())
    }

    pub(crate) async fn set_value(&self, key: &str, value: &Value) -> Result<(), String> {
        let key = normalize_kv_key(key)?;
        let payload = serde_json::to_string(value).map_err(|err| err.to_string())?;
        let now = now_unix_ms();
        sqlx::query(
            "INSERT INTO server_kv (key, value_json, updated_at) VALUES (?, ?, ?)\n             ON CONFLICT(key) DO UPDATE SET\n               value_json = excluded.value_json,\n               updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(payload)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|err| err.to_string())?;
        Ok(())
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, String> {
        let Some(value) = self.get_value(key).await? else {
            return Ok(None);
        };
        serde_json::from_value::<T>(value)
            .map(Some)
            .map_err(|err| err.to_string())
    }

    pub(crate) async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let json = serde_json::to_value(value).map_err(|err| err.to_string())?;
        self.set_value(key, &json).await
    }
}

async fn initialize_schema(pool: &SqlitePool) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|err| err.to_string())?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS server_kv (\n           key TEXT PRIMARY KEY,\n           value_json TEXT NOT NULL,\n           updated_at INTEGER NOT NULL\n         )",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_server_kv_updated_at ON server_kv(updated_at DESC)",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    // Attachment cache tables.
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS attachment_cache_blob_store (\n           digest_sha256 TEXT PRIMARY KEY,\n           bytes_b64 TEXT NOT NULL,\n           bytes_size INTEGER NOT NULL,\n           created_at INTEGER NOT NULL,\n           last_accessed_at INTEGER NOT NULL\n         )",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS attachment_cache_source_index (\n           source_path TEXT NOT NULL,\n           source_mtime_ns INTEGER NOT NULL,\n           source_size INTEGER NOT NULL,\n           mime TEXT NOT NULL,\n           digest_sha256 TEXT NOT NULL,\n           created_at INTEGER NOT NULL,\n           last_accessed_at INTEGER NOT NULL,\n           hit_count INTEGER NOT NULL DEFAULT 0,\n           PRIMARY KEY (source_path, source_mtime_ns, source_size, mime)\n         )",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_attachment_cache_source_last_accessed\n           ON attachment_cache_source_index(last_accessed_at DESC)",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_attachment_cache_blob_last_accessed\n           ON attachment_cache_blob_store(last_accessed_at DESC)",
    )
    .execute(&mut *tx)
    .await
    .map_err(|err| err.to_string())?;

    tx.commit().await.map_err(|err| err.to_string())?;
    Ok(())
}

fn normalize_kv_key(key: &str) -> Result<&str, String> {
    let trimmed = key.trim();
    if trimmed.is_empty() {
        return Err("db key is required".to_string());
    }
    if trimmed.len() > 200 {
        return Err("db key is too long".to_string());
    }
    Ok(trimmed)
}

fn now_unix_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
