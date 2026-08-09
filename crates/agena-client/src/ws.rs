//! WebSocket subscription multiplexer.
//!
//! [`WsClient::connect`] opens a single connection; each
//! [`WsClient::subscribe`] returns a [`Subscription`] that delivers v2 part
//! patches and ephemeral runtime signals. Many subscriptions share one socket.

use std::collections::HashMap;
use std::sync::Arc;

use agena_api::{
    live::{RuntimeSignalResource, SessionChangeResource},
    notifications::Notification,
    subscribe::{SubscribeRequest, SubscriptionId},
    ws::{ClientMessage, ServerMessage},
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::error::ClientError;

/// Item delivered to a subscriber.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
/// Item received on a websocket live subscription.
pub enum SubscriptionEvent {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Lagged(u64),
}

/// Handle to an active websocket subscription.
pub struct Subscription {
    id: SubscriptionId,
    rx: mpsc::Receiver<SubscriptionEvent>,
}

impl Subscription {
    pub fn id(&self) -> &SubscriptionId {
        &self.id
    }

    pub async fn recv(&mut self) -> Option<SubscriptionEvent> {
        self.rx.recv().await
    }
}

#[derive(Default)]
struct Subscribers {
    inner: HashMap<SubscriptionId, mpsc::Sender<SubscriptionEvent>>,
}

#[derive(Clone)]
/// Websocket client for Agena's live part-patch/signal stream.
pub struct WsClient {
    out_tx: mpsc::Sender<ClientMessage>,
    subscribers: Arc<Mutex<Subscribers>>,
}

impl WsClient {
    /// Connect to `ws://host:port/api/v1/ws` and spawn the read/write tasks.
    pub async fn connect(url: impl AsRef<str>) -> Result<Self, ClientError> {
        let (ws_stream, _) = connect_async(url.as_ref()).await?;
        let (mut sink, mut stream) = ws_stream.split();

        let (out_tx, mut out_rx) = mpsc::channel::<ClientMessage>(256);
        let subscribers = Arc::new(Mutex::new(Subscribers::default()));

        // Writer
        tokio::spawn(async move {
            while let Some(msg) = out_rx.recv().await {
                let payload = match serde_json::to_string(&msg) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if sink.send(Message::Text(payload.into())).await.is_err() {
                    break;
                }
            }
        });

        // Reader
        let subs_for_reader = Arc::clone(&subscribers);
        tokio::spawn(async move {
            while let Some(message) = stream.next().await {
                let Ok(message) = message else { break };
                let text = match message {
                    Message::Text(t) => t,
                    Message::Binary(_) => continue,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let parsed: Result<ServerMessage, _> = serde_json::from_str(&text);
                let Ok(server_msg) = parsed else { continue };
                if let ServerMessage::Notification(notification) = server_msg {
                    let (id, item) = match notification {
                        Notification::SessionChanged {
                            subscription,
                            change,
                        } => (subscription, SubscriptionEvent::SessionChanged(*change)),
                        Notification::RuntimeSignal {
                            subscription,
                            signal,
                        } => (subscription, SubscriptionEvent::RuntimeSignal(*signal)),
                        Notification::Lagged {
                            subscription,
                            skipped,
                        } => (subscription, SubscriptionEvent::Lagged(skipped)),
                        Notification::SubscriptionClosed { .. } => continue,
                    };
                    let guard = subs_for_reader.lock().await;
                    if let Some(tx) = guard.inner.get(&id) {
                        let _ = tx.send(item).await;
                    }
                }
            }
        });

        Ok(Self {
            out_tx,
            subscribers,
        })
    }

    pub async fn subscribe(&self, request: SubscribeRequest) -> Result<Subscription, ClientError> {
        let id: SubscriptionId = uuid::Uuid::new_v4().simple().to_string().into();
        let (tx, rx) = mpsc::channel(256);
        {
            let mut guard = self.subscribers.lock().await;
            guard.inner.insert(id.clone(), tx);
        }
        self.out_tx
            .send(ClientMessage::Subscribe {
                id: id.clone(),
                request,
            })
            .await
            .map_err(|_| ClientError::Transport("ws writer dropped".into()))?;
        Ok(Subscription { id, rx })
    }

    pub async fn unsubscribe(&self, id: SubscriptionId) -> Result<(), ClientError> {
        {
            let mut guard = self.subscribers.lock().await;
            guard.inner.remove(&id);
        }
        self.out_tx
            .send(ClientMessage::Unsubscribe { id })
            .await
            .map_err(|_| ClientError::Transport("ws writer dropped".into()))?;
        Ok(())
    }

    pub async fn ping(&self) -> Result<(), ClientError> {
        self.out_tx
            .send(ClientMessage::Ping { nonce: None })
            .await
            .map_err(|_| ClientError::Transport("ws writer dropped".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("checked-in client fixture must be readable")
    }

    #[test]
    fn checked_in_server_frames_decode_through_shared_protocol() {
        let hello: ServerMessage = serde_json::from_str(&fixture("ws-hello.json"))
            .expect("decode websocket hello fixture");
        assert!(matches!(
            hello,
            ServerMessage::Hello {
                protocol_version: 2
            }
        ));

        let pong: ServerMessage =
            serde_json::from_str(&fixture("ws-pong.json")).expect("decode websocket pong fixture");
        assert!(matches!(
            pong,
            ServerMessage::Pong { nonce: Some(nonce) } if nonce == "contract-ping"
        ));

        let error: ServerMessage = serde_json::from_str(&fixture("ws-error.json"))
            .expect("decode websocket error fixture");
        assert!(matches!(
            error,
            ServerMessage::Error { id: Some(id), error }
                if id == "missing-workspace"
                    && error.problem.category == agena_failure::FailureCategory::NotFound
        ));
    }

    #[test]
    fn ping_frame_uses_shared_wire_shape() {
        let message = ClientMessage::Ping {
            nonce: Some("contract-ping".into()),
        };
        let json = serde_json::to_value(message).expect("encode websocket ping");
        assert_eq!(json["type"], "ping");
        assert_eq!(json["nonce"], "contract-ping");
    }
}
