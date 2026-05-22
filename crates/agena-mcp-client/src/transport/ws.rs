//! WebSocket-based MCP transport.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use futures_util::{SinkExt, StreamExt};
use http::{HeaderName, HeaderValue};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{Message, client::IntoClientRequest},
};
use url::Url;

use crate::error::{McpError, McpResult};
use crate::protocol::InboundMessage;
use crate::transport::McpTransport;

pub struct WsTransport {
    inner: Arc<Inner>,
}

struct Inner {
    outbox: Mutex<Option<mpsc::UnboundedSender<Message>>>,
    inbox: Mutex<mpsc::UnboundedReceiver<McpResult<InboundMessage>>>,
    inbox_tx: mpsc::UnboundedSender<McpResult<InboundMessage>>,
}

impl WsTransport {
    pub async fn connect(url: Url, headers: HashMap<String, String>) -> McpResult<Self> {
        let request = handshake_request(&url, &headers)?;
        let (socket, _) = connect_async(request)
            .await
            .map_err(|err| McpError::Transport(format!("websocket connect failed: {err}")))?;
        let (mut writer, mut reader) = socket.split();

        let (outbox_tx, mut outbox_rx) = mpsc::unbounded_channel::<Message>();
        let (inbox_tx, inbox_rx) = mpsc::unbounded_channel::<McpResult<InboundMessage>>();

        let inbox_tx_for_writer = inbox_tx.clone();
        tokio::spawn(async move {
            while let Some(message) = outbox_rx.recv().await {
                if let Err(err) = writer.send(message).await {
                    let _ = inbox_tx_for_writer.send(Err(McpError::Transport(format!(
                        "websocket send failed: {err}"
                    ))));
                    break;
                }
            }
        });

        let inbox_tx_for_reader = inbox_tx.clone();
        let outbox_tx_for_reader = outbox_tx.clone();
        tokio::spawn(async move {
            while let Some(frame) = reader.next().await {
                match frame {
                    Ok(Message::Text(text)) => {
                        if forward_text_frame(text.as_str(), &inbox_tx_for_reader).is_err() {
                            break;
                        }
                    }
                    Ok(Message::Binary(bytes)) => match String::from_utf8(bytes.to_vec()) {
                        Ok(text) => {
                            if forward_text_frame(text.as_str(), &inbox_tx_for_reader).is_err() {
                                break;
                            }
                        }
                        Err(err) => {
                            let _ = inbox_tx_for_reader.send(Err(McpError::Malformed(format!(
                                "websocket binary frame is not utf-8: {err}"
                            ))));
                            break;
                        }
                    },
                    Ok(Message::Ping(payload)) => {
                        if outbox_tx_for_reader.send(Message::Pong(payload)).is_err() {
                            break;
                        }
                    }
                    Ok(Message::Pong(_)) | Ok(Message::Frame(_)) => {}
                    Ok(Message::Close(_)) => break,
                    Err(err) => {
                        let _ = inbox_tx_for_reader.send(Err(McpError::Transport(format!(
                            "websocket receive failed: {err}"
                        ))));
                        break;
                    }
                }
            }
            let _ = inbox_tx_for_reader.send(Err(McpError::TransportClosed));
        });

        Ok(Self {
            inner: Arc::new(Inner {
                outbox: Mutex::new(Some(outbox_tx)),
                inbox: Mutex::new(inbox_rx),
                inbox_tx,
            }),
        })
    }
}

fn handshake_request(url: &Url, headers: &HashMap<String, String>) -> McpResult<http::Request<()>> {
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|err| McpError::Transport(format!("invalid websocket url: {err}")))?;
    for (name, value) in headers {
        let header_name = HeaderName::from_bytes(name.as_bytes()).map_err(|err| {
            McpError::Transport(format!("invalid websocket header name '{name}': {err}"))
        })?;
        let header_value = HeaderValue::from_str(value).map_err(|err| {
            McpError::Transport(format!(
                "invalid websocket header value for '{name}': {err}"
            ))
        })?;
        request.headers_mut().insert(header_name, header_value);
    }
    Ok(request)
}

fn forward_text_frame(
    text: &str,
    inbox_tx: &mpsc::UnboundedSender<McpResult<InboundMessage>>,
) -> Result<(), ()> {
    let value: Value = match serde_json::from_str(text) {
        Ok(value) => value,
        Err(err) => {
            let _ = inbox_tx.send(Err(McpError::Malformed(format!(
                "invalid websocket JSON frame: {err}"
            ))));
            return Err(());
        }
    };
    let frame =
        InboundMessage::from_value(value).map_err(|err| McpError::Malformed(err.to_string()));
    inbox_tx.send(frame).map_err(|_| ())
}

#[async_trait]
impl McpTransport for WsTransport {
    async fn send(&self, payload: Value) -> McpResult<()> {
        let message = Message::Text(payload.to_string().into());
        let guard = self.inner.outbox.lock().await;
        let outbox = guard.as_ref().ok_or(McpError::TransportClosed)?;
        outbox.send(message).map_err(|_| McpError::TransportClosed)
    }

    async fn recv(&self) -> McpResult<InboundMessage> {
        let mut guard = self.inner.inbox.lock().await;
        guard.recv().await.unwrap_or(Err(McpError::TransportClosed))
    }

    async fn close(&self) -> McpResult<()> {
        let mut guard = self.inner.outbox.lock().await;
        if let Some(outbox) = guard.take() {
            let _ = outbox.send(Message::Close(None));
        }
        let _ = self.inner.inbox_tx.send(Err(McpError::TransportClosed));
        Ok(())
    }
}
