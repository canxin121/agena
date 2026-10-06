//! Shared live feed for WS, SSE, and IPC transports.
//!
//! Session data arrives as facade `SessionChange` callbacks. Ephemeral
//! activity/plugin/tool-registry values arrive on `RuntimeLiveSignalService`.
//! This module merges both best-effort sources without inventing persistence,
//! replay, or a global sequence.

use portable_atomic::AtomicU64;
use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};

#[cfg(any(feature = "ws", feature = "sse"))]
use agena_api::Scope;
use agena_api::live::{
    HumanPresentationResource, PartResource, RuntimeSignalResource, SessionChangeResource,
    SessionPartsResource, ToolDetailResource, ToolDetailSection,
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
    revisions: Arc<crate::revisions::ResourceRevisions>,
}

impl LiveSubscription {
    pub(crate) fn revisions_for(&self, item: &LiveItem) -> std::collections::BTreeMap<String, String> {
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
    let revisions = state.revisions()?;
    let signals = state.live_signals()?;
    let (tx, rx) = mpsc::channel(capacity);
    let (raw_change_tx, mut raw_change_rx) = mpsc::channel(capacity);
    let dropped = Arc::new(AtomicU64::new(0));
    let change_dropped = Arc::clone(&dropped);
    let visibility = SessionVisibility::default();
    let change_visibility = visibility.clone();
    let store_subscription = store.subscribe_all(Arc::new(move |change| {
        if !change_visible_to_user(&change)
            || change_visibility.known_change_visible(&change) == Some(false)
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
    }));
    let mut signal_subscription = signals.subscribe();
    let projection_state = state.clone();
    let projection_task = tokio::spawn(async move {
        loop {
            let item = tokio::select! {
                _ = tx.closed() => break,
                change = raw_change_rx.recv() => match change {
                    Some(change) => {
                        if !visibility.change_visible(store.as_ref(), &change).await {
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
                        if let Ok(revisions) = projection_state.revisions() { revisions.observe_signal(&signal); }
                        let signal = project_signal(signal);
                        if let Some(session_id) = signal.session_id
                            && !visibility.session_visible(store.as_ref(), session_id).await {
                            continue;
                        }
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

/// Cache classification per connection, so streamed parts do not trigger a
/// database query per token. The bound also covers very long-lived clients.
#[derive(Clone, Default)]
struct SessionVisibility {
    visible: Arc<Mutex<HashMap<i64, bool>>>,
}

impl SessionVisibility {
    fn remember(&self, session_id: i64, visible: bool) {
        let mut cache = self
            .visible
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if cache.len() >= 4096 && !cache.contains_key(&session_id) {
            cache.clear();
        }
        cache.insert(session_id, visible);
    }

    fn cached(&self, session_id: i64) -> Option<bool> {
        self.visible
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(&session_id)
            .copied()
    }

    async fn session_visible(&self, store: &dyn SessionStore, session_id: i64) -> bool {
        if let Some(visible) = self.cached(session_id) {
            return visible;
        }
        // An empty membership selection reads metadata without loading the
        // transcript, including when a client connects halfway through BTW.
        match store.load_part_ids(session_id, &[]).await {
            Ok(view) => {
                let visible = !view.meta.is_temporary();
                self.remember(session_id, visible);
                visible
            }
            Err(agena_storage::store::StoreError::NotFound(_)) => false,
            Err(error) => {
                tracing::warn!(session_id, %error, "could not classify live session visibility");
                false
            }
        }
    }

    // Filter known temporary changes before the bounded projection queue.
    // In particular, deleting a long inherited history must not flood every
    // UI subscriber with thousands of invisible membership removals.
    fn known_change_visible(&self, change: &SessionChange) -> Option<bool> {
        let session_id = change.session_id();
        match change {
            SessionChange::SessionMetaUpdated { meta, .. } => {
                let visible = !meta.is_temporary();
                self.remember(session_id, visible);
                Some(visible)
            }
            SessionChange::SessionDeleted { temporary, .. } => {
                self.visible
                    .lock()
                    .unwrap_or_else(|error| error.into_inner())
                    .remove(&session_id);
                Some(!temporary)
            }
            _ => self.cached(session_id),
        }
    }

    async fn change_visible(&self, store: &dyn SessionStore, change: &SessionChange) -> bool {
        match self.known_change_visible(change) {
            Some(visible) => visible,
            None => self.session_visible(store, change.session_id()).await,
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

#[cfg(any(feature = "ws", feature = "sse"))]
pub(crate) async fn matches_scope(
    item: &LiveItem,
    scope: &Scope,
    store: &dyn SessionStore,
) -> bool {
    if matches!(scope, Scope::Global) {
        return true;
    }
    if let (
        LiveItem::SessionChanged(SessionChangeResource::SessionDeleted { workspace_id, .. }),
        Scope::Workspace {
            workspace_id: expected,
        },
    ) = (item, scope)
    {
        return workspace_id == expected;
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
    let mut parts = project_parts_for_user(state, &visible).await;
    assign_user_message_ordinals(store, session_id, &mut parts).await?;
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
        user_message_count: None,
    })
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
    let presentation = if sections.contains(&ToolDetailSection::Presentation) {
        project_tool_presentation(state, part).await
    } else {
        None
    };
    PartResource {
        part_id: part.part_id,
        // The stored kind keeps user payloads under `text`; clients need the
        // canonical kind to render an attachment row at all.
        kind: agena_runtime_contracts::part_content::canonical_kind(&part.kind, &part.content),
        role: part.role.as_str().to_owned(),
        state: part.state.as_str().to_owned(),
        content: if part.kind == ToolCallContent::kind() {
            agena_api::live::project_tool_call_content(&part.content, sections)
        } else {
            part.content.clone()
        },
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
        // Assigned by `assign_user_message_ordinals` at the response boundary,
        // which is the only layer that knows the surrounding page/window.
        user_message_ordinal: None,
        provider_state: part.provider_state.clone(),
    }
}

/// Project exactly one tool-call detail section. The returned value never
/// contains a sibling section, which keeps the lazy-loading boundary useful
/// even when the requested section is large.
pub(crate) async fn project_tool_detail(
    state: &AppState,
    part: &Part,
    section: ToolDetailSection,
) -> Option<ToolDetailResource> {
    if part.kind != ToolCallContent::kind() {
        return None;
    }
    let content = ToolCallContent::try_from(&part.content).ok()?;
    let value = match section {
        ToolDetailSection::Metadata => serde_json::to_value(content.metadata).ok()?,
        ToolDetailSection::Input => content.input,
        ToolDetailSection::Output => serde_json::to_value(display_tool_output(&content)).ok()?,
        ToolDetailSection::Presentation => project_tool_presentation(state, part)
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

pub(crate) async fn project_parts_for_user(state: &AppState, parts: &[Part]) -> Vec<PartResource> {
    let mut projected = Vec::with_capacity(parts.len());
    for part in parts {
        if part.visibility.visible_to_user() {
            projected.push(project_part_for_user(state, part).await);
        }
    }
    projected
}

/// Attach durable user-message ordinals to the user-send run markers in
/// `parts`, which must already be in chronological
/// `(created_at_ms, part_id)` order — the order every read path projects.
///
/// The ordinal of the first user marker present is resolved from the store;
/// every following marker advances by one. That is exact because each read
/// path returns a contiguous slice of the session's role-grouped blocks, so
/// the user markers it contains are themselves a contiguous slice of the
/// session's user-message sequence. Deriving from the durable order (rather
/// than a stored counter) is what keeps the numbering contiguous after
/// rewind, fork, compaction, import, and withdrawal.
pub(crate) async fn assign_user_message_ordinals(
    store: &dyn SessionStore,
    session_id: i64,
    parts: &mut [PartResource],
) -> Result<(), ServerError> {
    let marker_indices = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| is_user_message_marker(part))
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    let Some(&first) = marker_indices.first() else {
        return Ok(());
    };
    let start = store
        .user_message_ordinal(session_id, parts[first].part_id)
        .await
        .map_err(|error| ServerError::internal_error(&error))?;
    let mut ordinals = parts
        .iter()
        .map(|part| part.user_message_ordinal)
        .collect::<Vec<_>>();
    number_user_markers(&mut ordinals, &marker_indices, start);
    for (part, ordinal) in parts.iter_mut().zip(ordinals) {
        part.user_message_ordinal = ordinal;
    }
    Ok(())
}

/// Number the user markers at `marker_indices` (ascending) starting at
/// `start`, incrementing by one. Kept pure and index-based so the walk — the
/// only place a page could be silently mis-numbered — is unit-testable
/// without a store.
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

/// Project what a still-running process has produced so far as one Markdown
/// block. A tool that has not produced anything yet keeps the empty body it had
/// before, so nothing is invented for a quiet process.
fn running_live_output_blocks(content: &ToolCallContent) -> Vec<agena_domain::ViewBlock> {
    let mut blocks = Vec::new();
    // Invocation input is deliberately omitted from transcript content, but
    // the human projection must still show the command before any output
    // exists. Both clients already render Command as a fenced code surface.
    if content.name == "shell.run" {
        let command = match content.input.get("command") {
            Some(serde_json::Value::String(command)) => command.clone(),
            Some(serde_json::Value::Array(args)) => args
                .iter()
                .filter_map(serde_json::Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
            _ => String::new(),
        };
        if !command.is_empty() {
            blocks.push(agena_domain::ViewBlock::Command {
                id: Some("command".to_owned()),
                command,
                cwd: content
                    .input
                    .get("workdir")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned),
                exit_code: None,
                stdout: String::new(),
                stderr: String::new(),
            });
        }
    }
    if let Some(text) = content.live_output().filter(|text| !text.is_empty()) {
        blocks.push(agena_domain::ViewBlock::Markdown {
            id: Some("live-output".to_owned()),
            text: live_output_markdown(text),
        });
    }
    blocks
}

/// A lazy Output disclosure sees the same live tail as presentation. This is
/// a read-time projection only, never a fabricated terminal tool result.
fn display_tool_output(content: &ToolCallContent) -> Option<agena_domain::RawOutput> {
    content.output.clone().or_else(|| {
        matches!(
            content.state,
            agena_domain::ToolResultState::Pending | agena_domain::ToolResultState::Running
        )
        .then(|| content.live_output().map(agena_domain::RawOutput::text))
        .flatten()
    })
}

/// Fence what a running process has produced so far, matching the code surface
/// of its terminal result.
fn live_output_markdown(text: &str) -> String {
    let mut longest_fence = 0;
    let mut run = 0;
    for character in text.chars() {
        if character == '`' {
            run += 1;
            longest_fence = longest_fence.max(run);
        } else {
            run = 0;
        }
    }
    let fence = "`".repeat((longest_fence + 1).max(3));
    let separator = if text.ends_with('\n') { "" } else { "\n" };
    format!("{fence}text\n{text}{separator}{fence}\n")
}

#[cfg(test)]
mod live_output_tests {
    use super::{live_output_markdown, running_live_output_blocks};

    #[test]
    fn fences_what_a_still_running_process_produced() {
        assert_eq!(
            live_output_markdown("one\ntwo\n"),
            "```text\none\ntwo\n```\n"
        );
    }

    #[test]
    fn a_fence_inside_the_output_widens_the_block() {
        assert_eq!(
            live_output_markdown("echo ```x```"),
            "````text\necho ```x```\n````\n"
        );
    }

    #[test]
    fn a_running_process_shows_its_output_as_one_markdown_block() {
        let mut content = agena_runtime_contracts::part_content::ToolCallContent {
            name: "command".to_owned(),
            input: serde_json::json!({ "command": ["echo", "hi"] }),
            call_id: 1,
            state: agena_domain::ToolResultState::Running,
            ..Default::default()
        };
        assert!(
            running_live_output_blocks(&content).is_empty(),
            "a quiet process keeps the empty body"
        );

        content.set_live_output("one\ntwo\n");
        match running_live_output_blocks(&content).as_slice() {
            [agena_domain::ViewBlock::Markdown { text, .. }] => {
                assert_eq!(text.as_str(), "```text\none\ntwo\n```\n")
            }
            other => panic!("unexpected blocks: {other:?}"),
        }
    }

    #[test]
    fn shell_presentation_shows_the_command_before_output_and_preserves_whitespace() {
        let mut content = agena_runtime_contracts::part_content::ToolCallContent {
            name: "shell.run".to_owned(),
            input: serde_json::json!({"command":"printf 'hello'","workdir":"/repo"}),
            state: agena_domain::ToolResultState::Running,
            ..Default::default()
        };
        assert!(matches!(running_live_output_blocks(&content).as_slice(),
            [agena_domain::ViewBlock::Command { command, cwd: Some(cwd), .. }]
                if command == "printf 'hello'" && cwd == "/repo"));
        content.set_live_output("  indented\n\n");
        assert!(matches!(running_live_output_blocks(&content).as_slice(),
            [agena_domain::ViewBlock::Command { .. }, agena_domain::ViewBlock::Markdown { text, .. }]
                if text == "```text\n  indented\n\n```\n"));
        let output = super::display_tool_output(&content).unwrap();
        assert_eq!(output.text_content(), "  indented\n\n");
        content.state = agena_domain::ToolResultState::Completed;
        assert!(super::display_tool_output(&content).is_none());
    }
}

async fn project_tool_presentation(
    state: &AppState,
    part: &Part,
) -> Option<HumanPresentationResource> {
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
    // A process that is still running reports what it produced so far, so the
    // reader can watch the command instead of waiting for its result. Read it
    // before the fields below move into the invocation.
    let live_blocks = running_live_output_blocks(&content);
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
        return Some(HumanPresentationResource {
            title: agena_tool::tool_title_for_state(&invocation, content.state),
            summary: part.summary.clone().unwrap_or_default(),
            blocks: live_blocks,
        });
    };
    let projection = state
        .application()
        .render_tool_result(&invocation, &output)
        .await;
    let title = agena_tool::completed_tool_title_for_state(&invocation, content.state, &output);
    Some(HumanPresentationResource {
        title,
        summary: projection.human.summary,
        blocks: projection.human.blocks,
    })
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
    use agena_api::live::PartResource;

    /// The minimal `PartResource` the ordinal walk inspects.
    fn part(part_id: i64, kind: &str, role: &str) -> PartResource {
        PartResource {
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
