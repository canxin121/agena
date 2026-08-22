//! Plugin substrate storage and secret backends.
//!
//! These are the host-side implementations that back the plugin SDK's
//! `storage_*` and `secret_*` `HostClient` methods. Both surfaces are scoped
//! for plugins while keeping private/plugin-owned data separate from
//! intentionally shared data.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agena_keyring_store::{KeyringSecretStore, SecretStore, SecretStoreError};
use agena_plugin_sdk::PluginKey;

use agena_plugin_host::sdk::host_api::{HostStorageScope, HostStorageVisibility};

#[derive(Debug, thiserror::Error)]
/// Error from plugin storage.
pub enum PluginStorageError {
    #[error("session-scoped storage requires a session id")]
    MissingSessionId,
    #[error("workspace-scoped storage requires a workspace root")]
    MissingWorkspaceRoot,
    #[error("namespace must not be empty")]
    EmptyNamespace,
    #[error("key must not be empty")]
    EmptyKey,
    #[error("storage io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("storage data error: {0}")]
    Data(String),
    #[error("secret store unavailable: {0}")]
    SecretUnavailable(String),
    #[error("secret store error: {0}")]
    Secret(String),
}

impl PluginStorageError {
    fn data_error(context: impl AsRef<str>, error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Data(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Locator of a plugin storage record.
pub struct StorageLocator {
    pub scope: HostStorageScope,
    pub visibility: HostStorageVisibility,
    pub plugin_id: PluginKey,
    pub session_id: Option<i64>,
    pub workspace_root: Option<String>,
}

impl StorageLocator {
    pub fn new(
        scope: HostStorageScope,
        visibility: HostStorageVisibility,
        plugin_id: PluginKey,
        session_id: Option<i64>,
        workspace_root: Option<String>,
    ) -> Result<Self, PluginStorageError> {
        match scope {
            HostStorageScope::Session => {
                if session_id.is_none() {
                    return Err(PluginStorageError::MissingSessionId);
                }
            }
            HostStorageScope::Workspace => {
                if workspace_root
                    .as_deref()
                    .is_none_or(|value| value.trim().is_empty())
                {
                    return Err(PluginStorageError::MissingWorkspaceRoot);
                }
            }
            HostStorageScope::Global => {}
        }
        Ok(Self {
            scope,
            visibility,
            plugin_id,
            session_id,
            workspace_root,
        })
    }
}

#[derive(Debug, Clone, Default)]
/// A stored plugin record.
pub struct StoredRecord {
    pub namespace: String,
    pub key: String,
}

/// Storage available to plugins.
pub trait PluginStorage: Send + Sync {
    fn get(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, PluginStorageError>;
    fn set(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<(), PluginStorageError>;
    fn delete(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
    ) -> Result<(), PluginStorageError>;
    fn list(
        &self,
        locator: &StorageLocator,
        namespace: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Vec<StoredRecord>, PluginStorageError>;
}

/// Secret storage available to plugins.
pub trait PluginSecretStore: Send + Sync {
    fn get(&self, plugin_id: &PluginKey, name: &str) -> Result<Option<String>, PluginStorageError>;
    fn set(&self, plugin_id: &PluginKey, name: &str, value: &str)
    -> Result<(), PluginStorageError>;
    fn delete(&self, plugin_id: &PluginKey, name: &str) -> Result<(), PluginStorageError>;
    fn list(&self, plugin_id: &PluginKey) -> Result<Vec<String>, PluginStorageError>;
}

/// Default keyring service for plugin secrets. Kept distinct from the agena
/// provider auth service so the two surfaces don't collide on the same key.
pub const PLUGIN_SECRETS_KEYRING_SERVICE: &str = "agena.plugin";

/// Default plugin storage root: `$HOME/agena/plugin-storage` unless
/// `AGENA_PLUGIN_STORAGE_DIR` overrides it.
pub fn default_storage_root() -> PathBuf {
    if let Ok(path) = std::env::var("AGENA_PLUGIN_STORAGE_DIR") {
        return PathBuf::from(path);
    }
    let mut base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|error| {
            tracing::error!(
                diagnostic = %error,
                "plugin storage home is unavailable; using the current-directory compatibility path"
            );
            PathBuf::from(".")
        });
    base.push("agena");
    base.push("plugin-storage");
    base
}

fn validate_namespace(namespace: &str) -> Result<(), PluginStorageError> {
    if namespace.trim().is_empty() {
        Err(PluginStorageError::EmptyNamespace)
    } else if namespace.contains('/') || namespace.contains('\\') {
        Err(PluginStorageError::Data(
            "namespace must not contain path separators".into(),
        ))
    } else {
        Ok(())
    }
}

fn validate_key(key: &str) -> Result<(), PluginStorageError> {
    if key.is_empty() {
        Err(PluginStorageError::EmptyKey)
    } else {
        Ok(())
    }
}

fn ensure_dir(path: &Path) -> Result<(), PluginStorageError> {
    if !path.exists() {
        fs::create_dir_all(path)?;
        secure_directory(path)?;
    }
    Ok(())
}

fn secure_directory(path: &Path) -> Result<(), PluginStorageError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    let _ = path;
    Ok(())
}

fn write_secure_file_unlocked(path: &Path, contents: &[u8]) -> Result<(), PluginStorageError> {
    let parent = path.parent().ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("plugin storage path has no parent: {}", path.display()),
        )
    })?;
    ensure_dir(parent)?;
    match fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => {
            agena_runtime_tools::atomic_replace_file(path, contents)?;
        }
        Ok(_) => {
            return Err(PluginStorageError::Data(format!(
                "plugin storage target is not a regular file: {}",
                path.display()
            )));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt as _;
                Some(fs::Permissions::from_mode(0o600))
            };
            #[cfg(not(unix))]
            let permissions = None;
            agena_runtime_tools::atomic_create_file(path, contents, permissions)?;
        }
        Err(error) => return Err(error.into()),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn read_namespace_map(path: &Path) -> Result<BTreeMap<String, String>, PluginStorageError> {
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(BTreeMap::new());
    }
    serde_json::from_str(&text).map_err(|error| {
        PluginStorageError::data_error("failed to decode the plugin storage namespace", &error)
    })
}

fn read_index(path: &Path) -> Result<BTreeSet<String>, PluginStorageError> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(BTreeSet::new());
    }
    serde_json::from_str(&text).map_err(|error| {
        PluginStorageError::data_error("failed to decode the plugin storage index", &error)
    })
}

fn plugin_dir(root: &Path, plugin_id: &PluginKey) -> PathBuf {
    let mut p = root.to_path_buf();
    p.push(plugin_id.to_string());
    p
}

fn storage_scope_dir(root: &Path, locator: &StorageLocator) -> Result<PathBuf, PluginStorageError> {
    let mut p = root.to_path_buf();
    match locator.scope {
        HostStorageScope::Session => {
            let session_id = locator
                .session_id
                .ok_or(PluginStorageError::MissingSessionId)?;
            p.push("session");
            p.push(session_id.to_string());
        }
        HostStorageScope::Workspace => {
            let workspace_root = locator
                .workspace_root
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or(PluginStorageError::MissingWorkspaceRoot)?;
            p.push("workspace");
            p.push(hex::encode(
                blake3::hash(workspace_root.as_bytes()).as_bytes(),
            ));
        }
        HostStorageScope::Global => {
            p.push("global");
        }
    }
    match locator.visibility {
        HostStorageVisibility::Private => {
            p.push("private");
            p.push(locator.plugin_id.to_string());
        }
        HostStorageVisibility::Shared => {
            p.push("shared");
        }
    }
    Ok(p)
}

fn namespace_path(
    root: &Path,
    locator: &StorageLocator,
    namespace: &str,
) -> Result<PathBuf, PluginStorageError> {
    let mut p = storage_scope_dir(root, locator)?;
    p.push(format!("{namespace}.json"));
    Ok(p)
}

fn secrets_index_path(root: &Path, plugin_id: &PluginKey) -> PathBuf {
    let mut p = plugin_dir(root, plugin_id);
    p.push("secrets.json");
    p
}

fn secrets_fallback_path(root: &Path, plugin_id: &PluginKey) -> PathBuf {
    let mut p = plugin_dir(root, plugin_id);
    p.push("secrets-store.json");
    p
}

/// JSON-on-disk plugin storage. Each storage bucket lives under:
///
/// - `global/private/<plugin_id>/`
/// - `global/shared/`
/// - `workspace/<hash>/private/<plugin_id>/`
/// - `workspace/<hash>/shared/`
/// - `session/<session_id>/private/<plugin_id>/`
/// - `session/<session_id>/shared/`
#[derive(Debug, Clone)]
pub struct FilePluginStorage {
    root: PathBuf,
}

impl FilePluginStorage {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl PluginStorage for FilePluginStorage {
    fn get(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, PluginStorageError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, locator, namespace)?;
        let map = read_namespace_map(&path)?;
        Ok(map.get(key).cloned())
    }

    fn set(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<(), PluginStorageError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, locator, namespace)?;
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            let mut map = read_namespace_map(&path)?;
            map.insert(key.to_string(), value.to_string());
            let json = serde_json::to_string_pretty(&map).map_err(|error| {
                PluginStorageError::data_error(
                    "failed to encode the plugin storage namespace",
                    &error,
                )
            })?;
            write_secure_file_unlocked(&path, json.as_bytes())
        })?
    }

    fn delete(
        &self,
        locator: &StorageLocator,
        namespace: &str,
        key: &str,
    ) -> Result<(), PluginStorageError> {
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, locator, namespace)?;
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            if !path.exists() {
                return Ok(());
            }
            let mut map = read_namespace_map(&path)?;
            if map.remove(key).is_none() {
                return Ok(());
            }
            if map.is_empty() {
                fs::remove_file(&path)?;
            } else {
                let json = serde_json::to_string_pretty(&map).map_err(|error| {
                    PluginStorageError::data_error(
                        "failed to encode the plugin storage namespace after deletion",
                        &error,
                    )
                })?;
                write_secure_file_unlocked(&path, json.as_bytes())?;
            }
            Ok(())
        })?
    }

    fn list(
        &self,
        locator: &StorageLocator,
        namespace: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Vec<StoredRecord>, PluginStorageError> {
        let dir = storage_scope_dir(&self.root, locator)?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<StoredRecord> = Vec::new();
        for ns_entry in fs::read_dir(&dir)? {
            let ns_entry = ns_entry?;
            let path = ns_entry.path();
            let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            // skip secret index/fallback files
            if stem == "secrets" || stem == "secrets-store" {
                continue;
            }
            if let Some(filter) = namespace
                && filter != stem
            {
                continue;
            }
            let map = read_namespace_map(&path)?;
            for key in map.keys() {
                if let Some(prefix) = prefix
                    && !key.starts_with(prefix)
                {
                    continue;
                }
                entries.push(StoredRecord {
                    namespace: stem.to_string(),
                    key: key.clone(),
                });
            }
        }
        entries.sort_by(|a, b| {
            a.namespace
                .cmp(&b.namespace)
                .then_with(|| a.key.cmp(&b.key))
        });
        Ok(entries)
    }
}

/// Plugin secret store backed by an OS keyring with optional file fallback.
///
/// Keyring keys are namespaced as `plugin/{plugin_id}/{name}` under the
/// `agena.plugin` service. The on-disk index lists secret names for `list()`,
/// and the optional fallback file holds raw values when the keyring is
/// unavailable.
#[derive(Clone)]
pub struct PluginKeyringSecretStore {
    inner: Arc<dyn SecretStore>,
    root: PathBuf,
    fallback_to_file: bool,
}

impl std::fmt::Debug for PluginKeyringSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginKeyringSecretStore")
            .field("root", &self.root)
            .field("fallback_to_file", &self.fallback_to_file)
            .finish()
    }
}

impl PluginKeyringSecretStore {
    pub fn new(
        inner: Arc<dyn SecretStore>,
        root: impl Into<PathBuf>,
        fallback_to_file: bool,
    ) -> Self {
        Self {
            inner,
            root: root.into(),
            fallback_to_file,
        }
    }

    pub fn system(root: impl Into<PathBuf>, fallback_to_file: bool) -> Self {
        Self::new(
            Arc::new(KeyringSecretStore::new(PLUGIN_SECRETS_KEYRING_SERVICE)),
            root,
            fallback_to_file,
        )
    }

    fn keyring_key(plugin_id: &PluginKey, name: &str) -> String {
        format!("plugin/{}/{name}", plugin_id)
    }

    fn read_fallback(
        &self,
        plugin_id: &PluginKey,
    ) -> Result<BTreeMap<String, String>, PluginStorageError> {
        let path = secrets_fallback_path(&self.root, plugin_id);
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = fs::read_to_string(&path)?;
        if text.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_str(&text).map_err(|error| {
            PluginStorageError::data_error(
                "failed to decode the plugin secret fallback file",
                &error,
            )
        })
    }

    fn update_fallback(
        &self,
        plugin_id: &PluginKey,
        update: impl FnOnce(&mut BTreeMap<String, String>),
    ) -> Result<(), PluginStorageError> {
        let path = secrets_fallback_path(&self.root, plugin_id);
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            let mut map = self.read_fallback(plugin_id)?;
            update(&mut map);
            if map.is_empty() {
                if path.exists() {
                    fs::remove_file(&path)?;
                }
                return Ok(());
            }
            let json = serde_json::to_string_pretty(&map).map_err(|error| {
                PluginStorageError::data_error(
                    "failed to encode the plugin secret fallback file",
                    &error,
                )
            })?;
            write_secure_file_unlocked(&path, json.as_bytes())
        })?
    }

    fn record_name(&self, plugin_id: &PluginKey, name: &str) -> Result<(), PluginStorageError> {
        let path = secrets_index_path(&self.root, plugin_id);
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            let mut names = read_index(&path)?;
            if names.insert(name.to_string()) {
                let json = serde_json::to_string_pretty(&names).map_err(|error| {
                    PluginStorageError::data_error(
                        "failed to encode the plugin secret index",
                        &error,
                    )
                })?;
                write_secure_file_unlocked(&path, json.as_bytes())?;
            }
            Ok(())
        })?
    }

    fn forget_name(&self, plugin_id: &PluginKey, name: &str) -> Result<(), PluginStorageError> {
        let path = secrets_index_path(&self.root, plugin_id);
        agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
            let mut names = read_index(&path)?;
            if names.remove(name) {
                if names.is_empty() {
                    fs::remove_file(&path)?;
                } else {
                    let json = serde_json::to_string_pretty(&names).map_err(|error| {
                        PluginStorageError::data_error(
                            "failed to encode the plugin secret index after deletion",
                            &error,
                        )
                    })?;
                    write_secure_file_unlocked(&path, json.as_bytes())?;
                }
            }
            Ok(())
        })?
    }

    fn map_secret_err(&self, err: SecretStoreError) -> PluginStorageError {
        if err.is_unavailable() {
            PluginStorageError::SecretUnavailable(
                agena_failure::diagnostic::format_error_chain_with_context(
                    "plugin secret store is unavailable",
                    &err,
                ),
            )
        } else {
            PluginStorageError::Secret(agena_failure::diagnostic::format_error_chain_with_context(
                "plugin secret store operation failed",
                &err,
            ))
        }
    }
}

impl PluginSecretStore for PluginKeyringSecretStore {
    fn get(&self, plugin_id: &PluginKey, name: &str) -> Result<Option<String>, PluginStorageError> {
        validate_key(name)?;
        let key = Self::keyring_key(plugin_id, name);
        match self.inner.get_secret(&key) {
            Ok(value) => Ok(value),
            Err(err) if err.is_unavailable() => {
                if self.fallback_to_file {
                    Ok(self.read_fallback(plugin_id)?.get(name).cloned())
                } else {
                    Err(self.map_secret_err(err))
                }
            }
            Err(err) => Err(self.map_secret_err(err)),
        }
    }

    fn set(
        &self,
        plugin_id: &PluginKey,
        name: &str,
        value: &str,
    ) -> Result<(), PluginStorageError> {
        validate_key(name)?;
        let key = Self::keyring_key(plugin_id, name);
        match self.inner.set_secret(&key, value) {
            Ok(()) => {
                self.record_name(plugin_id, name)?;
                Ok(())
            }
            Err(err) if err.is_unavailable() => {
                if self.fallback_to_file {
                    self.update_fallback(plugin_id, |map| {
                        map.insert(name.to_string(), value.to_string());
                    })?;
                    self.record_name(plugin_id, name)?;
                    Ok(())
                } else {
                    Err(self.map_secret_err(err))
                }
            }
            Err(err) => Err(self.map_secret_err(err)),
        }
    }

    fn delete(&self, plugin_id: &PluginKey, name: &str) -> Result<(), PluginStorageError> {
        validate_key(name)?;
        let key = Self::keyring_key(plugin_id, name);
        match self.inner.delete_secret(&key) {
            Ok(()) => {}
            Err(err) if err.is_unavailable() => {
                if !self.fallback_to_file {
                    return Err(self.map_secret_err(err));
                }
            }
            Err(err) => return Err(self.map_secret_err(err)),
        }
        if self.fallback_to_file {
            self.update_fallback(plugin_id, |map| {
                map.remove(name);
            })?;
        }
        self.forget_name(plugin_id, name)?;
        Ok(())
    }

    fn list(&self, plugin_id: &PluginKey) -> Result<Vec<String>, PluginStorageError> {
        let path = secrets_index_path(&self.root, plugin_id);
        Ok(read_index(&path)?.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use agena_plugin_host::sdk::host_api::{HostStorageScope, HostStorageVisibility};
    use agena_plugin_sdk::PluginKey;

    use super::{FilePluginStorage, PluginStorage, StorageLocator};

    #[test]
    fn concurrent_namespace_updates_keep_both_records() {
        let directory = tempfile::tempdir().expect("plugin storage directory");
        let storage = Arc::new(FilePluginStorage::new(directory.path()));
        let locator = StorageLocator::new(
            HostStorageScope::Global,
            HostStorageVisibility::Private,
            PluginKey::new("test", "storage").expect("plugin key"),
            None,
            None,
        )
        .expect("storage locator");
        let barrier = Arc::new(Barrier::new(2));

        let handles = [("first", "one"), ("second", "two")].map(|(key, value)| {
            let storage = Arc::clone(&storage);
            let locator = locator.clone();
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                storage
                    .set(&locator, "state", key, value)
                    .expect("concurrent set");
            })
        });
        for handle in handles {
            handle.join().expect("storage writer");
        }

        assert_eq!(
            storage
                .get(&locator, "state", "first")
                .expect("first record")
                .as_deref(),
            Some("one")
        );
        assert_eq!(
            storage
                .get(&locator, "state", "second")
                .expect("second record")
                .as_deref(),
            Some("two")
        );
    }
}
