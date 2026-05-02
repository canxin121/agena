use std::{
    env, fs,
    path::{Path, PathBuf},
};

use crate::error::AppError;

/// Unified database storage configuration.
///
/// Resolves the SQLite database URL from (in priority order):
/// 1. An explicit `database_url` string (e.g. `sqlite::memory:` for tests)
/// 2. A `database_path` file path
/// 3. The default path `~/.agena/agena.db`
#[derive(Debug, Clone, Default)]
pub struct StorageConfig {
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
}

impl StorageConfig {
    /// Build from environment variables (`AGENA_DATABASE_URL` / `AGENA_DATABASE_PATH`).
    pub fn from_env() -> Self {
        Self {
            database_url: env::var("AGENA_DATABASE_URL").ok(),
            database_path: env::var("AGENA_DATABASE_PATH").ok().map(PathBuf::from),
        }
    }

    /// Resolve the final connection URL, creating the parent directory when
    /// necessary.
    pub fn resolve_url(&self) -> Result<String, AppError> {
        if let Some(url) = self.database_url.as_deref() {
            return Ok(url.to_owned());
        }
        let path = self
            .database_path
            .clone()
            .unwrap_or_else(Self::default_path);
        Ok(format!("sqlite://{}?mode=rwc", path.display()))
    }

    /// Ensure the parent directory of the database file exists.
    /// No-op for in-memory URLs.
    pub fn ensure_parent(url: &str) -> Result<(), AppError> {
        let Some(path) = sqlite_file_path(url) else {
            return Ok(());
        };
        if let Some(parent) = path.parent()
            && !parent.as_os_str().is_empty()
        {
            fs::create_dir_all(parent)?;
        }
        Ok(())
    }

    /// Default SQLite file path: `~/.agena/agena.db`.
    pub fn default_path() -> PathBuf {
        let mut base = home_dir().unwrap_or_else(|| PathBuf::from("."));
        base.push(".agena");
        base.push("agena.db");
        base
    }

    /// Human-readable label for the resolved URL (hides non-sqlite schemes).
    pub fn display_location(url: &str) -> String {
        sqlite_file_path(url)
            .map(|p| p.display().to_string())
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
    if path.is_empty() || path == ":memory:" {
        None
    } else {
        Some(Path::new(path).to_path_buf())
    }
}

fn home_dir() -> Option<PathBuf> {
    env::var("HOME")
        .or_else(|_| env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)
}
