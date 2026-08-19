//! Local marketplace cache layout under `~/agena/marketplace/` (override via
//! `AGENA_MARKETPLACE_DIR`). Provides path helpers, atomic write, secure
//! permissions, and persistence for `installed.json`.

use std::collections::BTreeMap;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::MarketplaceError;
use crate::manifest::{PluginKind, PluginVersion};

#[derive(Debug, Clone)]
/// Cache of marketplace plugin state.
pub struct MarketplaceCache {
    root: PathBuf,
}

impl MarketplaceCache {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn index_path(&self, registry_id: &str) -> PathBuf {
        self.root.join("index").join(format!("{registry_id}.json"))
    }

    pub fn installed_path(&self) -> PathBuf {
        self.root.join("installed.json")
    }

    pub fn plugin_dir(&self, plugin_id: &str, version: &str) -> PathBuf {
        self.root.join("plugins").join(plugin_id).join(version)
    }

    pub fn artifact_path(&self, plugin_id: &str, version: &str, kind: PluginKind) -> PathBuf {
        let mut path = self.plugin_dir(plugin_id, version);
        path.push(format!("binary.{}", kind.artifact_extension()));
        path
    }

    pub fn manifest_snapshot_path(&self, plugin_id: &str, version: &str) -> PathBuf {
        let mut path = self.plugin_dir(plugin_id, version);
        path.push("manifest.json");
        path
    }

    pub fn ensure_dirs(&self) -> Result<(), MarketplaceError> {
        ensure_dir(&self.root)?;
        ensure_dir(&self.root.join("index"))?;
        ensure_dir(&self.root.join("plugins"))?;
        Ok(())
    }

    pub fn load_installed(&self) -> Result<InstalledRecords, MarketplaceError> {
        let path = self.installed_path();
        if !path.exists() {
            return Ok(InstalledRecords::default());
        }
        let text = fs::read_to_string(&path)?;
        if text.trim().is_empty() {
            return Ok(InstalledRecords::default());
        }
        Ok(serde_json::from_str(&text)?)
    }

    pub fn save_installed(&self, records: &InstalledRecords) -> Result<(), MarketplaceError> {
        self.ensure_dirs()?;
        let path = self.installed_path();
        let json = serde_json::to_string_pretty(records)?;
        write_secure_file(&path, json.as_bytes())
    }

    pub fn save_manifest_snapshot(
        &self,
        plugin_id: &str,
        version: &PluginVersion,
    ) -> Result<(), MarketplaceError> {
        let path = self.manifest_snapshot_path(plugin_id, &version.version);
        if let Some(parent) = path.parent() {
            ensure_dir(parent)?;
        }
        let json = serde_json::to_string_pretty(version)?;
        write_secure_file(&path, json.as_bytes())
    }

    pub fn save_index(&self, registry_id: &str, bytes: &[u8]) -> Result<(), MarketplaceError> {
        self.ensure_dirs()?;
        let path = self.index_path(registry_id);
        write_secure_file(&path, bytes)
    }

    pub fn load_index_raw(&self, registry_id: &str) -> Result<Option<Vec<u8>>, MarketplaceError> {
        let path = self.index_path(registry_id);
        if !path.exists() {
            return Ok(None);
        }
        Ok(Some(fs::read(&path)?))
    }
}

fn ensure_dir(path: &Path) -> Result<(), MarketplaceError> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        secure_directory(path)?;
    }
    Ok(())
}

fn secure_directory(path: &Path) -> Result<(), MarketplaceError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

pub fn write_secure_file(path: &Path, contents: &[u8]) -> Result<(), MarketplaceError> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("secure file path has no parent: {}", path.display()),
        )
    })?;
    ensure_dir(parent)?;
    let mut builder = tempfile::Builder::new();
    builder.prefix(".agena-marketplace-").suffix(".tmp");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        builder.permissions(fs::Permissions::from_mode(0o600));
    }
    let mut staged = builder.tempfile_in(parent)?;
    staged.write_all(contents)?;
    staged.flush()?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_secure_file;

    #[test]
    fn secure_write_atomically_replaces_without_fixed_staging_file() {
        let directory = tempfile::tempdir().expect("marketplace directory");
        let path = directory.path().join("installed.json");

        write_secure_file(&path, b"first").expect("first write");
        write_secure_file(&path, b"replacement").expect("replacement write");

        assert_eq!(std::fs::read(&path).expect("read result"), b"replacement");
        assert_eq!(
            directory.path().read_dir().expect("read directory").count(),
            1
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                std::fs::metadata(path)
                    .expect("secure file metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Installed plugin records.
pub struct InstalledRecords {
    #[serde(default)]
    pub records: BTreeMap<String, InstalledRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One installed plugin record.
pub struct InstalledRecord {
    pub plugin_id: String,
    pub version: String,
    pub kind: PluginKind,
    pub platform: String,
    pub binary_path: PathBuf,
    pub config_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub installed_at: DateTime<Utc>,
    /// Registry id used to fetch this plugin. Required for `upgrade` and
    /// `outdated` to know which index to consult.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub registry_id: String,
    /// Registry index URL stored verbatim so upgrades can re-resolve without
    /// requiring the user to pass `--registry` every time.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub registry_url: String,
    /// Trust policy captured at install time. Upgrades must not silently
    /// weaken signature or GitHub provenance requirements.
    #[serde(default)]
    pub require_signature: bool,
    #[serde(default)]
    pub require_github_distribution: bool,
    /// True when the artifact was extracted from a tar.gz archive.
    #[serde(default)]
    pub archive_extracted: bool,
}

/// Default cache root: `$AGENA_MARKETPLACE_DIR` or `~/agena/marketplace`.
pub fn default_cache_root() -> PathBuf {
    if let Ok(path) = std::env::var("AGENA_MARKETPLACE_DIR") {
        return PathBuf::from(path);
    }
    let mut base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push("agena");
    base.push("marketplace");
    base
}
