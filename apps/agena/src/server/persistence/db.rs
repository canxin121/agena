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

pub(crate) const KV_KEY_TERMINAL_SESSION_REGISTRY: &str = "terminal.sessionRegistry";
pub(crate) const KV_KEY_WORKSPACE_PREVIEW_SERVER_STATE: &str = "workspacePreview.state.server";
pub(crate) const KV_KEY_MCP_SERVER_CONTROL: &str = "mcp.server.control";
pub(crate) const KV_KEY_MCP_OAUTH_RUNTIME: &str = "mcp.oauth.runtime";
pub(crate) const KV_KEY_MCP_OAUTH_SIGNING_KEY: &str = "mcp.oauth.signing_key";

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
            secure_server_state_directory(parent).await?;
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
        secure_server_state_files(&path).await?;

        Ok(Self { path, pool })
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
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

#[cfg(unix)]
async fn secure_server_state_directory(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .await
        .map_err(|error| {
            format!(
                "failed to restrict Agena server-state directory {}: {error}",
                path.display()
            )
        })
}

#[cfg(not(unix))]
async fn secure_server_state_directory(_path: &Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
async fn secure_server_state_files(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt as _;

    for candidate in [
        path.to_path_buf(),
        PathBuf::from(format!("{}-wal", path.display())),
        PathBuf::from(format!("{}-shm", path.display())),
    ] {
        match tokio::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(0o600)).await {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to restrict Agena server-state file {}: {error}",
                    candidate.display()
                ));
            }
        }
    }
    Ok(())
}

#[cfg(not(unix))]
async fn secure_server_state_files(_path: &Path) -> Result<(), String> {
    Ok(())
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

#[cfg(all(test, unix))]
mod permission_tests {
    use std::os::unix::fs::PermissionsExt as _;

    use super::ServerStateDb;

    #[tokio::test]
    async fn server_state_directory_and_database_are_private() {
        let fixture = tempfile::tempdir().expect("create server-state permission fixture");
        let data_dir = fixture.path().join("agena-data");
        let path = data_dir.join("agena.db");
        let db = ServerStateDb::open_at_path(path.clone())
            .await
            .expect("open private server-state database");

        assert_eq!(
            std::fs::metadata(&data_dir)
                .expect("server-state directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            std::fs::metadata(db.path())
                .expect("server-state database metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
