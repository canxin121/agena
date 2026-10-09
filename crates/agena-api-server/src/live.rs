//! Shared live feed for WS, SSE, and IPC transports.
//!
//! Session data arrives as facade `SessionChange` callbacks. Ephemeral
//! activity/plugin/tool-registry values arrive on `RuntimeLiveSignalService`.
//! This module merges both best-effort sources without inventing persistence,
//! replay, or a global sequence.

use agena_api::part::PartResource;
use agena_domain::PartDocument;
use portable_atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

use agena_api::Scope;
use agena_api::live::{
    RuntimeSignalResource, SessionChangeResource, SessionPartsResource, ToolDetailResource,
    ToolDetailSection,
};
use agena_runtime::{RuntimeLiveSignal, RuntimeLiveSignalItem};
use agena_runtime_contracts::part_content::ToolCallContent;
use agena_storage::store::{GlobalSubscription, Part, SessionChange, SessionStore};
use tokio::sync::mpsc;

use crate::{error::ServerError, state::AppState};

#[cfg(feature = "ws")]
pub(crate) async fn spawn_content_delivery(
    state: &AppState,
    id: agena_api::subscribe::SubscriptionId,
    request: agena_api::content::ReadContentParams,
    tx: mpsc::Sender<agena_api::ws::ServerMessage>,
) -> Result<tokio::task::JoinHandle<()>, ServerError> {
    use agena_api::ws::ServerMessage;
    let watch = state
        .application()
        .watch_content(request.session_id, request.resource_id)
        .await?;
    watch.read(request.after, 1).await?; // Reject an invalid cursor before acknowledging.
    let mut delivery = watch.delivery(request.after, request.max_bytes);
    tx.send(ServerMessage::Subscribed { id: id.clone() })
        .await
        .map_err(|_| ServerError::bad_request("The subscription connection closed."))?;
    Ok(tokio::spawn(async move {
        loop {
            let page = tokio::select! {
                _ = tx.closed() => return,
                page = delivery.next() => page,
            };
            let message = match page {
                Ok(Some(page)) => ServerMessage::Content {
                    subscription: id.clone(),
                    page,
                },
                Ok(None) => break,
                Err(error) => {
                    let _ = tx
                        .send(ServerMessage::Error {
                            id: Some(id.clone()),
                            error: ServerError::from(error).into_api(),
                        })
                        .await;
                    break;
                }
            };
            if tx.send(message).await.is_err() {
                return;
            }
        }
        let _ = tx
            .send(ServerMessage::Notification(
                agena_api::notifications::Notification::SubscriptionClosed {
                    subscription: id,
                    reason: "content subscription ended".into(),
                },
            ))
            .await;
    }))
}

#[derive(Debug, Clone)]
pub(crate) enum LiveItem {
    SessionChanged(SessionChangeResource),
    RuntimeSignal(RuntimeSignalResource),
    Lagged(u64),
}

enum StoreSubscription {
    All {
        _handle: GlobalSubscription,
    },
    Session {
        _handle: agena_storage::store::Subscription,
    },
}

pub(crate) struct LiveSubscription {
    rx: mpsc::Receiver<LiveItem>,
    dropped: Arc<AtomicU64>,
    pending_lag: u64,
    _store_subscription: StoreSubscription,
    projection_task: tokio::task::JoinHandle<()>,
    revisions: Arc<crate::revisions::ResourceRevisions>,
}

impl LiveSubscription {
    pub(crate) fn revisions_for(
        &self,
        item: &LiveItem,
    ) -> std::collections::BTreeMap<String, String> {
        match item {
            LiveItem::SessionChanged(change) => self.revisions.live_tokens(Some(change), None),
            LiveItem::RuntimeSignal(signal) => self.revisions.live_tokens(None, Some(signal)),
            LiveItem::Lagged(_) => Default::default(),
        }
    }
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

pub(crate) fn subscribe(state: &AppState, scope: Scope) -> Result<LiveSubscription, ServerError> {
    const LIVE_QUEUE_CAPACITY: usize = 256;

    subscribe_with_queue_capacity(state, scope, LIVE_QUEUE_CAPACITY)
}

#[cfg(test)]
pub(crate) fn subscribe_with_capacity(
    state: &AppState,
    scope: Scope,
    capacity: usize,
) -> Result<LiveSubscription, ServerError> {
    subscribe_with_queue_capacity(state, scope, capacity.max(1))
}

fn subscribe_with_queue_capacity(
    state: &AppState,
    scope: Scope,
    capacity: usize,
) -> Result<LiveSubscription, ServerError> {
    let store = state.session_store()?;
    let revisions = state.revisions()?;
    let signals = state.live_signals()?;
    let (tx, rx) = mpsc::channel(capacity);
    let (raw_change_tx, mut raw_change_rx) = mpsc::channel(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    let change_dropped = Arc::clone(&dropped);
    let visibility = SessionFilter::new(scope.clone());
    let change_visibility = visibility.clone();
    let observer: agena_storage::store::SessionObserver = Arc::new(move |change| {
        if !change_visible_to_user(&change)
            || change_visibility.known_change_allowed(&change) == Some(false)
        {
            return;
        }
        match raw_change_tx.try_send(change) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                change_dropped.fetch_add(1, Ordering::Relaxed);
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {}
        }
    });
    let store_subscription = match scope {
        Scope::Session { session_id } => StoreSubscription::Session {
            _handle: store.subscribe(session_id, observer),
        },
        _ => StoreSubscription::All {
            _handle: store.subscribe_all(observer),
        },
    };
    let mut signal_subscription = signals.subscribe();
    let projection_state = state.clone();
    let projection_task = tokio::spawn(async move {
        loop {
            let item = tokio::select! {
                _ = tx.closed() => break,
                change = raw_change_rx.recv() => match change {
                    Some(change) => {
                        if !visibility.change_allowed(store.as_ref(), &change).await {
                            continue;
                        }
                        if let Ok(revisions) = projection_state.revisions() { revisions.observe_change(&change); }
                        let Some(change) = project_change(&projection_state, change).await else {
                            continue;
                        };
                        Some(LiveItem::SessionChanged(change))
                    }
                    None => None,
                },
                signal = signal_subscription.recv() => match signal {
                    Some(RuntimeLiveSignalItem::Signal(signal)) => {
                        let session_id = match &signal {
                            RuntimeLiveSignal::Activity(activity) => activity.activity.parent_session_id.or(activity.activity.session_id),
                            RuntimeLiveSignal::Plugin { session_id, .. } => *session_id,
                            RuntimeLiveSignal::ToolRegistryChanged(_) => None,
                        };
                        if session_id.is_none() && !matches!(visibility.scope, Scope::Global) { continue; }
                        if let Some(session_id) = session_id
                            && !visibility.session_allowed(store.as_ref(), session_id).await {
                            continue;
                        }
                        if let Ok(revisions) = projection_state.revisions() { revisions.observe_signal(&signal); }
                        let signal = project_signal(signal);
                        Some(LiveItem::RuntimeSignal(signal))
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
        revisions,
    })
}

/// Scope and visibility are resolved before projection and serialization.
/// Session subscriptions attach directly to their membership bus; workspace
/// subscriptions cache factual classification rather than reading per event.
#[derive(Clone)]
struct SessionFilter {
    scope: Scope,
    sessions: Arc<Mutex<lru::LruCache<i64, (bool, i64)>>>,
}

impl SessionFilter {
    fn new(scope: Scope) -> Self {
        Self {
            scope,
            sessions: Arc::new(Mutex::new(lru::LruCache::new(
                std::num::NonZeroUsize::new(4096).unwrap(),
            ))),
        }
    }
    fn scope_allows(&self, session_id: i64, workspace_id: i64) -> bool {
        match self.scope {
            Scope::Global => true,
            Scope::Session {
                session_id: expected,
            } => session_id == expected,
            Scope::Workspace {
                workspace_id: expected,
            } => workspace_id == expected,
        }
    }
    fn known_change_allowed(&self, change: &SessionChange) -> Option<bool> {
        let session_id = change.session_id();
        if let Scope::Session {
            session_id: expected,
        } = self.scope
            && session_id != expected
        {
            return Some(false);
        }
        let mut sessions = self
            .sessions
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        match change {
            SessionChange::SessionMetaUpdated { meta, .. } => {
                let visible = !meta.is_temporary();
                sessions.put(session_id, (visible, meta.workspace_id));
                Some(visible && self.scope_allows(session_id, meta.workspace_id))
            }
            SessionChange::SessionDeleted {
                workspace_id,
                temporary,
                ..
            } => {
                sessions.pop(&session_id);
                Some(!temporary && self.scope_allows(session_id, *workspace_id))
            }
            _ => sessions
                .get(&session_id)
                .map(|(visible, workspace)| *visible && self.scope_allows(session_id, *workspace)),
        }
    }
    async fn session_allowed(&self, store: &dyn SessionStore, session_id: i64) -> bool {
        if let Scope::Session {
            session_id: expected,
        } = self.scope
            && session_id != expected
        {
            return false;
        }
        {
            let mut sessions = self
                .sessions
                .lock()
                .unwrap_or_else(|error| error.into_inner());
            if let Some((visible, workspace)) = sessions.get(&session_id) {
                return *visible && self.scope_allows(session_id, *workspace);
            }
        }
        match store.load_part_ids(session_id, &[]).await {
            Ok(view) => {
                let visible = !view.meta.is_temporary();
                self.sessions
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .put(session_id, (visible, view.meta.workspace_id));
                visible && self.scope_allows(session_id, view.meta.workspace_id)
            }
            Err(error) => {
                tracing::warn!(session_id,%error,"could not authorize live session scope");
                false
            }
        }
    }
    async fn change_allowed(&self, store: &dyn SessionStore, change: &SessionChange) -> bool {
        match self.known_change_allowed(change) {
            Some(allowed) => allowed,
            None => self.session_allowed(store, change.session_id()).await,
        }
    }
}

fn change_visible_to_user(change: &SessionChange) -> bool {
    match change {
        SessionChange::PartAdded { part, .. } | SessionChange::PartUpdated { part, .. } => {
            part.visibility.visible_to_user()
        }
        // Removals carry no visibility. Sending the id lets a client discard
        // a previously visible row; meta changes are session-level state.
        SessionChange::PartRemoved { .. }
        | SessionChange::SessionMetaUpdated { .. }
        | SessionChange::SessionDeleted { .. } => true,
    }
}

pub(crate) async fn session_parts(
    state: &AppState,
    session_id: i64,
) -> Result<SessionPartsResource, ServerError> {
    state
        .application()
        .read_runs(agena_api::queries::ReadRunsParams {
            session_id,
            limit: Some(8),
            part_limit: Some(32),
            ..Default::default()
        })
        .await
        .map_err(ServerError::from)
}

pub(crate) async fn project_part_for_user(state: &AppState, part: &Part) -> PartResource {
    project_part_for_sections(state, part, &[ToolDetailSection::Presentation]).await
}

/// Project one part for a caller that has explicitly requested particular
/// tool-call sections. Transcript snapshots deliberately request only the
/// human presentation; the raw input/output sections are fetched separately
/// when a disclosure row is opened.
pub(crate) async fn project_part_for_sections(
    state: &AppState,
    part: &Part,
    sections: &[ToolDetailSection],
) -> PartResource {
    state.application().project_part(part, sections).await
}

/// Project exactly one tool-call detail section. The returned value never
/// contains a sibling section, which keeps the lazy-loading boundary useful
/// even when the requested section is large. `locale` localizes the human
/// presentation; the stored metadata/input/output sections stay as recorded.
pub(crate) async fn project_tool_detail(
    state: &AppState,
    part: &Part,
    section: ToolDetailSection,
    locale: Option<&str>,
) -> Option<ToolDetailResource> {
    if part.kind != ToolCallContent::kind() {
        return None;
    }
    let content = ToolCallContent::try_from(&part.content).ok()?;
    let value = match section {
        ToolDetailSection::Metadata => serde_json::to_value(content.metadata).ok()?,
        ToolDetailSection::Input => content.input,
        ToolDetailSection::Output => serde_json::to_value(content.output).ok()?,
        ToolDetailSection::Presentation => project_tool_presentation(state, part, locale)
            .await
            .and_then(|presentation| serde_json::to_value(presentation).ok())
            .unwrap_or(serde_json::Value::Null),
    };
    Some(ToolDetailResource {
        part_id: part.part_id,
        revision: part.revision,
        updated_at_ms: part.updated_at_ms,
        section,
        part_state: (section == ToolDetailSection::Output).then(|| part.state.as_str().to_owned()),
        value,
    })
}

/// Number the user markers at `marker_indices` (ascending) starting at
/// `start`, incrementing by one. Kept pure and index-based so the walk — the
/// only place a page could be silently mis-numbered — is unit-testable
/// without a store.
#[cfg(test)]
fn number_user_markers(ordinals: &mut [Option<u64>], marker_indices: &[usize], start: Option<u64>) {
    let mut next = start;
    for index in marker_indices {
        if let Some(slot) = ordinals.get_mut(*index) {
            *slot = next;
        }
        next = next.map(|value| value.saturating_add(1));
    }
}

/// Resolve one live part's user-message ordinal in isolation. There is no
/// surrounding page to count against, so the store ranks the marker directly.
async fn project_part_with_ordinal(state: &AppState, session_id: i64, part: &Part) -> PartResource {
    let mut projected = project_part_for_user(state, part).await;
    if !is_user_message_marker(&projected) {
        return projected;
    }
    let store = match state.session_store() {
        Ok(store) => store,
        Err(error) => {
            tracing::error!(
                part_id = part.part_id,
                diagnostic = %error,
                "live user-message ordinal could not be resolved: session store unavailable"
            );
            return projected;
        }
    };
    match store.user_message_ordinal(session_id, part.part_id).await {
        Ok(ordinal) => projected.user_message_ordinal = ordinal,
        Err(error) => {
            tracing::error!(
                part_id = part.part_id,
                diagnostic = %error,
                "live user-message ordinal could not be resolved"
            );
        }
    }
    projected
}

fn is_user_message_marker(part: &PartResource) -> bool {
    part.kind == "run" && part.role == "user"
}

async fn project_tool_presentation(
    state: &AppState,
    part: &Part,
    locale: Option<&str>,
) -> Option<PartDocument> {
    let mut document = state
        .application()
        .part_document(&agena_application::session::part_resource_from_fact(part))
        .await?;
    agena_application::part_locale::localize_part_document(&mut document, locale);
    Some(document)
}

async fn project_change(state: &AppState, change: SessionChange) -> Option<SessionChangeResource> {
    Some(match change {
        SessionChange::SessionDeleted {
            session_id,
            workspace_id,
            ..
        } => SessionChangeResource::SessionDeleted {
            session_id,
            workspace_id,
        },
        SessionChange::PartAdded { session_id, part } if part.visibility.visible_to_user() => {
            SessionChangeResource::PartAdded {
                session_id,
                part: Box::new(project_part_with_ordinal(state, session_id, &part).await),
            }
        }
        SessionChange::PartUpdated { session_id, part } if part.visibility.visible_to_user() => {
            SessionChangeResource::PartUpdated {
                session_id,
                part: Box::new(project_part_with_ordinal(state, session_id, &part).await),
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
            // Delegated work is displayed and controlled by its parent. Its
            // child transcript has a separate stream of part updates.
            session_id: activity
                .activity
                .parent_session_id
                .or(activity.activity.session_id),
            payload: live_signal_payload(
                &serde_json::json!({
                    "activity_id": activity.activity_id,
                    "reason": activity.reason,
                    "ts_ms": activity.ts_ms,
                    "activity": agena_api::resource::BackgroundActivityResource::from(&activity.activity),
                }),
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
mod ordinal_tests {
    use super::{is_user_message_marker, number_user_markers};
    use agena_api::part::PartResource;

    /// The minimal `PartResource` the ordinal walk inspects.
    fn part(part_id: i64, kind: &str, role: &str) -> PartResource {
        PartResource {
            sections: Vec::new(),
            part_id,
            kind: kind.to_owned(),
            role: role.to_owned(),
            state: "completed".to_owned(),
            content: serde_json::Value::Null,
            presentation: None,
            summary: None,
            visibility: "both".to_owned(),
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 0,
            started_at_ms: 0,
            finished_at_ms: None,
            created_at_ms: 0,
            updated_at_ms: 0,
            user_message_ordinal: None,
            provider_state: None,
        }
    }

    #[test]
    fn only_user_send_run_markers_carry_an_ordinal() {
        assert!(is_user_message_marker(&part(1, "run", "user")));
        assert!(!is_user_message_marker(&part(2, "run", "assistant")));
        assert!(!is_user_message_marker(&part(3, "text", "user")));
    }

    #[test]
    fn a_page_numbers_its_user_markers_from_the_resolved_offset() {
        // parts: [assistant run, user run, text, user run]
        let mut ordinals = vec![None, None, None, None];
        number_user_markers(&mut ordinals, &[1, 3], Some(7));
        assert_eq!(ordinals, vec![None, Some(7), None, Some(8)]);
    }

    #[test]
    fn a_page_without_a_resolved_offset_leaves_markers_unset() {
        let mut ordinals = vec![None, None];
        number_user_markers(&mut ordinals, &[0, 1], None);
        assert_eq!(ordinals, vec![None, None]);
    }

    #[test]
    fn out_of_range_indices_are_ignored_instead_of_panicking() {
        let mut ordinals = vec![None];
        number_user_markers(&mut ordinals, &[0, 5], Some(3));
        assert_eq!(ordinals, vec![Some(3)]);
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
