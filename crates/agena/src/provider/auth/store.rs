use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
};

use agena_keyring_store::{KeyringSecretStore, SecretStore, SecretStoreError};
use serde::{Deserialize, Serialize};

use crate::error::AppError;

use super::AuthData;

pub trait AuthStore: Send + Sync {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError>;
    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError>;
    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError>;
    fn remove(&self, provider_id: &str) -> Result<(), AppError>;
}

#[derive(Debug, Clone)]
pub enum ConfiguredAuthStore {
    File(FileAuthStore),
    Keyring(KeyringAuthStore<KeyringSecretStore>),
}

impl AuthStore for ConfiguredAuthStore {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        match self {
            Self::File(store) => store.all(),
            Self::Keyring(store) => store.all(),
        }
    }

    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        match self {
            Self::File(store) => store.get(provider_id),
            Self::Keyring(store) => store.get(provider_id),
        }
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        match self {
            Self::File(store) => store.set(provider_id, auth),
            Self::Keyring(store) => store.set(provider_id, auth),
        }
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        match self {
            Self::File(store) => store.remove(provider_id),
            Self::Keyring(store) => store.remove(provider_id),
        }
    }
}

#[derive(Debug, Clone)]
pub struct FileAuthStore {
    path: PathBuf,
}

impl FileAuthStore {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn default_path() -> PathBuf {
        if let Ok(path) = std::env::var("AGENA_AUTH_FILE") {
            return PathBuf::from(path);
        }

        let mut base = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("."));
        base.push(".agena");
        base.push("auth.json");
        base
    }

    fn read_file(&self) -> Result<AuthFile, AppError> {
        if !self.path.exists() {
            return Ok(AuthFile::default());
        }

        let text = fs::read_to_string(&self.path)?;
        let parsed = serde_json::from_str::<AuthFile>(&text)?;
        Ok(parsed)
    }

    fn write_file(&self, file: &AuthFile) -> Result<(), AppError> {
        if let Some(parent) = self.path.parent() {
            ensure_directory(parent)?;
        }

        let json = serde_json::to_string_pretty(file)?;
        fs::write(&self.path, json.as_bytes())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let permissions = fs::Permissions::from_mode(0o600);
            fs::set_permissions(&self.path, permissions)?;
        }
        Ok(())
    }

    fn set_stored(&self, provider_id: &str, auth: StoredAuthData) -> Result<(), AppError> {
        let mut data = self.read_file()?;
        data.providers
            .insert(normalize_provider_id(provider_id), auth);
        self.write_file(&data)
    }
}

impl AuthStore for FileAuthStore {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        self.read_file()?
            .providers
            .into_iter()
            .map(|(provider_id, stored)| {
                Ok((provider_id.clone(), stored.into_plain(&provider_id)?))
            })
            .collect()
    }

    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        let provider_id = normalize_provider_id(provider_id);
        self.read_file()?
            .providers
            .remove(provider_id.as_str())
            .map(|stored| stored.into_plain(&provider_id))
            .transpose()
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        self.set_stored(provider_id, StoredAuthData::Plain(auth))
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        let mut data = self.read_file()?;
        data.providers
            .remove(normalize_provider_id(provider_id).as_str());
        self.write_file(&data)
    }
}

#[derive(Debug, Clone)]
pub struct KeyringAuthStore<S = KeyringSecretStore> {
    file: FileAuthStore,
    secrets: S,
    fallback_to_file: bool,
}

impl KeyringAuthStore<KeyringSecretStore> {
    pub fn system(file: FileAuthStore, fallback_to_file: bool) -> Self {
        Self::new(file, KeyringSecretStore::default(), fallback_to_file)
    }
}

impl<S: SecretStore> KeyringAuthStore<S> {
    pub fn new(file: FileAuthStore, secrets: S, fallback_to_file: bool) -> Self {
        Self {
            file,
            secrets,
            fallback_to_file,
        }
    }

    fn key_for_provider(provider_id: &str) -> String {
        format!("provider:{}", normalize_provider_id(provider_id))
    }

    fn secret_unavailable(&self, err: &SecretStoreError) -> bool {
        self.fallback_to_file && err.is_unavailable()
    }

    fn read_secret(&self, key: &str) -> Result<Option<AuthData>, AppError> {
        let Some(payload) = self.secrets.get_secret(key).map_err(secret_error)? else {
            return Ok(None);
        };
        Ok(Some(serde_json::from_str(payload.as_str())?))
    }
}

impl<S: SecretStore> AuthStore for KeyringAuthStore<S> {
    fn all(&self) -> Result<HashMap<String, AuthData>, AppError> {
        let provider_ids: Vec<String> = self.file.read_file()?.providers.into_keys().collect();
        let mut auth = HashMap::new();
        for provider_id in provider_ids {
            if let Some(data) = self.get(provider_id.as_str())? {
                auth.insert(provider_id, data);
            }
        }
        Ok(auth)
    }

    fn get(&self, provider_id: &str) -> Result<Option<AuthData>, AppError> {
        let provider_id = normalize_provider_id(provider_id);
        let mut file = self.file.read_file()?;
        let Some(stored) = file.providers.get(provider_id.as_str()).cloned() else {
            return Ok(None);
        };

        match stored {
            StoredAuthData::Plain(auth) => {
                let key = Self::key_for_provider(provider_id.as_str());
                let payload = serde_json::to_string(&auth)?;
                match self.secrets.set_secret(key.as_str(), payload.as_str()) {
                    Ok(()) => {
                        file.providers.insert(
                            provider_id.clone(),
                            StoredAuthData::KeyringRef(KeyringReference::new(key)),
                        );
                        self.file.write_file(&file)?;
                    }
                    Err(err) if self.secret_unavailable(&err) => {
                        tracing::warn!(target: "agena::auth", "keyring unavailable; using file auth store fallback");
                    }
                    Err(err) => return Err(secret_error(err)),
                }
                Ok(Some(auth))
            }
            StoredAuthData::KeyringRef(reference) => match self.read_secret(reference.key()) {
                Ok(auth) => Ok(auth),
                Err(AppError::Config(message)) if self.fallback_to_file => {
                    tracing::warn!(target: "agena::auth", "keyring unavailable; using file auth store fallback: {message}");
                    Ok(None)
                }
                Err(err) => Err(err),
            },
        }
    }

    fn set(&self, provider_id: &str, auth: AuthData) -> Result<(), AppError> {
        let key = Self::key_for_provider(provider_id);
        let payload = serde_json::to_string(&auth)?;
        match self.secrets.set_secret(key.as_str(), payload.as_str()) {
            Ok(()) => self.file.set_stored(
                provider_id,
                StoredAuthData::KeyringRef(KeyringReference::new(key)),
            ),
            Err(err) if self.secret_unavailable(&err) => {
                tracing::warn!(target: "agena::auth", "keyring unavailable; writing credential to file auth store fallback");
                self.file.set(provider_id, auth)
            }
            Err(err) => Err(secret_error(err)),
        }
    }

    fn remove(&self, provider_id: &str) -> Result<(), AppError> {
        let provider_id = normalize_provider_id(provider_id);
        let key = self
            .file
            .read_file()?
            .providers
            .get(provider_id.as_str())
            .and_then(|stored| match stored {
                StoredAuthData::KeyringRef(reference) => Some(reference.key().to_owned()),
                StoredAuthData::Plain(_) => None,
            })
            .unwrap_or_else(|| Self::key_for_provider(provider_id.as_str()));

        match self.secrets.delete_secret(key.as_str()) {
            Ok(()) => {}
            Err(err) if self.secret_unavailable(&err) => {
                tracing::warn!(target: "agena::auth", "keyring unavailable while removing credential; removing file auth entry only");
            }
            Err(err) => return Err(secret_error(err)),
        }
        self.file.remove(provider_id.as_str())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct AuthFile {
    #[serde(default)]
    providers: HashMap<String, StoredAuthData>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum StoredAuthData {
    KeyringRef(KeyringReference),
    Plain(AuthData),
}

impl StoredAuthData {
    fn into_plain(self, provider_id: &str) -> Result<AuthData, AppError> {
        match self {
            Self::Plain(auth) => Ok(auth),
            Self::KeyringRef(_) => Err(AppError::Config(format!(
                "{provider_id} credential is stored in keyring; enable the keyring auth backend"
            ))),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct KeyringReference {
    keyring_ref: String,
}

impl KeyringReference {
    fn key(&self) -> &str {
        self.keyring_ref.as_str()
    }
}

impl KeyringReference {
    fn new(key: String) -> Self {
        Self { keyring_ref: key }
    }
}

impl From<KeyringReference> for StoredAuthData {
    fn from(value: KeyringReference) -> Self {
        Self::KeyringRef(value)
    }
}

fn normalize_provider_id(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

fn ensure_directory(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        return Ok(());
    }
    fs::create_dir_all(path)?;
    Ok(())
}

fn secret_error(err: SecretStoreError) -> AppError {
    AppError::Config(err.to_string())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use super::*;

    #[derive(Default)]
    struct MemorySecretStore {
        data: Mutex<HashMap<String, String>>,
        unavailable: bool,
    }

    impl SecretStore for MemorySecretStore {
        fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            if self.unavailable {
                return Err(SecretStoreError::Unavailable(
                    "test keyring down".to_owned(),
                ));
            }
            Ok(self
                .data
                .lock()
                .map_err(|_| SecretStoreError::Other("lock poisoned".to_owned()))?
                .get(key)
                .cloned())
        }

        fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            if self.unavailable {
                return Err(SecretStoreError::Unavailable(
                    "test keyring down".to_owned(),
                ));
            }
            self.data
                .lock()
                .map_err(|_| SecretStoreError::Other("lock poisoned".to_owned()))?
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
            if self.unavailable {
                return Err(SecretStoreError::Unavailable(
                    "test keyring down".to_owned(),
                ));
            }
            self.data
                .lock()
                .map_err(|_| SecretStoreError::Other("lock poisoned".to_owned()))?
                .remove(key);
            Ok(())
        }
    }

    fn temp_auth_path(label: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir()
            .join(format!("agena-auth-{label}-{suffix}"))
            .join("auth.json")
    }

    #[test]
    fn file_store_reads_legacy_auth_file() {
        let store = FileAuthStore::new(temp_auth_path("legacy"));
        store
            .set(
                "openai",
                AuthData::Api {
                    key: "sk".to_owned(),
                },
            )
            .expect("legacy file auth should write");

        assert_eq!(
            store.get("openai").expect("legacy auth should read"),
            Some(AuthData::Api {
                key: "sk".to_owned()
            })
        );
    }

    #[test]
    fn keyring_store_writes_reference_and_reads_secret() {
        let path = temp_auth_path("keyring");
        let store = KeyringAuthStore::new(
            FileAuthStore::new(path.clone()),
            MemorySecretStore::default(),
            false,
        );
        store
            .set(
                "openai",
                AuthData::Api {
                    key: "sk".to_owned(),
                },
            )
            .expect("keyring auth should write");

        let text = fs::read_to_string(path).expect("auth index should exist");
        assert!(text.contains("keyring_ref"));
        assert!(!text.contains("sk"));
        assert_eq!(
            store.get("openai").expect("keyring auth should read"),
            Some(AuthData::Api {
                key: "sk".to_owned()
            })
        );
    }

    #[test]
    fn keyring_store_migrates_legacy_file_on_read() {
        let path = temp_auth_path("migrate");
        let file = FileAuthStore::new(path.clone());
        file.set(
            "openai",
            AuthData::Api {
                key: "sk".to_owned(),
            },
        )
        .expect("legacy auth should write");
        let store = KeyringAuthStore::new(file, MemorySecretStore::default(), false);

        assert_eq!(
            store.get("openai").expect("legacy auth should migrate"),
            Some(AuthData::Api {
                key: "sk".to_owned()
            })
        );
        let text = fs::read_to_string(path).expect("auth index should exist");
        assert!(text.contains("keyring_ref"));
        assert!(!text.contains("sk"));
    }

    #[test]
    fn keyring_store_falls_back_to_file_when_unavailable() {
        let path = temp_auth_path("fallback");
        let store = KeyringAuthStore::new(
            FileAuthStore::new(path.clone()),
            MemorySecretStore {
                data: Mutex::default(),
                unavailable: true,
            },
            true,
        );
        store
            .set(
                "openai",
                AuthData::Api {
                    key: "sk".to_owned(),
                },
            )
            .expect("file fallback should write");

        let text = fs::read_to_string(path).expect("auth fallback file should exist");
        assert!(text.contains("sk"));
        assert_eq!(
            store.get("openai").expect("file fallback should read"),
            Some(AuthData::Api {
                key: "sk".to_owned()
            })
        );
    }
}
