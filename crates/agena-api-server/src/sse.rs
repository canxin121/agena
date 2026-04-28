//! Server-Sent Events transport. Push-only event stream — no polling. The
//! handler subscribes to the in-process [`EventBus`] and streams every
//! matching event to the HTTP response.

use std::convert::Infallible;
use std::time::Duration;

use agena::event::EventKind;
use agena_api::notifications::Notification;
use agena_event::{EventBus, bus::SubscriptionItem};
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
    let bus = state.event_bus()?;
    let filter = query.into_filter()?;

    let (tx, rx) = mpsc::channel::<Result<Event, Infallible>>(256);
    let subscription_id: smol_str::SmolStr = "sse".into();
    let mut subscription = bus.subscribe(filter);

    tokio::spawn(async move {
        while let Some(item) = subscription.recv().await {
            let notification = match item {
                SubscriptionItem::Event(event) => Notification::Event {
                    subscription: subscription_id.clone(),
                    event: Box::new((*event).clone()),
                },
                SubscriptionItem::Lagged(skipped) => Notification::Lagged {
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
    fn into_filter(self) -> Result<agena_event::EventFilter, ServerError> {
        let scope = match self.scope_kind.as_deref() {
            None | Some("global") => agena_event::Scope::Global,
            Some("workspace") => {
                let workspace_id = self.workspace_id.ok_or_else(|| {
                    ServerError::BadRequest(
                        "scope_kind=workspace requires workspace_id".into(),
                    )
                })?;
                agena_event::Scope::Workspace { workspace_id }
            }
            Some("session") => {
                let session_id = self.session_id.ok_or_else(|| {
                    ServerError::BadRequest("scope_kind=session requires session_id".into())
                })?;
                agena_event::Scope::Session { session_id }
            }
            Some(other) => {
                return Err(ServerError::BadRequest(format!(
                    "unknown scope_kind: {other}"
                )));
            }
        };
        let kinds = self.kinds.map(|csv| {
            csv.split(',')
                .filter(|s| !s.is_empty())
                .map(|s| agena_event::EventKindTag::from(s.trim()))
                .collect::<std::collections::HashSet<_>>()
        });
        Ok(agena_event::EventFilter {
            scope,
            kinds,
            since_seq_global: self.since_seq_global,
        })
    }
}

// Suppress unused-import warning when EventBus<EventKind> is only used via
// trait-object dispatch.
#[allow(dead_code)]
fn _bus_assertion(b: std::sync::Arc<dyn EventBus<EventKind>>) -> std::sync::Arc<dyn EventBus<EventKind>> {
    b
}
