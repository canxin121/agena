//! WebSocket subscription multiplexer.
//!
//! [`WsClient::connect`] opens a single connection; each
//! [`WsClient::subscribe`] returns a [`Subscription`] that delivers part
//! patches and ephemeral runtime signals. Many subscriptions share one socket.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use agena_api::{
    live::{RuntimeSignalResource, SessionChangeResource},
    notifications::Notification,
    subscribe::{SubscribeRequest, SubscriptionId},
    ws::{ClientMessage, ServerMessage},
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use tokio_util::sync::CancellationToken;

use crate::error::ClientError;

/// Item delivered to a subscriber.
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone)]
/// Item received on a websocket live subscription.
pub enum SubscriptionEvent {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Content(agena_domain::ContentPage),
    Lagged(u64),
}

/// Handle to an active websocket subscription.
pub struct Subscription {
    id: SubscriptionId,
    rx: broadcast::Receiver<SubscriptionEvent>,
    subscribers: Arc<Mutex<Subscribers>>,
    out_tx: mpsc::Sender<ClientMessage>,
}

impl Subscription {
    pub fn id(&self) -> &SubscriptionId {
        &self.id
    }

    pub async fn recv(&mut self) -> Option<SubscriptionEvent> {
        match self.rx.recv().await {
            Ok(event) => Some(event),
            Err(broadcast::error::RecvError::Lagged(skipped)) => {
                Some(SubscriptionEvent::Lagged(skipped))
            }
            Err(broadcast::error::RecvError::Closed) => None,
        }
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.subscribers
            .lock()
            .expect("subscription lock")
            .inner
            .remove(&self.id);
        let _ = self.out_tx.try_send(ClientMessage::Unsubscribe {
            id: self.id.clone(),
        });
    }
}

struct Subscriber {
    events: broadcast::Sender<SubscriptionEvent>,
    ready: Option<oneshot::Sender<Result<(), String>>>,
}

#[derive(Default)]
struct Subscribers {
    inner: HashMap<SubscriptionId, Subscriber>,
    closed: bool,
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

        let cancellation = CancellationToken::new();
        // Reader and writer share one lifecycle: either transport failure
        // closes every subscriber, even when the other half is idle.
        let writer_cancel = cancellation.clone();
        let subs_for_writer = Arc::clone(&subscribers);
        tokio::spawn(async move {
            while let Some(msg) = tokio::select! {
                _ = writer_cancel.cancelled() => None,
                message = out_rx.recv() => message,
            } {
                let payload = match serde_json::to_string(&msg) {
                    Ok(p) => p,
                    Err(error) => {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to serialize an Agena WebSocket client message",
                                &error,
                            ),
                            "Agena WebSocket client message was not sent"
                        );
                        continue;
                    }
                };
                let result = tokio::select! {
                    _ = writer_cancel.cancelled() => break,
                    result = tokio::time::timeout(std::time::Duration::from_secs(10), sink.send(Message::Text(payload.into()))) => result,
                };
                if !matches!(result, Ok(Ok(()))) {
                    let error = std::io::Error::other(format!("WebSocket send failed: {result:?}"));
                    tracing::warn!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to send an Agena WebSocket client message",
                            &error,
                        ),
                        "Agena WebSocket writer stopped"
                    );
                    break;
                }
            }
            writer_cancel.cancel();
            let mut guard = subs_for_writer.lock().expect("subscription lock");
            guard.closed = true;
            guard.inner.clear();
        });

        // Reader
        let subs_for_reader = Arc::clone(&subscribers);
        tokio::spawn(async move {
            while let Some(message) = tokio::select! {
                _ = cancellation.cancelled() => None,
                message = stream.next() => message,
            } {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to receive an Agena WebSocket server message",
                                &error,
                            ),
                            "Agena WebSocket reader stopped"
                        );
                        break;
                    }
                };
                let text = match message {
                    Message::Text(t) => t,
                    Message::Binary(_) => continue,
                    Message::Close(_) => break,
                    _ => continue,
                };
                let server_msg: ServerMessage = match serde_json::from_str(&text) {
                    Ok(message) => message,
                    Err(error) => {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "failed to decode an Agena WebSocket server message",
                                &error,
                            ),
                            "invalid Agena WebSocket server message; reconnect required"
                        );
                        break;
                    }
                };
                match &server_msg {
                    ServerMessage::Subscribed { id } => {
                        if let Some(subscriber) = subs_for_reader
                            .lock()
                            .expect("subscription lock")
                            .inner
                            .get_mut(id)
                            && let Some(ready) = subscriber.ready.take()
                        {
                            let _ = ready.send(Ok(()));
                        }
                    }
                    ServerMessage::Error { id: Some(id), .. } => {
                        if let Some(mut subscriber) = subs_for_reader
                            .lock()
                            .expect("subscription lock")
                            .inner
                            .remove(id)
                            && let Some(ready) = subscriber.ready.take()
                        {
                            let _ = ready.send(Err("WebSocket subscription was rejected".into()));
                        }
                    }
                    _ => {}
                }
                if let ServerMessage::Content { subscription, page } = server_msg {
                    let guard = subs_for_reader.lock().expect("subscription lock");
                    if let Some(subscriber) = guard.inner.get(&subscription) {
                        let _ = subscriber.events.send(SubscriptionEvent::Content(page));
                    }
                } else if let ServerMessage::Notification(notification) = server_msg {
                    let (id, item) = match notification {
                        Notification::SessionChanged {
                            subscription,
                            change,
                            ..
                        } => (subscription, SubscriptionEvent::SessionChanged(*change)),
                        Notification::RuntimeSignal {
                            subscription,
                            signal,
                            ..
                        } => (subscription, SubscriptionEvent::RuntimeSignal(*signal)),
                        Notification::Lagged {
                            subscription,
                            skipped,
                        } => (subscription, SubscriptionEvent::Lagged(skipped)),
                        Notification::SubscriptionClosed { subscription, .. } => {
                            subs_for_reader
                                .lock()
                                .expect("subscription lock")
                                .inner
                                .remove(&subscription);
                            continue;
                        }
                    };
                    let guard = subs_for_reader.lock().expect("subscription lock");
                    if let Some(subscriber) = guard.inner.get(&id) {
                        let _ = subscriber.events.send(item);
                    }
                }
            }
            cancellation.cancel();
            let mut guard = subs_for_reader.lock().expect("subscription lock");
            guard.closed = true;
            guard.inner.clear();
        });

        Ok(Self {
            out_tx,
            subscribers,
        })
    }

    pub async fn subscribe(&self, request: SubscribeRequest) -> Result<Subscription, ClientError> {
        self.open_subscription(|id| ClientMessage::Subscribe { id, request }, 256)
            .await
    }

    pub async fn watch_content(
        &self,
        request: agena_api::content::ReadContentParams,
    ) -> Result<Subscription, ClientError> {
        self.open_subscription(|id| ClientMessage::WatchContent { id, request }, 8)
            .await
    }

    async fn open_subscription(
        &self,
        message: impl FnOnce(SubscriptionId) -> ClientMessage,
        capacity: usize,
    ) -> Result<Subscription, ClientError> {
        let id: SubscriptionId = uuid::Uuid::new_v4().simple().to_string().into();
        let (events, rx) = broadcast::channel(capacity);
        let (ready, acknowledgement) = oneshot::channel();
        {
            let mut guard = self.subscribers.lock().expect("subscription lock");
            if guard.closed {
                return Err(ClientError::Transport("ws reader closed".into()));
            }
            guard.inner.insert(
                id.clone(),
                Subscriber {
                    events,
                    ready: Some(ready),
                },
            );
        }
        let subscription = Subscription {
            id: id.clone(),
            rx,
            subscribers: self.subscribers.clone(),
            out_tx: self.out_tx.clone(),
        };
        let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
            self.out_tx
                .send(message(id))
                .await
                .map_err(|_| ClientError::Transport("ws writer dropped".into()))?;
            acknowledgement
                .await
                .map_err(|_| {
                    ClientError::Transport("ws reader closed before acknowledgement".into())
                })?
                .map_err(ClientError::Transport)
        })
        .await;
        match result {
            Ok(Ok(())) => Ok(subscription),
            Ok(Err(error)) => Err(error),
            Err(_) => Err(ClientError::Transport(
                "ws subscription acknowledgement timed out".into(),
            )),
        }
    }

    pub async fn unsubscribe(&self, id: SubscriptionId) -> Result<(), ClientError> {
        {
            let mut guard = self.subscribers.lock().expect("subscription lock");
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
