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
    ExecutionStatus, Message, MessageMetadata, MessagePart, MessageSource, MessageStatus,
    PartContent, ReasoningPart, StructuredObject, TimeRange, ToolExecutionPart, ToolInvocation,
};
use crate::model::ModelRef;
use crate::provider::{
    CompletionFinishReason, CompletionRequest, CompletionStreamEvent, ProviderRegistry,
};
use crate::role::Role;
use crate::tool::EntryDefinition;

use super::history::{
    FinishReason, MessageId as HistoryMessageId, MessageIdAllocator, ToolCallId, TurnBuffer, TurnId,
};
use super::{context_governor::ContextGovernor, store::ProcessorPartIdAllocator};

#[derive(Clone)]
pub(crate) struct SessionRunRequest {
    pub session_id: i64,
    pub model: ModelRef,
    pub completion: CompletionRequest,
    pub next_message_id: i64,
    pub part_ids: ProcessorPartIdAllocator,
    pub next_call_id: i64,
    /// Live publisher used to push streaming events ("running") onto the
    /// unified bus while the turn is in flight. `None` keeps test harnesses
    /// terse — they observe the buffered `client_events` on the result.
    pub event_publisher: Option<Arc<EventPublisher>>,
    /// Optional cancel handle. When the token fires the stream loop
    /// terminates between provider events and surfaces a `TurnAbortReason::
    /// Cancelled`-shaped terminal error. `None` keeps the legacy "run to
    /// completion" semantics for callers that don't have a control object.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Debug)]
pub(crate) struct SessionRunResult {
    pub assistant_message_id: i64,
    pub state: Vec<Message>,
    /// UI-projection events buffered during the turn (also pushed onto the
    /// bus when `event_publisher` was set).
    pub client_events: Vec<EventKind>,
    pub provider_metadata: Option<serde_json::Value>,
    pub terminal_error: Option<AppError>,
    /// Append-only history events emitted by the turn buffer. Routed by the
    /// manager into `SessionStore::append_history_items`.
    pub history_items: Vec<EventKind>,
    /// The turn id used by `history_items` — the manager wraps this with
    /// `TurnStarted` / `TurnCompleted` / `TurnAborted` boundary events.
    pub turn_id: TurnId,
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

    /// Apply the experimental `chat.system.transform` hook chain to the
    /// system prompt before sending it to the provider.
    async fn apply_chat_system_transform_hook(
        &self,
        session_id: i64,
        request: &mut CompletionRequest,
    ) {
        let Some(plugins) = &self.plugins else { return };
        if plugins.is_empty() {
            return;
        }
        let current = request.system.clone().unwrap_or_default();
        let input = crate::plugin::ChatSystemTransformInput {
            session_id,
            current_system: current,
        };
        match plugins.dispatch_chat_system_transform(input).await {
            Ok(updated) => {
                request.system = if updated.current_system.is_empty() {
                    None
                } else {
                    Some(updated.current_system)
                };
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::chat_system_transform",
                    "chat.system.transform hook failed: {err}"
                );
            }
        }
    }

    /// Apply the `chat.message` hook chain to every outgoing message before
    /// the provider request goes out. Messages whose `content` becomes
    /// `Value::Null` (the SDK's drop signal) are filtered out.
    async fn apply_chat_message_hook(&self, session_id: i64, request: &mut CompletionRequest) {
        let Some(plugins) = &self.plugins else { return };
        if plugins.is_empty() {
            return;
        }
        let mut kept: Vec<crate::message::Message> = Vec::with_capacity(request.messages.len());
        for mut msg in std::mem::take(&mut request.messages) {
            let content_json = match serde_json::to_value(&msg.parts) {
                Ok(v) => v,
                Err(_) => {
                    kept.push(msg);
                    continue;
                }
            };
            let role = match msg.role {
                crate::role::Role::User => "user",
                crate::role::Role::Assistant => "assistant",
                crate::role::Role::Tool => "tool",
                crate::role::Role::System => "system",
            };
            let chat_msg = crate::plugin::ChatMessage {
                role: role.to_string(),
                content: content_json,
            };
            let input = crate::plugin::ChatMessageInput {
                session_id,
                direction: crate::plugin::ChatDirection::ToProvider,
                message: chat_msg,
            };
            match plugins.dispatch_chat_message(input).await {
                Ok(after) => {
                    if matches!(after.message.content, serde_json::Value::Null) {
                        continue;
                    }
                    // Re-serialize the (potentially patched) content back onto msg.
                    if let Ok(patched) = serde_json::from_value::<Vec<crate::message::MessagePart>>(
                        after.message.content,
                    ) {
                        msg.parts = patched;
                    }
                    kept.push(msg);
                }
                Err(err) => {
                    tracing::warn!(
                        target: "agena_plugin_host::chat_message",
                        "chat.message hook failed (keeping original): {err}"
                    );
                    kept.push(msg);
                }
            }
        }
        request.messages = kept;
    }

    /// Apply the `chat.messages.transform` hook: dispatches the entire outgoing
    /// message list as `ChatMessage` SDK values; plugins can add, remove, or
    /// reorder messages wholesale.
    async fn apply_chat_messages_transform_hook(
        &self,
        session_id: i64,
        request: &mut CompletionRequest,
    ) {
        let Some(plugins) = &self.plugins else { return };
        if plugins.is_empty() {
            return;
        }
        let sdk_messages: Vec<crate::plugin::ChatMessage> = request
            .messages
            .iter()
            .filter_map(|msg| {
                let content = serde_json::to_value(&msg.parts).ok()?;
                let role = match msg.role {
                    crate::role::Role::User => "user",
                    crate::role::Role::Assistant => "assistant",
                    crate::role::Role::Tool => "tool",
                    crate::role::Role::System => "system",
                };
                Some(crate::plugin::ChatMessage {
                    role: role.to_string(),
                    content,
                })
            })
            .collect();

        let input = crate::plugin::ChatMessagesTransformInput {
            session_id,
            messages: sdk_messages,
        };
        match plugins.dispatch_chat_messages_transform(input).await {
            Ok(updated) => {
                // Rebuild the message list from the patched SDK messages.
                // We keep original Message fields (id, metadata, etc.) for
                // messages whose position still lines up; new or reordered
                // messages get a best-effort reconstruction.
                let mut patched: Vec<crate::message::Message> =
                    Vec::with_capacity(updated.messages.len());
                for (i, sdk_msg) in updated.messages.into_iter().enumerate() {
                    if let Some(original) = request.messages.get(i).filter(|m| {
                        let role = match m.role {
                            crate::role::Role::User => "user",
                            crate::role::Role::Assistant => "assistant",
                            crate::role::Role::Tool => "tool",
                            crate::role::Role::System => "system",
                        };
                        role == sdk_msg.role
                    }) {
                        let mut msg = original.clone();
                        if let Ok(parts) = serde_json::from_value::<Vec<crate::message::MessagePart>>(
                            sdk_msg.content,
                        ) {
                            msg.parts = parts;
                        }
                        patched.push(msg);
                    }
                    // Messages added/moved by plugins are silently dropped —
                    // synthesising valid Message structs from SDK fragments is
                    // not safe without proper id allocation.
                }
                request.messages = patched;
            }
            Err(err) => {
                tracing::warn!(
                    target: "agena_plugin_host::chat_messages_transform",
                    "chat.messages.transform hook failed (keeping original): {err}"
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
        self.apply_chat_system_transform_hook(run.session_id, &mut run.completion)
            .await;
        self.apply_chat_message_hook(run.session_id, &mut run.completion)
            .await;
        self.apply_chat_messages_transform_hook(run.session_id, &mut run.completion)
            .await;
        self.apply_chat_params_hook(
            run.model.provider_id.as_str(),
            run.model.model_id.as_str(),
            &mut run.completion,
        )
        .await;
        let mut stream = self
            .provider_registry
            .complete_stream(&run.model, run.completion.clone())
            .instrument(processor_span.clone())
            .await?;

        let assistant_message_id = run.next_message_id;
        run.next_message_id += 1;

        let assistant_metadata = MessageMetadata {
            source: MessageSource::Assistant,
            parent_message_id: run.completion.messages.last().map(|message| message.id),
            generated_by_call_id: None,
            model_provider_id: run.model.provider_id.to_string(),
            model_id: run.completion.model.to_string(),
            provider_metadata: None,
            tags: Vec::new(),
        };

        let started_at = Utc::now();
        let mut assistant = Message {
            id: assistant_message_id,
            role: Role::Assistant,
            state: MessageStatus::Pending,
            parts: Vec::new(),
            created_at: started_at,
            metadata: assistant_metadata.clone(),
            usage: None,
            finish: None,
        };

        let turn_id = TurnId::new();
        let mut turn_buffer = TurnBuffer::new(turn_id);
        let mut id_provider = FixedAssistantId::new(assistant_message_id);
        turn_buffer.begin_assistant(&mut id_provider);
        if let Err(err) = turn_buffer.set_metadata(assistant_metadata.clone()) {
            return Err(AppError::Internal(err.to_string()));
        }

        let mut active_text_part: Option<i64> = None;
        let mut active_reasoning_part: Option<i64> = None;
        let mut pending_calls: BTreeMap<String, PendingToolCall> = BTreeMap::new();
        let mut part_delta_sequences = BTreeMap::<i64, u64>::new();
        let mut provider_err: Option<AppError> = None;
        let mut usage = None;
        let mut finish = None;
        let mut finish_reason_enum = FinishReason::default();
        let mut provider_metadata = None;

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
                    turn_buffer
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
                        pending.name = Some(name);
                    }
                    pending.arguments_json.push_str(arguments_delta.as_str());
                    self.ensure_pending_tool_call_part(
                        &mut run,
                        &mut assistant,
                        &mut turn_buffer,
                        pending,
                    )
                    .await?;
                    if !arguments_delta.is_empty()
                        && let Some(history_call_id) = pending.history_call_id.as_ref()
                    {
                        turn_buffer
                            .append_tool_arguments(history_call_id, arguments_delta.as_str())
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
                    finish = finish_reason.map(|item| format!("{item:?}"));
                    provider_metadata = completed_provider_metadata;
                }
                Ok(CompletionStreamEvent::ThinkingDelta { delta, .. }) => {
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
                    turn_buffer
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
        // the caller knows the turn was cancelled rather than completed.
        if provider_err.is_none()
            && let Some(token) = cancel.as_ref()
            && token.is_cancelled()
        {
            provider_err = Some(AppError::Internal("turn cancelled by user".to_string()));
        }

        if let Some(part_id) = active_text_part {
            complete_part_status(&mut assistant, part_id)?;
        }
        if let Some(part_id) = active_reasoning_part {
            complete_part_status(&mut assistant, part_id)?;
        }

        self.finalize_pending_tool_calls(&mut run, &mut assistant, &mut turn_buffer, pending_calls)
            .await?;

        if let Some(err) = provider_err {
            assistant.state = MessageStatus::Failed;
            assistant.finish = Some(err.to_string());

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
            let history_items = turn_buffer
                .commit(&mut crate::session::history::SequentialIdAllocator::starting_at(
                    run.next_message_id.saturating_add(1),
                ))
                .unwrap_or_default();
            return Ok(SessionRunResult {
                assistant_message_id,
                state: vec![assistant],
                client_events,
                provider_metadata,
                terminal_error: Some(err),
                history_items,
                turn_id,
            });
        }

        // Successful turn: drive terminal state on the message snapshot and
        // reflect the same finish/usage on the turn buffer for history.
        if assistant.state == MessageStatus::Pending {
            let _ = assistant.transition_state(MessageStatus::InProgress);
        }
        let _ = assistant.transition_state(MessageStatus::Completed);
        assistant.finish = finish;
        assistant.usage = usage.clone();
        assistant.metadata.provider_metadata = provider_metadata.clone();

        if let Some(usage_ref) = usage {
            turn_buffer
                .set_usage(usage_ref)
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }
        turn_buffer
            .set_finish_reason(finish_reason_enum)
            .map_err(|err| AppError::Internal(err.to_string()))?;

        let history_items = turn_buffer
            .commit(&mut crate::session::history::SequentialIdAllocator::starting_at(
                run.next_message_id.saturating_add(1),
            ))
            .map_err(|err| AppError::Internal(err.to_string()))?;

        Ok(SessionRunResult {
            assistant_message_id,
            state: vec![assistant],
            client_events,
            provider_metadata,
            terminal_error: None,
            history_items,
            turn_id,
        })
    }

    pub(crate) fn should_retry_with_compaction(&self, err: &AppError, rounds: u8) -> bool {
        self.context_governor
            .should_retry_with_compaction(err, rounds)
    }

    pub(crate) fn should_compact_prompt_with_budget(
        &self,
        messages: &[Message],
        max_prompt_chars: usize,
    ) -> bool {
        self.context_governor
            .should_compact_prompt_with_budget(messages, max_prompt_chars)
    }

    pub(crate) fn keep_tail_messages(&self) -> usize {
        self.context_governor.keep_tail_messages()
    }

    pub(crate) fn max_prompt_chars(&self) -> usize {
        self.context_governor.max_prompt_chars()
    }

    pub(crate) fn can_retry_compaction(&self, rounds: u8) -> bool {
        self.context_governor.can_retry_compaction(rounds)
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
        turn_buffer: &mut TurnBuffer,
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
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: tool_execution_title(pending.name.as_deref()),
                    lifecycle: TimeRange {
                        start_ms: start.timestamp_millis(),
                        end_ms: None,
                    },
                }),
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

            // Mirror into TurnBuffer with a stable history-side call id.
            // Prefer the provider-supplied id when present; otherwise fall
            // back to a synthetic one derived from the integer call_id so it
            // remains stable for the lifetime of this turn.
            let history_call_id = match pending.id.as_deref() {
                Some(id) if !id.trim().is_empty() => ToolCallId::new(id),
                _ => ToolCallId::new(format!("call_{call_id}")),
            };
            turn_buffer
                .start_tool_call(history_call_id.clone())
                .map_err(|err| AppError::Internal(err.to_string()))?;
            if let Some(name) = pending.name.as_deref() {
                turn_buffer
                    .name_tool_call(&history_call_id, name)
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            // Replay any argument fragments that arrived before we knew the
            // call existed (uncommon, but possible if name-only deltas arrived
            // first).
            if !pending.arguments_json.is_empty() {
                turn_buffer
                    .append_tool_arguments(&history_call_id, pending.arguments_json.as_str())
                    .map_err(|err| AppError::Internal(err.to_string()))?;
            }
            pending.history_call_id = Some(history_call_id);
        } else if let Some(history_call_id) = pending.history_call_id.as_ref()
            && let Some(name) = pending.name.as_deref()
        {
            // A second name fragment can arrive after the part already exists.
            // Re-set the name; TurnBuffer accepts repeated assignment.
            turn_buffer
                .name_tool_call(history_call_id, name)
                .map_err(|err| AppError::Internal(err.to_string()))?;
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
        turn_buffer: &mut TurnBuffer,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, assistant, turn_buffer, &mut pending)
                .await?;

            let tool_name = pending.name.unwrap_or_else(|| "unknown".to_string());
            let invocation = parse_tool_invocation(
                tool_name.as_str(),
                pending.arguments_json.as_str(),
                run.completion.tools.as_slice(),
            )?;
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
            part.set_content(PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id,
                invocation,
                title: tool_execution_title(Some(tool_name.as_str())),
                lifecycle: TimeRange {
                    start_ms: pending.started_at_ms.unwrap_or_default(),
                    end_ms: None,
                },
            }));

            // Re-assert name on TurnBuffer (final, authoritative). The
            // accumulated `arguments_json` was already streamed in chunks via
            // `append_tool_arguments`; we don't repeat it here.
            if let Some(history_call_id) = pending.history_call_id.as_ref() {
                turn_buffer
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
/// `TurnBuffer` API. The processor reserves message ids via the global session
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
            .expect("FixedAssistantId only yields one id per turn")
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

fn map_finish_reason(reason: &CompletionFinishReason) -> FinishReason {
    match reason {
        CompletionFinishReason::Stop => FinishReason::Stop,
        CompletionFinishReason::ToolCalls => FinishReason::ToolCalls,
        CompletionFinishReason::Length => FinishReason::MaxTokens,
        CompletionFinishReason::ContentFilter => FinishReason::ContentFilter,
        CompletionFinishReason::Other(_) => FinishReason::Other,
    }
}

#[derive(Debug, Default, Clone)]
struct PendingToolCall {
    part_id: Option<i64>,
    call_id: Option<i64>,
    started_at_ms: Option<i64>,
    id: Option<String>,
    name: Option<String>,
    arguments_json: String,
    /// History-side call identifier propagated to `TurnBuffer`. Set the first
    /// time the part is materialized and reused for every subsequent argument
    /// fragment so chunks land on the right tool entry.
    history_call_id: Option<ToolCallId>,
}

fn tool_execution_title(name: Option<&str>) -> String {
    format!("Tool {}", name.unwrap_or("unknown").trim())
}

fn placeholder_tool_invocation(
    name: Option<&str>,
    available_tools: &[EntryDefinition],
) -> ToolInvocation {
    let requested_name = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let Some(tool) = available_tools
        .iter()
        .find(|tool| tool.name == requested_name)
    else {
        return ToolInvocation {
            name: requested_name.to_string(),
            input: StructuredObject::default(),
        };
    };

    tool_invocation_for_definition(tool, StructuredObject::default())
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[EntryDefinition],
) -> Result<ToolInvocation, AppError> {
    let trimmed_name = name.trim();
    let tool = available_tools
        .iter()
        .find(|tool| tool.name == trimmed_name)
        .ok_or_else(|| {
            AppError::Provider(format!("unsupported tool call from model: {trimmed_name}"))
        })?;

    let parsed = parse_custom_input(arguments_json)?;
    Ok(tool_invocation_for_definition(tool, parsed))
}

fn tool_invocation_for_definition(
    tool: &EntryDefinition,
    input: StructuredObject,
) -> ToolInvocation {
    ToolInvocation {
        name: tool.name.clone(),
        input,
    }
}

fn parse_custom_input(arguments_json: &str) -> Result<StructuredObject, AppError> {
    let value = parse_json_body::<serde_json::Value>(arguments_json)?;
    StructuredObject::try_from(value)
        .map_err(|err| AppError::Internal(format!("invalid custom tool input: {err}")))
}

#[cfg(test)]
pub(crate) fn parse_input<T>(arguments_json: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    parse_json_body(arguments_json)
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

#[cfg(test)]
mod tests {
    use std::{
        pin::Pin,
        sync::{Arc, Mutex},
    };

    use async_trait::async_trait;
    use futures_core::Stream;
    use futures_util::stream;
    use serde_json::json;

    use super::*;
    use crate::event::DomainEvent;
    use crate::message::{BuiltinToolInput, GlobToolInput, GrepToolInput, ReadToolInput};
    use crate::model::{ModelId, ModelRef, ProviderId};
    use crate::provider::{
        CompletionFinishReason, CompletionResponse, ModelProvider, ProviderModel,
    };
    use crate::tool::{EntryBehavior, EntryDefinition};

    /// Construct an in-memory `EventPublisher` whose `MemEventStore` collects
    /// every event the publisher routes to it — including non-persistent
    /// (UI) ones in tests, since the test bus mirrors them back into the
    /// store. Tests inspect the store side as a single ordered transcript.
    fn test_publisher() -> (Arc<EventPublisher>, Arc<MemEventStore>) {
        use crate::event::{EventStore, SequenceAllocator};
        let store: Arc<MemEventStore> = Arc::new(MemEventStore::default());
        let store_dyn: Arc<dyn EventStore<EventKind>> = Arc::clone(&store) as _;
        // The test bus forwards every published event into the same
        // collector store, so non-persistent UI events show up in the
        // assertion transcript even though the production publisher would
        // skip them on the store side.
        let bus: Arc<dyn crate::event::EventBus<EventKind>> =
            Arc::new(MirrorBus::new(Arc::clone(&store)));
        let seq = Arc::new(SequenceAllocator::new());
        (Arc::new(EventPublisher::new(seq, store_dyn, bus)), store)
    }

    #[derive(Default)]
    struct MemEventStore {
        events: Mutex<Vec<DomainEvent>>,
    }

    /// Test-only bus that mirrors every published event into a `MemEventStore`,
    /// so assertions can read the full event transcript (history + UI) from a
    /// single Vec. It does not deliver to subscribers — tests don't need
    /// subscription semantics.
    struct MirrorBus {
        store: Arc<MemEventStore>,
    }

    impl MirrorBus {
        fn new(store: Arc<MemEventStore>) -> Self {
            Self { store }
        }
    }

    #[async_trait]
    impl crate::event::EventBus<EventKind> for MirrorBus {
        async fn publish(&self, event: DomainEvent) -> Result<(), crate::event::BusError> {
            self.store
                .events
                .lock()
                .expect("test bus lock")
                .push(event);
            Ok(())
        }

        fn subscribe(
            &self,
            filter: crate::event::EventFilter,
        ) -> crate::event::Subscription<EventKind> {
            // Tests don't subscribe; route through a one-shot in-process bus
            // so the trait method has a working implementation if anyone ever
            // calls it.
            let bridge = crate::event::InProcessEventBus::<EventKind>::new(1);
            bridge.subscribe(filter)
        }

        fn capacity(&self) -> usize {
            usize::MAX
        }
    }

    #[async_trait]
    impl crate::event::EventStore<EventKind> for MemEventStore {
        async fn append_batch(
            &self,
            _events: &[DomainEvent],
        ) -> Result<(), crate::event::EventStoreError> {
            // No-op: the test transcript is the bus side. The bus mirrors
            // every published event into `events`, so a no-op store keeps
            // each event recorded exactly once.
            Ok(())
        }

        async fn range(
            &self,
            filter: &crate::event::EventFilter,
            range: crate::event::StoreRange,
        ) -> Result<Vec<DomainEvent>, crate::event::EventStoreError> {
            let mut out: Vec<_> = self
                .events
                .lock()
                .expect("test store lock")
                .iter()
                .filter(|e| e.meta.seq_global > range.after_seq_global)
                .filter(|e| filter.scope.matches(&e.meta))
                .cloned()
                .collect();
            out.truncate(range.limit);
            Ok(out)
        }

        async fn high_watermark(&self) -> Result<Option<i64>, crate::event::EventStoreError> {
            Ok(self
                .events
                .lock()
                .expect("test store lock")
                .iter()
                .map(|e| e.meta.seq_global)
                .max())
        }

        async fn session_high_watermark(
            &self,
            session_id: i64,
        ) -> Result<Option<i64>, crate::event::EventStoreError> {
            Ok(self
                .events
                .lock()
                .expect("test store lock")
                .iter()
                .filter(|e| e.meta.session_id == Some(session_id))
                .filter_map(|e| e.meta.seq_session)
                .max())
        }
    }

    #[test]
    fn parse_tool_invocation_recognizes_plugin_tools() {
        let tools = vec![EntryDefinition::plugin(
            "plugin_echo",
            "Echo a message from a plugin.",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            EntryBehavior::ReadOnly,
            "fixture",
        )];

        let invocation =
            parse_tool_invocation("plugin_echo", "{\"message\":\"hello\"}", tools.as_slice())
                .expect("custom tool call should parse");

        let ToolInvocation { name, input } = invocation;
        assert_eq!(name, "plugin_echo");
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["message"], "hello");
    }

    #[test]
    fn parse_tool_invocation_rejects_unloaded_builtin_tools() {
        let tools = vec![EntryDefinition::builtin::<ReadToolInput>(
            "read",
            "Read a file.",
            crate::tool::EntryBehavior::ReadOnly,
        )];

        let err = parse_tool_invocation("bash", "{\"command\":\"pwd\"}", tools.as_slice())
            .expect_err("unexpected builtin should be rejected");

        assert!(err.to_string().contains("unsupported tool call from model"));
    }

    #[test]
    fn parse_input_accepts_trailing_text_after_valid_json_prefix() {
        let parsed =
            parse_input::<GlobToolInput>("{\"pattern\":\"**/*.md\"}\nPlease use glob first.")
                .expect("valid JSON prefix should parse");

        assert_eq!(parsed.pattern, "**/*.md");
        assert_eq!(parsed.path, None);
    }

    #[test]
    fn parse_tool_invocation_accepts_builtin_arguments_with_trailing_text() {
        let tools = vec![EntryDefinition::builtin::<GrepToolInput>(
            "grep",
            "Search files for a pattern.",
            EntryBehavior::ReadOnly,
        )];

        let invocation = parse_tool_invocation(
            "grep",
            "{\"pattern\":\"cache marker\"}\nThen report the result.",
            tools.as_slice(),
        )
        .expect("valid JSON prefix should parse for builtin tools");

        match invocation.as_builtin() {
            Some(BuiltinToolInput::Grep(payload)) => {
                assert_eq!(payload.pattern, "cache marker");
                assert_eq!(payload.path, None);
                assert_eq!(payload.include, None);
            }
            other => panic!("expected grep builtin invocation, got {other:?}"),
        }
    }

    #[derive(Clone)]
    struct OrderedStreamProvider {
        events: Vec<CompletionStreamEvent>,
    }

    #[async_trait]
    impl ModelProvider for OrderedStreamProvider {
        fn id(&self) -> &str {
            "ordered-stream"
        }

        fn default_model(&self) -> &ModelId {
            static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("ordered-model"));
            &DEFAULT_MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new("ordered-stream", "ordered-model")])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: ProviderId::new("ordered-stream"),
                model: ModelId::new("ordered-model"),
                text: String::new(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            Ok(Box::pin(stream::iter(
                self.events.clone().into_iter().map(Ok::<_, AppError>),
            )))
        }
    }

    #[tokio::test]
    async fn run_turn_preserves_interleaved_text_and_tool_call_part_order() {
        let mut registry = ProviderRegistry::new();
        registry.register(OrderedStreamProvider {
            events: vec![
                CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "Before ".to_owned(),
                },
                CompletionStreamEvent::ToolCallDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    stream_key: "call_1".to_owned(),
                    id: Some("call_1".to_owned()),
                    name: Some("search".to_owned()),
                    arguments_delta: "{\"q\":".to_owned(),
                },
                CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "After".to_owned(),
                },
                CompletionStreamEvent::ToolCallDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    stream_key: "call_1".to_owned(),
                    id: None,
                    name: None,
                    arguments_delta: "\"rust\"}".to_owned(),
                },
                CompletionStreamEvent::Completed {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                },
            ],
        });

        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(crate::session::ContextPolicy::default()),
        );

        let result = processor
            .run_turn(SessionRunRequest {
                session_id: 1,
                model: ModelRef::new("ordered-stream", "ordered-model"),
                completion: CompletionRequest {
                    model: ModelId::new("ordered-model"),
                    system: None,
                    messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                    tools: vec![EntryDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        EntryBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    response_format: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_publisher: None,
                cancel: None,
            })
            .await
            .expect("processor run should succeed");

        let assistant = result
            .state
            .into_iter()
            .find(|message| message.id == 100)
            .expect("assistant message should be present");

        assert_eq!(assistant.parts.len(), 3);
        assert_eq!(assistant.parts[0].text(), Some("Before "));
        assert_eq!(assistant.parts[2].text(), Some("After"));

        let tool_part = assistant.parts[1]
            .content
            .as_ref()
            .expect("tool part should have content");
        match tool_part {
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id,
                invocation,
                ..
            }) => {
                assert_eq!(*call_id, 300);
                let ToolInvocation { name, input } = invocation;
                assert_eq!(name, "search");
                let payload = serde_json::Value::from(input.clone());
                assert_eq!(payload["q"], "rust");
            }
            other => panic!("expected tool execution part, got {other:?}"),
        }
        assert_eq!(assistant.parts[1].operation_id.as_deref(), Some("call_1"));
    }

    #[tokio::test]
    async fn run_turn_persists_reasoning_and_provider_metadata() {
        let mut registry = ProviderRegistry::new();
        registry.register(OrderedStreamProvider {
            events: vec![
                CompletionStreamEvent::ThinkingDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "think ".to_owned(),
                },
                CompletionStreamEvent::ThinkingDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "again".to_owned(),
                },
                CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "done".to_owned(),
                },
                CompletionStreamEvent::Completed {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: Some(json!({"response_id": "resp_1"})),
                },
            ],
        });

        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(crate::session::ContextPolicy::default()),
        );
        let (event_publisher, store) = test_publisher();

        let result = processor
            .run_turn(SessionRunRequest {
                session_id: 1,
                model: ModelRef::new("ordered-stream", "ordered-model"),
                completion: CompletionRequest {
                    model: ModelId::new("ordered-model"),
                    system: None,
                    messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                    tools: Vec::new(),
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    response_format: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_publisher: Some(event_publisher),
                cancel: None,
            })
            .await
            .expect("processor run should succeed");

        let assistant = result
            .state
            .into_iter()
            .find(|message| message.id == 100)
            .expect("assistant message should be present");

        assert_eq!(assistant.parts.len(), 2);
        assert_eq!(
            assistant.parts[0].reasoning_summary(),
            Some(&["think ".to_string(), "again".to_string()][..])
        );
        assert_eq!(assistant.parts[1].text(), Some("done"));
        assert_eq!(
            assistant.metadata.provider_metadata,
            Some(json!({"response_id": "resp_1"}))
        );

        let events: Vec<EventKind> = store
            .events
            .lock()
            .expect("test store lock")
            .iter()
            .map(|e| e.kind.clone())
            .collect();
        assert!(events.iter().any(|event| {
            matches!(
                event,
                EventKind::MessagePartDelta(MessagePartDeltaEvent {
                    message_id,
                    part_id,
                    field: PartDeltaField::ReasoningSummary,
                    delta,
                    seq,
                    ..
                }) if *message_id == 100 && *part_id == 200 && delta == "think " && *seq == 1
            )
        }));
    }

    #[tokio::test]
    async fn run_turn_allocates_unique_parts_beyond_previous_fixed_block() {
        let tool_count = 1_100usize;
        let mut events = Vec::with_capacity(tool_count + 1);
        for index in 0..tool_count {
            events.push(CompletionStreamEvent::ToolCallDelta {
                provider_id: ProviderId::new("ordered-stream"),
                model: ModelId::new("ordered-model"),
                stream_key: format!("call_{index}"),
                id: Some(format!("call_{index}")),
                name: Some("search".to_owned()),
                arguments_delta: format!("{{\"q\":\"item-{index}\"}}"),
            });
        }
        events.push(CompletionStreamEvent::Completed {
            provider_id: ProviderId::new("ordered-stream"),
            model: ModelId::new("ordered-model"),
            finish_reason: Some(CompletionFinishReason::ToolCalls),
            usage: None,
            provider_metadata: None,
        });

        let mut registry = ProviderRegistry::new();
        registry.register(OrderedStreamProvider { events });

        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(crate::session::ContextPolicy::default()),
        );

        let result = processor
            .run_turn(SessionRunRequest {
                session_id: 1,
                model: ModelRef::new("ordered-stream", "ordered-model"),
                completion: CompletionRequest {
                    model: ModelId::new("ordered-model"),
                    system: None,
                    messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                    tools: vec![EntryDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        EntryBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    response_format: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_publisher: None,
                cancel: None,
            })
            .await
            .expect("processor run should succeed");

        let assistant = result
            .state
            .into_iter()
            .find(|message| message.id == 100)
            .expect("assistant message should be present");
        let allocated_ids = assistant
            .parts
            .iter()
            .map(|part| part.id)
            .collect::<Vec<_>>();

        assert_eq!(assistant.parts.len(), tool_count);
        assert_eq!(allocated_ids.first().copied(), Some(200));
        assert_eq!(
            allocated_ids.last().copied(),
            Some(200 + tool_count as i64 - 1)
        );
        assert_eq!(
            allocated_ids,
            (200..200 + tool_count as i64).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn run_turn_emits_incremental_stream_events() {
        let mut registry = ProviderRegistry::new();
        registry.register(OrderedStreamProvider {
            events: vec![
                CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "Hel".to_owned(),
                },
                CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    delta: "lo".to_owned(),
                },
                CompletionStreamEvent::ToolCallDelta {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    stream_key: "call_1".to_owned(),
                    id: Some("call_1".to_owned()),
                    name: Some("search".to_owned()),
                    arguments_delta: "{\"q\":\"rust\"}".to_owned(),
                },
                CompletionStreamEvent::Completed {
                    provider_id: ProviderId::new("ordered-stream"),
                    model: ModelId::new("ordered-model"),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                },
            ],
        });

        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(crate::session::ContextPolicy::default()),
        );
        let (event_publisher, store) = test_publisher();

        let result = processor
            .run_turn(SessionRunRequest {
                session_id: 7,
                model: ModelRef::new("ordered-stream", "ordered-model"),
                completion: CompletionRequest {
                    model: ModelId::new("ordered-model"),
                    system: None,
                    messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                    tools: vec![EntryDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        EntryBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking: None,
                    response_format: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_publisher: Some(event_publisher.clone()),
                cancel: None,
            })
            .await
            .expect("processor run should succeed");

        let events: Vec<EventKind> = store
            .events
            .lock()
            .expect("test store lock")
            .iter()
            .map(|e| e.kind.clone())
            .collect();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                EventKind::MessagePartUpdated(MessagePartUpdatedEvent {
                    message_id,
                    part,
                    ..
                }) if *message_id == 100 && part.id == 200
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                EventKind::MessagePartDelta(MessagePartDeltaEvent {
                    message_id,
                    part_id,
                    delta,
                    seq,
                    ..
                }) if *message_id == 100 && *part_id == 200 && delta == "Hel" && *seq == 1
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                EventKind::MessagePartDelta(MessagePartDeltaEvent {
                    message_id,
                    part_id,
                    delta,
                    seq,
                    ..
                }) if *message_id == 100 && *part_id == 200 && delta == "lo" && *seq == 2
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                EventKind::MessagePartUpdated(MessagePartUpdatedEvent {
                    message_id,
                    part,
                    ..
                }) if *message_id == 100 && part.id == 201 && part.operation_id.as_deref() == Some("call_1")
            )
        }));

        // The committed history must contain at least the assistant
        // completion and one tool-call-issued event for the streamed call.
        let kinds: Vec<&'static str> = result
            .history_items
            .iter()
            .map(EventKind::tag_str)
            .collect();
        assert!(kinds.contains(&"assistant_message_completed"));
        assert!(kinds.contains(&"tool_call_issued"));
    }
}
