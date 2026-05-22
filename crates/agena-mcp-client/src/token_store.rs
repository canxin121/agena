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
