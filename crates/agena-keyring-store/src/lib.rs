//! OS keyring-backed credential store. Wraps the `keyring` crate behind a
//! single trait so providers (Anthropic, OpenAI, GitLab, …) can persist
//! tokens without each one talking to libsecret/macOS Keychain directly.
//!
//! Used by `agena::provider::auth` to store API keys and OAuth tokens.

use std::sync::OnceLock;

use keyring::use_native_store;
use keyring_core::{Entry, Error as KeyringError};

pub const DEFAULT_SERVICE: &str = "agena";

#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error("secret not found")]
    NotFound,
    #[error("keyring unavailable: {0}")]
    Unavailable(String),
    #[error("keyring error: {0}")]
    Other(String),
}

impl SecretStoreError {
    pub fn is_unavailable(&self) -> bool {
        matches!(self, Self::Unavailable(_))
    }
}

impl From<KeyringError> for SecretStoreError {
    fn from(value: KeyringError) -> Self {
        match value {
            KeyringError::NoEntry => Self::NotFound,
            KeyringError::NoStorageAccess(err) | KeyringError::PlatformFailure(err) => {
                Self::Unavailable(err.to_string())
            }
            KeyringError::NoDefaultStore => {
                Self::Unavailable("no default keyring store configured".to_owned())
            }
            other => Self::Other(other.to_string()),
        }
    }
}

pub trait SecretStore: Send + Sync {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError>;
    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError>;
    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError>;
}

#[derive(Debug, Clone)]
pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        Self {
            service: service.into(),
        }
    }

    fn entry(&self, key: &str) -> Result<Entry, SecretStoreError> {
        static KEYRING_INIT_ERROR: OnceLock<Option<String>> = OnceLock::new();
        if let Some(message) = KEYRING_INIT_ERROR
            .get_or_init(|| use_native_store(false).err().map(|err| err.to_string()))
            .as_ref()
        {
            return Err(SecretStoreError::Unavailable(message.clone()));
        }
        Entry::new(self.service.as_str(), key).map_err(SecretStoreError::from)
    }
}

impl Default for KeyringSecretStore {
    fn default() -> Self {
        Self::new(DEFAULT_SERVICE)
    }
}

impl SecretStore for KeyringSecretStore {
    fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
        match self.entry(key)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(err) => Err(SecretStoreError::from(err)),
        }
    }

    fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
        self.entry(key)?
            .set_password(value)
            .map_err(SecretStoreError::from)
    }

    fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
        match self.entry(key)?.delete_credential() {
            Ok(()) | Err(KeyringError::NoEntry) => Ok(()),
            Err(err) => Err(SecretStoreError::from(err)),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use parking_lot::Mutex;

    use super::*;

    #[derive(Default)]
    struct MemorySecretStore {
        values: Mutex<HashMap<String, String>>,
    }

    impl SecretStore for MemorySecretStore {
        fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            Ok(self.values.lock().get(key).cloned())
        }

        fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            self.values.lock().insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
            self.values.lock().remove(key);
            Ok(())
        }
    }

    #[test]
    fn mock_secret_store_round_trips_values() {
        let store = MemorySecretStore::default();
        store
            .set_secret("openai", "secret")
            .expect("secret should write");
        assert_eq!(
            store.get_secret("openai").expect("secret should read"),
            Some("secret".to_owned())
        );
        store.delete_secret("openai").expect("secret should delete");
        assert_eq!(store.get_secret("openai").unwrap(), None);
    }
}
