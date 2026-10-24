//! Local marketplace cache layout under the instance data directory's
//! `marketplace/`. Provides path helpers, atomic write, secure
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
            return Err(MarketplaceError::Config(format!(
                "installed plugin state at {} is empty; delete it and reinstall current plugins",
                path.display()
            )));
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
    agena_storage_layout::secure_directory(path)?;
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
    agena_storage_layout::secure_file(staged.path())?;
    staged.as_file().sync_all()?;
    staged.persist(path).map_err(|error| error.error)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
/// Installed plugin records.
pub struct InstalledRecords {
    pub records: BTreeMap<String, InstalledRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    pub registry_id: String,
    /// Registry index URL stored verbatim so upgrades can re-resolve without
    /// requiring the user to pass `--registry` every time.
    pub registry_url: String,
    /// Trust policy captured at install time. Upgrades must not silently
    /// weaken signature or GitHub provenance requirements.
    pub require_signature: bool,
    pub require_github_distribution: bool,
    /// True when the artifact was extracted from a tar.gz archive.
    pub archive_extracted: bool,
}

/// Marketplace state for the selected Agena data directory.
pub fn default_cache_root() -> PathBuf {
    agena_storage_layout::StorageLayout::current().marketplace_dir()
}
