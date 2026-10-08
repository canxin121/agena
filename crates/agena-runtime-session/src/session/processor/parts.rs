use super::{AppError, SessionProcessor, SessionRunRequest, Utc};
use crate::session::store::{
    StoreAdapter, new_part_from_content, typed_content_from_value, typed_content_to_value,
};
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
        let writer = run
            .store
            .facade
            .contents()
            .open(run.session_id, part_id, agena_domain::ContentKind::Text)
            .await
            .map_err(|error| AppError::Internal(format!("open model content: {error}")))?;
        let mut content = persisted.content;
        content["resources"] = serde_json::json!([writer.resource().reference()]);
        let persisted = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    content: Some(content),
                    ..Default::default()
                },
            )
            .await?;
        run.content_writers
            .lock()
            .expect("model content writers")
            .insert(part_id, writer);
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
                    // Canonical thinking shape (4.1.1): `summary`/`raw`
                    // arrays. The typed content serializer owns the exact
                    // persisted document.
                    content: serde_json::json!({ "summary": [], "raw": [] }),
                    summary: None,
                    visibility: PartVisibility::Both,
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
        let writer = run
            .store
            .facade
            .contents()
            .open(run.session_id, part_id, agena_domain::ContentKind::Text)
            .await
            .map_err(|error| AppError::Internal(format!("open model content: {error}")))?;
        let mut content = persisted.content;
        content["resources"] = serde_json::json!([writer.resource().reference()]);
        let persisted = run
            .store
            .update_part(
                run.session_id,
                part_id,
                PartDelta {
                    content: Some(content),
                    ..Default::default()
                },
            )
            .await?;
        run.content_writers
            .lock()
            .expect("model content writers")
            .insert(part_id, writer);
        parts.push(persisted);
        Ok(part_id)
    }

    /// Only source memory and its cursor advance on model tokens. Part facts
    /// stay small and immutable until a semantic state transition.
    pub(crate) async fn append_text_delta(
        &self,
        run: &SessionRunRequest,
        _parts: &mut [Part],
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        let writer = run
            .content_writers
            .lock()
            .expect("model content writers")
            .get(&part_id)
            .cloned()
            .ok_or_else(|| AppError::Internal(format!("model source missing: {part_id}")))?;
        writer
            .append_text(delta)
            .await
            .map_err(|error| AppError::Internal(format!("append model content: {error}")))?;
        Ok(())
    }

    pub(crate) async fn append_reasoning_delta(
        &self,
        run: &SessionRunRequest,
        parts: &mut [Part],
        part_id: i64,
        delta: &str,
    ) -> Result<(), AppError> {
        self.append_text_delta(run, parts, part_id, delta).await
    }

    /// Push a part's current in-memory state/content onto its durable row
    /// (`update_part`) and refresh the turn accumulator. The caller must have
    /// terminalized the part in the accumulator first (via `complete_part_status`
    /// or `cancel_nonterminal_parts`/`fail_nonterminal_parts`). The independent
    /// source commits its final cursor before the terminal Part fact.
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
        let writer = run
            .content_writers
            .lock()
            .expect("model content writers")
            .remove(&part_id);
        if let Some(writer) = writer {
            let final_state = if part.state == PartState::Completed {
                agena_domain::ContentState::Complete
            } else {
                agena_domain::ContentState::Interrupted
            };
            let sealed = writer.finalize(final_state).await;
            sealed.map_err(|error| AppError::Internal(format!("commit model content: {error}")))?;
        }
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
                    summary: part.summary.clone(),
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
    /// reach the database, so failed/cancelled runs leave no ghost calls.
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
            if part.part_id < 0
                && let Some(durable) = created_iter.next()
            {
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
    if let Some(existing) = parts
        .iter_mut()
        .find(|part| part.part_id == updated.part_id)
    {
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
