use crate::{Application, ApplicationError, LiveItem};
use agena_api::extended_sessions::*;
use serde::Serialize;
use serde_json::Value;
use tokio::sync::mpsc;

fn encode<T: Serialize>(data: T) -> Result<Value, ApplicationError> {
    serde_json::to_value(data).map_err(|error| ApplicationError::internal_error(&error))
}
async fn send<T: Serialize>(tx: &mpsc::Sender<(String, Value)>, event: &str, data: T) -> bool {
    match encode(data) {
        Ok(data) => tx.send((event.to_owned(), data)).await.is_ok(),
        Err(error) => {
            let _ = tx
                .send((
                    "error".to_owned(),
                    serde_json::json!({"failure":error.failure}),
                ))
                .await;
            false
        }
    }
}

impl Application {
    pub(crate) fn register_serialized_stream<T: Serialize + Send + 'static>(
        &self,
        event: &'static str,
        mut source: mpsc::Receiver<T>,
    ) -> StreamHandle {
        let (tx, rx) = mpsc::channel(8);
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = tx.closed() => break,
                    item = source.recv() => match item {
                        Some(item) => if !send(&tx, event, item).await { break; },
                        None => break,
                    }
                }
            }
        });
        self.operation_io.register_stream(rx)
    }

    pub(crate) async fn open_operation_stream(
        &self,
        source: StreamSource,
    ) -> Result<StreamHandle, ApplicationError> {
        let (tx, rx) = mpsc::channel(8);
        match source {
            StreamSource::Changes { scope, locale } => {
                let scope = match scope {
                    ChangeScope::Global => agena_api::Scope::Global,
                    ChangeScope::Workspace { workspace_id } => {
                        agena_api::Scope::Workspace { workspace_id }
                    }
                    ChangeScope::Session { session_id } => agena_api::Scope::Session { session_id },
                };
                let mut subscription = self.watch_changes(scope.clone())?;
                // Subscribe before snapshot, so reads cannot lose a racing mutation.
                let initial = if let agena_api::Scope::Session { session_id } = scope {
                    let mut snapshot = self.session_parts(session_id).await?;
                    crate::part_locale::localize_session_parts(&mut snapshot, locale.as_deref());
                    Some(snapshot)
                } else {
                    None
                };
                tokio::spawn(async move {
                    if let Some(snapshot) = initial {
                        if !send(&tx, "session_snapshot", snapshot).await {
                            return;
                        }
                    }
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            item = subscription.recv() => {
                                let sent = match item {
                                    Some(LiveItem::SessionChanged(mut change)) => {
                                        crate::part_locale::localize_session_change(&mut change, locale.as_deref());
                                        send(&tx, "session_change", change).await
                                    }
                                    Some(LiveItem::RuntimeSignal(signal)) => send(&tx, "runtime_signal", signal).await,
                                    Some(LiveItem::Lagged(skipped)) => send(&tx, "lagged", skipped).await,
                                    None => break,
                                };
                                if !sent { break; }
                            }
                        }
                    }
                });
            }
            StreamSource::Content(request) => {
                let watch = self
                    .watch_content(request.session_id, request.resource_id)
                    .await?;
                watch.read(request.after, 1).await?;
                let mut delivery =
                    watch.delivery(request.after, request.max_bytes.clamp(1, 64 * 1024));
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            page = delivery.next() => match page {
                                Ok(Some(page)) => if !send(&tx, "content", page).await { break; },
                                Ok(None) => break,
                                Err(error) => { send(&tx, "content_error", serde_json::json!({"failure":error.failure})).await; break; },
                            }
                        }
                    }
                });
            }
            StreamSource::Notifications { filter, since_ms } => {
                let mut notifications = self.watch_notifications(filter, since_ms).await?;
                tokio::spawn(async move {
                    loop {
                        tokio::select! {
                            _ = tx.closed() => break,
                            event = notifications.recv() => match event {
                                Some(event) => if !send(&tx, event.event_name(), event.payload()).await { break; },
                                None => break,
                            }
                        }
                    }
                });
            }
            StreamSource::InlineUi {
                session_id,
                reference,
            } => {
                let mut subscription = self.watch_inline_ui(session_id, &reference).await?;
                let backend_slot = self.session_api_backend.clone();
                tokio::spawn(async move {
                    let mut revision = 0;
                    let mut recovery = tokio::time::interval(std::time::Duration::from_secs(2));
                    loop {
                        let snapshot = async {
                            let backend = backend_slot.backend().ok_or_else(|| {
                                ApplicationError::service_unavailable("application has stopped")
                            })?;
                            let response = backend
                                .query(agena_api::queries::Query::ExtendedSession(
                                    SessionQuery::InlineUiSnapshot {
                                        session_id,
                                        reference: reference.clone(),
                                    },
                                ))
                                .await
                                .map_err(ApplicationError::from_failure)?;
                            serde_json::from_value::<agena_domain::InlineSnapshot>(
                                response.get("data").cloned().unwrap_or(Value::Null),
                            )
                            .map_err(|error| ApplicationError::internal_error(&error))
                        }
                        .await;
                        match snapshot {
                            Ok(snapshot) => {
                                if snapshot.revision != revision {
                                    revision = snapshot.revision;
                                    if !send(&tx, "inline_ui", snapshot).await {
                                        break;
                                    }
                                }
                            }
                            Err(error) => {
                                send(
                                    &tx,
                                    "inline_ui_error",
                                    serde_json::json!({"failure":error.failure}),
                                )
                                .await;
                                break;
                            }
                        }
                        tokio::select! {
                            _ = tx.closed() => break,
                            result = subscription.changed() => if result.is_err() { break; },
                            _ = recovery.tick() => {},
                        }
                    }
                });
            }
        }
        Ok(self.operation_io.register_stream(rx))
    }
}
