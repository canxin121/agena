use super::{ExecutionControlError, execution_control_to_app_error};
use crate::{
    AppError,
    session::{
        Session, SessionManager,
        store::{
            OPERATION_ID_METADATA_KEY, execution_status_from_part_state, parts_into_runs,
            role_from_part_role, timestamp_millis_to_utc, typed_content_from_value,
            typed_content_to_value,
        },
    },
};
use agena_domain::{ExecutionStatus, Role, SessionSummary};
use agena_runtime::{SessionForkRequest, SessionRewindRequest};
use agena_runtime_contracts::part_content::{
    TypedContent, attachment_from_file_ref, interaction_from_content, operation_from_tool_call,
    skill_reference_from_skill_ref, user_problem_from_error,
};
use agena_storage::store::{Part, PartRole};

impl SessionManager {
    pub async fn fork_session(&self, request: SessionForkRequest) -> Result<Session, AppError> {
        let source = self.store.load_session(request.session_id).await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        // The public request names a message (its run-marker part id), while
        // storage forks at an inclusive part boundary. Resolve a marker to the
        // message's final member part so the fork includes the entire message,
        // not only its marker. A literal part id that is not a message marker
        // remains a valid precise cutoff for internal callers.
        let at_part_id = match request.at_message_id {
            Some(part_id) => last_part_id_for_run_marker(source.parts(), part_id),
            None => last_part_id_for_last_run(source.parts()),
        }
        .ok_or_else(|| {
            AppError::Internal(format!(
                "cannot fork session {}: it has no message to use as the cutoff",
                request.session_id
            ))
        })?;
        let title = request
            .title
            .unwrap_or_else(|| format!("Fork of {}", source.title));
        let child_id = self.store.fork(source.id, at_part_id, title).await?;
        self.store.load_session(child_id).await
    }

    /// External entry: cancel the active execution for `session_id` and every
    /// active descendant. Descendants are cancelled deepest-first so a parent
    /// waiting on a delegated tool cannot keep its child alive.
    ///
    /// Cancellation is idempotent: a task can complete between the UI
    /// deciding to cancel and this call reaching the manager, so the absence
    /// of a control is a successful no-op rather than an error.
    pub async fn cancel_active_execution(&self, session_id: i64) -> Result<(), AppError> {
        // Signal the requested execution before any database traversal. This
        // keeps Ctrl+C latency independent of session-tree size and storage
        // contention; descendant discovery continues after the active model
        // stream or tool has already received cancellation.
        let root_result = self.execution_registry.cancel_current(session_id).await;
        self.cancel_host_interactive_waiters(session_id).await;
        let cancellation_order = match self.store.load_session(session_id).await {
            Ok(session) => {
                let tree = self.store.list_session_tree(session.root_id).await?;
                descendant_cancellation_order(session_id, session_tree_domain(tree)?.as_slice())
            }
            Err(_) => vec![session_id],
        };

        let mut first_error = cancel_active_execution_result(root_result).err();
        for target_id in cancellation_order
            .into_iter()
            .filter(|target_id| *target_id != session_id)
        {
            let result = self.execution_registry.cancel_current(target_id).await;
            // A plugin-hosted tool can be suspended in a host permission or
            // user-input callback. A cancellation token is only observed
            // between run-loop iterations, so release those one-shot waiters
            // as well; otherwise Ctrl+C leaves the executor blocked forever.
            self.cancel_host_interactive_waiters(target_id).await;
            if let Err(error) = cancel_active_execution_result(result)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    /// Exact external cancellation. Only after the observed root execution is
    /// matched do we cascade to its active descendants.
    pub async fn cancel_execution(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult, AppError> {
        let result = self
            .execution_registry
            .cancel_exact(session_id, execution_id)
            .await
            .map_err(execution_control_to_app_error)?;
        if result != agena_domain::CancellationResult::CancellationRequested {
            return Ok(result);
        }
        self.cancel_host_interactive_waiters(session_id).await;

        if let Ok(session) = self.store.load_session(session_id).await {
            let tree = self.store.list_session_tree(session.root_id).await?;
            for target_id in
                descendant_cancellation_order(session_id, session_tree_domain(tree)?.as_slice())
                    .into_iter()
                    .filter(|target_id| *target_id != session_id)
            {
                let _ = self.execution_registry.cancel_current(target_id).await;
                self.cancel_host_interactive_waiters(target_id).await;
            }
        }
        Ok(result)
    }

    /// External entry: inject `parts` as a steer message into the active
    /// execution for `session_id`. Returns `Err` if no execution is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<TypedContent>,
    ) -> Result<(), AppError> {
        self.execution_registry
            .steer(session_id, parts)
            .await
            .map_err(execution_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let source = self.store.load_session(request.session_id).await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let message_id = user_message_id_for_turn(&source, request.turn_id)?;
        let user_marker = source
            .parts()
            .iter()
            .find(|part| part.is_run_marker() && part.part_id == message_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "canonical turn {} has no projected user message in session {}",
                    request.turn_id, source.id
                ))
            })?;
        if !is_completed_user_rewind_target(user_marker) {
            return Err(AppError::Internal(format!(
                "rewind target must be a completed canonical user turn: {}",
                request.turn_id
            )));
        }
        let title = format!("Rewind of {}", source.title);
        let child_id = self.store.rewind(source.id, message_id, title).await?;
        self.store.load_session(child_id).await
    }

    /// Serialise `session_id` as a JSONL bundle (session header line followed
    /// by the session's ordered parts).
    pub async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.store.export_session_jsonl(session_id).await
    }

    /// Replay a JSONL bundle produced by [`Self::export_session_jsonl`] into
    /// this manager's workspace as a fresh session.
    pub async fn import_session_jsonl(&self, bundle: &str) -> Result<Session, AppError> {
        let workspace_id = self.current_workspace_id().await?;
        let session_id = self
            .store
            .import_session_jsonl(workspace_id, bundle)
            .await?;
        self.store.load_session(session_id).await
    }

    /// Return every session that shares the same `root_id`, ordered by
    /// `(depth, id)`. Useful for tree visualisation and bulk export.
    pub async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, AppError> {
        let summaries = self.store.list_session_tree(root_id).await?;
        summaries
            .into_iter()
            .map(crate::session::store::domain_summary_from_storage)
            .collect()
    }

    /// Snapshot the session transcript in canonical turn order.
    ///
    /// v2 has no separate event log or content-node projection: the canonical
    /// transcript is derived from the session aggregate rebuilt from parts.
    /// A turn is one completed (or in-flight) user message followed by every
    /// run-marker message that carries the turn's conversation id; the
    /// assistant content document is composed from the run's parts.
    pub async fn transcript_snapshot(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::TranscriptSnapshot, AppError> {
        let session = self.store.load_session(session_id).await?;
        transcript_snapshot_from_session(&session)
    }
}

/// Inclusive storage cutoff for a projected message named by its run-marker
/// part id. The marker is the message id; content parts follow it in canonical
/// `(created_at_ms, part_id)` order, so the final member with `run_id ==
/// marker_part_id` is the end of the message's shared prefix. A part id that
/// is not a run marker (a literal cutoff) passes through unchanged.
fn last_part_id_for_run_marker(parts: &[Part], marker_part_id: i64) -> Option<i64> {
    if !parts
        .iter()
        .any(|part| part.is_run_marker() && part.part_id == marker_part_id)
    {
        return Some(marker_part_id);
    }
    parts
        .iter()
        .rev()
        .find(|part| part.run_id == Some(marker_part_id))
        .map(|part| part.part_id)
        .or(Some(marker_part_id))
}

/// Inclusive storage cutoff for a session's final message: the id of the last
/// content part of the final run, or the marker id when that run is empty.
/// Sessions with no run markers fall back to the final part id (foreign data
/// may hold bare content parts).
fn last_part_id_for_last_run(parts: &[Part]) -> Option<i64> {
    match parts.iter().rev().find(|part| part.is_run_marker()) {
        Some(marker) => parts
            .iter()
            .rev()
            .find(|part| part.run_id == Some(marker.part_id))
            .map(|part| part.part_id)
            .or(Some(marker.part_id)),
        None => parts.last().map(|part| part.part_id),
    }
}

/// Resolve the canonical user message that owns `turn_id`.
///
/// Assistant run markers persist the conversation UUID pair on their content
/// (`turn_id`/`reply_id`), so the run that carries the turn id is the
/// assistant reply. The user input of the same canonical turn is the nearest
/// user-role run marker before that reply; user-run markers themselves do not
/// persist the UUID pair (they are written as `{"run_kind":"user_send"}`).
fn user_message_id_for_turn(
    session: &Session,
    turn_id: agena_domain::TurnId,
) -> Result<i64, AppError> {
    let parts = session.parts();
    let reply_index = parts
        .iter()
        .position(|part| {
            part.is_run_marker()
                && part
                    .content
                    .get("turn_id")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(agena_domain::TurnId)
                    == Some(turn_id)
        })
        .ok_or_else(|| {
            AppError::Internal(format!(
                "canonical turn not found in session {}: {turn_id}",
                session.id
            ))
        })?;
    parts[..reply_index]
        .iter()
        .rev()
        .find(|part| part.is_run_marker() && part.role == PartRole::User)
        .map(|part| part.part_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "canonical turn {} has no user message in session {}",
                turn_id, session.id
            ))
        })
}

/// Derive the full [`agena_domain::TranscriptSnapshot`] from the session
/// aggregate. A canonical turn is one user message (its created time anchors
/// the turn) followed by the assistant messages of the same canonical
/// conversation; one user turn can yield several assistant markers when the
/// reply is continued across multiple model runs.
fn transcript_snapshot_from_session(
    session: &Session,
) -> Result<agena_domain::TranscriptSnapshot, AppError> {
    let seq_session = session.version;
    let mut turns = Vec::new();
    let mut sequence = 0i64;
    let runs = parts_into_runs(session.parts());
    for (index, run) in runs.iter().enumerate() {
        let marker = run.first().expect("run group has a marker");
        if marker.role != PartRole::User {
            continue;
        }
        let user_document = content_document_from_run(session, run)?;
        let reply = assistant_reply_snapshot(session, index)?;
        let created_at_ms = marker.created_at_ms;
        let turn_id = reply_turn_id(marker, &reply);
        turns.push(agena_domain::TurnSnapshot {
            id: turn_id,
            session_id: session.id,
            sequence,
            input: user_document,
            reply,
            created_at_ms,
        });
        sequence += 1;
    }
    Ok(agena_domain::TranscriptSnapshot {
        session_id: session.id,
        seq_session,
        turns,
        // v2 has no session-scoped content nodes; the session aggregate
        // carries all activity content within its turn documents.
        session_activities: Vec::new(),
    })
}

/// The canonical turn id of a turn: the user marker's own persisted turn id
/// when present, otherwise the reply's turn id (recovered from the assistant
/// run marker).
fn reply_turn_id(
    user_marker: &Part,
    reply: &agena_domain::AssistantReplySnapshot,
) -> agena_domain::TurnId {
    user_marker
        .content
        .get("turn_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(agena_domain::TurnId)
        .unwrap_or(reply.turn_id)
}

/// Compose the assistant reply snapshot for the canonical turn beginning at
/// run `user_index`. The turn's assistant content is the concatenation of every
/// non-user run that follows the user run (a canonical reply can span several
/// continuation runs), in run order.
fn assistant_reply_snapshot(
    session: &Session,
    user_index: usize,
) -> Result<agena_domain::AssistantReplySnapshot, AppError> {
    let runs = parts_into_runs(session.parts());
    let user_marker = runs[user_index].first().expect("run group has a marker");
    let turn_id = user_marker
        .content
        .get("turn_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|value| uuid::Uuid::parse_str(value).ok())
        .map(agena_domain::TurnId)
        .unwrap_or_else(|| {
            // Reloaded user markers do not persist the UUID pair; recover the
            // canonical turn id from the first following non-user run marker.
            runs[user_index + 1..]
                .iter()
                .find(|run| run.first().map(|marker| marker.role) != Some(PartRole::User))
                .and_then(|run| run.first())
                .and_then(|marker| {
                    marker
                        .content
                        .get("turn_id")
                        .and_then(serde_json::Value::as_str)
                        .and_then(|value| uuid::Uuid::parse_str(value).ok())
                })
                .map(agena_domain::TurnId)
                .unwrap_or_else(agena_domain::TurnId::new)
        });
    let turn_span = canonical_turn_span(session, user_index);
    let (reply_id, status, transcript_nodes, created_at_ms, finished_at_ms, failure) =
        assistant_reply_fields(session, &turn_span)?;
    Ok(agena_domain::AssistantReplySnapshot {
        id: reply_id,
        turn_id,
        status,
        content: agena_domain::ContentDocument::new(transcript_nodes),
        revision_seq: session.version,
        created_at_ms,
        finished_at_ms,
        failure,
    })
}

/// Return the run-index span `[start, end)` that belongs to the canonical
/// turn beginning at run `user_index`: the user run and every following
/// non-user run until the next user run. Reloaded user markers do not persist
/// the conversation UUID pair, so adjacency is the reliable grouping.
fn canonical_turn_span(session: &Session, user_index: usize) -> std::ops::Range<usize> {
    let runs = parts_into_runs(session.parts());
    let mut end = user_index + 1;
    while end < runs.len() && runs[end].first().map(|marker| marker.role) != Some(PartRole::User) {
        end += 1;
    }
    user_index..end
}

/// Compute the reply identity fields by scanning the runs of one canonical
/// turn.
fn assistant_reply_fields(
    session: &Session,
    turn_span: &std::ops::Range<usize>,
) -> Result<
    (
        agena_domain::AssistantReplyId,
        agena_domain::AssistantReplyStatus,
        Vec<agena_domain::ContentNode>,
        i64,
        Option<i64>,
        Option<agena_failure::UserProblem>,
    ),
    AppError,
> {
    let mut reply_id: Option<agena_domain::AssistantReplyId> = None;
    let mut status = agena_domain::AssistantReplyStatus::Pending;
    let mut nodes = Vec::new();
    let mut created_at_ms = i64::MAX;
    let mut finished_at_ms: Option<i64> = None;
    let mut failure: Option<agena_failure::UserProblem> = None;
    let runs = parts_into_runs(session.parts());
    for run in &runs[turn_span.clone()] {
        let marker = run.first().expect("run group has a marker");
        if marker.role == PartRole::User {
            continue;
        }
        reply_id = reply_id.or(marker
            .content
            .get("reply_id")
            .and_then(serde_json::Value::as_str)
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(agena_domain::AssistantReplyId));
        let role = role_from_part_role(marker.role);
        for (index, part) in run.iter().enumerate().skip(1) {
            let decoded = decode_part(part, index as i32)?;
            if let Some(node) =
                transcript_node_from_part(session.version, &decoded, role, reply_id)?
            {
                nodes.push(node);
            }
            status = more_terminal_status(status, assistant_status_from_execution(decoded.status));
            created_at_ms = created_at_ms.min(decoded.created_at.timestamp_millis());
            if decoded.status.is_terminal() {
                finished_at_ms = Some(
                    finished_at_ms
                        .unwrap_or(decoded.created_at.timestamp_millis())
                        .max(decoded.created_at.timestamp_millis()),
                );
            }
            if let Some(problem) = part_failure(&decoded) {
                failure = Some(problem);
            }
        }
    }
    let reply_id = reply_id.unwrap_or_default();
    let created_at_ms = if created_at_ms == i64::MAX {
        0
    } else {
        created_at_ms
    };
    Ok((
        reply_id,
        status,
        nodes,
        created_at_ms,
        finished_at_ms,
        failure,
    ))
}

/// Fold two reply statuses, promoting toward the most terminal state.
fn more_terminal_status(
    current: agena_domain::AssistantReplyStatus,
    next: agena_domain::AssistantReplyStatus,
) -> agena_domain::AssistantReplyStatus {
    use agena_domain::AssistantReplyStatus as Status;
    match (current, next) {
        (Status::Failed, _) | (_, Status::Failed) => Status::Failed,
        (Status::Cancelled, _) | (_, Status::Cancelled) => Status::Cancelled,
        (Status::Completed, _) | (_, Status::Completed) => Status::Completed,
        (Status::InProgress, _) | (_, Status::InProgress) => Status::InProgress,
        _ => Status::Pending,
    }
}

/// The failure of a canonical turn, when any part reports one.
fn part_failure(part: &DecodedPart) -> Option<agena_failure::UserProblem> {
    match part.content.as_ref()? {
        TypedContent::ToolCall(tool_call) => operation_from_tool_call(tool_call)
            .error
            .as_ref()
            .map(|error| (&error.failure).into()),
        TypedContent::Error(error) => Some(user_problem_from_error(error)),
        _ => None,
    }
}

/// Compose the user input [`agena_domain::ContentDocument`] for one canonical
/// turn. The user run's content parts render as text/activity nodes; when the
/// run carries no content parts (the persisted user marker holds only its run
/// metadata), the document is empty.
fn content_document_from_run(
    session: &Session,
    run: &[Part],
) -> Result<agena_domain::ContentDocument, AppError> {
    let marker = run.first().expect("run group has a marker");
    let role = role_from_part_role(marker.role);
    let mut nodes = Vec::new();
    for (index, part) in run.iter().enumerate().skip(1) {
        let decoded = decode_part(part, index as i32)?;
        if let Some(node) = transcript_node_from_part(session.version, &decoded, role, None)? {
            nodes.push(node);
        }
    }
    Ok(agena_domain::ContentDocument::new(nodes))
}

/// Project one decoded content part into a transcript [`agena_domain::ContentNode`].
///
/// Mirrors the live patch mapping: text parts become text segments, activity
/// parts become activity nodes carrying the durable payload with the
/// human-facing operation detail derived on load.
fn transcript_node_from_part(
    revision_seq: i64,
    part: &DecodedPart,
    role: Role,
    reply_id: Option<agena_domain::AssistantReplyId>,
) -> Result<Option<agena_domain::ContentNode>, AppError> {
    let position = u32::try_from(part.part_index).unwrap_or_default();
    if let Some(activity_id) = part.activity_id {
        let Some(payload) = activity_payload_from_part(part, role)? else {
            return Ok(None);
        };
        let state = activity_state_from_execution(part.status);
        let finished_at_ms = state
            .is_terminal()
            .then_some(part.created_at.timestamp_millis());
        Ok(Some(agena_domain::ContentNode::activity(
            agena_domain::ActivityNode {
                id: activity_id,
                owner: agena_domain::ActivityOwner::AssistantReply {
                    reply_id: reply_id.unwrap_or_default(),
                },
                actor: activity_actor_from_role(role),
                payload,
                state,
                position: agena_domain::ContentPosition { index: position },
                revision_seq,
                lifecycle: agena_domain::ActivityLifecycle {
                    started_at_ms: part.created_at.timestamp_millis(),
                    finished_at_ms,
                },
                provenance: Default::default(),
            },
        )))
    } else if let Some(segment_id) = part.segment_id {
        let Some(content) = part.content.as_ref() else {
            return Ok(None);
        };
        let TypedContent::Text(text) = content else {
            return Ok(None);
        };
        Ok(Some(agena_domain::ContentNode::text_at(
            segment_id,
            text.text.clone(),
            position,
            revision_seq,
        )))
    } else {
        Ok(None)
    }
}

/// The coarse transcript activity state for a part execution status.
fn activity_state_from_execution(status: ExecutionStatus) -> agena_domain::ActivityState {
    match status {
        ExecutionStatus::Pending => agena_domain::ActivityState::Pending,
        ExecutionStatus::InProgress => agena_domain::ActivityState::InProgress,
        ExecutionStatus::Completed => agena_domain::ActivityState::Completed,
        // ActivityState is intentionally coarse; the transcript part and
        // tool-result envelope preserve the precise non-execution reason.
        ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => agena_domain::ActivityState::Completed,
        ExecutionStatus::Failed => agena_domain::ActivityState::Failed,
        ExecutionStatus::Cancelled => agena_domain::ActivityState::Cancelled,
    }
}

/// The coarse reply status for a part execution status (terminal wins).
fn assistant_status_from_execution(status: ExecutionStatus) -> agena_domain::AssistantReplyStatus {
    match status {
        ExecutionStatus::Pending => agena_domain::AssistantReplyStatus::Pending,
        ExecutionStatus::InProgress => agena_domain::AssistantReplyStatus::InProgress,
        ExecutionStatus::Completed
        | ExecutionStatus::PolicyDenied
        | ExecutionStatus::UserDeclined
        | ExecutionStatus::CapabilityUnavailable
        | ExecutionStatus::ToolUnavailable => agena_domain::AssistantReplyStatus::Completed,
        ExecutionStatus::Failed => agena_domain::AssistantReplyStatus::Failed,
        ExecutionStatus::Cancelled => agena_domain::AssistantReplyStatus::Cancelled,
    }
}

/// The transcript actor for a message role.
fn activity_actor_from_role(role: Role) -> agena_domain::ActivityActor {
    match role {
        Role::User => agena_domain::ActivityActor::User,
        Role::Assistant => agena_domain::ActivityActor::Assistant,
        Role::Tool => agena_domain::ActivityActor::Tool,
        Role::System => agena_domain::ActivityActor::Runtime,
    }
}

/// Map a message part's content into the transcript [`agena_domain::ActivityPayload`].
///
/// v2 persists parts in their rich typed form (there is no separate
/// content-node projection), so the durable payload is recovered directly from
/// the part. Operation parts carry their compact tool data; the human-facing
/// detail Markdown is derived here, at snapshot load, and never persisted.
fn activity_payload_from_part(
    part: &DecodedPart,
    role: Role,
) -> Result<Option<agena_domain::ActivityPayload>, AppError> {
    use agena_domain::{
        ActivityPayload, ErrorActivity, InteractionActivity, NoticeActivity, OperationActivity,
        OperationActivityError, ReasoningActivity, ResourceActivity, ResourceKind,
        ResourceReference, SkillReferenceActivity, TextArtifactActivity, TextSegmentActivity,
        ToolCallId,
    };
    let content = part.content.as_ref();
    let Some(payload) = (match content {
        // Assistant text parts that carry an ActivityId are interstitial body
        // segments (produced between tool calls); they render as their own
        // collapsible block. User text activities stay TextArtifact.
        Some(TypedContent::Text(text)) => match role {
            Role::Assistant => Some(ActivityPayload::TextSegment(TextSegmentActivity {
                text: text.text.clone(),
            })),
            _ => Some(ActivityPayload::TextArtifact(TextArtifactActivity {
                text: text.text.clone(),
                language: None,
                label: part.summary.clone(),
            })),
        },
        Some(TypedContent::Think(think)) => Some(ActivityPayload::Reasoning(ReasoningActivity {
            content: crate::session::store::reasoning_from_think(think),
        })),
        Some(TypedContent::ToolCall(tool_call)) => {
            let operation = operation_from_tool_call(tool_call);
            // The compact `ToolResult` payload is the only durable tool data.
            // The human-facing detail Markdown is derived from it at render
            // time and is never persisted.
            let data = operation
                .details
                .to_json_payload()
                .unwrap_or(serde_json::Value::Null);
            Some(ActivityPayload::Operation(OperationActivity {
                call_id: ToolCallId::new(
                    part.operation_id
                        .clone()
                        .unwrap_or_else(|| operation.call_id.to_string()),
                ),
                invocation: operation.invocation.clone(),
                title: operation.title.clone(),
                summary: operation.summary.clone(),
                data,
                // The derived projection carries no detail Markdown; it is
                // derived at snapshot load / lazy detail fetch time.
                markdown: String::new(),
                authorization: operation.authorization.clone(),
                error: operation
                    .error
                    .as_ref()
                    .map(|error| OperationActivityError {
                        problem: (&error.failure).into(),
                    }),
            }))
        }
        Some(TypedContent::FileRef(file_ref)) => {
            let attachment = attachment_from_file_ref(file_ref);
            let Some(item) = attachment.attachments.first() else {
                return Ok(None);
            };
            let kind = match item.kind {
                crate::part::AttachmentKind::Image => ResourceKind::Image,
                crate::part::AttachmentKind::Audio => ResourceKind::Audio,
                crate::part::AttachmentKind::Video => ResourceKind::Video,
                crate::part::AttachmentKind::Pdf => ResourceKind::Pdf,
                crate::part::AttachmentKind::File if item.mime == "inode/directory" => {
                    ResourceKind::Directory
                }
                crate::part::AttachmentKind::File => ResourceKind::File,
            };
            let reference = match &item.source {
                crate::part::AttachmentSource::Url { url } => {
                    Some(ResourceReference::Url { url: url.clone() })
                }
                crate::part::AttachmentSource::FileId { file_id } => {
                    Some(ResourceReference::ProviderFile {
                        provider_id: "provider".to_owned(),
                        file_id: file_id.clone(),
                    })
                }
                crate::part::AttachmentSource::LocalPath { path } => {
                    Some(ResourceReference::WorkspacePath { path: path.clone() })
                }
                crate::part::AttachmentSource::DataUrl { .. }
                | crate::part::AttachmentSource::Base64 { .. } => None,
            };
            let Some(reference) = reference else {
                return Ok(None);
            };
            Some(ActivityPayload::Resource(ResourceActivity {
                kind,
                reference,
                name: item.summary_label(),
                media_type: (!item.mime.is_empty()).then(|| item.mime.clone()),
                size_bytes: item.size_bytes,
                width: item.width,
                height: item.height,
                duration_ms: item.duration_ms,
                page_count: item.page_count,
            }))
        }
        Some(TypedContent::SkillRef(skill_ref)) => {
            let skills = skill_reference_from_skill_ref(skill_ref);
            let Some(skill) = skills.skills.first() else {
                return Ok(None);
            };
            Some(ActivityPayload::SkillReference(SkillReferenceActivity {
                name: skill.name.clone(),
                description: skill.description.clone(),
                instructions: skill.instructions.clone(),
                content_hash: skill.content_hash.clone(),
                source: skill.source.clone(),
                aliases: skill.aliases.clone(),
            }))
        }
        Some(TypedContent::Interaction(interaction)) => Some(ActivityPayload::Interaction(
            match interaction_from_content(interaction) {
                crate::part::RequestPart::UserInput(value) => InteractionActivity::UserInput {
                    request: value.request.clone(),
                    reply: value.reply.clone(),
                },
            },
        )),
        Some(TypedContent::Error(error)) => Some(ActivityPayload::Error(ErrorActivity {
            problem: user_problem_from_error(error),
        })),
        Some(TypedContent::Hook(hook)) => Some(ActivityPayload::Notice(NoticeActivity {
            kind: "hook".to_owned(),
            summary: hook.summary.clone(),
            detail: hook
                .message
                .as_deref()
                .filter(|text| !text.trim().is_empty())
                .map(str::to_owned)
                .or_else(|| hook.detail.clone()),
            occurred_at_ms: None,
            title: None,
        })),
        Some(TypedContent::Notice(notice)) => Some(ActivityPayload::Notice(NoticeActivity {
            kind: notice.kind.clone(),
            summary: notice.summary.clone(),
            detail: notice.detail.clone(),
            occurred_at_ms: None,
            title: notice.title.clone(),
        })),
        Some(
            TypedContent::Run(_)
            | TypedContent::ToolResult(_)
            | TypedContent::PasteRef(_)
            | TypedContent::Compaction(_),
        ) => None,
        None => None,
    }) else {
        return Ok(None);
    };
    Ok(Some(payload))
}

/// Recover every interactive request currently awaiting a reply in `session`,
/// from the parts projection (v2 equivalent of the removed
/// `Session::pending_interactive_requests`): unanswered permissions recorded on
/// in-flight `tool_call` parts, plus unanswered user-input `interaction` parts.
/// Requests are de-duplicated by `(kind, request_id)` as the v1 projection did.
fn pending_interactive_requests_from_session(
    session: &Session,
) -> Vec<agena_domain::PendingInteractiveRequest> {
    let mut seen = std::collections::HashSet::new();
    let mut requests = Vec::new();
    // Pending permissions live on the in-flight tool-call part's operation
    // authorization record (`operation.authorization.awaiting()`).
    for part in session.parts() {
        if part.kind != "tool_call" || !part.state.is_in_flight() {
            continue;
        }
        let Some(operation) = super::replies::operation_from_part(part) else {
            continue;
        };
        for permission in operation.authorization.awaiting() {
            let request = agena_domain::PendingInteractiveRequest::from(permission.request.clone());
            if seen.insert(format!("{:?}:{}", request.kind(), request.request_id())) {
                requests.push(request);
            }
        }
    }
    // Pending user-input requests live on the in-flight tool-call part's
    // operation user-input records (`operation.user_input.awaiting()`), and,
    // for legacy data, as separate in-flight `interaction` parts.
    for part in session.pending_interactions() {
        if part.kind == "tool_call" {
            let Some(operation) = super::replies::operation_from_part(part) else {
                continue;
            };
            for record in operation.user_input.awaiting() {
                let request =
                    agena_domain::PendingInteractiveRequest::from(record.request.clone());
                if seen.insert(format!("{:?}:{}", request.kind(), request.request_id())) {
                    requests.push(request);
                }
            }
            continue;
        }
        let Some(request) = agena_runtime_contracts::part_content::InteractionContent::try_from(
            &part.content,
        )
        .ok()
        .and_then(|interaction| interaction.request())
        else {
            continue;
        };
        let request = agena_domain::PendingInteractiveRequest::from(request);
        if seen.insert(format!("{:?}:{}", request.kind(), request.request_id())) {
            requests.push(request);
        }
    }
    requests
}

#[async_trait::async_trait]
impl agena_runtime::SessionQueryService for SessionManager {
    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<agena_domain::SessionSummary>, agena_runtime::SessionQueryError> {
        self.list_session_summaries(request)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionPresentation, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let workflow_state = session.workflow_state();
        let message_count = session
            .parts()
            .iter()
            .filter(|part| part.is_run_marker())
            .count();
        Ok(agena_runtime::SessionPresentation {
            id: session.id,
            parent_id: session.parent_id,
            workspace_id: session.workspace_id,
            title: session.title,
            version: session.version,
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count,
            workflow_state,
        })
    }

    async fn transcript_snapshot(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::TranscriptSnapshot, agena_runtime::SessionQueryError> {
        SessionManager::transcript_snapshot(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn operation_detail(
        &self,
        session_id: i64,
        activity_id: agena_domain::ActivityId,
    ) -> Result<Option<agena_runtime::OperationDetail>, agena_runtime::SessionQueryError> {
        // Load the snapshot (which derives detail Markdown into Operation
        // Activities on load) and locate the requested Activity.
        let snapshot = SessionManager::transcript_snapshot(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let mut found: Option<agena_domain::ActivityNode> = None;
        for turn in &snapshot.turns {
            for node in turn.input.nodes() {
                if let agena_domain::ContentNode::Activity { activity } = node
                    && activity.id == activity_id
                {
                    found = Some(activity.as_ref().clone());
                }
            }
            for node in turn.reply.content.nodes() {
                if let agena_domain::ContentNode::Activity { activity } = node
                    && activity.id == activity_id
                {
                    found = Some(activity.as_ref().clone());
                }
            }
        }
        for activity in &snapshot.session_activities {
            if activity.id == activity_id {
                found = Some(activity.clone());
            }
        }
        let activity = match found {
            Some(activity) => activity,
            None => return Ok(None),
        };
        let agena_domain::ActivityPayload::Operation(operation) = &activity.payload else {
            return Ok(None);
        };
        // If the snapshot did not pre-derive the detail (e.g. a live in-memory
        // node), derive it now from the compact data.
        let markdown = if !operation.markdown.is_empty() {
            operation.markdown.clone()
        } else if !operation.data.is_null() {
            let command = operation
                .invocation
                .input
                .get("command")
                .and_then(|value| value.as_text())
                .map(str::to_owned);
            crate::session::manager::helpers::derive_operation_markdown(
                &operation.invocation.name,
                &operation.data,
                command.as_deref(),
            )
        } else {
            String::new()
        };
        let streaming = activity.state == agena_domain::ActivityState::InProgress;
        Ok(Some(agena_runtime::OperationDetail {
            activity_id,
            markdown,
            streaming,
        }))
    }

    async fn list_projected_runs(
        &self,
        session_id: i64,
        include_content: bool,
    ) -> Result<Vec<agena_runtime::SessionProjectedRun>, agena_runtime::SessionQueryError> {
        // Parts-native: `SessionManager::list_projected_runs` already
        // builds the stable `SessionProjectedRun` values directly from the
        // session's parts; only the error type needs adapting here.
        SessionManager::list_projected_runs(self, session_id, include_content)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, agena_runtime::SessionQueryError> {
        SessionManager::list_session_tree(self, root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn export_session_jsonl(
        &self,
        session_id: i64,
    ) -> Result<String, agena_runtime::SessionQueryError> {
        SessionManager::export_session_jsonl(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn latest_event_seq(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        // v2 has no event log: the session's optimistic-lock version is the
        // monotonic per-session change sequence that consumers treat as the
        // durable high-water mark.
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        Ok(Some(session.version))
    }

    async fn session_usage(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionUsage, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        SessionManager::session_usage_async(self, &session)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionCostSummary, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        Ok(crate::session::cost::summarize(session.parts()))
    }

    async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, agena_runtime::SessionQueryError> {
        SessionManager::usage_stats(self, query)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))
    }

    async fn pending_interactive_requests(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_domain::PendingInteractiveRequestContext>, agena_runtime::SessionQueryError>
    {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, session.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let mut descendants = std::collections::HashSet::from([session.id]);
        loop {
            let previous_len = descendants.len();
            for summary in &tree {
                if summary
                    .parent_id
                    .is_some_and(|parent_id| descendants.contains(&parent_id))
                {
                    descendants.insert(summary.id);
                }
            }
            if descendants.len() == previous_len {
                break;
            }
        }

        let mut sessions = vec![session];
        for summary in tree {
            if summary.id == session_id
                || !descendants.contains(&summary.id)
                || SessionManager::active_execution(self, summary.id)
                    .await
                    .is_none()
            {
                continue;
            }
            sessions.push(
                SessionManager::get_session(self, summary.id)
                    .await
                    .map_err(|error| {
                        agena_runtime::SessionQueryError::internal(error.to_string())
                    })?,
            );
        }

        Ok(sessions
            .into_iter()
            .flat_map(|pending_session| {
                let session_id = pending_session.id;
                let parent_session_id = pending_session.parent_id;
                let task_id = pending_session.task_id.clone();
                pending_interactive_requests_from_session(&pending_session)
                    .into_iter()
                    .map(
                        move |request| agena_domain::PendingInteractiveRequestContext {
                            session_id,
                            parent_session_id,
                            task_id: task_id.clone(),
                            request,
                        },
                    )
            })
            .collect())
    }

    async fn execution_context(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionExecutionContext, agena_runtime::SessionQueryError> {
        let mut session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let state = self.execution_state();
        self.refresh_execution_policy(&mut session, state.as_ref());
        let runtime = session.runtime();
        Ok(agena_runtime::SessionExecutionContext {
            workflow_state: session.workflow_state(),
            agent_id: crate::identity::AGENA_AGENT_ID.to_string(),
            execution_access: runtime.execution.access,
            selected_permission: runtime.execution.selection.permission.clone(),
            effective_permission: runtime.execution.effective_permission.clone(),
            permission_ceiling: runtime.execution.permission_ceiling.clone(),
            model_provider_id: runtime.execution.selection.provider.clone(),
            model_adapter_id: runtime.execution.selection.adapter.clone(),
            model_id: runtime.execution.selection.model.clone(),
            model_thinking_mode: runtime.execution.selection.thinking_mode.clone(),
            model_speed_mode: runtime.execution.selection.speed_mode.clone(),
            model_verbosity: runtime.execution.selection.verbosity.clone(),
            model_parallel_tool_calls: runtime.execution.selection.parallel_tool_calls,
            effective_workspace_root: runtime
                .effective_workspace_root()
                .map(|path| path.display().to_string()),
            task_id: session.task_id.clone(),
            subtask_status: session.is_subagent().then_some(runtime.subtask.status),
            subtask_started_at: runtime
                .subtask
                .started_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_finished_at: runtime
                .subtask
                .finished_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_failure: runtime.subtask.failure.clone(),
        })
    }

    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, agena_runtime::SessionQueryError> {
        let descendant = SessionManager::get_session(self, descendant_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let tree = SessionManager::list_session_tree(self, descendant.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal(error.to_string()))?;
        let parents = tree
            .into_iter()
            .map(|summary| (summary.id, summary.parent_id))
            .collect::<std::collections::HashMap<_, _>>();
        let mut cursor = parents.get(&descendant_id).copied().flatten();
        let mut visited = std::collections::HashSet::new();
        while let Some(session_id) = cursor {
            if !visited.insert(session_id) {
                return Ok(false);
            }
            if session_id == ancestor_id {
                return Ok(true);
            }
            cursor = parents.get(&session_id).copied().flatten();
        }
        Ok(false)
    }
}

/// Project a session's parts into the stable transcript values, one per run.
///
/// Each run marker becomes a `SessionProjectedRun` whose parts are the
/// run's content parts, decoded from the canonical store payload. This is the
/// parts-native replacement for the removed v1 message bridge,
/// preserving the exact wire shape `agena-application` / `agena-cli` consume.
pub(crate) fn projected_runs_from_parts(
    parts: &[Part],
) -> Result<Vec<crate::session_query_service::SessionProjectedRun>, AppError> {
    parts_into_runs(parts)
        .into_iter()
        .map(|run| {
            let marker = run.first().expect("run group has a marker");
            let mut projected_parts = Vec::with_capacity(run.len().saturating_sub(1));
            for (index, part) in run.iter().enumerate().skip(1) {
                projected_parts.push(project_storage_part(part, marker.part_id, index as i32)?);
            }
            Ok(crate::session_query_service::SessionProjectedRun {
                id: marker.part_id,
                role: role_from_part_role(marker.role),
                state: execution_status_from_part_state(marker.state),
                created_at: timestamp_millis_to_utc(marker.created_at_ms)?,
                // The run marker's content is the durable header payload; the
                // v1 bridge surfaced a derived `MessageMetadata` projection of
                // the same fields, and consumers pass it through as the run
                // part's content.
                metadata: marker.content.clone(),
                usage: None,
                parts: projected_parts,
            })
        })
        .collect()
}

/// A decoded content part: the parts-native projection of one storage [`Part`]
/// into the fields the transcript and query surfaces need, mirroring the v1
/// `MessagePart` reconstruction without materializing v1 messages.
struct DecodedPart {
    id: i64,
    part_index: i32,
    status: ExecutionStatus,
    kind: String,
    name: Option<String>,
    summary: Option<String>,
    has_detail: bool,
    activity_id: Option<agena_domain::ActivityId>,
    segment_id: Option<agena_domain::TextSegmentId>,
    operation_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    content: Option<TypedContent>,
}

/// Decode one persisted content part (its `kind` column plus canonical JSON
/// payload) into the [`DecodedPart`] view. Mirrors the removed
/// `store::part_to_message_part` exactly so the transcript and projected-message
/// surfaces keep the same shape without the v1 message bridge.
fn decode_part(part: &Part, part_index: i32) -> Result<DecodedPart, AppError> {
    let content = typed_content_from_value(&part.kind, &part.content)?;
    // The coarse state column carries the lifecycle; the fine-grained status
    // (including denial outcomes) is reconstructed from the rich content.
    let status = match &content {
        TypedContent::ToolCall(tool_call) => operation_from_tool_call(tool_call).status(),
        TypedContent::Interaction(interaction) => match interaction_from_content(interaction) {
            crate::part::RequestPart::UserInput(request) => request.status(),
        },
        _ => execution_status_from_part_state(part.state),
    };
    // Recover the provider operation id stashed by the tool-call serialization
    // so pending-tool correlation and prompt assembly survive a reload.
    let operation_id = match &content {
        TypedContent::ToolCall(tool_call) => operation_from_tool_call(tool_call)
            .metadata
            .get(OPERATION_ID_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned),
        _ => None,
    };
    let activity_kind = matches!(
        &content,
        TypedContent::Think(_)
            | TypedContent::ToolCall(_)
            | TypedContent::FileRef(_)
            | TypedContent::SkillRef(_)
            | TypedContent::Notice(_)
            | TypedContent::Hook(_)
            | TypedContent::Error(_)
            | TypedContent::Interaction(_)
    );
    Ok(DecodedPart {
        id: part.part_id,
        part_index,
        status,
        // Carry the precise storage kind through to the transcript surfaces
        // (the v1 `PartKind` binary collapsed every non-text part to
        // "activity", breaking `think`/`tool_call`/... rendering dispatch).
        kind: part.kind.clone(),
        name: part_name_from_content(&content),
        summary: part.summary.clone(),
        has_detail: part.content.is_object(),
        activity_id: activity_kind.then(agena_domain::ActivityId::new),
        segment_id: matches!(&content, TypedContent::Text(_))
            .then(agena_domain::TextSegmentId::new),
        operation_id,
        created_at: timestamp_millis_to_utc(part.created_at_ms)?,
        content: Some(content),
    })
}

/// The `MessagePart::name` derivation from decoded content: text/reasoning
/// are plain labels, operations use their header title (falling back to the
/// invocation name), and failures use their problem code.
fn part_name_from_content(content: &TypedContent) -> Option<String> {
    match content {
        TypedContent::Text(_) => Some("text".to_string()),
        TypedContent::Think(_) => Some("reasoning".to_string()),
        TypedContent::ToolCall(tool_call) => {
            let operation = operation_from_tool_call(tool_call);
            let title = operation.title.trim();
            Some(if title.is_empty() {
                operation.invocation.name.clone()
            } else {
                title.to_owned()
            })
        }
        TypedContent::SkillRef(_) => Some("skill_reference".to_string()),
        TypedContent::Error(error) => Some(user_problem_from_error(error).code.to_string()),
        TypedContent::FileRef(_) => Some("resource".to_string()),
        TypedContent::Interaction(interaction) => match interaction_from_content(interaction) {
            crate::part::RequestPart::UserInput(_) => Some("user_input".to_string()),
        },
        TypedContent::Hook(hook) => Some(format!("hook:{}", hook.hook)),
        TypedContent::Notice(_) => Some("notice".to_string()),
        TypedContent::Run(_)
        | TypedContent::PasteRef(_)
        | TypedContent::ToolResult(_)
        | TypedContent::Compaction(_) => None,
    }
}

/// Project one persisted content part into the stable transcript part value,
/// preserving the wire shape the v1 bridge produced for `MessagePart`.
fn project_storage_part(
    part: &Part,
    run_id: i64,
    part_index: i32,
) -> Result<agena_runtime::SessionProjectedPart, AppError> {
    let decoded = decode_part(part, part_index)?;
    Ok(agena_runtime::SessionProjectedPart {
        id: decoded.id,
        run_id,
        part_index: decoded.part_index,
        status: decoded.status,
        kind: decoded.kind,
        name: decoded.name,
        summary: decoded.summary,
        has_detail: decoded.has_detail,
        activity_id: decoded.activity_id,
        segment_id: decoded.segment_id,
        operation_id: decoded.operation_id,
        created_at: decoded.created_at,
        detail: decoded.content.as_ref().map(project_part_detail),
        content: decoded
            .content
            .as_ref()
            .map(|content| {
                typed_content_to_value(content)
                    .map_err(|error| AppError::Internal(format!("serialize part content: {error}")))
            })
            .transpose()?,
    })
}

fn project_part_detail(content: &TypedContent) -> agena_runtime::SessionProjectedPartDetail {
    match content {
        TypedContent::Text(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: value.synthetic,
        },
        TypedContent::Think(value) => agena_runtime::SessionProjectedPartDetail::Reasoning {
            summary: value.summary.clone(),
            raw_content: value.raw.clone(),
            encrypted_content: value.encrypted_content.clone(),
        },
        TypedContent::Error(value) => agena_runtime::SessionProjectedPartDetail::Error {
            problem: user_problem_from_error(value),
        },
        TypedContent::FileRef(value) => {
            agena_runtime::SessionProjectedPartDetail::Attachment(attachment_from_file_ref(value))
        }
        TypedContent::SkillRef(value) => agena_runtime::SessionProjectedPartDetail::SkillReference(
            skill_reference_from_skill_ref(value),
        ),
        TypedContent::Interaction(value) => match interaction_from_content(value) {
            crate::part::RequestPart::UserInput(request) => {
                agena_runtime::SessionProjectedPartDetail::UserInputRequest {
                    request: request.request.clone(),
                    reply: request.reply.clone(),
                }
            }
        },
        TypedContent::ToolCall(value) => agena_runtime::SessionProjectedPartDetail::Operation(
            Box::new(project_operation_part(&operation_from_tool_call(value))),
        ),
        TypedContent::Hook(value) => agena_runtime::SessionProjectedPartDetail::Hook(Box::new(
            agena_runtime::SessionProjectedHookPart {
                hook: value.hook.clone(),
                plugin_id: value.plugin_id.clone(),
                summary: value.summary.clone(),
                detail: value.detail.clone(),
                message: value.message.clone(),
            },
        )),
        TypedContent::Notice(value) => agena_runtime::SessionProjectedPartDetail::Notice {
            summary: value.summary.clone(),
            detail: value.detail.clone(),
        },
        // These kinds have no v1 rich projection; degrade to a text detail
        // exactly as the v1 typed fold did (Run → empty, PasteRef/ToolResult/
        // Compaction → their text).
        TypedContent::Run(_) => agena_runtime::SessionProjectedPartDetail::Text {
            text: String::new(),
            synthetic: false,
        },
        TypedContent::PasteRef(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: false,
        },
        TypedContent::ToolResult(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.output.clone(),
            synthetic: false,
        },
        TypedContent::Compaction(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.summary.clone().unwrap_or_default(),
            synthetic: false,
        },
    }
}

fn project_operation_part(
    value: &crate::part::OperationPart,
) -> agena_runtime::SessionProjectedOperationPart {
    let details = value.details.clone();
    agena_runtime::SessionProjectedOperationPart {
        call_id: value.call_id,
        invocation: value.invocation.clone(),
        authorization: value.authorization.clone(),
        user_input: value.user_input.clone(),
        title: value.title.clone(),
        summary: value.summary.clone(),
        model_output: project_model_visible_output(&value.result.model_preview),
        blocks: value
            .result
            .content
            .iter()
            .map(project_operation_block)
            .collect(),
        artifacts: value.artifacts.clone(),
        attachments: value.result.attachments.clone(),
        details,
        result: agena_runtime::SessionProjectedToolResult {
            state: value.result.state,
            structured: value.result.structured.clone(),
            content: value
                .result
                .content
                .iter()
                .map(project_operation_block)
                .collect(),
            model_preview: project_model_visible_output(&value.result.model_preview),
            managed_outputs: value.result.managed_outputs.clone(),
            display: value.result.display.clone(),
            attachments: value.result.attachments.clone(),
            error: value.result.error.clone(),
            metadata: value.result.metadata.clone(),
            raw: value.result.raw.clone(),
        },
        structured: value.result.structured.clone(),
        metadata: value.metadata.clone(),
        error: value.error.clone(),
        raw: value.raw.clone(),
        lifecycle: value.lifecycle.clone(),
    }
}

fn project_model_visible_output(
    value: &crate::part::ModelVisibleOutput,
) -> agena_runtime::SessionProjectedModelVisibleOutput {
    agena_runtime::SessionProjectedModelVisibleOutput {
        text: value.text.clone(),
        attachments: value.attachments.clone(),
        truncated: value.truncated,
    }
}

fn project_operation_block(
    value: &agena_domain::ViewBlock,
) -> agena_runtime::SessionProjectedOperationBlock {
    use agena_runtime::SessionProjectedOperationBlock as Projected;
    match value {
        agena_domain::ViewBlock::Text { text, .. } => Projected::Text { text: text.clone() },
        agena_domain::ViewBlock::Markdown { text, .. } => {
            Projected::Markdown { text: text.clone() }
        }
        agena_domain::ViewBlock::Json { value, .. } => Projected::Json {
            value: value.clone(),
        },
        agena_domain::ViewBlock::Table { columns, rows, .. } => Projected::Table {
            columns: columns
                .iter()
                .map(|label| agena_domain::TableColumn {
                    key: label.clone(),
                    label: Some(label.clone()),
                })
                .collect(),
            rows: rows.clone(),
        },
        agena_domain::ViewBlock::Log { stream, text, .. } => Projected::Log {
            stream: Some(match stream {
                agena_domain::CommandOutputStream::Stdout => "stdout".to_string(),
                agena_domain::CommandOutputStream::Stderr => "stderr".to_string(),
            }),
            text: text.clone(),
        },
        agena_domain::ViewBlock::Command {
            command,
            cwd,
            exit_code,
            stdout,
            stderr,
            ..
        } => Projected::Command {
            command: command.clone(),
            cwd: cwd.clone(),
            exit_code: *exit_code,
            stdout: Some(stdout.clone()),
            stderr: Some(stderr.clone()),
        },
        agena_domain::ViewBlock::Diff { diff, language, .. } => Projected::Diff {
            diff: diff.clone(),
            language: language.clone(),
        },
        agena_domain::ViewBlock::FileChanges { changes, .. } => Projected::FileChanges {
            changes: changes.clone(),
        },
        agena_domain::ViewBlock::SearchResults { items, .. } => Projected::SearchResults {
            query: None,
            results: items
                .iter()
                .map(|item| agena_domain::SearchResultItem {
                    title: item.title.clone(),
                    uri: item.url.clone(),
                    snippet: item.snippet.clone(),
                    score: None,
                })
                .collect(),
        },
        agena_domain::ViewBlock::Media { artifact, .. } => Projected::Media {
            mime_type: artifact.mime.clone(),
            artifact: artifact.clone(),
        },
        agena_domain::ViewBlock::Custom {
            kind,
            schema,
            presentation,
            ..
        } => Projected::Custom {
            schema: if schema.is_null() {
                None
            } else {
                Some(schema.to_string())
            },
            value: serde_json::json!({ "kind": kind, "presentation": presentation }),
        },
    }
}

/// Convert a storage session tree into the shared domain DTO.
fn session_tree_domain(
    tree: Vec<agena_storage::store::SessionSummary>,
) -> Result<Vec<SessionSummary>, AppError> {
    tree.into_iter()
        .map(crate::session::store::domain_summary_from_storage)
        .collect()
}

fn descendant_cancellation_order(session_id: i64, tree: &[SessionSummary]) -> Vec<i64> {
    let mut included = std::collections::HashSet::from([session_id]);
    loop {
        let previous_len = included.len();
        for summary in tree {
            if summary
                .parent_id
                .is_some_and(|parent_id| included.contains(&parent_id))
            {
                included.insert(summary.id);
            }
        }
        if included.len() == previous_len {
            break;
        }
    }

    let mut descendants = tree
        .iter()
        .filter(|summary| included.contains(&summary.id))
        .map(|summary| (summary.depth, summary.id))
        .collect::<Vec<_>>();
    if !descendants.iter().any(|(_, id)| *id == session_id) {
        descendants.push((i64::MIN, session_id));
    }
    descendants.sort_by(|left, right| right.cmp(left));
    descendants.into_iter().map(|(_, id)| id).collect()
}

fn cancel_active_execution_result(
    result: Result<(), ExecutionControlError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) | Err(ExecutionControlError::NoActiveExecution(_)) => Ok(()),
        Err(error) => Err(execution_control_to_app_error(error)),
    }
}

fn is_completed_user_rewind_target(part: &Part) -> bool {
    part.role == PartRole::User
        && execution_status_from_part_state(part.state) == ExecutionStatus::Completed
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionControlError, cancel_active_execution_result, descendant_cancellation_order,
        is_completed_user_rewind_target,
    };
    use agena_domain::SessionSummary;
    use agena_domain::SubtaskStatus;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

    fn marker(part_id: i64, role: PartRole, state: PartState) -> Part {
        let now = chrono::Utc::now().timestamp_millis();
        Part {
            part_id,
            kind: "run".to_owned(),
            role,
            state,
            content: serde_json::json!({}),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 0,
            started_at_ms: now,
            finished_at_ms: None,
            created_at_ms: now,
            updated_at_ms: now,
            provider_state: None,
        }
    }

    #[test]
    fn cancelling_a_completed_run_is_a_successful_no_op() {
        assert!(
            cancel_active_execution_result(Err(ExecutionControlError::NoActiveExecution(42)))
                .is_ok()
        );
        assert!(cancel_active_execution_result(Err(ExecutionControlError::SteerClosed)).is_err());
    }

    #[test]
    fn rewind_accepts_only_completed_user_messages() {
        let user = marker(1, PartRole::User, PartState::Completed);
        let assistant = marker(2, PartRole::Assistant, PartState::Completed);
        let pending_user = marker(3, PartRole::User, PartState::Pending);

        assert!(is_completed_user_rewind_target(&user));
        assert!(!is_completed_user_rewind_target(&assistant));
        assert!(!is_completed_user_rewind_target(&pending_user));
    }

    #[test]
    fn cancellation_orders_descendants_deepest_first() {
        let now = chrono::Utc::now();
        let summary = |id, parent_id, depth| SessionSummary {
            id,
            parent_id,
            depth,
            root_id: 1,
            workspace_id: 1,
            title: id.to_string(),
            version: 1,
            relation_kind: if parent_id.is_some() {
                agena_domain::SessionRelationKind::Subagent
            } else {
                agena_domain::SessionRelationKind::Root
            },
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            task_id: None,
            subtask_access: None,
            subtask_status: parent_id.map(|_| SubtaskStatus::Running),
            created_at: now,
            updated_at: now,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        };
        let tree = vec![
            summary(1, None, 0),
            summary(2, Some(1), 1),
            summary(3, Some(2), 2),
            summary(4, Some(1), 1),
        ];

        assert_eq!(descendant_cancellation_order(2, &tree), vec![3, 2]);
        assert_eq!(descendant_cancellation_order(1, &tree), vec![3, 4, 2, 1]);
    }
}
