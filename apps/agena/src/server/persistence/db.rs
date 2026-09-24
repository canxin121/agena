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

fn database_diagnostic(
    context: impl AsRef<str>,
    error: &(dyn std::error::Error + 'static),
) -> String {
    agena_failure::diagnostic::format_error_chain_with_context(context, error)
}

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
            tokio::fs::create_dir_all(parent).await.map_err(|error| {
                database_diagnostic("failed to create the server-state directory", &error)
            })?;
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
            .map_err(|error| {
                database_diagnostic("failed to open the server-state SQLite database", &error)
            })?;

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
        .map_err(|error| {
            database_diagnostic("failed to read a server-state database value", &error)
        })?;

        let Some(raw) = raw else {
            return Ok(None);
        };
        serde_json::from_str::<Value>(&raw)
            .map(Some)
            .map_err(|error| {
                database_diagnostic("failed to decode a server-state database value", &error)
            })
    }

    pub(crate) async fn set_value(&self, key: &str, value: &Value) -> Result<(), String> {
        let key = normalize_kv_key(key)?;
        let payload = serde_json::to_string(value).map_err(|error| {
            database_diagnostic("failed to encode a server-state database value", &error)
        })?;
        let now = now_unix_ms();
        sqlx::query(
            "INSERT INTO server_kv (key, value_json, updated_at) VALUES (?, ?, ?)\n             ON CONFLICT(key) DO UPDATE SET\n               value_json = excluded.value_json,\n               updated_at = excluded.updated_at",
        )
        .bind(key)
        .bind(payload)
        .bind(now)
        .execute(&self.pool)
        .await
        .map_err(|error| {
            database_diagnostic("failed to write a server-state database value", &error)
        })?;
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
            .map_err(|error| {
                database_diagnostic("failed to decode typed server-state data", &error)
            })
    }

    pub(crate) async fn set_json<T: Serialize>(&self, key: &str, value: &T) -> Result<(), String> {
        let json = serde_json::to_value(value).map_err(|error| {
            database_diagnostic("failed to encode typed server-state data", &error)
        })?;
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

const SCHEMA_STATEMENTS: &[&str] = &[
    "CREATE TABLE IF NOT EXISTS server_kv (\n           key TEXT PRIMARY KEY,\n           value_json TEXT NOT NULL,\n           updated_at INTEGER NOT NULL\n         )",
    "CREATE INDEX IF NOT EXISTS idx_server_kv_updated_at ON server_kv(updated_at DESC)",
    "CREATE TABLE IF NOT EXISTS attachment_cache_blob_store (\n           digest_sha256 TEXT PRIMARY KEY,\n           bytes_b64 TEXT NOT NULL,\n           bytes_size INTEGER NOT NULL,\n           created_at INTEGER NOT NULL,\n           last_accessed_at INTEGER NOT NULL\n         )",
    "CREATE TABLE IF NOT EXISTS attachment_cache_source_index (\n           source_path TEXT NOT NULL,\n           source_mtime_ns INTEGER NOT NULL,\n           source_size INTEGER NOT NULL,\n           mime TEXT NOT NULL,\n           digest_sha256 TEXT NOT NULL,\n           created_at INTEGER NOT NULL,\n           last_accessed_at INTEGER NOT NULL,\n           hit_count INTEGER NOT NULL DEFAULT 0,\n           PRIMARY KEY (source_path, source_mtime_ns, source_size, mime)\n         )",
    "CREATE INDEX IF NOT EXISTS idx_attachment_cache_source_last_accessed\n           ON attachment_cache_source_index(last_accessed_at DESC)",
    "CREATE INDEX IF NOT EXISTS idx_attachment_cache_blob_last_accessed\n           ON attachment_cache_blob_store(last_accessed_at DESC)",
];

fn normalized_schema_sql(sql: &str) -> String {
    sql.replacen(" IF NOT EXISTS ", " ", 1)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn schema_identity(sql: &str) -> Result<(String, String), String> {
    for (prefix, kind) in [
        ("CREATE TABLE IF NOT EXISTS ", "table"),
        ("CREATE INDEX IF NOT EXISTS ", "index"),
        ("CREATE UNIQUE INDEX IF NOT EXISTS ", "index"),
    ] {
        if let Some(rest) = sql.strip_prefix(prefix) {
            let name = rest
                .split(|ch: char| ch.is_whitespace() || ch == '(')
                .next()
                .unwrap_or_default();
            if !name.is_empty() {
                return Ok((kind.to_owned(), name.to_owned()));
            }
        }
    }
    Err("invalid internal server-state schema declaration".to_owned())
}

async fn schema_objects(
    pool: &SqlitePool,
) -> Result<std::collections::BTreeMap<(String, String), String>, String> {
    let rows = sqlx::query_as::<_, (String, String, String)>(
        "SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*'",
    )
    .fetch_all(pool)
    .await
    .map_err(|error| database_diagnostic("failed to inspect the server-state schema", &error))?;
    Ok(rows
        .into_iter()
        .map(|(kind, name, sql)| ((kind, name), normalized_schema_sql(sql.as_str())))
        .collect())
}

async fn initialize_schema(pool: &SqlitePool) -> Result<(), String> {
    let actual = schema_objects(pool).await?;
    if !actual.is_empty() {
        let mut expected = std::collections::BTreeMap::new();
        for statement in SCHEMA_STATEMENTS {
            expected.insert(
                schema_identity(statement)?,
                normalized_schema_sql(statement),
            );
        }
        if actual == expected {
            return Ok(());
        }
        return Err(
            "server-state database schema does not match this Agena build; delete the database and start with a fresh one"
                .to_owned(),
        );
    }

    let mut tx = pool.begin().await.map_err(|error| {
        database_diagnostic(
            "failed to begin the server-state schema transaction",
            &error,
        )
    })?;
    for statement in SCHEMA_STATEMENTS {
        sqlx::query(*statement)
            .execute(&mut *tx)
            .await
            .map_err(|error| database_diagnostic("failed to create server-state schema", &error))?;
    }
    tx.commit().await.map_err(|error| {
        database_diagnostic(
            "failed to commit the server-state schema transaction",
            &error,
        )
    })?;
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

    #[tokio::test]
    async fn modified_server_state_schema_is_rejected_without_repair() {
        let fixture = tempfile::tempdir().expect("create server-state schema fixture");
        let path = fixture.path().join("agena.db");
        let db = ServerStateDb::open_at_path(path.clone())
            .await
            .expect("create current server-state database");
        sqlx::query("ALTER TABLE server_kv ADD COLUMN obsolete TEXT")
            .execute(&db.pool)
            .await
            .expect("mutate server-state schema");
        drop(db);

        let error = ServerStateDb::open_at_path(path)
            .await
            .expect_err("modified server-state schema must be rejected");
        assert!(error.contains("does not match this Agena build"), "{error}");
    }
}
