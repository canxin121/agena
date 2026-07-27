//! MCP credential stores.
//!
//! [`KeyringTokenStore`] is the default durable store used by the runtime.
//! It keeps the bearer value in the platform credential manager under the
//! dedicated `agena.mcp` service. Server identifiers are SHA-256 derived
//! before becoming keyring account names so arbitrary config identifiers do
//! not create invalid platform account strings or expose server names in the
//! credential index.
//!
//! [`FileTokenStore`] remains available only as an explicit compatibility
//! backend/fallback. Its on-disk file is `chmod 600` on Unix, but it is not a
//! substitute for a system secret store. Neither implementation ever writes a
//! token back into Agena's normal JSON configuration.

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use agena_keyring_store::{KeyringSecretStore, SecretStore, SecretStoreError};
use async_trait::async_trait;
use oauth2::TokenResponse;
use rmcp::transport::{AuthError, CredentialStore, StoredCredentials};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

use crate::{McpCredentialState, TokenStore};

const DEFAULT_RELATIVE_PATH: &str = "agena/mcp-tokens.json";
pub const MCP_KEYRING_SERVICE: &str = "agena.mcp";
const KEYRING_KEY_PREFIX: &str = "mcp-bearer-v1-";
const OAUTH_KEYRING_KEY_PREFIX: &str = "mcp-oauth-v1-";
/// Keep this aligned with rmcp's refresh threshold. The health projection is
/// intentionally read-only: it tells callers that a connection is likely to
/// refresh soon, but never performs that refresh itself.
const OAUTH_REFRESH_BUFFER_SECS: u64 = 30;

/// Redacted state of the single OAuth credential record associated with an
/// MCP server. No variant conveys a client id, token, scope, keyring account,
/// or error payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthCredentialState {
    /// No non-empty record exists in the dedicated OAuth keyring slot.
    Missing,
    /// A record parsed as `rmcp::transport::StoredCredentials`.
    Configured,
    /// The keyring could not be read or its value was not a valid credential
    /// record. The underlying error is deliberately not surfaced in status.
    Unreadable,
}

impl OAuthCredentialState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Configured => "configured",
            Self::Unreadable => "unreadable",
        }
    }
}

/// Redacted expiry projection for an OAuth access token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OAuthExpiryState {
    /// A token exists and has more than rmcp's refresh buffer remaining.
    Valid,
    /// A token exists but will expire within rmcp's refresh buffer.
    Expiring,
    /// A token exists and its declared lifetime has elapsed.
    Expired,
    /// The credential record has no token, no expiry, or no receipt time.
    Unknown,
}

impl OAuthExpiryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Valid => "valid",
            Self::Expiring => "expiring",
            Self::Expired => "expired",
            Self::Unknown => "unknown",
        }
    }
}

/// Safe, read-only OAuth credential health used by operational status and
/// doctor views. It has no credential material and must stay suitable for
/// logs, CLI JSON, plugin tool payloads, and APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OAuthCredentialHealth {
    pub credential_state: OAuthCredentialState,
    pub expiry_state: Option<OAuthExpiryState>,
    pub refresh_available: Option<bool>,
}

impl OAuthCredentialHealth {
    const fn missing() -> Self {
        Self {
            credential_state: OAuthCredentialState::Missing,
            expiry_state: None,
            refresh_available: None,
        }
    }

    const fn unreadable() -> Self {
        Self {
            credential_state: OAuthCredentialState::Unreadable,
            expiry_state: None,
            refresh_available: None,
        }
    }
}

#[derive(Debug, Error)]
pub enum TokenStoreError {
    #[error("io error: {0}")]
    Io(#[from] io::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
    #[error("token store lock poisoned")]
    LockPoisoned,
    #[error("keyring error: {0}")]
    Keyring(#[from] SecretStoreError),
}

/// System-keyring-backed MCP bearer store. The public methods make it usable
/// by future login/refresh/logout flows; the [`TokenStore`] implementation is
/// deliberately read-only because connection setup only needs bearer lookup.
#[derive(Clone)]
pub struct KeyringTokenStore {
    store: Arc<dyn SecretStore>,
}

impl std::fmt::Debug for KeyringTokenStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeyringTokenStore")
            .field("service", &MCP_KEYRING_SERVICE)
            .finish_non_exhaustive()
    }
}

impl Default for KeyringTokenStore {
    fn default() -> Self {
        Self::new()
    }
}

impl KeyringTokenStore {
    pub fn new() -> Self {
        Self::with_secret_store(Arc::new(KeyringSecretStore::new(MCP_KEYRING_SERVICE)))
    }

    pub fn with_secret_store(store: Arc<dyn SecretStore>) -> Self {
        Self { store }
    }

    /// Persist a bearer token without exposing it in the config or logs.
    pub fn put_bearer(&self, server: &str, token: &str) -> Result<(), TokenStoreError> {
        let server = normalized_server_name(server)?;
        let token = normalized_token(token)?;
        self.store
            .set_secret(keyring_key(server.as_str()).as_str(), token.as_str())
            .map_err(TokenStoreError::from)
    }

    /// Remove the durable bearer token. Deleting a missing value is
    /// intentionally idempotent, matching keyring-store semantics.
    pub fn delete(&self, server: &str) -> Result<(), TokenStoreError> {
        let server = normalized_server_name(server)?;
        self.store
            .delete_secret(keyring_key(server.as_str()).as_str())
            .map_err(TokenStoreError::from)
    }

    pub fn bearer_result(&self, server: &str) -> Result<Option<String>, TokenStoreError> {
        let server = normalized_server_name(server)?;
        self.store
            .get_secret(keyring_key(server.as_str()).as_str())
            .map_err(TokenStoreError::from)
            .map(|token| token.filter(|value| !value.trim().is_empty()))
    }
}

impl TokenStore for KeyringTokenStore {
    fn bearer(&self, server: &str) -> Option<String> {
        self.bearer_result(server).ok().flatten()
    }

    fn credential_state(&self, server: &str) -> McpCredentialState {
        match self.bearer_result(server) {
            Ok(Some(_)) => McpCredentialState::Configured,
            Ok(None) => McpCredentialState::Missing,
            Err(_) => McpCredentialState::Unreadable,
        }
    }
}

/// System-keyring credential store for one MCP OAuth server. OAuth client
/// registration, access tokens and refresh tokens are one JSON payload owned
/// by `rmcp`; they are never copied into normal configuration, status
/// responses, or logs. The key has a separate versioned prefix from manual
/// bearer credentials, so one logout cannot silently remove the other.
#[derive(Clone)]
pub struct KeyringOAuthCredentialStore {
    store: Arc<dyn SecretStore>,
    server: String,
}

impl std::fmt::Debug for KeyringOAuthCredentialStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KeyringOAuthCredentialStore")
            .field("service", &MCP_KEYRING_SERVICE)
            .field("server", &"[HASHED]")
            .finish_non_exhaustive()
    }
}

impl KeyringOAuthCredentialStore {
    pub fn new(server: impl AsRef<str>) -> Result<Self, TokenStoreError> {
        Self::with_secret_store(
            server,
            Arc::new(KeyringSecretStore::new(MCP_KEYRING_SERVICE)),
        )
    }

    pub fn with_secret_store(
        server: impl AsRef<str>,
        store: Arc<dyn SecretStore>,
    ) -> Result<Self, TokenStoreError> {
        Ok(Self {
            store,
            server: normalized_server_name(server.as_ref())?,
        })
    }

    pub fn delete(&self) -> Result<(), TokenStoreError> {
        self.store
            .delete_secret(oauth_keyring_key(self.server.as_str()).as_str())
            .map_err(TokenStoreError::from)
    }

    /// Inspect the local OAuth record without returning, logging, refreshing,
    /// or otherwise using its credential material. This does *not* contact an
    /// authorization server; rmcp remains responsible for refresh during an
    /// actual connection flow.
    pub fn health(&self) -> OAuthCredentialHealth {
        let raw = match self
            .store
            .get_secret(oauth_keyring_key(self.server.as_str()).as_str())
        {
            Ok(Some(value)) if !value.trim().is_empty() => value,
            Ok(Some(_)) | Ok(None) => return OAuthCredentialHealth::missing(),
            Err(_) => return OAuthCredentialHealth::unreadable(),
        };
        let credentials = match serde_json::from_str::<StoredCredentials>(raw.as_str()) {
            Ok(credentials) => credentials,
            Err(_) => return OAuthCredentialHealth::unreadable(),
        };
        let Some(token) = credentials.token_response.as_ref() else {
            return OAuthCredentialHealth {
                credential_state: OAuthCredentialState::Configured,
                expiry_state: Some(OAuthExpiryState::Unknown),
                refresh_available: Some(false),
            };
        };
        let expiry_state = match (token.expires_in(), credentials.token_received_at) {
            (Some(expires_in), Some(received_at)) => {
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let remaining = expires_in
                    .as_secs()
                    .saturating_sub(now.saturating_sub(received_at));
                if remaining == 0 {
                    OAuthExpiryState::Expired
                } else if remaining < OAUTH_REFRESH_BUFFER_SECS {
                    OAuthExpiryState::Expiring
                } else {
                    OAuthExpiryState::Valid
                }
            }
            _ => OAuthExpiryState::Unknown,
        };
        OAuthCredentialHealth {
            credential_state: OAuthCredentialState::Configured,
            expiry_state: Some(expiry_state),
            refresh_available: Some(token.refresh_token().is_some()),
        }
    }

    fn auth_error(error: impl std::fmt::Display) -> AuthError {
        AuthError::InternalError(format!("MCP OAuth credential store: {error}"))
    }
}

#[async_trait]
impl CredentialStore for KeyringOAuthCredentialStore {
    async fn load(&self) -> Result<Option<StoredCredentials>, AuthError> {
        let raw = self
            .store
            .get_secret(oauth_keyring_key(self.server.as_str()).as_str())
            .map_err(Self::auth_error)?;
        raw.filter(|value| !value.trim().is_empty())
            .map(|value| serde_json::from_str(value.as_str()).map_err(Self::auth_error))
            .transpose()
    }

    async fn save(&self, credentials: StoredCredentials) -> Result<(), AuthError> {
        let raw = serde_json::to_string(&credentials).map_err(Self::auth_error)?;
        self.store
            .set_secret(
                oauth_keyring_key(self.server.as_str()).as_str(),
                raw.as_str(),
            )
            .map_err(Self::auth_error)
    }

    async fn clear(&self) -> Result<(), AuthError> {
        self.store
            .delete_secret(oauth_keyring_key(self.server.as_str()).as_str())
            .map_err(Self::auth_error)
    }
}

/// Read-through composition used when a user explicitly opts into the legacy
/// file fallback. A keyring value always wins; the file is consulted only
/// after the keyring has no value or is unavailable.
pub struct FallbackTokenStore {
    primary: Arc<dyn TokenStore>,
    fallback: Arc<dyn TokenStore>,
}

impl std::fmt::Debug for FallbackTokenStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FallbackTokenStore")
            .field("primary", &"TokenStore")
            .field("fallback", &"TokenStore")
            .finish()
    }
}

impl FallbackTokenStore {
    pub fn new(primary: Arc<dyn TokenStore>, fallback: Arc<dyn TokenStore>) -> Self {
        Self { primary, fallback }
    }
}

impl TokenStore for FallbackTokenStore {
    fn bearer(&self, server: &str) -> Option<String> {
        self.primary
            .bearer(server)
            .or_else(|| self.fallback.bearer(server))
    }

    fn credential_state(&self, server: &str) -> McpCredentialState {
        let primary = self.primary.credential_state(server);
        let fallback = self.fallback.credential_state(server);
        match (primary, fallback) {
            (McpCredentialState::Configured, _) | (_, McpCredentialState::Configured) => {
                McpCredentialState::Configured
            }
            (McpCredentialState::Unreadable, _) | (_, McpCredentialState::Unreadable) => {
                McpCredentialState::Unreadable
            }
            _ => McpCredentialState::Missing,
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct StoreFile {
    #[serde(default)]
    servers: BTreeMap<String, ServerTokenRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ServerTokenRecord {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    bearer: Option<String>,
}

#[derive(Debug)]
pub struct FileTokenStore {
    path: PathBuf,
    inner: Mutex<StoreFile>,
}

impl FileTokenStore {
    /// Open or create the token file at the default `~/agena/mcp-tokens.json`
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
        let server = normalized_server_name(server)?;
        let token = normalized_token(token)?;
        {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| TokenStoreError::LockPoisoned)?;
            let entry = guard
                .servers
                .entry(server)
                .or_insert_with(|| ServerTokenRecord { bearer: None });
            entry.bearer = Some(token);
        }
        self.persist()
    }

    pub fn delete(&self, server: &str) -> Result<bool, TokenStoreError> {
        let server = normalized_server_name(server)?;
        let removed = {
            let mut guard = self
                .inner
                .lock()
                .map_err(|_| TokenStoreError::LockPoisoned)?;
            guard.servers.remove(server.as_str()).is_some()
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

    fn credential_state(&self, server: &str) -> McpCredentialState {
        let Ok(guard) = self.inner.lock() else {
            return McpCredentialState::Unreadable;
        };
        if guard
            .servers
            .get(server)
            .and_then(|entry| entry.bearer.as_deref())
            .is_some_and(|token| !token.trim().is_empty())
        {
            McpCredentialState::Configured
        } else {
            McpCredentialState::Missing
        }
    }
}

fn normalized_server_name(server: &str) -> Result<String, TokenStoreError> {
    let server = server.trim();
    if server.is_empty() {
        return Err(TokenStoreError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MCP server name must not be empty",
        )));
    }
    Ok(server.to_owned())
}

fn normalized_token(token: &str) -> Result<String, TokenStoreError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(TokenStoreError::Io(io::Error::new(
            io::ErrorKind::InvalidInput,
            "MCP bearer token must not be empty",
        )));
    }
    Ok(token.to_owned())
}

fn keyring_key(server: &str) -> String {
    let digest = Sha256::digest(server.as_bytes());
    let mut key = String::with_capacity(KEYRING_KEY_PREFIX.len() + digest.len() * 2);
    key.push_str(KEYRING_KEY_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(key, "{byte:02x}");
    }
    key
}

fn oauth_keyring_key(server: &str) -> String {
    let digest = Sha256::digest(server.as_bytes());
    let mut key = String::with_capacity(OAUTH_KEYRING_KEY_PREFIX.len() + digest.len() * 2);
    key.push_str(OAUTH_KEYRING_KEY_PREFIX);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(key, "{byte:02x}");
    }
    key
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
fn chmod_user_only(_: &Path) {}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use agena_keyring_store::{SecretStore, SecretStoreError};
    use oauth2::{AccessToken, RefreshToken, basic::BasicTokenType};

    use rmcp::transport::auth::OAuthTokenResponse;
    use rmcp::transport::{CredentialStore, StoredCredentials};

    use super::{
        KeyringOAuthCredentialStore, KeyringTokenStore, OAuthCredentialState, OAuthExpiryState,
        TokenStore, keyring_key, oauth_keyring_key,
    };

    #[derive(Default)]
    struct InMemorySecretStore(Mutex<BTreeMap<String, String>>);

    impl SecretStore for InMemorySecretStore {
        fn get_secret(&self, key: &str) -> Result<Option<String>, SecretStoreError> {
            Ok(self.0.lock().expect("lock").get(key).cloned())
        }

        fn set_secret(&self, key: &str, value: &str) -> Result<(), SecretStoreError> {
            self.0
                .lock()
                .expect("lock")
                .insert(key.to_owned(), value.to_owned());
            Ok(())
        }

        fn delete_secret(&self, key: &str) -> Result<(), SecretStoreError> {
            self.0.lock().expect("lock").remove(key);
            Ok(())
        }
    }

    #[test]
    fn keyring_store_uses_hashed_server_keys_and_has_a_full_lifecycle() {
        let backing = std::sync::Arc::new(InMemorySecretStore::default());
        let store = KeyringTokenStore::with_secret_store(backing.clone());

        store
            .put_bearer("example-server", " bearer-value ")
            .expect("put");
        assert_eq!(
            store.bearer("example-server").as_deref(),
            Some("bearer-value")
        );
        assert!(
            backing
                .0
                .lock()
                .expect("lock")
                .contains_key(keyring_key("example-server").as_str())
        );
        assert!(
            !backing
                .0
                .lock()
                .expect("lock")
                .contains_key("example-server")
        );

        store.delete("example-server").expect("delete");
        assert_eq!(store.bearer("example-server"), None);
    }

    #[tokio::test]
    async fn oauth_store_is_separate_hashed_key_and_clears_its_full_record() {
        let backing = std::sync::Arc::new(InMemorySecretStore::default());
        let store =
            KeyringOAuthCredentialStore::with_secret_store("example-server", backing.clone())
                .expect("oauth store");
        let credentials = StoredCredentials::new(
            "registered-client".to_owned(),
            None,
            vec!["mcp:read".to_owned()],
            Some(123),
        );

        store.save(credentials).await.expect("save");
        let loaded = store.load().await.expect("load").expect("credentials");
        assert_eq!(loaded.client_id, "registered-client");
        assert_eq!(loaded.granted_scopes, ["mcp:read"]);
        {
            let guard = backing.0.lock().expect("lock");
            assert!(guard.contains_key(oauth_keyring_key("example-server").as_str()));
            assert!(!guard.contains_key(keyring_key("example-server").as_str()));
            assert!(!guard.contains_key("example-server"));
        }

        store.clear().await.expect("clear");
        assert!(store.load().await.expect("load after clear").is_none());
    }

    #[test]
    fn oauth_health_is_redacted_and_classifies_missing_unreadable_and_known_expiry() {
        let backing = std::sync::Arc::new(InMemorySecretStore::default());
        let store =
            KeyringOAuthCredentialStore::with_secret_store("example-server", backing.clone())
                .expect("oauth store");

        let missing = store.health();
        assert_eq!(missing.credential_state, OAuthCredentialState::Missing);
        assert_eq!(missing.expiry_state, None);
        assert_eq!(missing.refresh_available, None);

        backing
            .set_secret(
                oauth_keyring_key("example-server").as_str(),
                "not valid json",
            )
            .expect("write malformed record");
        let unreadable = store.health();
        assert_eq!(
            unreadable.credential_state,
            OAuthCredentialState::Unreadable
        );
        assert_eq!(unreadable.expiry_state, None);

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current epoch")
            .as_secs();
        let mut valid_token = OAuthTokenResponse::new(
            AccessToken::new("access-token-must-not-leak".to_owned()),
            BasicTokenType::Bearer,
            Default::default(),
        );
        valid_token.set_expires_in(Some(&Duration::from_secs(3600)));
        valid_token.set_refresh_token(Some(RefreshToken::new(
            "refresh-token-must-not-leak".to_owned(),
        )));
        let valid = StoredCredentials::new(
            "client-id-must-not-leak".to_owned(),
            Some(valid_token),
            vec!["sensitive-scope".to_owned()],
            Some(now),
        );
        let encoded_valid = serde_json::to_string(&valid).expect("serialize credential");
        backing
            .set_secret(
                oauth_keyring_key("example-server").as_str(),
                encoded_valid.as_str(),
            )
            .expect("write credential");
        let health = store.health();
        assert_eq!(health.credential_state, OAuthCredentialState::Configured);
        assert_eq!(health.expiry_state, Some(OAuthExpiryState::Valid));
        assert_eq!(health.refresh_available, Some(true));
        let debug = format!("{health:?}");
        assert!(!debug.contains("access-token-must-not-leak"));
        assert!(!debug.contains("refresh-token-must-not-leak"));
        assert!(!debug.contains("client-id-must-not-leak"));

        let mut expiring_token = OAuthTokenResponse::new(
            AccessToken::new("other-access-token".to_owned()),
            BasicTokenType::Bearer,
            Default::default(),
        );
        expiring_token.set_expires_in(Some(&Duration::from_secs(1)));
        let expiring = StoredCredentials::new(
            "client".to_owned(),
            Some(expiring_token),
            Vec::new(),
            Some(now),
        );
        let encoded_expiring = serde_json::to_string(&expiring).expect("serialize credential");
        backing
            .set_secret(
                oauth_keyring_key("example-server").as_str(),
                encoded_expiring.as_str(),
            )
            .expect("write expiring credential");
        let expiring_health = store.health();
        assert_eq!(
            expiring_health.expiry_state,
            Some(OAuthExpiryState::Expiring)
        );
        assert_eq!(expiring_health.refresh_available, Some(false));

        let mut expired_token = OAuthTokenResponse::new(
            AccessToken::new("expired-access-token".to_owned()),
            BasicTokenType::Bearer,
            Default::default(),
        );
        expired_token.set_expires_in(Some(&Duration::from_secs(1)));
        let expired = StoredCredentials::new(
            "client".to_owned(),
            Some(expired_token),
            Vec::new(),
            Some(now.saturating_sub(5)),
        );
        let encoded_expired = serde_json::to_string(&expired).expect("serialize credential");
        backing
            .set_secret(
                oauth_keyring_key("example-server").as_str(),
                encoded_expired.as_str(),
            )
            .expect("write expired credential");
        assert_eq!(store.health().expiry_state, Some(OAuthExpiryState::Expired));
    }
}
