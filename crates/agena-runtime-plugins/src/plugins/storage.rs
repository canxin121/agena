//! Plugin substrate storage and secret backends.
//!
//! These are the host-side implementations that back the plugin SDK's
//! `storage_*` and `secret_*` `HostClient` methods. Both surfaces are scoped
//! for plugins while keeping private/plugin-owned data separate from
//! intentionally shared data.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use agena_plugin_sdk::PluginKey;
use agena_secret_store::{FileSecretStore, SecretStore, SecretStoreError};

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

/// A separate namespace keeps plugin credentials isolated from other secrets.
pub const PLUGIN_SECRETS_NAMESPACE: &str = "agena.plugin";

/// Plugin state belongs to the selected Agena instance.
pub fn default_storage_root() -> PathBuf {
    agena_storage_layout::StorageLayout::current().plugin_storage_dir()
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
    agena_storage_layout::secure_directory(path)?;
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
    match fs::symlink_metadata(path) {
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
    agena_storage_layout::secure_file(path)?;
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
            p.push(agena_storage_layout::workspace_key(Path::new(
                workspace_root,
            )));
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

/// Plugin credentials share the application-managed secret store. Names and
/// values are committed together, so listings cannot drift from stored values.
#[derive(Clone)]
pub struct PluginFileSecretStore {
    inner: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for PluginFileSecretStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PluginFileSecretStore")
            .finish_non_exhaustive()
    }
}

impl Default for PluginFileSecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginFileSecretStore {
    pub fn new() -> Self {
        Self::with_secret_store(Arc::new(FileSecretStore::new(PLUGIN_SECRETS_NAMESPACE)))
    }

    pub fn with_secret_store(inner: Arc<dyn SecretStore>) -> Self {
        Self { inner }
    }

    fn prefix(plugin_id: &PluginKey) -> String {
        // Encode the entire plugin identity, including separators. A plugin
        // can never impersonate another plugin by choosing a secret name.
        format!("plugin/{}/", hex::encode(plugin_id.to_string().as_bytes()))
    }

    fn secret_key(plugin_id: &PluginKey, name: &str) -> String {
        format!("{}{name}", Self::prefix(plugin_id))
    }

    fn map_secret_error(error: SecretStoreError) -> PluginStorageError {
        if error.is_unavailable() {
            PluginStorageError::SecretUnavailable(error.to_string())
        } else {
            PluginStorageError::Secret(error.to_string())
        }
    }
}

impl PluginSecretStore for PluginFileSecretStore {
    fn get(&self, plugin_id: &PluginKey, name: &str) -> Result<Option<String>, PluginStorageError> {
        validate_key(name)?;
        self.inner
            .get_secret(&Self::secret_key(plugin_id, name))
            .map_err(Self::map_secret_error)
    }

    fn set(
        &self,
        plugin_id: &PluginKey,
        name: &str,
        value: &str,
    ) -> Result<(), PluginStorageError> {
        validate_key(name)?;
        self.inner
            .set_secret(&Self::secret_key(plugin_id, name), value)
            .map_err(Self::map_secret_error)
    }

    fn delete(&self, plugin_id: &PluginKey, name: &str) -> Result<(), PluginStorageError> {
        validate_key(name)?;
        self.inner
            .delete_secret(&Self::secret_key(plugin_id, name))
            .map_err(Self::map_secret_error)
    }

    fn list(&self, plugin_id: &PluginKey) -> Result<Vec<String>, PluginStorageError> {
        let prefix = Self::prefix(plugin_id);
        Ok(self
            .inner
            .list_secret_keys()
            .map_err(Self::map_secret_error)?
            .into_iter()
            .filter_map(|key| key.strip_prefix(&prefix).map(str::to_owned))
            .collect())
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
    #[test]
    fn plugin_secrets_list_the_same_committed_records_and_isolate_plugins() {
        use super::{PluginFileSecretStore, PluginSecretStore};
        let directory = tempfile::tempdir().unwrap();
        let backend = Arc::new(agena_secret_store::FileSecretStore::at(
            directory.path(),
            "plugin-test",
        ));
        let store = PluginFileSecretStore::with_secret_store(backend);
        let first = PluginKey::new("test", "one").unwrap();
        let second = PluginKey::new("test", "two").unwrap();
        store
            .set(&first, "token/with/separators", "sensitive")
            .unwrap();
        assert_eq!(store.list(&first).unwrap(), vec!["token/with/separators"]);
        assert!(store.list(&second).unwrap().is_empty());
        assert_eq!(store.get(&second, "token/with/separators").unwrap(), None);
        store.delete(&first, "token/with/separators").unwrap();
        assert!(store.list(&first).unwrap().is_empty());
    }
}
