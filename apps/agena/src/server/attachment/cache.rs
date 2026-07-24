use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use sha2::{Digest as _, Sha256};

use crate::server::persistence::db;

#[derive(Clone)]
pub(crate) struct AttachmentCacheManager {
    db: Arc<crate::server::persistence::db::ServerStateDb>,
}

#[derive(Debug, Clone)]
struct SourceSnapshot {
    source_path: String,
    source_mtime_ns: i64,
    source_size: i64,
}

impl AttachmentCacheManager {
    pub(crate) fn new(db: Arc<crate::server::persistence::db::ServerStateDb>) -> Self {
        Self { db }
    }

    pub(crate) async fn register_uploaded_file(
        &self,
        source: &Path,
        bytes: &[u8],
        mime: &str,
    ) -> Result<(), String> {
        let source_abs = normalize_source_path(source)?;
        let meta = tokio::fs::metadata(&source_abs)
            .await
            .map_err(|err| err.to_string())?;
        if !meta.is_file() {
            return Ok(());
        }

        let source = SourceSnapshot {
            source_path: source_abs.to_string_lossy().to_string(),
            source_mtime_ns: system_time_to_ns(meta.modified().unwrap_or(UNIX_EPOCH)),
            source_size: i64::try_from(meta.len()).unwrap_or(i64::MAX),
        };
        let mime = normalize_mime(mime);
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        self.persist_data_url(&source, &mime, bytes, &encoded).await
    }

    async fn persist_data_url(
        &self,
        source: &SourceSnapshot,
        mime: &str,
        bytes: &[u8],
        encoded: &str,
    ) -> Result<(), String> {
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        let digest = hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let pool = self.db.pool();
        let source = source.clone();
        let mime = mime.to_string();
        let digest_for_db = digest.clone();
        let bytes_size = i64::try_from(bytes.len()).unwrap_or(i64::MAX);

        let mut tx = pool.begin().await.map_err(|err| err.to_string())?;
        let now = now_unix_ms();

        sqlx::query(
            "INSERT INTO attachment_cache_blob_store\n               (digest_sha256, bytes_b64, bytes_size, created_at, last_accessed_at)\n             VALUES (?, ?, ?, ?, ?)\n             ON CONFLICT(digest_sha256) DO UPDATE SET\n               bytes_b64 = excluded.bytes_b64,\n               bytes_size = excluded.bytes_size,\n               last_accessed_at = excluded.last_accessed_at",
        )
        .bind(&digest_for_db)
        .bind(encoded)
        .bind(bytes_size)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

        sqlx::query(
            "INSERT INTO attachment_cache_source_index\n               (source_path, source_mtime_ns, source_size, mime, digest_sha256, created_at, last_accessed_at, hit_count)\n             VALUES (?, ?, ?, ?, ?, ?, ?, 0)\n             ON CONFLICT(source_path, source_mtime_ns, source_size, mime) DO UPDATE SET\n               digest_sha256 = excluded.digest_sha256,\n               last_accessed_at = excluded.last_accessed_at",
        )
        .bind(&source.source_path)
        .bind(source.source_mtime_ns)
        .bind(source.source_size)
        .bind(&mime)
        .bind(&digest)
        .bind(now)
        .bind(now)
        .execute(&mut *tx)
        .await
        .map_err(|err| err.to_string())?;

        tx.commit().await.map_err(|err| err.to_string())?;

        Ok(())
    }
}

fn normalize_source_path(source: &Path) -> Result<PathBuf, String> {
    if source.is_absolute() {
        Ok(source.to_path_buf())
    } else {
        let cwd = std::env::current_dir().map_err(|err| err.to_string())?;
        Ok(cwd.join(source))
    }
}

fn normalize_mime(mime: &str) -> String {
    let trimmed = mime.trim();
    if trimmed.is_empty() {
        return "application/octet-stream".to_string();
    }
    trimmed.to_string()
}

fn now_unix_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn system_time_to_ns(time: SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
