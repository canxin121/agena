//! Live session event presentation: the [`LiveEvent`] value the TUI consumes
//! and the subscription adapter that pumps runtime presentation events into a
//! typed channel.

use agena_application::Application;
use tokio::sync::mpsc;

/// Push notification emitted by the unified bus for the active session.
/// Indicates whether the change requires reloading messages.
#[derive(Debug, Clone)]
pub struct LiveEvent {
    /// Snapshot captured after a live subscription was established. Remote
    /// reconnect uses this to close the subscribe/read race.
    pub snapshot: Option<agena_api::resource::SessionExecutionResource>,
    /// Concrete event payload when the subscriber kept up with the bus.
    /// `None` means the receiver lagged and the UI should force-refresh
    /// from persisted state instead of trying to apply an incremental patch.
    pub event: Option<agena_runtime::RuntimePresentationEvent>,
    /// True when the UI should ignore incremental assumptions and force a
    /// replay from persisted state (for example after bus lag).
    pub force_refresh: bool,
}

/// Subscribe through the Runtime-owned typed presentation stream. Generic
/// transport events remain available separately for timeline consumers.
pub(crate) fn subscribe_session_events(
    application: &super::TuiBackend,
    session_id: i64,
) -> Option<mpsc::Receiver<LiveEvent>> {
    Some(application.subscribe_session_events(session_id))
}

pub(super) fn subscribe_session_events_embedded(
    application: &Application,
    session_id: i64,
) -> Option<mpsc::Receiver<LiveEvent>> {
    const SESSION_CHANGE_QUEUE_CAPACITY: usize = 256;
    const LIVE_EVENT_QUEUE_CAPACITY: usize = 256;

    let store = application.session_store_facade().ok()?;
    let queries = application.session_execution_services().ok()?.queries;
    let (tx, rx) = mpsc::channel::<LiveEvent>(LIVE_EVENT_QUEUE_CAPACITY);
    let (change_tx, mut change_rx) = mpsc::channel(SESSION_CHANGE_QUEUE_CAPACITY);
    let (overflow_tx, mut overflow_rx) = tokio::sync::watch::channel(0u64);
    let subscription = store.subscribe_all(std::sync::Arc::new(move |change| {
        match change_tx.try_send(change) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                overflow_tx.send_modify(|generation| {
                    *generation = generation.wrapping_add(1);
                });
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    }));
    let change_output = tx.clone();
    tokio::spawn(async move {
        let _subscription = subscription;
        loop {
            let change = tokio::select! {
                change = change_rx.recv() => change,
                changed = overflow_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    if change_output
                        .send(LiveEvent {
                            snapshot: None,
                            event: None,
                            force_refresh: true,
                        })
                        .await
                        .is_err()
                    {
                        break;
                    }
                    continue;
                }
                _ = change_output.closed() => break,
            };
            let Some(change) = change else { break };
            let event = presentation_event_from_session_change(change);
            if event.meta.session_id != Some(session_id) {
                let Some(descendant_id) = event.meta.session_id else {
                    continue;
                };
                if !event.invalidates_ancestor_projection {
                    continue;
                }
                let is_descendant = queries
                    .is_descendant_session(descendant_id, session_id)
                    .await
                    .unwrap_or(false);
                if !is_descendant {
                    continue;
                }
                if change_output
                    .send(LiveEvent {
                        snapshot: None,
                        event: None,
                        force_refresh: true,
                    })
                    .await
                    .is_err()
                {
                    break;
                }
                continue;
            }
            let live = LiveEvent {
                snapshot: None,
                event: Some(event),
                force_refresh: false,
            };
            if change_output.send(live).await.is_err() {
                break;
            }
        }
    });
    if let Ok(signals) = application.live_signal_service() {
        let mut subscription = signals.subscribe();
        tokio::spawn(async move {
            loop {
                let item = tokio::select! {
                    item = subscription.recv() => item,
                    _ = tx.closed() => break,
                };
                let Some(item) = item else { break };
                let live = live_event_from_runtime_signal(item, session_id);
                if let Some(live) = live
                    && tx.send(live).await.is_err()
                {
                    break;
                }
            }
        });
    }
    Some(rx)
}

fn presentation_event_from_session_change(
    change: agena_storage::store::SessionChange,
) -> agena_runtime::RuntimePresentationEvent {
    let (session_id, workspace_id, seq_global, seq_session, created_at_ms) = match &change {
        agena_storage::store::SessionChange::PartAdded { session_id, part }
        | agena_storage::store::SessionChange::PartUpdated { session_id, part } => (
            *session_id,
            None,
            part.updated_at_ms,
            Some(part.revision),
            part.updated_at_ms,
        ),
        agena_storage::store::SessionChange::PartRemoved {
            session_id,
            part_id,
        } => (
            *session_id,
            None,
            *part_id,
            None,
            chrono::Utc::now().timestamp_millis(),
        ),
        agena_storage::store::SessionChange::SessionMetaUpdated { session_id, meta } => (
            *session_id,
            Some(meta.workspace_id),
            meta.version,
            Some(meta.version),
            meta.updated_at_ms,
        ),
    };
    agena_runtime::RuntimePresentationEvent {
        meta: agena_runtime::RuntimePresentationEventMeta {
            id: uuid::Uuid::new_v4(),
            seq_global,
            seq_session,
            session_id: Some(session_id),
            workspace_id,
            created_at: chrono::DateTime::from_timestamp_millis(created_at_ms)
                .unwrap_or(chrono::DateTime::UNIX_EPOCH),
            causation_id: None,
            correlation_id: None,
            envelope_schema: 1,
        },
        invalidates_ancestor_projection: true,
        durable: true,
        kind: agena_runtime::RuntimePresentationEventKind::PartPatch(Box::new(change)),
    }
}

fn live_event_from_runtime_signal(
    item: agena_runtime::RuntimeLiveSignalItem,
    selected_session_id: i64,
) -> Option<LiveEvent> {
    let signal = match item {
        agena_runtime::RuntimeLiveSignalItem::Lagged(_) => {
            return Some(LiveEvent {
                snapshot: None,
                event: None,
                force_refresh: true,
            });
        }
        agena_runtime::RuntimeLiveSignalItem::Signal(signal) => signal,
    };
    let now = chrono::Utc::now();
    let (session_id, invalidates_ancestor_projection, kind) = match signal {
        agena_runtime::RuntimeLiveSignal::Activity(activity) => {
            let session_id = activity.activity.session_id;
            let invalidates_ancestor = activity.activity.parent_session_id
                == Some(selected_session_id)
                && session_id != Some(selected_session_id);
            if session_id != Some(selected_session_id) && !invalidates_ancestor {
                return None;
            }
            (
                session_id,
                invalidates_ancestor,
                agena_runtime::RuntimePresentationEventKind::ActivityChanged {
                    activity: Box::new(activity.activity),
                    reason: activity.reason,
                },
            )
        }
        agena_runtime::RuntimeLiveSignal::Plugin { session_id, .. } => {
            if session_id != Some(selected_session_id) {
                return None;
            }
            (
                session_id,
                false,
                agena_runtime::RuntimePresentationEventKind::Refresh {
                    force_refresh: false,
                },
            )
        }
        agena_runtime::RuntimeLiveSignal::ToolRegistryChanged(_) => return None,
    };
    Some(LiveEvent {
        snapshot: None,
        event: Some(agena_runtime::RuntimePresentationEvent {
            meta: agena_runtime::RuntimePresentationEventMeta {
                id: uuid::Uuid::new_v4(),
                seq_global: now.timestamp_millis(),
                seq_session: None,
                session_id,
                workspace_id: None,
                created_at: now,
                causation_id: None,
                correlation_id: None,
                envelope_schema: 1,
            },
            invalidates_ancestor_projection,
            durable: false,
            kind,
        }),
        force_refresh: false,
    })
}
