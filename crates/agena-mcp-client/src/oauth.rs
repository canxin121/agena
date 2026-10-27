//! Bounded, revision-bound OAuth sign-in sessions for external MCP connections.

use std::collections::BTreeMap;
use std::sync::{Arc, Weak};
use std::time::Duration;

use chrono::{SecondsFormat, Utc};
use oauth2::CsrfToken;
use rmcp::transport::auth::{AuthorizationManager, AuthorizationRequest, OAuthState};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, OwnedSemaphorePermit, Semaphore};
use tokio_util::sync::CancellationToken;
use url::Url;

use crate::oauth_http::{AuthorizedOAuthHttpClient, OAuthAuthorizationSlot};
use crate::{
    FileOAuthCredentialStore, McpError, McpOAuthNetworkAuthorizer, McpResult, credential_store_name,
};

const FLOW_LIFETIME: Duration = Duration::from_secs(600);
const AUTH_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_PENDING_FLOWS: usize = 32;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpOAuthStart {
    pub flow_id: String,
    pub authorization_url: String,
    pub expires_at: String,
}

/// Optional client identity supplied before OAuth sign-in. Debug output never
/// includes registration credentials or a potentially sensitive metadata URL.
#[derive(Clone, Default)]
pub struct McpOAuthRegistration {
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub client_metadata_url: Option<String>,
}

impl std::fmt::Debug for McpOAuthRegistration {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpOAuthRegistration")
            .field("client_id_configured", &self.client_id.is_some())
            .field("client_secret_configured", &self.client_secret.is_some())
            .field(
                "client_metadata_url_configured",
                &self.client_metadata_url.is_some(),
            )
            .finish()
    }
}

impl McpOAuthRegistration {
    fn apply(self, mut request: AuthorizationRequest) -> McpResult<AuthorizationRequest> {
        if self.client_secret.is_some() && self.client_id.is_none() {
            return Err(McpError::Auth(
                "A pre-registered client ID is required with a client secret.".into(),
            ));
        }
        if let Some(client_id) = self.client_id {
            if client_id.trim().is_empty() || client_id.chars().any(char::is_control) {
                return Err(McpError::Auth(
                    "The OAuth client ID must be nonempty and contain no control characters."
                        .into(),
                ));
            }
            request = request.with_preregistered_client(client_id);
        }
        if let Some(secret) = self.client_secret {
            if secret.trim().is_empty() {
                return Err(McpError::Auth(
                    "The OAuth client secret must not be empty.".into(),
                ));
            }
            request = request.with_client_secret(secret);
        }
        if let Some(metadata_url) = self.client_metadata_url {
            let url = Url::parse(&metadata_url).map_err(|_| McpError::Auth("The OAuth client metadata URL must be an absolute HTTPS URL with a non-root path.".into()))?;
            validate_http_url(&url)?;
            if url.scheme() != "https" || url.path() == "/" {
                return Err(McpError::Auth(
                    "The OAuth client metadata URL must use HTTPS and have a non-root path.".into(),
                ));
            }
            request = request.with_client_metadata_url(metadata_url);
        }
        Ok(request)
    }
}

/// Durable binding of an OAuth record to the requested scopes and client
/// identity. Sorting scopes avoids unnecessary sign-in after reordering them;
/// changing any registration credential requires a fresh authorization.
pub fn oauth_credential_binding(scopes: &[String], registration: &McpOAuthRegistration) -> String {
    let mut digest = Sha256::new();
    digest.update(b"agena-mcp-oauth-binding-v1\0");
    let scopes = scopes.iter().collect::<std::collections::BTreeSet<_>>();
    digest.update((scopes.len() as u64).to_be_bytes());
    for scope in scopes {
        digest.update((scope.len() as u64).to_be_bytes());
        digest.update(scope.as_bytes());
    }
    for value in [
        &registration.client_id,
        &registration.client_secret,
        &registration.client_metadata_url,
    ] {
        digest.update([u8::from(value.is_some())]);
        if let Some(value) = value {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value.as_bytes());
        }
    }
    digest
        .finalize()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

struct PendingAuthorization {
    server: String,
    endpoint: Url,
    redirect_uri: Url,
    binding_revision: String,
    deadline: tokio::time::Instant,
    state: OAuthState,
    network_authorization: Arc<OAuthAuthorizationSlot>,
    _permit: OwnedSemaphorePermit,
}

pub struct McpOAuthSessionManager {
    pending: Mutex<BTreeMap<String, PendingAuthorization>>,
    operations: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    capacity: Arc<Semaphore>,
    shutdown: CancellationToken,
    secret_store: Option<Arc<dyn agena_secret_store::SecretStore>>,
}

impl Default for McpOAuthSessionManager {
    fn default() -> Self {
        Self {
            pending: Mutex::new(BTreeMap::new()),
            operations: Mutex::new(BTreeMap::new()),
            capacity: Arc::new(Semaphore::new(MAX_PENDING_FLOWS)),
            shutdown: CancellationToken::new(),
            secret_store: None,
        }
    }
}

impl McpOAuthSessionManager {
    pub fn with_secret_store(store: Arc<dyn agena_secret_store::SecretStore>) -> Self {
        Self {
            secret_store: Some(store),
            ..Self::default()
        }
    }

    fn credential_store(
        &self,
        server: &str,
        endpoint: &Url,
    ) -> McpResult<FileOAuthCredentialStore> {
        let server = credential_store_name(server, endpoint);
        match self.secret_store.as_ref() {
            Some(store) => FileOAuthCredentialStore::with_secret_store(&server, Arc::clone(store)),
            None => FileOAuthCredentialStore::new(&server),
        }
        .map_err(|_| McpError::Auth("Could not open the MCP OAuth credential store.".into()))
    }

    pub async fn begin(
        &self,
        server: &str,
        endpoint: Url,
        scopes: Vec<String>,
        redirect_uri: &str,
        binding_revision: String,
    ) -> McpResult<McpOAuthStart> {
        self.begin_with_registration(
            server,
            endpoint,
            scopes,
            redirect_uri,
            binding_revision,
            McpOAuthRegistration::default(),
        )
        .await
    }

    pub async fn begin_with_registration(
        &self,
        server: &str,
        endpoint: Url,
        scopes: Vec<String>,
        redirect_uri: &str,
        binding_revision: String,
        registration: McpOAuthRegistration,
    ) -> McpResult<McpOAuthStart> {
        self.begin_internal(
            server,
            endpoint,
            scopes,
            redirect_uri,
            binding_revision,
            registration,
            None,
        )
        .await
    }

    pub async fn begin_authorized(
        &self,
        server: &str,
        endpoint: Url,
        scopes: Vec<String>,
        redirect_uri: &str,
        binding_revision: String,
        registration: McpOAuthRegistration,
        authorizer: Arc<dyn McpOAuthNetworkAuthorizer>,
    ) -> McpResult<McpOAuthStart> {
        self.begin_internal(
            server,
            endpoint,
            scopes,
            redirect_uri,
            binding_revision,
            registration,
            Some(authorizer),
        )
        .await
    }

    async fn begin_internal(
        &self,
        server: &str,
        endpoint: Url,
        scopes: Vec<String>,
        redirect_uri: &str,
        binding_revision: String,
        registration: McpOAuthRegistration,
        authorizer: Option<Arc<dyn McpOAuthNetworkAuthorizer>>,
    ) -> McpResult<McpOAuthStart> {
        if self.shutdown.is_cancelled() {
            return Err(McpError::Shutdown);
        }
        if server.trim().is_empty() || binding_revision.is_empty() {
            return Err(McpError::Auth(
                "The OAuth connection binding is missing.".into(),
            ));
        }
        validate_http_url(&endpoint)?;
        let redirect = Url::parse(redirect_uri).map_err(|_| {
            McpError::Auth("The OAuth redirect URI must be an absolute HTTP(S) URL.".into())
        })?;
        validate_http_url(&redirect)?;
        if redirect.query().is_some() {
            return Err(McpError::Auth(
                "The OAuth redirect URI must not contain a query string.".into(),
            ));
        }
        let credential_binding = oauth_credential_binding(&scopes, &registration);
        let request = registration.apply(
            AuthorizationRequest::new(redirect.as_str())
                .with_scopes(scopes)
                .with_client_name("Agena")
                .with_application_type(if is_loopback(&redirect) {
                    "native"
                } else {
                    "web"
                }),
        )?;
        self.expire().await;
        let permit = Arc::clone(&self.capacity)
            .try_acquire_owned()
            .map_err(|_| {
                McpError::Auth(
                    "Too many pending MCP sign-ins. Complete or cancel an existing sign-in.".into(),
                )
            })?;
        let operation = self.operation(server).await;
        let _operation_guard = operation.lock().await;
        let network_authorization = Arc::new(OAuthAuthorizationSlot::default());
        let _authorization_scope = network_authorization.install(authorizer);
        let prepared = async {
            let client = Arc::new(AuthorizedOAuthHttpClient::new(Arc::clone(
                &network_authorization,
            ))?);
            let mut manager =
                AuthorizationManager::new_with_oauth_http_client(endpoint.clone(), client)
                    .await
                    .map_err(|_| McpError::Auth("Could not initialize MCP OAuth.".into()))?;
            manager.set_credential_store(
                self.credential_store(server, &endpoint)?
                    .with_binding(credential_binding),
            );
            let mut state = OAuthState::Unauthorized(manager);
            state.start_authorization(request).await
                .map_err(|_| McpError::Auth("OAuth discovery or client registration failed. Check the server and its configured OAuth client identity.".into()))?;
            let authorization_url = state.get_authorization_url().await.map_err(|_| {
                McpError::Auth("The OAuth provider did not return an authorization URL.".into())
            })?;
            let parsed_authorization_url = Url::parse(&authorization_url).map_err(|_| {
                McpError::Auth("The OAuth provider returned an invalid authorization URL.".into())
            })?;
            validate_http_url(&parsed_authorization_url)?;
            Ok::<_, McpError>((state, authorization_url))
        };
        let prepared = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => return Err(McpError::Shutdown),
            result = tokio::time::timeout(AUTH_REQUEST_TIMEOUT, prepared) => result
                .map_err(|_| McpError::Auth("MCP OAuth sign-in preparation timed out.".into()))?,
        };
        if let Some(error) = network_authorization.take_failure() {
            return Err(error);
        }
        let (state, authorization_url) = prepared?;
        let flow_id = CsrfToken::new_random().secret().clone();
        let expires_at = (Utc::now() + chrono::Duration::seconds(FLOW_LIFETIME.as_secs() as i64))
            .to_rfc3339_opts(SecondsFormat::Secs, true);
        let mut pending = self.pending.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(McpError::Shutdown);
        }
        // A new sign-in for the same connection replaces its old pending flow.
        // Different connections never evict one another silently.
        pending.retain(|_, flow| flow.server != server);
        pending.insert(
            flow_id.clone(),
            PendingAuthorization {
                server: server.to_owned(),
                endpoint,
                redirect_uri: redirect,
                binding_revision,
                deadline: tokio::time::Instant::now() + FLOW_LIFETIME,
                state,
                network_authorization,
                _permit: permit,
            },
        );
        Ok(McpOAuthStart {
            flow_id,
            authorization_url,
            expires_at,
        })
    }

    pub async fn complete(
        &self,
        flow_id: &str,
        callback_url: &str,
        expected_binding_revision: &str,
    ) -> McpResult<()> {
        self.complete_internal(flow_id, callback_url, expected_binding_revision, None)
            .await
    }

    pub async fn complete_authorized(
        &self,
        flow_id: &str,
        callback_url: &str,
        expected_binding_revision: &str,
        authorizer: Arc<dyn McpOAuthNetworkAuthorizer>,
    ) -> McpResult<()> {
        self.complete_internal(
            flow_id,
            callback_url,
            expected_binding_revision,
            Some(authorizer),
        )
        .await
    }

    async fn complete_internal(
        &self,
        flow_id: &str,
        callback_url: &str,
        expected_binding_revision: &str,
        authorizer: Option<Arc<dyn McpOAuthNetworkAuthorizer>>,
    ) -> McpResult<()> {
        self.expire().await;
        let server = self
            .pending
            .lock()
            .await
            .get(flow_id)
            .map(|flow| flow.server.clone())
            .ok_or_else(expired_flow)?;
        let operation = self.operation(&server).await;
        let _operation_guard = operation.lock().await;
        let mut flow = {
            let mut pending = self.pending.lock().await;
            let flow = pending.get(flow_id).ok_or_else(expired_flow)?;
            if flow.deadline <= tokio::time::Instant::now() {
                pending.remove(flow_id);
                return Err(expired_flow());
            }
            if flow.binding_revision != expected_binding_revision {
                return Err(McpError::Auth(
                    "The MCP connection changed during sign-in. Start sign-in again.".into(),
                ));
            }
            let callback = Url::parse(callback_url)
                .map_err(|_| McpError::Auth("Paste the complete OAuth callback URL.".into()))?;
            if !same_redirect(&flow.redirect_uri, &callback) {
                return Err(McpError::Auth(
                    "The OAuth callback does not match the registered redirect URI.".into(),
                ));
            }
            // Check that the stored endpoint remains valid without returning it
            // in diagnostics; the revision binds it to the current definition.
            validate_http_url(&flow.endpoint)?;
            pending
                .remove(flow_id)
                .expect("validated pending OAuth flow")
        };
        let _authorization_scope = flow.network_authorization.install(authorizer);
        let result = tokio::select! {
            biased;
            _ = self.shutdown.cancelled() => Err(McpError::Shutdown),
            result = tokio::time::timeout(AUTH_REQUEST_TIMEOUT, flow.state.handle_callback_url(callback_url)) => {
                result.map_err(|_| McpError::Auth("MCP OAuth token exchange timed out. Start sign-in again.".into()))?
                    .map_err(|_| McpError::Auth("OAuth callback validation or token exchange failed. Start sign-in again.".into()))
            }
        };
        match flow.network_authorization.take_failure() {
            Some(error) => Err(error),
            None => result,
        }
    }

    pub async fn cancel(&self, flow_id: &str) -> bool {
        self.pending.lock().await.remove(flow_id).is_some()
    }

    /// Cancel only a flow owned by the selected connection revision. A stale
    /// or wrong connection ID cannot cancel another connection's pending login.
    pub async fn cancel_checked(&self, flow_id: &str, binding_revision: &str) -> McpResult<bool> {
        self.expire().await;
        let mut pending = self.pending.lock().await;
        let Some(flow) = pending.get(flow_id) else {
            return Ok(false);
        };
        if flow.binding_revision != binding_revision {
            return Err(McpError::Auth(
                "The MCP sign-in belongs to a different connection revision.".into(),
            ));
        }
        Ok(pending.remove(flow_id).is_some())
    }

    pub async fn logout(&self, server: &str, endpoint: &Url) -> McpResult<()> {
        let operation = self.operation(server).await;
        let _operation_guard = operation.lock().await;
        self.pending
            .lock()
            .await
            .retain(|_, flow| flow.server != server || flow.endpoint != *endpoint);
        let store = self.credential_store(server, endpoint)?;
        crate::token_store::SECRET_STORE_WORKERS
            .run(move || store.delete())
            .await
            .map_err(|_| McpError::Auth("Could not access the MCP OAuth credential store.".into()))?
            .map_err(|_| McpError::Auth("Could not remove the MCP OAuth credential.".into()))
    }

    pub async fn shutdown(&self) {
        self.shutdown.cancel();
        self.pending.lock().await.clear();
    }

    async fn expire(&self) {
        let now = tokio::time::Instant::now();
        self.pending
            .lock()
            .await
            .retain(|_, flow| flow.deadline > now);
    }

    async fn operation(&self, server: &str) -> Arc<Mutex<()>> {
        let mut operations = self.operations.lock().await;
        operations.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = operations.get(server).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            operations.insert(server.to_owned(), Arc::downgrade(&lock));
            lock
        }
    }
}

fn expired_flow() -> McpError {
    McpError::Auth(
        "This MCP sign-in is missing, expired, or already completed. Start sign-in again.".into(),
    )
}

fn validate_http_url(url: &Url) -> McpResult<()> {
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(McpError::Auth(
            "OAuth URLs must be absolute HTTP(S) URLs without user information or fragments."
                .into(),
        ));
    }
    Ok(())
}

fn same_redirect(expected: &Url, actual: &Url) -> bool {
    expected.scheme() == actual.scheme()
        && expected.host_str() == actual.host_str()
        && expected.port_or_known_default() == actual.port_or_known_default()
        && expected.path() == actual.path()
        && actual.username().is_empty()
        && actual.password().is_none()
        && actual.fragment().is_none()
}

fn is_loopback(url: &Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_bindings_cover_scopes_and_registration_without_scope_order_sensitivity() {
        let scopes = vec!["read".into(), "write".into()];
        let base = oauth_credential_binding(&scopes, &McpOAuthRegistration::default());
        assert_eq!(
            base,
            oauth_credential_binding(
                &["write".into(), "read".into(), "read".into()],
                &McpOAuthRegistration::default()
            )
        );
        assert_ne!(
            base,
            oauth_credential_binding(&["read".into()], &McpOAuthRegistration::default())
        );
        for registration in [
            McpOAuthRegistration {
                client_id: Some("client".into()),
                ..Default::default()
            },
            McpOAuthRegistration {
                client_secret: Some("secret".into()),
                ..Default::default()
            },
            McpOAuthRegistration {
                client_metadata_url: Some("https://client.example/metadata.json".into()),
                ..Default::default()
            },
        ] {
            assert_ne!(base, oauth_credential_binding(&scopes, &registration));
        }
    }

    #[test]
    fn callbacks_match_the_registered_origin_and_path_without_echoing_codes() {
        let expected = Url::parse("http://localhost:4020/settings/mcp-connections").unwrap();
        let valid =
            Url::parse("http://localhost:4020/settings/mcp-connections?code=private&state=secret")
                .unwrap();
        assert!(same_redirect(&expected, &valid));
        for invalid in [
            "http://localhost:4021/settings/mcp-connections?code=private",
            "http://elsewhere/settings/mcp-connections?code=private",
            "http://localhost:4020/other?code=private",
            "http://user:private@localhost:4020/settings/mcp-connections",
            "http://localhost:4020/settings/mcp-connections#code=private",
        ] {
            assert!(!same_redirect(&expected, &Url::parse(invalid).unwrap()));
        }
    }

    #[tokio::test]
    async fn cancelled_or_unknown_flows_cannot_exchange_tokens() {
        let manager = McpOAuthSessionManager::default();
        assert!(!manager.cancel("unknown").await);
        let error = manager
            .complete("unknown", "https://callback/?code=private", "revision")
            .await
            .unwrap_err();
        assert!(!error.to_string().contains("private"));
        manager.shutdown().await;
        let error = manager
            .begin(
                "example",
                Url::parse("https://example.test/mcp").unwrap(),
                Vec::new(),
                "https://callback.test/",
                "revision".into(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, McpError::Shutdown));
    }
}
