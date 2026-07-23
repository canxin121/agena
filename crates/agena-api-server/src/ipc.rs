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
    use agena_domain::EventFilter;
    use agena_runtime::{RuntimeEventStreamService, RuntimeLiveEventSubscriptionItem};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{UnixListener, UnixStream};
    use tokio::sync::{Mutex, mpsc};

    use crate::{
        error::ServerError,
        state::{AppState, event_filter_from_subscribe},
    };
    use agena_application::dispatch;

    /// Bind a Unix socket at `path` and serve the WS-equivalent protocol
    /// until the future is dropped.
    pub async fn serve(path: PathBuf, state: AppState) -> std::io::Result<()> {
        if path.exists() {
            std::fs::remove_file(&path)?;
        }
        let listener = UnixListener::bind(&path)?;
        loop {
            let (stream, _) = listener.accept().await?;
            let state_clone = state.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_connection(stream, state_clone).await {
                    tracing::warn!(?err, "ipc connection failed");
                }
            });
        }
    }

    async fn handle_connection(stream: UnixStream, state: AppState) -> std::io::Result<()> {
        let (read, mut write) = stream.into_split();
        let mut lines = BufReader::new(read).lines();
        let (tx, mut rx) = mpsc::channel::<ServerMessage>(256);

        let _ = tx
            .send(ServerMessage::Hello {
                protocol_version: PROTOCOL_VERSION,
            })
            .await;

        let writer = tokio::spawn(async move {
            while let Some(msg) = rx.recv().await {
                let payload = match serde_json::to_string(&msg) {
                    Ok(p) => p,
                    Err(_) => continue,
                };
                if write.write_all(payload.as_bytes()).await.is_err() {
                    break;
                }
                if write.write_all(b"\n").await.is_err() {
                    break;
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
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: None,
                            error: ApiError::protocol(format!("invalid frame: {err}")),
                        })
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
        let _ = writer.await;
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
                        let _ = tx.send(ServerMessage::CommandResult { id, result }).await;
                    }
                    Err(err) => {
                        let _ = tx
                            .send(ServerMessage::Error {
                                id: Some(id),
                                error: ServerError::from(err).into_api(),
                            })
                            .await;
                    }
                }
            }
            ClientMessage::Query { id, query } => {
                match dispatch::dispatch_query(&state, query).await {
                    Ok(result) => {
                        let _ = tx.send(ServerMessage::QueryResult { id, result }).await;
                    }
                    Err(err) => {
                        let _ = tx
                            .send(ServerMessage::Error {
                                id: Some(id),
                                error: ServerError::from(err).into_api(),
                            })
                            .await;
                    }
                }
            }
            ClientMessage::Subscribe { id, request } => {
                let stream_service = match state.event_stream_service() {
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
                spawn_subscription(
                    id,
                    event_filter_from_subscribe(request),
                    stream_service,
                    tx,
                    registry,
                )
                .await;
            }
            ClientMessage::Unsubscribe { id } => {
                let mut guard = registry.lock().await;
                if let Some(handle) = guard.remove(&id) {
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
        stream_service: Arc<dyn RuntimeEventStreamService>,
        tx: mpsc::Sender<ServerMessage>,
        registry: Arc<Mutex<HashMap<SubscriptionId, tokio::task::JoinHandle<()>>>>,
    ) {
        let mut subscription = stream_service.subscribe_events(filter);
        let id_for_task = id.clone();
        let tx_clone = tx.clone();
        let handle = tokio::spawn(async move {
            while let Some(item) = subscription.recv().await {
                let notification = match item {
                    RuntimeLiveEventSubscriptionItem::Event(event) => Notification::Event {
                        subscription: id_for_task.clone(),
                        event: Box::new(
                            agena_application::event_projection::event_resource_from_runtime(
                                &event,
                            ),
                        ),
                    },
                    RuntimeLiveEventSubscriptionItem::Lagged(skipped) => Notification::Lagged {
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
        if let Some(prev) = guard.insert(id.clone(), handle) {
            prev.abort();
        }
        drop(guard);
        let _ = tx.send(ServerMessage::Subscribed { id }).await;
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
