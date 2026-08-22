//! WebSocket transport implementing the [`agena_api::ws`] duplex protocol.
//!
//! One connection = one writer task + one reader task. Subscriptions are
//! managed by a [`SubscriptionRegistry`]: each subscribe spawns a forwarder
//! task that pulls from the bus and pushes serialized notifications onto a
//! shared `mpsc` channel feeding the writer.

use std::collections::HashMap;
use std::sync::Arc;

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

use crate::dispatch;
use crate::{
    error::ServerError,
    live::{self, LiveItem},
    state::AppState,
};

pub async fn handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| run(socket, state))
}

async fn queue_server_message(
    tx: &mpsc::Sender<ServerMessage>,
    message: ServerMessage,
    context: &'static str,
) -> bool {
    match tx.send(message).await {
        Ok(()) => true,
        Err(error) => {
            tracing::debug!(
                operation = context,
                diagnostic = %error,
                "WebSocket server message could not be queued because the connection closed"
            );
            false
        }
    }
}

async fn run(socket: WebSocket, state: AppState) {
    let (sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::channel::<ServerMessage>(256);

    // Greet the client with the protocol version.
    if tx
        .send(ServerMessage::Hello {
            protocol_version: PROTOCOL_VERSION,
        })
        .await
        .is_err()
    {
        tracing::debug!("WebSocket closed before the protocol greeting could be queued");
        return;
    }

    let mut writer = tokio::spawn(async move {
        let mut sink = sink;
        while let Some(msg) = rx.recv().await {
            let payload = match serde_json::to_string(&msg) {
                Ok(p) => p,
                Err(err) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&err),
                        "failed to serialize a WebSocket server message"
                    );
                    continue;
                }
            };
            if let Err(error) = sink.send(Message::Text(payload.into())).await {
                tracing::debug!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "WebSocket writer stopped after the peer disconnected"
                );
                return;
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
                        let error = ApiError::protocol(format!("invalid frame: {err}"));
                        tracing::warn!(failure_id = %error.problem.id, diagnostic = %err, "invalid WebSocket protocol frame");
                        queue_server_message(
                            &tx,
                            ServerMessage::Error { id: None, error },
                            "deliver an invalid-frame protocol error",
                        )
                        .await;
                    }
                }
            }
            Message::Binary(_) => {
                queue_server_message(
                    &tx,
                    ServerMessage::Error {
                        id: None,
                        error: ApiError::protocol("binary frames are not supported"),
                    },
                    "deliver a binary-frame protocol error",
                )
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
    match tokio::time::timeout(std::time::Duration::from_secs(1), &mut writer).await {
        Ok(Ok(())) => {}
        Ok(Err(error)) => tracing::error!(
            diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
            "WebSocket writer task failed"
        ),
        Err(timeout_error) => {
            tracing::warn!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "WebSocket writer did not stop within the 1-second shutdown window",
                    &timeout_error,
                ),
                "aborting WebSocket writer after shutdown timeout"
            );
            writer.abort();
            if let Err(error) = writer.await
                && !error.is_cancelled()
            {
                tracing::error!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                    "WebSocket writer task failed while being aborted after its shutdown timeout"
                );
            }
        }
    }
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
                    queue_server_message(
                        &tx,
                        ServerMessage::CommandResult { id, result },
                        "deliver a command result",
                    )
                    .await;
                }
                Err(err) => {
                    queue_server_message(
                        &tx,
                        ServerMessage::Error {
                            id: Some(id),
                            error: ServerError::from(err).into_api(),
                        },
                        "deliver a command failure",
                    )
                    .await;
                }
            }
        }
        ClientMessage::Query { id, query } => match dispatch::dispatch_query(&state, query).await {
            Ok(result) => {
                queue_server_message(
                    &tx,
                    ServerMessage::QueryResult { id, result },
                    "deliver a query result",
                )
                .await;
            }
            Err(err) => {
                queue_server_message(
                    &tx,
                    ServerMessage::Error {
                        id: Some(id),
                        error: ServerError::from(err).into_api(),
                    },
                    "deliver a query failure",
                )
                .await;
            }
        },
        ClientMessage::Subscribe { id, request } => {
            let subscription = match live::subscribe(&state) {
                Ok(subscription) => subscription,
                Err(err) => {
                    queue_server_message(
                        &tx,
                        ServerMessage::Error {
                            id: Some(id),
                            error: err.into_api(),
                        },
                        "deliver a subscription setup failure",
                    )
                    .await;
                    return;
                }
            };
            let store = match state.session_store() {
                Ok(store) => store,
                Err(err) => {
                    queue_server_message(
                        &tx,
                        ServerMessage::Error {
                            id: Some(id),
                            error: err.into_api(),
                        },
                        "deliver a subscription storage failure",
                    )
                    .await;
                    return;
                }
            };
            spawn_subscription(id, request.scope, subscription, store, tx, registry).await;
        }
        ClientMessage::Unsubscribe { id } => {
            let mut guard = registry.lock().await;
            if let Some(handle) = guard.0.remove(&id) {
                handle.abort();
            }
            drop(guard);
            queue_server_message(
                &tx,
                ServerMessage::Unsubscribed { id },
                "deliver an unsubscribe acknowledgement",
            )
            .await;
        }
        ClientMessage::Ping { nonce } => {
            queue_server_message(
                &tx,
                ServerMessage::Pong { nonce },
                "deliver a WebSocket pong",
            )
            .await;
        }
    }
}

async fn spawn_subscription(
    id: SubscriptionId,
    scope: agena_api::Scope,
    mut subscription: live::LiveSubscription,
    store: Arc<dyn agena_storage::store::SessionStore>,
    tx: mpsc::Sender<ServerMessage>,
    registry: Arc<Mutex<SubscriptionRegistry>>,
) {
    let id_for_task = id.clone();
    let tx_clone = tx.clone();
    let handle = tokio::spawn(async move {
        while let Some(item) = subscription.recv().await {
            if !live::matches_scope(&item, &scope, store.as_ref()).await {
                continue;
            }
            let notification = match item {
                LiveItem::SessionChanged(change) => Notification::SessionChanged {
                    subscription: id_for_task.clone(),
                    change: Box::new(change),
                },
                LiveItem::RuntimeSignal(signal) => Notification::RuntimeSignal {
                    subscription: id_for_task.clone(),
                    signal: Box::new(signal),
                },
                LiveItem::Lagged(skipped) => Notification::Lagged {
                    subscription: id_for_task.clone(),
                    skipped,
                },
            };
            if let Err(error) = tx_clone
                .send(ServerMessage::Notification(notification))
                .await
            {
                tracing::debug!(
                    subscription_id = %id_for_task,
                    diagnostic = %error,
                    "WebSocket subscription notification delivery stopped because the connection closed"
                );
                break;
            }
        }
    });

    let mut guard = registry.lock().await;
    if let Some(prev) = guard.0.insert(id.clone(), handle) {
        prev.abort();
    }
    drop(guard);
    queue_server_message(
        &tx,
        ServerMessage::Subscribed { id },
        "deliver a subscription acknowledgement",
    )
    .await;
}
