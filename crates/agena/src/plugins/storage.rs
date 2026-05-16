//! Plugin substrate storage and secret backends.
//!
//! These are the host-side implementations that back the plugin SDK's
//! `storage_*` and `secret_*` `HostClient` methods. Both surfaces are scoped
//! for plugins while keeping private/plugin-owned data separate from
//! intentionally shared data.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use agena_keyring_store::{KeyringSecretStore, SecretStore, SecretStoreError};

use crate::plugin::sdk::host_api::{HostStorageScope, HostStorageVisibility};

#[derive(Debug, thiserror::Error)]
pub enum PluginStorageError {
    #[error("plugin id is required")]
    MissingPluginId,
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageLocator {
    pub scope: HostStorageScope,
    pub visibility: HostStorageVisibility,
    pub plugin_id: String,
    pub session_id: Option<i64>,
    pub workspace_root: Option<String>,
}

impl StorageLocator {
    pub fn new(
        scope: HostStorageScope,
        visibility: HostStorageVisibility,
        plugin_id: impl Into<String>,
        session_id: Option<i64>,
        workspace_root: Option<String>,
    ) -> Result<Self, PluginStorageError> {
        let plugin_id = plugin_id.into();
        validate_plugin_id(plugin_id.as_str())?;
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
pub struct StoredEntry {
    pub namespace: String,
    pub key: String,
}

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
    ) -> Result<Vec<StoredEntry>, PluginStorageError>;
}

pub trait PluginSecretStore: Send + Sync {
    fn get(&self, plugin_id: &str, name: &str) -> Result<Option<String>, PluginStorageError>;
    fn set(&self, plugin_id: &str, name: &str, value: &str) -> Result<(), PluginStorageError>;
    fn delete(&self, plugin_id: &str, name: &str) -> Result<(), PluginStorageError>;
    fn list(&self, plugin_id: &str) -> Result<Vec<String>, PluginStorageError>;
}

/// Default keyring service for plugin secrets. Kept distinct from the agena
/// provider auth service so the two surfaces don't collide on the same key.
pub const PLUGIN_SECRETS_KEYRING_SERVICE: &str = "agena.plugin";

/// Default plugin storage root: `$HOME/.agena/plugin-storage` unless
/// `AGENA_PLUGIN_STORAGE_DIR` overrides it.
pub fn default_storage_root() -> PathBuf {
    if let Ok(path) = std::env::var("AGENA_PLUGIN_STORAGE_DIR") {
        return PathBuf::from(path);
    }
    let mut base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    base.push(".agena");
    base.push("plugin-storage");
    base
}

fn validate_plugin_id(plugin_id: &str) -> Result<(), PluginStorageError> {
    if plugin_id.trim().is_empty() {
        Err(PluginStorageError::MissingPluginId)
    } else {
        Ok(())
    }
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

fn write_secure_file(path: &Path, contents: &[u8]) -> Result<(), PluginStorageError> {
    if let Some(parent) = path.parent() {
        ensure_dir(parent)?;
    }
    fs::write(path, contents)?;
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
    serde_json::from_str(&text).map_err(|e| PluginStorageError::Data(e.to_string()))
}

fn write_namespace_map(
    path: &Path,
    map: &BTreeMap<String, String>,
) -> Result<(), PluginStorageError> {
    let json =
        serde_json::to_string_pretty(map).map_err(|e| PluginStorageError::Data(e.to_string()))?;
    write_secure_file(path, json.as_bytes())
}

fn read_index(path: &Path) -> Result<BTreeSet<String>, PluginStorageError> {
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(path)?;
    if text.trim().is_empty() {
        return Ok(BTreeSet::new());
    }
    serde_json::from_str(&text).map_err(|e| PluginStorageError::Data(e.to_string()))
}

fn write_index(path: &Path, names: &BTreeSet<String>) -> Result<(), PluginStorageError> {
    let json =
        serde_json::to_string_pretty(names).map_err(|e| PluginStorageError::Data(e.to_string()))?;
    write_secure_file(path, json.as_bytes())
}

fn plugin_dir(root: &Path, plugin_id: &str) -> PathBuf {
    let mut p = root.to_path_buf();
    p.push(plugin_id);
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
            p.push(locator.plugin_id.as_str());
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

fn secrets_index_path(root: &Path, plugin_id: &str) -> PathBuf {
    let mut p = plugin_dir(root, plugin_id);
    p.push("secrets.json");
    p
}

fn secrets_fallback_path(root: &Path, plugin_id: &str) -> PathBuf {
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

    pub fn root(&self) -> &Path {
        &self.root
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
        let mut map = read_namespace_map(&path)?;
        map.insert(key.to_string(), value.to_string());
        write_namespace_map(&path, &map)
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
            write_namespace_map(&path, &map)?;
        }
        Ok(())
    }

    fn list(
        &self,
        locator: &StorageLocator,
        namespace: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Vec<StoredEntry>, PluginStorageError> {
        let dir = storage_scope_dir(&self.root, locator)?;
        if !dir.exists() {
            return Ok(Vec::new());
        }
        let mut entries: Vec<StoredEntry> = Vec::new();
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
                entries.push(StoredEntry {
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

    fn keyring_key(plugin_id: &str, name: &str) -> String {
        format!("plugin/{plugin_id}/{name}")
    }

    fn read_fallback(
        &self,
        plugin_id: &str,
    ) -> Result<BTreeMap<String, String>, PluginStorageError> {
        let path = secrets_fallback_path(&self.root, plugin_id);
        if !path.exists() {
            return Ok(BTreeMap::new());
        }
        let text = fs::read_to_string(&path)?;
        if text.trim().is_empty() {
            return Ok(BTreeMap::new());
        }
        serde_json::from_str(&text).map_err(|e| PluginStorageError::Data(e.to_string()))
    }

    fn write_fallback(
        &self,
        plugin_id: &str,
        map: &BTreeMap<String, String>,
    ) -> Result<(), PluginStorageError> {
        let path = secrets_fallback_path(&self.root, plugin_id);
        if map.is_empty() {
            if path.exists() {
                fs::remove_file(&path)?;
            }
            return Ok(());
        }
        let json = serde_json::to_string_pretty(map)
            .map_err(|e| PluginStorageError::Data(e.to_string()))?;
        write_secure_file(&path, json.as_bytes())
    }

    fn record_name(&self, plugin_id: &str, name: &str) -> Result<(), PluginStorageError> {
        let path = secrets_index_path(&self.root, plugin_id);
        let mut names = read_index(&path)?;
        if names.insert(name.to_string()) {
            write_index(&path, &names)?;
        }
        Ok(())
    }

    fn forget_name(&self, plugin_id: &str, name: &str) -> Result<(), PluginStorageError> {
        let path = secrets_index_path(&self.root, plugin_id);
        let mut names = read_index(&path)?;
        if names.remove(name) {
            if names.is_empty() {
                fs::remove_file(&path)?;
            } else {
                write_index(&path, &names)?;
            }
        }
        Ok(())
    }

    fn map_secret_err(&self, err: SecretStoreError) -> PluginStorageError {
        if err.is_unavailable() {
            PluginStorageError::SecretUnavailable(err.to_string())
        } else {
            PluginStorageError::Secret(err.to_string())
        }
    }
}

impl PluginSecretStore for PluginKeyringSecretStore {
    fn get(&self, plugin_id: &str, name: &str) -> Result<Option<String>, PluginStorageError> {
        validate_plugin_id(plugin_id)?;
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

    fn set(&self, plugin_id: &str, name: &str, value: &str) -> Result<(), PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        validate_key(name)?;
        let key = Self::keyring_key(plugin_id, name);
        match self.inner.set_secret(&key, value) {
            Ok(()) => {
                self.record_name(plugin_id, name)?;
                Ok(())
            }
            Err(err) if err.is_unavailable() => {
                if self.fallback_to_file {
                    let mut map = self.read_fallback(plugin_id)?;
                    map.insert(name.to_string(), value.to_string());
                    self.write_fallback(plugin_id, &map)?;
                    self.record_name(plugin_id, name)?;
                    Ok(())
                } else {
                    Err(self.map_secret_err(err))
                }
            }
            Err(err) => Err(self.map_secret_err(err)),
        }
    }

    fn delete(&self, plugin_id: &str, name: &str) -> Result<(), PluginStorageError> {
        validate_plugin_id(plugin_id)?;
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
            let mut map = self.read_fallback(plugin_id)?;
            map.remove(name);
            self.write_fallback(plugin_id, &map)?;
        }
        self.forget_name(plugin_id, name)?;
        Ok(())
    }

    fn list(&self, plugin_id: &str) -> Result<Vec<String>, PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        let path = secrets_index_path(&self.root, plugin_id);
        Ok(read_index(&path)?.into_iter().collect())
    }
}

/// Convenience snapshot helper used by test scenarios — keeps an in-memory
/// secret store with the same shape as the keyring.
#[derive(Default)]
pub struct InMemorySecretStore {
    secrets: RwLock<BTreeMap<String, String>>,
}

impl SecretStore for InMemorySecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        let guard = self.secrets.read().unwrap_or_else(|e| e.into_inner());
        Ok(guard.get(key).cloned())
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        let mut guard = self.secrets.write().unwrap_or_else(|e| e.into_inner());
        guard.insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
        let mut guard = self.secrets.write().unwrap_or_else(|e| e.into_inner());
        guard.remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn private_global(plugin_id: &str) -> StorageLocator {
        StorageLocator::new(
            HostStorageScope::Global,
            HostStorageVisibility::Private,
            plugin_id,
            None,
            None,
        )
        .expect("global private locator")
    }

    fn shared_global(plugin_id: &str) -> StorageLocator {
        StorageLocator::new(
            HostStorageScope::Global,
            HostStorageVisibility::Shared,
            plugin_id,
            None,
            None,
        )
        .expect("global shared locator")
    }

    fn shared_workspace(plugin_id: &str, workspace_root: &str) -> StorageLocator {
        StorageLocator::new(
            HostStorageScope::Workspace,
            HostStorageVisibility::Shared,
            plugin_id,
            None,
            Some(workspace_root.to_string()),
        )
        .expect("workspace shared locator")
    }

    fn shared_session(plugin_id: &str, session_id: i64) -> StorageLocator {
        StorageLocator::new(
            HostStorageScope::Session,
            HostStorageVisibility::Shared,
            plugin_id,
            Some(session_id),
            None,
        )
        .expect("session shared locator")
    }

    fn temp_root() -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "agena-plugin-storage-{}-{}",
            std::process::id(),
            uuid::Uuid::new_v4().simple()
        ));
        path
    }

    #[test]
    fn file_storage_scopes_private_and_shared_entries() {
        let root = temp_root();
        let store = FilePluginStorage::new(&root);
        let alpha_private = private_global("alpha");
        let beta_private = private_global("beta");
        let alpha_shared = shared_global("alpha");
        let beta_shared = shared_global("beta");
        let ws_one = shared_workspace("alpha", "/repo/one");
        let ws_two = shared_workspace("beta", "/repo/two");
        let session_one = shared_session("alpha", 7);
        let session_two = shared_session("beta", 8);

        store
            .set(&alpha_private, "settings", "lang", "rust")
            .unwrap();
        store.set(&alpha_shared, "registry", "flag", "on").unwrap();
        store
            .set(&ws_one, "index", "symbols", "workspace-one")
            .unwrap();
        store
            .set(&ws_two, "index", "symbols", "workspace-two")
            .unwrap();
        store
            .set(&session_one, "drafts", "step", "session-one")
            .unwrap();
        store
            .set(&session_two, "drafts", "step", "session-two")
            .unwrap();

        assert_eq!(
            store
                .get(&alpha_private, "settings", "lang")
                .unwrap()
                .as_deref(),
            Some("rust")
        );
        assert_eq!(
            store
                .get(&beta_private, "settings", "lang")
                .unwrap()
                .as_deref(),
            None
        );
        assert_eq!(
            store
                .get(&beta_shared, "registry", "flag")
                .unwrap()
                .as_deref(),
            Some("on"),
            "shared global storage should be readable across plugins"
        );
        assert_eq!(
            store.get(&ws_one, "index", "symbols").unwrap().as_deref(),
            Some("workspace-one")
        );
        assert_eq!(
            store.get(&ws_two, "index", "symbols").unwrap().as_deref(),
            Some("workspace-two")
        );
        assert_eq!(
            store
                .get(&session_one, "drafts", "step")
                .unwrap()
                .as_deref(),
            Some("session-one")
        );
        assert_eq!(
            store
                .get(&session_two, "drafts", "step")
                .unwrap()
                .as_deref(),
            Some("session-two")
        );

        let listed_private = store.list(&alpha_private, None, None).unwrap();
        assert_eq!(listed_private.len(), 1);
        assert_eq!(listed_private[0].namespace, "settings");
        let listed_shared = store.list(&beta_shared, Some("registry"), None).unwrap();
        assert_eq!(listed_shared.len(), 1);
        assert_eq!(listed_shared[0].key, "flag");

        store.delete(&alpha_shared, "registry", "flag").unwrap();
        assert!(
            store
                .get(&alpha_shared, "registry", "flag")
                .unwrap()
                .is_none()
        );
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_storage_rejects_bad_inputs() {
        let store = FilePluginStorage::new(temp_root());
        let valid = private_global("p");
        let missing_plugin = StorageLocator::new(
            HostStorageScope::Global,
            HostStorageVisibility::Private,
            "",
            None,
            None,
        );
        assert!(matches!(
            missing_plugin,
            Err(PluginStorageError::MissingPluginId)
        ));
        assert!(matches!(
            StorageLocator::new(
                HostStorageScope::Session,
                HostStorageVisibility::Shared,
                "p",
                None,
                None
            ),
            Err(PluginStorageError::MissingSessionId)
        ));
        assert!(matches!(
            StorageLocator::new(
                HostStorageScope::Workspace,
                HostStorageVisibility::Shared,
                "p",
                None,
                None
            ),
            Err(PluginStorageError::MissingWorkspaceRoot)
        ));
        assert!(matches!(
            store.set(&valid, "", "k", "v"),
            Err(PluginStorageError::EmptyNamespace)
        ));
        assert!(matches!(
            store.set(&valid, "ns", "", "v"),
            Err(PluginStorageError::EmptyKey)
        ));
        assert!(matches!(
            store.set(&valid, "bad/ns", "k", "v"),
            Err(PluginStorageError::Data(_))
        ));
    }

    #[test]
    fn keyring_secret_store_roundtrips_with_inmemory_backend() {
        let root = temp_root();
        let inner: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::default());
        let store = PluginKeyringSecretStore::new(inner, &root, false);

        store.set("alpha", "token", "abc123").unwrap();
        assert_eq!(
            store.get("alpha", "token").unwrap().as_deref(),
            Some("abc123")
        );
        let names = store.list("alpha").unwrap();
        assert_eq!(names, vec!["token".to_string()]);

        store.delete("alpha", "token").unwrap();
        assert!(store.get("alpha", "token").unwrap().is_none());
        assert!(store.list("alpha").unwrap().is_empty());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keyring_secret_store_falls_back_to_file_when_unavailable() {
        struct AlwaysUnavailable;
        impl SecretStore for AlwaysUnavailable {
            fn get_secret(&self, _: &str) -> Result<Option<String>, SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
            fn set_secret(&self, _: &str, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
            fn delete_secret(&self, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
        }

        let root = temp_root();
        let store = PluginKeyringSecretStore::new(Arc::new(AlwaysUnavailable), &root, true);
        store.set("alpha", "token", "fallback").unwrap();
        assert_eq!(
            store.get("alpha", "token").unwrap().as_deref(),
            Some("fallback")
        );
        let names = store.list("alpha").unwrap();
        assert_eq!(names, vec!["token".to_string()]);
        store.delete("alpha", "token").unwrap();
        assert!(store.get("alpha", "token").unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn keyring_secret_store_propagates_unavailable_without_fallback() {
        struct AlwaysUnavailable;
        impl SecretStore for AlwaysUnavailable {
            fn get_secret(&self, _: &str) -> Result<Option<String>, SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
            fn set_secret(&self, _: &str, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
            fn delete_secret(&self, _: &str) -> Result<(), SecretStoreError> {
                Err(SecretStoreError::Unavailable("no keyring".into()))
            }
        }
        let store = PluginKeyringSecretStore::new(Arc::new(AlwaysUnavailable), temp_root(), false);
        let err = store.set("alpha", "token", "x").unwrap_err();
        assert!(matches!(err, PluginStorageError::SecretUnavailable(_)));
    }
}
