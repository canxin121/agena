use std::sync::Mutex;

use futures::{StreamExt, stream::BoxStream};
use http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use rmcp::transport::common::client_side_sse::{DEFAULT_MAX_SSE_EVENT_SIZE, bounded_sse_stream};
use sse_stream::{Error as SseError, Sse};

use super::*;

/// Legacy Streamable HTTP notifications use a separate GET on the original
/// MCP session. Keep the headers of its last successful POST, including the
/// negotiated version and credentials already applied by the MCP client.
#[derive(Clone)]
pub(super) struct LegacyEventEndpoint {
    url: Url,
    headers: HeaderMap,
}

impl LegacyEventEndpoint {
    pub(super) fn from_request(request: &reqwest::Request) -> Option<Self> {
        if request.method() != Method::POST || !request.headers().contains_key("mcp-session-id") {
            return None;
        }
        let mut headers = request.headers().clone();
        headers.remove(header::CONTENT_TYPE);
        headers.remove(header::CONTENT_LENGTH);
        headers.remove(header::TRANSFER_ENCODING);
        headers.insert(
            header::ACCEPT,
            HeaderValue::from_static("text/event-stream"),
        );
        Some(Self {
            url: request.url().clone(),
            headers,
        })
    }
}

#[derive(Clone, Default)]
pub(crate) struct AppHttpEvents(Arc<Mutex<Option<LegacyEventEndpoint>>>);

impl AppHttpEvents {
    pub(super) fn remember(&self, endpoint: LegacyEventEndpoint) {
        *self.0.lock().unwrap_or_else(|error| error.into_inner()) = Some(endpoint);
    }

    /// Open only the original receive endpoint with the App's live authority.
    /// The caller owns the body and closes it when the catalog watch ends.
    pub(crate) async fn open_legacy(
        &self,
        authority: &dyn McpHttpNetworkAuthorizer,
    ) -> McpResult<Option<BoxStream<'static, Result<Sse, SseError>>>> {
        let endpoint = self
            .0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .clone();
        let Some(endpoint) = endpoint else {
            return Ok(None);
        };
        let mut request = reqwest::Request::new(Method::GET, endpoint.url);
        *request.headers_mut() = endpoint.headers;
        let response = execute_authorized(request, authority, None).await?;
        if response.status() == StatusCode::METHOD_NOT_ALLOWED {
            return Ok(None);
        }
        let response = response
            .error_for_status()
            .map_err(|error| McpError::http_error(&error.without_url()))?;
        if !response
            .headers()
            .get(header::CONTENT_TYPE)
            .is_some_and(|value| value.as_bytes().starts_with(b"text/event-stream"))
        {
            return Ok(None);
        }
        Ok(Some(
            bounded_sse_stream(response.bytes_stream(), DEFAULT_MAX_SSE_EVENT_SIZE).boxed(),
        ))
    }
}
