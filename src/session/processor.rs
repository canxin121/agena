use std::collections::BTreeMap;
use std::sync::Arc;

use chrono::Utc;
use futures_util::StreamExt;

use crate::error::AppError;
use crate::event::{
    ErrorInfo, MessageProjectionEvent, MessageProjector, SessionEvent, StreamErrorEvent,
};
use crate::message::{
    ApplyPatchToolInput, BashToolInput, BuiltinToolInput, GlobToolInput, GrepToolInput, Message,
    MessageMetadata, MessagePart, MessageSource, MessageStateStore, MessageUpdate, PartContent,
    ReadToolInput, StructuredObject, TaskToolInput, TimeRange, TodoWriteToolInput,
    ToolExecutionPart, ToolInvocation, ToolSearchToolInput,
};
use crate::provider::{CompletionRequest, CompletionStreamEvent, ProviderRegistry};
use crate::role::Role;
use crate::tool::{ToolDefinition, ToolSource};

use super::context_governor::ContextGovernor;

#[derive(Debug, Clone)]
pub struct SessionRunRequest {
    pub session_id: i64,
    pub provider_id: String,
    pub completion: CompletionRequest,
    pub next_message_id: i64,
    pub next_part_id: i64,
    pub next_call_id: i64,
}

#[derive(Debug, Clone)]
pub struct SessionRunResult {
    pub assistant_message_id: i64,
    pub state: Vec<Message>,
    pub client_events: Vec<SessionEvent>,
}

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

    pub async fn run_turn(&self, mut run: SessionRunRequest) -> Result<SessionRunResult, AppError> {
        let mut store = MessageStateStore::default();
        let mut client_events = Vec::new();
        let mut compacted_rounds = 0_u8;

        loop {
            let prepared = self
                .context_governor
                .prepare_messages(&run.completion.messages);
            let completion_request = CompletionRequest {
                messages: prepared,
                ..run.completion.clone()
            };

            let mut stream = match self
                .provider_registry
                .complete_stream(run.provider_id.as_str(), completion_request)
                .await
            {
                Ok(stream) => stream,
                Err(err)
                    if self
                        .context_governor
                        .should_retry_with_compaction(&err, compacted_rounds) =>
                {
                    compacted_rounds += 1;
                    run.completion.messages = self
                        .context_governor
                        .compact_messages(&run.completion.messages);
                    continue;
                }
                Err(err) => return Err(err),
            };

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
                            parent_message_id: run
                                .completion
                                .messages
                                .last()
                                .map(|message| message.id),
                            generated_by_call_id: None,
                            model_provider_id: run.provider_id.clone(),
                            model_id: run.completion.model.clone(),
                            tags: Vec::new(),
                        }),
                    },
                )
                .map_err(|err| AppError::Internal(err.to_string()))?;

            let mut active_text_part: Option<i64> = None;
            let mut pending_calls: BTreeMap<String, PendingToolCall> = BTreeMap::new();
            let mut provider_err: Option<AppError> = None;
            let mut usage = None;
            let mut finish = None;

            while let Some(item) = stream.next().await {
                match item {
                    Ok(CompletionStreamEvent::TextDelta { delta, .. }) => {
                        let part_id = if let Some(part_id) = active_text_part {
                            part_id
                        } else {
                            let part_id = run.next_part_id;
                            run.next_part_id += 1;
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
                            active_text_part = Some(part_id);
                            part_id
                        };
                        self.projector
                            .apply_to_store(
                                &mut store,
                                MessageProjectionEvent::TextDelta { part_id, delta },
                            )
                            .map_err(|err| AppError::Internal(err.to_string()))?;
                    }
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        stream_key,
                        id,
                        name,
                        arguments_delta,
                        ..
                    }) => {
                        let pending = pending_calls.entry(stream_key).or_default();
                        if let Some(id) = id {
                            pending.id = Some(id);
                        }
                        if let Some(name) = name {
                            pending.name = Some(name);
                        }
                        pending.arguments_json.push_str(arguments_delta.as_str());
                    }
                    Ok(CompletionStreamEvent::Completed {
                        finish_reason,
                        usage: usage_value,
                        ..
                    }) => {
                        usage = usage_value.map(Into::into);
                        finish = finish_reason.map(|item| format!("{item:?}"));
                    }
                    Err(err)
                        if self
                            .context_governor
                            .should_retry_with_compaction(&err, compacted_rounds) =>
                    {
                        compacted_rounds += 1;
                        run.completion.messages = self
                            .context_governor
                            .compact_messages(&run.completion.messages);
                        provider_err = Some(err);
                        break;
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

            if provider_err.as_ref().is_some_and(|err| {
                self.context_governor
                    .should_retry_with_compaction(err, compacted_rounds.saturating_sub(1))
            }) {
                continue;
            }

            self.append_pending_tool_calls(
                &mut run,
                &mut store,
                assistant_message_id,
                pending_calls,
            )?;

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
                return Err(err);
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

            return Ok(SessionRunResult {
                assistant_message_id,
                state: store.list_message_snapshots(),
                client_events,
            });
        }
    }

    fn append_pending_tool_calls(
        &self,
        run: &mut SessionRunRequest,
        store: &mut MessageStateStore,
        assistant_message_id: i64,
        pending_calls: BTreeMap<String, PendingToolCall>,
    ) -> Result<(), AppError> {
        for pending in pending_calls.into_values() {
            let tool_name = pending.name.unwrap_or_else(|| "unknown".to_string());
            let invocation = parse_tool_invocation(
                tool_name.as_str(),
                pending.arguments_json.as_str(),
                run.completion.tools.as_slice(),
            )?;

            let part_id = run.next_part_id;
            run.next_part_id += 1;
            let call_id = run.next_call_id;
            run.next_call_id += 1;
            let start = Utc::now();

            let mut part = MessagePart::with_content(
                part_id,
                assistant_message_id,
                start,
                crate::message::ExecutionStatus::Pending,
                PartContent::ToolExecution(ToolExecutionPart::Pending {
                    call_id,
                    invocation,
                    title: format!("Tool {tool_name}"),
                    lifecycle: TimeRange {
                        start_ms: start.timestamp_millis(),
                        end_ms: None,
                    },
                }),
            );
            if let Some(operation_id) = pending.id.filter(|id| !id.trim().is_empty()) {
                part.operation_id = Some(operation_id);
            }

            store
                .apply(MessageUpdate::InsertPart {
                    message_id: assistant_message_id,
                    part,
                })
                .map_err(|err| AppError::Internal(err.to_string()))?;
        }

        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
struct PendingToolCall {
    id: Option<String>,
    name: Option<String>,
    arguments_json: String,
}

pub(crate) fn parse_tool_invocation(
    name: &str,
    arguments_json: &str,
    available_tools: &[ToolDefinition],
) -> Result<ToolInvocation, AppError> {
    let trimmed_name = name.trim();
    let tool = available_tools
        .iter()
        .find(|tool| tool.name == trimmed_name)
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

    let input = match trimmed_name {
        "bash" => BuiltinToolInput::Bash(parse_input::<BashToolInput>(arguments_json)?),
        "read" => BuiltinToolInput::Read(parse_input::<ReadToolInput>(arguments_json)?),
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
        other => {
            return Err(AppError::Provider(format!(
                "unsupported builtin tool call from model: {other}"
            )));
        }
    };

    Ok(ToolInvocation::Builtin { input })
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
    use serde_json::json;

    use super::*;
    use crate::tool::{ToolBehavior, ToolDefinition};

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
}
