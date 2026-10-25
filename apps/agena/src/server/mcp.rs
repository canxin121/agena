//! Remote MCP transport with an optional OAuth resource/authorization server.
//!
//! The stdio bridge lives in `agena-cli`; this module is the HTTP-facing
//! integration used by ChatGPT's Custom MCP Connector. The MCP surface is
//! anonymous by default; the Web/TUI control plane can enable the OAuth
//! resource server when a client requires it.

use portable_atomic::AtomicI64;
use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, RwLock, atomic::Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use agena_application::{Application, dto::OperatorToolResource};
use agena_mcp_server::{
    CallToolParams, CallToolResult, McpServerBackend, McpServerError, McpToolAccess,
    StatelessMcpToolMetadata, ToolDescriptor, is_stateless_mcp_tool_exposed, mcp_tool_access,
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

use super::persistence::db::{KV_KEY_MCP_OAUTH_RUNTIME, KV_KEY_MCP_SERVER_CONTROL, ServerStateDb};

const MCP_SCOPE: &str = "agena:tools";
const OFFLINE_ACCESS_SCOPE: &str = "offline_access";
const DEFAULT_OAUTH_SCOPE: &str = "agena:tools offline_access";
const MCP_PATH: &str = "/mcp";
const ACCESS_TOKEN_TTL: Duration = Duration::from_secs(60 * 60);
const REFRESH_TOKEN_TTL: Duration = Duration::from_secs(30 * 24 * 60 * 60);
const AUTHORIZATION_CODE_TTL: Duration = Duration::from_secs(5 * 60);
const CIMD_CACHE_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_MCP_METADATA_BODY_BYTES: usize = 8 * 1024 * 1024;
static MCP_METADATA: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static MCP_REWRITES: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

#[derive(Default)]
struct McpRequestMetadata {
    rpc_method: Option<String>,
    has_id: bool,
    tools_list: bool,
    is_batch: bool,
    tool_calls: Vec<(String, Option<serde_json::Value>)>,
}

impl McpRequestMetadata {
    fn parse(body: &[u8]) -> Self {
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
            return Self::default();
        };
        let mut metadata = Self {
            rpc_method: jsonrpc_method(&value).map(|method| method.chars().take(128).collect()),
            has_id: jsonrpc_request_id(&value).is_some(),
            tools_list: contains_tools_list(&value),
            is_batch: value.is_array(),
            tool_calls: Vec::new(),
        };
        let mut capture = |request: &serde_json::Value| {
            if request.get("method").and_then(serde_json::Value::as_str) != Some("tools/call") {
                return;
            }
            let Some(name) = request
                .get("params")
                .and_then(serde_json::Value::as_object)
                .and_then(|params| params.get("name"))
                .and_then(serde_json::Value::as_str)
            else {
                return;
            };
            metadata.tool_calls.push((
                name.to_owned(),
                request.get("id").cloned().filter(|id| !id.is_null()),
            ));
        };
        if let Some(requests) = value.as_array() {
            for request in requests {
                capture(request);
            }
        } else {
            capture(&value);
        }
        metadata
    }
}

async fn request_metadata(
    parts: &mut axum::http::request::Parts,
    body: axum::body::Bytes,
) -> Result<Arc<McpRequestMetadata>, String> {
    if let Some(metadata) = parts.extensions.get::<Arc<McpRequestMetadata>>() {
        return Ok(Arc::clone(metadata));
    }
    let inline = body.len() < 64 * 1024;
    let parse = move || Arc::new(McpRequestMetadata::parse(&body));
    let metadata = if inline {
        parse()
    } else {
        MCP_METADATA
            .run(parse)
            .await
            .map_err(|error| format!("MCP request metadata worker failed: {error}"))?
    };
    parts.extensions.insert(Arc::clone(&metadata));
    Ok(metadata)
}

fn mcp_worker_unavailable(error: String) -> Response {
    tracing::error!(target: "agena::server::mcp", %error, "MCP worker failed");
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(serde_json::json!({"error": "MCP processing is temporarily unavailable"})),
    )
        .into_response()
}
const MAX_CIMD_DOCUMENT_BYTES: usize = 256 * 1024;
const MAX_MCP_OAUTH_PASSWORD_BYTES: usize = 4096;
const MAX_CACHED_CIMD_CLIENTS: usize = 128;
const MAX_OUTSTANDING_AUTHORIZATION_CODES: usize = 1024;
const MAX_REFRESH_TOKEN_RECORDS: usize = 4096;
const MAX_OAUTH_URI_BYTES: usize = 4096;
const MAX_OAUTH_LABEL_BYTES: usize = 256;
const MAX_OAUTH_TOKEN_BYTES: usize = 8192;
const MAX_OAUTH_SCOPE_BYTES: usize = 512;
const MAX_REGISTERED_CLIENTS: usize = 256;
// ChatGPT submits the user-facing OAuth authorization form from its hosted
// connector origin. Keep this allowlist explicit and narrow; arbitrary
// browser origins must not be accepted for the MCP/OAuth surface.
const TRUSTED_MCP_CLIENT_ORIGINS: &[&str] = &["https://chatgpt.com", "https://chat.openai.com"];

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

fn mcp_tool_metadata(tool: &OperatorToolResource) -> StatelessMcpToolMetadata<'_> {
    StatelessMcpToolMetadata {
        name: tool.name.as_str(),
        plugin_id: tool.plugin_id.as_str(),
        interactive: tool.interactive,
        agent: tool.agent,
    }
}

fn mcp_tool_is_exposed(tool: &OperatorToolResource) -> bool {
    is_stateless_mcp_tool_exposed(mcp_tool_metadata(tool))
}

fn refresh_token_fingerprint(token: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(token.as_bytes()))
}

fn accepts_opaque_oauth_form_origin(request: &axum::http::Request<Body>) -> bool {
    if request.method().as_str() != "POST" || request.uri().path() != "/oauth/authorize" {
        return false;
    }
    let header_matches = |name: &str, expected: &str| {
        request
            .headers()
            .get(name)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.eq_ignore_ascii_case(expected))
    };
    header_matches("sec-fetch-site", "same-origin")
        && header_matches("sec-fetch-mode", "navigate")
        && header_matches("sec-fetch-dest", "document")
        && request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split(';').next())
            .is_some_and(|value| {
                value
                    .trim()
                    .eq_ignore_ascii_case("application/x-www-form-urlencoded")
            })
}

async fn validate_mcp_host_origin(
    State(state): State<Arc<McpServerState>>,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|value| value.to_str().ok());
    if !host.is_some_and(|host| state.accepts_host(host)) {
        return control_error(
            StatusCode::BAD_REQUEST,
            "untrusted Host header for the Agena MCP/OAuth surface".to_owned(),
        );
    }

    if let Some(origin_header) = request.headers().get(header::ORIGIN) {
        let origin = origin_header.to_str().ok();
        let accepted = origin.is_some_and(|origin| {
            state.accepts_origin(origin)
                || (origin.eq_ignore_ascii_case("null")
                    && accepts_opaque_oauth_form_origin(&request))
        });
        if !accepted {
            tracing::warn!(
                target: "agena::server::mcp",
                origin = %origin.unwrap_or("<invalid utf-8>"),
                host = ?host,
                method = %request.method(),
                path = %request.uri().path(),
                "rejected MCP/OAuth request from untrusted Origin"
            );
            return control_error(
                StatusCode::FORBIDDEN,
                "untrusted Origin header for the Agena MCP/OAuth surface".to_owned(),
            );
        }
    }
    next.run(request).await
}

/// Authentication selector written by releases that let operators choose an MCP
/// authentication mode.
///
/// Agena no longer offers selectable authentication modes. The MCP surface
/// always serves the OAuth discovery/authorization surface, always keeps
/// protocol discovery anonymous, and decides per tool whether a call needs a
/// bearer token. This type exists only so control records written by older
/// releases still deserialize; its value is reported once and ignored.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LegacyMcpAuthMode {
    None,
    Oauth,
    Mixed,
}

/// Legacy companion selector of [`LegacyMcpAuthMode::Mixed`].
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
enum LegacyMcpAnonymousAccess {
    None,
    ReadOnly,
}

struct McpControlState {
    enabled: std::sync::atomic::AtomicBool,
    configured_resource: RwLock<Option<String>>,
    configured_issuer: RwLock<Option<String>>,
    oauth_password: RwLock<Option<McpOAuthPassword>>,
    /// Plaintext of a password Agena generated itself because the deployment
    /// configured none. It is surfaced once through the control plane and then
    /// dropped from memory.
    generated_password: RwLock<Option<String>>,
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
#[serde(deny_unknown_fields)]
struct PersistedMcpServerControl {
    enabled: bool,
    public_url: Option<String>,
    /// Removed selector. Parsed for backward compatibility and never written.
    #[serde(default, skip_serializing)]
    auth_mode: Option<LegacyMcpAuthMode>,
    /// Removed selector. Parsed for backward compatibility and never written.
    #[serde(default, skip_serializing)]
    anonymous_access: Option<LegacyMcpAnonymousAccess>,
    oauth_password_phc: Option<String>,
    oauth_issuer_url: Option<String>,
}

#[derive(Clone)]
struct OAuthSigningKey {
    signing_key: Arc<SigningKey>,
    kid: String,
}

impl OAuthSigningKey {
    fn generate() -> Result<Self, String> {
        let mut secret = [0u8; ed25519_dalek::SECRET_KEY_LENGTH];
        getrandom::fill(&mut secret)
            .map_err(|error| format!("MCP OAuth signing key generation failed: {error}"))?;
        Self::from_secret_key(secret)
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
    store: agena_secret_store::FileSecretStore,
) -> Result<OAuthSigningKey, String> {
    tokio::task::spawn_blocking(move || {
        use agena_secret_store::SecretStore;
        if let Some(encoded) = store
            .get_secret("signing-key")
            .map_err(|error| error.to_string())?
        {
            return decode_persisted_signing_key(&encoded);
        }
        let candidate = OAuthSigningKey::generate()?;
        let encoded = store
            .get_or_insert_secret("signing-key", &candidate.secret_key_b64())
            .map_err(|error| error.to_string())?;
        decode_persisted_signing_key(&encoded)
    })
    .await
    .map_err(|_| "MCP signing-key storage worker failed".to_owned())?
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

pub(crate) struct McpServerStartupConfig<'a> {
    pub(crate) enabled: Option<bool>,
    pub(crate) public_url: Option<&'a str>,
    pub(crate) oauth_issuer_url: Option<&'a str>,
    pub(crate) fallback_public_url: &'a str,
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
            Arc::new(OAuthState::new(OAuthSigningKey::generate()?, None, None)),
            configured_resource,
            None,
            fallback_resource,
            true,
            None,
            None,
        ))
    }

    pub(crate) async fn load(
        application: Application,
        workspace_id: i64,
        ui_auth: UiAuth,
        database: Arc<ServerStateDb>,
        startup: McpServerStartupConfig<'_>,
    ) -> Result<Self, String> {
        let persisted = database
            .get_json::<PersistedMcpServerControl>(KV_KEY_MCP_SERVER_CONTROL)
            .await?;
        let had_persisted_control = persisted.is_some();
        let (
            mut enabled,
            configured_public_url,
            configured_issuer_url,
            oauth_password,
            legacy_selectors,
        ) = match persisted.as_ref() {
            Some(persisted) => {
                let oauth_password = persisted
                    .oauth_password_phc
                    .clone()
                    .map(|phc| {
                        let verifier = auth::init_ui_auth_from_phc(phc.clone())?;
                        Ok::<_, String>(McpOAuthPassword { verifier, phc })
                    })
                    .transpose()?;
                (
                    persisted.enabled,
                    persisted.public_url.clone(),
                    persisted.oauth_issuer_url.clone(),
                    oauth_password,
                    Some((persisted.auth_mode, persisted.anonymous_access)),
                )
            }
            None => (true, None, None, None, None),
        };
        let mut configured_resource = configured_public_url
            .map(|value| normalize_public_mcp_url(value.as_str()))
            .transpose()?;
        let mut configured_issuer = configured_issuer_url
            .map(|value| normalize_oauth_issuer_url(value.as_str()))
            .transpose()?;
        let persisted_effective = (
            enabled,
            configured_resource.clone(),
            configured_issuer.clone(),
        );

        // Explicit process configuration is authoritative on every startup,
        // not only on a brand-new database. This is essential for immutable
        // deployments where a domain, OAuth mode, or policy is changed through
        // environment variables and the existing state volume is retained.
        if let Some(value) = startup.enabled {
            enabled = value;
        }
        if let Some(public_url) = startup.public_url {
            configured_resource = Some(normalize_public_mcp_url(public_url)?);
            if startup.oauth_issuer_url.is_none() {
                // An explicit deployment resource without a matching issuer
                // override selects the simple same-origin topology and keeps
                // the effective connector identity internally coherent.
                configured_issuer = None;
            }
        }
        if let Some(oauth_issuer_url) = startup.oauth_issuer_url {
            configured_issuer = Some(normalize_oauth_issuer_url(oauth_issuer_url)?);
        }
        // A deployment without any operator credential is no longer a startup
        // error: the unified policy always serves the OAuth surface, so Agena
        // generates a dedicated authorization password below instead of
        // refusing to start or leaving the authorization page unusable.
        let effective_control = (
            enabled,
            configured_resource.clone(),
            configured_issuer.clone(),
        );
        let control_changed = !had_persisted_control || persisted_effective != effective_control;
        let fallback_resource = normalize_public_mcp_url(startup.fallback_public_url)?;
        if enabled {
            let effective_resource = configured_resource
                .as_deref()
                .unwrap_or(fallback_resource.as_str());
            validate_managed_oauth_identity(effective_resource, configured_issuer.as_deref())?;
        }
        let signing_key = load_or_create_oauth_signing_key(
            agena_secret_store::FileSecretStore::new("server.oauth"),
        )
        .await?;
        let persisted_runtime = database
            .get_json::<PersistedOAuthRuntime>(KV_KEY_MCP_OAUTH_RUNTIME)
            .await?;
        let oauth_runtime = if control_changed {
            None
        } else {
            persisted_runtime
        };
        let state = Self::with_control(
            application,
            workspace_id,
            Arc::new(AtomicI64::new(1)),
            ui_auth,
            Arc::new(OAuthState::new(
                signing_key,
                Some(Arc::clone(&database)),
                oauth_runtime,
            )),
            configured_resource.clone(),
            configured_issuer.clone(),
            fallback_resource,
            enabled,
            oauth_password,
            Some(Arc::clone(&database)),
        );

        if control_changed {
            // Tokens, refresh families, and clients are bound to
            // the previous resource/issuer/policy. Never carry them across an
            // environment-driven deployment identity change.
            state.clear_oauth_runtime_state().await?;
        }
        // Persist normalized values so a later restart without the explicit
        // environment still has one coherent connector identity. Do
        // this after invalidating the previous OAuth runtime: if the control
        // write fails, the previous identity with cleared tokens is safer than
        // a new identity accidentally retaining prior token state.
        if control_changed {
            state
                .persist_control(enabled, configured_resource, configured_issuer)
                .await?;
        }
        if let Some((auth_mode, anonymous_access)) = legacy_selectors {
            tracing::warn!(
                target: "agena::server::mcp",
                auth_mode = ?auth_mode,
                anonymous_access = ?anonymous_access,
                "ignoring removed MCP authentication selectors; the unified policy applies"
            );
        }
        state.ensure_oauth_password().await?;
        Ok(state)
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
                enabled: std::sync::atomic::AtomicBool::new(enabled),
                configured_resource: RwLock::new(configured_resource),
                configured_issuer: RwLock::new(configured_issuer),
                oauth_password: RwLock::new(oauth_password),
                generated_password: RwLock::new(None),
                database,
                update_lock: tokio::sync::Mutex::new(()),
            }),
            fallback_resource,
        }
    }

    fn is_enabled(&self) -> bool {
        self.control.enabled.load(Ordering::Acquire)
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

    fn tool_is_exposed(&self, tool: &OperatorToolResource) -> bool {
        mcp_tool_is_exposed(tool)
    }

    /// Whether one exposed tool requires an OAuth access token.
    ///
    /// The decision belongs to the tool catalog, not to a deployment-wide
    /// switch: the shared stateless MCP policy marks a tool as anonymously
    /// callable only when it is explicitly listed as safe for that transport.
    fn tool_requires_auth(&self, tool: &OperatorToolResource) -> bool {
        mcp_tool_access(mcp_tool_metadata(tool)) == McpToolAccess::OAuth
    }

    async fn exposed_tool_auth_requirements(&self) -> HashMap<String, bool> {
        self.application
            .list_operator_tools()
            .await
            .into_iter()
            .filter(|tool| self.tool_is_exposed(tool))
            .map(|tool| {
                let requires_auth = self.tool_requires_auth(&tool);
                (tool.name, requires_auth)
            })
            .collect()
    }

    fn accepts_host(&self, authority: &str) -> bool {
        let authority = authority.trim().trim_end_matches('.').to_ascii_lowercase();
        if authority.is_empty() {
            return false;
        }
        if authority_host(authority.as_str())
            .as_deref()
            .is_some_and(is_loopback_host_name)
        {
            return true;
        }
        self.identity_urls()
            .into_iter()
            .any(|url| authority_matches_url(authority.as_str(), url.as_str()))
    }

    fn accepts_origin(&self, origin: &str) -> bool {
        let Ok(origin) = Url::parse(origin.trim()) else {
            return false;
        };
        if !origin.username().is_empty()
            || origin.password().is_some()
            || origin.query().is_some()
            || origin.fragment().is_some()
            || origin.path() != "/"
        {
            return false;
        }
        let candidate = origin.origin().ascii_serialization();
        TRUSTED_MCP_CLIENT_ORIGINS.contains(&candidate.as_str())
            || self.identity_urls().into_iter().any(|url| {
                Url::parse(url.as_str())
                    .map(|url| url.origin().ascii_serialization() == candidate)
                    .unwrap_or(false)
            })
            || (is_loopback_resource(self.fallback_resource.as_str())
                && is_loopback_resource(candidate.as_str()))
    }

    fn identity_urls(&self) -> Vec<String> {
        let mut urls = vec![self.fallback_resource.clone()];
        if let Some(resource) = self.configured_public_url() {
            urls.push(resource);
        }
        if let Some(issuer) = self.configured_oauth_issuer_url() {
            urls.push(issuer);
        }
        urls
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
        _headers: &HeaderMap,
        configured_resource: Option<&str>,
    ) -> String {
        configured_resource
            .map(str::to_owned)
            .unwrap_or_else(|| self.fallback_resource.clone())
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
        if !is_https_resource(resource.as_str()) && !is_loopback_resource(resource.as_str()) {
            warnings.push(
                "ChatGPT requires a canonical remote HTTPS MCP resource URL. Configure a public HTTPS endpoint or Secure MCP Tunnel before connecting it.".to_owned(),
            );
        }
        // The OAuth surface is always advertised, so it must always be
        // operational before the control plane calls the connector ready.
        if !self.has_custom_oauth_password() && !matches!(self.ui_auth, UiAuth::Enabled(_)) {
            warnings.push(
                "OAuth authorization password is not configured; set an MCP OAuth password in Web/TUI or start the server with a UI password.".to_owned(),
            );
        }
        if let Err(error) = validate_managed_oauth_identity(
            resource.as_str(),
            self.configured_oauth_issuer_url().as_deref(),
        ) {
            warnings.push(error);
        }

        McpReadiness {
            ready: warnings.is_empty(),
            warnings,
        }
    }

    pub(crate) fn log_startup_status(&self) {
        let headers = HeaderMap::new();
        let resource = self.public_resource(&headers);
        let issuer = self.issuer_for_headers(&headers);
        let readiness = self.readiness(&headers);
        tracing::info!(
            target: "agena::server::mcp",
            enabled = self.is_enabled(),
            resource = %resource,
            issuer = %issuer,
            ready = readiness.ready,
            "Agena MCP server configured"
        );
        for warning in readiness.warnings {
            tracing::warn!(
                target: "agena::server::mcp",
                warning = %warning,
                "Agena MCP deployment needs attention"
            );
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
        auth::verify_password_for_oauth_async(
            custom_verifier.unwrap_or_else(|| self.ui_auth.clone()),
            candidate.to_owned(),
            headers.clone(),
        )
        .await
    }

    async fn clear_oauth_runtime_state(&self) -> Result<(), String> {
        self.oauth.cimd_clients.clear();
        self.oauth.registered_clients.clear();
        self.oauth.authorization_codes.clear();
        self.oauth.refresh_tokens.clear();
        self.oauth.revoked_jti.clear();
        self.oauth.persist().await
    }

    fn control_status(&self, headers: &HeaderMap) -> McpServerControlResponse {
        let resource_url = self.public_resource(headers);
        let issuer = self.issuer_for_headers(headers);
        let readiness = self.readiness(headers);
        McpServerControlResponse {
            enabled: self.is_enabled(),
            public_url: self.configured_public_url(),
            generated_oauth_password: self.take_generated_password(),
            oauth_issuer_url: self.configured_oauth_issuer_url(),
            resource_url: resource_url.clone(),
            ready: readiness.ready,
            warnings: readiness.warnings.clone(),
            oauth: McpOAuthControlStatus {
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
                scope: DEFAULT_OAUTH_SCOPE.to_owned(),
                issuer: issuer.clone(),
                authorization_endpoint: append_public_endpoint(&issuer, "/oauth/authorize"),
                token_endpoint: append_public_endpoint(&issuer, "/oauth/token"),
                revocation_endpoint: append_public_endpoint(&issuer, "/oauth/revoke"),
                protected_resource_metadata: protected_resource_metadata_url(&resource_url),
                authorization_server_metadata: authorization_server_metadata_url(&issuer),
                jwks_uri: append_public_endpoint(&issuer, "/oauth/jwks.json"),
            },
        }
    }

    async fn persist_control(
        &self,
        enabled: bool,
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
                PersistedMcpServerControl {
                    enabled,
                    auth_mode: None,
                    anonymous_access: None,
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
        configured_resource: Option<String>,
        configured_issuer: Option<String>,
    ) -> Result<(), String> {
        let _guard = self.control.update_lock.lock().await;
        let previous_enabled = self.is_enabled();
        let previous_resource = self.configured_public_url();
        let previous_issuer = self.configured_oauth_issuer_url();
        let control_changed = previous_enabled != enabled
            || previous_resource != configured_resource
            || previous_issuer != configured_issuer;
        if control_changed {
            // Invalidate the previous token/client state before committing a
            // new public identity or policy. A later persistence failure then
            // leaves the previous control with no valid stale credentials.
            self.clear_oauth_runtime_state().await?;
        }
        self.persist_control(
            enabled,
            configured_resource.clone(),
            configured_issuer.clone(),
        )
        .await?;
        self.control.enabled.store(enabled, Ordering::Release);
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
        let phc = auth::hash_password_async(password.to_owned()).await?;
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
        self.clear_oauth_runtime_state().await?;
        database
            .set_json(
                KV_KEY_MCP_SERVER_CONTROL,
                PersistedMcpServerControl {
                    enabled,
                    auth_mode: None,
                    anonymous_access: None,
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
        // Removing the dedicated authorization credential revokes every
        // outstanding access/refresh token before the fallback credential is
        // committed.
        self.clear_oauth_runtime_state().await?;
        database
            .set_json(
                KV_KEY_MCP_SERVER_CONTROL,
                PersistedMcpServerControl {
                    enabled,
                    auth_mode: None,
                    anonymous_access: None,
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
        Ok(())
    }

    /// Guarantee that the authorization page has an operator credential.
    ///
    /// The unified policy always serves the OAuth surface, so a deployment that
    /// configured no password would otherwise advertise an authorization server
    /// nobody can complete. Generate a dedicated MCP OAuth password, store it
    /// through the same Argon2 path as an operator-supplied one, and surface it
    /// once (startup log + control plane) so the operator can copy it.
    async fn ensure_oauth_password(&self) -> Result<(), String> {
        if !self.is_enabled()
            || self.has_custom_oauth_password()
            || matches!(&self.ui_auth, UiAuth::Enabled(_))
        {
            return Ok(());
        }
        let password = crate::server::issue_token();
        self.set_oauth_password(password.as_str()).await?;
        tracing::warn!(
            target: "agena::server::mcp",
            password = %password,
            "generated an MCP OAuth password because no operator password is configured; set one in Web/TUI to replace it"
        );
        *self
            .control
            .generated_password
            .write()
            .expect("MCP generated password lock poisoned") = Some(password);
        Ok(())
    }

    /// Consume the generated password so the control plane reports it once.
    fn take_generated_password(&self) -> Option<String> {
        self.control
            .generated_password
            .write()
            .expect("MCP generated password lock poisoned")
            .take()
    }
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct McpServerControlResponse {
    enabled: bool,
    public_url: Option<String>,
    oauth_issuer_url: Option<String>,
    resource_url: String,
    ready: bool,
    warnings: Vec<String>,
    /// Plaintext of a password Agena generated itself. Reported exactly once and
    /// omitted afterwards.
    #[serde(skip_serializing_if = "Option::is_none")]
    generated_oauth_password: Option<String>,
    oauth: McpOAuthControlStatus,
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
    revocation_endpoint: String,
    protected_resource_metadata: String,
    authorization_server_metadata: String,
    jwks_uri: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct UpdateMcpServerControlRequest {
    #[serde(default)]
    enabled: Option<bool>,
    /// `None` means "leave unchanged"; `Some(None)` explicitly clears the
    /// configured public URL and returns to the listener-local fallback.
    #[serde(default)]
    public_url: Option<Option<String>>,
    /// Removed selector. Accepted so clients built against an older control API
    /// keep working; the value is ignored.
    #[serde(default)]
    auth_mode: Option<serde_json::Value>,
    /// Removed selector. Accepted for backward compatibility and ignored.
    #[serde(default)]
    anonymous_access: Option<serde_json::Value>,
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
    if request.auth_mode.is_some() || request.anonymous_access.is_some() {
        tracing::debug!(
            target: "agena::server::mcp",
            "ignoring removed MCP authentication selectors from the control request"
        );
    }
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
    if enabled {
        let effective_resource = state
            .mcp_server
            .public_resource_with_configured(&headers, configured_resource.as_deref());
        if let Err(error) = validate_managed_oauth_identity(
            effective_resource.as_str(),
            configured_issuer.as_deref(),
        ) {
            return control_error(StatusCode::BAD_REQUEST, error);
        }
    }
    if let Err(error) = state
        .mcp_server
        .update_control(enabled, configured_resource, configured_issuer)
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

/// Build the MCP Streamable HTTP route and its OAuth surface.
///
/// The route table is fixed and always serves the same protocol surface: OAuth
/// discovery, the built-in authorization server, and `/mcp`. There is no
/// authentication mode to switch at runtime; whether a given tool call needs a
/// bearer token is decided per tool (see `mcp_tool_access`). The only remaining
/// runtime switch is the MCP kill switch, which removes the whole surface when
/// the server is disabled.
pub(crate) fn router(state: Arc<McpServerState>) -> Router {
    // ChatGPT and Secure MCP Tunnel may probe the endpoint before they have a
    // session ID (for example with server/discover or an early tools/list).
    // A stateless Streamable HTTP server is explicitly allowed by MCP and
    // avoids rmcp's stateful pre-initialize 422 for those requests. JSON
    // responses are also accepted by Streamable HTTP and are easier for
    // connector gateways to proxy than one-request SSE streams.
    let mut stream_config = StreamableHttpServerConfig::default()
        .with_legacy_session_mode(false)
        .with_stateless_protocol_metadata_required(true)
        .with_json_response(true);
    // The public URL can change through Web/TUI while this router is alive.
    // Agena therefore performs a dynamic Host/Origin check in middleware
    // below rather than freezing rmcp's startup-only allowlist. Disabling the
    // SDK gate is safe only because every MCP and OAuth route is covered by
    // `validate_mcp_host_origin`.
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
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_mcp_bearer,
        ))
        // The standard MCP tool model still does not include the Apps SDK's
        // top-level `securitySchemes` extension. Rewrite only tools/list at the
        // HTTP boundary; all protocol negotiation and modern stateless
        // discovery are handled by rmcp 3.x itself.
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            rewrite_tool_security_schemes,
        ))
        .with_state(Arc::clone(&state));

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
            "/.well-known/oauth-protected-resource/{*path}",
            axum::routing::get(protected_resource_metadata),
        )
        .route(
            "/.well-known/oauth-authorization-server",
            axum::routing::get(authorization_server_metadata),
        )
        .route("/.well-known/jwks.json", axum::routing::get(jwks))
        .route("/oauth/jwks.json", axum::routing::get(jwks))
        .route(
            "/oauth/authorize",
            axum::routing::get(authorize_get).post(authorize_post),
        )
        .route("/oauth/token", axum::routing::post(token))
        .route("/oauth/revoke", axum::routing::post(revoke))
        .route("/register", axum::routing::post(register_client))
        .with_state(Arc::clone(&state));

    Router::new()
        .merge(oauth_routes)
        .merge(protected_mcp)
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            validate_mcp_host_origin,
        ))
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            require_mcp_enabled,
        ))
        .with_state(state)
}

fn mcp_body_error_contains_length_limit(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut current = Some(error);
    while let Some(cause) = current {
        if cause.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        current = cause.source();
    }
    false
}

fn mcp_request_body_error(error: &(dyn std::error::Error + 'static)) -> Response {
    let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
        "failed to read MCP request body",
        error,
    );
    if mcp_body_error_contains_length_limit(error) {
        tracing::warn!(
            target: "agena::server::mcp",
            diagnostic = %diagnostic,
            max_bytes = MAX_MCP_METADATA_BODY_BYTES,
            "rejected oversized MCP request body"
        );
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("MCP request body exceeds the {MAX_MCP_METADATA_BODY_BYTES}-byte limit"),
        )
            .into_response();
    }

    tracing::warn!(
        target: "agena::server::mcp",
        diagnostic = %diagnostic,
        "failed to read MCP request body"
    );
    let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 400);
    let public = if public.is_empty() {
        "MCP request body could not be read".to_owned()
    } else {
        public
    };
    (StatusCode::BAD_REQUEST, public).into_response()
}

fn mcp_response_body_error(error: &(dyn std::error::Error + 'static)) -> Response {
    let diagnostic = agena_failure::diagnostic::format_error_chain_with_context(
        "failed to read MCP tools/list response body",
        error,
    );
    tracing::error!(
        target: "agena::server::mcp",
        diagnostic = %diagnostic,
        max_bytes = MAX_MCP_METADATA_BODY_BYTES,
        "MCP tools/list response could not be read for metadata rewriting"
    );
    let public = agena_failure::diagnostic::user_message_with_context(&diagnostic, 400);
    let public = if public.is_empty() {
        "MCP tools/list response could not be read for metadata rewriting".to_owned()
    } else {
        public
    };
    (StatusCode::INTERNAL_SERVER_ERROR, public).into_response()
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
    let (mut parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return mcp_request_body_error(&error),
    };
    let metadata = match request_metadata(&mut parts, body.clone()).await {
        Ok(metadata) => metadata,
        Err(error) => return mcp_worker_unavailable(error),
    };
    let rpc_method = &metadata.rpc_method;
    let has_id = metadata.has_id;
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
            .filter(|tool| self.state.tool_is_exposed(tool))
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
            .any(|tool| tool.name == params.name && self.state.tool_is_exposed(tool))
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
        output_schema: tool.output_schema,
        annotations: Some(serde_json::json!({
            "readOnlyHint": tool.read_only,
            "destructiveHint": tool.destructive,
            "idempotentHint": tool.read_only,
            "openWorldHint": tool.open_world,
        })),
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
    auth_requirements: &HashMap<String, bool>,
) -> Option<serde_json::Value> {
    fn rewrite_message(
        payload: &mut serde_json::Value,
        auth_requirements: &HashMap<String, bool>,
    ) -> bool {
        if let Some(messages) = payload.as_array_mut() {
            let mut changed = false;
            for message in messages {
                changed |= rewrite_message(message, auth_requirements);
            }
            return changed;
        }
        let Some(tools) = payload
            .get_mut("result")
            .and_then(serde_json::Value::as_object_mut)
            .and_then(|result| result.get_mut("tools"))
            .and_then(serde_json::Value::as_array_mut)
        else {
            return false;
        };

        for tool in tools {
            let Some(tool) = tool.as_object_mut() else {
                continue;
            };
            let name = tool
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            // Missing names default closed: a serialized descriptor that does
            // not match the authoritative catalog must never become anonymous.
            let requires_auth = auth_requirements.get(name).copied().unwrap_or(true);
            let security_schemes = if requires_auth {
                serde_json::json!([{
                    "type": "oauth2",
                    "scopes": [MCP_SCOPE],
                }])
            } else {
                serde_json::json!([{"type": "noauth"}])
            };
            tool.insert("securitySchemes".to_owned(), security_schemes);
        }
        true
    }

    rewrite_message(&mut payload, auth_requirements).then_some(payload)
}

fn contains_tools_list(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.get("method").and_then(serde_json::Value::as_str) == Some("tools/list")
        }
        serde_json::Value::Array(messages) => messages.iter().any(contains_tools_list),
        _ => false,
    }
}

fn jsonrpc_method(value: &serde_json::Value) -> Option<&str> {
    match value {
        serde_json::Value::Object(object) => {
            object.get("method").and_then(serde_json::Value::as_str)
        }
        serde_json::Value::Array(messages) => messages.first().and_then(jsonrpc_method),
        _ => None,
    }
}

fn jsonrpc_request_id(value: &serde_json::Value) -> Option<&serde_json::Value> {
    match value {
        serde_json::Value::Object(object) => object.get("id").filter(|id| !id.is_null()),
        serde_json::Value::Array(messages) => messages.first().and_then(jsonrpc_request_id),
        _ => None,
    }
}

fn rewrite_tool_list_json(
    body: &[u8],
    auth_requirements: &HashMap<String, bool>,
) -> Option<Vec<u8>> {
    let payload = match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::warn!(
                target: "agena::server::mcp",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode MCP tools/list JSON response for metadata rewriting",
                    &error,
                ),
                "MCP tools/list JSON metadata was not rewritten"
            );
            return None;
        }
    };
    let payload = add_tool_security_schemes(payload, auth_requirements)?;
    match serde_json::to_vec(&payload) {
        Ok(payload) => Some(payload),
        Err(error) => {
            tracing::error!(
                target: "agena::server::mcp",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize rewritten MCP tools/list JSON response",
                    &error,
                ),
                "rewritten MCP tools/list JSON response could not be serialized"
            );
            None
        }
    }
}

fn rewrite_tool_list_sse(
    body: &[u8],
    auth_requirements: &HashMap<String, bool>,
) -> Option<Vec<u8>> {
    let text = match std::str::from_utf8(body) {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(
                target: "agena::server::mcp",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode MCP tools/list SSE response as UTF-8",
                    &error,
                ),
                "MCP tools/list SSE metadata was not rewritten"
            );
            return None;
        }
    };
    let mut changed = false;
    let mut output = String::with_capacity(text.len());

    for line in text.split_inclusive('\n') {
        let line_without_newline = line.strip_suffix('\n').unwrap_or(line);
        let line_without_newline = line_without_newline
            .strip_suffix('\r')
            .unwrap_or(line_without_newline);
        if let Some(data) = line_without_newline.strip_prefix("data:") {
            match serde_json::from_str::<serde_json::Value>(data.trim_start()) {
                Ok(payload) => {
                    if let Some(payload) = add_tool_security_schemes(payload, auth_requirements) {
                        let payload = match serde_json::to_string(&payload) {
                            Ok(payload) => payload,
                            Err(error) => {
                                tracing::error!(
                                    target: "agena::server::mcp",
                                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                        "serialize rewritten MCP tools/list SSE event",
                                        &error,
                                    ),
                                    "rewritten MCP tools/list SSE event could not be serialized"
                                );
                                return None;
                            }
                        };
                        let newline = &line[line_without_newline.len()..];
                        output.push_str("data: ");
                        output.push_str(payload.as_str());
                        output.push_str(newline);
                        changed = true;
                        continue;
                    }
                }
                Err(error) => {
                    tracing::debug!(
                        target: "agena::server::mcp",
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "decode MCP tools/list SSE data event",
                            &error,
                        ),
                        "non-JSON MCP SSE data event was preserved without metadata rewriting"
                    );
                }
            }
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

    let (mut parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return mcp_request_body_error(&error),
    };
    let metadata = match request_metadata(&mut parts, body.clone()).await {
        Ok(metadata) => metadata,
        Err(error) => return mcp_worker_unavailable(error),
    };
    let should_rewrite = metadata.tools_list;
    let auth_requirements = if should_rewrite {
        state.exposed_tool_auth_requirements().await
    } else {
        HashMap::new()
    };
    let request = axum::http::Request::from_parts(parts, Body::from(body));
    let response = next.run(request).await;
    if should_rewrite {
        rewrite_tool_list_response(response, auth_requirements).await
    } else {
        response
    }
}

async fn rewrite_tool_list_response(
    response: Response,
    auth_requirements: HashMap<String, bool>,
) -> Response {
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
        Err(error) => return mcp_response_body_error(&error),
    };
    let (body, rewritten) = match MCP_REWRITES
        .run(move || {
            let rewritten = if is_json {
                rewrite_tool_list_json(&body, &auth_requirements)
            } else {
                rewrite_tool_list_sse(&body, &auth_requirements)
            };
            (body, rewritten)
        })
        .await
    {
        Ok(result) => result,
        Err(error) => {
            return mcp_worker_unavailable(format!("MCP metadata rewrite worker failed: {error}"));
        }
    };
    let Some(rewritten) = rewritten else {
        return Response::from_parts(parts, Body::from(body));
    };

    let mut response = Response::from_parts(parts, Body::from(rewritten));
    response.headers_mut().remove(header::CONTENT_LENGTH);
    response
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
    revocation_endpoint_auth_methods_supported: Vec<String>,
    client_id_metadata_document_supported: bool,
    authorization_response_iss_parameter_supported: bool,
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
    let resource = state.public_resource(&headers);
    let issuer = state.issuer_for_headers(&headers);
    json_no_store(
        StatusCode::OK,
        ProtectedResourceMetadata {
            authorization_servers: vec![issuer],
            resource,
            scopes_supported: vec![MCP_SCOPE.to_owned(), OFFLINE_ACCESS_SCOPE.to_owned()],
            bearer_methods_supported: vec!["header".to_owned()],
        },
    )
}

async fn authorization_server_metadata(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
) -> Response {
    let issuer = state.issuer_for_headers(&headers);
    json_no_store(
        StatusCode::OK,
        AuthorizationServerMetadata {
            authorization_endpoint: append_public_endpoint(&issuer, "/oauth/authorize"),
            authorization_response_iss_parameter_supported: true,
            client_id_metadata_document_supported: true,
            code_challenge_methods_supported: vec!["S256".to_owned()],
            registration_endpoint: append_public_endpoint(&issuer, "/register"),
            grant_types_supported: vec![
                "authorization_code".to_owned(),
                "refresh_token".to_owned(),
            ],
            issuer: issuer.clone(),
            response_types_supported: vec!["code".to_owned()],
            revocation_endpoint: append_public_endpoint(&issuer, "/oauth/revoke"),
            revocation_endpoint_auth_methods_supported: vec!["none".to_owned()],
            scopes_supported: vec![MCP_SCOPE.to_owned(), OFFLINE_ACCESS_SCOPE.to_owned()],
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
    json_no_store(StatusCode::OK, state.oauth.signing_key.jwks())
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
    // ChatGPT's production CIMD currently publishes a plural capability set
    // containing both `none` and `private_key_jwt`, while the singular field
    // prefers `private_key_jwt`. The plural field is authoritative for
    // capability intersection: Agena may select `none` because its token
    // endpoint advertises and implements that public-client method.
    match plural {
        Some(methods) => methods.iter().any(|method| method == "none"),
        None => singular == Some("none"),
    }
}

fn client_metadata_supports_grants(grants: Option<&[String]>) -> bool {
    grants.is_none_or(|values| {
        !values.is_empty()
            && values.iter().any(|value| value == "authorization_code")
            && values
                .iter()
                .all(|value| matches!(value.as_str(), "authorization_code" | "refresh_token"))
    })
}

fn client_metadata_supports_code_response(response_types: Option<&[String]>) -> bool {
    response_types.is_none_or(|values| {
        !values.is_empty()
            && values.iter().any(|value| value == "code")
            && values.iter().all(|value| value == "code")
    })
}

async fn fetch_cimd_document(client_id: &str) -> Result<CimdClientMetadata, String> {
    if !is_supported_cimd_client_id(client_id) {
        return Err("client id is not a usable metadata document URL".to_owned());
    }
    let metadata_url = Url::parse(client_id.trim())
        .map_err(|error| format!("client metadata URL is invalid: {error}"))?;
    let host = metadata_url
        .host_str()
        .ok_or_else(|| "client metadata URL must include a host".to_owned())?;
    ensure_public_metadata_host(host, metadata_url.port_or_known_default().unwrap_or(443)).await?;

    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(8))
        .build()
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "build the CIMD metadata HTTP client",
                &error,
            )
        })?;
    let response = client
        .get(client_id)
        .header(header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "fetch the CIMD metadata document",
                &error,
            )
        })?;
    if !response.status().is_success() {
        return Err(format!(
            "CIMD metadata endpoint returned HTTP {}",
            response.status()
        ));
    }
    if let Some(value) = response.headers().get(header::CONTENT_TYPE) {
        let value = value.to_str().map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "decode the CIMD metadata Content-Type header",
                &error,
            )
        })?;
        let media_type = value.split(';').next().unwrap_or_default().trim();
        if !media_type.eq_ignore_ascii_case("application/json") && !media_type.ends_with("+json") {
            return Err(format!(
                "CIMD metadata endpoint returned unsupported Content-Type {value:?}"
            ));
        }
    }
    if response
        .content_length()
        .is_some_and(|length| length > MAX_CIMD_DOCUMENT_BYTES as u64)
    {
        return Err(format!(
            "CIMD metadata document exceeded the {MAX_CIMD_DOCUMENT_BYTES}-byte limit"
        ));
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            agena_failure::diagnostic::format_error_chain_with_context(
                "read the CIMD metadata response body",
                &error,
            )
        })?;
        if body.len().saturating_add(chunk.len()) > MAX_CIMD_DOCUMENT_BYTES {
            return Err(format!(
                "CIMD metadata document exceeded the {MAX_CIMD_DOCUMENT_BYTES}-byte limit while streaming"
            ));
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice::<CimdClientMetadata>(&body).map_err(|error| {
        agena_failure::diagnostic::format_error_chain_with_context(
            "decode the CIMD metadata document",
            &error,
        )
    })
}

fn validate_cimd_document(client_id: &str, metadata: &CimdClientMetadata) -> Result<(), String> {
    if metadata.client_id.as_deref() != Some(client_id) {
        return Err("CIMD document client_id did not match its metadata URL".to_owned());
    }
    if metadata.redirect_uris.is_empty() || metadata.redirect_uris.len() > 16 {
        return Err("CIMD document must declare between 1 and 16 redirect URIs".to_owned());
    }
    if metadata
        .client_name
        .as_deref()
        .is_some_and(|name| name.trim().is_empty() || name.len() > MAX_OAUTH_LABEL_BYTES)
    {
        return Err("CIMD document client_name is empty or too long".to_owned());
    }
    for (index, redirect_uri) in metadata.redirect_uris.iter().enumerate() {
        if redirect_uri.len() > MAX_OAUTH_URI_BYTES {
            return Err(format!(
                "CIMD redirect URI {index} exceeds the length limit"
            ));
        }
        if let Err(error) = validate_redirect_uri(redirect_uri) {
            return Err(format!("CIMD redirect URI {index} is not allowed: {error}"));
        }
    }
    if !client_metadata_supports_none(
        metadata.token_endpoint_auth_method.as_deref(),
        metadata.token_endpoint_auth_methods_supported.as_deref(),
    ) {
        return Err("CIMD document does not support token endpoint auth method none".to_owned());
    }
    if !client_metadata_supports_grants(metadata.grant_types.as_deref()) {
        return Err("CIMD document does not support the required OAuth grant types".to_owned());
    }
    if !client_metadata_supports_code_response(metadata.response_types.as_deref()) {
        return Err("CIMD document does not support the OAuth code response type".to_owned());
    }
    Ok(())
}

async fn load_cimd_client(
    state: &McpServerState,
    client_id: &str,
) -> Result<CimdClientMetadata, String> {
    if !is_supported_cimd_client_id(client_id) {
        return Err("CIMD client id is outside the supported metadata namespace".to_owned());
    }
    let now = Instant::now();
    state
        .oauth
        .cimd_clients
        .retain(|_, cached| cached.expires_at > now);
    if let Some(cached) = state.oauth.cimd_clients.get(client_id) {
        return Ok(cached.metadata.clone());
    }

    // Serialize cache misses so repeated authorization requests for one client
    // cannot fan out into duplicate public fetches, and cap arbitrary callback
    // IDs so the unauthenticated authorization endpoint cannot grow memory
    // without bound.
    let _fetch_guard = state.oauth.cimd_fetch_lock.lock().await;
    let now = Instant::now();
    state
        .oauth
        .cimd_clients
        .retain(|_, cached| cached.expires_at > now);
    if let Some(cached) = state.oauth.cimd_clients.get(client_id) {
        return Ok(cached.metadata.clone());
    }
    if state.oauth.cimd_clients.len() >= MAX_CACHED_CIMD_CLIENTS {
        return Err("CIMD metadata cache capacity has been reached".to_owned());
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

/// One dynamically registered OAuth client (RFC 7591).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisteredClient {
    client_id: String,
    #[serde(default)]
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    created_at: u64,
}

#[derive(Debug, Deserialize)]
struct RegisterClientRequest {
    redirect_uris: Vec<String>,
    #[serde(default)]
    client_name: Option<String>,
    #[serde(default)]
    token_endpoint_auth_method: Option<String>,
    #[serde(default)]
    grant_types: Option<Vec<String>>,
    #[serde(default)]
    response_types: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
struct RegisterClientResponse {
    client_id: String,
    client_id_issued_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_name: Option<String>,
    redirect_uris: Vec<String>,
    token_endpoint_auth_method: &'static str,
    grant_types: Vec<String>,
    response_types: Vec<String>,
    scope: String,
}

/// Register a public OAuth client, the way MCP clients that do not publish a
/// client metadata document expect to start.
///
/// Agena only issues public clients: the token endpoint accepts
/// `token_endpoint_auth_method=none` with PKCE, so no client secret is minted,
/// stored, or rotated. Registrations are capped and the oldest is evicted, which
/// keeps an unauthenticated endpoint from growing state without bound.
async fn register_client(
    State(state): State<Arc<McpServerState>>,
    request: Result<Json<RegisterClientRequest>, axum::extract::rejection::JsonRejection>,
) -> Response {
    let Json(request) = match request {
        Ok(request) => request,
        Err(error) => {
            tracing::debug!(
                target: "agena::server::mcp",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "decode the OAuth client registration body",
                    &error,
                ),
                "rejecting malformed OAuth client registration"
            );
            return registration_error(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "the registration body must be a JSON object",
            );
        }
    };

    if request.redirect_uris.is_empty() || request.redirect_uris.len() > 16 {
        return registration_error(
            StatusCode::BAD_REQUEST,
            "invalid_redirect_uri",
            "redirect_uris must contain between 1 and 16 URIs",
        );
    }
    let mut redirect_uris = Vec::with_capacity(request.redirect_uris.len());
    for uri in &request.redirect_uris {
        if uri.len() > MAX_OAUTH_URI_BYTES {
            return registration_error(
                StatusCode::BAD_REQUEST,
                "invalid_redirect_uri",
                "a redirect URI exceeds the length limit",
            );
        }
        if let Err(error) = validate_redirect_uri(uri.as_str()) {
            return registration_error(StatusCode::BAD_REQUEST, "invalid_redirect_uri", error);
        }
        redirect_uris.push(uri.clone());
    }

    if request
        .token_endpoint_auth_method
        .as_deref()
        .is_some_and(|method| method != "none")
    {
        return registration_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "only token_endpoint_auth_method=none is supported",
        );
    }
    let grant_types = match request.grant_types {
        None => vec!["authorization_code".to_owned(), "refresh_token".to_owned()],
        Some(values)
            if !values.is_empty()
                && values.iter().all(|value| {
                    matches!(value.as_str(), "authorization_code" | "refresh_token")
                }) =>
        {
            values
        }
        Some(_) => {
            return registration_error(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "grant_types must be a non-empty subset of authorization_code and refresh_token",
            );
        }
    };
    let response_types = match request.response_types {
        None => vec!["code".to_owned()],
        Some(values) if !values.is_empty() && values.iter().all(|value| value == "code") => values,
        Some(_) => {
            return registration_error(
                StatusCode::BAD_REQUEST,
                "invalid_client_metadata",
                "response_types must be a non-empty subset of code",
            );
        }
    };
    let client_name = request.client_name.filter(|name| !name.trim().is_empty());
    if client_name
        .as_deref()
        .is_some_and(|name| name.len() > MAX_OAUTH_LABEL_BYTES)
    {
        return registration_error(
            StatusCode::BAD_REQUEST,
            "invalid_client_metadata",
            "client_name is too long",
        );
    }

    if state.oauth.registered_clients.len() >= MAX_REGISTERED_CLIENTS
        && let Some(oldest) = state
            .oauth
            .registered_clients
            .iter()
            .min_by_key(|entry| entry.value().created_at)
            .map(|entry| entry.key().clone())
    {
        state.oauth.registered_clients.remove(oldest.as_str());
    }

    let client_id = crate::server::issue_token();
    let created_at = unix_timestamp();
    state.oauth.registered_clients.insert(
        client_id.clone(),
        RegisteredClient {
            client_id: client_id.clone(),
            client_name: client_name.clone(),
            redirect_uris: redirect_uris.clone(),
            created_at,
        },
    );
    if let Err(error) = state.oauth.persist().await {
        tracing::error!(
            diagnostic = %error,
            "OAuth client registration could not be persisted"
        );
        return registration_error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "server_error",
            "the authorization server could not persist the client registration",
        );
    }

    json_no_store(
        StatusCode::CREATED,
        RegisterClientResponse {
            client_id,
            client_id_issued_at: created_at,
            client_name,
            redirect_uris,
            token_endpoint_auth_method: "none",
            grant_types,
            response_types,
            scope: DEFAULT_OAUTH_SCOPE.to_owned(),
        },
    )
}

fn registration_error(
    status: StatusCode,
    error: &'static str,
    description: impl Into<String>,
) -> Response {
    json_no_store(
        status,
        serde_json::json!({
            "error": error,
            "error_description": description.into(),
        }),
    )
}

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

#[derive(Debug, Clone)]
struct ValidatedAuthorizationClient {
    client_id: String,
    client_name: Option<String>,
    redirect_uri: String,
    issuer: String,
}

#[derive(Debug, Clone)]
enum AuthorizationValidationError {
    /// The client identity or redirect URI is not trustworthy, so returning an
    /// OAuth redirect would create an open redirect or leak request details.
    Local(OAuthRequestError),
    /// The client and redirect URI are already validated. RFC 9207 requires the
    /// authorization-server issuer on both successful and error callbacks when
    /// the metadata advertises issuer identification support.
    Redirect {
        redirect_uri: String,
        state: Option<String>,
        issuer: String,
        error: OAuthRequestError,
    },
}

impl AuthorizationValidationError {
    fn into_response(self) -> Response {
        match self {
            Self::Local(error) => error.into_response(),
            Self::Redirect {
                redirect_uri,
                state,
                issuer,
                error,
            } => authorization_error_redirect(
                redirect_uri.as_str(),
                state.as_deref(),
                issuer.as_str(),
                &error,
            ),
        }
    }
}

async fn validate_authorization_client(
    state: &McpServerState,
    headers: &HeaderMap,
    request: &AuthorizeRequest,
) -> Result<ValidatedAuthorizationClient, OAuthRequestError> {
    let client_id = request
        .client_id
        .as_deref()
        .filter(|value| !value.trim().is_empty() && value.len() <= MAX_OAUTH_URI_BYTES)
        .ok_or_else(|| {
            OAuthRequestError::new(
                "invalid_request",
                "client_id is required and must be at most 4096 bytes",
            )
        })?;
    // A client id resolves in two ways: a dynamically registered public client
    // (RFC 7591) or a client metadata document URL (CIMD) hosted by the client.
    // Both reduce to the same validated redirect-URI list.
    let (client_name, client_redirect_uris) = match state
        .oauth
        .registered_clients
        .get(client_id)
        .map(|entry| (entry.client_name.clone(), entry.redirect_uris.clone()))
    {
        Some(registered) => registered,
        None => {
            let cimd_client = load_cimd_client(state, client_id).await.map_err(|error| {
                tracing::warn!(
                    client_id,
                    diagnostic = %error,
                    "client metadata document validation failed"
                );
                OAuthRequestError::new(
                    "invalid_client",
                    "the client is not registered and its metadata document could not be validated",
                )
            })?;
            (cimd_client.client_name, cimd_client.redirect_uris)
        }
    };
    let redirect_uri = request
        .redirect_uri
        .as_deref()
        .filter(|value| !value.trim().is_empty() && value.len() <= MAX_OAUTH_URI_BYTES)
        .ok_or_else(|| {
            OAuthRequestError::new(
                "invalid_request",
                "redirect_uri is required and must be at most 4096 bytes",
            )
        })?;
    let redirect_matches_client = client_redirect_uris.iter().any(|uri| uri == redirect_uri);
    if !redirect_matches_client {
        return Err(OAuthRequestError::new(
            "invalid_request",
            "redirect_uri is not registered for this client",
        ));
    }
    let client_name = client_name.or_else(|| Some("MCP client".to_owned()));
    Ok(ValidatedAuthorizationClient {
        client_id: client_id.to_owned(),
        client_name,
        redirect_uri: redirect_uri.to_owned(),
        issuer: state.issuer_for_headers(headers),
    })
}

async fn validate_authorization_request(
    state: &McpServerState,
    headers: &HeaderMap,
    request: &AuthorizeRequest,
) -> Result<ValidatedAuthorizationRequest, AuthorizationValidationError> {
    let client = validate_authorization_client(state, headers, request)
        .await
        .map_err(AuthorizationValidationError::Local)?;
    let redirect_error = |error| AuthorizationValidationError::Redirect {
        redirect_uri: client.redirect_uri.clone(),
        state: request.state.clone(),
        issuer: client.issuer.clone(),
        error,
    };

    if request.response_type.as_deref() != Some("code") {
        return Err(redirect_error(OAuthRequestError::new(
            "unsupported_response_type",
            "response_type=code is required",
        )));
    }
    let state_value = request
        .state
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_OAUTH_URI_BYTES)
        .ok_or_else(|| {
            redirect_error(OAuthRequestError::new(
                "invalid_request",
                "state is required and must be at most 4096 bytes",
            ))
        })?;
    let code_challenge = request
        .code_challenge
        .as_deref()
        .filter(|value| valid_pkce_challenge(value))
        .ok_or_else(|| {
            redirect_error(OAuthRequestError::new(
                "invalid_request",
                "a valid S256 code_challenge is required",
            ))
        })?;
    if request.code_challenge_method.as_deref() != Some("S256") {
        return Err(redirect_error(OAuthRequestError::new(
            "invalid_request",
            "code_challenge_method=S256 is required",
        )));
    }
    // RFC 8707 makes the resource indicator optional. A client that omits it
    // gets a grant bound to this MCP resource, so audience restriction still
    // holds for clients that never send one; an explicit value must match.
    let expected_resource = state.public_resource(headers);
    let resource = match request
        .resource
        .as_deref()
        .filter(|value| !value.is_empty())
    {
        Some(resource) if resource != expected_resource => {
            return Err(redirect_error(OAuthRequestError::new(
                "invalid_target",
                "resource does not match the MCP resource metadata",
            )));
        }
        Some(resource) => resource,
        None => expected_resource.as_str(),
    };
    let scope = normalize_scope(request.scope.as_deref()).map_err(redirect_error)?;
    Ok(ValidatedAuthorizationRequest {
        client_id: client.client_id,
        client_name: client.client_name,
        redirect_uri: client.redirect_uri,
        state: state_value.to_owned(),
        code_challenge: code_challenge.to_owned(),
        issuer: client.issuer,
        resource: resource.to_owned(),
        scope,
    })
}

async fn authorize_get(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    Query(request): Query<AuthorizeRequest>,
) -> Response {
    match validate_authorization_request(&state, &headers, &request).await {
        Ok(request) => authorization_html(
            StatusCode::OK,
            render_authorization_page(&request, None),
            request.redirect_uri.as_str(),
        ),
        Err(error) => error.into_response(),
    }
}

async fn authorize_post(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    request: Result<Form<AuthorizeRequest>, axum::extract::rejection::FormRejection>,
) -> Response {
    let Form(request) = match request {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "MCP OAuth authorization form is invalid",
                    &error,
                ),
                "rejecting invalid MCP OAuth authorization request"
            );
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
        let mut response = authorization_html(
            status,
            render_authorization_page(&validated, Some(message)),
            validated.redirect_uri.as_str(),
        );
        if let OAuthPasswordError::Locked(seconds) = error
            && let Ok(value) = HeaderValue::try_from(seconds.to_string())
        {
            response.headers_mut().insert(header::RETRY_AFTER, value);
        }
        return response;
    }

    let _authorization_guard = state.oauth.authorization_lock.lock().await;
    state.oauth.purge_expired();
    if state.oauth.authorization_codes.len() >= MAX_OUTSTANDING_AUTHORIZATION_CODES {
        return authorization_error_redirect(
            validated.redirect_uri.as_str(),
            Some(validated.state.as_str()),
            validated.issuer.as_str(),
            &OAuthRequestError::new(
                "temporarily_unavailable",
                "too many outstanding authorization requests; retry shortly",
            ),
        );
    }
    let code = crate::server::issue_token();
    let callback_issuer = validated.issuer.clone();
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
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to rebuild a previously validated MCP OAuth redirect URI",
                    &error,
                ),
                "MCP OAuth redirect URI invariant failed"
            );
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
        .append_pair("state", validated.state.as_str())
        .append_pair("iss", callback_issuer.as_str());
    // The authorization form is submitted with POST. A 303 explicitly tells
    // ChatGPT (and a browser) to follow the callback with GET instead of
    // replaying the password-bearing POST as a 307 would do.
    let mut response = Redirect::to(redirect.as_str()).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn authorization_error_redirect(
    redirect_uri: &str,
    state: Option<&str>,
    issuer: &str,
    error: &OAuthRequestError,
) -> Response {
    let mut redirect = match Url::parse(redirect_uri) {
        Ok(redirect) => redirect,
        Err(parse_error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "failed to rebuild an MCP OAuth error redirect URI",
                    &parse_error,
                ),
                "returning MCP OAuth error without redirect"
            );
            return error.clone().into_response();
        }
    };
    {
        let mut query = redirect.query_pairs_mut();
        query
            .append_pair("error", error.error)
            .append_pair("error_description", error.description)
            .append_pair("iss", issuer);
        if let Some(state) = state.filter(|value| !value.is_empty()) {
            query.append_pair("state", state);
        }
    }
    let mut response = Redirect::to(redirect.as_str()).into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
        .headers_mut()
        .insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    response
}

fn authorization_html(status: StatusCode, html: String, redirect_uri: &str) -> Response {
    let mut response = Html(html).into_response();
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        axum::http::HeaderName::from_static("content-security-policy"),
        authorization_content_security_policy(redirect_uri),
    );
    headers.insert(
        axum::http::HeaderName::from_static("x-frame-options"),
        HeaderValue::from_static("DENY"),
    );
    headers.insert(
        axum::http::HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("no-referrer"),
    );
    response
}

fn authorization_content_security_policy(redirect_uri: &str) -> HeaderValue {
    let redirect_origin = Url::parse(redirect_uri)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| "'none'".to_owned());
    let policy = format!(
        "default-src 'none'; style-src 'unsafe-inline'; form-action 'self' {redirect_origin}; base-uri 'none'; frame-ancestors 'none'"
    );
    HeaderValue::try_from(policy).unwrap_or_else(|_| {
        HeaderValue::from_static(
            "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'; frame-ancestors 'none'",
        )
    })
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
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Authorize Agena MCP</title><style>body{{font-family:system-ui,sans-serif;max-width: thirtyrem;max-width:30rem;margin:4rem auto;padding:0 1.25rem;color:#202123}}main{{border:1px solid #ddd;border-radius:12px;padding:2rem;box-shadow:0 4px 24px #00000012}}input[type=password]{{display:block;width:100%;box-sizing:border-box;padding:.7rem;border:1px solid #aaa;border-radius:6px;margin:.5rem 0 1rem}}button{{padding:.7rem 1.1rem;border:0;border-radius:6px;background:#111;color:white;cursor:pointer}}.error{{color:#b42318;background:#fff1f0;padding:.7rem;border-radius:6px}}</style></head><body><main><h1>Authorize Agena MCP</h1><p><strong>{}</strong> is requesting access to Agena tools.</p><p>Resource: <code>{}</code></p><p>Scope: <code>{}</code></p>{}<form method=\"post\" action=\"authorize\">{}{}{}{}{}{}{}{}<label for=\"password\">Agena server password</label><input id=\"password\" name=\"password\" type=\"password\" autocomplete=\"current-password\" required autofocus><button type=\"submit\">Authorize</button></form></main></body></html>",
        html_escape(client_label),
        html_escape(request.resource.as_str()),
        html_escape(request.scope.as_str()),
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
#[serde(deny_unknown_fields)]
struct RefreshTokenRecord {
    client_id: String,
    issuer: String,
    resource: String,
    scope: String,
    expires_at: u64,
    family_id: String,
    used: bool,
    revoked: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PersistedOAuthRuntime {
    refresh_tokens: HashMap<String, RefreshTokenRecord>,
    revoked_jti: HashMap<String, u64>,
    #[serde(default)]
    clients: Vec<RegisteredClient>,
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
    cimd_clients: DashMap<String, CachedCimdClient>,
    registered_clients: DashMap<String, RegisteredClient>,
    authorization_codes: DashMap<String, AuthorizationCodeRecord>,
    refresh_tokens: DashMap<String, RefreshTokenRecord>,
    revoked_jti: DashMap<String, u64>,
    authorization_lock: tokio::sync::Mutex<()>,
    cimd_fetch_lock: tokio::sync::Mutex<()>,
    refresh_rotation_lock: tokio::sync::Mutex<()>,
    persist_lock: tokio::sync::Mutex<()>,
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
            cimd_clients: DashMap::new(),
            registered_clients: DashMap::new(),
            authorization_codes: DashMap::new(),
            refresh_tokens: DashMap::new(),
            revoked_jti: DashMap::new(),
            authorization_lock: tokio::sync::Mutex::new(()),
            cimd_fetch_lock: tokio::sync::Mutex::new(()),
            refresh_rotation_lock: tokio::sync::Mutex::new(()),
            persist_lock: tokio::sync::Mutex::new(()),
            signing_key,
            database,
        };
        if let Some(persisted) = persisted {
            for (token, record) in persisted.refresh_tokens {
                state.refresh_tokens.insert(token, record);
            }
            for (jti, expires_at) in persisted.revoked_jti {
                state.revoked_jti.insert(jti, expires_at);
            }
            for client in persisted.clients {
                state
                    .registered_clients
                    .insert(client.client_id.clone(), client);
            }
        }
        state
    }

    async fn persist(&self) -> Result<(), String> {
        let Some(database) = self.database.as_ref() else {
            return Ok(());
        };
        // Snapshot and write under one lock so concurrent token/revocation
        // updates cannot overwrite newer persisted state.
        let _persist_guard = self.persist_lock.lock().await;
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
        let clients = self
            .registered_clients
            .iter()
            .map(|entry| entry.value().clone())
            .collect();
        database
            .set_json(
                KV_KEY_MCP_OAUTH_RUNTIME,
                PersistedOAuthRuntime {
                    refresh_tokens,
                    revoked_jti,
                    clients,
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
        if self.refresh_tokens.len() >= MAX_REFRESH_TOKEN_RECORDS {
            return Err("the MCP OAuth refresh-token limit has been reached".to_owned());
        }
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
            refresh_token_fingerprint(refresh_token.as_str()),
            RefreshTokenRecord {
                client_id: client_id.to_owned(),
                issuer: issuer.to_owned(),
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

    fn verified_access_token_claims(&self, token: &str) -> Option<AccessTokenClaims> {
        let untrusted = jsonwebtoken::dangerous::insecure_decode::<AccessTokenClaims>(token)
            .ok()?
            .claims;
        self.validate_access_token(token, untrusted.iss.as_str(), untrusted.resource.as_str())
            .then_some(untrusted)
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
    let Form(request) = match request {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "MCP OAuth token form is invalid",
                    &error,
                ),
                "rejecting invalid MCP OAuth token request"
            );
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the token request body must be a valid form",
            );
        }
    };
    // Defence in depth: never mint a token over a transport identity the
    // deployment could not advertise safely. Startup and control updates already
    // enforce this, but the effective identity can drift with the listener.
    if !is_acceptable_oauth_origin(state.public_resource(&headers).as_str()) {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_target",
            "the MCP resource URL must be HTTPS (loopback HTTP is allowed for local use)",
        );
    }
    if request.grant_type.len() > 64 {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "grant_type is too long",
        );
    }
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
        .filter(|value| !value.is_empty() && value.len() <= MAX_OAUTH_URI_BYTES)
        .ok_or_else(|| {
            OAuthTokenError::new(
                "invalid_request",
                "client_id is required and must be at most 4096 bytes",
            )
        })?;
    let code = request
        .code
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_OAUTH_TOKEN_BYTES)
        .ok_or_else(|| {
            OAuthTokenError::new(
                "invalid_request",
                "code is required and must be at most 8192 bytes",
            )
        })?;
    let (_, record) = state
        .oauth
        .authorization_codes
        .remove(code)
        .ok_or_else(|| {
            OAuthTokenError::new("invalid_grant", "authorization code is invalid or expired")
        })?;
    if record.expires_at <= Instant::now()
        || record.client_id != client_id
        || request
            .redirect_uri
            .as_deref()
            .filter(|value| value.len() <= MAX_OAUTH_URI_BYTES)
            != Some(record.redirect_uri.as_str())
        || request
            .resource
            .as_deref()
            .filter(|value| value.len() <= MAX_OAUTH_URI_BYTES)
            != Some(record.resource.as_str())
    {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "authorization code binding is invalid",
        ));
    }
    let verifier = request
        .code_verifier
        .as_deref()
        .filter(|value| (43..=128).contains(&value.len()))
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
        .map_err(|error| {
            tracing::error!(
                diagnostic = %error,
                "MCP OAuth access-token issuance failed"
            );
            OAuthTokenError::new(
                "server_error",
                "the authorization server could not issue an access token",
            )
        })?;
    state.oauth.persist().await.map_err(|error| {
        tracing::error!(
            diagnostic = %error,
            "MCP OAuth issued token could not be persisted"
        );
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
        .filter(|value| !value.is_empty() && value.len() <= MAX_OAUTH_URI_BYTES)
        .ok_or_else(|| {
            OAuthTokenError::new(
                "invalid_request",
                "client_id is required and must be at most 4096 bytes",
            )
        })?;
    let refresh_token = request
        .refresh_token
        .as_deref()
        .filter(|value| !value.is_empty() && value.len() <= MAX_OAUTH_TOKEN_BYTES)
        .ok_or_else(|| {
            OAuthTokenError::new(
                "invalid_request",
                "refresh_token is required and must be at most 8192 bytes",
            )
        })?;
    let refresh_key = refresh_token_fingerprint(refresh_token);
    // Serialize rotation so two concurrent uses of the same refresh token can
    // never both observe `used == false` and mint independent token families.
    let _rotation_guard = state.oauth.refresh_rotation_lock.lock().await;
    let mut current = state
        .oauth
        .refresh_tokens
        .get_mut(refresh_key.as_str())
        .ok_or_else(|| {
            OAuthTokenError::new("invalid_grant", "refresh token is invalid or expired")
        })?;
    if current.expires_at <= unix_timestamp()
        || current.client_id != client_id
        || current.issuer != state.issuer_for_headers(headers)
        || request
            .resource
            .as_deref()
            .filter(|value| value.len() <= MAX_OAUTH_URI_BYTES)
            != Some(current.resource.as_str())
        || current.revoked
    {
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "refresh token binding is invalid",
        ));
    }
    if current.used {
        let family_id = current.family_id.clone();
        drop(current);
        for mut entry in state.oauth.refresh_tokens.iter_mut() {
            if entry.family_id == family_id {
                entry.revoked = true;
            }
        }
        if let Err(error) = state.oauth.persist().await {
            tracing::error!(
                error = %error,
                family_id = %family_id,
                "failed to persist revoked MCP OAuth refresh-token family after replay detection"
            );
            return Err(OAuthTokenError::new(
                "server_error",
                "the authorization server could not persist the revoked token family",
            ));
        }
        return Err(OAuthTokenError::new(
            "invalid_grant",
            "refresh token replay detected; the token family has been revoked",
        ));
    }
    if let Some(scope) = request.scope.as_deref()
        && normalize_scope(Some(scope)).ok().as_deref() != Some(current.scope.as_str())
    {
        return Err(OAuthTokenError::new(
            "invalid_scope",
            "requested scope is not granted",
        ));
    }
    current.used = true;
    let record = current.clone();
    drop(current);
    let response = state
        .oauth
        .issue_tokens(
            client_id,
            record.issuer.as_str(),
            record.resource.as_str(),
            record.scope.as_str(),
            Some(record.family_id.as_str()),
        )
        .map_err(|error| {
            tracing::error!(
                diagnostic = %error,
                "MCP OAuth refresh-token rotation failed during token issuance"
            );
            OAuthTokenError::new(
                "server_error",
                "the authorization server could not issue an access token",
            )
        })?;
    state.oauth.persist().await.map_err(|error| {
        tracing::error!(
            diagnostic = %error,
            "MCP OAuth rotated refresh token could not be persisted"
        );
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
    let Form(request) = match request {
        Ok(request) => request,
        Err(error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "MCP OAuth revocation form is invalid",
                    &error,
                ),
                "rejecting invalid MCP OAuth revocation request"
            );
            return oauth_error(
                StatusCode::BAD_REQUEST,
                "invalid_request",
                "the revocation request body must be a valid form",
            );
        }
    };
    if request.token.trim().is_empty() || request.token.len() > MAX_OAUTH_TOKEN_BYTES {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "token is required and must be at most 8192 bytes",
        );
    }
    if let Some(client_id) = request.client_id.as_deref()
        && client_id.len() > MAX_OAUTH_URI_BYTES
    {
        return oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "client_id must be at most 4096 bytes",
        );
    }
    if let Some(claims) = state
        .oauth
        .verified_access_token_claims(request.token.as_str())
        && request
            .client_id
            .as_deref()
            .is_none_or(|client_id| client_id == claims.sub)
    {
        state.oauth.revoked_jti.insert(claims.jti, claims.exp);
    }
    let refresh_key = refresh_token_fingerprint(request.token.as_str());
    if let Some(mut refresh) = state.oauth.refresh_tokens.get_mut(refresh_key.as_str())
        && request
            .client_id
            .as_deref()
            .is_none_or(|client_id| client_id == refresh.client_id)
    {
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

/// Authorization requirement of one `/mcp` request that carried no valid token.
#[derive(Debug, Clone)]
enum McpAuthRequirement {
    NotRequired,
    ToolCall(serde_json::Value),
    Transport,
}

/// Decide whether one unauthenticated `/mcp` request may proceed.
///
/// Protocol discovery (initialize, ping, list operations, notifications) always
/// proceeds: an MCP client has to discover tools before it can obtain a token.
/// Tool invocations are checked one by one against the tool catalog, so a batch
/// cannot smuggle a protected call past an anonymous one, and a single protected
/// call is answered with an in-band authentication challenge so the client can
/// start the OAuth flow without losing the JSON-RPC exchange.
async fn tool_call_auth_requirement(
    state: &McpServerState,
    metadata: Arc<McpRequestMetadata>,
) -> Result<McpAuthRequirement, String> {
    if metadata.tool_calls.is_empty() {
        return Ok(McpAuthRequirement::NotRequired);
    }
    let requirements = state.exposed_tool_auth_requirements().await;
    MCP_METADATA
        .run(move || {
            let protected = metadata
                .tool_calls
                .iter()
                .find(|(name, _)| requirements.get(name).copied().unwrap_or(true));
            match protected {
                Some(_) if metadata.is_batch => McpAuthRequirement::Transport,
                Some((_, Some(id))) => McpAuthRequirement::ToolCall(id.clone()),
                Some((_, None)) => McpAuthRequirement::Transport,
                None => McpAuthRequirement::NotRequired,
            }
        })
        .await
        .map_err(|error| format!("MCP authentication metadata worker failed: {error}"))
}

async fn require_mcp_bearer(
    State(state): State<Arc<McpServerState>>,
    headers: HeaderMap,
    request: axum::http::Request<Body>,
    next: middleware::Next,
) -> Response {
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

    if request.method() != axum::http::Method::POST {
        return mcp_unauthorized(&issuer, &resource);
    }

    let (mut parts, body) = request.into_parts();
    let body = match to_bytes(body, MAX_MCP_METADATA_BODY_BYTES).await {
        Ok(body) => body,
        Err(error) => return mcp_request_body_error(&error),
    };
    let metadata = match request_metadata(&mut parts, body.clone()).await {
        Ok(metadata) => metadata,
        Err(error) => return mcp_worker_unavailable(error),
    };
    let requirement = match tool_call_auth_requirement(&state, metadata).await {
        Ok(requirement) => requirement,
        Err(error) => return mcp_worker_unavailable(error),
    };
    match requirement {
        McpAuthRequirement::NotRequired => {
            next.run(axum::http::Request::from_parts(parts, Body::from(body)))
                .await
        }
        McpAuthRequirement::ToolCall(id) => {
            match MCP_REWRITES
                .run(move || mcp_tool_authentication_required(&issuer, &resource, id))
                .await
            {
                Ok(response) => response,
                Err(error) => mcp_worker_unavailable(format!(
                    "MCP authentication response worker failed: {error}"
                )),
            }
        }
        McpAuthRequirement::Transport => mcp_unauthorized(&issuer, &resource),
    }
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
    match HeaderValue::try_from(value) {
        Ok(value) => {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, value);
        }
        Err(error) => {
            tracing::error!(
                target: "agena::server::mcp",
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "construct MCP WWW-Authenticate response header",
                    &error,
                ),
                "MCP unauthorized response omitted an invalid challenge header"
            );
        }
    }
    response
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
    _issuer: &str,
    resource: &str,
    error: Option<&str>,
    description: Option<&str>,
) -> String {
    let metadata_url = protected_resource_metadata_url(resource);
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
            "server_error" => StatusCode::INTERNAL_SERVER_ERROR,
            "temporarily_unavailable" => StatusCode::SERVICE_UNAVAILABLE,
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
    let headers = response.headers_mut();
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    headers.insert(header::PRAGMA, HeaderValue::from_static("no-cache"));
    headers.insert(
        axum::http::HeaderName::from_static("x-content-type-options"),
        HeaderValue::from_static("nosniff"),
    );
    response
}

fn normalize_scope(scope: Option<&str>) -> Result<String, OAuthRequestError> {
    let scope = scope.unwrap_or(DEFAULT_OAUTH_SCOPE);
    if scope.len() > MAX_OAUTH_SCOPE_BYTES {
        return Err(OAuthRequestError::new(
            "invalid_scope",
            "scope must be at most 512 bytes",
        ));
    }
    let scopes = scope.split_whitespace().collect::<HashSet<_>>();
    if scopes.is_empty()
        || !scopes.contains(MCP_SCOPE)
        || scopes
            .iter()
            .any(|value| *value != MCP_SCOPE && *value != OFFLINE_ACCESS_SCOPE)
    {
        Err(OAuthRequestError::new(
            "invalid_scope",
            "the agena:tools scope is required; offline_access is the only optional scope",
        ))
    } else if scopes.contains(OFFLINE_ACCESS_SCOPE) {
        Ok(DEFAULT_OAUTH_SCOPE.to_owned())
    } else {
        Ok(MCP_SCOPE.to_owned())
    }
}

fn scope_contains(scope: &str, expected: &str) -> bool {
    scope.split_whitespace().any(|value| value == expected)
}

fn valid_pkce_challenge(challenge: &str) -> bool {
    challenge.len() == 43
        && challenge
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
}

fn pkce_matches(verifier: &str, challenge: &str) -> bool {
    if !(43..=128).contains(&verifier.len()) {
        return false;
    }
    if !verifier
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~'))
        || !valid_pkce_challenge(challenge)
    {
        return false;
    }
    let digest = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(digest) == challenge
}

fn validate_redirect_uri(value: &str) -> Result<(), String> {
    let url = Url::parse(value).map_err(|error| format!("redirect URI is invalid: {error}"))?;
    if url.scheme() != "https" && !(url.scheme() == "http" && is_loopback_host(&url)) {
        return Err("redirect URI must use HTTPS unless it targets a loopback host".to_owned());
    }
    if url.host_str().is_none() {
        return Err("redirect URI must include a host".to_owned());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("redirect URI must not include user information".to_owned());
    }
    if url.fragment().is_some() {
        return Err("redirect URI must not include a fragment".to_owned());
    }
    if let Some(parameter) = url.query_pairs().find_map(|(key, _)| {
        matches!(
            key.as_ref(),
            "code" | "state" | "iss" | "error" | "error_description"
        )
        .then(|| key.into_owned())
    }) {
        return Err(format!(
            "redirect URI must not pre-populate the reserved OAuth query parameter `{parameter}`"
        ));
    }
    Ok(())
}

/// Whether a client id can be treated as a Client ID Metadata Document URL.
///
/// Any HTTPS origin may host the document: MCP clients that support CIMD
/// describe themselves this way, and keeping the surface to one vendor would
/// exclude every other client. Nothing about the URL is trusted on its own;
/// the fetch refuses non-public address space and the document is validated
/// against the URL it came from.
fn is_supported_cimd_client_id(value: &str) -> bool {
    let Ok(url) = Url::parse(value.trim()) else {
        return false;
    };
    url.scheme() == "https"
        && url.host_str().is_some()
        && url.username().is_empty()
        && url.password().is_none()
        && url.query().is_none()
        && url.fragment().is_none()
        && !matches!(url.path(), "" | "/")
        && url
            .path()
            .split('/')
            .all(|segment| segment != "." && segment != "..")
}

fn is_loopback_host(url: &Url) -> bool {
    url.host_str().is_some_and(is_loopback_host_name)
}

/// Reject client metadata URLs that resolve into address space Agena cannot
/// safely reach from a public endpoint.
///
/// Opening CIMD to any HTTPS origin would otherwise let an unauthenticated
/// caller point the authorization server at loopback, link-local, or private
/// networks (SSRF). Every resolved address must be globally routable; the fetch
/// client already refuses redirects and bounds time and size.
async fn ensure_public_metadata_host(host: &str, port: u16) -> Result<(), String> {
    if let Ok(address) = host.parse::<std::net::IpAddr>() {
        return if is_public_ip(address) {
            Ok(())
        } else {
            Err(format!(
                "client metadata host {host} is not a public address"
            ))
        };
    }
    let addresses = tokio::net::lookup_host((host, port))
        .await
        .map_err(|error| format!("client metadata host {host} could not be resolved: {error}"))?;
    let mut resolved = false;
    for address in addresses {
        resolved = true;
        if !is_public_ip(address.ip()) {
            return Err(format!(
                "client metadata host {host} resolves to the non-public address {}",
                address.ip()
            ));
        }
    }
    if !resolved {
        return Err(format!("client metadata host {host} did not resolve"));
    }
    Ok(())
}

fn is_public_ip(address: std::net::IpAddr) -> bool {
    match address {
        std::net::IpAddr::V4(address) => {
            !(address.is_private()
                || address.is_loopback()
                || address.is_link_local()
                || address.is_broadcast()
                || address.is_documentation()
                || address.is_unspecified()
                || address.is_multicast())
        }
        std::net::IpAddr::V6(address) => {
            if let Some(mapped) = address.to_ipv4_mapped() {
                // An IPv4-mapped address must be judged as the IPv4 address the
                // connection actually reaches.
                return is_public_ip(std::net::IpAddr::V4(mapped));
            }
            !(address.is_loopback()
                || address.is_unspecified()
                || address.is_multicast()
                || address.is_unique_local()
                || address.is_unicast_link_local())
        }
    }
}

fn is_loopback_host_name(host: &str) -> bool {
    let host = host.trim_matches(['[', ']']).trim_end_matches('.');
    host.eq_ignore_ascii_case("localhost")
        || host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback())
}

fn authority_host(authority: &str) -> Option<String> {
    Url::parse(format!("http://{authority}").as_str())
        .ok()?
        .host_str()
        .map(str::to_owned)
}

fn authority_matches_url(authority: &str, value: &str) -> bool {
    let Ok(authority_url) = Url::parse(format!("http://{authority}").as_str()) else {
        return false;
    };
    let Ok(identity_url) = Url::parse(value) else {
        return false;
    };
    if !authority_url
        .host_str()
        .zip(identity_url.host_str())
        .is_some_and(|(actual, expected)| actual.eq_ignore_ascii_case(expected))
    {
        return false;
    }
    let expected_port = identity_url.port_or_known_default();
    let actual_port = authority_url
        .port()
        .or_else(|| match identity_url.scheme() {
            "https" => Some(443),
            "http" => Some(80),
            _ => None,
        });
    actual_port == expected_port
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
    let mut url = Url::parse(value.trim())
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
    if !matches!(url.path(), "" | "/") {
        return Err(
            "Agena-managed OAuth issuer URL must be an origin without a path (for example https://agena.example.com)"
                .to_owned(),
        );
    }
    // Agena's embedded authorization endpoints are served at root paths such
    // as /oauth/authorize and /oauth/token. Canonicalize the issuer to the
    // origin so discovery can never advertise an unreachable path-prefixed
    // endpoint.
    url.set_path("");
    Ok(url.to_string().trim_end_matches('/').to_owned())
}

fn is_https_resource(value: &str) -> bool {
    Url::parse(value)
        .map(|url| url.scheme().eq_ignore_ascii_case("https"))
        .unwrap_or(false)
}

/// Whether a URL is a plain-HTTP URL on a loopback host.
///
/// A local `agena server` cannot serve HTTPS, so its own listener-local identity
/// is allowed to stay on loopback HTTP. Everything reachable from another host
/// must still be HTTPS, which is what `is_acceptable_oauth_origin` enforces.
fn is_loopback_resource(value: &str) -> bool {
    Url::parse(value)
        .map(|url| url.scheme().eq_ignore_ascii_case("http") && is_loopback_host(&url))
        .unwrap_or(false)
}

fn is_acceptable_oauth_origin(value: &str) -> bool {
    is_https_resource(value) || is_loopback_resource(value)
}

fn validate_managed_oauth_identity(
    resource: &str,
    configured_issuer: Option<&str>,
) -> Result<String, String> {
    if !is_acceptable_oauth_origin(resource) {
        return Err(
            "MCP OAuth requires a canonical remote HTTPS resource URL (for example https://agena.example.com/mcp); only loopback HTTP is accepted without TLS"
                .to_owned(),
        );
    }
    if resource_has_nonstandard_path(resource) && configured_issuer.is_none() {
        return Err(
            "A routed MCP resource URL requires an explicit browser-reachable OAuth issuer origin because Agena cannot derive the authorization-server host from that routed path"
                .to_owned(),
        );
    }
    let issuer = configured_issuer
        .map(str::to_owned)
        .unwrap_or_else(|| issuer_for_resource(resource));
    if !is_acceptable_oauth_origin(issuer.as_str()) {
        return Err(
            "MCP OAuth requires a canonical HTTPS issuer origin (for example https://agena.example.com); only loopback HTTP is accepted without TLS"
                .to_owned(),
        );
    }
    Ok(issuer)
}

fn issuer_for_resource(resource: &str) -> String {
    Url::parse(resource)
        .map(|url| url.origin().ascii_serialization())
        .unwrap_or_else(|_| resource.trim_end_matches('/').to_owned())
}

fn resource_has_nonstandard_path(resource: &str) -> bool {
    Url::parse(resource)
        .map(|url| url.path().trim_end_matches('/') != MCP_PATH)
        .unwrap_or(true)
}

/// RFC 9728 inserts the protected-resource well-known suffix before the
/// resource path. For example `https://example.test/public/mcp` maps to
/// `https://example.test/.well-known/oauth-protected-resource/public/mcp`.
fn protected_resource_metadata_url(resource: &str) -> String {
    let Ok(mut url) = Url::parse(resource) else {
        return append_public_endpoint(resource, "/.well-known/oauth-protected-resource");
    };
    let resource_path = url.path().trim_matches('/');
    let path = if resource_path.is_empty() {
        "/.well-known/oauth-protected-resource".to_owned()
    } else {
        format!("/.well-known/oauth-protected-resource/{resource_path}")
    };
    url.set_path(path.as_str());
    url.set_query(None);
    url.set_fragment(None);
    url.to_string().trim_end_matches('/').to_owned()
}

/// RFC 8414 applies the same well-known insertion rule to authorization-server
/// issuers with a path component.
fn authorization_server_metadata_url(issuer: &str) -> String {
    let Ok(mut url) = Url::parse(issuer) else {
        return append_public_endpoint(issuer, "/.well-known/oauth-authorization-server");
    };
    let issuer_path = url.path().trim_matches('/');
    let path = if issuer_path.is_empty() {
        "/.well-known/oauth-authorization-server".to_owned()
    } else {
        format!("/.well-known/oauth-authorization-server/{issuer_path}")
    };
    url.set_path(path.as_str());
    url.set_query(None);
    url.set_fragment(None);
    url.to_string().trim_end_matches('/').to_owned()
}

fn append_public_endpoint(base: &str, suffix: &str) -> String {
    format!(
        "{}/{}",
        base.trim_end_matches('/'),
        suffix.trim_start_matches('/')
    )
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

    #[test]
    fn redirect_uri_policy_matches_oauth_requirements() {
        assert!(validate_redirect_uri("https://chatgpt.com/connector_oauth_redirect").is_ok());
        assert!(validate_redirect_uri("http://127.0.0.1:6274/callback").is_ok());
        assert!(validate_redirect_uri("http://localhost:1234/callback").is_ok());
        assert!(validate_redirect_uri("http://example.com/callback").is_err());
        assert!(validate_redirect_uri("https://example.com/cb#fragment").is_err());
        assert!(validate_redirect_uri("https://user:pass@example.com/cb").is_err());
        assert!(validate_redirect_uri("https://example.com/cb?state=abc").is_err());
    }

    #[test]
    fn client_ids_must_be_https_metadata_documents() {
        assert!(is_supported_cimd_client_id(
            "https://chatgpt.com/oauth/client.json"
        ));
        assert!(is_supported_cimd_client_id(
            "https://example.com/clients/acme/client.json"
        ));
        assert!(!is_supported_cimd_client_id(
            "http://example.com/client.json"
        ));
        assert!(!is_supported_cimd_client_id(
            "https://example.com/client.json?x=1"
        ));
        assert!(!is_supported_cimd_client_id("https://example.com/"));
        // The URL parser collapses literal dot segments, so a document URL that
        // only names a directory is the case to reject.
        assert!(!is_supported_cimd_client_id("https://example.com/."));
        assert!(!is_supported_cimd_client_id("agena-static-client"));
    }

    #[test]
    fn client_metadata_fetch_refuses_non_public_addresses() {
        for address in [
            "127.0.0.1",
            "10.1.2.3",
            "169.254.169.254",
            "0.0.0.0",
            "::1",
            "fd00::1",
            "::ffff:10.0.0.1",
        ] {
            let parsed: std::net::IpAddr = address.parse().expect("valid test address");
            assert!(
                !is_public_ip(parsed),
                "{address} must not be treated as public"
            );
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            let parsed: std::net::IpAddr = address.parse().expect("valid test address");
            assert!(is_public_ip(parsed), "{address} must be treated as public");
        }
    }

    #[test]
    fn pkce_verification_uses_s256() {
        assert!(pkce_matches(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        ));
        assert!(!pkce_matches(
            "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cX"
        ));
    }

    #[test]
    fn scope_normalization_always_grants_tool_access() {
        assert_eq!(
            normalize_scope(None).expect("default scope"),
            DEFAULT_OAUTH_SCOPE
        );
        assert_eq!(
            normalize_scope(Some("agena:tools")).expect("tools scope"),
            MCP_SCOPE
        );
        assert_eq!(
            normalize_scope(Some("agena:tools offline_access")).expect("offline scope"),
            DEFAULT_OAUTH_SCOPE
        );
        assert!(normalize_scope(Some("offline_access")).is_err());
        assert!(normalize_scope(Some("agena:tools admin")).is_err());
    }

    #[test]
    fn loopback_identities_are_accepted_without_tls() {
        assert!(is_acceptable_oauth_origin("http://127.0.0.1:3210/mcp"));
        assert!(is_acceptable_oauth_origin("https://agena.example.com/mcp"));
        assert!(!is_acceptable_oauth_origin("http://agena.example.com/mcp"));
        let identity = validate_managed_oauth_identity("http://127.0.0.1:3210/mcp", None)
            .expect("loopback identity is accepted");
        assert_eq!(identity, "http://127.0.0.1:3210");
        assert!(validate_managed_oauth_identity("http://agena.example.com/mcp", None).is_err());
    }

    #[test]
    fn public_url_normalization_appends_the_mcp_path() {
        assert_eq!(
            normalize_public_mcp_url("https://agena.example.com").expect("origin"),
            "https://agena.example.com/mcp"
        );
        assert_eq!(
            normalize_public_mcp_url("https://tunnel.example.com/v1/mcp/tunnel_id")
                .expect("tunnel path"),
            "https://tunnel.example.com/v1/mcp/tunnel_id"
        );
    }

    #[test]
    fn tool_security_schemes_follow_the_tool_policy() {
        let mut requirements = HashMap::new();
        requirements.insert("fs.read".to_owned(), true);
        requirements.insert("ping.health".to_owned(), false);
        let payload = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "result": { "tools": [
                { "name": "fs.read" },
                { "name": "ping.health" },
                { "name": "unknown.tool" }
            ] }
        });
        let rewritten = add_tool_security_schemes(payload, &requirements).expect("rewritten");
        let tools = rewritten["result"]["tools"]
            .as_array()
            .expect("tools array");
        assert_eq!(tools[0]["securitySchemes"][0]["type"], "oauth2");
        assert_eq!(tools[1]["securitySchemes"][0]["type"], "noauth");
        // A tool missing from the catalog must never become anonymous.
        assert_eq!(tools[2]["securitySchemes"][0]["type"], "oauth2");
    }
}
