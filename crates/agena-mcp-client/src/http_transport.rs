//! Call-scoped HTTP execution with pinned addresses and native worker isolation.
//!
//! The parent retains MCP sessions and OAuth state. Every actual HTTP request
//! uses a fresh host-authorized policy; isolated sockets live in an MXC worker.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use agena_process::sandbox::SandboxPolicy;
use agena_process::sandbox_http::{HttpOptions, WorkerCancellation, execute};
use async_trait::async_trait;
use rmcp::model::{ClientJsonRpcMessage, ClientRequest, GetExtensions};
use rmcp::transport::streamable_http_client::{
    HttpRequestExecutor, RequestExecutingHttpClient, StreamableHttpError,
};
use url::{Host, Url};

use crate::{McpError, McpResult};

mod app_events;
pub(crate) use app_events::AppHttpEvents;

#[cfg(test)]
mod tests;

/// Active host authority for both MCP transport and OAuth requests.
#[async_trait]
pub trait McpHttpNetworkAuthorizer: Send + Sync {
    async fn authorize(&self, target: &str) -> McpResult<()>;

    /// Authorize each numeric address that may reach the socket connector.
    async fn authorize_resolved(&self, _target: &str, _address: SocketAddr) -> McpResult<bool> {
        Ok(false)
    }

    /// Compile the current call's execution mode and network ceiling. A missing
    /// implementation must not silently turn an isolated request into host I/O.
    async fn sandbox_policy(&self, _target: &str) -> McpResult<SandboxPolicy> {
        Err(McpError::PermissionDenied)
    }

    /// Compile the session policy for a trusted connection's local header provider.
    async fn header_helper_policy(&self, _cwd: &std::path::Path) -> McpResult<SandboxPolicy> {
        Err(McpError::PermissionDenied)
    }

    /// Lifetime of this authority, independent of whether an SSE body is being
    /// polled. Return a child token so consumers cannot cancel the owning call.
    fn cancellation(&self) -> Option<WorkerCancellation> {
        None
    }

    /// Optional detached authority for deleting the transport's own remote
    /// session. It must resolve current policy just like the normal authority.
    fn cleanup_authority(&self) -> Option<Arc<dyn McpHttpNetworkAuthorizer>> {
        None
    }
}

fn http_url(target: &str) -> McpResult<Url> {
    let url = Url::parse(target)
        .map_err(|_| McpError::Transport("MCP HTTP target is not a valid URL".into()))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host().is_none()
    {
        return Err(McpError::Transport(
            "MCP HTTP target must be an HTTP(S) URL without embedded credentials".into(),
        ));
    }
    Ok(url)
}

/// Authorization precedes DNS; only approved answers reach the worker. The
/// policy is refreshed for every POST, reconnecting SSE GET and OAuth redirect.
pub(crate) async fn authorized_options(
    target: &str,
    authorizer: &dyn McpHttpNetworkAuthorizer,
) -> McpResult<HttpOptions> {
    let url = http_url(target)?;
    let cancellation = authorizer.cancellation();
    if cancellation
        .as_ref()
        .is_some_and(WorkerCancellation::is_cancelled)
    {
        return Err(McpError::PermissionDenied);
    }
    authorizer.authorize(target).await?;
    let port = url
        .port_or_known_default()
        .ok_or_else(|| McpError::Transport("MCP HTTP target is missing its port".into()))?;
    let candidates = match url.host() {
        Some(Host::Ipv4(address)) => vec![SocketAddr::new(IpAddr::V4(address), port)],
        Some(Host::Ipv6(address)) => vec![SocketAddr::new(IpAddr::V6(address), port)],
        Some(Host::Domain(host)) => tokio::time::timeout(
            Duration::from_secs(20),
            tokio::net::lookup_host((host, port)),
        )
        .await
        .map_err(|_| McpError::Transport("MCP HTTP DNS resolution timed out".into()))?
        .map_err(|_| McpError::Transport("MCP HTTP DNS resolution failed".into()))?
        .collect(),
        None => return Err(McpError::PermissionDenied),
    };
    let mut addresses = Vec::new();
    for address in candidates {
        if !addresses.contains(&address) && authorizer.authorize_resolved(target, address).await? {
            addresses.push(address);
        }
    }
    if addresses.is_empty() {
        return Err(McpError::PermissionDenied);
    }
    Ok(HttpOptions {
        policy: authorizer.sandbox_policy(target).await?,
        pinned_addresses: addresses,
        connect_timeout: Duration::from_secs(20),
        total_timeout: None,
        max_request_bytes: 512 * 1024 * 1024,
        max_response_bytes: None,
        cancellation,
    })
}

pub(crate) async fn execute_authorized(
    request: reqwest::Request,
    authorizer: &dyn McpHttpNetworkAuthorizer,
    max_response_bytes: Option<u64>,
) -> McpResult<reqwest::Response> {
    let mut options = authorized_options(request.url().as_str(), authorizer).await?;
    options.max_response_bytes = max_response_bytes;
    execute_http(request, options).await
}

pub(crate) async fn execute_http(
    request: reqwest::Request,
    mut options: HttpOptions,
) -> McpResult<reqwest::Response> {
    options.total_timeout = request.timeout().copied();
    execute(request, options)
        .await
        .map_err(|error| McpError::Transport(format!("MCP HTTP execution failed: {error}")))
}

/// Trusted management traffic has no plugin-call authority. It remains
/// separate from every controlled tool request and never supplies a fallback.
pub(crate) fn trusted_client() -> McpResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| McpError::http_error(&error))
}

#[derive(Clone)]
pub(crate) struct McpRequestExecutor {
    authorizer: Arc<dyn McpHttpNetworkAuthorizer>,
    dynamic_headers: bool,
    remote_session_open: Arc<AtomicBool>,
    app_events: Option<AppHttpEvents>,
}

#[derive(Clone)]
pub(crate) struct ScopedHttpRequest {
    pub(crate) authority: Arc<dyn McpHttpNetworkAuthorizer>,
    pub(crate) require_connection_authority: bool,
}

impl ScopedHttpRequest {
    pub(crate) fn from_message(message: &ClientJsonRpcMessage) -> Option<&Self> {
        match message {
            ClientJsonRpcMessage::Request(request) => request.request.extensions().get(),
            ClientJsonRpcMessage::Notification(notification) => {
                notification.notification.extensions().get()
            }
            ClientJsonRpcMessage::Response(response) => response.extensions.get(),
            ClientJsonRpcMessage::Error(error) => error.extensions.get(),
        }
    }
}

struct RequestAuthority<'a> {
    connection: &'a dyn McpHttpNetworkAuthorizer,
    caller: &'a dyn McpHttpNetworkAuthorizer,
}

#[async_trait]
impl McpHttpNetworkAuthorizer for RequestAuthority<'_> {
    fn cancellation(&self) -> Option<WorkerCancellation> {
        self.caller.cancellation()
    }

    async fn authorize(&self, target: &str) -> McpResult<()> {
        self.connection.authorize(target).await?;
        self.caller.authorize(target).await
    }

    async fn authorize_resolved(&self, target: &str, address: SocketAddr) -> McpResult<bool> {
        Ok(self.connection.authorize_resolved(target, address).await?
            && self.caller.authorize_resolved(target, address).await?)
    }

    async fn sandbox_policy(&self, target: &str) -> McpResult<SandboxPolicy> {
        let session = self.connection.sandbox_policy(target).await?;
        let caller = self.caller.sandbox_policy(target).await?;
        if !session.disabled && caller.disabled {
            return Err(McpError::PermissionDenied);
        }
        Ok(caller)
    }
}

impl HttpRequestExecutor for McpRequestExecutor {
    async fn execute(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, StreamableHttpError<reqwest::Error>> {
        if request.method() == http::Method::DELETE {
            // Until DELETE succeeds, the remote session remains unconfirmed.
            let authority = self.authorizer.cleanup_authority();
            let response = self
                .execute_with_authority(
                    request,
                    authority.as_deref().unwrap_or(self.authorizer.as_ref()),
                )
                .await;
            if response.as_ref().is_ok_and(|response| {
                response.status().is_success()
                    || matches!(
                        response.status(),
                        http::StatusCode::NOT_FOUND | http::StatusCode::METHOD_NOT_ALLOWED
                    )
            }) {
                self.remote_session_open.store(false, Ordering::Release);
            }
            return response;
        }
        self.execute_with_authority(request, self.authorizer.as_ref())
            .await
    }

    async fn execute_message(
        &self,
        request: reqwest::Request,
        message: &ClientJsonRpcMessage,
    ) -> Result<reqwest::Response, StreamableHttpError<reqwest::Error>> {
        let endpoint = self
            .app_events
            .as_ref()
            .and_then(|_| app_events::LegacyEventEndpoint::from_request(&request));
        let response = if let Some(scope) = ScopedHttpRequest::from_message(message) {
            if scope.require_connection_authority {
                let authority = RequestAuthority {
                    connection: self.authorizer.as_ref(),
                    caller: scope.authority.as_ref(),
                };
                self.execute_with_authority(request, &authority).await
            } else {
                self.execute_with_authority(request, scope.authority.as_ref())
                    .await
            }
        } else {
            self.execute(request).await
        };
        if matches!(message, ClientJsonRpcMessage::Request(request) if matches!(&request.request, ClientRequest::InitializeRequest(_)))
            && let Ok(response) = &response
        {
            // Record the remote session as soon as the response supplies its
            // ID. Local cancellation can otherwise skip DELETE altogether.
            let opened = response.headers().contains_key("mcp-session-id");
            if opened || response.status().is_success() {
                self.remote_session_open.store(opened, Ordering::Release);
            }
        }
        if response.as_ref().is_ok_and(|response| response.status().is_success())
            && let (Some(events), Some(endpoint)) = (&self.app_events, endpoint)
        {
            events.remember(endpoint);
        }
        response
    }
}

impl McpRequestExecutor {
    async fn execute_with_authority(
        &self,
        request: reqwest::Request,
        authority: &dyn McpHttpNetworkAuthorizer,
    ) -> Result<reqwest::Response, StreamableHttpError<reqwest::Error>> {
        let mut options = authorized_options(request.url().as_str(), authority)
            .await
            .map_err(|error| {
                StreamableHttpError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    error,
                ))
            })?;
        options.total_timeout = request.timeout().copied();
        // Streaming responses remain open for their MCP call lifetime. Dropping
        // the response, cancelling a request or retiring its manager kills its
        // worker, including reconnecting SSE streams.
        let response = execute(request, options)
            .await
            .map_err(StreamableHttpError::Io)?;
        if self.dynamic_headers {
            use rmcp::transport::streamable_http_client::{
                AuthRequiredError, InsufficientScopeError,
            };
            match response.status() {
                http::StatusCode::UNAUTHORIZED => {
                    return Err(StreamableHttpError::AuthRequired(AuthRequiredError::new(
                        String::new(),
                    )));
                }
                http::StatusCode::FORBIDDEN => {
                    return Err(StreamableHttpError::InsufficientScope(
                        InsufficientScopeError::new(String::new(), None),
                    ));
                }
                _ => {}
            }
        }
        Ok(response)
    }
}

pub(crate) fn authorized_mcp_client(
    authorizer: Arc<dyn McpHttpNetworkAuthorizer>,
    dynamic_headers: bool,
    remote_session_open: Arc<AtomicBool>,
    app_events: Option<AppHttpEvents>,
) -> McpResult<RequestExecutingHttpClient<McpRequestExecutor>> {
    Ok(RequestExecutingHttpClient::new(
        // This client only builds Request values; it never opens a connection.
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpError::http_error(&error))?,
        McpRequestExecutor {
            authorizer,
            dynamic_headers,
            remote_session_open,
            app_events,
        },
    ))
}
