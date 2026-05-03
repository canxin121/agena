//! Plugin substrate storage and secret backends.
//!
//! These are the host-side implementations that back the plugin SDK's
//! `storage_*` and `secret_*` `HostClient` methods. Both surfaces are scoped
//! per plugin id so that one plugin cannot read or overwrite another's data.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use agena_keyring_store::{KeyringSecretStore, SecretStore, SecretStoreError};

#[derive(Debug, thiserror::Error)]
pub enum PluginStorageError {
    #[error("plugin id is required")]
    MissingPluginId,
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

#[derive(Debug, Clone, Default)]
pub struct StoredEntry {
    pub plugin_id: String,
    pub namespace: String,
    pub key: String,
}

pub trait PluginStorage: Send + Sync {
    fn get(
        &self,
        plugin_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, PluginStorageError>;
    fn set(
        &self,
        plugin_id: &str,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<(), PluginStorageError>;
    fn delete(&self, plugin_id: &str, namespace: &str, key: &str)
    -> Result<(), PluginStorageError>;
    fn list(
        &self,
        plugin_id: &str,
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

fn namespace_path(root: &Path, plugin_id: &str, namespace: &str) -> PathBuf {
    let mut p = plugin_dir(root, plugin_id);
    p.push(format!("{namespace}.json"));
    p
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

/// JSON-on-disk plugin storage. Each `plugin_id`/`namespace` pair lives in its
/// own file under `root`.
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
        plugin_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<Option<String>, PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, plugin_id, namespace);
        let map = read_namespace_map(&path)?;
        Ok(map.get(key).cloned())
    }

    fn set(
        &self,
        plugin_id: &str,
        namespace: &str,
        key: &str,
        value: &str,
    ) -> Result<(), PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, plugin_id, namespace);
        let mut map = read_namespace_map(&path)?;
        map.insert(key.to_string(), value.to_string());
        write_namespace_map(&path, &map)
    }

    fn delete(
        &self,
        plugin_id: &str,
        namespace: &str,
        key: &str,
    ) -> Result<(), PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        validate_namespace(namespace)?;
        validate_key(key)?;
        let path = namespace_path(&self.root, plugin_id, namespace);
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
        plugin_id: &str,
        namespace: Option<&str>,
        prefix: Option<&str>,
    ) -> Result<Vec<StoredEntry>, PluginStorageError> {
        validate_plugin_id(plugin_id)?;
        let dir = plugin_dir(&self.root, plugin_id);
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
                    plugin_id: plugin_id.to_string(),
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
        Ok(self.secrets.read().expect("poisoned").get(key).cloned())
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.secrets
            .write()
            .expect("poisoned")
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
        self.secrets.write().expect("poisoned").remove(key);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn file_storage_roundtrips_per_namespace() {
        let root = temp_root();
        let store = FilePluginStorage::new(&root);

        store.set("alpha", "settings", "lang", "rust").unwrap();
        store.set("alpha", "cache", "k1", "v1").unwrap();
        store.set("beta", "settings", "lang", "go").unwrap();

        assert_eq!(
            store.get("alpha", "settings", "lang").unwrap().as_deref(),
            Some("rust")
        );
        assert_eq!(
            store.get("beta", "settings", "lang").unwrap().as_deref(),
            Some("go")
        );
        let listed = store.list("alpha", None, None).unwrap();
        assert_eq!(listed.len(), 2);
        let listed_settings = store.list("alpha", Some("settings"), None).unwrap();
        assert_eq!(listed_settings.len(), 1);
        assert_eq!(listed_settings[0].key, "lang");

        store.delete("alpha", "cache", "k1").unwrap();
        assert!(store.get("alpha", "cache", "k1").unwrap().is_none());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn file_storage_rejects_bad_inputs() {
        let store = FilePluginStorage::new(temp_root());
        assert!(matches!(
            store.set("", "ns", "k", "v"),
            Err(PluginStorageError::MissingPluginId)
        ));
        assert!(matches!(
            store.set("p", "", "k", "v"),
            Err(PluginStorageError::EmptyNamespace)
        ));
        assert!(matches!(
            store.set("p", "ns", "", "v"),
            Err(PluginStorageError::EmptyKey)
        ));
        assert!(matches!(
            store.set("p", "bad/ns", "k", "v"),
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
