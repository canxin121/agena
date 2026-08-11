use super::{
    AppError, Arc, BTreeMap, CompletionRequest, CompletionStreamEvent, FinishReason, PathBuf,
    PendingProviderNativeToolCall, PendingToolCall, REASONING_PLACEHOLDER, SessionProcessor,
    SessionRunRequest, SessionRunResult, SessionRunTermination, cancel_nonterminal_parts,
    complete_part_status, fail_nonterminal_parts, map_finish_reason,
    message_provider_state_from_provider_metadata, pending_tool_call_stream_key,
};
use crate::provider::ProviderRegistry;
use agena_provider::{ProviderNativeToolArtifact, ProviderNativeToolOutputBlock};
use agena_storage::store::{Part, PartDelta, PartState, RunOutcome};
use futures_util::StreamExt;
use tracing::Instrument;

use agena_domain::{ArtifactRef, ViewBlock, WebSearchResult};

/// The JSON key under which a run marker's per-round records live. Multi-round
/// turns (one user message == one run marker) carry an array of round records,
/// each listing that provider round-trip's part ids and its provider replay
/// state, so the prompt-window projection can re-split the merged run into
/// per-round wire messages (each cpa round needs its own reasoning passback).
pub(crate) const MARKER_ROUNDS_KEY: &str = "rounds";

/// Build this turn's round record: the durable ids of every content part this
/// provider round-trip appended under the shared run marker, plus the round's
/// provider replay state. The record is appended to the marker's
/// `content["rounds"]` array (see [`MARKER_ROUNDS_KEY`]).
fn round_record_from_parts(
    parts: &[Part],
    provider_state: Option<&serde_json::Value>,
) -> serde_json::Value {
    serde_json::json!({
        "part_ids": parts.iter().map(|part| part.part_id).collect::<Vec<_>>(),
        "provider_state": provider_state,
    })
}

/// Append a round record to the marker's accumulated content, preserving every
/// other key (`run_kind`, `provider_id`, `model_id`, `turn_id`, `reply_id`,
/// `usage`, …). Returns `None` when no prior marker content is available (the
/// caller did not carry the marker into the turn — legacy single-round runs),
/// in which case the projection falls back to whole-run wire projection.
fn merge_round_record(
    marker_content: Option<&serde_json::Value>,
    round_record: serde_json::Value,
) -> Option<serde_json::Value> {
    let mut content = marker_content?.as_object()?.clone();
    let mut rounds = content
        .get(MARKER_ROUNDS_KEY)
        .and_then(serde_json::Value::as_array)
        .cloned()
        .unwrap_or_default();
    rounds.push(round_record);
    content.insert(
        MARKER_ROUNDS_KEY.to_owned(),
        serde_json::Value::Array(rounds),
    );
    Some(serde_json::Value::Object(content))
}

impl SessionProcessor {
    pub fn new(
        plugins: Arc<agena_plugin_host::PluginHost>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            plugins,
            workspace_root: workspace_root.into(),
        }
    }

    /// Apply the `chat.params` plugin hook chain to a [`CompletionRequest`]
    /// before sending it to the provider.
    pub(crate) async fn apply_chat_params_hook(
        &self,
        provider_id: &str,
        model_id: &str,
        session_id: i64,
        request: &mut CompletionRequest,
        cancellation: Option<tokio_util::sync::CancellationToken>,
    ) {
        let plugins = &self.plugins;
        if plugins.is_empty() {
            return;
        }
        let mut params = serde_json::Map::new();
        if let Some(t) = request.temperature {
            params.insert("temperature".into(), serde_json::json!(t));
        }
        if let Some(m) = request.max_output_tokens {
            params.insert("max_output_tokens".into(), serde_json::json!(m));
        }
        let input = agena_plugin_host::ChatParamsInput {
            provider: provider_id.to_string(),
            model: model_id.to_string(),
            params: serde_json::Value::Object(params),
            session_id: Some(session_id),
        };
        match plugins
            .dispatch_chat_params_cancellable(input, cancellation)
            .await
        {
            Ok(updated) => {
                if let Some(t) = updated.params.get("temperature").and_then(|v| v.as_f64()) {
                    request.temperature = Some(t as f32);
                }
                if let Some(m) = updated
                    .params
                    .get("max_output_tokens")
                    .and_then(|v| v.as_u64())
                {
                    request.max_output_tokens = Some(m as u32);
                }
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::chat_params",
                    "chat.params hook failed: {err}"
                );
            }
        }
    }

    /// Execute one provider model turn as a **parts-native** run (R2).
    ///
    /// The caller has already started the run marker (via
    /// [`StoreAdapter::start_run`]) and passes its id as `run.next_message_id`.
    /// This turn's durable state is written exclusively through parts: the
    /// active text/think parts are appended under the marker
    /// (`append_parts`), stream deltas are pushed as `content_text_delta`
    /// (`update_part`, amortized by the facade per D10), think deltas replace
    /// the full content document (D10 asymmetry — the thinking content is an
    /// array shape, so `content_text_delta` cannot be applied), and the run is
    /// terminalized with `complete_run`/`cancel_run`. In-flight tool-call
    /// placeholders are deferred and appended only on the success path. The
    /// result carries the persisted parts — never a v1 [`Message`] — so the
    /// caller must not re-persist this turn (parts are the only durable write
    /// source; no double write).
    pub(crate) async fn run_turn(
        &self,
        mut run: SessionRunRequest,
        provider_registry: &ProviderRegistry,
    ) -> Result<SessionRunResult, AppError> {
        let processor_span = tracing::info_span!(
            "session.processor_turn",
            session_id = run.session_id,
            provider_id = %run.model.provider_id,
            model_id = %run.model.model_id,
        );
        // Provider-visible prompt content is append-only for prompt-cache
        // affinity. Mutating chat hooks can rewrite/drop/reorder system and
        // message content, so they are not applied on the provider request path.
        self.apply_chat_params_hook(
            run.model.provider_id.as_ref(),
            run.model.model_id.as_ref(),
            run.session_id,
            &mut run.completion,
            run.cancel.clone(),
        )
        .await;
        if run
            .cancel
            .as_ref()
            .is_some_and(|token| token.is_cancelled())
        {
            return Err(AppError::Cancelled);
        }
        let provider_request = crate::provider::with_request_cancellation(
            run.cancel.clone(),
            provider_registry
                .complete_stream(&run.model, run.completion.clone())
                .instrument(processor_span.clone()),
        );
        let stream_result = match run.cancel.as_ref() {
            Some(cancel) => tokio::select! {
                biased;
                _ = cancel.cancelled() => return Err(AppError::Cancelled),
                result = provider_request => result,
            },
            None => provider_request.await,
        };
        agena_runtime::record_provider_stream();
        agena_runtime::record_provider_call(stream_result.is_ok());
        let mut stream = stream_result?;

        // The run marker started by the caller is the assistant message's
        // durable id (design 4.1: a message == one run).
        let assistant_message_id = run.next_message_id;
        run.next_message_id += 1;

        // Durable part rows created under the run marker, in creation order,
        // with the latest engine state applied. This is the turn's only
        // in-memory accumulator and what the result carries; the manager reads
        // the persisted run (marker + children) directly.
        let mut parts: Vec<Part> = Vec::new();

        let mut active_text_part: Option<i64> = None;
        let mut active_reasoning_part: Option<i64> = None;
        let mut pending_calls: BTreeMap<String, PendingToolCall> = BTreeMap::new();
        let mut pending_provider_native_tool_calls: BTreeMap<
            String,
            PendingProviderNativeToolCall,
        > = BTreeMap::new();
        let mut provider_err: Option<AppError> = None;
        let mut usage = None;
        let mut finish_reason_enum = FinishReason::Stop;
        let mut provider_metadata = None;
        let mut visible_text = String::new();
        let mut reasoning_text = String::new();
        let mut saw_tool_call = false;
        let mut saw_provider_native_tool_call = false;
        let mut follow_up_requested = false;

        let cancel = run.cancel.clone();
        loop {
            let next_event =
                crate::provider::with_request_cancellation(cancel.clone(), stream.next());
            let next = match cancel.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    _ = token.cancelled() => None,
                    item = next_event => item,
                },
                None => next_event.await,
            };
            let Some(item) = next else { break };
            match item {
                Ok(CompletionStreamEvent::TextDelta { delta, .. }) => {
                    visible_text.push_str(delta.as_str());
                    if let Some(part_id) = active_reasoning_part.take() {
                        // A thinking segment is complete the moment the model
                        // starts producing text. Terminalize it immediately —
                        // in memory and on the durable row — so the terminal
                        // stops showing the spinner instead of waiting for the
                        // stream end.
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    let part_id = match active_text_part {
                        Some(part_id) => part_id,
                        None => {
                            let part_id = self
                                .start_text_part(&run, assistant_message_id, &mut parts)
                                .await?;
                            active_text_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_text_delta(&run, &mut parts, part_id, delta.as_str())
                        .await?;
                }
                Ok(CompletionStreamEvent::ToolCallDelta {
                    stream_key,
                    id,
                    name,
                    arguments_delta,
                    ..
                }) => {
                    saw_tool_call = true;
                    if let Some(part_id) = active_text_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }

                    let stream_key =
                        pending_tool_call_stream_key(&mut pending_calls, stream_key, id.as_deref());
                    let pending = pending_calls.entry(stream_key).or_default();
                    if let Some(id) = id {
                        pending.id = Some(id);
                    }
                    if let Some(name) = name {
                        pending.name = Some(name);
                    }
                    pending.arguments_json.push_str(arguments_delta.as_str());
                    self.ensure_pending_tool_call_part(
                        &mut run,
                        assistant_message_id,
                        &mut parts,
                        pending,
                    )
                    .await?;
                }
                Ok(CompletionStreamEvent::ToolCallSnapshot {
                    stream_key,
                    id,
                    name,
                    arguments_json,
                    ..
                }) => {
                    saw_tool_call = true;
                    if let Some(part_id) = active_text_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }

                    let stream_key =
                        pending_tool_call_stream_key(&mut pending_calls, stream_key, id.as_deref());
                    let pending = pending_calls.entry(stream_key).or_default();
                    if let Some(id) = id {
                        pending.id = Some(id);
                    }
                    if let Some(name) = name {
                        pending.name = Some(name);
                    }
                    // A degenerate snapshot (empty or `{}`) must never wipe
                    // arguments already accumulated from deltas; an out-of-order
                    // or empty Start/Finish snapshot would otherwise discard a
                    // complete tool call. Mirror the accumulator's behavior,
                    // which treats an empty snapshot as "no change".
                    let snapshot_is_degenerate =
                        arguments_json.trim().is_empty() || arguments_json.trim() == "{}";
                    if !snapshot_is_degenerate {
                        pending.arguments_json = arguments_json.clone();
                    }
                    self.ensure_pending_tool_call_part(
                        &mut run,
                        assistant_message_id,
                        &mut parts,
                        pending,
                    )
                    .await?;
                }
                Ok(CompletionStreamEvent::ProviderNativeToolCallStarted {
                    stream_key,
                    id,
                    invocation,
                    title,
                    raw,
                    ..
                }) => {
                    saw_provider_native_tool_call = true;
                    if let Some(part_id) = active_text_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }

                    let pending = pending_provider_native_tool_calls
                        .entry(stream_key)
                        .or_default();
                    pending.id = id;
                    pending.invocation = Some(invocation);
                    pending.title = title;
                    pending.raw = raw;
                    self.ensure_provider_native_tool_call_part(
                        &mut run,
                        assistant_message_id,
                        &mut parts,
                        pending,
                    )
                    .await?;
                }
                Ok(CompletionStreamEvent::ProviderNativeToolCallCompleted {
                    stream_key,
                    id,
                    invocation,
                    title,
                    summary,
                    output_text,
                    blocks,
                    details,
                    raw,
                    ..
                }) => {
                    saw_provider_native_tool_call = true;
                    if let Some(part_id) = active_text_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }

                    let pending = pending_provider_native_tool_calls
                        .remove(&stream_key)
                        .unwrap_or_default();
                    self.complete_provider_native_tool_call_part(
                        &mut run,
                        assistant_message_id,
                        &mut parts,
                        pending,
                        id,
                        invocation,
                        title,
                        summary,
                        output_text,
                        provider_native_output_blocks_to_operation_blocks(blocks),
                        details,
                        raw,
                    )
                    .await?;
                }
                Ok(CompletionStreamEvent::Completed {
                    finish_reason,
                    usage: usage_value,
                    provider_metadata: completed_provider_metadata,
                    end_turn,
                    ..
                }) => {
                    usage = usage_value;
                    if let Some(reason) = finish_reason.as_ref() {
                        finish_reason_enum = map_finish_reason(reason);
                    }
                    provider_metadata = completed_provider_metadata;
                    follow_up_requested = end_turn == Some(false);
                }
                Ok(CompletionStreamEvent::ThinkingDelta { delta, .. }) => {
                    reasoning_text.push_str(delta.as_str());
                    if let Some(part_id) = active_text_part.take() {
                        complete_part_status(&mut parts, part_id)?;
                        self.persist_part_state(&run, &mut parts, part_id).await?;
                    }
                    let part_id = match active_reasoning_part {
                        Some(part_id) => part_id,
                        None => {
                            let part_id = self
                                .start_reasoning_part(&run, assistant_message_id, &mut parts)
                                .await?;
                            active_reasoning_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_reasoning_delta(&run, &mut parts, part_id, delta.as_str())
                        .await?;
                }
                Ok(CompletionStreamEvent::ProviderRetry { .. }) => {}
                Err(err) => {
                    provider_err = Some(err.into());
                    break;
                }
            }
        }
        // Releasing the provider stream drops the reqwest response body (or
        // websocket) before transcript finalization. Cancellation must not
        // keep an idle HTTP body alive while SQLite events are being written.
        drop(stream);

        // If the cancel token tripped, the loop above broke without an
        // explicit provider error. Surface a synthetic terminal error so
        // the caller knows the run was cancelled rather than completed.
        let cancelled = cancel.as_ref().is_some_and(|token| token.is_cancelled());
        if provider_err.is_none() && cancelled {
            provider_err = Some(AppError::Cancelled);
        }

        if provider_err.is_some() {
            if cancelled {
                cancel_nonterminal_parts(&mut parts)?;
            } else {
                fail_nonterminal_parts(&mut parts)?;
            }
        } else {
            if let Some(part_id) = active_text_part.take() {
                complete_part_status(&mut parts, part_id)?;
                self.persist_part_state(&run, &mut parts, part_id).await?;
            }
            if let Some(part_id) = active_reasoning_part.take() {
                complete_part_status(&mut parts, part_id)?;
                self.persist_part_state(&run, &mut parts, part_id).await?;
            }
        }

        if provider_err.is_none()
            && visible_text.trim().is_empty()
            && !saw_tool_call
            && !saw_provider_native_tool_call
            && !reasoning_text.trim().is_empty()
            && reasoning_text.trim() != REASONING_PLACEHOLDER
        {
            let part_id = self
                .start_text_part(&run, assistant_message_id, &mut parts)
                .await?;
            self.append_text_delta(&run, &mut parts, part_id, reasoning_text.as_str())
                .await?;
            complete_part_status(&mut parts, part_id)?;
            self.persist_part_state(&run, &mut parts, part_id).await?;
        }

        // A successful stream that produced no visible text, no tool call,
        // and no substantive reasoning is an empty response. Surface it as a
        // failure with guidance instead of persisting a silent empty
        // assistant message. Placeholder-only reasoning counts as empty: the
        // reasoning-copy block above deliberately skips the placeholder, so
        // the user would otherwise see nothing.
        if provider_err.is_none()
            && visible_text.trim().is_empty()
            && !saw_tool_call
            && !saw_provider_native_tool_call
            && (reasoning_text.trim().is_empty() || reasoning_text.trim() == REASONING_PLACEHOLDER)
        {
            provider_err = Some(AppError::EmptyResponse);
        }

        if provider_err.is_none() {
            // A malformed final tool call is a run failure, not an abort: fold
            // it into the terminal path below so the already-persisted text
            // and think parts are terminalized alongside the marker.
            if let Err(err) = self
                .finalize_pending_tool_calls(
                    &mut run,
                    assistant_message_id,
                    &mut parts,
                    pending_calls,
                )
                .await
            {
                provider_err = Some(err);
            }
        }
        if provider_err.is_none() {
            // Tool Operations publish their authoritative execution checkpoints
            // through the tool executor later; this only makes the call-side
            // parts durable so the run's children are complete.
            self.persist_deferred_tool_parts(&run, assistant_message_id, &mut parts)
                .await?;
        }

        if let Some(err) = provider_err {
            // Drop in-flight tool-call placeholders (ghost calls) from the
            // accumulator: a pending tool-call placeholder (e.g. streamed in a
            // name but aborted before execution) never reached the store, and
            // persisting its negative placeholder id would corrupt the run.
            // Text and reasoning parts are real content and are retained.
            parts.retain(|part| part.part_id >= 0);

            // Persist the terminalization of every durable content part
            // (text/think rows appended above), then terminalize the marker.
            let durable_part_ids: Vec<i64> = parts.iter().map(|part| part.part_id).collect();
            for part_id in durable_part_ids {
                self.persist_part_state(&run, &mut parts, part_id).await?;
            }
            let failure = (!cancelled).then(|| err.failure());
            if let Some(failure) = failure.as_ref() {
                parts.push(
                    run.store
                        .append_failure_part(run.session_id, assistant_message_id, failure)
                        .await?,
                );
            }
            let marker = if cancelled {
                // `cancel_run` cancels the marker and its in-flight children.
                run.store
                    .cancel_run(run.session_id, assistant_message_id)
                    .await?;
                self.collect_run_parts(&run.store, run.session_id, assistant_message_id)
                    .await?
            } else {
                run.store
                    .complete_run(
                        run.session_id,
                        assistant_message_id,
                        RunOutcome {
                            status: PartState::Failed,
                            abort_reason: Some("provider_error".to_owned()),
                            content: None,
                            provider_state: None,
                        },
                    )
                    .await?;
                self.collect_run_parts(&run.store, run.session_id, assistant_message_id)
                    .await?
            };

            if let Some(failure) = failure.as_ref() {
                tracing::warn!(
                    failure_id = %failure.id,
                    session_id = run.session_id,
                    diagnostic = %err,
                    "provider stream failed"
                );
            }
            return Ok(SessionRunResult {
                assistant_message_id,
                run_marker: marker,
                parts,
                provider_metadata,
                termination: if cancelled {
                    SessionRunTermination::Cancelled
                } else {
                    SessionRunTermination::Failed(err)
                },
                follow_up_requested: false,
                finish_reason: FinishReason::Stop,
                usage: None,
            });
        }

        // Successful run: the accumulated parts carry the terminal state driven
        // above. Terminalize the run marker only when every part under it is
        // terminal (design 17.3/17.5): a successful turn with in-flight tool
        // calls keeps the marker in-flight so the session stays Running while
        // the tools execute; the deferred tool parts were appended above, and
        // the tool-execution persist terminalizes the marker once they resolve.
        let provider_state = provider_metadata
            .as_ref()
            .and_then(message_provider_state_from_provider_metadata)
            .and_then(|state| {
                serde_json::to_value(&state)
                    .map_err(|error| {
                        tracing::warn!(
                            target: "agena::session::processor",
                            run_id = assistant_message_id,
                            "failed to serialize run provider state: {error}"
                        );
                    })
                    .ok()
            });
        let all_parts_terminal = parts.iter().all(|part| part.state.is_terminal());
        let run_marker = if all_parts_terminal {
            run.store
                .complete_run(
                    run.session_id,
                    assistant_message_id,
                    RunOutcome {
                        status: PartState::Completed,
                        abort_reason: None,
                        content: None,
                        provider_state,
                    },
                )
                .await?;
            self.collect_run_parts(&run.store, run.session_id, assistant_message_id)
                .await?
        } else if provider_state.is_some() {
            // A tool-calling turn deliberately leaves its run marker in
            // progress until the tool executor resolves every child. Provider
            // replay state belongs to that assistant turn and must already be
            // durable before the follow-up model request: reasoning-capable
            // providers reject a tool result when the preceding assistant
            // reasoning payload is not passed back. `complete_run` later
            // preserves this value when its outcome has no replacement.
            //
            // Multi-round turns accumulate a per-round record on the marker's
            // `content["rounds"]` so the prompt-window projection can re-split
            // the merged run into per-round wire messages (each carrying its
            // own reasoning passback). The marker stays in-flight across every
            // round of one turn; it is terminalized only when the whole turn
            // finishes.
            let round_record = round_record_from_parts(&parts, provider_state.as_ref());
            let merged_content = merge_round_record(run.marker_content.as_ref(), round_record);
            run.store
                .update_part(
                    run.session_id,
                    assistant_message_id,
                    PartDelta {
                        provider_state,
                        content: merged_content,
                        ..PartDelta::default()
                    },
                )
                .await?
        } else {
            self.collect_run_parts(&run.store, run.session_id, assistant_message_id)
                .await?
        };

        Ok(SessionRunResult {
            assistant_message_id,
            run_marker,
            parts,
            provider_metadata,
            termination: SessionRunTermination::Completed,
            follow_up_requested,
            finish_reason: finish_reason_enum,
            usage,
        })
    }
}

fn provider_native_output_blocks_to_operation_blocks(
    blocks: Vec<ProviderNativeToolOutputBlock>,
) -> Vec<ViewBlock> {
    blocks
        .into_iter()
        .map(|block| match block {
            ProviderNativeToolOutputBlock::Text { text } => ViewBlock::Text { id: None, text },
            ProviderNativeToolOutputBlock::SearchResults { results, .. } => {
                ViewBlock::SearchResults {
                    id: None,
                    items: results
                        .into_iter()
                        .map(|result| WebSearchResult {
                            title: result.title,
                            url: result.uri,
                            snippet: result.snippet,
                        })
                        .collect(),
                    total: None,
                }
            }
            ProviderNativeToolOutputBlock::Media {
                mime_type: _,
                artifact,
            } => ViewBlock::Media {
                id: None,
                artifact: provider_native_artifact_to_operation_block(artifact),
            },
        })
        .collect()
}

fn provider_native_artifact_to_operation_block(
    artifact: ProviderNativeToolArtifact,
) -> ArtifactRef {
    ArtifactRef {
        uri: artifact.uri,
        mime: artifact.mime,
        name: artifact.name,
        size_bytes: artifact.size_bytes,
        sha256: artifact.sha256,
    }
}
