//! Call-scoped HTTP execution with pinned addresses and native worker isolation.
//!
//! The parent retains MCP sessions and OAuth state. Every actual HTTP request
//! uses a fresh host-authorized policy; isolated sockets live in an MXC worker.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use agena_process::sandbox::SandboxPolicy;
use agena_process::sandbox_http::{HttpOptions, WorkerCancellation, execute};
use async_trait::async_trait;
use rmcp::transport::streamable_http_client::{
    HttpRequestExecutor, RequestExecutingHttpClient, StreamableHttpError,
};
use url::{Host, Url};

use crate::{McpError, McpResult};

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

    /// Lifetime of this authority, independent of whether an SSE body is being
    /// polled. Return a child token so consumers cannot cancel the owning call.
    fn cancellation(&self) -> Option<WorkerCancellation> {
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
    options.total_timeout = request.timeout().copied();
    options.max_response_bytes = max_response_bytes;
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
}

impl HttpRequestExecutor for McpRequestExecutor {
    async fn execute(
        &self,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, StreamableHttpError<reqwest::Error>> {
        let mut options = authorized_options(request.url().as_str(), self.authorizer.as_ref())
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
        execute(request, options)
            .await
            .map_err(StreamableHttpError::Io)
    }
}

pub(crate) fn authorized_mcp_client(
    authorizer: Arc<dyn McpHttpNetworkAuthorizer>,
) -> McpResult<RequestExecutingHttpClient<McpRequestExecutor>> {
    Ok(RequestExecutingHttpClient::new(
        // This client only builds Request values; it never opens a connection.
        reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|error| McpError::http_error(&error))?,
        McpRequestExecutor { authorizer },
    ))
}
