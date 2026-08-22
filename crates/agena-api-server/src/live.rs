//! Shared v2 live feed for WS, SSE, and IPC transports.
//!
//! Session data arrives as facade `SessionChange` callbacks. Ephemeral
//! activity/plugin/tool-registry values arrive on `RuntimeLiveSignalService`.
//! This module merges both best-effort sources without inventing persistence,
//! replay, or a global sequence.

use portable_atomic::AtomicU64;
use std::sync::Arc;
use std::sync::atomic::Ordering;

use agena_api::{
    Scope,
    live::{
        PartResource, RuntimeSignalResource, SessionChangeResource, SessionPartsResource,
        ToolHumanPresentationResource,
    },
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_storage::store::{GlobalSubscription, Part, SessionChange, SessionStore};
use tokio::sync::mpsc;

use crate::{error::ServerError, state::AppState};

#[derive(Debug, Clone)]
pub(crate) enum LiveItem {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Lagged(u64),
}

pub(crate) struct LiveSubscription {
    rx: mpsc::Receiver<LiveItem>,
    dropped: Arc<AtomicU64>,
    pending_lag: u64,
    _store_subscription: GlobalSubscription,
    projection_task: tokio::task::JoinHandle<()>,
}

impl LiveSubscription {
    pub(crate) async fn recv(&mut self) -> Option<LiveItem> {
        if self.pending_lag > 0 {
            return Some(LiveItem::Lagged(std::mem::take(&mut self.pending_lag)));
        }
        match self.rx.recv().await {
            Some(item) => {
                self.pending_lag = self.dropped.swap(0, Ordering::AcqRel);
                Some(item)
            }
            None => {
                let skipped = self.dropped.swap(0, Ordering::AcqRel);
                (skipped > 0).then_some(LiveItem::Lagged(skipped))
            }
        }
    }
}

impl Drop for LiveSubscription {
    fn drop(&mut self) {
        self.projection_task.abort();
    }
}

pub(crate) fn subscribe(state: &AppState) -> Result<LiveSubscription, ServerError> {
    const LIVE_QUEUE_CAPACITY: usize = 256;

    subscribe_with_queue_capacity(state, LIVE_QUEUE_CAPACITY)
}

#[cfg(test)]
pub(crate) fn subscribe_with_capacity(
    state: &AppState,
    capacity: usize,
) -> Result<LiveSubscription, ServerError> {
    subscribe_with_queue_capacity(state, capacity.max(1))
}

fn subscribe_with_queue_capacity(
    state: &AppState,
    capacity: usize,
) -> Result<LiveSubscription, ServerError> {
    let store = state.session_store()?;
    let signals = state.live_signals()?;
    let (tx, rx) = mpsc::channel(capacity);
    let (raw_change_tx, mut raw_change_rx) = mpsc::channel(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    let change_dropped = Arc::clone(&dropped);
    let store_subscription = store.subscribe_all(Arc::new(move |change| {
        if !change_visible_to_user(&change) {
            return;
        }
        match raw_change_tx.try_send(change) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                change_dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }));
    let mut signal_subscription = signals.subscribe();
    let projection_state = state.clone();
    let projection_task = tokio::spawn(async move {
        loop {
            let item = tokio::select! {
                change = raw_change_rx.recv() => match change {
                    Some(change) => project_change(&projection_state, change)
                        .await
                        .map(LiveItem::SessionChanged),
                    None => None,
                },
                signal = signal_subscription.recv() => match signal {
                    Some(RuntimeLiveSignalItem::Signal(signal)) => {
                        Some(LiveItem::RuntimeSignal(project_signal(signal)))
                    }
                    Some(RuntimeLiveSignalItem::Lagged(skipped)) => {
                        Some(LiveItem::Lagged(skipped))
                    }
                    None => None,
                }
            };
            let Some(item) = item else {
                break;
            };
            if tx.send(item).await.is_err() {
                break;
            }
        }
    });
    Ok(LiveSubscription {
        rx,
        dropped,
        pending_lag: 0,
        _store_subscription: store_subscription,
        projection_task,
    })
}

fn change_visible_to_user(change: &SessionChange) -> bool {
    match change {
        SessionChange::PartAdded { part, .. } | SessionChange::PartUpdated { part, .. } => {
            part.visibility.visible_to_user()
        }
        // Removals carry no visibility. Sending the id lets a client discard
        // a previously visible row; meta changes are session-level state.
        SessionChange::PartRemoved { .. } | SessionChange::SessionMetaUpdated { .. } => true,
    }
}

pub(crate) async fn matches_scope(
    item: &LiveItem,
    scope: &Scope,
    store: &dyn SessionStore,
) -> bool {
    if matches!(scope, Scope::Global) {
        return true;
    }
    let session_id = match item {
        LiveItem::SessionChanged(change) => Some(change.session_id()),
        LiveItem::RuntimeSignal(signal) => signal.session_id,
        LiveItem::Lagged(_) => return true,
    };
    let Some(session_id) = session_id else {
        return false;
    };
    match scope {
        Scope::Global => true,
        Scope::Session {
            session_id: expected,
        } => session_id == *expected,
        Scope::Workspace { workspace_id } => match store.get_session_summary(session_id).await {
            Ok(summary) => summary.is_some_and(|summary| summary.workspace_id == *workspace_id),
            Err(error) => {
                // Scope filtering is an authorization boundary. A lookup
                // failure must deny the event, but it must not disappear as
                // though the session simply belonged to another workspace.
                tracing::error!(
                    session_id,
                    workspace_id,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "load a session summary while applying a live workspace scope",
                        &error,
                    ),
                    "live event was denied because its workspace scope could not be verified"
                );
                false
            }
        },
    }
}

pub(crate) async fn session_parts(
    state: &AppState,
    store: &dyn SessionStore,
    session_id: i64,
) -> Result<SessionPartsResource, ServerError> {
    let view = store
        .load(session_id)
        .await
        .map_err(|error| ServerError::internal_error(&error))?;
    let visible = view
        .parts
        .into_iter()
        .filter(|part| part.visibility.visible_to_user())
        .collect::<Vec<_>>();
    let parts = project_parts_for_user(state, &visible).await;
    Ok(SessionPartsResource {
        session_id,
        version: view.meta.version,
        page: agena_api::pagination::PageInfo {
            next_cursor: None,
            has_more: false,
            returned: parts.len() as u64,
        },
        parts,
        folds: Vec::new(),
    })
}

pub(crate) async fn project_part_for_user(state: &AppState, part: &Part) -> PartResource {
    let presentation = project_tool_presentation(state, part).await;
    PartResource {
        part_id: part.part_id,
        kind: part.kind.clone(),
        role: part.role.as_str().to_owned(),
        state: part.state.as_str().to_owned(),
        content: part.content.clone(),
        presentation,
        summary: part.summary.clone(),
        visibility: part.visibility.as_str().to_owned(),
        parent_part_id: part.parent_part_id,
        run_id: part.run_id,
        origin_session_id: part.origin_session_id,
        revision: part.revision,
        started_at_ms: part.started_at_ms,
        finished_at_ms: part.finished_at_ms,
        created_at_ms: part.created_at_ms,
        updated_at_ms: part.updated_at_ms,
        provider_state: part.provider_state.clone(),
    }
}

pub(crate) async fn project_parts_for_user(state: &AppState, parts: &[Part]) -> Vec<PartResource> {
    let mut projected = Vec::with_capacity(parts.len());
    for part in parts {
        if part.visibility.visible_to_user() {
            projected.push(project_part_for_user(state, part).await);
        }
    }
    projected
}

async fn project_tool_presentation(
    state: &AppState,
    part: &Part,
) -> Option<ToolHumanPresentationResource> {
    if part.kind != ToolCallContent::kind() {
        return None;
    }
    let content = match ToolCallContent::try_from(&part.content) {
        Ok(content) => content,
        Err(error) => {
            tracing::warn!(
                part_id = part.part_id,
                diagnostic = %error,
                "tool presentation skipped malformed persisted tool-call content"
            );
            return None;
        }
    };
    let input = match agena_domain::StructuredObject::try_from(content.input) {
        Ok(input) => input,
        Err(error) => {
            tracing::warn!(
                part_id = part.part_id,
                diagnostic = %format!(
                    "decode persisted tool-call input for user presentation: {error}"
                ),
                "tool presentation skipped malformed persisted tool-call input"
            );
            return None;
        }
    };
    let invocation = agena_domain::ToolInvocation {
        tool_api_call: content.tool_api_call,
        name: content.name,
        plugin_name: content.plugin,
        input,
    };
    let Some(output) = content.output else {
        return Some(ToolHumanPresentationResource {
            title: invocation.name,
            summary: String::new(),
            blocks: Vec::new(),
        });
    };
    let projection = state
        .application()
        .render_tool_result(&invocation, &output)
        .await;
    Some(ToolHumanPresentationResource {
        title: projection.human.title,
        summary: projection.human.summary,
        blocks: projection.human.blocks,
    })
}

async fn project_change(state: &AppState, change: SessionChange) -> Option<SessionChangeResource> {
    Some(match change {
        SessionChange::PartAdded { session_id, part } if part.visibility.visible_to_user() => {
            SessionChangeResource::PartAdded {
                session_id,
                part: Box::new(project_part_for_user(state, &part).await),
            }
        }
        SessionChange::PartUpdated { session_id, part } if part.visibility.visible_to_user() => {
            SessionChangeResource::PartUpdated {
                session_id,
                part: Box::new(project_part_for_user(state, &part).await),
            }
        }
        SessionChange::PartAdded { .. } | SessionChange::PartUpdated { .. } => return None,
        SessionChange::PartRemoved {
            session_id,
            part_id,
        } => SessionChangeResource::PartRemoved {
            session_id,
            part_id,
        },
        SessionChange::SessionMetaUpdated { session_id, meta } => {
            SessionChangeResource::SessionMetaUpdated {
                session_id,
                version: meta.version,
                title: meta.title,
                favorite: meta.favorite,
                pinned: meta.pinned,
                updated_at_ms: meta.updated_at_ms,
            }
        }
    })
}

fn live_signal_payload<T: serde::Serialize>(value: &T, context: &'static str) -> serde_json::Value {
    match serde_json::to_value(value) {
        Ok(payload) => payload,
        Err(error) => {
            tracing::error!(
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    context,
                    &error,
                ),
                "runtime live signal payload could not be serialized"
            );
            serde_json::json!({
                "projection_error": "The runtime signal payload could not be encoded."
            })
        }
    }
}

fn project_signal(signal: RuntimeLiveSignal) -> RuntimeSignalResource {
    match signal {
        RuntimeLiveSignal::Activity(activity) => RuntimeSignalResource {
            kind: "activity".to_owned(),
            session_id: activity.activity.session_id,
            payload: live_signal_payload(
                &*activity,
                "serialize a runtime activity live signal payload",
            ),
        },
        RuntimeLiveSignal::Plugin {
            session_id,
            plugin_id,
            kind_label,
            payload,
        } => RuntimeSignalResource {
            kind: "plugin".to_owned(),
            session_id,
            payload: serde_json::json!({
                "plugin_id": plugin_id.to_string(),
                "kind": kind_label,
                "payload": payload,
            }),
        },
        RuntimeLiveSignal::ToolRegistryChanged(change) => RuntimeSignalResource {
            kind: "tool_registry_changed".to_owned(),
            session_id: None,
            payload: live_signal_payload(
                &*change,
                "serialize a runtime tool-registry live signal payload",
            ),
        },
    }
}

#[cfg(test)]
mod visibility_tests {
    use super::change_visible_to_user;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility, SessionChange};

    fn part(visibility: PartVisibility) -> Part {
        Part {
            part_id: 1,
            kind: "text".to_owned(),
            role: PartRole::Assistant,
            state: PartState::Completed,
            content: serde_json::json!({"text": "feed"}),
            summary: None,
            visibility,
            parent_part_id: None,
            run_id: Some(1),
            origin_session_id: 1,
            revision: 0,
            started_at_ms: 1,
            finished_at_ms: Some(1),
            created_at_ms: 1,
            updated_at_ms: 1,
            provider_state: None,
        }
    }

    #[test]
    fn shared_ws_sse_ipc_feed_exposes_both_and_user_but_not_ai() {
        for (visibility, expected) in [
            (PartVisibility::Both, true),
            (PartVisibility::User, true),
            (PartVisibility::Ai, false),
        ] {
            assert_eq!(
                change_visible_to_user(&SessionChange::PartAdded {
                    session_id: 1,
                    part: part(visibility),
                }),
                expected,
                "added {visibility:?}"
            );
            assert_eq!(
                change_visible_to_user(&SessionChange::PartUpdated {
                    session_id: 1,
                    part: part(visibility),
                }),
                expected,
                "updated {visibility:?}"
            );
        }
    }
}
