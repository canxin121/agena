use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Utc;
use futures_util::StreamExt;

use crate::error::AppError;
use crate::event::{
    ErrorInfo, MessagePartDeltaEvent, MessagePartUpdatedEvent, MessageProjectionEvent,
    MessageProjector, PartDeltaField, SessionEvent, StreamErrorEvent,
};
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolInput, GlobToolInput,
    GrepToolInput, Message, MessageMetadata, MessageSource, MessageStateStore, MessageUpdate,
    PartContent, ReadToolInput, StructuredObject, TaskToolInput, TimeRange, TodoWriteToolInput,
    ToolExecutionPart, ToolInvocation, ToolSearchToolInput, ViewFileToolInput,
};
use crate::model::ModelRef;
use crate::provider::{CompletionRequest, CompletionStreamEvent, ProviderRegistry};
use crate::role::Role;
use crate::tool::{ToolDefinition, ToolSource};

use super::{context_governor::ContextGovernor, store::ProcessorPartIdAllocator};

#[async_trait]
pub(crate) trait SessionEventSink: Send + Sync {
    async fn emit(
        &self,
        session_id: i64,
        message_snapshot: Option<Message>,
        events: Vec<SessionEvent>,
    ) -> Result<(), AppError>;
}

#[derive(Clone)]
pub(crate) struct SessionRunRequest {
    pub session_id: i64,
    pub model: ModelRef,
    pub completion: CompletionRequest,
    pub next_message_id: i64,
    pub part_ids: ProcessorPartIdAllocator,
    pub next_call_id: i64,
    pub event_sink: Option<Arc<dyn SessionEventSink>>,
}

#[derive(Debug)]
pub(crate) struct SessionRunResult {
    pub assistant_message_id: i64,
    pub state: Vec<Message>,
    pub client_events: Vec<SessionEvent>,
    pub provider_metadata: Option<serde_json::Value>,
    pub terminal_error: Option<AppError>,
}

#[derive(Clone)]
pub struct SessionProcessor {
    provider_registry: Arc<ProviderRegistry>,
    context_governor: ContextGovernor,
    projector: MessageProjector,
}

impl SessionProcessor {
    pub fn new(
        provider_registry: Arc<ProviderRegistry>,
        context_governor: ContextGovernor,
    ) -> Self {
        Self {
            provider_registry,
            context_governor,
            projector: MessageProjector,
        }
    }

    pub(crate) async fn run_turn(
        &self,
        mut run: SessionRunRequest,
    ) -> Result<SessionRunResult, AppError> {
        let mut store = MessageStateStore::default();
        let mut client_events = Vec::new();
        let mut stream = self
            .provider_registry
            .complete_stream(&run.model, run.completion.clone())
            .await?;

        let assistant_message_id = run.next_message_id;
        run.next_message_id += 1;

        self.projector
            .apply_to_store(
                &mut store,
                MessageProjectionEvent::MessageStarted {
                    message_id: assistant_message_id,
                    role: Role::Assistant,
                    created_at: Utc::now(),
                    metadata: Some(MessageMetadata {
                        source: MessageSource::Assistant,
                        parent_message_id: run.completion.messages.last().map(|message| message.id),
                        generated_by_call_id: None,
                        model_provider_id: run.model.provider_id.to_string(),
                        model_id: run.completion.model.to_string(),
                        tags: Vec::new(),
                    }),
                },
            )
            .map_err(|err| AppError::Internal(err.to_string()))?;

        let mut active_text_part: Option<i64> = None;
        let mut pending_calls: BTreeMap<String, PendingToolCall> = BTreeMap::new();
        let mut part_delta_sequences = BTreeMap::<i64, u64>::new();
        let mut provider_err: Option<AppError> = None;
        let mut usage = None;
        let mut finish = None;
        let mut provider_metadata = None;

        while let Some(item) = stream.next().await {
            match item {
                Ok(CompletionStreamEvent::TextDelta { delta, .. }) => {
                    let part_id = if let Some(part_id) = active_text_part {
                        part_id
                    } else {
                        let part_id = run.part_ids.reserve().await?;
                        self.projector
                            .apply_to_store(
                                &mut store,
                                MessageProjectionEvent::TextPartStarted {
                                    message_id: assistant_message_id,
                                    part_id,
                                    created_at: Utc::now(),
                                    synthetic: false,
                                    ignored: false,
                                },
                            )
                            .map_err(|err| AppError::Internal(err.to_string()))?;
                        self.emit_part_updated(&run, &store, assistant_message_id, part_id)
                            .await?;
                        active_text_part = Some(part_id);
                        part_id
                    };
                    self.projector
                        .apply_to_store(
                            &mut store,
                            MessageProjectionEvent::TextDelta {
                                part_id,
                                delta: delta.clone(),
                            },
                        )
                        .map_err(|err| AppError::Internal(err.to_string()))?;
                    let seq = part_delta_sequences.entry(part_id).or_default();
                    *seq += 1;
                    self.emit_part_delta(
                        &run,
                        &store,
                        assistant_message_id,
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
                        self.projector
                            .apply_to_store(
                                &mut store,
                                MessageProjectionEvent::TextCompleted { part_id },
                            )
                            .map_err(|err| AppError::Internal(err.to_string()))?;
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
                        &mut store,
                        assistant_message_id,
                        pending,
                    )
                    .await?;
                }
                Ok(CompletionStreamEvent::Completed {
                    finish_reason,
                    usage: usage_value,
                    provider_metadata: completed_provider_metadata,
                    ..
                }) => {
                    usage = usage_value.map(Into::into);
                    finish = finish_reason.map(|item| format!("{item:?}"));
                    provider_metadata = completed_provider_metadata;
                }
                Err(err) => {
                    provider_err = Some(err);
                    break;
                }
            }
        }

        if let Some(part_id) = active_text_part {
            self.projector
                .apply_to_store(
                    &mut store,
                    MessageProjectionEvent::TextCompleted { part_id },
                )
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }

        self.finalize_pending_tool_calls(&mut run, &mut store, assistant_message_id, pending_calls)
            .await?;

        if let Some(err) = provider_err {
            self.projector
                .apply_to_store(
                    &mut store,
                    MessageProjectionEvent::MessageFailed {
                        message_id: assistant_message_id,
                        finish: Some(err.to_string()),
                    },
                )
                .map_err(|inner| AppError::Internal(inner.to_string()))?;

            client_events.push(SessionEvent::StreamError(StreamErrorEvent {
                session_id: run.session_id,
                error: ErrorInfo {
                    code: "provider_stream_error".to_string(),
                    message: err.to_string(),
                },
                ts_ms: Utc::now().timestamp_millis(),
            }));
            return Ok(SessionRunResult {
                assistant_message_id,
                state: store.list_message_snapshots(),
                client_events,
                provider_metadata,
                terminal_error: Some(err),
            });
        }

        self.projector
            .apply_to_store(
                &mut store,
                MessageProjectionEvent::MessageCompleted {
                    message_id: assistant_message_id,
                    finish,
                    usage,
                },
            )
            .map_err(|err| AppError::Internal(err.to_string()))?;

        Ok(SessionRunResult {
            assistant_message_id,
            state: store.list_message_snapshots(),
            client_events,
            provider_metadata,
            terminal_error: None,
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

    async fn ensure_pending_tool_call_part(
        &self,
        run: &mut SessionRunRequest,
        store: &mut MessageStateStore,
        assistant_message_id: i64,
        pending: &mut PendingToolCall,
    ) -> Result<(), AppError> {
        let mut should_emit = false;
        if pending.part_id.is_none() {
            let part_id = run.part_ids.reserve().await?;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = Utc::now();

            self.projector
                .apply_to_store(
                    store,
                    MessageProjectionEvent::ToolExecutionStarted {
                        message_id: assistant_message_id,
                        part_id,
                        created_at: start,
                        call_id,
                        invocation: placeholder_tool_invocation(
                            pending.name.as_deref(),
                            run.completion.tools.as_slice(),
                        ),
                        title: tool_execution_title(pending.name.as_deref()),
                    },
                )
                .map_err(|err| AppError::Internal(err.to_string()))?;

            pending.part_id = Some(part_id);
            pending.call_id = Some(call_id);
            pending.started_at_ms = Some(start.timestamp_millis());
            should_emit = true;
        }

        if let (Some(part_id), Some(operation_id)) = (
            pending.part_id,
            pending
                .id
                .as_ref()
                .filter(|id| !id.trim().is_empty())
                .cloned(),
        ) {
            store
                .apply(MessageUpdate::SetPartOperationId {
                    part_id,
                    operation_id,
                })
                .map_err(|err| AppError::Internal(err.to_string()))?;
            should_emit = true;
        }

        if should_emit && let Some(part_id) = pending.part_id {
            self.emit_part_updated(run, store, assistant_message_id, part_id)
                .await?;
        }

        Ok(())
    }

    async fn finalize_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        store: &mut MessageStateStore,
        assistant_message_id: i64,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for mut pending in pending_calls.into_values() {
            self.ensure_pending_tool_call_part(run, store, assistant_message_id, &mut pending)
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

            store
                .apply(MessageUpdate::ReplacePartContent {
                    part_id,
                    content: PartContent::ToolExecution(ToolExecutionPart::Pending {
                        call_id,
                        invocation,
                        title: tool_execution_title(Some(tool_name.as_str())),
                        lifecycle: TimeRange {
                            start_ms: pending.started_at_ms.unwrap_or_default(),
                            end_ms: None,
                        },
                    }),
                })
                .map_err(|err| AppError::Internal(err.to_string()))?;
            self.emit_part_updated(run, store, assistant_message_id, part_id)
                .await?;
        }

        Ok(())
    }

    async fn emit_part_updated(
        &self,
        run: &SessionRunRequest,
        store: &MessageStateStore,
        message_id: i64,
        part_id: i64,
    ) -> Result<(), AppError> {
        let Some(sink) = run.event_sink.as_ref() else {
            return Ok(());
        };

        let message = store.get_message_snapshot(message_id).ok_or_else(|| {
            AppError::Internal(format!(
                "message snapshot not found for stream event: {message_id}"
            ))
        })?;
        let part = message
            .parts
            .iter()
            .find(|part| part.id == part_id)
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "part snapshot not found for stream event: {part_id}"
                ))
            })?;
        sink.emit(
            run.session_id,
            Some(message.clone()),
            vec![SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                session_id: run.session_id,
                message_id,
                message_role: message.role,
                message_state: message.state,
                message_created_at: message.created_at,
                part,
                ts_ms: Utc::now().timestamp_millis(),
            })],
        )
        .await
    }

    async fn emit_part_delta(
        &self,
        run: &SessionRunRequest,
        store: &MessageStateStore,
        message_id: i64,
        part_id: i64,
        call_id: Option<i64>,
        field: PartDeltaField,
        delta: String,
        seq: u64,
    ) -> Result<(), AppError> {
        let Some(sink) = run.event_sink.as_ref() else {
            return Ok(());
        };

        let message = store.get_message_snapshot(message_id).ok_or_else(|| {
            AppError::Internal(format!(
                "message snapshot not found for stream delta event: {message_id}"
            ))
        })?;
        sink.emit(
            run.session_id,
            Some(message),
            vec![SessionEvent::MessagePartDelta(MessagePartDeltaEvent {
                session_id: run.session_id,
                message_id,
                part_id,
                call_id,
                field,
                delta,
                seq,
                ts_ms: Utc::now().timestamp_millis(),
            })],
        )
        .await
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
}

fn tool_execution_title(name: Option<&str>) -> String {
    format!("Tool {}", name.unwrap_or("unknown").trim())
}

fn placeholder_tool_invocation(
    name: Option<&str>,
    available_tools: &[ToolDefinition],
) -> ToolInvocation {
    let requested_name = name
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("unknown");
    let canonical_name = canonical_builtin_tool_name(requested_name);

    let Some(tool) = available_tools
        .iter()
        .find(|tool| tool.name == requested_name || tool.name == canonical_name)
    else {
        return ToolInvocation::Custom {
            name: requested_name.to_string(),
            input: StructuredObject::default(),
        };
    };

    match &tool.source {
        ToolSource::Builtin => ToolInvocation::Builtin {
            input: placeholder_builtin_tool_input(tool.name.as_str()),
        },
        ToolSource::Plugin { .. } => ToolInvocation::Custom {
            name: tool.name.clone(),
            input: StructuredObject::default(),
        },
    }
}

fn placeholder_builtin_tool_input(name: &str) -> BuiltinToolInput {
    match canonical_builtin_tool_name(name) {
        "bash" => BuiltinToolInput::Bash(BashToolInput {
            command: String::new(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
        }),
        "read" => BuiltinToolInput::Read(ReadToolInput {
            file_path: String::new(),
            offset: None,
            limit: None,
        }),
        "view_file" => BuiltinToolInput::ViewFile(ViewFileToolInput {
            path: String::new(),
        }),
        "apply_patch" => BuiltinToolInput::ApplyPatch(ApplyPatchToolInput {
            patch: String::new(),
        }),
        "glob" => BuiltinToolInput::Glob(GlobToolInput {
            pattern: String::new(),
            path: None,
        }),
        "grep" => BuiltinToolInput::Grep(GrepToolInput {
            pattern: String::new(),
            path: None,
            include: None,
        }),
        "task" => BuiltinToolInput::Task(TaskToolInput {
            description: String::new(),
            prompt: String::new(),
            subagent_type: crate::message::TaskSubagentType::Explore,
            task_id: None,
            command: None,
        }),
        "tool_search" => BuiltinToolInput::ToolSearch(ToolSearchToolInput {
            query: String::new(),
            load: Vec::new(),
            limit: None,
        }),
        "todo_write" => BuiltinToolInput::TodoWrite(TodoWriteToolInput { items: Vec::new() }),
        "ask_user" => BuiltinToolInput::AskUser(AskUserToolInput {
            questions: Vec::new(),
        }),
        other => BuiltinToolInput::Task(TaskToolInput {
            description: String::new(),
            prompt: format!("placeholder for unsupported builtin {other}"),
            subagent_type: crate::message::TaskSubagentType::Explore,
            task_id: None,
            command: None,
        }),
    }
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[ToolDefinition],
) -> Result<ToolInvocation, AppError> {
    let trimmed_name = name.trim();
    let canonical_name = canonical_builtin_tool_name(trimmed_name);
    let tool = available_tools
        .iter()
        .find(|tool| tool.name == trimmed_name || tool.name == canonical_name)
        .ok_or_else(|| {
            AppError::Provider(format!("unsupported tool call from model: {trimmed_name}"))
        })?;

    if !matches!(tool.source, ToolSource::Builtin) {
        let parsed = parse_custom_input(arguments_json)?;
        return Ok(ToolInvocation::Custom {
            name: tool.name.clone(),
            input: parsed,
        });
    }

    let input = match canonical_name {
        "bash" => BuiltinToolInput::Bash(parse_input::<BashToolInput>(arguments_json)?),
        "read" => BuiltinToolInput::Read(parse_input::<ReadToolInput>(arguments_json)?),
        "view_file" => {
            BuiltinToolInput::ViewFile(parse_input::<ViewFileToolInput>(arguments_json)?)
        }
        "apply_patch" => {
            BuiltinToolInput::ApplyPatch(parse_input::<ApplyPatchToolInput>(arguments_json)?)
        }
        "glob" => BuiltinToolInput::Glob(parse_input::<GlobToolInput>(arguments_json)?),
        "grep" => BuiltinToolInput::Grep(parse_input::<GrepToolInput>(arguments_json)?),
        "task" => BuiltinToolInput::Task(parse_input::<TaskToolInput>(arguments_json)?),
        "tool_search" => {
            BuiltinToolInput::ToolSearch(parse_input::<ToolSearchToolInput>(arguments_json)?)
        }
        "todo_write" => {
            BuiltinToolInput::TodoWrite(parse_input::<TodoWriteToolInput>(arguments_json)?)
        }
        "ask_user" => BuiltinToolInput::AskUser(parse_input::<AskUserToolInput>(arguments_json)?),
        other => {
            return Err(AppError::Provider(format!(
                "unsupported builtin tool call from model: {other}"
            )));
        }
    };

    Ok(ToolInvocation::Builtin { input })
}

fn canonical_builtin_tool_name(name: &str) -> &str {
    match name {
        "request_user_input" => "ask_user",
        other => other,
    }
}

fn parse_custom_input(arguments_json: &str) -> Result<StructuredObject, AppError> {
    let value = if arguments_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(arguments_json)?
    };
    StructuredObject::try_from(value)
        .map_err(|err| AppError::Internal(format!("invalid custom tool input: {err}")))
}

pub(crate) fn parse_input<T>(arguments_json: &str) -> Result<T, AppError>
where
    T: serde::de::DeserializeOwned,
{
    let body = if arguments_json.trim().is_empty() {
        "{}"
    } else {
        arguments_json
    };
    serde_json::from_str::<T>(body).map_err(AppError::from)
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
    use crate::model::{ModelId, ModelRef, ProviderId};
    use crate::provider::{
        CompletionFinishReason, CompletionResponse, ModelProvider, ProviderModel,
    };
    use crate::tool::{ToolBehavior, ToolDefinition};

    #[derive(Default)]
    struct RecordingEventSink {
        events: Mutex<Vec<SessionEvent>>,
    }

    #[async_trait]
    impl SessionEventSink for RecordingEventSink {
        async fn emit(
            &self,
            _session_id: i64,
            _message_snapshot: Option<Message>,
            events: Vec<SessionEvent>,
        ) -> Result<(), AppError> {
            self.events
                .lock()
                .expect("recording sink lock should not be poisoned")
                .extend(events);
            Ok(())
        }
    }

    #[test]
    fn parse_tool_invocation_recognizes_plugin_tools() {
        let tools = vec![ToolDefinition::plugin(
            "plugin_echo",
            "Echo a message from a plugin.",
            json!({
                "type": "object",
                "properties": {
                    "message": { "type": "string" }
                },
                "required": ["message"]
            }),
            ToolBehavior::ReadOnly,
            "fixture",
        )];

        let invocation =
            parse_tool_invocation("plugin_echo", "{\"message\":\"hello\"}", tools.as_slice())
                .expect("custom tool call should parse");

        match invocation {
            ToolInvocation::Custom { name, input } => {
                assert_eq!(name, "plugin_echo");
                let payload = serde_json::Value::from(input);
                assert_eq!(payload["message"], "hello");
            }
            other => panic!("expected custom tool invocation, got {other:?}"),
        }
    }

    #[test]
    fn parse_tool_invocation_rejects_unloaded_builtin_tools() {
        let tools = vec![ToolDefinition::builtin::<ReadToolInput>(
            "read",
            "Read a file.",
            ToolBehavior::ReadOnly,
        )];

        let err = parse_tool_invocation("bash", "{\"command\":\"pwd\"}", tools.as_slice())
            .expect_err("unexpected builtin should be rejected");

        assert!(err.to_string().contains("unsupported tool call from model"));
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
                    tools: vec![ToolDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        ToolBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_sink: None,
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
                match invocation {
                    ToolInvocation::Custom { name, input } => {
                        assert_eq!(name, "search");
                        let payload = serde_json::Value::from(input.clone());
                        assert_eq!(payload["q"], "rust");
                    }
                    other => panic!("expected custom tool invocation, got {other:?}"),
                }
            }
            other => panic!("expected tool execution part, got {other:?}"),
        }
        assert_eq!(assistant.parts[1].operation_id.as_deref(), Some("call_1"));
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
                    tools: vec![ToolDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        ToolBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_sink: None,
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
        let event_sink = Arc::new(RecordingEventSink::default());

        let _ = processor
            .run_turn(SessionRunRequest {
                session_id: 7,
                model: ModelRef::new("ordered-stream", "ordered-model"),
                completion: CompletionRequest {
                    model: ModelId::new("ordered-model"),
                    system: None,
                    messages: vec![Message::prompt_text(crate::role::Role::User, "hello")],
                    tools: vec![ToolDefinition::plugin(
                        "search",
                        "Search the workspace.",
                        json!({
                            "type": "object",
                            "properties": {
                                "q": { "type": "string" }
                            },
                            "required": ["q"]
                        }),
                        ToolBehavior::ReadOnly,
                        "fixture",
                    )],
                    temperature: None,
                    max_output_tokens: Some(64),
                    prompt_cache_key: None,
                    previous_response_id: None,
                    prompt_window_generation: None,
                },
                next_message_id: 100,
                part_ids: ProcessorPartIdAllocator::for_test(200),
                next_call_id: 300,
                event_sink: Some(event_sink.clone()),
            })
            .await
            .expect("processor run should succeed");

        let events = event_sink
            .events
            .lock()
            .expect("recording sink lock should not be poisoned")
            .clone();

        assert!(events.iter().any(|event| {
            matches!(
                event,
                SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    message_id,
                    part,
                    ..
                }) if *message_id == 100 && part.id == 200
            )
        }));
        assert!(events.iter().any(|event| {
            matches!(
                event,
                SessionEvent::MessagePartDelta(MessagePartDeltaEvent {
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
                SessionEvent::MessagePartDelta(MessagePartDeltaEvent {
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
                SessionEvent::MessagePartUpdated(MessagePartUpdatedEvent {
                    message_id,
                    part,
                    ..
                }) if *message_id == 100 && part.id == 201 && part.operation_id.as_deref() == Some("call_1")
            )
        }));
    }
}
