//! HTTP transports whose actual socket addresses are authorized by the caller.
//!
//! A request gets a fresh client with only its approved DNS answers. No client
//! or connection pool carrying another call's authority is reused.

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::stream::BoxStream;
use http::{HeaderName, HeaderValue};
use rmcp::model::ClientJsonRpcMessage;
use rmcp::transport::streamable_http_client::{
    SseError, StreamableHttpClient, StreamableHttpError, StreamableHttpPostResponse,
};
use sse_stream::Sse;
use url::{Host, Url};

use crate::{McpError, McpResult};

#[cfg(test)]
mod tests;

/// An active host authority for MCP transport and OAuth HTTP requests.
/// Implementations must preserve the originating call's context and lifetime.
#[async_trait]
pub trait McpHttpNetworkAuthorizer: Send + Sync {
    /// Authorize the original URL, including permission to resolve its hostname.
    async fn authorize(&self, target: &str) -> McpResult<()>;

    /// Check the exact numeric address that the HTTP client will connect to.
    /// An implementation without resolved-address support cannot authorize I/O.
    async fn authorize_resolved(&self, _target: &str, _address: SocketAddr) -> McpResult<bool> {
        Ok(false)
    }
}

struct RejectUnpinnedDns;

impl reqwest::dns::Resolve for RejectUnpinnedDns {
    fn resolve(&self, _name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        Box::pin(async {
            Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "MCP HTTP transport cannot resolve an unpinned hostname",
            )
            .into())
        })
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

fn pinned_client(url: &Url, addresses: &[SocketAddr]) -> McpResult<reqwest::Client> {
    if addresses.is_empty() {
        return Err(McpError::PermissionDenied);
    }
    let host = url
        .host_str()
        .ok_or_else(|| McpError::Transport("MCP HTTP target is missing its host".into()))?;
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .dns_resolver(Arc::new(RejectUnpinnedDns))
        .resolve_to_addrs(host, addresses)
        .build()
        .map_err(|error| McpError::http_error(&error))
}

/// Called for every request and every OAuth redirect. Authorization happens
/// before DNS; only separately approved numeric answers enter the transport.
pub(crate) async fn authorized_client(
    target: &str,
    authorizer: &dyn McpHttpNetworkAuthorizer,
) -> McpResult<reqwest::Client> {
    let url = http_url(target)?;
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
    pinned_client(&url, &addresses)
}

/// The trusted management path has no plugin-call authority. Its client is
/// kept separate from every controlled plugin operation.
pub(crate) fn trusted_client() -> McpResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(20))
        .build()
        .map_err(|error| McpError::http_error(&error))
}

#[derive(Clone)]
pub(crate) struct AuthorizedMcpHttpClient {
    authorizer: Arc<dyn McpHttpNetworkAuthorizer>,
}

impl AuthorizedMcpHttpClient {
    pub(crate) fn new(authorizer: Arc<dyn McpHttpNetworkAuthorizer>) -> Self {
        Self { authorizer }
    }

    async fn client(
        &self,
        target: &str,
    ) -> Result<reqwest::Client, StreamableHttpError<reqwest::Error>> {
        authorized_client(target, self.authorizer.as_ref())
            .await
            .map_err(|error| {
                StreamableHttpError::Io(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    error,
                ))
            })
    }
}

impl StreamableHttpClient for AuthorizedMcpHttpClient {
    type Error = reqwest::Error;

    async fn post_message(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.client(&uri)
            .await?
            .post_message(uri, message, session_id, auth_header, custom_headers)
            .await
    }

    async fn post_message_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        message: ClientJsonRpcMessage,
        session_id: Option<Arc<str>>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<StreamableHttpPostResponse, StreamableHttpError<Self::Error>> {
        self.client(&uri)
            .await?
            .post_message_with_max_sse_event_size(
                uri,
                message,
                session_id,
                auth_header,
                custom_headers,
                max_sse_event_size,
            )
            .await
    }

    async fn delete_session(
        &self,
        uri: Arc<str>,
        session_id: Arc<str>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<(), StreamableHttpError<Self::Error>> {
        self.client(&uri)
            .await?
            .delete_session(uri, session_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.client(&uri)
            .await?
            .get_stream(uri, session_id, last_event_id, auth_header, custom_headers)
            .await
    }

    async fn get_stream_with_max_sse_event_size(
        &self,
        uri: Arc<str>,
        session_id: Option<Arc<str>>,
        last_event_id: Option<String>,
        auth_header: Option<String>,
        custom_headers: HashMap<HeaderName, HeaderValue>,
        max_sse_event_size: usize,
    ) -> Result<BoxStream<'static, Result<Sse, SseError>>, StreamableHttpError<Self::Error>> {
        self.client(&uri)
            .await?
            .get_stream_with_max_sse_event_size(
                uri,
                session_id,
                last_event_id,
                auth_header,
                custom_headers,
                max_sse_event_size,
            )
            .await
    }
}
