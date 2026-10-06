//! Server-Sent Events transport for best-effort part patches and ephemeral
//! runtime signals. Persisted catch-up is a separate ordered-parts read.

use std::convert::Infallible;
use std::time::Duration;

use agena_api::notifications::Notification;
use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::{Stream, StreamExt as _};
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{
    error::ServerError,
    live::{self, LiveItem},
    state::AppState,
};

#[derive(Debug, Deserialize, Default)]
/// Query for an SSE live-change stream.
pub struct StreamQuery {
    /// Filter by scope (`global` / `workspace` / `session`). Defaults to
    /// global. Encoded as a JSON-flattened scope object via `scope_kind` /
    /// `workspace_id` / `session_id` query params.
    #[serde(default)]
    pub scope_kind: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub session_id: Option<i64>,
}

pub async fn handler(
    State(state): State<AppState>,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ServerError> {
    let scope = query.into_scope()?;
    let store = state.session_store()?;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    let subscription_id: smol_str::SmolStr = "sse".into();
    let mut subscription = live::subscribe(&state)?;
    let _ = tx.try_send(Ok(Event::default().comment("subscribed")));

    let producer = tokio::spawn(async move {
        while let Some(item) = tokio::select! {
            _ = tx.closed() => None,
            item = subscription.recv() => item,
        } {
            tokio::task::consume_budget().await;
            if !live::matches_scope(&item, &scope, store.as_ref()).await {
                continue;
            }
            let revisions = subscription.revisions_for(&item);
            let notification = match item {
                LiveItem::SessionChanged(change) => Notification::SessionChanged {
                    subscription: subscription_id.clone(),
                    change: Box::new(change),
                    revisions,
                },
                LiveItem::RuntimeSignal(signal) => Notification::RuntimeSignal {
                    subscription: subscription_id.clone(),
                    signal: Box::new(signal),
                    revisions,
                },
                LiveItem::Lagged(skipped) => Notification::Lagged {
                    subscription: subscription_id.clone(),
                    skipped,
                },
            };
            let payload = match crate::json_codec::encode(notification).await {
                Ok(p) => p,
                Err(error) => {
                    tracing::error!(
                        subscription = %subscription_id,
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "failed to serialize an SSE subscription notification",
                            &error,
                        ),
                        "SSE subscription notification was not sent"
                    );
                    continue;
                }
            };
            let event = Event::default().event("notification").data(payload);
            if tx.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    let owner = agena_async::AbortOnDrop::new(&producer);
    let stream = ReceiverStream::new(rx).map(move |item| {
        let _ = &owner;
        item
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(25))))
}

async fn notification_event(
    message: agena_api::resource::NotificationStreamEvent,
) -> Option<Event> {
    let name = message.event_name();
    match crate::json_codec::encode_with(move || message.payload()).await {
        Ok(payload) => Some(Event::default().event(name).data(payload)),
        Err(error) => {
            tracing::error!(%error, "notification stream JSON worker failed");
            None
        }
    }
}

async fn notification_resource_event(
    notification: impl std::borrow::Borrow<agena_notification::model::Notification> + Send + 'static,
) -> Option<Event> {
    match crate::json_codec::encode_with(move || {
        agena_api::resource::NotificationResource::from(notification.borrow())
    })
    .await
    {
        Ok(payload) => Some(Event::default().event("notification").data(payload)),
        Err(error) => {
            tracing::error!(%error, "notification resource worker failed");
            None
        }
    }
}

impl StreamQuery {
    fn into_scope(self) -> Result<agena_api::Scope, ServerError> {
        let scope = match self.scope_kind.as_deref() {
            None | Some("global") => agena_api::Scope::Global,
            Some("workspace") => {
                let workspace_id = self.workspace_id.ok_or_else(|| {
                    ServerError::bad_request("scope_kind=workspace requires workspace_id")
                })?;
                agena_api::Scope::Workspace { workspace_id }
            }
            Some("session") => {
                let session_id = self.session_id.ok_or_else(|| {
                    ServerError::bad_request("scope_kind=session requires session_id")
                })?;
                agena_api::Scope::Session { session_id }
            }
            Some(other) => {
                return Err(ServerError::bad_request_with_diagnostic(
                    "The live-change scope is not supported.",
                    format!("unknown scope_kind: {other}"),
                ));
            }
        };
        Ok(scope)
    }
}

/// Query for the unified notification stream (`/api/v1/notifications/stream`).
#[derive(Debug, Deserialize, Default)]
pub struct NotificationStreamQuery {
    /// Replay notifications with `created_at_ms > since_ms` before attaching live.
    #[serde(default)]
    pub since_ms: Option<i64>,
    #[serde(default)]
    pub scope_kind: Option<String>,
    #[serde(default)]
    pub scope_id: Option<i64>,
    #[serde(default)]
    pub scope_key: Option<String>,
    #[serde(default)]
    pub severity: Option<agena_notification::model::NotificationSeverity>,
    #[serde(default)]
    pub surface: Option<agena_notification::model::NotificationSurface>,
    #[serde(default)]
    pub source: Option<agena_notification::model::NotificationSource>,
    #[serde(default = "default_active_only")]
    pub active_only: bool,
}

fn default_active_only() -> bool {
    true
}

impl NotificationStreamQuery {
    fn into_filter(self) -> Result<agena_notification::service::NotificationFilter, ServerError> {
        agena_api::resource::NotificationFilterParams {
            scope_kind: self.scope_kind,
            scope_id: self.scope_id,
            scope_key: self.scope_key,
            severity: self.severity,
            surface: self.surface,
            source: self.source,
            active_only: self.active_only,
            limit: None,
            cursor: None,
        }
        .into_filter()
        .map_err(|diagnostic| {
            ServerError::bad_request_with_diagnostic(
                "The notification filter is invalid.",
                diagnostic,
            )
        })
    }
}

/// Unified notification SSE stream. Deterministic event order on connect:
/// replayed history (`notification` events), then `resumed`, then live events
/// from the shared store broadcast (`notification` / `lagged` /
/// `subscription_closed`).
pub async fn notifications_stream(
    State(state): State<AppState>,
    Query(query): Query<NotificationStreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ServerError> {
    use crate::rest::notifications::notification_error;
    use agena_api::resource::NotificationStreamEvent;
    use agena_notification::NotificationService;
    use agena_runtime_notifications::store::SubscriptionEvent;

    let store = state.notifications().clone();
    let since_ms = query.since_ms.unwrap_or(0);
    let mut filter = query.into_filter()?;
    filter.limit = Some(1000);

    // Subscribe before reading: changes committed while the list is loading
    // must remain queued, including updates to an already replayed notice.
    let mut events = store.subscribe_events();
    let replayed = store
        .list(filter.clone())
        .await
        .map_err(notification_error)?;
    let mut watermark = since_ms;
    for notification in replayed.iter() {
        if notification.created_at_ms > since_ms {
            watermark = watermark.max(notification.created_at_ms);
        }
    }

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    let producer = tokio::spawn(async move {
        for notification in replayed.into_iter().filter(|n| n.created_at_ms > since_ms) {
            let Some(event) = notification_resource_event(notification).await else {
                continue;
            };
            if tx.send(Ok(event)).await.is_err() {
                return;
            }
        }
        let resumed = NotificationStreamEvent::Resumed {
            up_to_ms: watermark,
        };
        let Some(event) = notification_event(resumed).await else {
            return;
        };
        if tx.send(Ok(event)).await.is_err() {
            return;
        }

        loop {
            tokio::task::consume_budget().await;
            let item = tokio::select! {
                _ = tx.closed() => break,
                item = events.recv() => item,
            };
            let item = match item {
                Ok(item) => item,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    SubscriptionEvent::Lagged(skipped)
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            let event = match item {
                SubscriptionEvent::Notification(notification) => {
                    // An inactive update still has to remove a previously
                    // active row in this scope. Only the initial list uses
                    // active_only; live lifecycle changes use the same scope.
                    let live_filter = agena_notification::service::NotificationFilter {
                        active_only: false,
                        ..filter.clone()
                    };
                    if !agena_runtime_notifications::store::filter_matches(
                        &live_filter,
                        &notification,
                    ) {
                        continue;
                    }
                    notification_resource_event(notification).await
                }
                SubscriptionEvent::Lagged(skipped) => {
                    notification_event(NotificationStreamEvent::Lagged { skipped }).await
                }
                SubscriptionEvent::Closed => {
                    notification_event(NotificationStreamEvent::SubscriptionClosed {
                        reason: "notification store closed".into(),
                    })
                    .await
                }
            };
            let Some(event) = event else {
                continue;
            };
            if tx.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    let owner = agena_async::AbortOnDrop::new(&producer);
    let stream = ReceiverStream::new(rx).map(move |item| {
        let _ = &owner;
        item
    });
    Ok(Sse::new(stream).keep_alive(KeepAlive::new().interval(Duration::from_secs(25))))
}
