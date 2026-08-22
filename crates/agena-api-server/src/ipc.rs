//! Unix-socket IPC transport. Reuses the same JSON RPC protocol as
//! [`crate::ws`] but skips the HTTP upgrade — frames are line-delimited
//! JSON. Useful for local TUI / CLI clients that want zero-overhead access.
//!
//! Linux/macOS only; Windows builds expose a stub that returns
//! `Unsupported`.

#[cfg(unix)]
pub use unix::*;

#[cfg(not(unix))]
pub use stub::*;

#[cfg(unix)]
mod unix {
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::Arc;

    use agena_api::{
        PROTOCOL_VERSION,
        error::ApiError,
        notifications::Notification,
        subscribe::SubscriptionId,
        ws::{ClientMessage, ServerMessage},
    };
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::{Mutex, mpsc};

    use crate::dispatch;
    use crate::{
        error::ServerError,
        live::{self, LiveItem},
        state::AppState,
    };

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
                    "IPC server message could not be queued because the connection closed"
                );
                false
            }
        }
    }

    /// Bind a Unix socket at `path` and serve the WS-equivalent protocol
    /// until the future is dropped.
    pub async fn serve(path: PathBuf, state: AppState) -> std::io::Result<()> {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        let mut connections = tokio::task::JoinSet::new();
        loop {
            let (stream, _) = listener.accept().await?;
            let state_clone = state.clone();
            connections.spawn(async move {
                if let Err(err) = handle_connection(stream, state_clone).await {
                    tracing::warn!(?err, "ipc connection failed");
                }
            });
            while connections.try_join_next().is_some() {}
        }
    }

    async fn handle_connection(stream: UnixStream, state: AppState) -> std::io::Result<()> {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let (tx, mut rx) = mpsc::channel::<ServerMessage>(256);

        if tx
            .send(ServerMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
            })
            .await
            .is_err()
        {
            tracing::debug!("IPC connection closed before the protocol greeting could be queued");
            return Ok(());
        }

        let mut writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let payload = match serde_json::to_string(&msg) {
                    Ok(p) => p,
                    Err(error) => {
                        tracing::error!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                            "failed to serialize an IPC server message"
                        );
                        continue;
                    }
                };
                if let Err(error) = write.write_all(payload.as_bytes()).await {
                    tracing::debug!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                        "IPC writer stopped after the peer disconnected"
                    );
                    return;
                }
                if let Err(error) = write.write_all(b"\n").await {
                    tracing::debug!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                        "IPC writer could not terminate a frame after the peer disconnected"
                    );
                    return;
                }
            }
        });

        let registry: Arc<Mutex<HashMap<SubscriptionId, tokio::task::JoinHandle<()>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        while let Some(line) = lines.next_line().await? {
            if line.trim().is_empty() {
                continue;
            }
            let parsed: Result<ClientMessage, _> = serde_json::from_str(&line);
            match parsed {
                Ok(msg) => {
                    handle_client_message(msg, state.clone(), tx.clone(), Arc::clone(&registry))
                        .await;
                }
                Err(err) => {
                    let error = ApiError::protocol(format!("invalid frame: {err}"));
                    tracing::warn!(failure_id = %error.problem.id, diagnostic = %err, "invalid IPC protocol frame");
                    queue_server_message(
                        &tx,
                        ServerMessage::Error { id: None, error },
                        "deliver an invalid-frame IPC protocol error",
                    )
                    .await;
                }
            }
        }

        let mut guard = registry.lock().await;
        for (_, handle) in guard.drain() {
            handle.abort();
        }
        drop(guard);
        drop(tx);
        match tokio::time::timeout(std::time::Duration::from_secs(1), &mut writer).await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                "IPC writer task failed"
            ),
            Err(timeout_error) => {
                tracing::warn!(
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "IPC writer did not stop within the 1-second shutdown window",
                        &timeout_error,
                    ),
                    "aborting IPC writer after shutdown timeout"
                );
                writer.abort();
                if let Err(error) = writer.await
                    && !error.is_cancelled()
                {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                        "IPC writer task failed while being aborted after its shutdown timeout"
                    );
                }
            }
        }
        Ok(())
    }

    async fn handle_client_message(
        msg: ClientMessage,
        state: AppState,
        tx: mpsc::Sender<ServerMessage>,
        registry: Arc<Mutex<HashMap<SubscriptionId, tokio::task::JoinHandle<()>>>>,
    ) {
        match msg {
            ClientMessage::Command { id, command } => {
                match dispatch::dispatch_command(&state, command).await {
                    Ok(result) => {
                        queue_server_message(
                            &tx,
                            ServerMessage::CommandResult { id, result },
                            "deliver an IPC command result",
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
                            "deliver an IPC command failure",
                        )
                        .await;
                    }
                }
            }
            ClientMessage::Query { id, query } => {
                match dispatch::dispatch_query(&state, query).await {
                    Ok(result) => {
                        queue_server_message(
                            &tx,
                            ServerMessage::QueryResult { id, result },
                            "deliver an IPC query result",
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
                            "deliver an IPC query failure",
                        )
                        .await;
                    }
                }
            }
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
                            "deliver an IPC subscription setup failure",
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
                            "deliver an IPC subscription storage failure",
                        )
                        .await;
                        return;
                    }
                };
                spawn_subscription(id, request.scope, subscription, store, tx, registry).await;
            }
            ClientMessage::Unsubscribe { id } => {
                let mut guard = registry.lock().await;
                if let Some(handle) = guard.remove(&id) {
                    handle.abort();
                }
                drop(guard);
                queue_server_message(
                    &tx,
                    ServerMessage::Unsubscribed { id },
                    "deliver an IPC unsubscribe acknowledgement",
                )
                .await;
            }
            ClientMessage::Ping { nonce } => {
                queue_server_message(&tx, ServerMessage::Pong { nonce }, "deliver an IPC pong")
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
        registry: Arc<Mutex<HashMap<SubscriptionId, tokio::task::JoinHandle<()>>>>,
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
                        "IPC subscription notification delivery stopped because the connection closed"
                    );
                    break;
                }
            }
        });

        let mut guard = registry.lock().await;
        if let Some(prev) = guard.insert(id.clone(), handle) {
            prev.abort();
        }
        drop(guard);
        queue_server_message(
            &tx,
            ServerMessage::Subscribed { id },
            "deliver an IPC subscription acknowledgement",
        )
        .await;
    }
}

#[cfg(not(unix))]
mod stub {
    use std::path::PathBuf;

    use crate::state::AppState;

    pub async fn serve(_: PathBuf, _: AppState) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Unix socket IPC is only supported on Unix targets",
        ))
    }
}
