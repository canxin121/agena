use super::{
    AppError, DateTime, ExecutionStatus, Message, MessagePart, MessageProviderState, PartContent,
    SessionProcessor, SessionRunRequest, Utc,
};
use crate::message::{RequestPart, RuntimeActivity};
use crate::session::store::{
    OPERATION_ID_METADATA_KEY, StoreAdapter, execution_status_from_part_state,
    new_part_from_content, part_content_from_value, part_role_from_role,
    part_state_from_execution_status, role_from_part_role, serialize_part_content,
};
use agena_domain::{AssistantReplyId, MessageSource, ReasoningPart, TurnId};
use agena_storage::store::{NewPart, Part, PartDelta, PartRole, PartState, PartVisibility};

impl SessionProcessor {
    /// Create the run's active text part and persist it under the run marker
    /// as `InProgress` (R2): the engine owns the id, so the returned part id
    /// is durable from the moment the first token arrives. The part is also
    /// mirrored onto the in-memory projection for the legacy prompt path.
    /// Returns the durable part id.
    pub(crate) async fn start_text_part(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
        created_at: DateTime<Utc>,
    ) -> Result<i64, AppError> {
        let created = run
            .store
            .append_parts(
                run.session_id,
                assistant.id,
                vec![NewPart {
                    kind: "text".to_owned(),
                    role: PartRole::Assistant,
                    content: serde_json::json!({ "type": "text", "text": "" }),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
            )
            .await?;
        let persisted = created.into_iter().next().ok_or_else(|| {
            AppError::Internal(format!(
                "append_parts returned no text part for run {}",
                assistant.id
            ))
        })?;
        let part_id = persisted.part_id;
        parts.push(persisted);

        let part = MessagePart::from_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::InProgress,
            PartContent::text(String::new()),
        );
        assistant.push_part(part);
        if assistant.state == ExecutionStatus::Pending {
            assistant
                .transition_state(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        Ok(part_id)
    }

    /// Create the run's active reasoning part and persist it under the run
    /// marker as `InProgress` with the canonical thinking content shape.
    /// Returns the durable part id.
    pub(crate) async fn start_reasoning_part(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
        created_at: DateTime<Utc>,
    ) -> Result<i64, AppError> {
        let created = run
            .store
            .append_parts(
                run.session_id,
                assistant.id,
                vec![NewPart {
                    kind: "think".to_owned(),
                    role: PartRole::Assistant,
                    content: serde_json::json!({
                        "type": "activity",
                        "activity_type": "reasoning",
                        "payload": { "summary": [], "raw_content": [] }
                    }),
                    summary: None,
                    visibility: PartVisibility::Both,
                    rendered_markdown: None,
                    parent_part_id: None,
                    state: PartState::InProgress,
                }],
            )
            .await?;
        let persisted = created.into_iter().next().ok_or_else(|| {
            AppError::Internal(format!(
                "append_parts returned no think part for run {}",
                assistant.id
            ))
        })?;
        let part_id = persisted.part_id;
        parts.push(persisted);

        let part = MessagePart::from_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::InProgress,
            PartContent::Activity(RuntimeActivity::Reasoning(ReasoningPart {
                summary: Vec::new(),
                raw_content: Vec::new(),
                encrypted_content: None,
            })),
        );
        assistant.push_part(part);
        if assistant.state == ExecutionStatus::Pending {
            assistant
                .transition_state(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        Ok(part_id)
    }

    /// Stream one text delta: mirror it onto the in-memory projection and push
    /// it as `content_text_delta` through the facade. The facade coalesces
    /// deltas in its streaming buffer and flushes after
    /// `STREAMING_FLUSH_DELTA_COUNT` deltas or on any non-text/terminal update
    /// (D10), so revision advances once per coalesced flush, not per token.
    pub(crate) async fn append_text_delta(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active text part missing from assistant snapshot: {part_id}"
                ))
            })?;
        if !part.append_text_delta(delta) {
            return Err(AppError::Internal(format!(
                "failed to append text delta to part {part_id}: kind mismatch"
            )));
        }
        let updated = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    content_text_delta: Some(delta.to_owned()),
                    ..Default::default()
                },
            )
            .await?;
        upsert_part(parts, updated);
        Ok(())
    }

    /// Stream one thinking delta. D10 asymmetry: the think content is an array
    /// shape (summary/raw_content), so `content_text_delta` cannot be applied
    /// to it; each update replaces the whole content document, which the
    /// facade commits immediately (revision per update instead of per
    /// coalesced flush).
    pub(crate) async fn append_reasoning_delta(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active reasoning part missing from assistant snapshot: {part_id}"
                ))
            })?;
        if !part.append_reasoning_summary_delta(delta.to_string()) {
            return Err(AppError::Internal(format!(
                "failed to append reasoning delta to part {part_id}: kind mismatch"
            )));
        }
        let content = serialize_part_content(part)?;
        let updated = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    state: Some(PartState::InProgress),
                    content: Some(content),
                    ..Default::default()
                },
            )
            .await?;
        upsert_part(parts, updated);
        Ok(())
    }

    /// Push a part's current in-memory status/content onto its durable row
    /// (`update_part`) and refresh the turn accumulator. The caller must have
    /// terminalized the part in memory first (via `complete_part_status` or
    /// `cancel_nonterminal_parts`/`fail_nonterminal_parts`); this is what
    /// flushes the part's buffered stream deltas onto the engine row (D10).
    pub(crate) async fn persist_part_state(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
        part_id: i64,
    ) -> Result<(), AppError> {
        let part = assistant
            .parts
            .iter_mut()
            .find(|part| part.id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "part missing from assistant snapshot while persisting: {part_id}"
                ))
            })?;
        let content = serialize_part_content(part)?;
        let updated = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    state: Some(part_state_from_execution_status(part.status)),
                    content: Some(content),
                    content_text_delta: None,
                    summary: part.summary.clone(),
                    rendered_markdown: None,
                    provider_state: None,
                    finished_at_ms: part
                        .status
                        .is_terminal()
                        .then(|| Utc::now().timestamp_millis()),
                },
            )
            .await?;
        upsert_part(parts, updated);
        Ok(())
    }

    /// Persist the run's deferred tool-call parts (created in-memory with
    /// placeholder ids during streaming) under the run marker. Tool operations
    /// publish their authoritative execution checkpoints through the tool
    /// executor later; this only makes the call-side parts durable so the
    /// run's children are complete. The in-memory placeholder ids are remapped
    /// onto the engine ids. Returns the created durable parts.
    ///
    /// Called only on the success path: failed/cancelled runs drop in-flight
    /// operation placeholders (ghost calls) before this runs, so they never
    /// reach the database (matching the pre-R2 v1 persist behavior).
    pub(crate) async fn persist_deferred_tool_parts(
        &self,
        run: &SessionRunRequest,
        assistant: &mut Message,
        parts: &mut Vec<Part>,
    ) -> Result<(), AppError> {
        let deferred: Vec<MessagePart> = assistant
            .parts
            .iter()
            .filter(|part| part.id < 0)
            .cloned()
            .collect();
        if deferred.is_empty() {
            return Ok(());
        }
        let new_parts = deferred
            .iter()
            .map(new_part_for_deferred_tool_part)
            .collect::<Result<Vec<_>, _>>()?;
        let created = run
            .store
            .append_parts(run.session_id, assistant.id, new_parts)
            .await?;
        parts.extend(created.iter().cloned());
        // Remap the in-memory placeholder ids onto the durable ids so the
        // working projection stays consistent with the store.
        for (message_part, durable) in assistant.parts.iter_mut().zip(created.iter()) {
            if message_part.id < 0 {
                message_part.id = durable.part_id;
                message_part.message_id = assistant.id;
            }
        }
        Ok(())
    }

    /// Load the authoritative terminal state for a run marker from the facade.
    /// Called after `complete_run`/`cancel_run` so the result carries the
    /// marker's final content, state, and provider state (which the facade's
    /// `StoreAdapter` wrappers otherwise discard).
    pub(crate) async fn collect_run_parts(
        &self,
        store: &StoreAdapter,
        session_id: i64,
        run_id: i64,
    ) -> Result<Part, AppError> {
        let view = store
            .facade
            .load(session_id)
            .await
            .map_err(|error| AppError::Internal(format!("load run {run_id} parts: {error}")))?;
        view.parts
            .iter()
            .find(|part| part.part_id == run_id)
            .cloned()
            .ok_or_else(|| AppError::Internal(format!("run marker {run_id} missing after turn")))
    }
}

/// Replace (or append) a durable part row in the turn's part accumulator,
/// preserving creation order.
fn upsert_part(parts: &mut Vec<Part>, updated: Part) {
    if let Some(existing) = parts.iter_mut().find(|part| part.part_id == updated.part_id) {
        *existing = updated;
    } else {
        parts.push(updated);
    }
}

/// Build a [`NewPart`] for a deferred tool-call part, stashing the in-memory
/// provider `operation_id` into the operation metadata so a later projection
/// (and reload) can recover it (mirrors `serialize_part_content`).
fn new_part_for_deferred_tool_part(part: &MessagePart) -> Result<NewPart, AppError> {
    let mut content = part.content.as_ref().cloned().ok_or_else(|| {
        AppError::Internal("tool part with no content cannot be persisted".to_owned())
    })?;
    if let Some(operation_id) = part.operation_id.as_deref()
        && let PartContent::Activity(RuntimeActivity::Operation(operation)) = &mut content
    {
        operation.metadata.insert(
            OPERATION_ID_METADATA_KEY.to_owned(),
            serde_json::Value::String(operation_id.to_owned()),
        );
    }
    new_part_from_content(
        "tool_call",
        part_role_from_role(agena_domain::Role::Assistant),
        &content,
        part_state_from_execution_status(part.status),
    )
}

/// Project a persisted run (marker + child parts) back into the legacy
/// in-memory v1 [`Message`] the prompt/UI paths consume. Mirrors the reload
/// projection in `session_from_view` (store.rs), so the in-memory aggregate
/// after a turn is identical to what a reload would produce. `state` carries
/// the run's final v1 message state (Completed even when the marker stays
/// in-flight for pending tools).
pub(crate) fn assistant_message_from_run_parts(
    run_id: i64,
    state: ExecutionStatus,
    marker: &Part,
    parts: &[Part],
    provider_state: Option<MessageProviderState>,
    usage: Option<agena_provider::CompletionUsage>,
) -> Result<Message, AppError> {
    let role = role_from_part_role(marker.role);
    let created_at = timestamp_millis_to_utc(marker.created_at_ms)?;
    let metadata = metadata_from_run_marker(marker);
    let mut message_parts = Vec::with_capacity(parts.len());
    for (index, part) in parts.iter().enumerate() {
        message_parts.push(project_run_part(part, run_id, index as i32)?);
    }
    Ok(Message {
        id: run_id,
        role,
        state,
        parts: message_parts,
        created_at,
        metadata,
        provider_state,
        usage,
    })
}

fn timestamp_millis_to_utc(timestamp_ms: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms)
        .ok_or_else(|| AppError::Internal(format!("invalid timestamp {timestamp_ms}")))
}

/// Mirror of store.rs `metadata_from_parts`: reconstruct the conversation and
/// model identity recorded on the run marker content.
fn metadata_from_run_marker(marker: &Part) -> crate::message::MessageMetadata {
    let mut metadata = crate::message::MessageMetadata {
        model_turn_id: Some(marker.part_id),
        ..Default::default()
    };
    if let Some(turn_id) = marker.content.get("turn_id").and_then(serde_json::Value::as_str)
        && let Ok(uuid) = uuid::Uuid::parse_str(turn_id)
    {
        metadata.conversation_turn_id = Some(TurnId(uuid));
    }
    if let Some(reply_id) = marker.content.get("reply_id").and_then(serde_json::Value::as_str)
        && let Ok(uuid) = uuid::Uuid::parse_str(reply_id)
    {
        metadata.conversation_reply_id = Some(AssistantReplyId(uuid));
    }
    if let Some(model_id) = marker.content.get("model_id").and_then(serde_json::Value::as_str) {
        metadata.model_id = model_id.to_owned();
    }
    if let Some(provider_id) = marker.content.get("provider_id").and_then(serde_json::Value::as_str)
    {
        metadata.model_provider_id = provider_id.to_owned();
    }
    if let Some(source) = marker.content.get("source").and_then(serde_json::Value::as_str) {
        metadata.source = match source {
            "user" => MessageSource::User,
            "system" => MessageSource::System,
            "tool" => MessageSource::Assistant,
            _ => MessageSource::User,
        };
    }
    metadata
}

/// Mirror of store.rs `part_to_message_part`: decode one persisted part into
/// the execution-engine [`MessagePart`], recovering the stashed operation id.
fn project_run_part(part: &Part, message_id: i64, part_index: i32) -> Result<MessagePart, AppError> {
    let content = part_content_from_value(&part.kind, &part.content)?;
    let status = match &content {
        PartContent::Activity(RuntimeActivity::Operation(operation)) => operation.status(),
        PartContent::Activity(RuntimeActivity::Interaction(RequestPart::UserInput(request))) => {
            request.status()
        }
        _ => execution_status_from_part_state(part.state),
    };
    let mut message_part = MessagePart::from_content_with_index(
        part.part_id,
        message_id,
        part_index,
        timestamp_millis_to_utc(part.created_at_ms)?,
        status,
        content,
    );
    if let Some(summary) = part.summary.as_deref() {
        message_part.summary = Some(summary.to_owned());
    }
    message_part.has_detail = part.content.is_object();
    if let Some(PartContent::Activity(RuntimeActivity::Operation(operation))) =
        message_part.content.as_ref()
    {
        if message_part.operation_id.is_none() {
            message_part.operation_id = operation
                .metadata
                .get(OPERATION_ID_METADATA_KEY)
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned);
        }
        message_part.activity_id = Some(agena_domain::ActivityId::new());
    }
    Ok(message_part)
}
