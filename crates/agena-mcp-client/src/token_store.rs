//! File-backed token store for MCP server credentials.
//!
//! Persists `(server_name → bearer_token)` to a JSON file (default
//! `~/.agena/mcp-tokens.json`) so users don't have to keep the token in
//! the agena config or an env var. Implements [`crate::TokenStore`] so
//! [`crate::McpConnectionManager`] can resolve `HttpAuth::BearerFromStore`
//! with one line of wiring.
//!
//! Storage is a flat object: `{ "<server>": { "bearer": "<token>" } }`.
//! No keyring integration, no per-account multi-tenancy — that lives in a
//! follow-up. The on-disk file is `chmod 600` on Unix so a stray
//! `cat ~/.agena/mcp-tokens.json` from another user fails closed.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::TokenStore;

const DEFAULT_RELATIVE_PATH: &str = ".agena/mcp-tokens.json";

#[derive(Debug, Error)]
pub enum TokenStoreError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("token store lock poisoned")]
    LockPoisoned,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    servers: BTreeMap<String, ServerEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bearer: Option<String>,
}

#[derive(Debug)]
pub struct FileTokenStore {
    path: PathBuf,
    inner: Mutex<StoreFile>,
}

impl FileTokenStore {
    /// Open or create the token file at the default `~/.agena/mcp-tokens.json`
    /// path. Returns Ok with an empty store if the file does not exist.
    pub fn open_default() -> Result<Self, TokenStoreError> {
        Self::open(&default_path())
    }

    pub fn open(path: &Path) -> Result<Self, TokenStoreError> {
        let inner = if path.exists() {
            let raw = fs::read_to_string(path)?;
            if raw.trim().is_empty() {
                StoreFile::default()
            } else {
                serde_json::from_str(&raw)?
            }
        } else {
            StoreFile::default()
        };
        Ok(Self {
            path: path.to_path_buf(),
            inner: Mutex::new(inner),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn put_bearer(&self, server: &str, token: &str) -> Result<(), TokenStoreError> {
        {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| TokenStoreError::LockPoisoned)?;
            let entry = guard
                .servers
                .entry(server.to_string())
                .or_insert_with(|| ServerEntry { bearer: None });
            entry.bearer = Some(token.to_string());
        }
        self.persist()
    }

    pub fn delete(&self, server: &str) -> Result<bool, TokenStoreError> {
        let removed = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| TokenStoreError::LockPoisoned)?;
            guard.servers.remove(server).is_some()
        };
        if removed {
            self.persist()?;
        }
        Ok(removed)
    }

    pub fn list_servers(&self) -> Result<Vec<String>, TokenStoreError> {
        let guard = self
            .inner
            .lock()
            .map_err(|_| TokenStoreError::LockPoisoned)?;
        Ok(guard.servers.keys().cloned().collect())
    }

    fn persist(&self) -> Result<(), TokenStoreError> {
        let snapshot = {
            let guard = self
                .inner
                .lock()
                .map_err(|_| TokenStoreError::LockPoisoned)?;
            guard.clone()
        };
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)?;
        }
        let body = serde_json::to_string_pretty(&snapshot)?;
        fs::write(&self.path, body)?;
        chmod_user_only(&self.path);
        Ok(())
    }
}

impl TokenStore for FileTokenStore {
    fn bearer(&self, server: &str) -> Option<String> {
        let guard = self.inner.lock().ok()?;
        guard
            .servers
            .get(server)
            .and_then(|e| e.bearer.clone())
            .filter(|t| !t.is_empty())
    }
}

fn default_path() -> PathBuf {
    let home = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| ".".to_string());
    PathBuf::from(home).join(DEFAULT_RELATIVE_PATH)
}

#[cfg(unix)]
fn chmod_user_only(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = fs::metadata(path) {
        let mut perms = meta.permissions();
        perms.set_mode(0o600);
        let _ = fs::set_permissions(path, perms);
    }
}

#[cfg(not(unix))]
fn chmod_user_only(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn tmp_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("agena-mcp-tokens-{label}-{suffix}.json"))
    }

    #[test]
    fn open_missing_file_returns_empty_store() {
        let path = tmp_path("missing");
        let store = FileTokenStore::open(&path).unwrap();
        assert!(store.list_servers().unwrap().is_empty());
        assert!(store.bearer("any").is_none());
    }

    #[test]
    fn put_and_lookup_round_trip() {
        let path = tmp_path("round-trip");
        let store = FileTokenStore::open(&path).unwrap();
        store.put_bearer("github", "abc123").unwrap();
        assert_eq!(store.bearer("github").as_deref(), Some("abc123"));
        // Reopen to check persistence.
        let store2 = FileTokenStore::open(&path).unwrap();
        assert_eq!(store2.bearer("github").as_deref(), Some("abc123"));
        assert_eq!(store2.list_servers().unwrap(), vec!["github".to_string()]);
    }

    #[test]
    fn delete_removes_entry_and_persists() {
        let path = tmp_path("delete");
        let store = FileTokenStore::open(&path).unwrap();
        store.put_bearer("a", "x").unwrap();
        store.put_bearer("b", "y").unwrap();
        assert!(store.delete("a").unwrap());
        assert!(!store.delete("missing").unwrap());
        let store2 = FileTokenStore::open(&path).unwrap();
        assert_eq!(store2.list_servers().unwrap(), vec!["b".to_string()]);
    }

    #[test]
    fn empty_token_is_treated_as_missing() {
        let path = tmp_path("empty");
        let store = FileTokenStore::open(&path).unwrap();
        store.put_bearer("srv", "").unwrap();
        assert!(store.bearer("srv").is_none());
    }

    #[test]
    fn corrupt_file_is_an_error_not_a_silent_overwrite() {
        let path = tmp_path("corrupt");
        fs::write(&path, "not json at all").unwrap();
        let err = FileTokenStore::open(&path).unwrap_err();
        assert!(matches!(err, TokenStoreError::Json(_)));
    }

    #[cfg(unix)]
    #[test]
    fn persisted_file_is_user_readable_only() {
        use std::os::unix::fs::PermissionsExt;
        let path = tmp_path("perms");
        let store = FileTokenStore::open(&path).unwrap();
        store.put_bearer("srv", "tok").unwrap();
        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "expected mode 600, got {mode:o}");
    }
}
