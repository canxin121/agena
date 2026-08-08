//! OAuth authorization-code login for remote MCP servers.
//!
//! This module deliberately delegates protocol details (protected-resource
//! metadata, authorization-server discovery, S256 PKCE, RFC 9207 issuer
//! binding, dynamic client registration, token exchange and refresh) to
//! `rmcp`. Agena only supplies a server-scoped keyring credential store and a
//! small, redacted CLI-facing lifecycle wrapper.
//!
//! Revocation is deliberately an explicit action.  A local keyring delete is
//! useful when a user no longer wants Agena to use a credential, but it does
//! not tell the authorization server to invalidate that credential.  The
//! `revoke_and_clear` operation implements the optional RFC 7009 endpoint
//! when the discovered authorization-server metadata advertises one.

use std::time::Duration;

use oauth2::TokenResponse;
use rmcp::transport::{AuthorizationManager, AuthorizationSession, CredentialStore};
use url::Url;

use crate::{KeyringOAuthCredentialStore, McpError, McpResult};

/// An MCP OAuth login session.
pub struct McpOAuthLoginSession {
    authorization: AuthorizationSession,
    authorization_url: String,
}

impl std::fmt::Debug for McpOAuthLoginSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpOAuthLoginSession")
            .field("authorization_url", &self.authorization_url)
            .finish_non_exhaustive()
    }
}

impl McpOAuthLoginSession {
    /// Discover the MCP resource's authorization server and create a pending
    /// S256 PKCE browser authorization session. Credentials are committed to
    /// the keyring only after a verified callback completes successfully.
    pub async fn begin(
        server: &str,
        endpoint: Url,
        scopes: &[String],
        redirect_uri: &str,
    ) -> McpResult<Self> {
        let store = KeyringOAuthCredentialStore::new(server)
            .map_err(|error| McpError::Auth(error.to_string()))?;
        let mut manager = AuthorizationManager::new(endpoint)
            .await
            .map_err(|error| McpError::Auth(error.to_string()))?;
        manager.set_credential_store(store);
        let metadata = manager
            .discover_metadata()
            .await
            .map_err(|error| McpError::Auth(error.to_string()))?;
        manager.set_metadata(metadata);
        let scopes = scopes.iter().map(String::as_str).collect::<Vec<_>>();
        let authorization = AuthorizationSession::new(
            manager,
            scopes.as_slice(),
            redirect_uri,
            Some("Agena MCP"),
            None,
        )
        .await
        .map_err(|error| McpError::Auth(error.to_string()))?;
        let authorization_url = authorization.get_authorization_url().to_owned();
        Ok(Self {
            authorization,
            authorization_url,
        })
    }

    pub fn authorization_url(&self) -> &str {
        self.authorization_url.as_str()
    }

    /// Complete the pending browser flow. `rmcp` verifies the CSRF state and
    /// optional/required RFC 9207 issuer before persisting credentials.
    pub async fn complete(&self, code: &str, state: &str, issuer: Option<&str>) -> McpResult<()> {
        self.authorization
            .handle_callback_with_issuer(code, state, issuer)
            .await
            .map(|_| ())
            .map_err(|error| McpError::Auth(error.to_string()))
    }

    /// Revoke the refresh token when present (otherwise the access token) at
    /// the RFC 7009 endpoint advertised by the authorization server for an
    /// MCP resource, then remove the local keyring record.
    ///
    /// The resource endpoint is intentionally explicit.  Logout must not
    /// infer an authority from an unrelated configuration layer, and a plain
    /// `logout --oauth` remains a local-only operation for servers that do
    /// not advertise revocation.  No credential, client id, response body, or
    /// authorization-server error detail is returned or logged.
    pub async fn revoke_and_clear(server: &str, endpoint: Url) -> McpResult<()> {
        let store = KeyringOAuthCredentialStore::new(server)
            .map_err(|error| McpError::Auth(error.to_string()))?;
        let credentials = store
            .load()
            .await
            .map_err(|_| McpError::Auth("unable to read stored MCP OAuth credential".to_owned()))?
            .ok_or_else(|| McpError::Auth("no stored MCP OAuth credential to revoke".to_owned()))?;
        let token_response = credentials.token_response.ok_or_else(|| {
            McpError::Auth("stored MCP OAuth credential has no revocable token".to_owned())
        })?;

        let (token, token_type_hint) = match token_response.refresh_token() {
            Some(refresh) => (refresh.secret(), "refresh_token"),
            None => (token_response.access_token().secret(), "access_token"),
        };

        let manager = AuthorizationManager::new(endpoint)
            .await
            .map_err(|_| McpError::Auth("unable to discover MCP OAuth metadata".to_owned()))?;
        let metadata = manager
            .discover_metadata()
            .await
            .map_err(|_| McpError::Auth("unable to discover MCP OAuth metadata".to_owned()))?;
        let revocation_endpoint = metadata
            .additional_fields
            .get("revocation_endpoint")
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                McpError::Auth(
                    "MCP OAuth authorization server does not advertise an RFC 7009 revocation endpoint"
                        .to_owned(),
                )
            })?;
        let revocation_endpoint = Url::parse(revocation_endpoint).map_err(|_| {
            McpError::Auth("MCP OAuth revocation endpoint metadata is not a valid URL".to_owned())
        })?;
        if !matches!(revocation_endpoint.scheme(), "http" | "https")
            || revocation_endpoint.host_str().is_none()
            || !revocation_endpoint.username().is_empty()
            || revocation_endpoint.password().is_some()
        {
            return Err(McpError::Auth(
                "MCP OAuth revocation endpoint must be an http(s) URL without embedded credentials"
                    .to_owned(),
            ));
        }

        // RFC 7009 uses application/x-www-form-urlencoded.  Do not follow
        // redirects: a redirect could move a bearer-like token to a different
        // origin.  `client_id` is included for public dynamically registered
        // clients; it is never surfaced outside this request.
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .build()
            .map_err(|_| {
                McpError::Auth("unable to create MCP OAuth revocation client".to_owned())
            })?;
        // `reqwest` is deliberately built without its optional form feature
        // in this workspace.  `url::form_urlencoded` produces the exact
        // application/x-www-form-urlencoded encoding required by RFC 7009
        // without putting a secret into an error or debug value.
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("token", token);
        form.append_pair("token_type_hint", token_type_hint);
        form.append_pair("client_id", credentials.client_id.as_str());
        let response = client
            .post(revocation_endpoint)
            .header(
                reqwest::header::CONTENT_TYPE,
                "application/x-www-form-urlencoded",
            )
            .body(form.finish())
            .send()
            .await
            .map_err(|_| McpError::Auth("MCP OAuth revocation request failed".to_owned()))?;
        if !response.status().is_success() {
            return Err(McpError::Auth(format!(
                "MCP OAuth revocation endpoint returned HTTP {}",
                response.status().as_u16()
            )));
        }

        store.delete().map_err(|_| {
            McpError::Auth(
                "MCP OAuth credential was revoked remotely but could not be removed from the local keyring"
                    .to_owned(),
            )
        })
    }
}
