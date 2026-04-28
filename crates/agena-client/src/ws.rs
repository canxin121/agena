//! WebSocket subscription multiplexer.
//!
//! [`WsClient::connect`] opens a single connection; each
//! [`WsClient::subscribe`] returns a [`Subscription`] that delivers
//! [`agena_api::DomainEvent`]s. Many subscriptions share one socket.

use std::collections::HashMap;
use std::sync::Arc;

use agena_api::{
    DomainEvent,
    notifications::Notification,
    subscribe::{SubscribeRequest, SubscriptionId},
    ws::{ClientMessage, ServerMessage},
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::{connect_async, tungstenite::Message};

use crate::error::ClientError;

/// Item delivered to a subscriber.
#[derive(Debug, Clone)]
pub enum SubscriptionEvent {
    Event(DomainEvent),
    Lagged(u64),
}

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
                        Notification::Event { subscription, event } => {
                            (subscription, SubscriptionEvent::Event(*event))
                        }
                        Notification::Lagged { subscription, skipped } => {
                            (subscription, SubscriptionEvent::Lagged(skipped))
                        }
                        Notification::Resumed { .. } | Notification::SubscriptionClosed { .. } => {
                            continue
                        }
                    };
                    let guard = subs_for_reader.lock().await;
                    if let Some(tx) = guard.inner.get(&id) {
                        let _ = tx.send(item).await;
                    }
                }
            }
        });

        Ok(Self { out_tx, subscribers })
    }

    pub async fn subscribe(
        &self,
        request: SubscribeRequest,
    ) -> Result<Subscription, ClientError> {
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
