//! Server-Sent Events transport. Push-only event stream — no polling. The
//! handler subscribes through the runtime event-stream port and streams every
//! matching event to the HTTP response.

use std::convert::Infallible;
use std::time::Duration;

use agena_api::notifications::Notification;
use agena_runtime::RuntimeLiveEventSubscriptionItem;
use axum::{
    extract::{Query, State},
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::Stream;
use serde::Deserialize;
use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use crate::{error::ServerError, state::AppState};

#[derive(Debug, Deserialize, Default)]
pub struct StreamQuery {
    /// Resume from a previous `seq_global`. The server first replays the
    /// persisted store up to the high watermark and then attaches to the
    /// live broadcast.
    #[serde(default)]
    pub since_seq_global: Option<i64>,
    /// Filter by scope (`global` / `workspace` / `session`). Defaults to
    /// global. Encoded as a JSON-flattened scope object via `scope_kind` /
    /// `workspace_id` / `session_id` query params.
    #[serde(default)]
    pub scope_kind: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub session_id: Option<i64>,
    #[serde(default)]
    pub kinds: Option<String>, // comma-separated tags
}

pub async fn handler(
    State(state): State<AppState>,
    Query(query): Query<StreamQuery>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ServerError> {
    let stream_service = state.event_stream_service()?;
    let filter = query.into_filter()?;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    let subscription_id: smol_str::SmolStr = "sse".into();
    let mut subscription = stream_service.subscribe_events(filter);

    tokio::spawn(async move {
        while let Some(item) = subscription.recv().await {
            let notification = match item {
                RuntimeLiveEventSubscriptionItem::Event(event) => Notification::Event {
                    subscription: subscription_id.clone(),
                    event: Box::new(
                        agena_application::event_projection::event_resource_from_runtime(&event),
                    ),
                },
                RuntimeLiveEventSubscriptionItem::Lagged(skipped) => Notification::Lagged {
                    subscription: subscription_id.clone(),
                    skipped,
                },
            };
            let payload = match serde_json::to_string(&notification) {
                Ok(p) => p,
                Err(_) => continue,
            };
            let event = Event::default().event("notification").data(payload);
            if tx.send(Ok(event)).await.is_err() {
                break;
            }
        }
    });

    Ok(Sse::new(ReceiverStream::new(rx))
        .keep_alive(KeepAlive::new().interval(Duration::from_secs(25))))
}

impl StreamQuery {
    fn into_filter(self) -> Result<agena_domain::EventFilter, ServerError> {
        let scope = match self.scope_kind.as_deref() {
            None | Some("global") => agena_domain::EventScope::Global,
            Some("workspace") => {
                let workspace_id = self.workspace_id.ok_or_else(|| {
                    ServerError::bad_request("scope_kind=workspace requires workspace_id")
                })?;
                agena_domain::EventScope::Workspace { workspace_id }
            }
            Some("session") => {
                let session_id = self.session_id.ok_or_else(|| {
                    ServerError::bad_request("scope_kind=session requires session_id")
                })?;
                agena_domain::EventScope::Session { session_id }
            }
            Some(other) => {
                return Err(ServerError::bad_request_with_diagnostic(
                    "The event scope is not supported.",
                    format!("unknown scope_kind: {other}"),
                ));
            }
        };
        let kinds = self.kinds.map(|csv| {
            csv.split(',')
                .filter(|s| !s.is_empty())
                .map(|s| agena_domain::EventKindTag::from(s.trim()))
                .collect::<std::collections::HashSet<_>>()
        });
        Ok(agena_domain::EventFilter {
            scope,
            kinds,
            since_seq_global: self.since_seq_global,
        })
    }
}
