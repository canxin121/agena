//! WebSocket transport implementing the [`agena_api::ws`] duplex protocol.
//!
//! One connection = one writer task + one reader task. Subscriptions are
//! managed by a [`SubscriptionRegistry`]: each subscribe spawns a forwarder
//! task that pulls from the bus and pushes serialized notifications onto a
//! shared `mpsc` channel feeding the writer.

use std::collections::HashMap;
use std::sync::Arc;

use agena::event::EventKind;
use agena::event::{EventBus, EventFilter, bus::SubscriptionItem};
use agena_api::{
    PROTOCOL_VERSION,
    error::ApiError,
    notifications::Notification,
    subscribe::SubscriptionId,
    ws::{ClientMessage, ServerMessage},
};
use axum::{
    extract::{State, WebSocketUpgrade, ws::Message, ws::WebSocket},
    response::Response,
};
use futures_util::{SinkExt, StreamExt};
use tokio::sync::{Mutex, mpsc};

use crate::{dispatch, state::AppState};

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn run(socket: WebSocket, state: AppState) {
    let (sink, mut stream) = socket.split();
    let sink = Arc::new(Mutex::new(sink));
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(256);

    // Greet the client with the protocol version.
    let _ = tx
        .send(ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        })
        .await;

    let writer_sink = Arc::clone(&sink);
    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            let payload = match serde_json::to_string(&msg) {
                Ok(p) => p,
                Err(err) => {
                    tracing::warn!(?err, "failed to serialize server message");
                    continue;
                }
            };
            let mut guard = writer_sink.lock().await;
            if guard.send(Message::Text(payload.into())).await.is_err() {
                break;
            }
        }
    });

    let registry = SubscriptionRegistry::default();
    let registry = Arc::new(Mutex::new(registry));

    while let Some(message) = stream.next().await {
        let Ok(message) = message else { break };
        match message {
            Message::Text(text) => {
                let parsed: Result<ClientMessage, _> = serde_json::from_str(&text);
                match parsed {
                    Ok(client_msg) => {
                        handle_client_message(
                            client_msg,
                            state.clone(),
                            tx.clone(),
                            Arc::clone(&registry),
                        )
                        .await;
                    }
                    Err(err) => {
                        let _ = tx
                            .send(ServerMessage::Error {
                                id: None,
                                error: ApiError::protocol(format!("invalid frame: {err}")),
                            })
                            .await;
                    }
                }
            }
            Message::Binary(_) => {
                let _ = tx
                    .send(ServerMessage::Error {
                        id: None,
                        error: ApiError::protocol("binary frames are not supported"),
                    })
                    .await;
            }
            Message::Ping(_) | Message::Pong(_) => {}
            Message::Close(_) => break,
        }
    }

    // Drain subscriptions on close.
    let mut guard = registry.lock().await;
    for (_, handle) in guard.0.drain() {
        handle.abort();
    }
    drop(guard);
    drop(tx);
    let _ = writer.await;
}

#[derive(Default)]
struct SubscriptionRegistry(HashMap<SubscriptionId, tokio::task::JoinHandle<()>>);

async fn handle_client_message(
    msg: ClientMessage,
    state: AppState,
    tx: mpsc::Sender<ServerMessage>,
    registry: Arc<Mutex<SubscriptionRegistry>>,
) {
    match msg {
        ClientMessage::Command { id, command } => {
            match dispatch::dispatch_command(&state, command).await {
                Ok(result) => {
                    let _ = tx.send(ServerMessage::CommandResult { id, result }).await;
                }
                Err(err) => {
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: Some(id),
                            error: err.into_api(),
                        })
                        .await;
                }
            }
        }
        ClientMessage::Query { id, query } => match dispatch::dispatch_query(&state, query).await {
            Ok(result) => {
                let _ = tx.send(ServerMessage::QueryResult { id, result }).await;
            }
            Err(err) => {
                let _ = tx
                    .send(ServerMessage::Error {
                        id: Some(id),
                        error: err.into_api(),
                    })
                    .await;
            }
        },
        ClientMessage::Subscribe { id, request } => {
            let bus = match state.event_bus() {
                Ok(b) => b,
                Err(err) => {
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: Some(id),
                            error: err.into_api(),
                        })
                        .await;
                    return;
                }
            };
            let filter = request.into_filter();
            spawn_subscription(id, filter, bus, tx, registry).await;
        }
        ClientMessage::Unsubscribe { id } => {
            let mut guard = registry.lock().await;
            if let Some(handle) = guard.0.remove(&id) {
                handle.abort();
            }
            drop(guard);
            let _ = tx.send(ServerMessage::Unsubscribed { id }).await;
        }
        ClientMessage::Ping { nonce } => {
            let _ = tx.send(ServerMessage::Pong { nonce }).await;
        }
    }
}

async fn spawn_subscription(
    id: SubscriptionId,
    filter: EventFilter,
    bus: Arc<dyn EventBus<EventKind>>,
    tx: mpsc::Sender<ServerMessage>,
    registry: Arc<Mutex<SubscriptionRegistry>>,
) {
    let mut subscription = bus.subscribe(filter);
    let id_for_task = id.clone();
    let tx_clone = tx.clone();
    let handle = tokio::spawn(async move {
        while let Some(item) = subscription.recv().await {
            let notification = match item {
                SubscriptionItem::Event(event) => Notification::Event {
                    subscription: id_for_task.clone(),
                    event: Box::new((*event).clone()),
                },
                SubscriptionItem::Lagged(skipped) => Notification::Lagged {
                    subscription: id_for_task.clone(),
                    skipped,
                },
            };
            if tx_clone
                .send(ServerMessage::Notification(notification))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let mut guard = registry.lock().await;
    if let Some(prev) = guard.0.insert(id.clone(), handle) {
        prev.abort();
    }
    drop(guard);
    let _ = tx.send(ServerMessage::Subscribed { id }).await;
}
