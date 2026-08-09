use super::{AppError, SessionProcessor, SessionRunRequest, Utc};
use crate::session::store::{
    StoreAdapter, new_part_from_content, typed_content_from_value, typed_content_to_value,
};
use agena_runtime_contracts::part_content::TypedContent;
use agena_storage::store::{NewPart, Part, PartDelta, PartRole, PartState, PartVisibility};

impl SessionProcessor {
    /// Create the run's active text part and persist it under the run marker
    /// as `InProgress` (R2): the engine owns the id, so the returned part id
    /// is durable from the moment the first token arrives. Returns the durable
    /// part id.
    pub(crate) async fn start_text_part(
        &self,
        run: &SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
    ) -> Result<i64, AppError> {
        let created = run
            .store
            .append_parts(
                run.session_id,
                run_id,
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
                "append_parts returned no text part for run {run_id}"
            ))
        })?;
        let part_id = persisted.part_id;
        parts.push(persisted);
        Ok(part_id)
    }

    /// Create the run's active reasoning part and persist it under the run
    /// marker as `InProgress` with the canonical thinking content shape.
    /// Returns the durable part id.
    pub(crate) async fn start_reasoning_part(
        &self,
        run: &SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
    ) -> Result<i64, AppError> {
        let created = run
            .store
            .append_parts(
                run.session_id,
                run_id,
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
                "append_parts returned no think part for run {run_id}"
            ))
        })?;
        let part_id = persisted.part_id;
        parts.push(persisted);
        Ok(part_id)
    }

    /// Stream one text delta onto the durable row through the facade. The
    /// facade coalesces deltas in its streaming buffer and flushes after
    /// `STREAMING_FLUSH_DELTA_COUNT` deltas or on any non-text/terminal update
    /// (D10), so revision advances once per coalesced flush, not per token.
    /// The returned part (the authoritative in-memory overlay) is folded back
    /// into the turn accumulator.
    pub(crate) async fn append_text_delta(
        &self,
        run: &SessionRunRequest,
        parts: &mut Vec<Part>,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = parts
            .iter()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active text part missing from turn accumulator: {part_id}"
                ))
            })?;
        if part.kind != "text" {
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
        parts: &mut Vec<Part>,
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let part = parts
            .iter()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "active reasoning part missing from turn accumulator: {part_id}"
                ))
            })?;
        let mut content = typed_content_from_value(&part.kind, &part.content)?;
        if let TypedContent::Think(think) = &mut content {
            think.summary.push(delta.to_owned());
        } else {
            return Err(AppError::Internal(format!(
                "failed to append reasoning delta to part {part_id}: kind mismatch"
            )));
        }
        let content = typed_content_to_value(&content)?;
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

    /// Push a part's current in-memory state/content onto its durable row
    /// (`update_part`) and refresh the turn accumulator. The caller must have
    /// terminalized the part in the accumulator first (via `complete_part_status`
    /// or `cancel_nonterminal_parts`/`fail_nonterminal_parts`); this is what
    /// flushes the part's buffered stream deltas onto the engine row (D10).
    pub(crate) async fn persist_part_state(
        &self,
        run: &SessionRunRequest,
        parts: &mut Vec<Part>,
        part_id: i64,
    ) -> Result<(), AppError> {
        let part = parts
            .iter()
            .find(|part| part.part_id == part_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "part missing from turn accumulator while persisting: {part_id}"
                ))
            })?;
        let content = typed_content_from_value(&part.kind, &part.content)?;
        let content = typed_content_to_value(&content)?;
        let updated = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    state: Some(part.state),
                    content: Some(content),
                    content_text_delta: None,
                    summary: part.summary.clone(),
                    rendered_markdown: None,
                    provider_state: None,
                    finished_at_ms: part
                        .state
                        .is_terminal()
                        .then(|| Utc::now().timestamp_millis()),
                },
            )
            .await?;
        upsert_part(parts, updated);
        Ok(())
    }

    /// Persist the run's deferred tool-call parts (created in the accumulator
    /// with placeholder ids during streaming) under the run marker. Tool
    /// operations publish their authoritative execution checkpoints through the
    /// tool executor later; this only makes the call-side parts durable so the
    /// run's children are complete. The placeholder entries are remapped in
    /// place onto the engine ids.
    ///
    /// Called only on the success path: failed/cancelled runs drop in-flight
    /// operation placeholders (ghost calls) before this runs, so they never
    /// reach the database (matching the pre-R2 v1 persist behavior).
    pub(crate) async fn persist_deferred_tool_parts(
        &self,
        run: &SessionRunRequest,
        run_id: i64,
        parts: &mut Vec<Part>,
    ) -> Result<(), AppError> {
        let deferred: Vec<Part> = parts
            .iter()
            .filter(|part| part.part_id < 0)
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
            .append_parts(run.session_id, run_id, new_parts)
            .await?;
        // Remap the placeholder entries onto the durable rows in place,
        // preserving the accumulator's creation order.
        let mut created_iter = created.into_iter();
        for part in parts.iter_mut() {
            if part.part_id < 0 && let Some(durable) = created_iter.next() {
                *part = durable;
            }
        }
        // Any unmatched created rows (defensive) are appended in order.
        parts.extend(created_iter);
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

/// Build a [`NewPart`] for a deferred tool-call part. The placeholder part's
/// content already carries the provider `operation_id` stashed in the
/// operation metadata (see [`crate::session::store::OPERATION_ID_METADATA_KEY`]),
/// so re-serializing preserves it for a later projection (and reload).
fn new_part_for_deferred_tool_part(part: &Part) -> Result<NewPart, AppError> {
    let content = typed_content_from_value(&part.kind, &part.content)?;
    new_part_from_content("tool_call", part.role, &content, part.state)
}
