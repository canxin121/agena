use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::{DateTime, Utc};
use futures_util::StreamExt;
use tracing::Instrument;

use crate::error::AppError;
use crate::event::{
    ErrorInfo, EventKind, EventPublisher, MessagePartDeltaEvent, MessagePartUpdatedEvent,
    PartDeltaField, PublishContext, StreamErrorEvent,
};
use crate::message::{
    AssistantReasoningField, ExecutionStatus, Message, MessageMetadata, MessagePart,
    MessageProviderState, MessageSource, MessageStatus, OperationPart, PartContent, ReasoningPart,
    StructuredObject, TimeRange, ToolInvocation,
};
use crate::model::ModelRef;
use crate::plugin::registry::RegisteredTool;
use crate::provider::{
    CompletionFinishReason, CompletionRequest, CompletionStreamEvent, ProviderRegistry,
};
use crate::role::Role;

use super::history::{
    FinishReason, MessageId as HistoryMessageId, MessageIdAllocator, RunBuffer, RunId, ToolCallId,
};
use super::{context_governor::ContextGovernor, store::ProcessorPartIdAllocator};

const REASONING_PLACEHOLDER: &str = "(no reasoning recorded)";

#[derive(Clone)]
pub(crate) struct SessionRunRequest {
    pub session_id: i64,
    pub model: ModelRef,
    pub model_thinking_mode: Option<String>,
    pub model_speed_mode: Option<String>,
    pub completion: CompletionRequest,
    pub next_message_id: i64,
    pub part_ids: ProcessorPartIdAllocator,
    pub next_call_id: i64,
    /// Live publisher used to push streaming events ("running") onto the
    /// unified bus while the run is in flight. `None` keeps test harnesses
    /// terse — they observe the buffered `client_events` on the result.
    pub event_publisher: Option<Arc<EventPublisher>>,
    /// Optional cancel handle. When the token fires the stream loop
    /// terminates between provider events and surfaces a `RunAbortReason::
    /// Cancelled`-shaped terminal error. `None` keeps the legacy "run to
    /// completion" semantics for callers that don't have a control object.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Debug)]
pub(crate) struct SessionRunResult {
    pub assistant_message_id: i64,
    pub state: Vec<Message>,
    /// UI-projection events buffered during the run (also pushed onto the
    /// bus when `event_publisher` was set).
    pub client_events: Vec<EventKind>,
    pub provider_metadata: Option<serde_json::Value>,
    pub terminal_error: Option<AppError>,
    /// Append-only history events emitted by the run buffer. Routed by the
    /// manager into `SessionStore::append_history_items`.
    pub history_items: Vec<EventKind>,
    /// The run id used by `history_items` — the manager wraps this with
    /// `RunStarted` / `RunCompleted` / `RunAborted` boundary events.
    pub run_id: RunId,
}

#[derive(Clone)]
pub struct SessionProcessor {
    provider_registry: Arc<ProviderRegistry>,
    context_governor: ContextGovernor,
    plugins: Option<Arc<crate::plugin::PluginHost>>,
}

impl SessionProcessor {
    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
    ) -> Self {
        Self {
            provider_registry,
            context_governor,
            plugins: None,
        }
    }

    pub fn with_plugin_host(mut self, plugins: Arc<crate::plugin::PluginHost>) -> Self {
        self.plugins = Some(plugins);
        self
    }

    pub(crate) fn provider_registry(&self) -> &Arc<ProviderRegistry> {
        &self.provider_registry
    }

    /// Apply the `chat.params` plugin hook chain to a [`CompletionRequest`]
    /// before sending it to the provider.
    async fn apply_chat_params_hook(
        &self,
        provider_id: &str,
        model_id: &str,
        request: &mut CompletionRequest,
    ) {
        let Some(plugins) = &self.plugins else { return };
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
        let input = crate::plugin::ChatParamsInput {
            provider: provider_id.to_string(),
            model: model_id.to_string(),
            params: serde_json::Value::Object(params),
        };
        match plugins.dispatch_chat_params(input).await {
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
        let mut client_events = Vec::new();
        // Provider-visible prompt content is append-only for prompt-cache
        // affinity. Mutating chat hooks can rewrite/drop/reorder system and
        // message content, so they remain registered for compatibility but are
        // not applied on the provider request path.
        self.apply_chat_params_hook(
            run.model.provider_id.as_str(),
            run.model.model_id.as_str(),
            &mut run.completion,
        )
        .await;
        let stream_result = self
            .provider_registry
            .complete_stream(&run.model, run.completion.clone())
            .instrument(processor_span.clone())
            .await;
        crate::metrics::record_provider_stream();
        crate::metrics::record_provider_call(stream_result.is_ok());
        let mut stream = stream_result?;

        let assistant_message_id = run.next_message_id;
        run.next_message_id += 1;

        let assistant_metadata = MessageMetadata {
            source: MessageSource::Assistant,
            parent_message_id: run.completion.messages.last().map(|message| message.id),
            generated_by_call_id: None,
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
            state: MessageStatus::Pending,
            parts: Vec::new(),
            created_at: started_at,
            metadata: assistant_metadata.clone(),
            provider_state: None,
            usage: None,
        };

        let run_id = RunId::new();
        let mut run_buffer = RunBuffer::new(run_id);
        let mut id_provider = FixedAssistantId::new(assistant_message_id);
        run_buffer.begin_assistant(&mut id_provider);
        if let Err(err) = run_buffer.set_metadata(assistant_metadata.clone()) {
            return Err(AppError::Internal(err.to_string()));
        }

        let mut active_text_part: Option<i64> = None;
        let mut active_reasoning_part: Option<i64> = None;
        let mut pending_calls: BTreeMap<String, PendingToolCall> = BTreeMap::new();
        let mut part_delta_sequences = BTreeMap::<i64, u64>::new();
        let mut provider_err: Option<AppError> = None;
        let mut usage = None;
        let mut finish_reason_enum = FinishReason::default();
        let mut provider_metadata = None;
        let mut visible_text = String::new();
        let mut reasoning_text = String::new();
        let mut saw_tool_call = false;

        let cancel = run.cancel.clone();
        loop {
            let next = match cancel.as_ref() {
                Some(token) => tokio::select! {
                    biased;
                    _ = token.cancelled() => None,
                    item = stream.next() => item,
                },
                None => stream.next().await,
            };
            let Some(item) = next else { break };
            match item {
                Ok(CompletionStreamEvent::TextDelta { delta, .. }) => {
                    visible_text.push_str(delta.as_str());
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    let part_id = match active_text_part {
                        Some(part_id) => part_id,
                        None => {
                            let part_id = run.part_ids.reserve().await?;
                            self.start_text_part(&mut assistant, part_id, Utc::now())?;
                            self.emit_part_updated(&run, &assistant, part_id).await?;
                            active_text_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_text_delta(&mut assistant, part_id, delta.as_str())?;
                    run_buffer
                        .push_text_delta(delta.as_str())
                        .map_err(|err| AppError::Internal(err.to_string()))?;

                    let seq = part_delta_sequences.entry(part_id).or_default();
                    *seq += 1;
                    self.emit_part_delta(
                        &run,
                        &assistant,
                        part_id,
                        None,
                        PartDeltaField::Text,
                        delta,
                        *seq,
                    )
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
                        complete_part_status(&mut assistant, part_id)?;
                    }
                    if let Some(part_id) = active_reasoning_part.take() {
                        complete_part_status(&mut assistant, part_id)?;
                    }

                    let pending = pending_calls.entry(stream_key).or_default();
                    if let Some(id) = id {
                        pending.id = Some(id);
                    }
                    if let Some(name) = name {
                        pending.name = Some(canonical_tool_name_from_model_name(
                            name.as_str(),
                            run.completion.tools.as_slice(),
                        ));
                    }
                    pending.arguments_json.push_str(arguments_delta.as_str());
                    self.ensure_pending_tool_call_part(
                        &mut run,
                        &mut assistant,
                        &mut run_buffer,
                        pending,
                    )
                    .await?;
                    if !arguments_delta.is_empty()
                        && let Some(history_call_id) = pending.history_call_id.as_ref()
                    {
                        run_buffer
                            .append_tool_arguments(history_call_id, arguments_delta.as_str())
                            .map_err(|err| AppError::Internal(err.to_string()))?;
                    }
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

                    let pending = pending_calls.entry(stream_key).or_default();
                    if let Some(id) = id {
                        pending.id = Some(id);
                    }
                    if let Some(name) = name {
                        pending.name = Some(canonical_tool_name_from_model_name(
                            name.as_str(),
                            run.completion.tools.as_slice(),
                        ));
                    }
                    pending.arguments_json = arguments_json.clone();
                    self.ensure_pending_tool_call_part(
                        &mut run,
                        &mut assistant,
                        &mut run_buffer,
                        pending,
                    )
                    .await?;
                    if let Some(history_call_id) = pending.history_call_id.as_ref() {
                        run_buffer
                            .replace_tool_arguments(history_call_id, arguments_json)
                            .map_err(|err| AppError::Internal(err.to_string()))?;
                    }
                }
                Ok(CompletionStreamEvent::Completed {
                    finish_reason,
                    usage: usage_value,
                    provider_metadata: completed_provider_metadata,
                    ..
                }) => {
                    usage = usage_value.map(Into::into);
                    if let Some(reason) = finish_reason.as_ref() {
                        finish_reason_enum = map_finish_reason(reason);
                    }
                    provider_metadata = completed_provider_metadata;
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
                            self.emit_part_updated(&run, &assistant, part_id).await?;
                            active_reasoning_part = Some(part_id);
                            part_id
                        }
                    };

                    self.append_reasoning_delta(&mut assistant, part_id, delta.as_str())?;
                    run_buffer
                        .push_reasoning_delta(delta.as_str())
                        .map_err(|err| AppError::Internal(err.to_string()))?;

                    let seq = part_delta_sequences.entry(part_id).or_default();
                    *seq += 1;
                    self.emit_part_delta(
                        &run,
                        &assistant,
                        part_id,
                        None,
                        PartDeltaField::ReasoningSummary,
                        delta,
                        *seq,
                    )
                    .await?;
                }
                Err(err) => {
                    provider_err = Some(err);
                    break;
                }
            }
        }

        // If the cancel token tripped, the loop above broke without an
        // explicit provider error. Surface a synthetic terminal error so
        // the caller knows the run was cancelled rather than completed.
        if provider_err.is_none()
            && let Some(token) = cancel.as_ref()
            && token.is_cancelled()
        {
            provider_err = Some(AppError::Internal("run cancelled by user".to_string()));
        }

        if let Some(part_id) = active_text_part {
            complete_part_status(&mut assistant, part_id)?;
        }
        if let Some(part_id) = active_reasoning_part {
            complete_part_status(&mut assistant, part_id)?;
        }

        if provider_err.is_none()
            && visible_text.trim().is_empty()
            && !saw_tool_call
            && !reasoning_text.trim().is_empty()
            && reasoning_text.trim() != REASONING_PLACEHOLDER
        {
            let part_id = run.part_ids.reserve().await?;
            self.start_text_part(&mut assistant, part_id, Utc::now())?;
            self.emit_part_updated(&run, &assistant, part_id).await?;
            self.append_text_delta(&mut assistant, part_id, reasoning_text.as_str())?;
            run_buffer
                .push_text_delta(reasoning_text.as_str())
                .map_err(|err| AppError::Internal(err.to_string()))?;

            let seq = part_delta_sequences.entry(part_id).or_default();
            *seq += 1;
            self.emit_part_delta(
                &run,
                &assistant,
                part_id,
                None,
                PartDeltaField::Text,
                reasoning_text,
                *seq,
            )
            .await?;
            complete_part_status(&mut assistant, part_id)?;
        }

        self.finalize_pending_tool_calls(&mut run, &mut assistant, &mut run_buffer, pending_calls)
            .await?;

        if let Some(err) = provider_err {
            assistant.state = MessageStatus::Failed;

            client_events.push(EventKind::StreamError(StreamErrorEvent {
                session_id: run.session_id,
                error: ErrorInfo {
                    code: "provider_stream_error".to_string(),
                    message: err.to_string(),
                },
                ts_ms: Utc::now().timestamp_millis(),
            }));
            // Even on failure the buffer has accumulated state we can still
            // commit; downstream callers may inspect it for diagnostics.
            let mut history_items = run_buffer
                .commit(
                    &mut crate::session::history::SequentialIdAllocator::starting_at(
                        run.next_message_id.saturating_add(1),
                    ),
                )
                .unwrap_or_default();
            sync_assistant_completion_event(history_items.as_mut_slice(), &assistant);
            return Ok(SessionRunResult {
                assistant_message_id,
                state: vec![assistant],
                client_events,
                provider_metadata,
                terminal_error: Some(err),
                history_items,
                run_id,
            });
        }

        // Successful run: drive terminal state on the message snapshot and
        // reflect the same finish/usage on the run buffer for history.
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        let _ = assistant.transition_state(MessageStatus::Completed);
        assistant.usage = usage.clone();
        assistant.provider_state = provider_metadata
            .as_ref()
            .and_then(message_provider_state_from_provider_metadata);
        run_buffer
            .set_provider_state(assistant.provider_state.clone())
            .map_err(|err| AppError::Internal(err.to_string()))?;

        if let Some(usage_ref) = usage {
            run_buffer
                .set_usage(usage_ref)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        run_buffer
            .set_finish_reason(finish_reason_enum)
            .map_err(|err| AppError::Internal(err.to_string()))?;

        let mut history_items = run_buffer
            .commit(
                &mut crate::session::history::SequentialIdAllocator::starting_at(
                    run.next_message_id.saturating_add(1),
                ),
            )
            .map_err(|err| AppError::Internal(err.to_string()))?;
        sync_assistant_completion_event(history_items.as_mut_slice(), &assistant);

        Ok(SessionRunResult {
            assistant_message_id,
            state: vec![assistant],
            client_events,
            provider_metadata,
            terminal_error: None,
            history_items,
            run_id,
        })
    }

    pub(crate) fn prompt_exceeds_budget(
        &self,
        messages: &[Message],
        max_prompt_chars: usize,
    ) -> bool {
        self.context_governor
            .prompt_exceeds_budget(messages, max_prompt_chars)
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
    ) -> Result<Option<crate::provider::PromptCacheShape>, AppError> {
        self.provider_registry.prompt_cache_shape(model)
    }

    pub(crate) fn model_metadata(
        &self,
        model: &ModelRef,
    ) -> Result<crate::provider::ModelMetadata, AppError> {
        self.provider_registry.model_metadata(model)
    }

    fn start_text_part(
        &self,
        assistant: &mut Message,
        part_id: i64,
        created_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let mut part = MessagePart::with_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::Pending,
            PartContent::text(String::new()),
        );
        part.part_index = assistant.parts.len() as i32;
        assistant.parts.push(part);
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        Ok(())
    }

    fn start_reasoning_part(
        &self,
        assistant: &mut Message,
        part_id: i64,
        created_at: DateTime<Utc>,
    ) -> Result<(), AppError> {
        let mut part = MessagePart::with_content(
            part_id,
            assistant.id,
            created_at,
            ExecutionStatus::Pending,
            PartContent::Reasoning(ReasoningPart {
                summary: Vec::new(),
                raw_content: Vec::new(),
                encrypted_content: None,
            }),
        );
        part.part_index = assistant.parts.len() as i32;
        assistant.parts.push(part);
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        Ok(())
    }

    fn append_text_delta(
        &self,
        assistant: &mut Message,
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
        if part.status == ExecutionStatus::Pending {
            part.transition_status(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        if !part.append_text_delta(delta) {
            return Err(AppError::Internal(format!(
                "failed to append text delta to part {part_id}: kind mismatch"
            )));
        }
        Ok(())
    }

    fn append_reasoning_delta(
        &self,
        assistant: &mut Message,
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
        if part.status == ExecutionStatus::Pending {
            part.transition_status(ExecutionStatus::InProgress)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        if !part.append_reasoning_summary_delta(delta.to_string()) {
            return Err(AppError::Internal(format!(
                "failed to append reasoning delta to part {part_id}: kind mismatch"
            )));
        }
        Ok(())
    }

    async fn ensure_pending_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        run_buffer: &mut RunBuffer,
        pending: &mut PendingToolCall,
    ) -> Result<(), AppError> {
        let mut should_emit = false;
        if pending.part_id.is_none() {
            let part_id = run.part_ids.reserve().await?;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = Utc::now();
            let invocation = placeholder_tool_invocation(
                pending.name.as_deref(),
                run.completion.tools.as_slice(),
            );
            let mut part = MessagePart::with_content(
                part_id,
                assistant.id,
                start,
                ExecutionStatus::Pending,
                PartContent::Operation(OperationPart::pending(
                    call_id,
                    invocation,
                    tool_execution_title(pending.name.as_deref()),
                    TimeRange {
                        start_ms: start.timestamp_millis(),
                        end_ms: None,
                    },
                )),
            );
            part.part_index = assistant.parts.len() as i32;
            assistant.parts.push(part);
            if assistant.state == MessageStatus::Pending {
                let _ = assistant.transition_state(MessageStatus::InProgress);
            }

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
            should_emit = true;

            // Mirror into RunBuffer with a stable history-side call id.
            // Prefer the provider-supplied id when present; otherwise fall
            // back to a synthetic one derived from the integer call_id so it
            // remains stable for the lifetime of this run.
            let history_call_id = match pending.id.as_deref() {
                Some(id) if !id.trim().is_empty() => ToolCallId::new(id),
                _ => ToolCallId::new(format!("call_{call_id}")),
            };
            run_buffer
                .start_tool_call(history_call_id.clone())
                .map_err(|err| AppError::Internal(err.to_string()))?;
            if let Some(name) = pending.name.as_deref() {
                run_buffer
                    .name_tool_call(&history_call_id, name)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            pending.history_call_id = Some(history_call_id);
        } else if pending.history_call_id.is_some() {
            if let Some(provider_call_id) = pending.id.as_deref().filter(|id| !id.trim().is_empty())
            {
                let next_history_call_id = ToolCallId::new(provider_call_id);
                let should_replace = pending
                    .history_call_id
                    .as_ref()
                    .is_some_and(|history_call_id| history_call_id != &next_history_call_id);
                if should_replace {
                    let current_history_call_id =
                        pending.history_call_id.clone().expect("checked above");
                    run_buffer
                        .replace_tool_call_id(
                            &current_history_call_id,
                            next_history_call_id.clone(),
                        )
                        .map_err(|err| AppError::Internal(err.to_string()))?;
                    pending.history_call_id = Some(next_history_call_id);
                }
            }

            if let Some(history_call_id) = pending.history_call_id.as_ref()
                && let Some(name) = pending.name.as_deref()
            {
                // A second name fragment can arrive after the part already exists.
                // Re-set the name; RunBuffer accepts repeated assignment.
                run_buffer
                    .name_tool_call(history_call_id, name)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
        }

        if let (Some(part_id), Some(operation_id)) = (
            pending.part_id,
            pending
                .id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .cloned(),
        ) {
            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from assistant snapshot: {part_id}"
                    ))
                })?;
            if part.operation_id.as_deref() != Some(operation_id.as_str()) {
                part.operation_id = Some(operation_id);
                should_emit = true;
            }
        }

        if should_emit && let Some(part_id) = pending.part_id {
            self.emit_part_updated(run, assistant, part_id).await?;
        }

        Ok(())
    }

    async fn finalize_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        assistant: &mut Message,
        run_buffer: &mut RunBuffer,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, assistant, run_buffer, &mut pending)
                .await?;

            let tool_name = pending.name.unwrap_or_else(|| "unknown".to_string());
            let invocation = if tool_for_model_name(&tool_name, run.completion.tools.as_slice())
                .is_some()
            {
                parse_tool_invocation(
                    tool_name.as_str(),
                    pending.arguments_json.as_str(),
                    run.completion.tools.as_slice(),
                )?
            } else {
                tracing::debug!(
                    target: "agena::session::processor",
                    session_id = run.session_id,
                    tool = %tool_name,
                    "model requested unsupported tool; preserving call for tool-failure handling"
                );
                placeholder_tool_invocation(
                    Some(tool_name.as_str()),
                    run.completion.tools.as_slice(),
                )
            };
            let Some(part_id) = pending.part_id else {
                continue;
            };
            let call_id = pending.call_id.unwrap_or(0);

            let part = assistant
                .parts
                .iter_mut()
                .find(|part| part.id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "tool part missing from assistant snapshot: {part_id}"
                    ))
                })?;
            part.set_content(PartContent::Operation(OperationPart::pending(
                call_id,
                invocation,
                tool_execution_title(Some(tool_name.as_str())),
                TimeRange {
                    start_ms: pending.started_at_ms.unwrap_or_default(),
                    end_ms: None,
                },
            )));

            // Re-assert name on RunBuffer (final, authoritative). The
            // accumulated `arguments_json` was already streamed in chunks via
            // `append_tool_arguments`; we don't repeat it here.
            if let Some(history_call_id) = pending.history_call_id.as_ref() {
                run_buffer
                    .name_tool_call(history_call_id, tool_name.as_str())
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }

            self.emit_part_updated(run, assistant, part_id).await?;
        }

        Ok(())
    }

    async fn emit_part_updated(
        &self,
        run: &SessionRunRequest,
        assistant: &Message,
        part_id: i64,
    ) -> Result<(), AppError> {
        let Some(publisher) = run.event_publisher.as_ref() else {
            return Ok(());
        };

        let part = assistant
            .parts
            .iter()
            .find(|part| part.id == part_id)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "part snapshot not found for stream event: {part_id}"
                ))
            })?;
        let kind = EventKind::MessagePartUpdated(MessagePartUpdatedEvent {
            session_id: run.session_id,
            message_id: assistant.id,
            message_role: assistant.role,
            message_state: assistant.state,
            message_created_at: assistant.created_at,
            part,
            ts_ms: Utc::now().timestamp_millis(),
        });
        publisher
            .publish(PublishContext::for_session(run.session_id), kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish part-updated failed: {err}")))?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    async fn emit_part_delta(
        &self,
        run: &SessionRunRequest,
        assistant: &Message,
        part_id: i64,
        call_id: Option<i64>,
        field: PartDeltaField,
        delta: String,
        seq: u64,
    ) -> Result<(), AppError> {
        let Some(publisher) = run.event_publisher.as_ref() else {
            return Ok(());
        };

        let _ = assistant; // assistant snapshot is no longer needed: events
        // carry their own routing context.
        let kind = EventKind::MessagePartDelta(MessagePartDeltaEvent {
            session_id: run.session_id,
            message_id: assistant.id,
            part_id,
            call_id,
            field,
            delta,
            seq,
            ts_ms: Utc::now().timestamp_millis(),
        });
        publisher
            .publish(PublishContext::for_session(run.session_id), kind)
            .await
            .map_err(|err| AppError::Internal(format!("publish part-delta failed: {err}")))?;
        Ok(())
    }
}

/// Adapter that returns a single, pre-allocated `MessageId` to satisfy the
/// `RunBuffer` API. The processor reserves message ids via the global session
/// allocator before opening the buffer, so the buffer must adopt that id
/// rather than mint its own.
struct FixedAssistantId {
    next: Option<HistoryMessageId>,
}

impl FixedAssistantId {
    fn new(message_id: i64) -> Self {
        Self {
            next: Some(HistoryMessageId(message_id)),
        }
    }
}

impl MessageIdAllocator for FixedAssistantId {
    fn next_message_id(&mut self) -> HistoryMessageId {
        self.next
            .take()
            .expect("FixedAssistantId only yields one id per run")
    }
}

fn complete_part_status(assistant: &mut Message, part_id: i64) -> Result<(), AppError> {
    let part = assistant
        .parts
        .iter_mut()
        .find(|part| part.id == part_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "completing missing part on assistant snapshot: {part_id}"
            ))
        })?;
    if part.status == ExecutionStatus::InProgress {
        part.transition_status(ExecutionStatus::Completed)
            .map_err(|err| AppError::Internal(err.to_string()))?;
    }
    Ok(())
}

fn sync_assistant_completion_event(history_items: &mut [EventKind], assistant: &Message) {
    for event in history_items {
        let EventKind::AssistantMessageCompleted(payload) = event else {
            continue;
        };
        if payload.message_id.raw() != assistant.id {
            continue;
        }
        payload.parts = assistant.parts.clone();
        payload.usage = assistant.usage.clone();
        payload.metadata = assistant.metadata.clone();
        payload.provider_state = assistant.provider_state.clone();
    }
}

fn map_finish_reason(reason: &CompletionFinishReason) -> FinishReason {
    match reason {
        CompletionFinishReason::Stop => FinishReason::Stop,
        CompletionFinishReason::ToolCalls => FinishReason::ToolCalls,
        CompletionFinishReason::Length => FinishReason::MaxTokens,
        CompletionFinishReason::ContentFilter => FinishReason::ContentFilter,
        CompletionFinishReason::Other(_) => FinishReason::Other,
    }
}

fn message_provider_state_from_provider_metadata(
    provider_metadata: &serde_json::Value,
) -> Option<MessageProviderState> {
    let assistant_reasoning_field = provider_metadata
        .as_object()
        .and_then(|metadata| metadata.get("assistant_reasoning_field"))
        .and_then(|value| value.as_str())
        .and_then(|value| match value {
            "reasoning_content" => Some(AssistantReasoningField::ReasoningContent),
            "reasoning_details" => Some(AssistantReasoningField::ReasoningDetails),
            _ => None,
        });
    let state = MessageProviderState {
        assistant_reasoning_field,
    };
    (!state.is_empty()).then_some(state)
}

#[derive(Debug, Default, Clone)]
struct PendingToolCall {
    part_id: Option<i64>,
    call_id: Option<i64>,
    started_at_ms: Option<i64>,
    id: Option<String>,
    name: Option<String>,
    arguments_json: String,
    /// History-side call identifier propagated to `RunBuffer`. Set the first
    /// time the part is materialized and reused for every subsequent argument
    /// fragment so chunks land on the right tool.
    history_call_id: Option<ToolCallId>,
}

fn tool_execution_title(name: Option<&str>) -> String {
    format!("Tool {}", name.unwrap_or("unknown").trim())
}

fn placeholder_tool_invocation(
    name: Option<&str>,
    available_tools: &[RegisteredTool],
) -> ToolInvocation {
    let requested_name = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let Some(tool) = available_tools
        .iter()
        .find(|tool| crate::tool::tool_matches_model_name(tool, requested_name))
    else {
        return ToolInvocation {
            name: requested_name.to_string(),
            plugin_name: None,
            input: StructuredObject::default(),
        };
    };

    tool_invocation_for_definition(tool, StructuredObject::default())
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[RegisteredTool],
) -> Result<ToolInvocation, AppError> {
    let trimmed_name = name.trim();
    let tool = tool_for_model_name(trimmed_name, available_tools).ok_or_else(|| {
        AppError::Provider(format!("unsupported tool call from model: {trimmed_name}"))
    })?;

    let parsed = parse_custom_input(arguments_json)?;
    Ok(tool_invocation_for_definition(tool, parsed))
}

fn tool_for_model_name<'a>(
    name: &str,
    available_tools: &'a [RegisteredTool],
) -> Option<&'a RegisteredTool> {
    available_tools
        .iter()
        .find(|tool| crate::tool::tool_matches_model_name(tool, name))
}

fn canonical_tool_name_from_model_name(name: &str, available_tools: &[RegisteredTool]) -> String {
    tool_for_model_name(name, available_tools)
        .map(|tool| tool.exposed_name.clone())
        .unwrap_or_else(|| name.trim().to_owned())
}

fn tool_invocation_for_definition(
    tool: &RegisteredTool,
    input: StructuredObject,
) -> ToolInvocation {
    ToolInvocation {
        name: tool.exposed_name.clone(),
        plugin_name: Some(tool.plugin_name.clone()),
        input,
    }
}

fn parse_custom_input(arguments_json: &str) -> Result<StructuredObject, AppError> {
    let value = parse_json_body::<serde_json::Value>(arguments_json)?;
    StructuredObject::try_from(value)
        .map_err(|err| AppError::Internal(format!("invalid custom tool input: {err}")))
}

fn parse_json_body<T>(arguments_json: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    let body = if arguments_json.trim().is_empty() {
        "{}"
    } else {
        arguments_json
    };

    let mut deserializer = serde_json::Deserializer::from_str(body);
    let parsed =
        <T as serde::Deserialize>::deserialize(&mut deserializer).map_err(AppError::from)?;

    if let Err(err) = deserializer.end() {
        tracing::warn!(
            error = %err,
            arguments_len = body.len(),
            "tool arguments included trailing content; ignored suffix after valid JSON prefix"
        );
    }

    Ok(parsed)
}
