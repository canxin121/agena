//! MCP 2024-11-05 transport: receive on one SSE stream and send by POST.

use std::sync::Arc;

use agena_process::sandbox_http::{WorkerCancellation, response_peer_addr};
use futures::StreamExt;
use http::{HeaderMap, HeaderValue, Method, header};
use rmcp::{
    RoleClient,
    model::ServerJsonRpcMessage,
    transport::{
        common::client_side_sse::{
            BoxedSseResponse, DEFAULT_MAX_SSE_EVENT_SIZE, bounded_sse_stream,
        },
        worker::{Worker, WorkerContext, WorkerQuitReason, WorkerTransport},
    },
};
use url::Url;

use crate::{
    McpError, McpHttpNetworkAuthorizer, McpResult,
    http_transport::{authorized_options, execute_authorized, execute_http},
    legacy_app::LegacyAppPeer,
};

pub(crate) async fn connect(
    url: Url,
    headers: HeaderMap,
    authority: Arc<dyn McpHttpNetworkAuthorizer>,
    retain_for_app: bool,
) -> McpResult<WorkerTransport<LegacySse>> {
    let mut request = reqwest::Request::new(Method::GET, url.clone());
    *request.headers_mut() = headers.clone();
    request.headers_mut().insert(
        header::ACCEPT,
        HeaderValue::from_static("text/event-stream"),
    );
    let mut options = authorized_options(url.as_str(), authority.as_ref()).await?;
    let setup = options.cancellation.clone().unwrap_or_default();
    let lifetime = WorkerCancellation::new();
    let opening = lifetime.clone().drop_guard();
    if retain_for_app {
        options.cancellation = Some(lifetime.child_token());
    }
    let open = async {
        let sandbox_disabled = options.policy.disabled;
        let response = execute_http(request, options).await?;
        check_status(&response)?;
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default();
        if !content_type
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .eq_ignore_ascii_case("text/event-stream")
        {
            return Err(McpError::Malformed(
                "legacy MCP GET must return text/event-stream".into(),
            ));
        }
        let app_peer = if retain_for_app {
            let peer = response_peer_addr(&response).ok_or_else(|| {
                McpError::Transport("legacy MCP SSE peer address is unavailable".into())
            })?;
            Some(LegacyAppPeer::new(peer, sandbox_disabled, lifetime.clone()))
        } else {
            None
        };
        let mut events = bounded_sse_stream(response.bytes_stream(), DEFAULT_MAX_SSE_EVENT_SIZE);
        let endpoint = loop {
            let event = events
                .next()
                .await
                .ok_or(McpError::TransportClosed)?
                .map_err(event_error)?;
            if event.event.as_deref() == Some("endpoint") {
                break post_endpoint(&url, event.data.as_deref().unwrap_or_default())?;
            }
        };
        Ok(LegacySse {
            events,
            endpoint,
            headers,
            authority,
            app_peer,
        })
    };
    let worker = tokio::select! {
        biased;
        _ = setup.cancelled() => return Err(McpError::PermissionDenied),
        result = open => result?,
    };
    Ok(WorkerTransport::spawn_with_ct(worker, opening.disarm()))
}

fn post_endpoint(origin: &Url, endpoint: &str) -> McpResult<Url> {
    let endpoint = endpoint.trim();
    if endpoint.is_empty() || endpoint.len() > 8192 {
        return Err(McpError::Malformed(
            "legacy MCP SSE did not supply a valid POST endpoint".into(),
        ));
    }
    let endpoint = origin
        .join(endpoint)
        .map_err(|_| McpError::Malformed("invalid legacy MCP SSE POST endpoint".into()))?;
    // Both requests carry the same headers, including any configured credentials.
    if endpoint.origin() != origin.origin()
        || !endpoint.username().is_empty()
        || endpoint.password().is_some()
        || endpoint.fragment().is_some()
    {
        return Err(McpError::PermissionDenied);
    }
    Ok(endpoint)
}

fn check_status(response: &reqwest::Response) -> McpResult<()> {
    let status = response.status();
    if matches!(status.as_u16(), 401 | 403) {
        return Err(McpError::Auth(format!(
            "legacy MCP SSE returned HTTP {status}"
        )));
    }
    if !status.is_success() {
        return Err(McpError::Http(format!(
            "legacy MCP SSE returned HTTP {status}"
        )));
    }
    Ok(())
}

fn event_error(_: sse_stream::Error) -> McpError {
    McpError::Malformed("invalid legacy MCP SSE event stream".into())
}

pub(crate) struct LegacySse {
    events: BoxedSseResponse,
    endpoint: Url,
    headers: HeaderMap,
    authority: Arc<dyn McpHttpNetworkAuthorizer>,
    app_peer: Option<LegacyAppPeer>,
}

impl Worker for LegacySse {
    type Error = McpError;
    type Role = RoleClient;

    fn err_closed() -> Self::Error {
        McpError::TransportClosed
    }

    fn err_join(error: tokio::task::JoinError) -> Self::Error {
        McpError::transport_error(&error)
    }

    async fn run(self, context: WorkerContext<Self>) -> Result<(), WorkerQuitReason<McpError>> {
        let Self {
            mut events,
            endpoint,
            headers,
            authority,
            app_peer,
        } = self;
        let WorkerContext {
            to_handler_tx,
            mut from_handler_rx,
            cancellation_token,
            ..
        } = context;
        let receive = async {
            while let Some(event) = events.next().await {
                let event = event
                    .map_err(event_error)
                    .map_err(WorkerQuitReason::fatal_context("reading legacy SSE"))?;
                let Some(data) = event.data else { continue };
                if !matches!(event.event.as_deref(), None | Some("message")) {
                    continue;
                }
                let mut message =
                    serde_json::from_str::<ServerJsonRpcMessage>(&data).map_err(|_| {
                        WorkerQuitReason::fatal(
                            McpError::Malformed("invalid legacy MCP JSON-RPC message".into()),
                            "reading legacy SSE",
                        )
                    })?;
                if app_peer
                    .as_ref()
                    .is_some_and(|peer| !peer.receive(&mut message))
                {
                    continue;
                }
                to_handler_tx
                    .send(message)
                    .await
                    .map_err(|_| WorkerQuitReason::HandlerTerminated)?;
            }
            Err(WorkerQuitReason::TransportClosed)
        };
        let send = async {
            while let Some(outbound) = from_handler_rx.recv().await {
                let result = async {
                    let mut request = reqwest::Request::new(Method::POST, endpoint.clone());
                    *request.headers_mut() = headers.clone();
                    request.headers_mut().insert(
                        header::CONTENT_TYPE,
                        HeaderValue::from_static("application/json"),
                    );
                    *request.body_mut() = Some(serde_json::to_vec(&outbound.message)?.into());
                    let response = if let Some(peer) = &app_peer {
                        let options = peer
                            .authorize_send(endpoint.as_str(), &authority, &outbound.message)
                            .await?;
                        execute_http(request, options).await?
                    } else {
                        execute_authorized(request, authority.as_ref(), None).await?
                    };
                    check_status(&response)
                }
                .await;
                if let Some(peer) = &app_peer {
                    peer.sent(&outbound.message, result.is_ok());
                }
                let _ = outbound.responder.send(result);
            }
            Err(WorkerQuitReason::HandlerTerminated)
        };
        tokio::select! {
            biased;
            _ = cancellation_token.cancelled() => Err(WorkerQuitReason::Cancelled),
            result = receive => result,
            result = send => result,
        }
    }
}

#[cfg(test)]
#[test]
fn post_endpoint_preserves_the_get_origin() {
    let origin = Url::parse("https://example.test:8443/mcp/sse?token=fixture").unwrap();
    assert_eq!(
        post_endpoint(&origin, "../send?session=1")
            .unwrap()
            .as_str(),
        "https://example.test:8443/send?session=1"
    );
    for endpoint in [
        "//other.test:8443/send",
        "http://example.test:8443/send",
        "https://example.test/send",
        "https://user@example.test:8443/send",
        "/send#fragment",
    ] {
        assert!(matches!(
            post_endpoint(&origin, endpoint),
            Err(McpError::PermissionDenied)
        ));
    }
}
