//! Claude Code's MCP WebSocket transport over a pinned sandbox connection.

use std::sync::Arc;

use agena_process::{sandbox_http::WorkerCancellation, sandbox_tcp, sandbox_tcp::Socket};
use futures::{SinkExt, StreamExt};
use http::{HeaderMap, HeaderValue, header};
use rmcp::{
    RoleClient,
    model::ServerJsonRpcMessage,
    transport::worker::{Worker, WorkerContext, WorkerQuitReason, WorkerTransport},
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream, client_async_tls_with_config,
    tungstenite::{Message, client::IntoClientRequest, protocol::WebSocketConfig},
};
use url::Url;

use crate::{
    McpError, McpHttpNetworkAuthorizer, McpResult, http_transport::authorized_options,
    legacy_app::LegacyAppPeer,
};

const MAX_MESSAGE_BYTES: usize = 16 * 1024 * 1024;

pub(crate) async fn connect(
    mut url: Url,
    headers: HeaderMap,
    authority: Arc<dyn McpHttpNetworkAuthorizer>,
    retain_for_app: bool,
) -> McpResult<WorkerTransport<WebSocket>> {
    let options = authorized_options(url.as_str(), authority.as_ref()).await?;
    let target = retain_for_app.then(|| url.to_string());
    let scheme = if url.scheme() == "https" { "wss" } else { "ws" };
    url.set_scheme(scheme)
        .map_err(|()| McpError::Transport("invalid MCP WebSocket URL".into()))?;
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|_| McpError::Transport("invalid MCP WebSocket request".into()))?;
    for (name, value) in &headers {
        if !request.headers().contains_key(name) && !name.as_str().starts_with("sec-websocket-") {
            request.headers_mut().insert(name, value.clone());
        }
    }
    request.headers_mut().insert(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static("mcp"),
    );
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE_BYTES))
        .max_frame_size(Some(MAX_MESSAGE_BYTES));
    let cancellation = options.cancellation.unwrap_or_default();
    let sandbox_disabled = options.policy.disabled;
    let dial = async {
        let socket = sandbox_tcp::connect(options.policy, options.pinned_addresses).await?;
        let address = socket.peer_addr();
        let (stream, _) = client_async_tls_with_config(request, socket, Some(config), None)
            .await
            .map_err(websocket_error)?;
        Ok::<_, McpError>((stream, address))
    };
    let (stream, address) = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(McpError::PermissionDenied),
        result = tokio::time::timeout(options.connect_timeout, dial) => {
            result.map_err(|_| McpError::Timeout("MCP WebSocket connection timed out".into()))??
        }
    };
    let lifetime = if retain_for_app {
        WorkerCancellation::new()
    } else {
        cancellation
    };
    let app = target.map(|target| {
        (
            target,
            authority,
            LegacyAppPeer::new(address, sandbox_disabled, lifetime.clone()),
        )
    });
    Ok(WorkerTransport::spawn_with_ct(
        WebSocket { stream, app },
        lifetime,
    ))
}

fn websocket_error(error: tokio_tungstenite::tungstenite::Error) -> McpError {
    use tokio_tungstenite::tungstenite::Error;
    // Handshake errors may carry a response body or headers from the peer.
    match error {
        Error::Http(response) if matches!(response.status().as_u16(), 401 | 403) => {
            McpError::Auth(format!("MCP WebSocket returned HTTP {}", response.status()))
        }
        Error::Http(response) => {
            McpError::Http(format!("MCP WebSocket returned HTTP {}", response.status()))
        }
        Error::Io(error) => McpError::Io(error),
        Error::ConnectionClosed | Error::AlreadyClosed => McpError::TransportClosed,
        _ => McpError::Transport("MCP WebSocket protocol or TLS error".into()),
    }
}

pub(crate) struct WebSocket {
    stream: WebSocketStream<MaybeTlsStream<Socket>>,
    app: Option<(String, Arc<dyn McpHttpNetworkAuthorizer>, LegacyAppPeer)>,
}

impl Worker for WebSocket {
    type Error = McpError;
    type Role = RoleClient;

    fn err_closed() -> Self::Error {
        McpError::TransportClosed
    }

    fn err_join(error: tokio::task::JoinError) -> Self::Error {
        McpError::transport_error(&error)
    }

    async fn run(self, context: WorkerContext<Self>) -> Result<(), WorkerQuitReason<McpError>> {
        let WorkerContext {
            to_handler_tx,
            mut from_handler_rx,
            cancellation_token,
            ..
        } = context;
        let Self { stream, app } = self;
        let (mut sender, mut receiver) = stream.split();
        let receive = async {
            while let Some(frame) = receiver.next().await {
                let frame =
                    frame
                        .map_err(websocket_error)
                        .map_err(WorkerQuitReason::fatal_context(
                            "receiving MCP WebSocket frame",
                        ))?;
                if frame.is_close() {
                    break;
                }
                if !frame.is_text() && !frame.is_binary() {
                    continue;
                }
                let mut message = serde_json::from_slice::<ServerJsonRpcMessage>(
                    &frame.into_data(),
                )
                .map_err(|_| {
                    WorkerQuitReason::fatal(
                        McpError::Malformed("invalid MCP WebSocket JSON-RPC message".into()),
                        "receiving MCP WebSocket frame",
                    )
                })?;
                if app
                    .as_ref()
                    .is_some_and(|(_, _, peer)| !peer.receive(&mut message))
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
                let mut interrupted = false;
                let result = async {
                    let json = serde_json::to_string(&outbound.message)?;
                    if json.len() > MAX_MESSAGE_BYTES {
                        return Err(McpError::Transport(
                            "MCP WebSocket message exceeds the byte limit".into(),
                        ));
                    }
                    if let Some((target, authority, peer)) = &app {
                        let permission = peer.authorize_send(target, authority, &outbound.message).await?;
                        let cancellation = permission.cancellation.unwrap_or_default();
                        if cancellation.is_cancelled() {
                            return Err(McpError::PermissionDenied);
                        }
                        tokio::select! {
                            biased;
                            _ = cancellation.cancelled() => {
                                // A partially written frame cannot be retried
                                // on a later call's authority.
                                interrupted = true;
                                Err(McpError::PermissionDenied)
                            }
                            result = sender.send(Message::text(json)) => result.map_err(websocket_error),
                        }
                    } else {
                        sender.send(Message::text(json)).await.map_err(websocket_error)
                    }
                }
                .await;
                if let Some((_, _, peer)) = &app {
                    peer.sent(&outbound.message, result.is_ok());
                }
                let _ = outbound.responder.send(result);
                if interrupted {
                    return Err(WorkerQuitReason::Cancelled);
                }
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
