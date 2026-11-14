//! Server-Sent Events transport for best-effort part patches and ephemeral
//! runtime signals. Persisted catch-up is a separate ordered-parts read.

use std::convert::Infallible;
use std::time::Duration;

use agena_api::notifications::Notification;
use axum::{
    extract::{Query, State},
    response::{
        IntoResponse, Response,
        sse::{Event, KeepAlive, Sse},
    },
};
use futures_util::StreamExt as _;
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
    /// UI language for human headlines. Stored part data stays English.
    #[serde(default)]
    pub locale: Option<String>,
}

pub async fn handler(
    State(state): State<AppState>,
    Query(query): Query<StreamQuery>,
) -> Result<Response, ServerError> {
    let locale = query.locale.clone();
    let scope = query.into_scope()?;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    let subscription_id: smol_str::SmolStr = "sse".into();
    let mut subscription = live::subscribe(&state, scope)?;
    let _ = tx.try_send(Ok(Event::default().comment("subscribed")));

    let producer = tokio::spawn(async move {
        while let Some(item) = tokio::select! {
            _ = tx.closed() => None,
            item = subscription.recv() => item,
        } {
            tokio::task::consume_budget().await;
            let revisions = subscription.revisions_for(&item);
            let notification = match item {
                LiveItem::SessionChanged(mut change) => {
                    agena_application::part_locale::localize_session_change(
                        &mut change,
                        locale.as_deref(),
                    );
                    Notification::SessionChanged {
                        subscription: subscription_id.clone(),
                        change: Box::new(change),
                        revisions,
                    }
                }
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
    Ok(crate::rest::unbuffered_sse_response(
        Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
            .into_response(),
    ))
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
    fn into_params(self) -> agena_api::business::NotificationFilterParams {
        agena_api::business::NotificationFilterParams {
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
    }
}

/// Encode the application's shared replay-and-live notification stream as SSE.
pub async fn notifications_stream(
    State(state): State<AppState>,
    Query(query): Query<NotificationStreamQuery>,
) -> Result<Response, ServerError> {
    let since_ms = query.since_ms;
    let receiver = state
        .application()
        .watch_notifications(query.into_params(), since_ms)
        .await?;
    let stream = ReceiverStream::new(receiver).filter_map(|message| async move {
        notification_event(message).await.map(Ok::<_, Infallible>)
    });
    Ok(crate::rest::unbuffered_sse_response(
        Sse::new(stream)
            .keep_alive(KeepAlive::new().interval(Duration::from_secs(25)))
            .into_response(),
    ))
}
