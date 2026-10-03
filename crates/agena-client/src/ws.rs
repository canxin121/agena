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
        let id: SubscriptionId = uuid::Uuid::new_v4().simple().to_string().into();
        let (events, rx) = broadcast::channel(256);
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
                .send(ClientMessage::Subscribe { id, request })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn subscription_waits_for_ack_reports_overflow_and_terminates_on_socket_close() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let (received, received_rx) = oneshot::channel();
        let (release, released) = oneshot::channel();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let message = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let ClientMessage::Subscribe { id, .. } = serde_json::from_str(&message).unwrap()
            else {
                panic!("subscribe")
            };
            let _ = received.send(());
            released.await.unwrap();
            socket
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Subscribed { id: id.clone() })
                        .unwrap()
                        .into(),
                ))
                .await
                .unwrap();
            for n in 0..600 {
                let message = ServerMessage::Notification(Notification::RuntimeSignal {
                    subscription: id.clone(),
                    signal: Box::new(RuntimeSignalResource {
                        kind: "test".into(),
                        session_id: None,
                        payload: serde_json::json!(n),
                    }),
                });
                socket
                    .send(Message::Text(
                        serde_json::to_string(&message).unwrap().into(),
                    ))
                    .await
                    .unwrap();
            }
            socket.close(None).await.unwrap();
        });
        let client = WsClient::connect(format!("ws://{address}")).await.unwrap();
        let subscribing = tokio::spawn({
            let client = client.clone();
            async move {
                client
                    .subscribe(SubscribeRequest {
                        scope: agena_api::Scope::Global,
                    })
                    .await
            }
        });
        received_rx.await.unwrap();
        assert!(
            !subscribing.is_finished(),
            "the snapshot may start only after acknowledgement"
        );
        release.send(()).unwrap();
        let mut subscription = subscribing.await.unwrap().unwrap();
        server.await.unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if client.subscribers.lock().unwrap().closed {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .unwrap();
        assert!(
            matches!(subscription.recv().await, Some(SubscriptionEvent::Lagged(skipped)) if skipped > 0)
        );
        let mut remaining = 0;
        while subscription.recv().await.is_some() {
            remaining += 1;
        }
        assert_eq!(remaining, 256);
    }

    #[tokio::test]
    async fn malformed_frames_close_the_subscription_instead_of_silently_losing_an_event() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_async(socket).await.unwrap();
            let frame = socket.next().await.unwrap().unwrap().into_text().unwrap();
            let ClientMessage::Subscribe { id, .. } = serde_json::from_str(&frame).unwrap() else {
                panic!("subscribe")
            };
            socket
                .send(Message::Text(
                    serde_json::to_string(&ServerMessage::Subscribed { id })
                        .unwrap()
                        .into(),
                ))
                .await
                .unwrap();
            socket.send(Message::Text("{broken".into())).await.unwrap();
            while socket.next().await.is_some() {}
        });
        let client = WsClient::connect(format!("ws://{address}")).await.unwrap();
        let mut subscription = client
            .subscribe(SubscribeRequest {
                scope: agena_api::Scope::Global,
            })
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_secs(3), subscription.recv())
                .await
                .unwrap()
                .is_none()
        );
        assert!(client.subscribers.lock().unwrap().closed);
        server.abort();
    }

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
