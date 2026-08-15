use std::{
    collections::HashMap,
    sync::Mutex,
    time::{Duration, Instant},
};

use agena_mcp_client::{
    FileTokenStore, KeyringOAuthCredentialStore, KeyringTokenStore, McpOAuthLoginSession,
};

use crate::{
    Application, ApplicationError,
    dto::{
        McpCredentialKindResource, McpCredentialMutationResource, McpCredentialStoreResource,
        McpOAuthFinishRequest, McpOAuthStartRequest, McpOAuthStartResource,
    },
};

const MCP_OAUTH_FLOW_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_PENDING_MCP_OAUTH_FLOWS: usize = 16;

struct PendingMcpOAuthFlow {
    server: String,
    created_at: Instant,
    session: McpOAuthLoginSession,
}

#[derive(Default)]
pub(crate) struct McpOAuthFlowRegistry {
    flows: Mutex<HashMap<uuid::Uuid, PendingMcpOAuthFlow>>,
}

impl McpOAuthFlowRegistry {
    fn insert(
        &self,
        server: String,
        session: McpOAuthLoginSession,
    ) -> Result<uuid::Uuid, ApplicationError> {
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| ApplicationError::internal("MCP OAuth flow registry is unavailable"))?;
        flows.retain(|_, flow| flow.created_at.elapsed() < MCP_OAUTH_FLOW_TTL);
        if flows.len() >= MAX_PENDING_MCP_OAUTH_FLOWS {
            return Err(ApplicationError::conflict(
                "Too many MCP OAuth logins are already pending. Finish one or try again later.",
            ));
        }
        let flow_id = uuid::Uuid::new_v4();
        flows.insert(
            flow_id,
            PendingMcpOAuthFlow {
                server,
                created_at: Instant::now(),
                session,
            },
        );
        Ok(flow_id)
    }

    fn take(&self, flow_id: uuid::Uuid) -> Result<PendingMcpOAuthFlow, ApplicationError> {
        let mut flows = self
            .flows
            .lock()
            .map_err(|_| ApplicationError::internal("MCP OAuth flow registry is unavailable"))?;
        flows.retain(|_, flow| flow.created_at.elapsed() < MCP_OAUTH_FLOW_TTL);
        flows.remove(&flow_id).ok_or_else(|| {
            ApplicationError::not_found(
                "The MCP OAuth login is missing or expired. Start browser login again.",
            )
        })
    }
}

impl Application {
    pub fn set_mcp_bearer_credential(
        &self,
        server: &str,
        token: &str,
        store: McpCredentialStoreResource,
    ) -> Result<McpCredentialMutationResource, ApplicationError> {
        let server = normalized_server_name(server)?;
        if token.trim().is_empty() {
            return Err(ApplicationError::bad_request(
                "The MCP bearer token must not be empty.",
            ));
        }
        match store {
            McpCredentialStoreResource::Keyring => KeyringTokenStore::new()
                .put_bearer(server.as_str(), token)
                .map_err(|_| {
                    ApplicationError::internal("Could not store the MCP bearer token in keyring")
                })?,
            McpCredentialStoreResource::File => FileTokenStore::open_default()
                .and_then(|store| store.put_bearer(server.as_str(), token))
                .map_err(|_| {
                    ApplicationError::internal(
                        "Could not store the MCP bearer token in the file store",
                    )
                })?,
        }
        Ok(mcp_credential_result(
            server,
            McpCredentialKindResource::Bearer,
            store,
            "stored",
        ))
    }

    pub fn delete_mcp_bearer_credential(
        &self,
        server: &str,
        store: McpCredentialStoreResource,
    ) -> Result<McpCredentialMutationResource, ApplicationError> {
        let server = normalized_server_name(server)?;
        match store {
            McpCredentialStoreResource::Keyring => KeyringTokenStore::new()
                .delete(server.as_str())
                .map_err(|_| {
                    ApplicationError::internal("Could not remove the MCP bearer token from keyring")
                })?,
            McpCredentialStoreResource::File => {
                FileTokenStore::open_default()
                    .and_then(|store| store.delete(server.as_str()).map(|_| ()))
                    .map_err(|_| {
                        ApplicationError::internal(
                            "Could not remove the MCP bearer token from the file store",
                        )
                    })?;
            }
        }
        Ok(mcp_credential_result(
            server,
            McpCredentialKindResource::Bearer,
            store,
            "removed",
        ))
    }

    pub async fn start_mcp_oauth(
        &self,
        request: McpOAuthStartRequest,
    ) -> Result<McpOAuthStartResource, ApplicationError> {
        let server = normalized_server_name(request.server.as_str())?;
        let endpoint = parse_http_endpoint(request.url.as_str())?;
        validate_loopback_redirect(request.redirect_uri.as_str())?;
        let scopes = request
            .scopes
            .into_iter()
            .map(|scope| scope.trim().to_owned())
            .filter(|scope| !scope.is_empty())
            .collect::<Vec<_>>();
        let session = McpOAuthLoginSession::begin(
            server.as_str(),
            endpoint,
            scopes.as_slice(),
            request.redirect_uri.as_str(),
        )
        .await
        .map_err(|_| {
            ApplicationError::service_unavailable(
                "MCP OAuth discovery or dynamic client registration failed",
            )
        })?;
        let authorization_url = session.authorization_url().to_owned();
        let flow_id = self.mcp_oauth_flows.insert(server.clone(), session)?;
        Ok(McpOAuthStartResource {
            flow_id,
            server,
            authorization_url,
        })
    }

    pub async fn finish_mcp_oauth(
        &self,
        request: McpOAuthFinishRequest,
    ) -> Result<McpCredentialMutationResource, ApplicationError> {
        if request.code.trim().is_empty() || request.state.trim().is_empty() {
            return Err(ApplicationError::bad_request(
                "The MCP OAuth callback code and state are required.",
            ));
        }
        let flow = self.mcp_oauth_flows.take(request.flow_id)?;
        flow.session
            .complete(
                request.code.as_str(),
                request.state.as_str(),
                request.issuer.as_deref(),
            )
            .await
            .map_err(|_| {
                ApplicationError::bad_request(
                    "The MCP OAuth callback could not be verified or exchanged.",
                )
            })?;
        Ok(mcp_credential_result(
            flow.server,
            McpCredentialKindResource::OAuth,
            McpCredentialStoreResource::Keyring,
            "stored",
        ))
    }

    pub async fn delete_mcp_oauth_credential(
        &self,
        server: &str,
        revoke: bool,
        endpoint: Option<&str>,
    ) -> Result<McpCredentialMutationResource, ApplicationError> {
        let server = normalized_server_name(server)?;
        let action = if revoke {
            let endpoint = endpoint
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .ok_or_else(|| {
                    ApplicationError::bad_request(
                        "Revoking an MCP OAuth credential requires the MCP endpoint URL.",
                    )
                })?;
            McpOAuthLoginSession::revoke_and_clear(server.as_str(), parse_http_endpoint(endpoint)?)
                .await
                .map_err(|error| {
                    ApplicationError::bad_request_with_diagnostic(
                        "The MCP OAuth credential could not be revoked.",
                        error,
                    )
                })?;
            "revoked"
        } else {
            if endpoint.is_some() {
                return Err(ApplicationError::bad_request(
                    "An MCP endpoint URL is valid only when revocation is requested.",
                ));
            }
            KeyringOAuthCredentialStore::new(server.as_str())
                .and_then(|store| store.delete())
                .map_err(|_| {
                    ApplicationError::internal(
                        "Could not remove the MCP OAuth credential from keyring",
                    )
                })?;
            "removed"
        };
        Ok(mcp_credential_result(
            server,
            McpCredentialKindResource::OAuth,
            McpCredentialStoreResource::Keyring,
            action,
        ))
    }
}

fn normalized_server_name(server: &str) -> Result<String, ApplicationError> {
    let server = server.trim();
    if server.is_empty() {
        return Err(ApplicationError::bad_request(
            "The MCP server name must not be empty.",
        ));
    }
    Ok(server.to_owned())
}

fn parse_http_endpoint(raw: &str) -> Result<url::Url, ApplicationError> {
    let endpoint = url::Url::parse(raw.trim()).map_err(|_| {
        ApplicationError::bad_request(
            "The MCP OAuth endpoint must be an HTTP(S) URL without embedded credentials.",
        )
    })?;
    if !matches!(endpoint.scheme(), "http" | "https")
        || endpoint.host_str().is_none()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
    {
        return Err(ApplicationError::bad_request(
            "The MCP OAuth endpoint must be an HTTP(S) URL without embedded credentials.",
        ));
    }
    Ok(endpoint)
}

fn validate_loopback_redirect(raw: &str) -> Result<(), ApplicationError> {
    let redirect = url::Url::parse(raw.trim()).map_err(|_| {
        ApplicationError::bad_request(
            "The MCP OAuth redirect must use an explicit loopback HTTP callback.",
        )
    })?;
    if redirect.scheme() != "http"
        || redirect.host_str() != Some("127.0.0.1")
        || redirect.port().is_none()
        || redirect.path() != "/callback"
        || !redirect.username().is_empty()
        || redirect.password().is_some()
        || redirect.query().is_some()
        || redirect.fragment().is_some()
    {
        return Err(ApplicationError::bad_request(
            "The MCP OAuth redirect must use an explicit loopback HTTP callback.",
        ));
    }
    Ok(())
}

fn mcp_credential_result(
    server: String,
    credential_kind: McpCredentialKindResource,
    store: McpCredentialStoreResource,
    action: &str,
) -> McpCredentialMutationResource {
    McpCredentialMutationResource {
        server,
        credential_kind,
        store,
        action: action.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn oauth_endpoint_and_redirect_validation_reject_credentials_and_non_loopback_callbacks() {
        assert!(parse_http_endpoint("https://example.test/mcp").is_ok());
        assert!(parse_http_endpoint("https://token@example.test/mcp").is_err());
        assert!(validate_loopback_redirect("http://127.0.0.1:1455/callback").is_ok());
        assert!(validate_loopback_redirect("https://example.test/callback").is_err());
        assert!(validate_loopback_redirect("http://127.0.0.1/callback").is_err());
    }
}
