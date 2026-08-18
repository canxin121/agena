//! Remote MCP transport with an optional OAuth resource/authorization server.
//!
//! The stdio bridge lives in `agena-cli`; this module is the HTTP-facing
//! integration used by ChatGPT's Custom MCP Connector. The MCP surface is
//! anonymous by default; the Web/TUI control plane can enable the OAuth
//! resource server when a client requires it.

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, AtomicI64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_application::{Application, dto::OperatorToolResource};
use agena_keyring_store::{KeyringSecretStore, SecretStore};
use agena_mcp_server::{
    CallToolParams, CallToolResult, McpServerBackend, McpServerError, ToolDescriptor,
    serialize_call_tool_result, serialize_tool_descriptor,
};
use async_trait::async_trait;
use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Form, Query, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware,
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use dashmap::DashMap;
use ed25519_dalek::SigningKey;
use futures_util::StreamExt;
use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, encode};
use rmcp::transport::StreamableHttpServerConfig;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use super::auth::{self, OAuthPasswordError, UiAuth};

use super::persistence::db::{
    KV_KEY_MCP_OAUTH_RUNTIME, KV_KEY_MCP_OAUTH_SIGNING_KEY, KV_KEY_MCP_SERVER_CONTROL,
    ServerStateDb,
};

const MCP_SCOPE: &str = "agena:tools";
const MCP_PATH: &str = "/mcp";
const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(60 * 60);
const REFRESH_TOKEN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const AUTHORIZATION_CODE_TTL: Duration = Duration::from_secs(5 * 60);
const CIMD_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_MCP_METADATA_BODY_BYTES: usize = 8 * 1024 * 1024;
const MAX_CIMD_DOCUMENT_BYTES: usize = 256 * 1024;
const MAX_MCP_OAUTH_PASSWORD_BYTES: usize = 4096;
const OPENAI_MCP_DISCOVER_METHOD: &str = "server/discover";
const MCP_DISCOVERY_SUPPORTED_VERSIONS: &[&str] =
    &["2026-07-28", "2025-11-25", "2025-06-18", "2025-03-26"];
const MCP_DISCOVERY_TTL_MS: u64 = 60 * 60 * 1000;
const DEFAULT_MCP_PROTOCOL_VERSION: &str = "2025-06-18";
const OAUTH_SIGNING_KEY_VERSION: u32 = 1;
const OAUTH_RUNTIME_VERSION: u32 = 1;
const OAUTH_KEYRING_SERVICE: &str = "agena";
const OAUTH_KEYRING_READ_TIMEOUT: Duration = Duration::from_millis(750);

const HIDDEN_MCP_PLUGIN_IDS: &[&str] = &["agena.chatgpt", "agena.gemini", "agena.claude"];
const KNOWN_INTERACTIVE_MCP_TOOL_NAMES: &[&str] = &["interaction.ask", "interaction.notify"];
const KNOWN_INTERACTIVE_MCP_TOOL_PREFIXES: &[&str] = &["web.browser_", "agena.web.browser_"];

#[derive(Clone)]
pub(crate) struct McpServerState {
    application: Application,
    workspace_id: i64,
    next_call_id: Arc<AtomicI64>,
    ui_auth: UiAuth,
    oauth: Arc<OAuthState>,
    control: Arc<McpControlState>,
    fallback_resource: String,
}

struct McpControlState {
    enabled: AtomicBool,
    auth_enabled: AtomicBool,
    configured_resource: RwLock<Option<String>>,
    configured_issuer: RwLock<Option<String>>,
    oauth_password: RwLock<Option<McpOAuthPassword>>,
    database: Option<Arc<ServerStateDb>>,
    update_lock: tokio::sync::Mutex<()>,
}

struct McpOAuthPassword {
    verifier: UiAuth,
    phc: String,
}

#[derive(Debug, Clone)]
struct McpReadiness {
    ready: bool,
    warnings: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedMcpServerControl {
    #[serde(default = "default_mcp_server_enabled")]
    enabled: bool,
    #[serde(default)]
    public_url: Option<String>,
    #[serde(default)]
    auth_enabled: bool,
    #[serde(default)]
    oauth_password_phc: Option<String>,
    #[serde(default)]
    oauth_issuer_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct PersistedSigningKey {
    version: u32,
    secret_key: String,
}

#[derive(Clone)]
struct OAuthSigningKey {
    signing_key: Arc<SigningKey>,
    kid: String,
}

impl OAuthSigningKey {
    fn ephemeral() -> Self {
        let mut secret = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
        getrandom::fill(&mut secret).expect("MCP OAuth signing key generation failed");
        Self::from_secret_key(secret).expect("MCP OAuth signing key must be valid")
    }

    fn from_secret_key(secret_key: [u8; ed25519_dalek::SECRET_KEY_LENGTH]) -> Result<Self, String> {
        let signing_key = SigningKey::from_bytes(&secret_key);
        let kid = URL_SAFE_NO_PAD.encode(Sha256::digest(signing_key.verifying_key().as_bytes()));
        Ok(Self {
            signing_key: Arc::new(signing_key),
            kid,
        })
    }

    fn secret_key_b64(&self) -> String {
        URL_SAFE_NO_PAD.encode(self.signing_key.to_bytes())
    }

    fn encoding_key(&self) -> Result<EncodingKey, String> {
        use ed25519_dalek::pkcs8::EncodePrivateKey;

        let der = self
            .signing_key
            .to_pkcs8_der()
            .map_err(|error| format!("failed to encode MCP OAuth signing key: {error}"))?;
        Ok(EncodingKey::from_ed_der(der.as_bytes()))
    }

    fn decoding_key(&self) -> DecodingKey {
        DecodingKey::from_ed_components(
            URL_SAFE_NO_PAD
                .encode(self.signing_key.verifying_key().as_bytes())
                .as_str(),
        )
        .expect("MCP OAuth public signing key must be valid")
    }

    fn jwks(&self) -> serde_json::Value {
        serde_json::json!({
            "keys": [{
                "kty": "OKP",
                "crv": "Ed25519",
                "x": URL_SAFE_NO_PAD.encode(self.signing_key.verifying_key().as_bytes()),
                "use": "sig",
                "alg": "EdDSA",
                "kid": self.kid,
            }]
        })
    }
}

async fn load_or_create_oauth_signing_key(
    database: &Arc<ServerStateDb>,
) -> Result<OAuthSigningKey, String> {
    // The server database is the normal durable source. macOS Keychain calls
    // are synchronous and can wait indefinitely for an access decision; a
    // blocked keychain lookup must never hold up the HTTP listener (or make a
    // tunnel report the server as unhealthy).
    if let Some(persisted) = database
        .get_json::<PersistedSigningKey>(KV_KEY_MCP_OAUTH_SIGNING_KEY)
        .await?
    {
        if persisted.version != OAUTH_SIGNING_KEY_VERSION {
            return Err("unsupported persisted MCP OAuth signing key version".to_owned());
        }
        let key = decode_persisted_signing_key(persisted.secret_key.as_str())?;
        return Ok(key);
    }

    // Migrate an existing keyring value once, but bound the legacy lookup.
    // The blocking task is intentionally isolated from Tokio's worker pool;
    // if the platform keychain remains blocked, the server still falls back
    // to its database copy below and continues starting normally.
    let keyring = KeyringSecretStore::new(OAUTH_KEYRING_SERVICE);
    let keyring_result = tokio::time::timeout(
        OAUTH_KEYRING_READ_TIMEOUT,
        tokio::task::spawn_blocking(move || keyring.get_secret(KV_KEY_MCP_OAUTH_SIGNING_KEY)),
    )
    .await;
    match keyring_result {
        Ok(Ok(Ok(Some(value)))) => {
            let key = decode_persisted_signing_key(value.as_str())?;
            database
                .set_json(
                    KV_KEY_MCP_OAUTH_SIGNING_KEY,
                    &PersistedSigningKey {
                        version: OAUTH_SIGNING_KEY_VERSION,
                        secret_key: key.secret_key_b64(),
                    },
                )
                .await?;
            return Ok(key);
        }
        Ok(Ok(Ok(None))) => {}
        Ok(Ok(Err(error))) => {
            tracing::debug!(error = %error, "MCP OAuth keyring migration unavailable; using server database");
        }
        Ok(Err(error)) => {
            tracing::debug!(error = %error, "MCP OAuth keyring migration task failed; using server database");
        }
        Err(_) => {
            tracing::debug!("MCP OAuth keyring migration timed out; using server database");
        }
    }

    let key = OAuthSigningKey::ephemeral();
    let encoded = key.secret_key_b64();
    database
        .set_json(
            KV_KEY_MCP_OAUTH_SIGNING_KEY,
            &PersistedSigningKey {
                version: OAUTH_SIGNING_KEY_VERSION,
                secret_key: encoded,
            },
        )
        .await?;
    Ok(key)
}

fn decode_persisted_signing_key(value: &str) -> Result<OAuthSigningKey, String> {
    let bytes = URL_SAFE_NO_PAD
        .decode(value)
        .map_err(|error| format!("invalid persisted MCP OAuth signing key: {error}"))?;
    let secret_key: [u8; ed25519_dalek::SECRET_KEY_LENGTH] = bytes
        .try_into()
        .map_err(|_| "invalid persisted MCP OAuth signing key length".to_owned())?;
    OAuthSigningKey::from_secret_key(secret_key)
}

fn default_mcp_server_enabled() -> bool {
    true
}

impl McpServerState {
    #[allow(dead_code)]
    pub(crate) fn new(
        application: Application,
        workspace_id: i64,
        ui_auth: UiAuth,
        configured_public_url: Option<&str>,
        fallback_public_url: &str,
    ) -> Result<Self, String> {
        let configured_resource = configured_public_url
            .map(normalize_public_mcp_url)
            .transpose()?;
        let fallback_resource = normalize_public_mcp_url(fallback_public_url)?;
        Ok(Self::with_control(
            application,
            workspace_id,
            Arc::new(AtomicI64::new(1)),
            ui_auth,
            Arc::new(OAuthState::new(OAuthSigningKey::ephemeral(), None, None)),
            configured_resource,
            None,
            fallback_resource,
            true,
            false,
            None,
            None,
        ))
    }

    pub(crate) async fn load(
        application: Application,
        workspace_id: i64,
        ui_auth: UiAuth,
        database: Arc<ServerStateDb>,
        configured_public_url: Option<&str>,
        fallback_public_url: &str,
    ) -> Result<Self, String> {
        let persisted = database
            .get_json::<PersistedMcpServerControl>(KV_KEY_MCP_SERVER_CONTROL)
            .await?;
        let (enabled, auth_enabled, configured_public_url, configured_issuer, oauth_password) =
            match persisted {
                Some(persisted) => {
                    let oauth_password = persisted
                        .oauth_password_phc
                        .map(|phc| {
                            let verifier = auth::init_ui_auth_from_phc(phc.clone())?;
                            Ok::<_, String>(McpOAuthPassword { verifier, phc })
                        })
                        .transpose()?;
                    (
                        persisted.enabled,
                        persisted.auth_enabled,
                        persisted.public_url,
                        persisted.oauth_issuer_url,
                        oauth_password,
                    )
                }
                None => {
                    let configured_public_url = configured_public_url
                        .map(normalize_public_mcp_url)
                        .transpose()?;
                    (true, false, configured_public_url, None, None)
                }
            };
        let configured_resource = configured_public_url
            .map(|value| normalize_public_mcp_url(value.as_str()))
            .transpose()?;
        let configured_issuer = configured_issuer
            .map(|value| normalize_oauth_issuer_url(value.as_str()))
            .transpose()?;
        let fallback_resource = normalize_public_mcp_url(fallback_public_url)?;
        let signing_key = load_or_create_oauth_signing_key(&database).await?;
        let persisted_runtime = database
            .get_json::<PersistedOAuthRuntime>(KV_KEY_MCP_OAUTH_RUNTIME)
            .await?;
        if persisted_runtime
            .as_ref()
            .is_some_and(|runtime| runtime.version != OAUTH_RUNTIME_VERSION)
        {
            return Err("unsupported persisted MCP OAuth runtime version".to_owned());
        }
        Ok(Self::with_control(
            application,
            workspace_id,
            Arc::new(AtomicI64::new(1)),
            ui_auth,
            Arc::new(OAuthState::new(
                signing_key,
                Some(Arc::clone(&database)),
                persisted_runtime,
            )),
            configured_resource,
            configured_issuer,
            fallback_resource,
            enabled,
            auth_enabled,
            oauth_password,
            Some(database),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn with_control(
        application: Application,
        workspace_id: i64,
        next_call_id: Arc<AtomicI64>,
        ui_auth: UiAuth,
        oauth: Arc<OAuthState>,
        configured_resource: Option<String>,
        configured_issuer: Option<String>,
        fallback_resource: String,
        enabled: bool,
        auth_enabled: bool,
        oauth_password: Option<McpOAuthPassword>,
        database: Option<Arc<ServerStateDb>>,
    ) -> Self {
        Self {
            application,
            workspace_id,
            next_call_id,
            ui_auth,
            oauth,
            control: Arc::new(McpControlState {
                enabled: AtomicBool::new(enabled),
                auth_enabled: AtomicBool::new(auth_enabled),
                configured_resource: RwLock::new(configured_resource),
                configured_issuer: RwLock::new(configured_issuer),
                oauth_password: RwLock::new(oauth_password),
                database,
                update_lock: tokio::sync::Mutex::new(()),
            }),
            fallback_resource,
        }
    }

    fn is_enabled(&self) -> bool {
        self.control.enabled.load(Ordering::Acquire)
    }

    fn auth_enabled(&self) -> bool {
        self.control.auth_enabled.load(Ordering::Acquire)
    }

    fn configured_public_url(&self) -> Option<String> {
        self.control
            .configured_resource
            .read()
            .expect("MCP configured URL lock poisoned")
            .clone()
    }

    fn configured_oauth_issuer_url(&self) -> Option<String> {
        self.control
            .configured_issuer
            .read()
            .expect("MCP OAuth issuer lock poisoned")
            .clone()
    }

    fn has_custom_oauth_password(&self) -> bool {
        self.control
            .oauth_password
            .read()
            .expect("MCP OAuth password lock poisoned")
            .is_some()
    }

    fn oauth_password_phc(&self) -> Option<String> {
        self.control
            .oauth_password
            .read()
            .expect("MCP OAuth password lock poisoned")
            .as_ref()
            .map(|password| password.phc.clone())
    }

    fn public_resource(&self, headers: &HeaderMap) -> String {
        let configured_resource = self.configured_public_url();
        self.public_resource_with_configured(headers, configured_resource.as_deref())
    }

    fn public_resource_with_configured(
        &self,
        headers: &HeaderMap,
        configured_resource: Option<&str>,
    ) -> String {
        if let Some(resource) = configured_resource {
            return resource.to_owned();
        }

        let scheme = forwarded_header(headers, "x-forwarded-proto")
            .filter(|value| {
                value.eq_ignore_ascii_case("http") || value.eq_ignore_ascii_case("https")
            })
            .unwrap_or("http");
        let host = forwarded_header(headers, "x-forwarded-host")
            .or_else(|| forwarded_header(headers, "host"));
        let Some(host) = host else {
            return self.fallback_resource.clone();
        };

        let candidate = format!("{scheme}://{host}{MCP_PATH}");
        normalize_public_mcp_url(candidate.as_str())
            .unwrap_or_else(|_| self.fallback_resource.clone())
    }

    fn issuer_for_headers(&self, headers: &HeaderMap) -> String {
        self.configured_oauth_issuer_url()
            .unwrap_or_else(|| issuer_for_resource(self.public_resource(headers).as_str()))
    }

    fn readiness(&self, headers: &HeaderMap) -> McpReadiness {
        let mut warnings = Vec::new();
        let resource = self.public_resource(headers);

        if !self.is_enabled() {
            warnings.push("MCP server is disabled.".to_owned());
        }
        if self.auth_enabled() {
            if !is_https_resource(resource.as_str()) {
                warnings.push(
                    "OAuth requires a canonical HTTPS MCP resource URL; configure a public HTTPS URL or Secure MCP Tunnel before connecting ChatGPT.".to_owned(),
                );
            }
            if !self.has_custom_oauth_password() && !matches!(self.ui_auth, UiAuth::Enabled(_)) {
                warnings.push(
                    "OAuth authorization password is not configured; set an MCP OAuth password in Web/TUI or start the server with a UI password.".to_owned(),
                );
            }
            let issuer = self.issuer_for_headers(headers);
            if !is_https_resource(issuer.as_str()) {
                warnings.push(
                    "OAuth issuer must be a canonical HTTPS URL; configure an externally reachable HTTPS issuer before connecting ChatGPT.".to_owned(),
                );
            }
        }

        McpReadiness {
            ready: warnings.is_empty(),
            warnings,
        }
    }

    fn next_call_id(&self) -> i64 {
        self.next_call_id.fetch_add(1, Ordering::Relaxed)
    }

    async fn verify_oauth_password(
        &self,
        candidate: &str,
        headers: &HeaderMap,
    ) -> Result<(), OAuthPasswordError> {
        let custom_verifier = self
            .control
            .oauth_password
            .read()
            .expect("MCP OAuth password lock poisoned")
            .as_ref()
            .map(|password| password.verifier.clone());
        match custom_verifier {
            Some(verifier) => auth::verify_password_for_oauth(&verifier, candidate, headers),
            None => auth::verify_password_for_oauth(&self.ui_auth, candidate, headers),
        }
    }

    async fn clear_oauth_runtime_state(&self) -> Result<(), String> {
        self.oauth.clients.clear();
        self.oauth.cimd_clients.clear();
        self.oauth.authorization_codes.clear();
        self.oauth.refresh_tokens.clear();
        self.oauth.revoked_jti.clear();
        self.oauth.persist().await
    }

    fn control_status(&self, headers: &HeaderMap) -> McpServerControlResponse {
        let resource_url = self.public_resource(headers);
        let issuer = self.issuer_for_headers(headers);
        let auth_enabled = self.auth_enabled();
        let readiness = self.readiness(headers);
        McpServerControlResponse {
            enabled: self.is_enabled(),
            auth_enabled,
            public_url: self.configured_public_url(),
            oauth_issuer_url: self.configured_oauth_issuer_url(),
            resource_url,
            ready: readiness.ready,
            warnings: readiness.warnings.clone(),
            oauth: auth_enabled.then(|| McpOAuthControlStatus {
                configured: self.has_custom_oauth_password()
                    || matches!(self.ui_auth, UiAuth::Enabled(_)),
                password_configured: self.has_custom_oauth_password(),
                fallback_to_ui_password: !self.has_custom_oauth_password()
                    && matches!(self.ui_auth, UiAuth::Enabled(_)),
                ready: readiness.ready,
                authorization_server_kind: "agena-managed".to_owned(),
                registration_methods: vec!["cimd".to_owned(), "dcr".to_owned()],
                // The current Agena token endpoint implements the public
                // client/PKCE flow only. Do not advertise private_key_jwt
                // until assertion signature and replay validation are live;
                // ChatGPT uses this metadata to choose its token client
                // authentication method.
                token_endpoint_auth_methods: vec!["none".to_owned()],
                pkce_methods: vec!["S256".to_owned()],
                oidc_supported: false,
                warnings: readiness.warnings.clone(),
                scope: MCP_SCOPE.to_owned(),
                issuer: issuer.clone(),
                authorization_endpoint: append_public_endpoint(&issuer, "/oauth/authorize"),
                token_endpoint: append_public_endpoint(&issuer, "/oauth/token"),
                registration_endpoint: append_public_endpoint(&issuer, "/oauth/register"),
                revocation_endpoint: append_public_endpoint(&issuer, "/oauth/revoke"),
                protected_resource_metadata: append_public_endpoint(
                    &issuer,
                    "/.well-known/oauth-protected-resource",
                ),
                authorization_server_metadata: append_public_endpoint(
                    &issuer,
                    "/.well-known/oauth-authorization-server",
                ),
                jwks_uri: append_public_endpoint(&issuer, "/oauth/jwks.json"),
            }),
        }
    }

    async fn persist_control(
        &self,
        enabled: bool,
        auth_enabled: bool,
        public_url: Option<String>,
        oauth_issuer_url: Option<String>,
    ) -> Result<(), String> {
        let database = self
            .control
            .database
            .as_ref()
            .ok_or_else(|| "MCP server control persistence is unavailable".to_owned())?;
        database
            .set_json(
                KV_KEY_MCP_SERVER_CONTROL,
                &PersistedMcpServerControl {
                    enabled,
                    auth_enabled,
                    public_url,
                    oauth_issuer_url,
                    oauth_password_phc: self.oauth_password_phc(),
                },
            )
            .await
    }

    async fn update_control(
        &self,
        enabled: bool,
        auth_enabled: bool,
        configured_resource: Option<String>,
        configured_issuer: Option<String>,
    ) -> Result<(), String> {
        let _guard = self.control.update_lock.lock().await;
        let previous_enabled = self.is_enabled();
        let previous_auth_enabled = self.auth_enabled();
        let previous_resource = self.configured_public_url();
        let previous_issuer = self.configured_oauth_issuer_url();
        self.persist_control(
            enabled,
            auth_enabled,
            configured_resource.clone(),
            configured_issuer.clone(),
        )
        .await?;
        self.control.enabled.store(enabled, Ordering::Release);
        self.control
            .auth_enabled
            .store(auth_enabled, Ordering::Release);
        *self
            .control
            .configured_resource
            .write()
            .expect("MCP configured URL lock poisoned") = configured_resource;
        *self
            .control
            .configured_issuer
            .write()
            .expect("MCP OAuth issuer lock poisoned") = configured_issuer;
        if previous_enabled != enabled
            || previous_auth_enabled != auth_enabled
            || previous_resource != self.configured_public_url()
            || previous_issuer != self.configured_oauth_issuer_url()
        {
            self.clear_oauth_runtime_state().await?;
        }
        Ok(())
    }

    async fn set_oauth_password(&self, password: &str) -> Result<(), String> {
        let password = password.trim();
        if password.is_empty() {
            return Err("MCP OAuth password must not be empty".to_owned());
        }
        if password.len() > MAX_MCP_OAUTH_PASSWORD_BYTES {
            return Err(format!(
                "MCP OAuth password must be at most {MAX_MCP_OAUTH_PASSWORD_BYTES} bytes"
            ));
        }
        let phc = auth::hash_password(password)?;
        let verifier = auth::init_ui_auth_from_phc(phc.clone())?;
        let _guard = self.control.update_lock.lock().await;
        let enabled = self.is_enabled();
        let public_url = self.configured_public_url();
        let oauth_issuer_url = self.configured_oauth_issuer_url();
        let database = self
            .control
            .database
            .as_ref()
            .ok_or_else(|| "MCP server control persistence is unavailable".to_owned())?;
        database
            .set_json(
                KV_KEY_MCP_SERVER_CONTROL,
                &PersistedMcpServerControl {
                    enabled,
                    auth_enabled: self.auth_enabled(),
                    public_url,
                    oauth_issuer_url,
                    oauth_password_phc: Some(phc.clone()),
                },
            )
            .await?;
        *self
            .control
            .oauth_password
            .write()
            .expect("MCP OAuth password lock poisoned") = Some(McpOAuthPassword { verifier, phc });
        self.clear_oauth_runtime_state().await?;
        Ok(())
    }

    async fn clear_oauth_password(&self) -> Result<(), String> {
        let _guard = self.control.update_lock.lock().await;
        let enabled = self.is_enabled();
        let public_url = self.configured_public_url();
        let oauth_issuer_url = self.configured_oauth_issuer_url();
        let database = self
            .control
            .database
            .as_ref()
            .ok_or_else(|| "MCP server control persistence is unavailable".to_owned())?;
        database
            .set_json(
                KV_KEY_MCP_SERVER_CONTROL,
                &PersistedMcpServerControl {
                    enabled,
                    auth_enabled: self.auth_enabled(),
                    public_url,
                    oauth_issuer_url,
                    oauth_password_phc: None,
                },
            )
            .await?;
        *self
            .control
            .oauth_password
            .write()
            .expect("MCP OAuth password lock poisoned") = None;
        self.clear_oauth_runtime_state().await?;
        Ok(())
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerControlResponse {
    enabled: bool,
    auth_enabled: bool,
    public_url: Option<String>,
    oauth_issuer_url: Option<String>,
    resource_url: String,
    ready: bool,
    warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    oauth: Option<McpOAuthControlStatus>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct McpOAuthControlStatus {
    configured: bool,
    password_configured: bool,
    fallback_to_ui_password: bool,
    ready: bool,
    authorization_server_kind: String,
    registration_methods: Vec<String>,
    token_endpoint_auth_methods: Vec<String>,
    pkce_methods: Vec<String>,
    oidc_supported: bool,
    warnings: Vec<String>,
    scope: String,
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    revocation_endpoint: String,
    protected_resource_metadata: String,
    authorization_server_metadata: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UpdateMcpServerControlRequest {
    #[serde(default)]
    enabled: Option<bool>,
    /// `None` means "leave unchanged"; `Some(None)` explicitly clears the
    /// configured public URL and returns to request-derived discovery.
    #[serde(default)]
    public_url: Option<Option<String>>,
    /// `None` means "leave unchanged". When disabled, the MCP endpoint is
    /// anonymous and does not publish or enforce OAuth.
    #[serde(default)]
    auth_enabled: Option<bool>,
    /// `None` means "leave unchanged"; `Some(None)` clears the configured
    /// issuer and returns to the resource-derived issuer.
    #[serde(default)]
    oauth_issuer_url: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SetMcpOAuthPasswordRequest {
    password: String,
}

pub(crate) async fn get_mcp_server_control(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
) -> Response {
    json_no_store(StatusCode::OK, state.mcp_server.control_status(&headers))
}

pub(crate) async fn update_mcp_server_control(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(request): Json<UpdateMcpServerControlRequest>,
) -> Response {
    let enabled = request
        .enabled
        .unwrap_or_else(|| state.mcp_server.is_enabled());
    let auth_enabled = request
        .auth_enabled
        .unwrap_or_else(|| state.mcp_server.auth_enabled());
    let configured_resource = match request.public_url {
        None => state.mcp_server.configured_public_url(),
        Some(None) => None,
        Some(Some(value)) => match normalize_public_mcp_url(value.as_str()) {
            Ok(value) => Some(value),
            Err(error) => return control_error(StatusCode::BAD_REQUEST, error),
        },
    };
    let configured_issuer = match request.oauth_issuer_url {
        None => state.mcp_server.configured_oauth_issuer_url(),
        Some(None) => None,
        Some(Some(value)) => match normalize_oauth_issuer_url(value.as_str()) {
            Ok(value) => Some(value),
            Err(error) => return control_error(StatusCode::BAD_REQUEST, error),
        },
    };
    if auth_enabled && enabled {
        let effective_resource = state
            .mcp_server
            .public_resource_with_configured(&headers, configured_resource.as_deref());
        if !is_https_resource(effective_resource.as_str()) {
            return control_error(
                StatusCode::BAD_REQUEST,
                "MCP OAuth requires a canonical HTTPS resource URL. Configure the Secure MCP Tunnel/public HTTPS URL before enabling Auth.".to_owned(),
            );
        }
        let effective_issuer = configured_issuer
            .clone()
            .or_else(|| state.mcp_server.configured_oauth_issuer_url())
            .unwrap_or_else(|| issuer_for_resource(effective_resource.as_str()));
        if !is_https_resource(effective_issuer.as_str()) {
            return control_error(
                StatusCode::BAD_REQUEST,
                "MCP OAuth requires a canonical HTTPS issuer URL. Configure the public OAuth issuer before enabling Auth.".to_owned(),
            );
        }
    }
    if let Err(error) = state
        .mcp_server
        .update_control(
            enabled,
            auth_enabled,
            configured_resource,
            configured_issuer,
        )
        .await
    {
        return control_error(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    json_no_store(StatusCode::OK, state.mcp_server.control_status(&headers))
}

pub(crate) async fn set_mcp_oauth_password(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
    Json(request): Json<SetMcpOAuthPasswordRequest>,
) -> Response {
    if let Err(error) = state
        .mcp_server
        .set_oauth_password(request.password.as_str())
        .await
    {
        let status = if error.contains("must not be empty") || error.contains("at most") {
            StatusCode::BAD_REQUEST
        } else {
            StatusCode::INTERNAL_SERVER_ERROR
        };
        return control_error(status, error);
    }
    json_no_store(StatusCode::OK, state.mcp_server.control_status(&headers))
}

pub(crate) async fn clear_mcp_oauth_password(
    State(state): State<Arc<crate::AppState>>,
    headers: HeaderMap,
) -> Response {
    if let Err(error) = state.mcp_server.clear_oauth_password().await {
        return control_error(StatusCode::INTERNAL_SERVER_ERROR, error);
    }
    json_no_store(StatusCode::OK, state.mcp_server.control_status(&headers))
}

fn control_error(status: StatusCode, error: String) -> Response {
    json_no_store(
        status,
        serde_json::json!({
            "error": error,
            "code": "mcp_server_control_error",
        }),
    )
}

fn mcp_auth_disabled() -> Response {
    let mut response = StatusCode::NOT_FOUND.into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

/// Build the MCP Streamable HTTP route and the optional OAuth surface.
///
/// The route table is fixed so the running server can switch modes without a
/// restart. OAuth handlers and middleware return/bypass at runtime according
/// to `auth_enabled`; in anonymous mode the OAuth URLs behave as not found and
/// `/mcp` accepts requests without a bearer token.
pub(crate) fn router(state: Arc<McpServerState>) -> Router {
    // ChatGPT and Secure MCP Tunnel may probe the endpoint before they have a
    // session ID (for example with server/discover or an early tools/list).
    // A stateless Streamable HTTP server is explicitly allowed by MCP and
    // avoids rmcp's stateful pre-initialize 422 for those requests. JSON
    // responses are also accepted by Streamable HTTP and are easier for
    // connector gateways to proxy than one-request SSE streams.
    let mut stream_config = StreamableHttpServerConfig::default()
        .with_stateful_mode(false)
        .with_json_response(true);
    // The public URL can be changed through the Web/TUI control plane while
    // this router is alive. Keep rmcp's host gate open and perform the actual
    // resource binding against the request-derived/configured URL instead of
    // freezing the old URL at startup.
    stream_config = stream_config.disable_allowed_hosts();

    let service = agena_mcp_server::streamable_http_service(
        ApplicationMcpBackend {
            state: Arc::clone(&state),
        },
        stream_config,
    );
    let protected_mcp = Router::new()
        .nest_service(MCP_PATH, service)
        // Keep a safe request/response breadcrumb at the MCP boundary. The
        // tunnel transports one JSON-RPC message per HTTP request, so a
        // method/status pair is enough to diagnose connector failures without
        // putting tool arguments, bearer tokens, or result data in logs.
        .layer(middleware::from_fn(trace_mcp_http_request))
        // Secure MCP Tunnel forwards each JSON-RPC request independently. A
        // few connector probes therefore arrive before (or without) the
        // lifecycle state that rmcp 2.2 expects in its legacy dispatcher.
        // Keep this compatibility path inside the bearer middleware and use
        // it only for the finite, stateless MCP messages ChatGPT needs.
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            compat_stateless_mcp,
        ))
        // The compatibility layer must be inside the bearer middleware: an
        // OpenAI discovery probe is still an MCP request and must not bypass
        // OAuth when Auth is enabled.
        .layer(middleware::from_fn(compat_openai_mcp_discover))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_mcp_bearer,
        ))
        // rmcp 2.2 does not yet model the Apps SDK's top-level
        // `securitySchemes` extension. Rewrite only the finite tools/list
        // response at the HTTP boundary; tool-call responses remain streamed.
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            rewrite_tool_security_schemes,
        ))
        // Do not let an implementation-specific rmcp 422 escape through the
        // tunnel.  Secure MCP Tunnel forwards JSON-RPC messages one at a
        // time, while older rmcp dispatchers can still produce HTTP 422 for a
        // message that is valid on a stateless endpoint.  The compatibility
        // middleware above handles the known message shapes; this outer
        // guard is deliberately a last-resort boundary for future rmcp
        // changes or connector probes we do not recognize yet.
        .layer(middleware::from_fn(normalize_mcp_http_422))
    .with_state(Arc::clone(&state));

    // Keep the OAuth surface truly disabled when Auth is off. The individual
    // handlers also check this flag, but an extractor (Json/Form/Query) runs
    // before a handler body. Without this route-level guard, malformed OAuth
    // requests could turn anonymous mode back into an HTTP 422 instead of the
    // promised empty 404.
    let oauth_routes = Router::new()
        .route(
            "/.well-known/oauth-protected-resource",
            axum::routing::get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-protected-resource/mcp",
            axum::routing::get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            axum::routing::get(authorization_server_metadata),
        )
        .route("/.well-known/jwks.json", axum::routing::get(jwks))
        .route("/oauth/jwks.json", axum::routing::get(jwks))
        .route("/oauth/register", axum::routing::post(register_client))
        .route(
            "/oauth/authorize",
            axum::routing::get(authorize_get).post(authorize_post),
        )
        .route("/oauth/token", axum::routing::post(token))
        .route("/oauth/revoke", axum::routing::post(revoke))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_mcp_auth_enabled,
        ))
        .with_state(Arc::clone(&state));

    Router::new()
        .merge(oauth_routes)
        .merge(protected_mcp)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_mcp_enabled,
        ))
        .with_state(state)
}

/// Log the MCP HTTP boundary without logging request bodies.
///
/// Secure MCP Tunnel reports only the upstream HTTP status when forwarding a
/// failure to ChatGPT. Keeping the JSON-RPC method and resulting status in the
/// Agena log makes that report actionable while avoiding tool arguments,
/// access tokens, and tool results. The request body is reconstructed before
/// it is passed downstream because the compatibility layers also need to read
/// the one-shot body.
async fn trace_mcp_http_request(
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    let http_method = request.method().clone();
    let path = request.uri().path().to_owned();
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let has_bearer = request.headers().contains_key(header::AUTHORIZATION);
    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            tracing::warn!(
                target: "agena::server::mcp",
                %http_method,
                %path,
                error = %error,
                "could not read MCP request body for boundary logging"
            );
            return next
                .run(axum::http::Request::from_parts(parts, Body::empty()))
                .await;
        }
    };
    let rpc_method = jsonrpc_method_from_bytes(&body);
    let has_id = jsonrpc_request_id_from_bytes(&body).is_some();
    let response = next
        .run(axum::http::Request::from_parts(parts, Body::from(body)))
        .await;
    let status = response.status();
    tracing::info!(
        target: "agena::server::mcp",
        %http_method,
        %path,
        rpc_method = rpc_method.as_deref().unwrap_or("<invalid-or-batch>"),
        has_id,
        has_bearer,
        %status,
        content_type = %content_type,
        "MCP HTTP request"
    );
    response
}

#[derive(Clone)]
struct ApplicationMcpBackend {
    state: Arc<McpServerState>,
}

#[async_trait]
impl McpServerBackend for ApplicationMcpBackend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        Ok(self
            .state
            .application
            .list_operator_tools()
            .await
            .into_iter()
            .filter(mcp_tool_is_exposed)
            .map(operator_tool_descriptor)
            .collect())
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        // Always re-read the catalog immediately before invocation. This
        // prevents a client from bypassing the list filter by guessing a
        // hidden tool name, and also tracks runtime/plugin reloads.
        let tools = self.state.application.list_operator_tools().await;
        if !tools
            .iter()
            .any(|tool| tool.name == params.name && mcp_tool_is_exposed(tool))
        {
            return Err(McpServerError::NotFound(format!(
                "tool '{}' is not exposed by the Agena MCP server",
                params.name
            )));
        }

        let result = self
            .state
            .application
            .invoke_operator_tool(
                self.state.workspace_id,
                params.name.as_str(),
                params.arguments,
                self.state.next_call_id(),
            )
            .await;
        match result {
            Ok(summary) => {
                let text = if summary.output_text.is_empty() {
                    serde_json::to_string_pretty(&summary.payload)
                        .unwrap_or_else(|_| "<empty output>".to_owned())
                } else {
                    summary.output_text
                };
                Ok(agena_mcp_server::text_result(text))
            }
            Err(error) => Ok(agena_mcp_server::text_error(error.to_string())),
        }
    }
}

fn operator_tool_descriptor(tool: OperatorToolResource) -> ToolDescriptor {
    ToolDescriptor {
        name: tool.name,
        title: None,
        aliases: Vec::new(),
        description: tool.summary,
        before_help: tool.before_help,
        after_help: tool.after_help,
        input_schema: Some(tool.input_schema),
        output_schema: None,
        annotations: None,
        execution: None,
        icons: Vec::new(),
        meta: None,
    }
}

/// Add the per-tool OAuth declaration required by ChatGPT's connector UI.
///
/// `rmcp` currently serializes the standard MCP `Tool` model, which has no
/// top-level `securitySchemes` field. The Apps SDK extension is nevertheless
/// a top-level property on each tool descriptor, so the HTTP adapter promotes
/// it after rmcp has serialized the response. Keeping this transformation
/// here also prevents the OAuth declaration from leaking into Agena's stdio
/// MCP surface, where no HTTP OAuth resource server exists.
fn add_tool_security_schemes(
    mut payload: serde_json::Value,
    auth_enabled: bool,
) -> Option<serde_json::Value> {
    let tools = payload
        .get_mut("result")
        .and_then(serde_json::Value::as_object_mut)
        .and_then(|result| result.get_mut("tools"))
        .and_then(serde_json::Value::as_array_mut)?;

    let security_schemes = if auth_enabled {
        serde_json::json!([{
            "type": "oauth2",
            "scopes": [MCP_SCOPE],
        }])
    } else {
        serde_json::json!([{"type": "noauth"}])
    };
    for tool in tools {
        let tool = tool.as_object_mut()?;
        tool.insert("securitySchemes".to_owned(), security_schemes.clone());
    }
    Some(payload)
}

fn is_tools_list_request(body: &[u8]) -> bool {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| {
            value
                .get("method")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned)
        })
        .as_deref()
        == Some("tools/list")
}

/// Return the MCP 2026-07-28 discovery result used by ChatGPT's pre-handshake
/// probe.
///
/// `server/discover` is not part of the legacy initialize-based MCP revisions
/// implemented by rmcp 2.2. The current MCP revision nevertheless requires a
/// server to advertise its versions and capabilities through this method. Keep
/// the response at the HTTP boundary so the stateless tunnel path does not
/// create an rmcp session, while still returning the complete modern result
/// shape (`resultType`, cache hints, and server information in `_meta`).
fn mcp_discover_result() -> serde_json::Value {
    serde_json::json!({
        "resultType": "complete",
        "supportedVersions": MCP_DISCOVERY_SUPPORTED_VERSIONS,
        "capabilities": {
            "tools": {},
        },
        "_meta": {
            "io.modelcontextprotocol/serverInfo": {
                "name": "agena",
                "version": env!("CARGO_PKG_VERSION"),
            },
        },
        "instructions": "Agena exposes its local runtime tools over MCP. Interactive, browser, planning-review, and provider/plugin tools are intentionally unavailable on this endpoint.",
        "ttlMs": MCP_DISCOVERY_TTL_MS,
        "cacheScope": "public",
    })
}

/// Return the discovery response for ChatGPT's pre-handshake probe.
///
/// This is deliberately kept at the HTTP boundary. It is not exposed through
/// the stdio MCP server, and it does not create an rmcp session.
fn openai_mcp_discover_payload(body: &[u8]) -> Option<serde_json::Value> {
    let request = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let response = |request: &serde_json::Value| {
        if request.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0")
            || request.get("method").and_then(serde_json::Value::as_str)
                != Some(OPENAI_MCP_DISCOVER_METHOD)
        {
            return None;
        }
        let id = request.get("id").cloned().filter(|id| !id.is_null())?;
        Some(serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": mcp_discover_result(),
        }))
    };

    match request {
        serde_json::Value::Object(_) => response(&request),
        // Some MCP hosts send the preflight as a JSON-RPC batch. Only
        // intercept an all-discover batch; mixed batches must continue to the
        // normal rmcp parser so we never silently drop another request.
        serde_json::Value::Array(requests) if !requests.is_empty() => {
            let responses = requests.iter().map(response).collect::<Option<Vec<_>>>()?;
            Some(serde_json::Value::Array(responses))
        }
        _ => None,
    }
}

async fn compat_openai_mcp_discover(
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid MCP request body: {error}"),
            )
                .into_response();
        }
    };

    if let Some(payload) = openai_mcp_discover_payload(&body) {
        return json_no_store(StatusCode::OK, payload);
    }

    next.run(axum::http::Request::from_parts(parts, Body::from(body)))
        .await
}

/// Handle the small stateless MCP subset used by ChatGPT and Secure MCP
/// Tunnel before handing the request to rmcp.
///
/// `rmcp`'s Streamable HTTP implementation is the canonical implementation
/// for normal MCP traffic. Its 2.2 legacy dispatcher can nevertheless answer
/// with HTTP 422 when a request that is valid for a stateless tunnel arrives
/// without an initialized session. The tunnel contract forwards individual
/// JSON-RPC requests, so handling these messages at the HTTP boundary keeps
/// the public connector surface independent of that session bookkeeping.
///
/// This middleware deliberately runs after `require_mcp_bearer` in the
/// middleware stack. It never authenticates a request. The fallback below is
/// defensive only: a valid JSON-RPC request should always be handled here,
/// but if a future change lets one reach rmcp's legacy dispatcher, convert its
/// HTTP 422 into a JSON-RPC response so a Secure MCP Tunnel never exposes the
/// rmcp-specific session error to ChatGPT.
async fn compat_stateless_mcp(
    State(state): State<Arc<McpServerState>>,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid MCP request body: {error}"),
            )
                .into_response();
        }
    };
    let value = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(value) => value,
        Err(error) => {
            // A tunnel request is already one complete JSON-RPC message. Do
            // not pass malformed JSON to rmcp's transport parser: its legacy
            // error boundary can turn an otherwise recoverable connector
            // failure into HTTP 422 ("expect initialize"). A JSON-RPC parse
            // error is the protocol-level response and keeps the HTTP
            // transport usable for the next independent tunnel request.
            tracing::warn!(
                error = %error,
                "received malformed JSON-RPC message on stateless MCP endpoint"
            );
            return json_no_store(
                StatusCode::OK,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "id": null,
                    "error": {
                        "code": -32700,
                        "message": "Parse error"
                    }
                }),
            );
        }
    };
    let request_id = jsonrpc_request_id(&value);

    let payload = match compat_stateless_mcp_payload(&state, value).await {
        Ok(Some(payload)) => payload,
        Ok(None) => {
            let response = next
                .run(axum::http::Request::from_parts(parts, Body::from(body)))
                .await;
            return normalize_mcp_unprocessable_response(response, request_id);
        }
        Err(error) => {
            return json_no_store(StatusCode::OK, compat_jsonrpc_error(request_id, error));
        }
    };

    if payload.is_null() {
        // JSON-RPC notifications have no response body. 202 is the
        // Streamable HTTP acknowledgement used by rmcp for this case.
        return StatusCode::ACCEPTED.into_response();
    }
    json_no_store(StatusCode::OK, payload)
}

/// Convert an implementation-specific HTTP 422 from the MCP service into a
/// JSON-RPC error.  ChatGPT and Secure MCP Tunnel communicate in JSON-RPC;
/// exposing rmcp's transport-level 422 makes the connector report that the
/// target server failed even though the target is reachable.
///
/// This is intentionally the outermost MCP-only guard.  It does not change
/// authentication failures (401), disabled routes (404), content negotiation
/// failures (406/415), or application errors.  The request id is copied from
/// the bounded JSON body when possible, and the body is always reconstructed
/// before the downstream service sees it.
async fn normalize_mcp_http_422(
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    let (parts, body) = request.into_parts();
    let (request_id, body) = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => (jsonrpc_request_id_from_bytes(&body), Body::from(body)),
        Err(error) => {
            tracing::warn!(
                error = %error,
                "could not buffer MCP request before the HTTP 422 compatibility guard"
            );
            (None, Body::empty())
        }
    };
    let method = parts.method.clone();
    let uri = parts.uri.path().to_owned();
    let response = next.run(axum::http::Request::from_parts(parts, body)).await;
    if response.status() != StatusCode::UNPROCESSABLE_ENTITY {
        return response;
    }

    tracing::warn!(
        %method,
        %uri,
        "converted an unexpected MCP HTTP 422 into a JSON-RPC error"
    );
    json_no_store(
        StatusCode::OK,
        compat_jsonrpc_error(
            request_id,
            McpServerError::InvalidParams(
                "MCP request could not be processed by the stateless endpoint".to_owned(),
            ),
        ),
    )
}

fn jsonrpc_request_id_from_bytes(body: &[u8]) -> Option<serde_json::Value> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| jsonrpc_request_id(&value))
}

fn jsonrpc_method_from_bytes(body: &[u8]) -> Option<String> {
    serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|value| jsonrpc_method(&value))
}

fn jsonrpc_method(value: &serde_json::Value) -> Option<String> {
    match value {
        serde_json::Value::Object(object) => object
            .get("method")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
        serde_json::Value::Array(messages) => messages.first().and_then(jsonrpc_method),
        _ => None,
    }
}

fn jsonrpc_request_id(value: &serde_json::Value) -> Option<serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => object.get("id").cloned().filter(|id| !id.is_null()),
        serde_json::Value::Array(messages) => messages.first().and_then(jsonrpc_request_id),
        _ => None,
    }
}

fn normalize_mcp_unprocessable_response(
    response: Response,
    id: Option<serde_json::Value>,
) -> Response {
    if response.status() != StatusCode::UNPROCESSABLE_ENTITY {
        return response;
    }

    tracing::warn!(
        "converted legacy rmcp HTTP 422 response into a JSON-RPC error for stateless MCP"
    );
    json_no_store(
        StatusCode::OK,
        compat_jsonrpc_error(
            id,
            McpServerError::InvalidParams(
                "MCP request could not be processed as a stateless JSON-RPC message".to_owned(),
            ),
        ),
    )
}

async fn compat_stateless_mcp_payload(
    state: &McpServerState,
    value: serde_json::Value,
) -> Result<Option<serde_json::Value>, McpServerError> {
    match value {
        serde_json::Value::Object(_) => compat_stateless_mcp_message(state, &value).await,
        serde_json::Value::Array(requests) if !requests.is_empty() => {
            let mut responses = Vec::with_capacity(requests.len());
            for request in requests {
                let response = compat_stateless_mcp_message(state, &request)
                    .await?
                    .unwrap_or(serde_json::Value::Null);
                if !response.is_null() {
                    responses.push(response);
                }
            }
            if responses.is_empty() {
                Ok(Some(serde_json::Value::Null))
            } else {
                Ok(Some(serde_json::Value::Array(responses)))
            }
        }
        serde_json::Value::Array(_) => Ok(Some(compat_jsonrpc_error(
            None,
            McpServerError::InvalidParams(
                "JSON-RPC batch must contain at least one message".to_owned(),
            ),
        ))),
        _ => Ok(Some(compat_jsonrpc_error(
            None,
            McpServerError::InvalidParams("JSON-RPC message must be an object".to_owned()),
        ))),
    }
}

async fn compat_stateless_mcp_message(
    state: &McpServerState,
    request: &serde_json::Value,
) -> Result<Option<serde_json::Value>, McpServerError> {
    let Some(method) = request.get("method").and_then(serde_json::Value::as_str) else {
        // A response is not expected on the server's inbound endpoint, but it
        // is still a valid JSON-RPC message. A stateless tunnel must consume
        // it without handing it to rmcp's session dispatcher. Invalid message
        // shapes get a normal JSON-RPC error rather than an HTTP 422.
        if request.get("result").is_some() || request.get("error").is_some() {
            return Ok(Some(serde_json::Value::Null));
        }
        return Ok(Some(compat_jsonrpc_error(
            request.get("id").cloned().filter(|id| !id.is_null()),
            McpServerError::InvalidParams(
                "JSON-RPC request must contain a string method".to_owned(),
            ),
        )));
    };
    let id = request.get("id").cloned().filter(|id| !id.is_null());
    let response = |result: serde_json::Value| {
        id.clone().map(|id| {
            serde_json::json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            })
        })
    };

    match method {
        "server/discover" => Ok(response(mcp_discover_result()).or(Some(serde_json::Value::Null))),
        method if method.starts_with("notifications/") => Ok(Some(serde_json::Value::Null)),
        "ping" => Ok(response(serde_json::json!({})).or(Some(serde_json::Value::Null))),
        "initialize" => {
            Ok(response(compat_initialize_result(request)).or(Some(serde_json::Value::Null)))
        }
        "tools/list" => {
            let backend = ApplicationMcpBackend {
                state: Arc::new(state.clone()),
            };
            let tools = backend.list_tools().await?;
            let tools = tools
                .into_iter()
                .map(serialize_tool_descriptor)
                .collect::<Result<Vec<_>, _>>()?;
            let tools = serde_json::to_value(tools)?;
            let payload = response(serde_json::json!({"tools": tools}));
            let Some(payload) = payload else {
                return Ok(Some(serde_json::Value::Null));
            };
            let payload = add_tool_security_schemes(payload, state.auth_enabled())
                .unwrap_or_else(|| serde_json::json!({}));
            Ok(Some(payload))
        }
        "tools/call" => {
            let params = request
                .get("params")
                .and_then(serde_json::Value::as_object)
                .ok_or_else(|| {
                    McpServerError::InvalidParams("tools/call params must be an object".to_owned())
                })?;
            let name = params
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|name| !name.trim().is_empty())
                .ok_or_else(|| {
                    McpServerError::InvalidParams(
                        "tools/call requires a non-empty string name".to_owned(),
                    )
                })?;
            let backend = ApplicationMcpBackend {
                state: Arc::new(state.clone()),
            };
            let result = backend
                .call_tool(CallToolParams {
                    name: name.to_owned(),
                    arguments: params.get("arguments").cloned(),
                })
                .await?;
            let result = serialize_call_tool_result(result)?;
            Ok(response(result).or(Some(serde_json::Value::Null)))
        }
        // Agena intentionally exposes only tools on this endpoint. Keep the
        // response HTTP-successful and report unsupported MCP methods at the
        // JSON-RPC layer. Passing them to rmcp would re-enter its legacy
        // lifecycle dispatcher and can produce HTTP 422 when the tunnel has
        // delivered requests independently.
        _ => Ok(Some(if id.is_some() {
            compat_jsonrpc_error(id.clone(), McpServerError::NotFound(method.to_owned()))
        } else {
            serde_json::Value::Null
        })),
    }
}

fn compat_initialize_result(request: &serde_json::Value) -> serde_json::Value {
    let protocol_version = request
        .get("params")
        .and_then(|params| params.get("protocolVersion"))
        .and_then(serde_json::Value::as_str)
        .filter(|version| {
            matches!(
                *version,
                "2025-03-26" | "2025-06-18" | "2025-11-25" | "2026-06-18"
            )
        })
        .unwrap_or(DEFAULT_MCP_PROTOCOL_VERSION);
    serde_json::json!({
        "protocolVersion": protocol_version,
        "capabilities": {"tools": {}},
        "serverInfo": {
            "name": "agena",
            "version": env!("CARGO_PKG_VERSION"),
        },
        "instructions": "Agena exposes its local runtime tools over MCP. Resources and prompts are intentionally unavailable on this endpoint."
    })
}

fn compat_jsonrpc_error(id: Option<serde_json::Value>, error: McpServerError) -> serde_json::Value {
    let code = match &error {
        McpServerError::InvalidParams(_) => -32602,
        McpServerError::NotFound(_) => -32601,
        _ => -32603,
    };
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "error": {
            "code": code,
            "message": error.to_string(),
        },
    })
}

fn rewrite_tool_list_json(body: &[u8], auth_enabled: bool) -> Option<Vec<u8>> {
    let payload = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    let payload = add_tool_security_schemes(payload, auth_enabled)?;
    serde_json::to_vec(&payload).ok()
}

fn rewrite_tool_list_sse(body: &[u8], auth_enabled: bool) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(body).ok()?;
    let mut changed = false;
    let mut output = String::with_capacity(text.len());

    for line in text.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let line_without_newline = line_without_newline
            .strip_suffix('\r')
            .unwrap_or(line_without_newline);
        if let Some(data) = line_without_newline.strip_prefix("data:")
            && let Ok(payload) = serde_json::from_str::<serde_json::Value>(data.trim_start())
            && let Some(payload) = add_tool_security_schemes(payload, auth_enabled)
        {
            let newline = &line[line_without_newline.len()..];
            output.push_str("data: ");
            output.push_str(serde_json::to_string(&payload).ok()?.as_str());
            output.push_str(newline);
            changed = true;
            continue;
        }
        output.push_str(line);
    }

    // `split_inclusive` also returns a final non-newline-terminated line, so
    // the loop above covers both normal SSE responses and test fixtures.
    changed.then_some(output.into_bytes())
}

async fn rewrite_tool_security_schemes(
    State(state): State<Arc<McpServerState>>,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }

    let (parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("invalid MCP request body: {error}"),
            )
                .into_response();
        }
    };
    let should_rewrite = is_tools_list_request(&body);
    let auth_enabled = state.auth_enabled();
    let request = axum::http::Request::from_parts(parts, Body::from(body));
    let response = next.run(request).await;
    if should_rewrite {
        rewrite_tool_list_response(response, auth_enabled).await
    } else {
        response
    }
}

async fn rewrite_tool_list_response(response: Response, auth_enabled: bool) -> Response {
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let is_json = content_type.contains("application/json");
    let is_sse = content_type.contains("text/event-stream");
    if !is_json && !is_sse {
        return response;
    }

    let (parts, body) = response.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(_) => return Response::from_parts(parts, Body::empty()),
    };
    let rewritten = if is_json {
        rewrite_tool_list_json(&body, auth_enabled)
    } else {
        rewrite_tool_list_sse(&body, auth_enabled)
    };
    let Some(rewritten) = rewritten else {
        return Response::from_parts(parts, Body::from(body));
    };

    let mut response = Response::from_parts(parts, Body::from(rewritten));
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
}

fn name_belongs_to_plugin(name: &str, plugin_id: &str) -> bool {
    name == plugin_id
        || name
            .strip_prefix(plugin_id)
            .is_some_and(|suffix| suffix.starts_with('.'))
}

fn mcp_tool_uses_hidden_provider(tool: &OperatorToolResource) -> bool {
    if let Some(plugin_id) = tool.plugin_id.as_deref() {
        return HIDDEN_MCP_PLUGIN_IDS.contains(&plugin_id);
    }

    HIDDEN_MCP_PLUGIN_IDS.iter().any(|plugin_id| {
        let compact_plugin_id = plugin_id.strip_prefix("agena.").unwrap_or(plugin_id);
        name_belongs_to_plugin(tool.name.as_str(), compact_plugin_id)
            || name_belongs_to_plugin(tool.name.as_str(), plugin_id)
    })
}

fn mcp_tool_is_exposed(tool: &OperatorToolResource) -> bool {
    !tool.interactive
        && !mcp_tool_uses_hidden_provider(tool)
        && !mcp_tool_has_known_interactive_name(tool.name.as_str())
}

fn mcp_tool_has_known_interactive_name(name: &str) -> bool {
    KNOWN_INTERACTIVE_MCP_TOOL_NAMES.contains(&name)
        || KNOWN_INTERACTIVE_MCP_TOOL_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        // Plan phase can conditionally open the same approval dialog as
        // plan.review, depending on the request body. A non-interactive MCP
        // client cannot safely offer either entry point.
        || matches!(name, "plan.phase" | "plan.review")
}

// ---------------------------------------------------------------------------
// OAuth resource metadata and authorization server metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
struct ProtectedResourceMetadata {
    resource: String,
    authorization_servers: Vec<String>,
    scopes_supported: Vec<String>,
    bearer_methods_supported: Vec<String>,
}

#[derive(Debug, Serialize)]
struct AuthorizationServerMetadata {
    issuer: String,
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: String,
    revocation_endpoint: String,
    client_id_metadata_document_supported: bool,
    token_endpoint_auth_methods_supported: Vec<String>,
    code_challenge_methods_supported: Vec<String>,
    scopes_supported: Vec<String>,
    response_types_supported: Vec<String>,
    grant_types_supported: Vec<String>,
    jwks_uri: String,
}

async fn protected_resource_metadata(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let resource = state.public_resource(&headers);
    let issuer = state.issuer_for_headers(&headers);
    json_no_store(
        StatusCode::OK,
        ProtectedResourceMetadata {
            authorization_servers: vec![issuer],
            resource,
            scopes_supported: vec![MCP_SCOPE.to_owned()],
            bearer_methods_supported: vec!["header".to_owned()],
        },
    )
}

async fn authorization_server_metadata(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let issuer = state.issuer_for_headers(&headers);
    json_no_store(
        StatusCode::OK,
        AuthorizationServerMetadata {
            authorization_endpoint: append_public_endpoint(&issuer, "/oauth/authorize"),
            client_id_metadata_document_supported: true,
            code_challenge_methods_supported: vec!["S256".to_owned()],
            grant_types_supported: vec![
                "authorization_code".to_owned(),
                "refresh_token".to_owned(),
            ],
            issuer: issuer.clone(),
            registration_endpoint: append_public_endpoint(&issuer, "/oauth/register"),
            response_types_supported: vec!["code".to_owned()],
            revocation_endpoint: append_public_endpoint(&issuer, "/oauth/revoke"),
            scopes_supported: vec![MCP_SCOPE.to_owned()],
            token_endpoint: append_public_endpoint(&issuer, "/oauth/token"),
            // Keep discovery truthful. Advertising private_key_jwt before
            // validating client assertions makes ChatGPT select a flow the
            // server cannot actually complete.
            token_endpoint_auth_methods_supported: vec!["none".to_owned()],
            jwks_uri: append_public_endpoint(&issuer, "/oauth/jwks.json"),
        },
    )
}

async fn jwks(State(state): State<Arc<McpServerState>>, _headers: HeaderMap) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    json_no_store(StatusCode::OK, state.oauth.signing_key.jwks())
}

#[derive(Debug, Deserialize)]
struct ClientRegistrationRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
    #[serde(default)]
    jwks_uri: Option<String>,
    #[serde(default)]
    jwks: Option<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct ClientRegistrationResponse {
    client_id: String,
    client_id_issued_at: u64,
    client_secret_expires_at: u64,
    token_endpoint_auth_method: &'static str,
    redirect_uris: Vec<String>,
    grant_types: [&'static str; 2],
    response_types: [&'static str; 1],
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct RegisteredClient {
    redirect_uris: Vec<String>,
    client_name: Option<String>,
    token_endpoint_auth_method: String,
    jwks_uri: Option<String>,
    jwks: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Deserialize)]
struct CimdClientMetadata {
    client_id: Option<String>,
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    token_endpoint_auth_methods_supported: Option<Vec<String>>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
}

#[derive(Debug, Clone)]
struct CachedCimdClient {
    metadata: CimdClientMetadata,
    expires_at: Instant,
}

fn client_metadata_supports_none(singular: Option<&str>, plural: Option<&[String]>) -> bool {
    if let Some(methods) = plural {
        return methods.iter().any(|method| method == "none");
    }
    singular == Some("none")
}

async fn register_client(
    State(state): State<Arc<McpServerState>>,
    request: Result<Json<ClientRegistrationRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let Json(request) = match request {
        Ok(request) => request,
        Err(_) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "the client registration body must be a valid JSON object",
            );
        }
    };
    if request.redirect_uris.is_empty() || request.redirect_uris.len() > 16 {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "redirect_uris must contain between one and sixteen URIs",
        );
    }
    if !client_metadata_supports_none(
        request.token_endpoint_auth_method.as_deref(),
        request.token_endpoint_auth_methods_supported.as_deref(),
    ) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "the client must support token endpoint authentication method none",
        );
    }
    if request.grant_types.as_deref().is_some_and(|values| {
        !values.is_empty() && !values.iter().any(|value| value == "authorization_code")
    }) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "authorization_code grant type is required",
        );
    }
    if request
        .response_types
        .as_deref()
        .is_some_and(|values| !values.is_empty() && !values.iter().any(|value| value == "code"))
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "code response type is required",
        );
    }
    if request
        .redirect_uris
        .iter()
        .any(|redirect| validate_redirect_uri(redirect).is_err())
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris must be absolute HTTPS URLs without fragments",
        );
    }

    let client_id = format!("agena_client_{}", crate::server::issue_token());
    let client = RegisteredClient {
        redirect_uris: request.redirect_uris.clone(),
        client_name: request.client_name.clone(),
        token_endpoint_auth_method: "none".to_owned(),
        jwks_uri: request.jwks_uri.clone(),
        jwks: request.jwks.clone(),
    };
    state.oauth.clients.insert(client_id.clone(), client);
    if let Err(error) = state.oauth.persist().await {
        state.oauth.clients.remove(client_id.as_str());
        tracing::error!(error = %error, "failed to persist registered MCP OAuth client");
        return oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "the OAuth client could not be persisted",
        );
    }

    let response = ClientRegistrationResponse {
        client_id,
        client_id_issued_at: unix_timestamp(),
        client_secret_expires_at: 0,
        token_endpoint_auth_method: "none",
        redirect_uris: request.redirect_uris,
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
        client_name: request.client_name,
    };
    json_no_store(StatusCode::CREATED, response)
}

async fn fetch_cimd_document(client_id: &str) -> Result<CimdClientMetadata, ()> {
    if !is_supported_cimd_client_id(client_id) {
        return Err(());
    }

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|_| ())?;
    let response = client
        .get(client_id)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| ())?;
    if !response.status().is_success() {
        return Err(());
    }
    if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            let media_type = value.split(';').next().unwrap_or_default().trim();
            !media_type.eq_ignore_ascii_case("application/json") && !media_type.ends_with("+json")
        })
    {
        return Err(());
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CIMD_DOCUMENT_BYTES as u64)
    {
        return Err(());
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|_| ())?;
        if body.len().saturating_add(chunk.len()) > MAX_CIMD_DOCUMENT_BYTES {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<CimdClientMetadata>(&body).map_err(|_| ())
}

fn validate_cimd_document(client_id: &str, metadata: &CimdClientMetadata) -> Result<(), ()> {
    if metadata.client_id.as_deref() != Some(client_id)
        || metadata.redirect_uris.is_empty()
        || metadata
            .redirect_uris
            .iter()
            .any(|redirect_uri| validate_redirect_uri(redirect_uri).is_err())
        || !client_metadata_supports_none(
            metadata.token_endpoint_auth_method.as_deref(),
            metadata.token_endpoint_auth_methods_supported.as_deref(),
        )
        || metadata.grant_types.as_deref().is_some_and(|values| {
            !values.is_empty() && !values.iter().any(|value| value == "authorization_code")
        })
        || metadata
            .response_types
            .as_deref()
            .is_some_and(|values| !values.is_empty() && !values.iter().any(|value| value == "code"))
    {
        return Err(());
    }
    Ok(())
}

async fn load_cimd_client(
    state: &McpServerState,
    client_id: &str,
) -> Result<CimdClientMetadata, ()> {
    if !is_supported_cimd_client_id(client_id) {
        return Err(());
    }
    let now = Instant::now();
    state
        .oauth
        .cimd_clients
        .retain(|_, cached| cached.expires_at > now);
    if let Some(cached) = state.oauth.cimd_clients.get(client_id) {
        return Ok(cached.metadata.clone());
    }

    let metadata = fetch_cimd_document(client_id).await?;
    validate_cimd_document(client_id, &metadata)?;
    state.oauth.cimd_clients.insert(
        client_id.to_owned(),
        CachedCimdClient {
            metadata: metadata.clone(),
            expires_at: Instant::now() + CIMD_CACHE_TTL,
        },
    );
    Ok(metadata)
}

// ---------------------------------------------------------------------------
// Authorization code + PKCE
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Deserialize)]
struct AuthorizeRequest {
    #[serde(default)]
    response_type: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    state: Option<String>,
    #[serde(default)]
    code_challenge: Option<String>,
    #[serde(default)]
    code_challenge_method: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

#[derive(Debug, Clone)]
struct ValidatedAuthorizationRequest {
    client_id: String,
    client_name: Option<String>,
    redirect_uri: String,
    state: String,
    code_challenge: String,
    issuer: String,
    resource: String,
    scope: String,
}

async fn validate_authorization_request(
    state: &McpServerState,
    headers: &HeaderMap,
    request: &AuthorizeRequest,
) -> Result<ValidatedAuthorizationRequest, OAuthRequestError> {
    if request.response_type.as_deref() != Some("code") {
        return Err(OAuthRequestError::new(
            "unsupported_response_type",
            "response_type=code is required",
        ));
    }
    let client_id = request
        .client_id
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| OAuthRequestError::new("invalid_request", "client_id is required"))?;
    let registered_client = state
        .oauth
        .clients
        .get(client_id)
        .map(|entry| entry.clone());
    let cimd_client = if registered_client.is_none() {
        Some(load_cimd_client(state, client_id).await.map_err(|_| {
            OAuthRequestError::new(
                "invalid_client",
                "the CIMD client metadata document could not be validated",
            )
        })?)
    } else {
        None
    };
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| OAuthRequestError::new("invalid_request", "redirect_uri is required"))?;
    let redirect_matches_client = registered_client
        .as_ref()
        .is_some_and(|client| client.redirect_uris.iter().any(|uri| uri == redirect_uri))
        || cimd_client
            .as_ref()
            .is_some_and(|client| client.redirect_uris.iter().any(|uri| uri == redirect_uri));
    if !redirect_matches_client {
        return Err(OAuthRequestError::new(
            "invalid_request",
            "redirect_uri does not match the registered client metadata",
        ));
    }
    let state_value = request
        .state
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthRequestError::new("invalid_request", "state is required"))?;
    let code_challenge = request
        .code_challenge
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthRequestError::new("invalid_request", "code_challenge is required"))?;
    if request.code_challenge_method.as_deref() != Some("S256") {
        return Err(OAuthRequestError::new(
            "invalid_request",
            "code_challenge_method=S256 is required",
        ));
    }
    let resource = request
        .resource
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthRequestError::new("invalid_request", "resource is required"))?;
    let expected_resource = state.public_resource(headers);
    if resource != expected_resource {
        return Err(OAuthRequestError::new(
            "invalid_target",
            "resource does not match the MCP resource metadata",
        ));
    }
    let scope = normalize_scope(request.scope.as_deref())?;
    let issuer = state.issuer_for_headers(headers);
    Ok(ValidatedAuthorizationRequest {
        client_id: client_id.to_owned(),
        client_name: registered_client
            .and_then(|client| client.client_name)
            .or_else(|| cimd_client.and_then(|client| client.client_name))
            .or_else(|| Some("ChatGPT".to_owned())),
        redirect_uri: redirect_uri.to_owned(),
        state: state_value.to_owned(),
        code_challenge: code_challenge.to_owned(),
        issuer,
        resource: resource.to_owned(),
        scope,
    })
}

async fn authorize_get(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    Query(request): Query<AuthorizeRequest>,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    match validate_authorization_request(&state, &headers, &request).await {
        Ok(request) => Html(render_authorization_page(&request, None)).into_response(),
        Err(error) => error.into_response(),
    }
}

async fn authorize_post(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    request: Result<Form<AuthorizeRequest>, axum::extract::rejection::FormRejection>,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let Form(request) = match request {
        Ok(request) => request,
        Err(_) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the authorization request body must be a valid form",
            );
        }
    };
    let validated = match validate_authorization_request(&state, &headers, &request).await {
        Ok(request) => request,
        Err(error) => return error.into_response(),
    };
    if let Err(error) = state
        .verify_oauth_password(request.password.as_deref().unwrap_or(""), &headers)
        .await
    {
        let (status, message) = match error {
            OAuthPasswordError::NotConfigured => (
                StatusCode::SERVICE_UNAVAILABLE,
                "The Agena MCP OAuth password is not configured. Set it in the Agena Web/TUI control plane, or set --ui-password / AGENA_SERVER_UI_PASSWORD before using ChatGPT OAuth.",
            ),
            OAuthPasswordError::Invalid => (
                StatusCode::UNAUTHORIZED,
                "The server password is incorrect.",
            ),
            OAuthPasswordError::Locked(_seconds) => (
                StatusCode::TOO_MANY_REQUESTS,
                "Too many failed password attempts; try again after the lockout expires.",
            ),
        };
        let mut response =
            Html(render_authorization_page(&validated, Some(message))).into_response();
        *response.status_mut() = status;
        if let OAuthPasswordError::Locked(seconds) = error
            && let Ok(value) = HeaderValue::try_from(seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }

    let code = crate::server::issue_token();
    state.oauth.authorization_codes.insert(
        code.clone(),
        AuthorizationCodeRecord {
            client_id: validated.client_id,
            redirect_uri: validated.redirect_uri.clone(),
            issuer: validated.issuer,
            resource: validated.resource,
            scope: validated.scope,
            code_challenge: validated.code_challenge,
            expires_at: Instant::now() + AUTHORIZATION_CODE_TTL,
        },
    );

    let mut redirect = match Url::parse(validated.redirect_uri.as_str()) {
        Ok(redirect) => redirect,
        Err(_) => {
            return oauth_error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "server_error",
                "the registered redirect URI could not be rebuilt",
            );
        }
    };
    redirect
        .query_pairs_mut()
        .append_pair("code", code.as_str())
        .append_pair("state", validated.state.as_str());
    // The authorization form is submitted with POST. A 303 explicitly tells
    // ChatGPT (and a browser) to follow the callback with GET instead of
    // replaying the password-bearing POST as a 307 would do.
    Redirect::to(redirect.as_str()).into_response()
}

fn render_authorization_page(
    request: &ValidatedAuthorizationRequest,
    error: Option<&str>,
) -> String {
    let hidden = |name: &str, value: &str| {
        format!(
            "<input type=\"hidden\" name=\"{}\" value=\"{}\">",
            html_escape(name),
            html_escape(value)
        )
    };
    let error_html = error
        .map(|message| format!("<p class=\"error\">{}</p>", html_escape(message)))
        .unwrap_or_default();
    let client_label = request
        .client_name
        .as_deref()
        .filter(|name| !name.trim().is_empty())
        .unwrap_or("an MCP client");
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Authorize Agena MCP</title><style>body{{font-family:system-ui,sans-serif;max-width: thirtyrem;max-width:30rem;margin:4rem auto;padding:0 1.25rem;color:#202123}}main{{border:1px solid #ddd;border-radius:12px;padding:2rem;box-shadow:0 4px 24px #00000012}}input[type=password]{{display:block;width:100%;box-sizing:border-box;padding:.7rem;border:1px solid #aaa;border-radius:6px;margin:.5rem 0 1rem}}button{{padding:.7rem 1.1rem;border:0;border-radius:6px;background:#111;color:white;cursor:pointer}}.error{{color:#b42318;background:#fff1f0;padding:.7rem;border-radius:6px}}</style></head><body><main><h1>Authorize Agena MCP</h1><p><strong>{}</strong> is requesting access to Agena tools.</p>{}<form method=\"post\" action=\"authorize\">{}{}{}{}{}{}{}{}<label for=\"password\">Agena server password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required autofocus><button type=\"submit\">Authorize</button></form></main></body></html>",
        html_escape(client_label),
        error_html,
        hidden("response_type", "code"),
        hidden("client_id", request.client_id.as_str()),
        hidden("redirect_uri", request.redirect_uri.as_str()),
        hidden("state", request.state.as_str()),
        hidden("code_challenge", request.code_challenge.as_str()),
        hidden("code_challenge_method", "S256"),
        hidden("resource", request.resource.as_str()),
        hidden("scope", request.scope.as_str()),
    )
}

#[derive(Debug, Clone)]
struct AuthorizationCodeRecord {
    client_id: String,
    redirect_uri: String,
    issuer: String,
    resource: String,
    scope: String,
    code_challenge: String,
    expires_at: Instant,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct RefreshTokenRecord {
    client_id: String,
    resource: String,
    scope: String,
    expires_at: u64,
    family_id: String,
    used: bool,
    revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct PersistedOAuthRuntime {
    version: u32,
    #[serde(default)]
    clients: HashMap<String, RegisteredClient>,
    #[serde(default)]
    refresh_tokens: HashMap<String, RefreshTokenRecord>,
    #[serde(default)]
    revoked_jti: HashMap<String, u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct AccessTokenClaims {
    iss: String,
    sub: String,
    aud: String,
    resource: String,
    scope: String,
    iat: u64,
    nbf: u64,
    exp: u64,
    jti: String,
}

struct OAuthState {
    clients: DashMap<String, RegisteredClient>,
    cimd_clients: DashMap<String, CachedCimdClient>,
    authorization_codes: DashMap<String, AuthorizationCodeRecord>,
    refresh_tokens: DashMap<String, RefreshTokenRecord>,
    revoked_jti: DashMap<String, u64>,
    signing_key: OAuthSigningKey,
    database: Option<Arc<ServerStateDb>>,
}

impl OAuthState {
    fn new(
        signing_key: OAuthSigningKey,
        database: Option<Arc<ServerStateDb>>,
        persisted: Option<PersistedOAuthRuntime>,
    ) -> Self {
        let state = Self {
            clients: DashMap::new(),
            cimd_clients: DashMap::new(),
            authorization_codes: DashMap::new(),
            refresh_tokens: DashMap::new(),
            revoked_jti: DashMap::new(),
            signing_key,
            database,
        };
        if let Some(persisted) = persisted {
            for (client_id, client) in persisted.clients {
                state.clients.insert(client_id, client);
            }
            for (token, record) in persisted.refresh_tokens {
                state.refresh_tokens.insert(token, record);
            }
            for (jti, expires_at) in persisted.revoked_jti {
                state.revoked_jti.insert(jti, expires_at);
            }
        }
        state
    }

    async fn persist(&self) -> Result<(), String> {
        let Some(database) = self.database.as_ref() else {
            return Ok(());
        };
        let clients = self
            .clients
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let refresh_tokens = self
            .refresh_tokens
            .iter()
            .map(|entry| (entry.key().clone(), entry.value().clone()))
            .collect();
        let revoked_jti = self
            .revoked_jti
            .iter()
            .map(|entry| (entry.key().clone(), *entry.value()))
            .collect();
        database
            .set_json(
                KV_KEY_MCP_OAUTH_RUNTIME,
                &PersistedOAuthRuntime {
                    version: OAUTH_RUNTIME_VERSION,
                    clients,
                    refresh_tokens,
                    revoked_jti,
                },
            )
            .await
    }

    fn purge_expired(&self) {
        let now = Instant::now();
        let now_unix = unix_timestamp();
        self.authorization_codes
            .retain(|_, record| record.expires_at > now);
        self.refresh_tokens
            .retain(|_, record| record.expires_at > now_unix);
        self.revoked_jti
            .retain(|_, expires_at| *expires_at > now_unix);
    }

    fn issue_tokens(
        &self,
        client_id: &str,
        issuer: &str,
        resource: &str,
        scope: &str,
        family_id: Option<&str>,
    ) -> Result<TokenResponse, String> {
        self.purge_expired();
        let issued_at = unix_timestamp();
        let claims = AccessTokenClaims {
            iss: issuer.to_owned(),
            sub: client_id.to_owned(),
            aud: resource.to_owned(),
            resource: resource.to_owned(),
            scope: scope.to_owned(),
            iat: issued_at,
            nbf: issued_at,
            exp: issued_at + ACCESS_TOKEN_TTL.as_secs(),
            jti: crate::server::issue_token(),
        };
        let mut header = Header::new(Algorithm::EdDSA);
        header.kid = Some(self.signing_key.kid.clone());
        let encoding_key = self.signing_key.encoding_key()?;
        let access_token = encode(&header, &claims, &encoding_key)
            .map_err(|error| format!("failed to sign MCP OAuth access token: {error}"))?;
        let refresh_token = crate::server::issue_token();
        let family_id = family_id
            .map(str::to_owned)
            .unwrap_or_else(crate::server::issue_token);
        self.refresh_tokens.insert(
            refresh_token.clone(),
            RefreshTokenRecord {
                client_id: client_id.to_owned(),
                resource: resource.to_owned(),
                scope: scope.to_owned(),
                expires_at: issued_at + REFRESH_TOKEN_TTL.as_secs(),
                family_id,
                used: false,
                revoked: false,
            },
        );
        Ok(TokenResponse {
            access_token,
            token_type: "Bearer",
            expires_in: ACCESS_TOKEN_TTL.as_secs(),
            refresh_token,
            scope: scope.to_owned(),
        })
    }

    fn validate_access_token(&self, token: &str, issuer: &str, resource: &str) -> bool {
        self.purge_expired();
        let Ok(header) = jsonwebtoken::decode_header(token) else {
            return false;
        };
        if header.alg != Algorithm::EdDSA
            || header.kid.as_deref() != Some(self.signing_key.kid.as_str())
        {
            return false;
        }
        let mut validation = Validation::new(Algorithm::EdDSA);
        validation.set_issuer(&[issuer]);
        validation.set_audience(&[resource]);
        validation.set_required_spec_claims(&["exp", "nbf", "iss", "sub", "aud"]);
        validation.validate_nbf = true;
        validation.leeway = 5;
        let Ok(token_data) =
            decode::<AccessTokenClaims>(token, &self.signing_key.decoding_key(), &validation)
        else {
            return false;
        };
        let claims = token_data.claims;
        let now = unix_timestamp();
        claims.resource == resource
            && !claims.jti.is_empty()
            && claims.iat <= now.saturating_add(5)
            && scope_contains(claims.scope.as_str(), MCP_SCOPE)
            && !self.revoked_jti.contains_key(claims.jti.as_str())
    }
}

#[derive(Debug, Deserialize)]
struct TokenRequest {
    grant_type: String,
    #[serde(default)]
    code: Option<String>,
    #[serde(default)]
    redirect_uri: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    code_verifier: Option<String>,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    resource: Option<String>,
    #[serde(default)]
    scope: Option<String>,
}

#[derive(Debug, Serialize)]
struct TokenResponse {
    access_token: String,
    token_type: &'static str,
    expires_in: u64,
    refresh_token: String,
    scope: String,
}

async fn token(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    request: Result<Form<TokenRequest>, axum::extract::rejection::FormRejection>,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let Form(request) = match request {
        Ok(request) => request,
        Err(_) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the token request body must be a valid form",
            );
        }
    };
    let result = match request.grant_type.as_str() {
        "authorization_code" => exchange_authorization_code(&state, &headers, &request).await,
        "refresh_token" => refresh_access_token(&state, &headers, &request).await,
        _ => Err(OAuthTokenError::new(
            "unsupported_grant_type",
            "grant_type must be authorization_code or refresh_token",
        )),
    };
    match result {
        Ok(response) => json_no_store(StatusCode::OK, response),
        Err(error) => error.into_response(),
    }
}

async fn exchange_authorization_code(
    state: &McpServerState,
    headers: &HeaderMap,
    request: &TokenRequest,
) -> Result<TokenResponse, OAuthTokenError> {
    let client_id = request
        .client_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthTokenError::new("invalid_request", "client_id is required"))?;
    if !is_registered_oauth_client(state, client_id)
        && load_cimd_client(state, client_id).await.is_err()
    {
        return Err(OAuthTokenError::new(
            "invalid_client",
            "client_id is unknown or its CIMD metadata could not be validated",
        ));
    }
    let code = request
        .code
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthTokenError::new("invalid_request", "code is required"))?;
    let (_, record) = state
        .oauth
        .authorization_codes
        .remove(code)
        .ok_or_else(|| {
            OAuthTokenError::new("invalid_grant", "authorization code is invalid or expired")
        })?;
    if record.expires_at <= Instant::now()
        || record.client_id != client_id
        || request.redirect_uri.as_deref() != Some(record.redirect_uri.as_str())
        || request.resource.as_deref() != Some(record.resource.as_str())
    {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "authorization code binding is invalid",
        ));
    }
    let verifier = request
        .code_verifier
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthTokenError::new("invalid_grant", "code_verifier is required"))?;
    if !pkce_matches(verifier, record.code_challenge.as_str()) {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "PKCE verification failed",
        ));
    }
    if let Some(scope) = request.scope.as_deref()
        && normalize_scope(Some(scope)).ok().as_deref() != Some(record.scope.as_str())
    {
        return Err(OAuthTokenError::new(
            "invalid_scope",
            "requested scope is not granted",
        ));
    }
    if record.issuer != state.issuer_for_headers(headers) {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "authorization code issuer binding is invalid",
        ));
    }
    let response = state
        .oauth
        .issue_tokens(
            client_id,
            record.issuer.as_str(),
            record.resource.as_str(),
            record.scope.as_str(),
            None,
        )
        .map_err(|_| {
            OAuthTokenError::new(
                "server_error",
                "the authorization server could not issue an access token",
            )
        })?;
    state.oauth.persist().await.map_err(|_| {
        OAuthTokenError::new(
            "server_error",
            "the authorization server could not persist the issued token",
        )
    })?;
    Ok(response)
}

async fn refresh_access_token(
    state: &McpServerState,
    headers: &HeaderMap,
    request: &TokenRequest,
) -> Result<TokenResponse, OAuthTokenError> {
    let client_id = request
        .client_id
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthTokenError::new("invalid_request", "client_id is required"))?;
    let refresh_token = request
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or_else(|| OAuthTokenError::new("invalid_request", "refresh_token is required"))?;
    if !is_registered_oauth_client(state, client_id)
        && load_cimd_client(state, client_id).await.is_err()
    {
        return Err(OAuthTokenError::new(
            "invalid_client",
            "client_id is unknown or its CIMD metadata could not be validated",
        ));
    }
    let record = state
        .oauth
        .refresh_tokens
        .get_mut(refresh_token)
        .ok_or_else(|| OAuthTokenError::new("invalid_grant", "refresh token is invalid or expired"))
        .map(|record| record.clone())?;
    if record.expires_at <= unix_timestamp()
        || record.client_id != client_id
        || request.resource.as_deref() != Some(record.resource.as_str())
        || record.revoked
    {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "refresh token binding is invalid",
        ));
    }
    if record.used {
        for mut entry in state.oauth.refresh_tokens.iter_mut() {
            if entry.family_id == record.family_id {
                entry.revoked = true;
            }
        }
        let _ = state.oauth.persist().await;
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "refresh token replay detected; the token family has been revoked",
        ));
    }
    if let Some(scope) = request.scope.as_deref()
        && normalize_scope(Some(scope)).ok().as_deref() != Some(record.scope.as_str())
    {
        return Err(OAuthTokenError::new(
            "invalid_scope",
            "requested scope is not granted",
        ));
    }
    if let Some(mut current) = state.oauth.refresh_tokens.get_mut(refresh_token) {
        current.used = true;
    }
    let response = state
        .oauth
        .issue_tokens(
            client_id,
            state.issuer_for_headers(headers).as_str(),
            record.resource.as_str(),
            record.scope.as_str(),
            Some(record.family_id.as_str()),
        )
        .map_err(|_| {
            OAuthTokenError::new(
                "server_error",
                "the authorization server could not issue an access token",
            )
        })?;
    state.oauth.persist().await.map_err(|_| {
        OAuthTokenError::new(
            "server_error",
            "the authorization server could not persist the rotated token",
        )
    })?;
    Ok(response)
}

#[derive(Debug, Deserialize)]
struct RevokeRequest {
    token: String,
    #[serde(default)]
    client_id: Option<String>,
}

async fn revoke(
    State(state): State<Arc<McpServerState>>,
    request: Result<Form<RevokeRequest>, axum::extract::rejection::FormRejection>,
) -> Response {
    if !state.auth_enabled() {
        return mcp_auth_disabled();
    }
    let Form(request) = match request {
        Ok(request) => request,
        Err(_) => {
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the revocation request body must be a valid form",
            );
        }
    };
    if request.token.trim().is_empty() {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required",
        );
    }
    if let Some(client_id) = request.client_id.as_deref()
        && !is_registered_oauth_client(&state, client_id)
        && load_cimd_client(&state, client_id).await.is_err()
    {
        return oauth_error(
            StatusCode::UNAUTHORIZED,
            "invalid_client",
            "client_id is unknown or its CIMD metadata could not be validated",
        );
    }
    if let Ok(token_data) =
        jsonwebtoken::dangerous::insecure_decode::<AccessTokenClaims>(request.token.as_str())
        && !token_data.claims.jti.is_empty()
    {
        state
            .oauth
            .revoked_jti
            .insert(token_data.claims.jti, token_data.claims.exp);
    }
    if let Some(mut refresh) = state.oauth.refresh_tokens.get_mut(request.token.as_str()) {
        let family_id = refresh.family_id.clone();
        refresh.revoked = true;
        for mut entry in state.oauth.refresh_tokens.iter_mut() {
            if entry.family_id == family_id {
                entry.revoked = true;
            }
        }
    }
    if let Err(error) = state.oauth.persist().await {
        tracing::error!(error = %error, "failed to persist revoked MCP OAuth token");
        return oauth_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "the token revocation could not be persisted",
        );
    }
    json_no_store(StatusCode::OK, serde_json::json!({}))
}

async fn require_mcp_enabled(
    State(state): State<Arc<McpServerState>>,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if state.is_enabled() {
        return next.run(request).await;
    }

    // Keep the Agena Web/TUI control plane alive while removing the complete
    // public MCP surface. A disabled connector should not be able to discover
    // OAuth metadata, register a client, or continue using a bearer token.
    mcp_auth_disabled()
}

async fn require_mcp_auth_enabled(
    State(state): State<Arc<McpServerState>>,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if state.auth_enabled() {
        return next.run(request).await;
    }
    mcp_auth_disabled()
}

async fn require_mcp_bearer(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    if !state.auth_enabled() {
        return next.run(request).await;
    }
    let resource = state.public_resource(&headers);
    let issuer = state.issuer_for_headers(&headers);
    let authorized = bearer_token(&headers).is_some_and(|token| {
        state
            .oauth
            .validate_access_token(token.as_str(), issuer.as_str(), resource.as_str())
    });
    if authorized {
        return next.run(request).await;
    }

    // ChatGPT can use this MCP-level result to open the tool OAuth linking UI
    // when a call reaches the server without a usable access token. Other MCP
    // requests still receive the RFC 9728 HTTP challenge so discovery and
    // whole-server OAuth continue to work as usual.
    let (parts, body) = request.into_parts();
    if parts.method == axum::http::Method::POST
        && let Ok(body) = to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await
        && let Some(id) = unauthorized_tool_call_id(&body)
    {
        return mcp_tool_authentication_required(&issuer, &resource, id);
    }
    mcp_unauthorized(&issuer, &resource)
}

fn mcp_unauthorized(issuer: &str, resource: &str) -> Response {
    let value = mcp_www_authenticate(issuer, resource, None, None);
    let mut response = (
        StatusCode::UNAUTHORIZED,
        Json(serde_json::json!({
            "error": "invalid_token",
            "error_description": "A valid Agena MCP OAuth access token is required."
        })),
    )
        .into_response();
    if let Ok(value) = HeaderValue::try_from(value) {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, value);
    }
    response
}

fn unauthorized_tool_call_id(body: &[u8]) -> Option<serde_json::Value> {
    let request = serde_json::from_slice::<serde_json::Value>(body).ok()?;
    if request.get("method").and_then(serde_json::Value::as_str) != Some("tools/call") {
        return None;
    }
    request.get("id").cloned().filter(|id| !id.is_null())
}

fn mcp_tool_authentication_required(
    issuer: &str,
    resource: &str,
    id: serde_json::Value,
) -> Response {
    json_no_store(
        StatusCode::OK,
        mcp_tool_authentication_payload(issuer, resource, id),
    )
}

fn mcp_tool_authentication_payload(
    issuer: &str,
    resource: &str,
    id: serde_json::Value,
) -> serde_json::Value {
    let challenge = mcp_www_authenticate(
        issuer,
        resource,
        Some("invalid_token"),
        Some("A valid Agena MCP OAuth access token is required."),
    );
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "result": {
            "content": [{
                "type": "text",
                "text": "Authentication required: a valid Agena MCP OAuth access token is required."
            }],
            "isError": true,
            "_meta": {
                "mcp/www_authenticate": [challenge]
            }
        }
    })
}

fn mcp_www_authenticate(
    issuer: &str,
    _resource: &str,
    error: Option<&str>,
    description: Option<&str>,
) -> String {
    let metadata_url = append_public_endpoint(issuer, "/.well-known/oauth-protected-resource");
    let mut value = format!(
        "Bearer resource_metadata=\"{}\", scope=\"{}\"",
        metadata_url, MCP_SCOPE
    );
    if let Some(error) = error {
        value.push_str(", error=\"");
        value.push_str(&escape_auth_param(error));
        value.push('"');
    }
    if let Some(description) = description {
        value.push_str(", error_description=\"");
        value.push_str(&escape_auth_param(description));
        value.push('"');
    }
    value
}

fn escape_auth_param(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[derive(Debug, Clone)]
struct OAuthRequestError {
    error: &'static str,
    description: &'static str,
}

impl OAuthRequestError {
    const fn new(error: &'static str, description: &'static str) -> Self {
        Self { error, description }
    }
}

impl IntoResponse for OAuthRequestError {
    fn into_response(self) -> Response {
        oauth_error(StatusCode::BAD_REQUEST, self.error, self.description)
    }
}

#[derive(Debug, Clone)]
struct OAuthTokenError {
    error: &'static str,
    description: &'static str,
}

impl OAuthTokenError {
    const fn new(error: &'static str, description: &'static str) -> Self {
        Self { error, description }
    }
}

impl IntoResponse for OAuthTokenError {
    fn into_response(self) -> Response {
        let status = match self.error {
            "invalid_client" => StatusCode::UNAUTHORIZED,
            _ => StatusCode::BAD_REQUEST,
        };
        oauth_error(status, self.error, self.description)
    }
}

#[derive(Debug, Serialize)]
struct OAuthErrorBody {
    error: &'static str,
    error_description: &'static str,
}

fn oauth_error(status: StatusCode, error: &'static str, description: &'static str) -> Response {
    json_no_store(
        status,
        OAuthErrorBody {
            error,
            error_description: description,
        },
    )
}

fn json_no_store<T: Serialize>(status: StatusCode, value: T) -> Response {
    let mut response = (status, Json(value)).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn normalize_scope(scope: Option<&str>) -> Result<String, OAuthRequestError> {
    let scope = scope.unwrap_or(MCP_SCOPE);
    let scopes = scope.split_whitespace().collect::<Vec<_>>();
    if !scopes.is_empty() && scopes.iter().all(|value| *value == MCP_SCOPE) {
        Ok(MCP_SCOPE.to_owned())
    } else {
        Err(OAuthRequestError::new(
            "invalid_scope",
            "only the agena:tools scope is supported",
        ))
    }
}

fn scope_contains(scope: &str, expected: &str) -> bool {
    scope.split_whitespace().any(|value| value == expected)
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    if !(43..=128).contains(&verifier.len()) {
        return false;
    }
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest) == challenge
}

fn validate_redirect_uri(value: &str) -> Result<(), ()> {
    let url = Url::parse(value).map_err(|_| ())?;
    if url.scheme() != "https" && !(url.scheme() == "http" && is_loopback_host(&url)) {
        return Err(());
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(());
    }
    Ok(())
}

fn is_registered_oauth_client(state: &McpServerState, client_id: &str) -> bool {
    state.oauth.clients.contains_key(client_id)
}

/// ChatGPT's CIMD client ID is an HTTPS metadata-document URL rather than a
/// value previously returned by /oauth/register. Keep the accepted surface
/// deliberately narrow: only OpenAI's production ChatGPT metadata namespace is
/// trusted, and the document is fetched and validated before use.
fn is_supported_cimd_client_id(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    let path = url.path();
    url.scheme() == "https"
        && url.host_str() == Some("chatgpt.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && path.starts_with("/oauth/")
        && path.ends_with("/client.json")
        && path
            .split('/')
            .all(|segment| segment != "." && segment != "..")
        && path
            .strip_prefix("/oauth/")
            .is_some_and(|suffix| suffix.len() > "/client.json".len())
}

#[cfg(test)]
fn is_supported_chatgpt_redirect_uri(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    let Some(callback_id) = url.path().strip_prefix("/connector/oauth/") else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str() == Some("chatgpt.com")
        && url.port().is_none()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && !callback_id.is_empty()
        && !callback_id.contains('/')
}

fn is_loopback_host(url: &Url) -> bool {
    matches!(
        url.host_str(),
        Some("localhost" | "127.0.0.1" | "[::1]" | "::1")
    )
}

fn normalize_public_mcp_url(value: &str) -> Result<String, String> {
    let mut url =
        Url::parse(value.trim()).map_err(|error| format!("invalid MCP public URL: {error}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("MCP public URL must use http or https".to_owned());
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "MCP public URL must contain only an HTTP(S) URL without credentials, query, or fragment"
                .to_owned(),
        );
    }
    if url
        .path()
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("MCP public URL path must not contain dot segments".to_owned());
    }
    let path = url.path().trim_end_matches('/').to_owned();
    if path.is_empty() || path == MCP_PATH {
        url.set_path(MCP_PATH);
    } else {
        // Secure MCP Tunnel URLs contain an OpenAI routing prefix such as
        // /v1/mcp/{tunnel_id}. That path is the externally visible resource
        // identity and must not be replaced with the local upstream /mcp.
        url.set_path(path.as_str());
    }
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn normalize_oauth_issuer_url(value: &str) -> Result<String, String> {
    let url = Url::parse(value.trim())
        .map_err(|error| format!("invalid MCP OAuth issuer URL: {error}"))?;
    if url.scheme() != "https" && url.scheme() != "http" {
        return Err("MCP OAuth issuer URL must use http or https".to_owned());
    }
    if url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(
            "MCP OAuth issuer URL must contain only an HTTP(S) URL without credentials, query, or fragment"
                .to_owned(),
        );
    }
    if url
        .path()
        .split('/')
        .any(|segment| segment == "." || segment == "..")
    {
        return Err("MCP OAuth issuer URL path must not contain dot segments".to_owned());
    }
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn is_https_resource(value: &str) -> bool {
    Url::parse(value)
        .map(|url| url.scheme().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

fn issuer_for_resource(resource: &str) -> String {
    resource
        .strip_suffix(MCP_PATH)
        .unwrap_or(resource)
        .trim_end_matches('/')
        .to_owned()
}

fn append_public_endpoint(base: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
}

fn forwarded_header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.split(',').next())
        .map(str::trim)
        .filter(|value| !value.is_empty())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let mut parts = value.split_whitespace();
    if !parts.next()?.eq_ignore_ascii_case("bearer") {
        return None;
    }
    let token = parts.next()?.trim();
    if token.is_empty() || parts.next().is_some() {
        return None;
    }
    Some(token.to_owned())
}

fn unix_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool(name: &str, interactive: bool, plugin_id: Option<&str>) -> OperatorToolResource {
        OperatorToolResource {
            name: name.to_owned(),
            summary: None,
            before_help: None,
            after_help: None,
            input_schema: serde_json::json!({"type": "object"}),
            interactive,
            plugin_id: plugin_id.map(str::to_owned),
        }
    }

    #[test]
    fn public_mcp_url_is_normalized() {
        assert_eq!(
            normalize_public_mcp_url("https://example.test").expect("origin"),
            "https://example.test/mcp"
        );
        assert_eq!(
            normalize_public_mcp_url("https://example.test/mcp/").expect("MCP URL"),
            "https://example.test/mcp"
        );
        assert_eq!(
            normalize_public_mcp_url(
                "https://tunnel-service.gateway.unified-0.internal.api.openai.org/v1/mcp/tunnel_123/"
            )
            .expect("Secure MCP Tunnel URL"),
            "https://tunnel-service.gateway.unified-0.internal.api.openai.org/v1/mcp/tunnel_123"
        );
        assert!(normalize_public_mcp_url("https://example.test/other?query=1").is_err());
        assert_eq!(
            normalize_public_mcp_url("https://example.test/a/%2e%2e/mcp").expect("normalized path"),
            "https://example.test/mcp"
        );
        assert!(normalize_public_mcp_url("https://user@example.test").is_err());
    }

    #[test]
    fn tunnel_resource_uses_its_full_path_for_oauth_endpoints() {
        let resource = "https://tunnel-service.example/v1/mcp/tunnel_123";
        let issuer = issuer_for_resource(resource);
        assert_eq!(issuer, resource);
        assert_eq!(
            append_public_endpoint(&issuer, "/oauth/token"),
            "https://tunnel-service.example/v1/mcp/tunnel_123/oauth/token"
        );
    }

    #[test]
    fn chatgpt_cimd_and_callback_allowlist_is_strict() {
        assert!(is_supported_cimd_client_id(
            "https://chatgpt.com/oauth/abc123/client.json"
        ));
        assert!(!is_supported_cimd_client_id(
            "http://chatgpt.com/oauth/abc123/client.json"
        ));
        assert!(!is_supported_cimd_client_id(
            "https://attacker.example/oauth/abc123/client.json"
        ));
        assert!(!is_supported_cimd_client_id(
            "https://chatgpt.com/oauth/abc123/client.json?redirect_uri=https://attacker.example"
        ));
        assert!(is_supported_chatgpt_redirect_uri(
            "https://chatgpt.com/connector/oauth/QTOb4VcHdCsW"
        ));
        assert!(!is_supported_chatgpt_redirect_uri(
            "https://chatgpt.com/connector/oauth/QTOb4VcHdCsW/extra"
        ));
        assert!(!is_supported_chatgpt_redirect_uri(
            "https://attacker.example/connector/oauth/QTOb4VcHdCsW"
        ));
    }

    #[test]
    fn mcp_catalog_hides_interactive_and_provider_tools() {
        assert!(mcp_tool_is_exposed(&tool(
            "fs.read",
            false,
            Some("agena.fs")
        )));
        assert!(!mcp_tool_is_exposed(&tool(
            "prompt.ask",
            true,
            Some("agena.prompt")
        )));
        assert!(!mcp_tool_is_exposed(&tool(
            "chatgpt.search",
            false,
            Some("agena.chatgpt")
        )));
        assert!(!mcp_tool_is_exposed(&tool("gemini.search", false, None)));
        assert!(!mcp_tool_is_exposed(&tool("claude.ask", false, None)));
        assert!(!mcp_tool_is_exposed(&tool("web.browser_list", false, None)));
        assert!(!mcp_tool_is_exposed(&tool(
            "agena.web.browser_wait",
            false,
            None
        )));
        assert!(!mcp_tool_is_exposed(&tool(
            "interaction.notify",
            false,
            None
        )));
        assert!(!mcp_tool_is_exposed(&tool("plan.phase", false, None)));
        assert!(!mcp_tool_is_exposed(&tool("plan.review", false, None)));
    }

    #[test]
    fn pkce_s256_matches_only_the_expected_verifier() {
        let verifier = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-._~";
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        assert!(pkce_matches(verifier, challenge.as_str()));
        assert!(!pkce_matches(
            "wrong-verifier-that-is-long-enough-to-test-pkce-0123456789",
            challenge.as_str()
        ));
    }

    #[test]
    fn scope_is_exactly_limited_to_agena_tools() {
        assert_eq!(normalize_scope(None).expect("default scope"), MCP_SCOPE);
        assert!(normalize_scope(Some("agena:tools")).is_ok());
        assert_eq!(
            normalize_scope(Some("agena:tools agena:tools")).expect("duplicate scope"),
            MCP_SCOPE
        );
        assert!(normalize_scope(Some("agena:tools other")).is_err());
        assert!(normalize_scope(Some("")).is_err());
    }

    #[test]
    fn disabled_mcp_auth_hides_oauth_routes_with_an_empty_not_found() {
        let response = mcp_auth_disabled();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
    }

    #[test]
    fn chatgpt_security_schemes_are_top_level_tool_fields() {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {
                "tools": [{
                    "name": "fs.read",
                    "_meta": {"server": "agena"}
                }]
            }
        });
        let payload = add_tool_security_schemes(payload, true).expect("tools/list payload");
        assert_eq!(
            payload["result"]["tools"][0]["securitySchemes"],
            serde_json::json!([{"type": "oauth2", "scopes": [MCP_SCOPE]}])
        );
        assert_eq!(payload["result"]["tools"][0]["_meta"]["server"], "agena");
    }

    #[test]
    fn anonymous_chatgpt_security_scheme_is_noauth() {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 7,
            "result": {"tools": [{"name": "fs.read"}]}
        });
        let payload = add_tool_security_schemes(payload, false).expect("tools/list payload");
        assert_eq!(
            payload["result"]["tools"][0]["securitySchemes"],
            serde_json::json!([{"type": "noauth"}])
        );
    }

    #[test]
    fn chatgpt_security_schemes_are_added_inside_sse_data_events() {
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 8,
            "result": {"tools": [{"name": "fs.read"}]}
        });
        let body = format!(
            "retry: 3000\n\nevent: message\ndata: {}\n\n",
            serde_json::to_string(&payload).expect("serialize payload")
        );
        let rewritten = rewrite_tool_list_sse(body.as_bytes(), true).expect("SSE tools/list");
        let rewritten = String::from_utf8(rewritten).expect("UTF-8 SSE body");
        assert!(rewritten.contains("\"securitySchemes\":[{"));
        assert!(rewritten.contains("\"type\":\"oauth2\""));
        assert!(rewritten.contains("\"scopes\":[\"agena:tools\"]"));
        assert!(rewritten.starts_with("retry: 3000\n\nevent: message\ndata: "));
    }

    #[test]
    fn authorization_form_action_preserves_a_tunnel_path_prefix() {
        let request = ValidatedAuthorizationRequest {
            client_id: "client-1".to_owned(),
            client_name: Some("ChatGPT".to_owned()),
            redirect_uri: "https://chatgpt.com/connector/oauth/callback".to_owned(),
            state: "state".to_owned(),
            code_challenge: "challenge".to_owned(),
            issuer: "https://tunnel.example".to_owned(),
            resource: "https://tunnel.example/v1/mcp/tunnel_123".to_owned(),
            scope: MCP_SCOPE.to_owned(),
        };
        let html = render_authorization_page(&request, None);
        assert!(html.contains("action=\"authorize\""));
        assert!(!html.contains("action=\"/oauth/authorize\""));
    }

    #[test]
    fn openai_server_discover_probe_returns_modern_result_with_echoed_id() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "openai-mcp-discover",
            "method": "server/discover",
            "params": {
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": "2026-07-28",
                    "io.modelcontextprotocol/clientInfo": {
                        "name": "ChatGPT",
                        "version": "diagnostic"
                    },
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }
        });
        let payload = openai_mcp_discover_payload(
            &serde_json::to_vec(&request).expect("serialize discover request"),
        )
        .expect("OpenAI discover request");

        assert_eq!(payload["jsonrpc"], "2.0");
        assert_eq!(payload["id"], "openai-mcp-discover");
        assert_eq!(payload["result"]["resultType"], "complete");
        assert_eq!(payload["result"]["supportedVersions"][0], "2026-07-28");
        assert_eq!(
            payload["result"]["capabilities"]["tools"],
            serde_json::json!({})
        );
        assert_eq!(
            payload["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
            "agena"
        );
        assert_eq!(payload["result"]["cacheScope"], "public");
        assert!(payload["result"]["ttlMs"].as_u64().is_some());
    }

    #[test]
    fn openai_server_discover_probe_supports_an_all_discover_batch() {
        let request = serde_json::json!([
            {
                "jsonrpc": "2.0",
                "id": "openai-mcp-discover-1",
                "method": "server/discover"
            },
            {
                "jsonrpc": "2.0",
                "id": "openai-mcp-discover-2",
                "method": "server/discover",
                "params": {}
            }
        ]);
        let payload = openai_mcp_discover_payload(
            &serde_json::to_vec(&request).expect("serialize discover batch"),
        )
        .expect("OpenAI discover batch");

        assert_eq!(payload[0]["id"], "openai-mcp-discover-1");
        assert_eq!(payload[1]["id"], "openai-mcp-discover-2");
        assert_eq!(payload[0]["result"]["resultType"], "complete");
        assert_eq!(payload[1]["result"]["resultType"], "complete");
        assert_eq!(payload[0]["result"]["supportedVersions"][0], "2026-07-28");
        assert_eq!(payload[1]["result"]["supportedVersions"][0], "2026-07-28");
    }

    #[test]
    fn openai_server_discover_compatibility_does_not_match_other_requests() {
        let initialize = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {}
        });
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "server/discover",
            "params": {}
        });

        assert!(openai_mcp_discover_payload(&serde_json::to_vec(&initialize).unwrap()).is_none());
        assert!(openai_mcp_discover_payload(&serde_json::to_vec(&notification).unwrap()).is_none());
    }

    #[test]
    fn stateless_initialize_compatibility_returns_chatgpt_handshake() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 12,
            "method": "initialize",
            "params": {"protocolVersion": "2025-06-18"}
        });
        let result = compat_initialize_result(&request);
        assert_eq!(result["protocolVersion"], "2025-06-18");
        assert_eq!(result["capabilities"]["tools"], serde_json::json!({}));
        assert_eq!(result["serverInfo"]["name"], "agena");
    }

    #[test]
    fn stateless_jsonrpc_error_uses_standard_error_codes() {
        let error = compat_jsonrpc_error(
            Some(serde_json::json!(12)),
            McpServerError::InvalidParams("bad params".to_owned()),
        );
        assert_eq!(error["jsonrpc"], "2.0");
        assert_eq!(error["id"], 12);
        assert_eq!(error["error"]["code"], -32602);
    }

    #[test]
    fn legacy_rmcp_422_is_normalized_for_stateless_mcp() {
        let response = Response::builder()
            .status(StatusCode::UNPROCESSABLE_ENTITY)
            .body(Body::from("Unexpected message, expect initialize request"))
            .expect("build legacy 422 response");
        let response = normalize_mcp_unprocessable_response(response, Some(serde_json::json!(9)));
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[test]
    fn unauthorized_tool_call_payload_contains_chatgpt_auth_challenge() {
        let payload = mcp_tool_authentication_payload(
            "https://example.test",
            "https://example.test/mcp",
            serde_json::json!(4),
        );
        assert_eq!(payload["result"]["isError"], true);
        assert_eq!(
            payload["result"]["_meta"]["mcp/www_authenticate"][0],
            "Bearer resource_metadata=\"https://example.test/.well-known/oauth-protected-resource\", scope=\"agena:tools\", error=\"invalid_token\", error_description=\"A valid Agena MCP OAuth access token is required.\""
        );
    }

    #[test]
    fn unauthorized_tool_call_id_only_accepts_tools_call_requests() {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": "call-1",
            "method": "tools/call",
            "params": {"name": "fs.read"}
        });
        assert_eq!(
            unauthorized_tool_call_id(&serde_json::to_vec(&request).expect("request")),
            Some(serde_json::json!("call-1"))
        );
        let list_request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/list"
        });
        assert_eq!(
            unauthorized_tool_call_id(&serde_json::to_vec(&list_request).expect("request")),
            None
        );
    }

    #[test]
    fn redirect_uri_validation_requires_https_except_loopback() {
        assert!(validate_redirect_uri("https://chatgpt.com/connector/oauth/abc").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1:3210/callback").is_ok());
        assert!(validate_redirect_uri("http://example.test/callback").is_err());
        assert!(validate_redirect_uri("https://example.test/callback#fragment").is_err());
        assert!(validate_redirect_uri("https://user@example.test/callback").is_err());
    }

    #[test]
    fn access_tokens_are_bound_to_resource_and_scope() {
        let state = OAuthState::new(OAuthSigningKey::ephemeral(), None, None);
        let issuer = "https://example.test";
        let resource = "https://example.test/mcp";
        let response = state
            .issue_tokens("client-1", issuer, resource, MCP_SCOPE, None)
            .expect("issue token");
        assert!(state.validate_access_token(response.access_token.as_str(), issuer, resource));
        assert!(!state.validate_access_token(
            response.access_token.as_str(),
            issuer,
            "https://other.example.test/mcp"
        ));
    }
}
