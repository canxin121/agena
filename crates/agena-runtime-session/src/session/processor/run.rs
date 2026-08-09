use super::{
    AppError, Arc, BTreeMap, CompletionRequest, CompletionStreamEvent, ContextGovernor,
    ExecutionStatus, FinishReason, Message, MessageMetadata, MessageSource, ModelRef, PathBuf,
    PendingProviderNativeToolCall, PendingToolCall, ProviderRegistry, REASONING_PLACEHOLDER, Role,
    SessionProcessor, SessionRunRequest, SessionRunResult, SessionRunTermination, Utc,
    cancel_nonterminal_parts, complete_part_status, fail_nonterminal_parts, map_finish_reason,
    message_provider_state_from_provider_metadata, pending_tool_call_stream_key,
};
use agena_provider::{ProviderNativeToolArtifact, ProviderNativeToolOutputBlock};
use futures_util::StreamExt;
use tracing::Instrument;

use agena_domain::{ArtifactRef, ViewBlock, WebSearchResult};

impl SessionProcessor {
    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
        plugins: Arc<agena_plugin_host::PluginHost>,
        workspace_root: impl Into<PathBuf>,
    ) -> Self {
        Self {
            provider_registry,
            context_governor,
            plugins,
            workspace_root: workspace_root.into(),
        }
    }

    pub(crate) fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.provider_registry
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

    pub(crate) async fn run_turn(
        &self,
        mut run: SessionRunRequest,
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
            self.provider_registry
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

        let assistant_message_id = run.next_message_id;
        run.next_message_id += 1;
        let model_turn_id = run.model_turn_id.unwrap_or(assistant_message_id);

        let assistant_metadata = MessageMetadata {
            source: MessageSource::Assistant,
            idempotency_key: None,
            model_turn_id: Some(model_turn_id),
            conversation_turn_id: Some(run.turn_id),
            conversation_reply_id: Some(run.reply_id),
            parent_message_id: run.completion_parent_message_id,
            generated_by_call_id: None,
            externally_initiated_tool: false,
            model_provider_id: run.model.provider_id.to_string(),
            model_adapter_id: run.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: run.completion.model.to_string(),
            model_thinking_mode: run.model_thinking_mode.clone(),
            model_speed_mode: run.model_speed_mode.clone(),
        };

        let started_at = Utc::now();
        let mut assistant = Message {
            id: assistant_message_id,
            role: Role::Assistant,
            state: ExecutionStatus::Pending,
            parts: Vec::new(),
            created_at: started_at,
            metadata: assistant_metadata.clone(),
            provider_state: None,
            usage: None,
        };

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
                        // starts producing text. Terminalize it immediately so
                        // the terminal stops showing the spinner instead of
                        // waiting for the stream end.
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    let part_id = match active_text_part {
                        Some(part_id) => part_id,
                        None => {
                            let part_id = run.part_ids.reserve().await?;
                            self.start_text_part(&mut assistant, part_id, Utc::now())?;
                            active_text_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_text_delta(&mut assistant, part_id, delta.as_str())?;
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
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
                    self.ensure_pending_tool_call_part(&mut run, &mut assistant, pending)
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
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
                    self.ensure_pending_tool_call_part(&mut run, &mut assistant, pending)
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
                    }

                    let pending = pending_provider_native_tool_calls
                        .entry(stream_key)
                        .or_default();
                    pending.id = id;
                    pending.invocation = Some(invocation);
                    pending.title = title;
                    pending.raw = raw;
                    self.ensure_provider_native_tool_call_part(&mut run, &mut assistant, pending)
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
                    }

                    let pending = pending_provider_native_tool_calls
                        .remove(&stream_key)
                        .unwrap_or_default();
                    self.complete_provider_native_tool_call_part(
                        &mut run,
                        &mut assistant,
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    let part_id = match active_reasoning_part {
                        Some(part_id) => part_id,
                        None => {
                            let part_id = run.part_ids.reserve().await?;
                            self.start_reasoning_part(&mut assistant, part_id, Utc::now())?;
                            active_reasoning_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_reasoning_delta(&mut assistant, part_id, delta.as_str())?;
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
                cancel_nonterminal_parts(&mut assistant)?;
            } else {
                fail_nonterminal_parts(&mut assistant)?;
            }
            // Mirror `discard_incomplete_tool_calls` on the message parts:
            // drop operation parts that never reached a terminal execution
            // outcome. A pending tool-call placeholder (e.g. streamed in a
            // name but aborted before execution) would otherwise persist as a
            // ghost "Run unknown" activity. Text and reasoning parts are real
            // content and are retained.
            assistant.parts.retain(|part| {
                if matches!(
                    part.status,
                    ExecutionStatus::Pending | ExecutionStatus::InProgress
                ) {
                    return !matches!(
                        part.content,
                        Some(crate::message::PartContent::Activity(
                            crate::message::RuntimeActivity::Operation(_)
                        ))
                    );
                }
                true
            });
        } else {
            if let Some(part_id) = active_text_part {
                complete_part_status(&mut assistant, part_id)?;
            }
            if let Some(part_id) = active_reasoning_part {
                complete_part_status(&mut assistant, part_id)?;
            }
        }

        if provider_err.is_none()
            && visible_text.trim().is_empty()
            && !saw_tool_call
            && !saw_provider_native_tool_call
            && !reasoning_text.trim().is_empty()
            && reasoning_text.trim() != REASONING_PLACEHOLDER
        {
            let part_id = run.part_ids.reserve().await?;
            self.start_text_part(&mut assistant, part_id, Utc::now())?;
            self.append_text_delta(&mut assistant, part_id, reasoning_text.as_str())?;
            complete_part_status(&mut assistant, part_id)?;
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
            self.finalize_pending_tool_calls(&mut run, &mut assistant, pending_calls)
                .await?;
        }

        if let Some(err) = provider_err {
            let terminal_status = if cancelled {
                ExecutionStatus::Cancelled
            } else {
                ExecutionStatus::Failed
            };
            assistant
                .transition_state(terminal_status)
                .map_err(|err| AppError::Internal(err.to_string()))?;

            if !cancelled {
                let failure = err.failure();
                tracing::warn!(
                    failure_id = %failure.id,
                    session_id = run.session_id,
                    diagnostic = %err,
                    "provider stream failed"
                );
            }
            return Ok(SessionRunResult {
                assistant_message_id,
                state: vec![assistant],
                provider_metadata,
                termination: if cancelled {
                    SessionRunTermination::Cancelled
                } else {
                    SessionRunTermination::Failed(err)
                },
                follow_up_requested: false,
                finish_reason: FinishReason::Stop,
            });
        }

        // Successful run: drive terminal state on the message snapshot.
        if assistant.state == ExecutionStatus::Pending {
            assistant
                .transition_state(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        assistant
            .transition_state(ExecutionStatus::Completed)
            .map_err(|err| AppError::Internal(err.to_string()))?;
        assistant.usage = usage.clone();
        assistant.provider_state = provider_metadata
            .as_ref()
            .and_then(message_provider_state_from_provider_metadata);

        Ok(SessionRunResult {
            assistant_message_id,
            state: vec![assistant],
            provider_metadata,
            termination: SessionRunTermination::Completed,
            follow_up_requested,
            finish_reason: finish_reason_enum,
        })
    }

    pub(crate) fn prompt_exceeds_budget(
        &self,
        messages: &[Message],
        max_prompt_chars: usize,
    ) -> bool {
        self.context_governor.prompt_exceeds_budget(
            crate::session::prompt_window::approximate_prompt_payload_chars(messages),
            max_prompt_chars,
        )
    }

    pub(crate) fn max_prompt_chars(&self) -> usize {
        self.context_governor.max_prompt_chars()
    }

    pub(crate) fn supports_prompt_continuation(&self, model: &ModelRef) -> bool {
        self.provider_registry
            .supports_prompt_continuation(model)
            .unwrap_or(false)
    }

    pub(crate) fn prompt_cache_shape(
        &self,
        model: &ModelRef,
    ) -> Result<Option<agena_provider::PromptCacheShape>, AppError> {
        Ok(self.provider_registry.prompt_cache_shape(model)?)
    }

    pub(crate) fn model_metadata(
        &self,
        model: &ModelRef,
    ) -> Result<agena_domain::ModelMetadata, AppError> {
        Ok(self.provider_registry.model_metadata(model)?)
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
