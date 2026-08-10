//! Shared v2 live feed for WS, SSE, and IPC transports.
//!
//! Session data arrives as facade `SessionChange` callbacks. Ephemeral
//! activity/plugin/tool-registry values arrive on `RuntimeLiveSignalService`.
//! This module merges both best-effort sources without inventing persistence,
//! replay, or a global sequence.

use std::sync::Arc;

use agena_api::{
    Scope,
    live::{PartResource, RuntimeSignalResource, SessionChangeResource, SessionPartsResource},
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
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
    rx: mpsc::UnboundedReceiver<LiveItem>,
    _store_subscription: GlobalSubscription,
    signal_task: tokio::task::JoinHandle<()>,
}

impl LiveSubscription {
    pub(crate) async fn recv(&mut self) -> Option<LiveItem> {
        self.rx.recv().await
    }
}

impl Drop for LiveSubscription {
    fn drop(&mut self) {
        self.signal_task.abort();
    }
}

pub(crate) fn subscribe(state: &AppState) -> Result<LiveSubscription, ServerError> {
    let store = state.session_store()?;
    let signals = state.live_signals()?;
    let (tx, rx) = mpsc::unbounded_channel();
    let change_tx = tx.clone();
    let store_subscription = store.subscribe_all(Arc::new(move |change| {
        let _ = change_tx.send(LiveItem::SessionChanged(project_change(change)));
    }));
    let mut signal_subscription = signals.subscribe();
    let signal_task = tokio::spawn(async move {
        while let Some(item) = signal_subscription.recv().await {
            let item = match item {
                RuntimeLiveSignalItem::Signal(signal) => {
                    LiveItem::RuntimeSignal(project_signal(signal))
                }
                RuntimeLiveSignalItem::Lagged(skipped) => LiveItem::Lagged(skipped),
            };
            if tx.send(item).is_err() {
                break;
            }
        }
    });
    Ok(LiveSubscription {
        rx,
        _store_subscription: store_subscription,
        signal_task,
    })
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
        Scope::Workspace { workspace_id } => store
            .get_session_summary(session_id)
            .await
            .ok()
            .flatten()
            .is_some_and(|summary| summary.workspace_id == *workspace_id),
    }
}

pub(crate) async fn session_parts(
    store: &dyn SessionStore,
    session_id: i64,
) -> Result<SessionPartsResource, ServerError> {
    let view = store
        .load(session_id)
        .await
        .map_err(|error| ServerError::internal(error.to_string()))?;
    Ok(SessionPartsResource {
        session_id,
        version: view.meta.version,
        parts: view.parts.iter().map(project_part).collect(),
    })
}

pub(crate) fn project_part(part: &Part) -> PartResource {
    PartResource {
        part_id: part.part_id,
        kind: part.kind.clone(),
        role: part.role.as_str().to_owned(),
        state: part.state.as_str().to_owned(),
        content: part.content.clone(),
        summary: part.summary.clone(),
        visibility: part.visibility.as_str().to_owned(),
        rendered_markdown: part.rendered_markdown.clone(),
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

fn project_change(change: SessionChange) -> SessionChangeResource {
    match change {
        SessionChange::PartAdded { session_id, part } => SessionChangeResource::PartAdded {
            session_id,
            part: Box::new(project_part(&part)),
        },
        SessionChange::PartUpdated { session_id, part } => SessionChangeResource::PartUpdated {
            session_id,
            part: Box::new(project_part(&part)),
        },
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
                updated_at_ms: meta.updated_at_ms,
            }
        }
    }
}

fn project_signal(signal: RuntimeLiveSignal) -> RuntimeSignalResource {
    match signal {
        RuntimeLiveSignal::Activity(activity) => RuntimeSignalResource {
            kind: "activity".to_owned(),
            session_id: activity.activity.session_id,
            payload: serde_json::to_value(&*activity).unwrap_or(serde_json::Value::Null),
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
            payload: serde_json::to_value(&*change).unwrap_or(serde_json::Value::Null),
        },
    }
}
