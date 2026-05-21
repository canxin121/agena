use std::collections::BTreeMap;
use std::fs;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures_core::Stream;
use futures_util::stream;
use sea_orm::{
    ColumnTrait, Database, DatabaseConnection, EntityTrait, PaginatorTrait, QueryFilter,
};
use uuid::Uuid;

use crate::agent::Agent;
use crate::db::entities::{activity_message, activity_part};
use crate::db::init_schema;
use crate::entry::{ToolPayloadExecution, ToolPayloadOutput};
use crate::event::EventKind;
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, AttachmentSource, ExecutionStatus, MessageMetadata,
    MessagePart, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput, ToolAttachment,
    ToolOutput, ToolSearchToolInput, UserInputOption, UserInputQuestion, UserInputReply,
    UserInputReplyKind,
};
use crate::model::{ModelId, ModelRef, ProviderId};
use crate::permission::{PermissionPolicy, ToolPermissionPolicy};
use crate::provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionUsage, ModelRuntime, ProviderModel, ProviderRegistry,
};
use crate::role::Role;
use crate::session::history::{
    AssistantMessageCompleted, ToolCallCompleted, ToolCallIssued, TranscriptContent,
};
use crate::session::ids::ToolCallId;
use crate::session::{ContextGovernor, ContextPolicy};

use super::*;
use crate::session::cache::{SessionCache, SessionCachePolicy};

struct TempWorkspace {
    root: std::path::PathBuf,
}

impl TempWorkspace {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("agena-session-tests-{}", Uuid::new_v4()));
        fs::create_dir_all(&root).expect("failed to create temp workspace");
        Self { root }
    }
}

impl Drop for TempWorkspace {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

struct SessionStartFixturePlugin;

#[async_trait]
impl crate::plugin::sdk::Plugin for SessionStartFixturePlugin {
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {
        crate::plugin::sdk::PluginManifest::builder("session-start-fixture", "0.1.0")
            .hooks(crate::plugin::sdk::HookSubscription::SESSION_START)
            .build()
    }

    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        _host: Arc<dyn crate::plugin::sdk::host_api::HostClient>,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::InitOutcome> {
        Ok(crate::plugin::sdk::InitOutcome::ack(self.manifest()))
    }

    async fn session_start(
        &self,
        _input: crate::plugin::sdk::SessionStartInput,
    ) -> crate::plugin::sdk::Result<Option<crate::plugin::sdk::SessionStartPatch>> {
        Ok(Some(crate::plugin::sdk::SessionStartPatch {
            additional_context: Some("fixture context".to_string()),
            initial_user_message: Some("fixture user prompt".to_string()),
        }))
    }
}

struct SessionEndFixturePlugin {
    tx: tokio::sync::mpsc::UnboundedSender<crate::plugin::sdk::SessionEndInput>,
}

#[async_trait]
impl crate::plugin::sdk::Plugin for SessionEndFixturePlugin {
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {
        crate::plugin::sdk::PluginManifest::builder("session-end-fixture", "0.1.0")
            .hooks(crate::plugin::sdk::HookSubscription::SESSION_END)
            .build()
    }

    async fn session_end(
        &self,
        input: crate::plugin::sdk::SessionEndInput,
    ) -> crate::plugin::sdk::Result<()> {
        let _ = self.tx.send(input);
        Ok(())
    }
}

struct HostInvokeSourceFixturePlugin {
    host: tokio::sync::RwLock<Option<Arc<dyn crate::plugin::sdk::host_api::HostClient>>>,
}

impl HostInvokeSourceFixturePlugin {
    fn new() -> Self {
        Self {
            host: tokio::sync::RwLock::new(None),
        }
    }
}

#[async_trait]
impl crate::plugin::sdk::Plugin for HostInvokeSourceFixturePlugin {
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {
        crate::plugin::sdk::PluginManifest::builder("host-invoke-source-fixture", "0.1.0")
            .tool(
                crate::plugin::sdk::PluginToolDecl::new(
                    "host_invoke_source",
                    serde_json::json!({"type": "object"}),
                )
                .description("Call another tool through host/tool.invoke.")
                .host_capability(crate::plugin::sdk::HostCapability::InvokeTool),
            )
            .build()
    }

    async fn init(
        &self,
        _ctx: crate::plugin::sdk::InitContext,
        host: Arc<dyn crate::plugin::sdk::host_api::HostClient>,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::InitOutcome> {
        *self.host.write().await = Some(host);
        Ok(crate::plugin::sdk::InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "host_invoke_source" => {
                let host = self
                    .host
                    .read()
                    .await
                    .clone()
                    .expect("host client should be installed");
                host.invoke_tool("host_invoke_target".to_string(), serde_json::json!({}))
                    .await
            }
            other => Err(crate::plugin::PluginError::new(format!(
                "unexpected tool {other}"
            ))),
        }
    }
}

struct HostInvokeTargetFixturePlugin;

#[async_trait]
impl crate::plugin::sdk::Plugin for HostInvokeTargetFixturePlugin {
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {
        crate::plugin::sdk::PluginManifest::builder("host-invoke-target-fixture", "0.1.0")
            .tool(
                crate::plugin::sdk::PluginToolDecl::new(
                    "host_invoke_target",
                    serde_json::json!({"type": "object"}),
                )
                .description("Target tool for host/tool.invoke."),
            )
            .build()
    }

    async fn tool_invoke(
        &self,
        input: crate::plugin::sdk::ToolInvokeInput,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
        match input.tool_name.as_str() {
            "host_invoke_target" => {
                Ok(crate::plugin::sdk::ToolInvokeOutput::text("target ok").with_title("Target"))
            }
            other => Err(crate::plugin::PluginError::new(format!(
                "unexpected tool {other}"
            ))),
        }
    }
}

struct StreamingFixturePlugin {
    chunk_sent: Arc<tokio::sync::Notify>,
    finish: Arc<tokio::sync::Notify>,
}

#[async_trait]
impl crate::plugin::sdk::Plugin for StreamingFixturePlugin {
    fn manifest(&self) -> crate::plugin::sdk::PluginManifest {
        crate::plugin::sdk::PluginManifest::builder("streaming-fixture", "0.1.0")
            .hooks(crate::plugin::sdk::HookSubscription::TOOL_INVOKE_STREAM)
            .tool(
                crate::plugin::sdk::PluginToolDecl::new(
                    "stream_fixture_count",
                    serde_json::json!({
                        "type": "object",
                        "properties": { "n": { "type": "integer" } }
                    }),
                )
                .description("Stream fixture count.")
                .streaming(crate::plugin::sdk::ToolStreamingMode::Streaming),
            )
            .build()
    }

    async fn tool_invoke_stream(
        &self,
        _input: crate::plugin::sdk::ToolInvokeInput,
        sink: crate::plugin::sdk::ToolStreamSink,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolStreamEnd> {
        let stream_id = sink.stream_id().to_string();
        sink.chunk(crate::plugin::sdk::ToolStreamChunk {
            stream_id: stream_id.clone(),
            text_delta: Some("partial ".to_string()),
            payload_delta: None,
            metadata: Default::default(),
        })
        .await;
        self.chunk_sent.notify_waiters();
        self.finish.notified().await;
        Ok(crate::plugin::sdk::ToolStreamEnd {
            stream_id,
            title: "Stream fixture".to_string(),
            output_text: "partial done".to_string(),
            payload: None,
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }
}

struct ScriptedProvider;

#[derive(Clone)]
struct RecordingProvider {
    requests: Arc<Mutex<Vec<CompletionRequest>>>,
    next_response_id: Arc<Mutex<u64>>,
    metadata: crate::provider::ModelMetadata,
    usage: Option<CompletionUsage>,
    response_delay: Option<Duration>,
    current_prompt_cache_shape: Arc<Mutex<Option<crate::provider::PromptCacheShape>>>,
    dynamic_prompt_cache_shape: Option<crate::provider::PromptCacheShape>,
    remote_compact_error: Option<String>,
}

fn scripted_provider_id() -> ProviderId {
    ProviderId::new("scripted")
}

fn scripted_model_id() -> ModelId {
    ModelId::new("scripted-model")
}

fn scripted_model_ref() -> ModelRef {
    ModelRef::new("scripted", "scripted-model")
}

fn recording_provider_id() -> ProviderId {
    ProviderId::new("recording")
}

fn recording_model_id() -> ModelId {
    ModelId::new("recording-model")
}

fn recording_model_ref() -> ModelRef {
    ModelRef::new("recording", "recording-model")
}

impl RecordingProvider {
    fn new(requests: Arc<Mutex<Vec<CompletionRequest>>>) -> Self {
        Self {
            requests,
            next_response_id: Arc::new(Mutex::new(0)),
            metadata: crate::provider::ModelMetadata::default(),
            usage: None,
            response_delay: None,
            current_prompt_cache_shape: Arc::new(Mutex::new(None)),
            dynamic_prompt_cache_shape: None,
            remote_compact_error: None,
        }
    }

    fn next_response_id(&self) -> String {
        let mut guard = self
            .next_response_id
            .lock()
            .expect("recording provider response id lock should succeed");
        *guard += 1;
        format!("resp_{}", *guard)
    }

    #[allow(dead_code)]
    fn with_metadata(mut self, metadata: crate::provider::ModelMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    #[allow(dead_code)]
    fn with_usage(mut self, usage: CompletionUsage) -> Self {
        self.usage = Some(usage);
        self
    }

    fn with_dynamic_prompt_cache_shape(mut self, shape: crate::provider::PromptCacheShape) -> Self {
        self.dynamic_prompt_cache_shape = Some(shape);
        self
    }

    fn with_remote_compact_error(mut self, message: impl Into<String>) -> Self {
        self.remote_compact_error = Some(message.into());
        self
    }
}

#[async_trait]
impl ModelRuntime for ScriptedProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("scripted-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new("scripted", "scripted-model").with_display_name("Scripted"),
        ])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Ok(CompletionResponse {
            provider_id: scripted_provider_id(),
            model: scripted_model_id(),
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
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let last_user_text = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(Message::as_text_lossy)
            .unwrap_or_default();

        let tool_result = request.messages.iter().find_map(|message| {
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some("call_apply_patch_1") {
                    return None;
                }
                let operation = match part.content.as_ref() {
                    Some(PartContent::Operation(operation)) => operation,
                    _ => return None,
                };
                match part.status {
                    ExecutionStatus::Completed => Some(Ok(operation.model_output.text.clone())),
                    ExecutionStatus::Failed => Some(Err(operation
                        .error_message()
                        .unwrap_or(operation.model_output.text.as_str())
                        .to_string())),
                    _ => None,
                }
            })
        });
        let user_input_result = request.messages.iter().find_map(|message| {
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some("call_ask_user_1") {
                    return None;
                }
                let operation = match part.content.as_ref() {
                    Some(PartContent::Operation(operation)) => operation,
                    _ => return None,
                };
                match part.status {
                    ExecutionStatus::Completed => {
                        let answers = answers_from_tool_output(&operation.details)?;
                        answers
                            .get("model_choice")
                            .and_then(|values| values.first().cloned())
                            .map(Ok)
                    }
                    ExecutionStatus::Failed => Some(Err(operation
                        .error_message()
                        .unwrap_or(operation.model_output.text.as_str())
                        .to_string())),
                    _ => None,
                }
            })
        });
        let todo_result = request.messages.iter().find_map(|message| {
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some("call_todo_1") {
                    return None;
                }
                let operation = match part.content.as_ref() {
                    Some(PartContent::Operation(operation)) => operation,
                    _ => return None,
                };
                match part.status {
                    ExecutionStatus::Completed => Some(Ok(())),
                    ExecutionStatus::Failed => Some(Err(operation
                        .error_message()
                        .unwrap_or(operation.model_output.text.as_str())
                        .to_string())),
                    _ => None,
                }
            })
        });
        let stream_tool_result = request.messages.iter().find_map(|message| {
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some("call_stream_tool_1") {
                    return None;
                }
                let operation = match part.content.as_ref() {
                    Some(PartContent::Operation(operation)) => operation,
                    _ => return None,
                };
                match part.status {
                    ExecutionStatus::Completed => Some(Ok(operation.model_output.text.clone())),
                    ExecutionStatus::Failed => Some(Err(operation
                        .error_message()
                        .unwrap_or(operation.model_output.text.as_str())
                        .to_string())),
                    _ => None,
                }
            })
        });
        let apply_patch_tool_loaded = request.messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                let operation = match part.content.as_ref() {
                    Some(PartContent::Operation(operation))
                        if part.status == ExecutionStatus::Completed =>
                    {
                        operation
                    }
                    _ => return false,
                };
                loaded_tools_from_tool_output(&operation.details)
                    .is_some_and(|loaded_tools| loaded_tools.iter().any(|name| name == "fs_edit"))
            })
        });

        let events = if last_user_text.contains("permission todo") && todo_result.is_none() {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    stream_key: "call_todo_1".to_string(),
                    id: Some("call_todo_1".to_string()),
                    name: Some("todo".to_string()),
                    arguments_delta: serde_json::json!({
                        "command": "write",
                        "args": TodoWriteToolInput {
                            items: vec![TodoItem {
                                content: "confirm permission recovery".to_string(),
                                status: TodoStatus::Completed,
                                priority: TodoPriority::Low,
                            }],
                        },
                    })
                    .to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if let Some(todo_result) = todo_result {
            let delta = match todo_result {
                Ok(()) => "permission todo done".to_string(),
                Err(_) => "permission todo failed".to_string(),
            };
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    delta,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if last_user_text.contains("stream plugin") && stream_tool_result.is_none() {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    stream_key: "call_stream_tool_1".to_string(),
                    id: Some("call_stream_tool_1".to_string()),
                    name: Some("stream_fixture_count".to_string()),
                    arguments_delta: serde_json::json!({ "n": 5 }).to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if let Some(stream_tool_result) = stream_tool_result {
            let delta = match stream_tool_result {
                Ok(output) => format!("stream tool done: {output}"),
                Err(_) => "stream tool failed".to_string(),
            };
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    delta,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if last_user_text.contains("patch")
            && tool_result.is_none()
            && !apply_patch_tool_loaded
        {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    stream_key: "call_tool_search_1".to_string(),
                    id: Some("call_tool_search_1".to_string()),
                    name: Some("tools".to_string()),
                    arguments_delta: serde_json::json!({
                        "command": "search",
                        "args": ToolSearchToolInput {
                            query: "patch file".to_string(),
                            load: vec!["fs_edit".to_string()],
                            limit: None,
                        },
                    })
                    .to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if last_user_text.contains("choose model") && user_input_result.is_none() {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    stream_key: "call_ask_user_1".to_string(),
                    id: Some("call_ask_user_1".to_string()),
                    name: Some("user".to_string()),
                    arguments_delta: serde_json::json!({
                        "command": "ask",
                        "args": AskUserToolInput {
                            questions: vec![UserInputQuestion {
                                id: "model_choice".to_string(),
                                header: "Model".to_string(),
                                question: "Which model should we use?".to_string(),
                                options: vec![
                                    UserInputOption {
                                        label: "gpt-5".to_string(),
                                        description: "Use the flagship reasoning model."
                                            .to_string(),
                                    },
                                    UserInputOption {
                                        label: "gpt-4.1".to_string(),
                                        description: "Use the faster general-purpose model."
                                            .to_string(),
                                    },
                                ],
                                multiple: false,
                                allow_custom: false,
                            }],
                        },
                    })
                    .to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if let Some(user_input_result) = user_input_result {
            let delta = match user_input_result {
                Ok(answer) => format!("selected model: {answer}"),
                Err(_) => "selection cancelled".to_string(),
            };
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    delta,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if last_user_text.contains("patch") && tool_result.is_none() {
            vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        stream_key: "call_apply_patch_1".to_string(),
                        id: Some("call_apply_patch_1".to_string()),
                        name: Some("fs_edit".to_string()),
                        arguments_delta: serde_json::json!({
                            "command": "apply_patch",
                            "args": ApplyPatchToolInput {
                                patch: "*** Begin Patch\n*** Add File: result.txt\n+approved\n*** End Patch"
                                    .to_string(),
                            },
                        })
                        .to_string(),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: scripted_provider_id(),
                        model: scripted_model_id(),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
        } else if let Some(tool_result) = tool_result {
            let delta = match tool_result {
                Ok(_) => "patch done".to_string(),
                Err(_) => "patch denied".to_string(),
            };
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    delta,
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else {
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    delta: format!("echo:{last_user_text}"),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        };

        Ok(Box::pin(stream::iter(events)))
    }
}

#[derive(Clone, Copy)]
enum ToolErrorRecoveryScenario {
    BadTodo,
    ParallelBadTools,
}

struct ToolErrorRecoveryProvider {
    scenario: ToolErrorRecoveryScenario,
}

impl ToolErrorRecoveryProvider {
    fn bad_todo() -> Self {
        Self {
            scenario: ToolErrorRecoveryScenario::BadTodo,
        }
    }

    fn parallel_bad_tools() -> Self {
        Self {
            scenario: ToolErrorRecoveryScenario::ParallelBadTools,
        }
    }
}

#[async_trait]
impl ModelRuntime for ToolErrorRecoveryProvider {
    fn id(&self) -> &str {
        "scripted"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("scripted-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new("scripted", "scripted-model").with_display_name("Scripted"),
        ])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Ok(CompletionResponse {
            provider_id: scripted_provider_id(),
            model: scripted_model_id(),
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
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let events = match self.scenario {
            ToolErrorRecoveryScenario::BadTodo => {
                if completed_or_failed_operation_count(&request, &["call_todo_1"]) == 0 {
                    scripted_tool_call_events(vec![(
                        "call_todo_1",
                        "todo",
                        serde_json::json!({
                            "command": "write",
                        })
                        .to_string(),
                    )])
                } else {
                    scripted_text_events("permission todo failed")
                }
            }
            ToolErrorRecoveryScenario::ParallelBadTools => {
                if completed_or_failed_operation_count(
                    &request,
                    &["call_bad_tools_1", "call_bad_tools_2"],
                ) == 0
                {
                    scripted_tool_call_events(vec![
                        (
                            "call_bad_tools_1",
                            "tools",
                            serde_json::json!({
                                "command": "search",
                            })
                            .to_string(),
                        ),
                        (
                            "call_bad_tools_2",
                            "tools",
                            serde_json::json!({
                                "command": "search",
                                "args": ToolSearchToolInput {
                                    query: "todo".to_string(),
                                    load: Vec::new(),
                                    limit: Some(1),
                                },
                            })
                            .to_string(),
                        ),
                    ])
                } else {
                    scripted_text_events("parallel tool failures returned")
                }
            }
        };

        Ok(Box::pin(stream::iter(events)))
    }
}

fn completed_or_failed_operation_count(
    request: &CompletionRequest,
    operation_ids: &[&str],
) -> usize {
    request
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter(|part| {
            part.operation_id
                .as_deref()
                .is_some_and(|operation_id| operation_ids.contains(&operation_id))
        })
        .filter(|part| {
            matches!(
                part.status,
                ExecutionStatus::Completed | ExecutionStatus::Failed
            )
        })
        .count()
}

fn scripted_tool_call_events(
    calls: Vec<(&'static str, &'static str, String)>,
) -> Vec<Result<CompletionStreamEvent, AppError>> {
    let mut events = calls
        .into_iter()
        .map(|(id, name, arguments_delta)| {
            Ok(CompletionStreamEvent::ToolCallDelta {
                provider_id: scripted_provider_id(),
                model: scripted_model_id(),
                stream_key: id.to_string(),
                id: Some(id.to_string()),
                name: Some(name.to_string()),
                arguments_delta,
            })
        })
        .collect::<Vec<_>>();

    events.push(Ok(CompletionStreamEvent::Completed {
        provider_id: scripted_provider_id(),
        model: scripted_model_id(),
        finish_reason: Some(CompletionFinishReason::ToolCalls),
        usage: None,
        provider_metadata: None,
    }));
    events
}

fn scripted_text_events(delta: &str) -> Vec<Result<CompletionStreamEvent, AppError>> {
    vec![
        Ok(CompletionStreamEvent::TextDelta {
            provider_id: scripted_provider_id(),
            model: scripted_model_id(),
            delta: delta.to_string(),
        }),
        Ok(CompletionStreamEvent::Completed {
            provider_id: scripted_provider_id(),
            model: scripted_model_id(),
            finish_reason: Some(CompletionFinishReason::Stop),
            usage: None,
            provider_metadata: None,
        }),
    ]
}

async fn build_manager(
    root: &std::path::Path,
    permission_policy: PermissionPolicy,
    config: SessionManagerConfig,
) -> SessionManager {
    build_manager_with_provider(
        root,
        permission_policy,
        config,
        ContextPolicy::default(),
        ScriptedProvider,
    )
    .await
}

async fn build_manager_with_provider<P>(
    root: &std::path::Path,
    permission_policy: PermissionPolicy,
    config: SessionManagerConfig,
    context_policy: ContextPolicy,
    provider: P,
) -> SessionManager
where
    P: ModelRuntime + 'static,
{
    let db = Database::connect("sqlite::memory:")
        .await
        .expect("failed to create sqlite db");
    init_schema(&db).await.expect("failed to init schema");

    build_manager_with_provider_on_db(
        root,
        db,
        permission_policy,
        ToolPermissionPolicy::allow_all(),
        config,
        context_policy,
        provider,
    )
    .await
}

async fn open_temp_database(root: &std::path::Path, name: &str) -> DatabaseConnection {
    let path = root.join(name);
    let db = Database::connect(format!("sqlite://{}?mode=rwc", path.display()))
        .await
        .expect("failed to create sqlite db");
    init_schema(&db).await.expect("failed to init schema");
    db
}

async fn build_session_start_plugin_host(
    workspace_root: &std::path::Path,
) -> Arc<crate::plugin::PluginHost> {
    let mut list = BTreeMap::new();
    list.insert(
        "fixture".to_string(),
        crate::plugin::PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            disabled: false,
        },
    );
    let config = crate::plugin::PluginsConfig {
        enabled: true,
        timeouts: Default::default(),
        list,
        trusted_keys: Default::default(),
        default_quota: Default::default(),
        quotas: Default::default(),
        tool_presentation: Default::default(),
    };
    crate::plugin::PluginHostBuilder::new(workspace_root, "test")
        .with_config(config)
        .register_static("fixture", SessionStartFixturePlugin)
        .build()
        .await
        .expect("plugin host should build")
}

async fn build_session_end_plugin_host(
    workspace_root: &std::path::Path,
) -> (
    Arc<crate::plugin::PluginHost>,
    tokio::sync::mpsc::UnboundedReceiver<crate::plugin::sdk::SessionEndInput>,
) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    let mut list = BTreeMap::new();
    list.insert(
        "fixture".to_string(),
        crate::plugin::PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            disabled: false,
        },
    );
    let config = crate::plugin::PluginsConfig {
        enabled: true,
        timeouts: Default::default(),
        list,
        trusted_keys: Default::default(),
        default_quota: Default::default(),
        quotas: Default::default(),
        tool_presentation: Default::default(),
    };
    let host = crate::plugin::PluginHostBuilder::new(workspace_root, "test")
        .with_config(config)
        .register_static("fixture", SessionEndFixturePlugin { tx })
        .build()
        .await
        .expect("plugin host should build");
    (host, rx)
}

async fn build_host_invoke_plugin_host(
    workspace_root: &std::path::Path,
) -> Arc<crate::plugin::PluginHost> {
    let mut list = BTreeMap::new();
    list.insert(
        "source".to_string(),
        crate::plugin::PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            disabled: false,
        },
    );
    list.insert(
        "target".to_string(),
        crate::plugin::PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            disabled: false,
        },
    );
    let config = crate::plugin::PluginsConfig {
        enabled: true,
        timeouts: Default::default(),
        list,
        trusted_keys: Default::default(),
        default_quota: Default::default(),
        quotas: Default::default(),
        tool_presentation: Default::default(),
    };
    crate::plugin::PluginHostBuilder::new(workspace_root, "test")
        .with_config(config)
        .register_static("source", HostInvokeSourceFixturePlugin::new())
        .register_static("target", HostInvokeTargetFixturePlugin)
        .build()
        .await
        .expect("plugin host should build")
}

async fn build_streaming_plugin_host(
    workspace_root: &std::path::Path,
) -> (
    Arc<crate::plugin::PluginHost>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Notify>,
) {
    let chunk_sent = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Notify::new());
    let mut list = BTreeMap::new();
    list.insert(
        "fixture".to_string(),
        crate::plugin::PluginEntry::Static {
            options: serde_json::Value::Null,
            timeouts: Default::default(),
            disabled: false,
        },
    );
    let config = crate::plugin::PluginsConfig {
        enabled: true,
        timeouts: Default::default(),
        list,
        trusted_keys: Default::default(),
        default_quota: Default::default(),
        quotas: Default::default(),
        tool_presentation: Default::default(),
    };
    let host = crate::plugin::PluginHostBuilder::new(workspace_root, "test")
        .with_config(config)
        .register_static(
            "fixture",
            StreamingFixturePlugin {
                chunk_sent: Arc::clone(&chunk_sent),
                finish: Arc::clone(&finish),
            },
        )
        .build()
        .await
        .expect("plugin host should build");
    (host, chunk_sent, finish)
}

#[derive(Clone)]
struct HostInvokeRuntimeTestHostClient {
    manager: Arc<tokio::sync::RwLock<Option<Arc<SessionManager>>>>,
}

impl HostInvokeRuntimeTestHostClient {
    fn new() -> Self {
        Self {
            manager: Arc::new(tokio::sync::RwLock::new(None)),
        }
    }

    async fn install_manager(&self, manager: Arc<SessionManager>) {
        *self.manager.write().await = Some(manager);
    }
}

fn host_invoke_execution_output(
    execution: ToolInvocationExecution,
) -> crate::plugin::sdk::ToolInvokeOutput {
    crate::plugin::sdk::ToolInvokeOutput {
        title: execution.view.title,
        output_text: execution.view.output_text,
        payload: execution.output.to_json_payload(),
        metadata: execution.view.metadata.into_iter().collect(),
        attachments: execution.view.attachments,
    }
}

#[async_trait::async_trait]
impl crate::plugin::sdk::host_api::HostClient for HostInvokeRuntimeTestHostClient {
    async fn log(
        &self,
        _level: crate::plugin::sdk::host_api::LogLevel,
        _message: String,
        _fields: serde_json::Value,
    ) {
    }

    async fn publish_event(
        &self,
        _env: crate::plugin::sdk::EventEnvelope,
    ) -> crate::plugin::sdk::Result<()> {
        Ok(())
    }

    async fn subscribe_events(
        &self,
        _filter: crate::plugin::sdk::EventFilter,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::EventSubscription> {
        Ok(crate::plugin::sdk::host_api::EventSubscription { id: "sub".into() })
    }

    async fn ask_permission(
        &self,
        _req: crate::plugin::sdk::PermissionAskInput,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::PermissionDecision> {
        Ok(crate::plugin::sdk::PermissionDecision::Prompt)
    }

    async fn read_config(
        &self,
        _path: Option<String>,
    ) -> crate::plugin::sdk::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    async fn invoke_tool(
        &self,
        tool: String,
        input: serde_json::Value,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
        let manager = self
            .manager
            .read()
            .await
            .clone()
            .ok_or_else(|| crate::plugin::PluginError::new("session manager not installed"))?;
        let context = crate::plugin::sdk::host_api::current_host_callback_context()
            .ok_or_else(|| crate::plugin::PluginError::new("missing host callback context"))?;
        let session_id = context
            .session_id
            .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
        let call_id = context.call_id.unwrap_or(-1);
        let structured = crate::message::StructuredObject::try_from(input)
            .map_err(|err| crate::plugin::PluginError::invalid_params(err.to_string()))?;
        let invocation = ToolInvocation::new(tool, structured);
        let execution = manager
            .execute_host_invoked_tool(session_id, call_id, invocation)
            .await
            .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
        Ok(host_invoke_execution_output(execution))
    }
}

#[derive(Clone)]
struct SessionTestHostClient {
    executor: ToolExecutor,
}

#[async_trait::async_trait]
impl crate::plugin::sdk::host_api::HostClient for SessionTestHostClient {
    async fn log(
        &self,
        _level: crate::plugin::sdk::host_api::LogLevel,
        _message: String,
        _fields: serde_json::Value,
    ) {
    }

    async fn publish_event(
        &self,
        _env: crate::plugin::sdk::EventEnvelope,
    ) -> crate::plugin::sdk::Result<()> {
        Ok(())
    }

    async fn subscribe_events(
        &self,
        _filter: crate::plugin::sdk::EventFilter,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::EventSubscription> {
        Ok(crate::plugin::sdk::host_api::EventSubscription { id: "sub".into() })
    }

    async fn ask_permission(
        &self,
        _req: crate::plugin::sdk::PermissionAskInput,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::PermissionDecision> {
        Ok(crate::plugin::sdk::PermissionDecision::Prompt)
    }

    async fn read_config(
        &self,
        _path: Option<String>,
    ) -> crate::plugin::sdk::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    async fn invoke_tool(
        &self,
        tool: String,
        _input: serde_json::Value,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
        Err(crate::plugin::PluginError::new(format!(
            "unexpected invoke_tool for {tool}"
        )))
    }

    async fn list_tools(
        &self,
    ) -> crate::plugin::sdk::Result<Vec<crate::plugin::sdk::host_api::ToolDescriptor>> {
        Ok(self
            .executor
            .searchable_tools()
            .into_iter()
            .map(|tool| {
                let deferred = tool.is_deferred();
                let description = tool.description_text().to_string();
                let summary = tool.summary_text().map(ToString::to_string);
                let help = tool.help_text().map(ToString::to_string);
                let input_schema = Some(tool.sanitized_input_schema());
                let description_mode = tool.decl.description_mode;
                let tags = tool.effective_tags();
                crate::plugin::sdk::host_api::ToolDescriptor {
                    name: tool.exposed_name,
                    description: Some(description),
                    summary,
                    help,
                    input_schema,
                    description_mode,
                    tags,
                    deferred,
                    plugin_id: (!tool.plugin_name.trim().is_empty()).then_some(tool.plugin_name),
                }
            })
            .collect())
    }

    async fn todo_write(
        &self,
        req: crate::plugin::sdk::host_api::HostTodoWriteRequest,
    ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
        let context =
            crate::plugin::sdk::host_api::current_host_callback_context().unwrap_or_default();
        self.executor
            .execute_tool_payload_for_host(
                "todo_write",
                serde_json::to_value(req)
                    .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?,
                context.session_id.filter(|id| *id >= 0),
                context.call_id.filter(|id| *id >= 0),
                None,
            )
            .map_err(|err| crate::plugin::PluginError::new(err.to_string()))
    }
}

async fn build_manager_with_provider_on_db<P>(
    root: &std::path::Path,
    db: DatabaseConnection,
    permission_policy: PermissionPolicy,
    tool_policy: ToolPermissionPolicy,
    config: SessionManagerConfig,
    context_policy: ContextPolicy,
    provider: P,
) -> SessionManager
where
    P: ModelRuntime + 'static,
{
    let agents = crate::agents::SubagentRegistry::discover(root, None);
    let executor = ToolExecutor::new(
        root,
        Agent::new("build", permission_policy.clone()).with_tool_policy(tool_policy.clone()),
    )
    .with_subagent_registry(agents.clone());
    let plugins = crate::tool::default_tool_host(root).expect("default plugin host");
    plugins
        .host_handle()
        .install_client(Arc::new(SessionTestHostClient {
            executor: executor.clone().with_plugin_manager(Arc::clone(&plugins)),
        }))
        .await;
    let executor = executor.with_plugin_manager(plugins.clone());
    let mut registry = ProviderRegistry::new();
    registry.register(provider);
    let processor = SessionProcessor::new(Arc::new(registry), ContextGovernor::new(context_policy))
        .with_plugin_host(Arc::clone(&plugins));

    SessionManager::new(db, processor, executor).with_config(config)
}

#[allow(clippy::too_many_arguments)]
async fn build_manager_with_provider_and_plugins_on_db<P>(
    root: &std::path::Path,
    db: DatabaseConnection,
    permission_policy: PermissionPolicy,
    tool_policy: ToolPermissionPolicy,
    config: SessionManagerConfig,
    context_policy: ContextPolicy,
    provider: P,
    plugins: Arc<crate::plugin::PluginHost>,
) -> SessionManager
where
    P: ModelRuntime + 'static,
{
    let agents = crate::agents::SubagentRegistry::discover(root, None);
    let mut registry = ProviderRegistry::new();
    registry.register(provider);
    let processor = SessionProcessor::new(Arc::new(registry), ContextGovernor::new(context_policy))
        .with_plugin_host(Arc::clone(&plugins));
    let executor = ToolExecutor::new(
        root,
        Agent::new("build", permission_policy).with_tool_policy(tool_policy),
    )
    .with_subagent_registry(agents)
    .with_plugin_manager(plugins);

    SessionManager::new(db, processor, executor).with_config(config)
}

async fn resume_event_sequence(manager: &SessionManager) {
    manager
        .event_publisher()
        .resume_from_store()
        .await
        .expect("event sequence should resume from persisted history");
}

async fn persist_goal_without_auto_run(
    manager: &SessionManager,
    session_id: i64,
    objective: &str,
    _ignored_goal_limit: Option<u64>,
) -> SessionGoal {
    let state = manager.execution_state();
    let mut updated = manager
        .store
        .upsert_goal(session_id, objective.to_string(), state.cache_policy())
        .await
        .expect("upsert goal without auto run");
    let goal = updated
        .goal
        .clone()
        .expect("upserted goal should be present");
    updated.runtime.goal.clear();
    updated
        .runtime
        .goal
        .set_pending_steering(goal.id, GoalSteeringKind::ObjectiveUpdated);
    let updated = manager
        .persist_session_changes(updated, Vec::new(), Vec::new(), None, state)
        .await
        .expect("persist runtime goal state without auto run");
    updated
        .goal
        .expect("persisted goal should remain attached to session")
}

fn pending_permission_request_id(session: &Session) -> String {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .find_map(|part| match part.content.as_ref() {
            Some(PartContent::Request(crate::message::RequestPart::Permission(request)))
                if request.reply.is_none() =>
            {
                Some(request.request.request_id.clone())
            }
            _ => None,
        })
        .expect("session should contain a pending permission request")
}

struct InterruptibleProvider {
    call_count: Arc<std::sync::atomic::AtomicUsize>,
}

#[async_trait]
impl ModelRuntime for InterruptibleProvider {
    fn id(&self) -> &str {
        "interruptible"
    }

    fn default_model(&self) -> &ModelId {
        static MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("interruptible-model"));
        &MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![ProviderModel::new(
            "interruptible",
            "interruptible-model",
        )])
    }

    async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Err(AppError::Provider("streaming only".to_string()))
    }

    async fn complete_stream(
        &self,
        _: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        let call_index = self
            .call_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if call_index == 0 {
            let stream = async_stream::stream! {
                yield Ok(CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("interruptible"),
                    model: ModelId::new("interruptible-model"),
                    delta: "thinking".to_string(),
                });
                std::future::pending::<()>().await;
            };
            return Ok(Box::pin(stream));
        }

        Ok(Box::pin(stream::iter(vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new("interruptible"),
                model: ModelId::new("interruptible-model"),
                delta: "resumed work".to_string(),
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: ProviderId::new("interruptible"),
                model: ModelId::new("interruptible-model"),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: None,
                provider_metadata: None,
            }),
        ])))
    }
}

fn interruptible_options() -> SessionRunOptions {
    SessionRunOptions {
        model: ModelRef::new("interruptible", "interruptible-model"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: Some(64),
        agent_profile: None,
        max_turn_loops: None,
    }
}

async fn wait_for_active_turn(manager: &SessionManager, session_id: i64) {
    let registered = async {
        for _ in 0..500 {
            if manager.is_turn_active(session_id).await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        false
    }
    .await;
    assert!(registered, "turn should register within 10s");
}

async fn wait_for_provider_calls(
    call_count: &std::sync::atomic::AtomicUsize,
    expected_at_least: usize,
) {
    let started = async {
        for _ in 0..500 {
            if call_count.load(std::sync::atomic::Ordering::SeqCst) >= expected_at_least {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        false
    }
    .await;
    assert!(
        started,
        "provider should be invoked at least {expected_at_least} time(s) within 10s"
    );
}

async fn cancel_running_turn(
    manager: Arc<SessionManager>,
    session_id: i64,
    call_count: &std::sync::atomic::AtomicUsize,
) {
    let submit_manager = Arc::clone(&manager);
    let submit = tokio::spawn(async move {
        submit_manager
            .submit_user_turn(SessionUserTurnRequest {
                session_id,
                options: interruptible_options(),
                parts: vec![PartContent::text("start work")],
            })
            .await
    });

    wait_for_active_turn(manager.as_ref(), session_id).await;
    wait_for_provider_calls(call_count, 1).await;
    for attempt in 0..3 {
        match manager.cancel_active_turn(session_id).await {
            Ok(()) => break,
            Err(_) if attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(err) => panic!("cancel should find active turn: {err}"),
        }
    }

    let result = tokio::time::timeout(std::time::Duration::from_secs(15), submit)
        .await
        .expect("submit should complete after cancel")
        .expect("join");
    assert!(
        result.is_err(),
        "expected turn to be reported as failed/cancelled"
    );
}

#[tokio::test]
async fn host_invoked_tool_obeys_target_tool_permission_policy() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "host_invoke_permission.db").await;
    let plugins = build_host_invoke_plugin_host(&workspace.root).await;
    let manager = build_manager_with_provider_and_plugins_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all().with_tool_mode(
            "host_invoke_target",
            crate::permission::PermissionMode::Deny,
        ),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
        plugins,
    )
    .await;
    let session = manager
        .create_session(SessionCreateRequest {
            title: "host invoke permission".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let invocation = crate::message::ToolInvocation::new(
        "host_invoke_target",
        crate::message::StructuredObject::default(),
    );

    let err = manager
        .execute_host_invoked_tool(session.id, 42, invocation)
        .await
        .expect_err("host-invoked target should be denied");

    assert!(
        err.to_string().contains("permission denied"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn host_invoked_tool_executes_when_permissions_allow() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "host_invoke_allow.db").await;
    let plugins = build_host_invoke_plugin_host(&workspace.root).await;
    let manager = build_manager_with_provider_and_plugins_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
        plugins,
    )
    .await;
    let session = manager
        .create_session(SessionCreateRequest {
            title: "host invoke allow".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let invocation = crate::message::ToolInvocation::new(
        "host_invoke_target",
        crate::message::StructuredObject::default(),
    );

    let execution = manager
        .execute_host_invoked_tool(session.id, 42, invocation)
        .await
        .expect("host-invoked target should execute");

    assert_eq!(execution.view.output_text, "target ok");
}

#[tokio::test]
async fn host_tool_invoke_callback_obeys_target_tool_permission_policy() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "host_invoke_callback_permission.db").await;
    let plugins = build_host_invoke_plugin_host(&workspace.root).await;
    let host_client = HostInvokeRuntimeTestHostClient::new();
    plugins
        .host_handle()
        .install_client(Arc::new(host_client.clone()))
        .await;
    let manager = Arc::new(
        build_manager_with_provider_and_plugins_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all().with_tool_mode(
                "host_invoke_target",
                crate::permission::PermissionMode::Deny,
            ),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
            plugins,
        )
        .await,
    );
    host_client.install_manager(Arc::clone(&manager)).await;
    let session = manager
        .create_session(SessionCreateRequest {
            title: "host invoke callback permission".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let invocation = crate::message::ToolInvocation::new(
        "host_invoke_source",
        crate::message::StructuredObject::default(),
    );

    let err = manager
        .execute_host_invoked_tool(session.id, 42, invocation)
        .await
        .expect_err("host/tool.invoke target should be denied");

    assert!(
        err.to_string().contains("permission denied"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn host_tool_invoke_callback_executes_when_permissions_allow() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "host_invoke_callback_allow.db").await;
    let plugins = build_host_invoke_plugin_host(&workspace.root).await;
    let host_client = HostInvokeRuntimeTestHostClient::new();
    plugins
        .host_handle()
        .install_client(Arc::new(host_client.clone()))
        .await;
    let manager = Arc::new(
        build_manager_with_provider_and_plugins_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
            plugins,
        )
        .await,
    );
    host_client.install_manager(Arc::clone(&manager)).await;
    let session = manager
        .create_session(SessionCreateRequest {
            title: "host invoke callback allow".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");
    let invocation = crate::message::ToolInvocation::new(
        "host_invoke_source",
        crate::message::StructuredObject::default(),
    );

    let execution = manager
        .execute_host_invoked_tool(session.id, 42, invocation)
        .await
        .expect("host/tool.invoke target should execute");

    assert_eq!(execution.view.output_text, "target ok");
}

fn run_options() -> SessionRunOptions {
    SessionRunOptions {
        model: scripted_model_ref(),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: Some(128),
        agent_profile: None,
        max_turn_loops: None,
    }
}

fn recording_run_options() -> SessionRunOptions {
    SessionRunOptions {
        model: recording_model_ref(),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: Some("system".to_string()),
        temperature: Some(0.2),
        max_output_tokens: Some(256),
        agent_profile: None,
        max_turn_loops: None,
    }
}

#[allow(dead_code)]
fn interrupted_model_ref() -> ModelRef {
    ModelRef::new("interrupted", "interrupted-model")
}

#[allow(dead_code)]
fn interrupted_run_options() -> SessionRunOptions {
    SessionRunOptions {
        model: interrupted_model_ref(),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: Some(128),
        agent_profile: None,
        max_turn_loops: None,
    }
}

#[allow(dead_code)]
fn high_recording_usage() -> CompletionUsage {
    CompletionUsage {
        input_tokens: 3_800,
        output_tokens: 200,
        reasoning_tokens: 0,
        cache_write_tokens: 0,
        cache_read_tokens: 0,
        total_cost: 0.0,
    }
}

#[tokio::test]
async fn create_session_applies_session_start_patch_messages() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "session_start_patch.db").await;
    let plugins = build_session_start_plugin_host(&workspace.root).await;
    let manager = build_manager_with_provider_and_plugins_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
        plugins,
    )
    .await;

    let created = manager
        .create_session(SessionCreateRequest {
            title: "Session start fixture".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session creation should succeed");
    let session_id = created.id;
    let loaded = manager
        .get_session(session_id)
        .await
        .expect("session should reload");

    assert_eq!(loaded.messages.len(), 2);
    assert_eq!(loaded.messages[0].role, crate::role::Role::System);
    assert_eq!(loaded.messages[0].as_text_lossy(), "fixture context");
    assert_eq!(
        loaded.messages[0].metadata.source,
        crate::message::MessageSource::System
    );
    assert_eq!(loaded.messages[1].role, crate::role::Role::User);
    assert_eq!(loaded.messages[1].as_text_lossy(), "fixture user prompt");
    assert_eq!(
        loaded.messages[1].metadata.source,
        crate::message::MessageSource::System
    );
}

#[tokio::test]
async fn broadcast_active_session_end_notifies_plugins() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "session_end_broadcast.db").await;
    let (plugins, mut rx) = build_session_end_plugin_host(&workspace.root).await;
    let manager = build_manager_with_provider_and_plugins_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
        plugins,
    )
    .await;
    let created = manager
        .create_session(SessionCreateRequest {
            title: "Session end fixture".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session creation should succeed");
    let session_id = created.id;
    let (_control, _steer_rx) = manager.turn_registry.register(session_id).await;

    manager
        .broadcast_active_session_end(crate::plugin::SessionEndReason::Other)
        .await;

    let received = tokio::time::timeout(std::time::Duration::from_secs(1), rx.recv())
        .await
        .expect("session.end hook should arrive")
        .expect("session.end payload should be sent");
    assert_eq!(received.session_id, session_id);
    assert_eq!(received.reason, crate::plugin::SessionEndReason::Other);
}

#[test]
fn streaming_tool_execution_persists_in_progress_output() {
    std::thread::Builder::new()
        .name("streaming-tool-execution-test".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime should build")
                .block_on(streaming_tool_execution_persists_in_progress_output_impl())
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should complete");
}

async fn streaming_tool_execution_persists_in_progress_output_impl() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "streaming_tool_execution.db").await;
    let (plugins, chunk_sent, finish) = build_streaming_plugin_host(&workspace.root).await;
    let manager = Arc::new(
        build_manager_with_provider_and_plugins_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
            plugins,
        )
        .await,
    );
    let created = manager
        .create_session(SessionCreateRequest {
            title: "Streaming tool fixture".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session creation should succeed");
    let session_id = created.id;

    let chunk_ready = chunk_sent.notified();
    let manager_task = Arc::clone(&manager);
    let submit = tokio::spawn(async move {
        manager_task
            .submit_user_turn(SessionUserTurnRequest {
                session_id,
                options: run_options(),
                parts: vec![crate::message::PartContent::text("stream plugin")],
            })
            .await
    });

    tokio::time::timeout(std::time::Duration::from_secs(1), chunk_ready)
        .await
        .expect("streaming chunk should be emitted");

    let partial_output = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let session = manager
                .get_session(session_id)
                .await
                .expect("session should reload while streaming");
            if let Some(output_text) = session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .find_map(|part| match part.content.as_ref() {
                    Some(crate::message::PartContent::Operation(operation))
                        if part.operation_id.as_deref() == Some("call_stream_tool_1")
                            && part.status == ExecutionStatus::InProgress =>
                    {
                        Some(operation.model_output.text.clone())
                    }
                    _ => None,
                })
            {
                break output_text;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("streaming output should persist as in-progress");
    assert_eq!(partial_output, "partial ");

    finish.notify_waiters();
    let completed = submit
        .await
        .expect("submit task should join")
        .expect("streaming submit should complete");
    let final_output = completed
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .find_map(|part| match part.content.as_ref() {
            Some(crate::message::PartContent::Operation(operation))
                if part.operation_id.as_deref() == Some("call_stream_tool_1")
                    && part.status == ExecutionStatus::Completed =>
            {
                Some(operation.model_output.text.clone())
            }
            _ => None,
        })
        .expect("completed streamed tool output should exist");
    assert_eq!(final_output, "partial done");
}

fn operation_snapshot(
    session: &Session,
    operation_id: &str,
) -> (ExecutionStatus, Option<String>, String) {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .find_map(|part| {
            if part.operation_id.as_deref() != Some(operation_id) {
                return None;
            }
            match part.content.as_ref() {
                Some(PartContent::Operation(operation)) => Some((
                    part.status,
                    operation.error_message().map(ToString::to_string),
                    operation.model_output.text.clone(),
                )),
                _ => None,
            }
        })
        .unwrap_or_else(|| panic!("operation {operation_id} should exist"))
}

#[test]
fn tool_execution_error_is_returned_to_model_as_failed_tool_result() {
    std::thread::Builder::new()
        .name("tool-error-recovery".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime should build")
                .block_on(tool_execution_error_is_returned_to_model_as_failed_tool_result_impl())
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should complete");
}

async fn tool_execution_error_is_returned_to_model_as_failed_tool_result_impl() {
    let workspace = TempWorkspace::new();
    let manager = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ToolErrorRecoveryProvider::bad_todo(),
    )
    .await;
    let created = manager
        .create_session(SessionCreateRequest {
            title: "tool error recovery".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("bad todo")],
        })
        .await
        .expect("tool execution errors should not abort the session run");

    let (status, error, output_text) = operation_snapshot(&session, "call_todo_1");
    assert_eq!(status, ExecutionStatus::Failed);
    let failure = error.as_deref().unwrap_or(output_text.as_str()).to_string();
    assert!(
        failure.contains("missing field `args`"),
        "unexpected failure text: {failure}"
    );
    assert!(session.messages.iter().rev().any(|message| {
        message.role == Role::Assistant
            && message.as_text_lossy().contains("permission todo failed")
    }));
}

#[test]
fn concurrent_tool_execution_error_is_returned_without_dropping_other_results() {
    std::thread::Builder::new()
        .name("concurrent-tool-error-recovery".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime should build")
                .block_on(
                    concurrent_tool_execution_error_is_returned_without_dropping_other_results_impl(
                    ),
                )
        })
        .expect("test thread should spawn")
        .join()
        .expect("test thread should complete");
}

async fn concurrent_tool_execution_error_is_returned_without_dropping_other_results_impl() {
    let workspace = TempWorkspace::new();
    let manager = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ToolErrorRecoveryProvider::parallel_bad_tools(),
    )
    .await;
    let created = manager
        .create_session(SessionCreateRequest {
            title: "parallel tool error recovery".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("session should be created");

    let session = manager
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("parallel bad tools")],
        })
        .await
        .expect("parallel tool execution errors should not abort the session run");

    let (failed_status, failed_error, failed_output) =
        operation_snapshot(&session, "call_bad_tools_1");
    assert_eq!(failed_status, ExecutionStatus::Failed);
    let failure = failed_error
        .as_deref()
        .unwrap_or(failed_output.as_str())
        .to_string();
    assert!(
        failure.contains("missing field `args`"),
        "unexpected failure text: {failure}"
    );

    let (success_status, _success_error, success_output) =
        operation_snapshot(&session, "call_bad_tools_2");
    assert_eq!(success_status, ExecutionStatus::Completed);
    assert!(
        success_output.contains("Found"),
        "unexpected success output: {success_output}"
    );
    assert!(session.messages.iter().rev().any(|message| {
        message.role == Role::Assistant
            && message
                .as_text_lossy()
                .contains("parallel tool failures returned")
    }));
}

#[allow(dead_code)]
struct InterruptedStreamProvider;

#[async_trait]
impl ModelRuntime for InterruptedStreamProvider {
    fn id(&self) -> &str {
        "interrupted"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("interrupted-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![ProviderModel::new("interrupted", "interrupted-model")])
    }

    async fn complete(&self, _request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        Err(AppError::Provider(
            "interrupted provider only supports streaming".to_string(),
        ))
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
        AppError,
    > {
        Ok(Box::pin(stream::iter(vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new("interrupted"),
                model: ModelId::new("interrupted-model"),
                delta: "partial reply".to_string(),
            }),
            Err(AppError::Provider("stream interrupted".to_string())),
        ])))
    }
}

#[async_trait]
impl ModelRuntime for RecordingProvider {
    fn id(&self) -> &str {
        "recording"
    }

    fn default_model(&self) -> &ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("recording-model"));
        &DEFAULT_MODEL
    }

    fn model_metadata(&self, _model: &ModelId) -> crate::provider::ModelMetadata {
        self.metadata.clone()
    }

    fn supports_prompt_continuation(&self, _model: &ModelId) -> bool {
        true
    }

    fn prompt_cache_shape(&self, _model: &ModelId) -> Option<crate::provider::PromptCacheShape> {
        self.current_prompt_cache_shape
            .lock()
            .expect("recording provider prompt cache shape lock should succeed")
            .clone()
    }

    async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
        Ok(vec![
            ProviderModel::new("recording", "recording-model").with_display_name("Recording"),
        ])
    }

    async fn complete(&self, request: CompletionRequest) -> Result<CompletionResponse, AppError> {
        self.requests
            .lock()
            .expect("recording provider request lock should succeed")
            .push(request);
        if let Some(delay) = self.response_delay {
            tokio::time::sleep(delay).await;
        }
        if let Some(shape) = self.dynamic_prompt_cache_shape.clone() {
            *self
                .current_prompt_cache_shape
                .lock()
                .expect("recording provider prompt cache shape lock should succeed") = Some(shape);
        }

        Ok(CompletionResponse {
            provider_id: recording_provider_id(),
            model: recording_model_id(),
            text: "recorded".to_string(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: self.usage.clone(),
            provider_metadata: Some(serde_json::json!({
                "response_id": self.next_response_id()
            })),
        })
    }

    async fn compact_conversation(
        &self,
        _request: CompletionRequest,
    ) -> Result<Option<String>, AppError> {
        if let Some(message) = self.remote_compact_error.as_ref() {
            return Err(AppError::Provider(message.clone()));
        }
        Ok(None)
    }
}

fn cache_state(session_id: i64, text: impl Into<String>) -> Session {
    Session::new(session_id, 1, format!("session-{session_id}"), Utc::now())
        .with_messages(vec![Message::prompt_text(Role::User, text.into())])
}

#[test]
fn operation_blocks_materialize_mcp_resources_as_attachment_blocks() {
    let invocation = ToolInvocation::new("resource_tool", Default::default());
    let blocks = operation_blocks_from_tool_output(
        &invocation,
        &ToolOutput::from_json_payload(Some(&serde_json::json!({
            "server": "fixtures",
            "tool": "resource_tool",
            "content_blocks": [
                {
                    "type": "image",
                    "mime": "image/png",
                    "url": "https://example.com/chart.png"
                },
                {
                    "type": "resource_link",
                    "uri": "https://example.com/report.pdf",
                    "title": "report"
                }
            ]
        })))
        .expect("payload should parse"),
        &[ToolAttachment {
            kind: crate::message::AttachmentKind::Audio,
            mime: "audio/mpeg".to_string(),
            source: AttachmentSource::Url {
                url: "https://example.com/audio.mp3".to_string(),
            },
            filename: Some("audio.mp3".to_string()),
            title: None,
            size_bytes: None,
            sha256: None,
            width: None,
            height: None,
            duration_ms: None,
            page_count: None,
        }],
        "",
    );

    let attachments = blocks
        .iter()
        .filter_map(OperationBlock::to_attachment_item)
        .collect::<Vec<_>>();
    assert_eq!(attachments.len(), 3);
    assert!(attachments.iter().any(|item| {
        item.kind == crate::message::AttachmentKind::Image
            && matches!(
                item.source,
                AttachmentSource::Url { ref url }
                    if url == "https://example.com/chart.png"
            )
    }));
    assert!(attachments.iter().any(|item| {
        item.kind == crate::message::AttachmentKind::Pdf
            && matches!(
                item.source,
                AttachmentSource::Url { ref url }
                    if url == "https://example.com/report.pdf"
            )
    }));
    assert!(attachments.iter().any(|item| {
        item.kind == crate::message::AttachmentKind::Audio
            && matches!(
                item.source,
                AttachmentSource::Url { ref url }
                    if url == "https://example.com/audio.mp3"
            )
    }));
}

#[test]
fn validate_user_input_reply_supports_multi_select_and_custom_answers() {
    let request = UserInputRequest {
        request_id: "ask-1".to_string(),
        session_id: Some(1),
        questions: vec![UserInputQuestion {
            id: "stack".to_string(),
            header: "Stack".to_string(),
            question: "Which stacks should we support?".to_string(),
            options: vec![
                UserInputOption {
                    label: "rust".to_string(),
                    description: String::new(),
                },
                UserInputOption {
                    label: "go".to_string(),
                    description: String::new(),
                },
            ],
            multiple: true,
            allow_custom: true,
        }],
        created_at: Utc::now(),
    };
    let reply = UserInputReply {
        request_id: "ask-1".to_string(),
        kind: UserInputReplyKind::Submit,
        answers: BTreeMap::from([(
            "stack".to_string(),
            vec![
                "rust".to_string(),
                "zig".to_string(),
                "rust".to_string(),
                "  ".to_string(),
            ],
        )]),
        reason: None,
    };

    let answers = validate_user_input_reply(&request, &reply).expect("reply should validate");
    assert_eq!(
        answers.get("stack"),
        Some(&vec!["rust".to_string(), "zig".to_string()])
    );
}

#[tokio::test]
async fn cache_eviction_falls_back_to_db_reload() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig {
            cache_max_sessions: 1,
            cache_ttl: Duration::from_secs(60),
            cache_max_bytes: usize::MAX,
            max_turn_loops: 16,
            doom_loop: crate::session::DoomLoopPolicy::default(),
            default_agent: None,
            permission: crate::agent::PermissionConfig::default(),
            auto_compaction: SessionAutoCompactionConfig::default(),
        },
    )
    .await;

    let first = service
        .create_session(SessionCreateRequest {
            title: "first".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create first session");
    let second = service
        .create_session(SessionCreateRequest {
            title: "second".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create second session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: first.id,
            options: run_options(),
            parts: vec![PartContent::text("hello one")],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: second.id,
            options: run_options(),
            parts: vec![PartContent::text("hello two")],
        })
        .await
        .expect("submit second turn");

    let reloaded = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: first.id,
            options: run_options(),
            parts: vec![PartContent::text("hello again")],
        })
        .await
        .expect("submit turn after cache eviction");

    assert!(
        reloaded
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .any(|message| message.as_text_lossy() == "hello one")
    );
    assert!(
        reloaded
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .any(|message| message.as_text_lossy() == "hello again")
    );
}

#[tokio::test]
async fn list_session_summaries_reports_workspace_order_counts_and_pagination() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let parent = service
        .create_session(SessionCreateRequest {
            title: "parent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create parent session");
    let child = service
        .create_session(SessionCreateRequest {
            title: "child".to_string(),
            parent_session_id: Some(parent.id),
        })
        .await
        .expect("create child session");
    let sibling = service
        .create_session(SessionCreateRequest {
            title: "sibling".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create sibling session");

    let updated_parent = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: parent.id,
            options: run_options(),
            parts: vec![PartContent::text("hello parent")],
        })
        .await
        .expect("update parent session");

    let session_ids = service
        .workspace_session_ids()
        .await
        .expect("list workspace session ids");
    assert_eq!(session_ids, vec![parent.id, sibling.id, child.id]);

    let summaries = service
        .list_session_summaries(SessionListRequest::default())
        .await
        .expect("list session summaries");
    assert_eq!(summaries.len(), 3);
    assert_eq!(summaries[0].id, parent.id);
    assert_eq!(summaries[0].title, "parent");
    assert_eq!(summaries[0].version, updated_parent.version);
    assert_eq!(summaries[0].message_count, 2);
    assert_eq!(summaries[0].child_session_count, 1);
    assert!(summaries[0].last_message_at.is_some());
    assert_eq!(summaries[1].id, sibling.id);
    assert_eq!(summaries[1].message_count, 0);
    assert_eq!(summaries[1].child_session_count, 0);
    assert_eq!(summaries[1].last_message_at, None);
    assert_eq!(summaries[2].id, child.id);
    assert_eq!(summaries[2].parent_id, Some(parent.id));

    let paged = service
        .list_session_summaries(SessionListRequest {
            offset: 1,
            limit: Some(1),
            include_subagents: false,
        })
        .await
        .expect("list paged session summaries");
    assert_eq!(paged.len(), 1);
    assert_eq!(paged[0].id, sibling.id);
}

#[tokio::test]
async fn spawn_subtask_reuses_real_child_session_for_same_task_id() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let parent = service
        .create_session(SessionCreateRequest {
            title: "parent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create parent session");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: parent.id,
            options: run_options(),
            parts: vec![PartContent::text("parent context")],
        })
        .await
        .expect("seed parent turn");

    let first = service
        .spawn_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "inspect".to_string(),
            prompt: TaskSubagentType::Explore.apply_prompt_guidance("look around"),
            subagent_type: TaskSubagentType::Explore,
            profile_name: None,
            task_id: Some("task-1".to_string()),
            command: None,
            requested_model: None,
        })
        .await
        .expect("spawn first subtask");

    let second = service
        .spawn_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "inspect again".to_string(),
            prompt: TaskSubagentType::Explore.apply_prompt_guidance("look around again"),
            subagent_type: TaskSubagentType::Explore,
            profile_name: None,
            task_id: Some("task-1".to_string()),
            command: None,
            requested_model: None,
        })
        .await
        .expect("resume existing subtask");

    assert_eq!(first.session.parent_id, Some(parent.id));
    assert_eq!(second.session.id, first.session.id);
    assert_eq!(
        second.session.runtime.execution.task_id.as_deref(),
        Some("task-1")
    );

    let summaries = service
        .list_session_summaries(SessionListRequest {
            include_subagents: true,
            ..SessionListRequest::default()
        })
        .await
        .expect("list session summaries");
    let child_count = summaries
        .iter()
        .filter(|summary| summary.parent_id == Some(parent.id))
        .count();
    assert_eq!(child_count, 1);
}

#[tokio::test]
async fn spawn_subtask_applies_registered_profile_context() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
            agents_dir.join("reviewer.md"),
            "---\ndescription: reviewer\nmode: all\nallowed_entries:\n  - fs\npermission:\n  path:\n    rules:\n      \"*.env\":\n        read: ask\n      \"*\":\n        read: allow\ndefault:\n  model: scripted-model/audit\naliases: [\"audit\"]\n---\nYou are a strict reviewer.",
        )
        .expect("write reviewer profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let parent = service
        .create_session(SessionCreateRequest {
            title: "parent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create parent session");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: parent.id,
            options: run_options(),
            parts: vec![PartContent::text("parent context")],
        })
        .await
        .expect("seed parent turn");

    let spawned = service
        .spawn_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "review changes".to_string(),
            prompt: "Inspect the implementation and call out risks.".to_string(),
            subagent_type: TaskSubagentType::Verify,
            profile_name: Some("audit".to_string()),
            task_id: Some("review-1".to_string()),
            command: None,
            requested_model: None,
        })
        .await
        .expect("spawn subtask");

    assert_eq!(spawned.profile_name.as_deref(), Some("reviewer"));
    assert_eq!(
        spawned.model_provider_id.as_deref(),
        Some(scripted_provider_id().as_str())
    );
    assert_eq!(spawned.model_id.as_deref(), Some("scripted-model/audit"));

    let child = service
        .get_session(spawned.session.id)
        .await
        .expect("load child session");
    assert_eq!(
        child.runtime.execution.agent_profile.as_deref(),
        Some("reviewer")
    );
    assert_eq!(child.runtime.allowed_tools(), ["fs"]);
    let system = child
        .runtime
        .execution
        .system_prompt_override
        .as_deref()
        .expect("system prompt override");
    assert!(system.contains("You are a strict reviewer."));
    assert!(system.contains("Delegated task:"));
    assert!(system.contains("Inspect the implementation"));
    let rules = &child.runtime.execution.agent_permission.path.rules;
    assert_eq!(rules.len(), 2);
    match rules.get("*.env") {
        Some(crate::agent::PathAccessRuleConfig::Modes(modes)) => {
            assert_eq!(modes.read, Some(crate::permission::PermissionMode::Ask));
        }
        other => panic!("expected *.env read rule, got {other:?}"),
    }
    match rules.get("*") {
        Some(crate::agent::PathAccessRuleConfig::Modes(modes)) => {
            assert_eq!(modes.read, Some(crate::permission::PermissionMode::Allow));
        }
        other => panic!("expected wildcard read rule, got {other:?}"),
    }
    assert_eq!(
        child.runtime.execution.agent_mode,
        Some(crate::agent::AgentMode::All)
    );
    assert!(!child.runtime.execution.agent_hidden);
    assert_eq!(child.runtime.execution.agent_color, None);
    assert!(child.runtime.execution.agent_run.is_empty());
}

#[tokio::test]
async fn submit_user_turn_applies_requested_root_agent_profile() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
            agents_dir.join("planner.md"),
            "---\ndescription: planner\nallowed_entries:\n  - fs\n  - shell\npermission:\n  path:\n    workspace:\n      read: allow\n      write: deny\n  entries:\n    names:\n      shell: ask\n    rules:\n      shell:\n        \"git push *\": deny\n        \"git *\": allow\n        \"*\": ask\ndefault:\n  model: scripted-model/plan\naliases: [\"plan\"]\n---\nYou are a precise planner.",
        )
        .expect("write planner profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "root agent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: SessionRunOptions {
                agent_profile: Some("plan".to_string()),
                ..run_options()
            },
            parts: vec![PartContent::text("Draft a plan.")],
        })
        .await
        .expect("submit turn");

    assert_eq!(
        session.runtime.execution.agent_profile.as_deref(),
        Some("planner")
    );
    assert_eq!(session.runtime.allowed_tools(), ["fs", "shell"]);
    assert_eq!(
        session.runtime.execution.system_prompt_override.as_deref(),
        Some("You are a precise planner.")
    );
    assert_eq!(
        session
            .runtime
            .execution
            .agent_permission
            .path
            .workspace
            .as_ref()
            .and_then(|modes| modes.write),
        Some(crate::permission::PermissionMode::Deny)
    );
    match session
        .runtime
        .execution
        .agent_permission
        .tools
        .rules
        .get("shell")
    {
        Some(crate::agent::ToolPermissionRules::Ordered(entries)) => {
            let collected = entries
                .iter()
                .map(|(pattern, mode)| (pattern.as_str(), *mode))
                .collect::<Vec<_>>();
            assert_eq!(collected.len(), 3);
            assert!(collected.contains(&("git push *", crate::permission::PermissionMode::Deny)));
            assert!(collected.contains(&("git *", crate::permission::PermissionMode::Allow)));
            assert!(collected.contains(&("*", crate::permission::PermissionMode::Ask)));
        }
        other => panic!("expected ordered shell tool rules, got {other:?}"),
    }
    assert_eq!(
        session.runtime.execution.model_provider_id.as_deref(),
        Some(scripted_provider_id().as_str())
    );
    assert_eq!(
        session.runtime.execution.model_id.as_deref(),
        Some("scripted-model/plan")
    );
    let user_message = session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::User)
        .expect("user message");
    assert_eq!(
        user_message.metadata.model_provider_id,
        scripted_provider_id().as_str()
    );
    assert_eq!(user_message.metadata.model_id, "scripted-model/plan");
}

#[tokio::test]
async fn submit_user_turn_parses_explicit_root_agent_provider_route() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
        agents_dir.join("router.md"),
        "---\ndescription: router\nallowed_entries:\n  - fs\ndefault:\n  provider: scripted\n  model: scripted-model/plan\naliases: [\"route\"]\n---\nYou route to an explicit provider model.",
    )
    .expect("write router profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "route model".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: SessionRunOptions {
                agent_profile: Some("route".to_string()),
                ..run_options()
            },
            parts: vec![PartContent::text("Use the routed model.")],
        })
        .await
        .expect("submit turn");

    assert_eq!(
        session.runtime.execution.agent_profile.as_deref(),
        Some("router")
    );
    assert_eq!(
        session.runtime.execution.model_provider_id.as_deref(),
        Some("scripted")
    );
    assert_eq!(
        session.runtime.execution.model_id.as_deref(),
        Some("scripted-model/plan")
    );
}

#[tokio::test]
async fn submit_user_turn_default_agent_keeps_root_session_model() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig {
            default_agent: Some("build".to_string()),
            ..SessionManagerConfig::default()
        },
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "implicit default agent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("hello")],
        })
        .await
        .expect("submit turn");

    assert_eq!(
        session.runtime.execution.agent_profile.as_deref(),
        Some("build")
    );
    assert_eq!(
        session.runtime.execution.model_provider_id.as_deref(),
        Some("scripted")
    );
    assert_eq!(
        session.runtime.execution.model_id.as_deref(),
        Some("scripted-model")
    );
}

#[tokio::test]
async fn switch_session_agent_pushes_and_restores_runtime_profile() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let session = service
        .create_session(SessionCreateRequest {
            title: "agent switch".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let switched = service
        .switch_session_agent(session.id, Some("planner".to_string()), true)
        .await
        .expect("switch to planner");
    assert_eq!(switched.previous_agent, None);
    assert_eq!(switched.current_agent.as_deref(), Some("planner"));
    assert_eq!(switched.stack_depth, 1);

    let loaded = service
        .get_session(session.id)
        .await
        .expect("load switched session");
    assert_eq!(
        loaded.runtime.execution.agent_profile.as_deref(),
        Some("planner")
    );
    assert_eq!(loaded.runtime.execution.agent_stack, vec![None]);
    assert!(
        loaded
            .runtime
            .execution
            .system_prompt_override
            .as_deref()
            .is_some_and(|prompt| prompt.contains("planning agent"))
    );

    let restored = service
        .restore_session_agent(session.id)
        .await
        .expect("restore previous agent");
    assert!(restored.restored);
    assert_eq!(restored.previous_agent.as_deref(), Some("planner"));
    assert_eq!(restored.current_agent, None);
    assert_eq!(restored.stack_depth, 0);

    let loaded = service
        .get_session(session.id)
        .await
        .expect("load restored session");
    assert_eq!(loaded.runtime.execution.agent_profile, None);
    assert!(loaded.runtime.execution.agent_stack.is_empty());
    assert_eq!(loaded.runtime.execution.system_prompt_override, None);
    assert!(loaded.runtime.allowed_tools().is_empty());
}

#[tokio::test]
async fn submit_user_turn_rejects_subagent_only_root_profile() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
        agents_dir.join("delegate.md"),
        "---\ndescription: delegate\nmode: subagent\n---\nYou only run as a delegated subagent.",
    )
    .expect("write delegate profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "root agent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let error = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: SessionRunOptions {
                agent_profile: Some("delegate".to_string()),
                ..run_options()
            },
            parts: vec![PartContent::text("Handle this at the root.")],
        })
        .await
        .expect_err("subagent-only profile should be rejected for root sessions");

    match error {
        AppError::Config(message) => {
            assert!(message.contains("delegate"));
            assert!(message.contains("root sessions"));
        }
        other => panic!("expected config error, got {other:?}"),
    }
}

#[tokio::test]
async fn spawn_subtask_rejects_primary_only_profile() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
        agents_dir.join("lead.md"),
        "---\ndescription: lead\nmode: primary\n---\nYou only run as a root agent.",
    )
    .expect("write primary-only profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let parent = service
        .create_session(SessionCreateRequest {
            title: "parent".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create parent session");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: parent.id,
            options: run_options(),
            parts: vec![PartContent::text("parent context")],
        })
        .await
        .expect("seed parent turn");

    let error = service
        .spawn_subtask(SessionSubtaskRequest {
            parent_session_id: parent.id,
            description: "delegate".to_string(),
            prompt: "Handle this as a subtask.".to_string(),
            subagent_type: TaskSubagentType::Explore,
            profile_name: Some("lead".to_string()),
            task_id: Some("lead-1".to_string()),
            command: None,
            requested_model: None,
        })
        .await
        .expect_err("primary-only profile should be rejected for subtask sessions");

    match error {
        AppError::Config(message) => {
            assert!(message.contains("lead"));
            assert!(message.contains("subtask sessions"));
        }
        other => panic!("expected config error, got {other:?}"),
    }
}

#[tokio::test]
async fn submit_user_turn_applies_agent_run_defaults() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
            agents_dir.join("focused.md"),
            "---\ndescription: focused\ntemperature: 0.33\nmax_output_tokens: 77\nsteps: 2\n---\nYou are focused.",
        )
        .expect("write focused profile");
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "focused root".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: SessionRunOptions {
                model: recording_model_ref(),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
                agent_profile: Some("focused".to_string()),
                max_turn_loops: None,
            },
            parts: vec![PartContent::text("Answer briefly.")],
        })
        .await
        .expect("submit turn");

    assert_eq!(
        session.runtime.execution.agent_run,
        crate::agent::AgentRunConfig {
            temperature: Some(crate::agent::AgentTemperature(0.33)),
            max_output_tokens: Some(77),
            steps: Some(2),
        }
    );

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    assert_eq!(recorded.len(), 1);
    assert_eq!(recorded[0].temperature, Some(0.33));
    assert_eq!(recorded[0].max_output_tokens, Some(77));
}

#[tokio::test]
async fn submit_user_turn_uses_agent_step_budget_for_turn_loop() {
    let workspace = TempWorkspace::new();
    let agents_dir = workspace.root.join(".agena").join("agents");
    fs::create_dir_all(&agents_dir).expect("create agents dir");
    fs::write(
        agents_dir.join("single_step.md"),
        "---\ndescription: single step\nsteps: 1\n---\nYou only get one loop.",
    )
    .expect("write single-step profile");
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "single step".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let error = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: SessionRunOptions {
                agent_profile: Some("single_step".to_string()),
                ..run_options()
            },
            parts: vec![PartContent::text("patch")],
        })
        .await
        .expect_err("single-step profile should exhaust the loop budget on tool call turns");

    match error {
        AppError::Internal(message) => {
            assert!(message.contains("max turn loop budget"));
        }
        other => panic!("expected loop-budget error, got {other:?}"),
    }
}

#[tokio::test]
async fn apply_tool_success_updates_worktree_root() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let mut session = service
        .create_session(SessionCreateRequest {
            title: "worktree".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let entered: ToolInvocationExecution = ToolPayloadExecution::new(
        ToolPayloadOutput::EnterWorktree {
            path: "/tmp/worktree".to_string(),
            branch: "agena/demo".to_string(),
        },
        crate::entry::ToolExecutionView::simple("enter_worktree", "entered"),
    )
    .into();
    let entered_invocation = ToolInvocation::new("enter_worktree", Default::default());
    service.apply_tool_success_execution_context(&mut session, &entered_invocation, &entered);
    assert_eq!(
        session
            .runtime
            .effective_workspace_root()
            .map(|path| path.to_string_lossy().to_string()),
        Some("/tmp/worktree".to_string())
    );

    let exited: ToolInvocationExecution = ToolPayloadExecution::new(
        ToolPayloadOutput::ExitWorktree {
            action: "keep".to_string(),
            path: "/tmp/worktree".to_string(),
        },
        crate::entry::ToolExecutionView::simple("exit_worktree", "exited"),
    )
    .into();
    let exited_invocation = ToolInvocation::new("exit_worktree", Default::default());
    service.apply_tool_success_execution_context(&mut session, &exited_invocation, &exited);
    assert!(session.runtime.effective_workspace_root().is_none());
}

#[tokio::test]
async fn continue_session_prefers_execution_context_model_override() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let mut session = service
        .create_session(SessionCreateRequest {
            title: "override".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    session.runtime.set_model_override(
        Some("scripted".to_string()),
        None,
        Some("claude-sonnet-4-6".to_string()),
    );

    let options = service
        .apply_execution_context_to_run_options(&session, run_options())
        .expect("apply execution context");
    assert_eq!(options.model.provider_id.as_ref(), "scripted");
    assert_eq!(options.model.model_id.as_ref(), "claude-sonnet-4-6");
}

#[tokio::test]
async fn apply_execution_context_to_run_options_uses_model_default_temperature_last() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)).with_metadata(
            crate::provider::ModelMetadata::default().with_default_temperature("0.55"),
        ),
    )
    .await;

    let session = service
        .create_session(SessionCreateRequest {
            title: "default temperature".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let options = service
        .apply_execution_context_to_run_options(
            &session,
            SessionRunOptions {
                model: recording_model_ref(),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
                agent_profile: None,
                max_turn_loops: None,
            },
        )
        .expect("apply execution context");
    assert_eq!(options.temperature, Some(0.55));
}

#[tokio::test]
async fn apply_execution_context_to_run_options_prefers_agent_temperature_over_model_default() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let manager = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)).with_metadata(
            crate::provider::ModelMetadata::default().with_default_temperature("0.55"),
        ),
    )
    .await;

    let created = manager
        .create_session(SessionCreateRequest {
            title: "agent temperature".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let state = manager.execution_state();
    let mut seeded = manager
        .get_session(created.id)
        .await
        .expect("reload session");
    seeded.runtime.execution.agent_run = crate::agent::AgentRunConfig {
        temperature: Some(crate::agent::AgentTemperature(0.2)),
        max_output_tokens: None,
        steps: None,
    };
    let _ = manager
        .persist_session_changes(seeded, Vec::new(), Vec::new(), None, state)
        .await
        .expect("persist session");

    let reloaded = manager
        .get_session(created.id)
        .await
        .expect("reload session after persist");
    let options = manager
        .apply_execution_context_to_run_options(
            &reloaded,
            SessionRunOptions {
                model: recording_model_ref(),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
                agent_profile: None,
                max_turn_loops: None,
            },
        )
        .expect("apply execution context");
    assert_eq!(options.temperature, Some(0.2));
}

#[tokio::test]
async fn fork_session_copies_event_prefix_without_mutating_source() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let source = service
        .create_session(SessionCreateRequest {
            title: "source".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create source session");
    service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("submit first turn");
    let first_turn_last_message_id = service
        .get_session(source.id)
        .await
        .expect("reload source")
        .messages
        .last()
        .expect("first turn produced at least one message")
        .id;
    service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(),
            parts: vec![PartContent::text("second")],
        })
        .await
        .expect("submit second turn");

    let forked = service
        .fork_session(SessionForkRequest {
            session_id: source.id,
            at_message_id: Some(first_turn_last_message_id),
            title: Some("forked".to_string()),
            expected_version: None,
        })
        .await
        .expect("fork session");
    let reloaded_source = service
        .get_session(source.id)
        .await
        .expect("reload source session");

    assert_eq!(forked.parent_id, Some(source.id));
    assert_eq!(forked.title, "forked");
    assert!(
        forked
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "first" })
    );
    assert!(
        !forked
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "second" })
    );
    assert!(
        reloaded_source
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "second" })
    );
}

#[tokio::test]
async fn fork_session_allows_empty_source() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;
    let source = service
        .create_session(SessionCreateRequest {
            title: "source".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create source session");

    let forked = service
        .fork_session(SessionForkRequest {
            session_id: source.id,
            at_message_id: None,
            title: Some("empty fork".to_string()),
            expected_version: None,
        })
        .await
        .expect("fork empty session");

    assert_eq!(forked.parent_id, Some(source.id));
    assert_eq!(forked.title, "empty fork");
    assert!(forked.messages.is_empty());
}

#[tokio::test]
async fn rewind_session_forks_without_mutating_source() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let source = service
        .create_session(SessionCreateRequest {
            title: "source".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create source session");
    service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("submit first turn");
    let first_user_message_id = service
        .get_session(source.id)
        .await
        .expect("reload source")
        .messages
        .iter()
        .find(|message| message.role == Role::User && message.as_text_lossy() == "first")
        .expect("first user message")
        .id;
    service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: source.id,
            options: run_options(),
            parts: vec![PartContent::text("second")],
        })
        .await
        .expect("submit second turn");

    let rewound = service
        .rewind_session(SessionRewindRequest {
            session_id: source.id,
            message_id: first_user_message_id,
            expected_version: None,
        })
        .await
        .expect("rewind should create fork");
    let reloaded_source = service
        .get_session(source.id)
        .await
        .expect("reload source session");

    assert_ne!(rewound.id, source.id);
    assert_eq!(rewound.parent_id, Some(source.id));
    assert!(
        rewound
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "first" })
    );
    assert!(
        !rewound
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "second" })
    );
    assert!(
        reloaded_source
            .messages
            .iter()
            .any(|message| { message.role == Role::User && message.as_text_lossy() == "second" })
    );
}

#[test]
fn cache_skips_entries_larger_than_byte_budget() {
    let state = cache_state(1, "x".repeat(256));
    let mut cache = SessionCache::default();
    let max_bytes = state.approx_bytes().saturating_sub(1).max(1);
    let cache_policy = SessionCachePolicy {
        max_sessions: 8,
        ttl: Duration::from_secs(60),
        max_bytes,
    };

    cache.insert(state.clone(), cache_policy);

    assert!(cache.get(state.id, cache_policy).is_none());
    assert_eq!(cache.total_bytes(), 0);
}

#[test]
fn cache_evicts_lru_entries_when_byte_budget_is_exceeded() {
    let first = cache_state(1, "alpha");
    let second = cache_state(2, "beta beta beta");
    let mut cache = SessionCache::default();
    let max_bytes = first
        .approx_bytes()
        .saturating_add(second.approx_bytes())
        .saturating_sub(1);
    let cache_policy = SessionCachePolicy {
        max_sessions: 8,
        ttl: Duration::from_secs(60),
        max_bytes,
    };

    cache.insert(first.clone(), cache_policy);
    cache.insert(second.clone(), cache_policy);

    assert!(cache.get(first.id, cache_policy).is_none());
    assert!(cache.get(second.id, cache_policy).is_some());
    assert!(cache.total_bytes() <= max_bytes);
}

#[tokio::test]
async fn follow_up_requests_reuse_prompt_cache_key_and_send_full_prefix() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "recording".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("second")],
        })
        .await
        .expect("submit second turn");

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    let expected_cache_key = prompt_window::prompt_cache_key_for_session(&created);

    assert_eq!(recorded.len(), 2);
    assert_eq!(
        recorded[0].prompt_cache_key.as_deref(),
        Some(expected_cache_key.as_str())
    );
    assert_eq!(
        recorded[1].prompt_cache_key.as_deref(),
        Some(expected_cache_key.as_str())
    );
    assert_eq!(recorded[0].previous_response_id, None);
    assert_eq!(recorded[1].previous_response_id, None);
    assert_eq!(recorded[1].system.as_deref(), Some("system"));
    assert_eq!(recorded[1].messages.len(), 3);
    assert_eq!(recorded[1].messages[0].as_text_lossy(), "first");
    assert_eq!(recorded[1].messages[1].as_text_lossy(), "recorded");
    assert_eq!(recorded[1].messages[2].as_text_lossy(), "second");
}

#[tokio::test]
async fn follow_up_requests_send_full_prefix_when_shape_appears_after_first_response() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()).with_dynamic_prompt_cache_shape(
            crate::provider::PromptCacheShape::new("recording")
                .with_string("runtime_route", "route-a"),
        ),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "dynamic shape".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("first")],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("second")],
        })
        .await
        .expect("submit second turn");

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();

    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].previous_response_id, None);
    assert_eq!(recorded[1].previous_response_id, None);
    assert_eq!(recorded[1].system.as_deref(), Some("system"));
    assert_eq!(recorded[1].messages.len(), 3);
    assert_eq!(recorded[1].messages[0].as_text_lossy(), "first");
    assert_eq!(recorded[1].messages[1].as_text_lossy(), "recorded");
    assert_eq!(recorded[1].messages[2].as_text_lossy(), "second");
}

#[tokio::test]
async fn compact_session_installs_summary_projection_and_restores_agent_context() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "compact".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let mut first_options = recording_run_options();
    first_options.agent_profile = Some("explore".to_string());
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: first_options,
            parts: vec![PartContent::text("old topic alpha")],
        })
        .await
        .expect("submit first turn");
    for text in ["recent topic beta", "latest topic gamma"] {
        let _ = service
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options: recording_run_options(),
                parts: vec![PartContent::text(text)],
            })
            .await
            .expect("submit seeded turn");
    }

    let compacted = service
        .compact_session(SessionCompactRequest {
            session_id: created.id,
            options: recording_run_options(),
        })
        .await
        .expect("compact session");
    assert_eq!(
        compacted.runtime.execution.agent_profile.as_deref(),
        Some("explore")
    );
    assert_ne!(
        compacted.runtime.execution.agent_profile.as_deref(),
        Some("compaction")
    );
    let compaction = compacted
        .runtime
        .prompt_window
        .compaction
        .as_ref()
        .expect("compaction runtime should be installed");
    assert_eq!(compaction.summary, "recorded");
    assert_eq!(compaction.strategy, PromptCompactionStrategy::LocalAgent);
    assert!(compacted.runtime.provider_anchors.is_empty());
    assert!(compacted.runtime.prompt_tokens.is_empty());

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("after compact")],
        })
        .await
        .expect("submit after compact");

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    let after_compact = recorded.last().expect("recorded follow-up request");
    let prompt_text = after_compact
        .messages
        .iter()
        .map(Message::as_text_lossy)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(prompt_text.contains("Conversation summary before the current active context"));
    assert!(prompt_text.contains("recorded"));
    assert!(prompt_text.contains("recent topic beta"));
    assert!(prompt_text.contains("latest topic gamma"));
    assert!(prompt_text.contains("after compact"));
    assert!(!prompt_text.contains("old topic alpha"));
    assert!(!prompt_text.contains("Summarize the conversation so far"));
}

#[tokio::test]
async fn compact_session_falls_back_to_local_agent_when_remote_compact_fails() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests).with_remote_compact_error("remote compact failed"),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "compact fallback".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("seed history")],
        })
        .await
        .expect("submit seeded turn");

    let compacted = service
        .compact_session(SessionCompactRequest {
            session_id: created.id,
            options: recording_run_options(),
        })
        .await
        .expect("compact should fall back to local agent");
    let compaction = compacted
        .runtime
        .prompt_window
        .compaction
        .as_ref()
        .expect("fallback compaction runtime should be installed");
    assert_eq!(compaction.summary, "recorded");
    assert_eq!(compaction.strategy, PromptCompactionStrategy::LocalAgent);
    assert_ne!(
        compacted.runtime.execution.agent_profile.as_deref(),
        Some("compaction")
    );
}

#[tokio::test]
async fn auto_compaction_runs_before_turn_when_session_context_is_full() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig {
            auto_compaction: SessionAutoCompactionConfig {
                enabled: true,
                reserved_tokens: Some(256),
            },
            ..SessionManagerConfig::default()
        },
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone())
            .with_metadata(
                crate::provider::ModelMetadata::default()
                    .with_context_window_tokens(4_096)
                    .with_max_input_tokens(1_200)
                    .with_max_output_tokens(256),
            )
            .with_usage(CompletionUsage {
                input_tokens: 800,
                output_tokens: 32,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            }),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "auto compact".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let first = "first-turn-alpha";
    let second = format!("second-turn-beta {}", "b".repeat(200));
    let third = format!("third-turn-gamma {}", "c".repeat(6_000));

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text(first)],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text(second.clone())],
        })
        .await
        .expect("submit second turn");
    let mut synthetic = service
        .get_session(created.id)
        .await
        .expect("load session before synthetic third turn");
    let ids = service
        .store
        .reserve_message_ids(1)
        .await
        .expect("reserve synthetic message ids");
    synthetic.messages.push(build_message(
        ids,
        Role::User,
        MessageStatus::Completed,
        vec![PartContent::text(third.clone())],
        MessageMetadata {
            source: MessageSource::User,
            parent_message_id: synthetic
                .last_conversation_message()
                .map(|message| message.id),
            generated_by_call_id: None,
            model_provider_id: "recording".to_string(),
            model_adapter_id: None,
            model_id: "recording-model".to_string(),
            model_thinking_mode: None,
            model_speed_mode: None,
            model_verbosity: None,
            model_parallel_tool_calls: None,
            provider_metadata: None,
            tags: Vec::new(),
        },
    ));
    let synthetic_usage = service
        .session_usage(&synthetic)
        .expect("session usage should compute");
    let projected_tokens = synthetic_usage
        .projected_tokens
        .unwrap_or(synthetic_usage.current_tokens);
    assert!(
        synthetic_usage.limit_basis == Some(SessionUsageLimitBasis::ContextWindow)
            && projected_tokens >= synthetic_usage.limit_tokens.unwrap_or_default(),
        "synthetic third turn should exceed usable context: {:?}",
        synthetic_usage
    );
    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text(third.clone())],
        })
        .await
        .expect("submit third turn");

    let compaction = session
        .runtime
        .prompt_window
        .compaction
        .as_ref()
        .expect("auto compaction should install runtime");
    assert_eq!(compaction.summary, "recorded");
    assert_eq!(compaction.strategy, PromptCompactionStrategy::LocalAgent);

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    assert_eq!(recorded.len(), 4);
    let final_prompt = recorded
        .last()
        .expect("final request should exist")
        .messages
        .iter()
        .map(Message::as_text_lossy)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(final_prompt.contains("Conversation summary before the current active context"));
    assert!(final_prompt.contains("recorded"));
    assert!(final_prompt.contains("second-turn-beta"));
    assert!(final_prompt.contains("third-turn-gamma"));
    assert!(!final_prompt.contains(first));
}

#[tokio::test]
async fn auto_compaction_can_be_disabled_per_session_config() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig {
            auto_compaction: SessionAutoCompactionConfig {
                enabled: false,
                reserved_tokens: Some(256),
            },
            ..SessionManagerConfig::default()
        },
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone())
            .with_metadata(
                crate::provider::ModelMetadata::default()
                    .with_context_window_tokens(4_096)
                    .with_max_input_tokens(1_200)
                    .with_max_output_tokens(256),
            )
            .with_usage(CompletionUsage {
                input_tokens: 800,
                output_tokens: 32,
                reasoning_tokens: 0,
                cache_write_tokens: 0,
                cache_read_tokens: 0,
                total_cost: 0.0,
            }),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "auto compact off".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("first-turn-alpha")],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text(format!(
                "second-turn-beta {}",
                "b".repeat(200)
            ))],
        })
        .await
        .expect("submit second turn");
    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: recording_run_options(),
            parts: vec![PartContent::text(format!(
                "third-turn-gamma {}",
                "c".repeat(6_000)
            ))],
        })
        .await
        .expect("submit third turn without auto compaction");

    assert!(session.runtime.prompt_window.compaction.is_none());

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    assert_eq!(recorded.len(), 3);
    let final_prompt = recorded
        .last()
        .expect("final request should exist")
        .messages
        .iter()
        .map(Message::as_text_lossy)
        .collect::<Vec<_>>()
        .join("\n");
    assert!(!final_prompt.contains("Conversation summary before the current active context"));
    assert!(final_prompt.contains("first-turn-alpha"));
    assert!(final_prompt.contains("second-turn-beta"));
    assert!(final_prompt.contains("third-turn-gamma"));
}

#[tokio::test]
async fn persisted_runtime_anchor_survives_cache_eviction() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig {
            cache_max_sessions: 1,
            cache_ttl: Duration::from_secs(60),
            cache_max_bytes: usize::MAX,
            max_turn_loops: 16,
            doom_loop: crate::session::DoomLoopPolicy::default(),
            default_agent: None,
            permission: crate::agent::PermissionConfig::default(),
            auto_compaction: SessionAutoCompactionConfig::default(),
        },
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()),
    )
    .await;

    let first = service
        .create_session(SessionCreateRequest {
            title: "first".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create first session");
    let second = service
        .create_session(SessionCreateRequest {
            title: "second".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create second session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: first.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("hello one")],
        })
        .await
        .expect("submit first turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: second.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("hello two")],
        })
        .await
        .expect("submit second session turn");
    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: first.id,
            options: recording_run_options(),
            parts: vec![PartContent::text("hello again")],
        })
        .await
        .expect("submit reloaded turn");

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed")
        .clone();
    let expected_cache_key = prompt_window::prompt_cache_key_for_session(&first);

    assert_eq!(recorded.len(), 3);
    assert_eq!(
        recorded[2].prompt_cache_key.as_deref(),
        Some(expected_cache_key.as_str())
    );
    assert_eq!(recorded[2].previous_response_id, None);
    assert_eq!(recorded[2].system.as_deref(), Some("system"));
    assert_eq!(recorded[2].messages.len(), 3);
    assert_eq!(recorded[2].messages[0].as_text_lossy(), "hello one");
    assert_eq!(recorded[2].messages[1].as_text_lossy(), "recorded");
    assert_eq!(recorded[2].messages[2].as_text_lossy(), "hello again");
}

/// Sub-task C: verify that `submit_user_turn` writes the new append-only
/// `UserMessageAppended` event (wrapped in `TurnStarted` / `TurnCompleted`).
/// After the processor turn completes there must also be
/// `AssistantMessageCompleted` from the TurnBuffer commit. The test
/// enforces append-only invariants on the event log: events for the
/// user-input turn are written exactly once and never rewritten.
#[tokio::test]
async fn submit_user_turn_emits_append_only_user_message_event() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "append-only-user".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("hello there")],
        })
        .await
        .expect("submit turn");

    let history = service
        .list_session_events(created.id)
        .await
        .expect("history should load");

    // Locate the user-message turn boundary (TurnStarted with no model
    // request_digest immediately followed by UserMessageAppended +
    // TurnCompleted) and verify the user payload is present and correctly
    // wired to the turn id.
    let mut user_payload: Option<&UserMessageAppended> = None;
    let mut user_turn_id: Option<HistoryTurnId> = None;
    for record in &history {
        if let EventKind::UserMessageAppended(payload) = &record.kind {
            user_payload = Some(payload);
            user_turn_id = Some(payload.turn_id);
            break;
        }
    }
    let user_payload = user_payload.expect("user_message_appended event must exist");
    let user_turn_id = user_turn_id.expect("user message turn id must be set");
    assert_eq!(user_payload.content.blocks.len(), 1);

    // Both the wrapping TurnStarted and TurnCompleted for this turn id
    // must be present in the event log.
    let turn_starts = history
            .iter()
            .filter(|record| {
                matches!(&record.kind, EventKind::TurnStarted(payload) if payload.turn_id == user_turn_id)
            })
            .count();
    let turn_completes = history
            .iter()
            .filter(|record| {
                matches!(&record.kind, EventKind::TurnCompleted(payload) if payload.turn_id == user_turn_id)
            })
            .count();
    assert_eq!(turn_starts, 1, "user turn started exactly once");
    assert_eq!(turn_completes, 1, "user turn completed exactly once");

    // Append-only invariant: each event row has a unique seq.
    let seqs: Vec<i64> = history.iter().map(|r| r.meta.seq_global).collect();
    let mut sorted = seqs.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(seqs.len(), sorted.len(), "no duplicate seq values");
}

// ─── Phase 8: append-only integration tests ─────────────────────────────

#[tokio::test]
async fn assistant_projection_rebuilds_missing_parts_from_history() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "assistant-repair".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let session = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("repair me")],
        })
        .await
        .expect("submit turn");

    let assistant = session
        .messages
        .iter()
        .find(|message| message.role == Role::Assistant)
        .cloned()
        .expect("assistant message should exist");
    assert_eq!(assistant.as_text_lossy(), "echo:repair me");

    let history = service
        .list_session_events(created.id)
        .await
        .expect("history should load");
    let completed = history
        .iter()
        .find_map(|record| match &record.kind {
            EventKind::AssistantMessageCompleted(payload) => Some(payload),
            _ => None,
        })
        .expect("assistant completion history should exist");
    assert_eq!(completed.parts.len(), 1);
    assert_eq!(completed.parts[0].text(), Some("echo:repair me"));

    activity_part::Entity::delete_many()
        .filter(activity_part::Column::MessageId.eq(assistant.id))
        .exec(service.store.db())
        .await
        .expect("delete projected assistant parts");

    let repaired = service
        .list_projected_messages(created.id, true)
        .await
        .expect("projected messages should reload");
    let repaired_assistant = repaired
        .iter()
        .find(|message| message.id == assistant.id)
        .expect("repaired assistant message should exist");
    assert_eq!(repaired_assistant.as_text_lossy(), "echo:repair me");
    assert_eq!(repaired_assistant.parts.len(), 1);

    let repaired_part_count = activity_part::Entity::find()
        .filter(activity_part::Column::MessageId.eq(assistant.id))
        .count(service.store.db())
        .await
        .expect("count repaired assistant parts");
    assert_eq!(repaired_part_count, 1);
}

#[tokio::test]
async fn load_session_rebuilds_projection_when_history_is_published_directly() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "direct-history".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let turn_id = HistoryTurnId::new();
    let message_id = 90_001;
    let created_at = chrono::Utc::now();
    let mut part = MessagePart::with_content(
        90_101,
        message_id,
        created_at,
        ExecutionStatus::Completed,
        PartContent::text("published directly"),
    );
    part.part_index = 0;

    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::TurnStarted(TurnStarted {
                turn_id,
                model_id: "direct-model".into(),
                provider_id: "direct-provider".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("publish direct turn start");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id: HistoryMessageId(message_id),
                turn_id,
                created_at,
                content: TranscriptContent::from_text("published directly"),
                parts: vec![part.clone()],
                metadata: MessageMetadata::default(),
            }),
        )
        .await
        .expect("publish direct user message");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::TurnCompleted(TurnCompleted {
                turn_id,
                finish_reason: FinishReason::Stop,
            }),
        )
        .await
        .expect("publish direct turn completion");

    let projected_before = activity_message::Entity::find()
        .filter(activity_message::Column::SessionId.eq(created.id))
        .count(service.store.db())
        .await
        .expect("count projected messages before rebuild");
    assert_eq!(
        projected_before, 0,
        "direct event-store publish should not bypass the explicit projection catch-up path"
    );

    service.store.prune_cache(SessionCachePolicy {
        max_sessions: 0,
        ttl: std::time::Duration::from_secs(0),
        max_bytes: 0,
    });
    let cache_policy = SessionCachePolicy {
        max_sessions: 8,
        ttl: std::time::Duration::from_secs(60),
        max_bytes: usize::MAX,
    };

    let reloaded = service
        .store
        .load_session(created.id, cache_policy)
        .await
        .expect("session should rebuild stale projection");

    assert!(
        reloaded
            .messages
            .iter()
            .any(|message| message.id == message_id
                && message.as_text_lossy() == "published directly"),
        "load_session should catch the projection up to durable history"
    );

    let projected_after = activity_message::Entity::find()
        .filter(activity_message::Column::SessionId.eq(created.id))
        .count(service.store.db())
        .await
        .expect("count projected messages after rebuild");
    assert_eq!(projected_after, 1);
}

#[tokio::test]
async fn load_session_rebuilds_tool_completion_parts_from_history() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "direct-tool-history".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let turn_id = HistoryTurnId::new();
    let message_id = 91_001;
    let created_at = chrono::Utc::now();
    let call_id = ToolCallId::new("call_attachment");

    let mut text_part = MessagePart::with_content(
        91_101,
        message_id,
        created_at,
        ExecutionStatus::Completed,
        PartContent::text("running"),
    );
    text_part.part_index = 0;

    let mut completed_part = MessagePart::with_content(
        91_102,
        message_id,
        created_at,
        ExecutionStatus::Completed,
        PartContent::Operation(crate::message::OperationPart::completed(
            0,
            crate::message::ToolInvocation {
                name: "read_file".to_string(),
                plugin_name: None,
                input: crate::message::StructuredObject::try_from(
                    serde_json::json!({ "path": "x" }),
                )
                .expect("tool input"),
            },
            "ok".to_string(),
            Vec::new(),
            vec![crate::message::AttachmentItem {
                kind: crate::message::AttachmentKind::Image,
                mime: "image/png".to_string(),
                source: AttachmentSource::DataUrl {
                    url: "data:image/png;base64,AAAA".to_string(),
                },
                filename: Some("preview.png".to_string()),
                title: None,
                size_bytes: None,
                sha256: None,
                width: Some(1),
                height: Some(1),
                duration_ms: None,
                page_count: None,
            }],
            ToolOutput::default(),
            crate::message::TimeRange::default(),
        )),
    );
    completed_part.part_index = 1;
    completed_part.operation_id = Some(call_id.as_str().to_string());

    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::TurnStarted(TurnStarted {
                turn_id,
                model_id: "direct-model".into(),
                provider_id: "direct-provider".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("publish direct turn start");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::AssistantMessageCompleted(AssistantMessageCompleted {
                message_id: HistoryMessageId(message_id),
                turn_id,
                created_at,
                content: TranscriptContent::from_text("running"),
                parts: vec![text_part],
                usage: None,
                finish_reason: FinishReason::ToolCalls,
                metadata: MessageMetadata::default(),
            }),
        )
        .await
        .expect("publish direct assistant message");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::ToolCallIssued(ToolCallIssued {
                message_id: HistoryMessageId(message_id),
                turn_id,
                call_id: call_id.clone(),
                name: "read_file".into(),
                arguments: serde_json::json!({ "path": "x" }),
                created_at,
            }),
        )
        .await
        .expect("publish direct tool call");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::ToolCallCompleted(ToolCallCompleted {
                message_id: HistoryMessageId(message_id),
                call_id: call_id.clone(),
                turn_id,
                tool_name: "read_file".into(),
                part: Some(completed_part),
                output: crate::session::history::TranscriptToolOutput::Text { text: "ok".into() },
                completed_at: created_at,
            }),
        )
        .await
        .expect("publish direct tool completion");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::TurnCompleted(TurnCompleted {
                turn_id,
                finish_reason: FinishReason::ToolCalls,
            }),
        )
        .await
        .expect("publish direct turn completion");

    service.store.prune_cache(SessionCachePolicy {
        max_sessions: 0,
        ttl: std::time::Duration::from_secs(0),
        max_bytes: 0,
    });
    let reloaded = service
        .store
        .load_session(
            created.id,
            SessionCachePolicy {
                max_sessions: 8,
                ttl: std::time::Duration::from_secs(60),
                max_bytes: usize::MAX,
            },
        )
        .await
        .expect("session should rebuild direct tool history");

    let assistant = reloaded
        .messages
        .iter()
        .find(|message| message.id == message_id)
        .expect("assistant message should be reloaded");
    let tool_part = assistant
        .parts
        .iter()
        .find(|part| part.operation_id.as_deref() == Some(call_id.as_str()))
        .expect("tool part should be reconstructed");
    let Some(PartContent::Operation(operation)) = tool_part.content.as_ref() else {
        panic!("expected reconstructed operation part");
    };
    assert_eq!(operation.output_text(), Some("ok"));
    assert_eq!(operation.attachments.len(), 1);
    assert_eq!(
        operation.attachments[0].filename.as_deref(),
        Some("preview.png")
    );
}

#[tokio::test]
async fn append_only_full_turn_writes_one_row_per_event_no_overwrites() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "append-only-turn".into(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("hi")],
        })
        .await
        .expect("submit turn");

    let history = service
        .list_session_events(created.id)
        .await
        .expect("history should load");

    // The legacy mutable-snapshot variant has been removed; nothing to
    // assert here beyond the seq invariant below.

    // Every seq is unique and monotonically increasing — the cardinal
    // invariant of an append-only log.
    let mut prev: Option<i64> = None;
    for record in &history {
        if let Some(p) = prev {
            assert!(
                record.meta.seq_global > p,
                "seq must be strictly increasing"
            );
        }
        prev = Some(record.meta.seq_global);
    }
}

#[test]
fn append_only_prefix_digest_stable_across_different_trailing_user_message() {
    std::thread::Builder::new()
        .name("append-only-prefix-digest".to_string())
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("test runtime");
            runtime.block_on(async {
                use crate::session::history::ProviderTranscriptBuilder;
                use crate::session::history::fold_history;

                let workspace = TempWorkspace::new();
                let service = build_manager(
                    &workspace.root,
                    PermissionPolicy::allow_all(),
                    SessionManagerConfig::default(),
                )
                .await;

                async fn run_prefix_then(service: &SessionManager, trailing: &str) -> blake3::Hash {
                    let created = service
                        .create_session(SessionCreateRequest {
                            title: "digest".into(),
                            parent_session_id: None,
                        })
                        .await
                        .expect("create session");
                    service
                        .submit_user_turn(SessionUserTurnRequest {
                            session_id: created.id,
                            options: run_options(),
                            parts: vec![PartContent::text("shared prefix")],
                        })
                        .await
                        .expect("first turn");
                    let records = service
                        .list_session_events(created.id)
                        .await
                        .expect("records");
                    // Trailing message is intentionally unused: compare only the closed prefix.
                    let prefix_records: Vec<_> = records.to_vec();
                    let _ = trailing;
                    let transcript =
                        fold_history::<ProviderTranscriptBuilder>(prefix_records.as_slice())
                            .expect("fold");
                    transcript.digest()
                }

                let a = run_prefix_then(&service, "follow-up A").await;
                let b = run_prefix_then(&service, "follow-up B").await;
                assert_eq!(
                    a, b,
                    "prefix digest must be stable across different trailing messages"
                );
            });
        })
        .expect("spawn test thread")
        .join()
        .expect("test thread should finish");
}

#[tokio::test]
async fn append_only_dangling_turn_started_gets_aborted_on_reload() {
    use crate::session::history::TurnAbortReason;

    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "dangling".into(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    // Inject a hanging TurnStarted event directly into the history table
    // (no matching TurnCompleted/TurnAborted) to simulate a process
    // restart mid-turn.
    let dangling_turn = HistoryTurnId::new();
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::TurnStarted(TurnStarted {
                turn_id: dangling_turn,
                model_id: "test-model".into(),
                provider_id: "test-provider".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("inject dangling TurnStarted");

    // Force the session out of cache so load_session takes the DB path
    // (and runs repair_hanging_turns).
    let cache_policy = SessionCachePolicy {
        max_sessions: 8,
        ttl: std::time::Duration::from_secs(60),
        max_bytes: usize::MAX,
    };
    service.store.prune_cache(SessionCachePolicy {
        max_sessions: 0,
        ttl: std::time::Duration::from_secs(0),
        max_bytes: 0,
    });

    // Now load the session — the store must repair the dangling turn by
    // appending a `TurnAborted{ProcessRestart}` marker.
    service
        .store
        .load_session(created.id, cache_policy)
        .await
        .expect("session should reload");

    let history = service
        .list_session_events(created.id)
        .await
        .expect("history");

    let aborted = history
        .iter()
        .find_map(|r| match &r.kind {
            EventKind::TurnAborted(payload) if payload.turn_id == dangling_turn => Some(payload),
            _ => None,
        })
        .expect("dangling turn must be repaired with a TurnAborted marker");
    assert_eq!(aborted.reason, TurnAbortReason::ProcessRestart);
}

#[tokio::test]
async fn restart_after_interrupted_turn_can_continue_session() {
    use crate::session::history::TurnAbortReason;

    struct RestartableProvider {
        stall: bool,
    }

    #[async_trait]
    impl ModelRuntime for RestartableProvider {
        fn id(&self) -> &str {
            "restartable"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("restartable-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new("restartable", "restartable-model")])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider(
                "restartable provider streams only".into(),
            ))
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            if self.stall {
                let stream = async_stream::stream! {
                    yield Ok(CompletionStreamEvent::TextDelta {
                        provider_id: ProviderId::new("restartable"),
                        model: ModelId::new("restartable-model"),
                        delta: "partial".to_string(),
                    });
                    std::future::pending::<()>().await;
                };
                return Ok(Box::pin(stream));
            }

            Ok(Box::pin(stream::iter(vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("restartable"),
                    model: ModelId::new("restartable-model"),
                    delta: "recovered reply".to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: ProviderId::new("restartable"),
                    model: ModelId::new("restartable-model"),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                }),
            ])))
        }
    }

    fn restartable_options() -> SessionRunOptions {
        SessionRunOptions {
            model: ModelRef::new("restartable", "restartable-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: Some(128),
            agent_profile: None,
            max_turn_loops: None,
        }
    }

    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "interrupted-resume.db").await;
    let first = Arc::new(
        build_manager_with_provider_on_db(
            &workspace.root,
            db.clone(),
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            RestartableProvider { stall: true },
        )
        .await,
    );
    let created = first
        .create_session(SessionCreateRequest {
            title: "interrupted-resume".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let session_id = created.id;

    let running = {
        let manager = Arc::clone(&first);
        tokio::spawn(async move {
            manager
                .submit_user_turn(SessionUserTurnRequest {
                    session_id,
                    options: restartable_options(),
                    parts: vec![PartContent::text("start then restart")],
                })
                .await
        })
    };

    for _ in 0..20 {
        let has_model_turn = first
            .list_session_events(session_id)
            .await
            .expect("history should load")
            .iter()
            .any(|record| {
                matches!(
                    &record.kind,
                    EventKind::TurnStarted(payload)
                        if payload.provider_id == "restartable"
                )
            });
        if has_model_turn {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    running.abort();
    assert!(
        running
            .await
            .expect_err("turn task should be aborted")
            .is_cancelled()
    );
    let interrupted_turn = HistoryTurnId::new();
    first
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(session_id),
            EventKind::TurnStarted(TurnStarted {
                turn_id: interrupted_turn,
                model_id: "restartable-model".into(),
                provider_id: "restartable".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("interrupted turn should be persisted");
    drop(first);

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RestartableProvider { stall: false },
    )
    .await;
    resume_event_sequence(&second).await;

    let recovered = second
        .continue_session(SessionContinueRequest {
            session_id,
            options: restartable_options(),
        })
        .await
        .expect("continue should recover after restart");
    let history = second
        .list_session_events(session_id)
        .await
        .expect("history should load");

    assert!(history.iter().any(|record| {
        matches!(
            &record.kind,
            EventKind::TurnAborted(payload)
                if payload.turn_id == interrupted_turn
                    && payload.reason == TurnAbortReason::ProcessRestart
        )
    }));
    assert!(recovered.messages.iter().any(|message| {
        message.role == Role::Assistant && message.as_text_lossy().contains("recovered reply")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_permission_survives_restart_and_reply_continues() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "permission-resume.db").await;
    let tool_policy = ToolPermissionPolicy::allow_all().with_tool_mode("todo", PermissionMode::Ask);
    let first = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        tool_policy.clone(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
    )
    .await;
    let created = first
        .create_session(SessionCreateRequest {
            title: "permission-resume".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let session_id = created.id;
    let blocked = first
        .submit_user_turn(SessionUserTurnRequest {
            session_id,
            options: run_options(),
            parts: vec![PartContent::text("permission todo")],
        })
        .await
        .expect("turn should block on permission");
    let request_id = pending_permission_request_id(&blocked);
    assert!(blocked.blocked());
    drop(first);

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        tool_policy,
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
    )
    .await;
    resume_event_sequence(&second).await;
    let reloaded = second
        .get_session(session_id)
        .await
        .expect("session should reload");
    assert!(
        reloaded.blocked(),
        "reloaded session was not blocked: messages={:?}, runtime={:?}",
        reloaded.messages,
        reloaded.runtime()
    );

    let completed = second
        .reply_permission(SessionPermissionReplyRequest {
            session_id,
            options: run_options(),
            reply: PermissionReply {
                request_id,
                kind: PermissionReplyKind::AllowOnce,
                reason: None,
                scope: None,
            },
            operator: Some("test".to_string()),
        })
        .await
        .expect("permission reply should continue session");

    assert!(!completed.blocked());
    assert!(
        completed.messages.iter().any(|message| {
            message.role == Role::Assistant
                && message.as_text_lossy().contains("permission todo done")
        }),
        "completed session missing final assistant reply: messages={:?}, runtime={:?}",
        completed.messages,
        completed.runtime()
    );
}

/// Phase 3 of the event-system refactor: every legacy `SessionEvent` and
/// `HistoryItem` produced by a turn must also surface on the unified
/// `EventBus` as the corresponding `EventKind`. This guards the cutover
/// while readers are migrated.
#[tokio::test]
async fn unified_bus_mirrors_legacy_events_during_a_turn() {
    use crate::event::{EventFilter, Scope, bus::SubscriptionItem};

    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let bus = service.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Global));

    let created = service
        .create_session(SessionCreateRequest {
            title: "mirror-test".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let _ = service
        .submit_user_turn(SessionUserTurnRequest {
            session_id: created.id,
            options: run_options(),
            parts: vec![PartContent::text("hello mirror")],
        })
        .await
        .expect("submit turn");

    // Drain the bus into a vector with a hard timeout so the test can't
    // hang if mirroring is broken.
    let mut received = Vec::new();
    let drain = async {
        while let Some(item) = subscription.recv().await {
            if let SubscriptionItem::Event(event) = item {
                received.push(event.kind.tag_str());
            }
            if received.len() >= 16 {
                break;
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), drain).await;

    assert!(
        received.contains(&"run_started"),
        "bus should carry RunStarted, got: {received:?}"
    );
    assert!(
        received.contains(&"user_message_appended"),
        "bus should carry UserMessageAppended, got: {received:?}"
    );
    assert!(
        received.contains(&"turn_started"),
        "bus should carry TurnStarted, got: {received:?}"
    );
}

#[tokio::test]
async fn goal_lifecycle_persists_and_publishes_updates() {
    use crate::event::{EventFilter, Scope, bus::SubscriptionItem};

    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "goal-lifecycle".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let mut subscription = service
        .event_bus()
        .subscribe(EventFilter::new(Scope::Global));

    let created_goal = service
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "ship goal system".to_string(),
        })
        .await
        .expect("create goal");
    assert_eq!(created_goal.objective, "ship goal system");
    assert_eq!(created_goal.status, GoalStatus::Active);

    let loaded_goal = service
        .get_goal(created.id)
        .await
        .expect("get goal")
        .expect("goal should exist");
    assert_eq!(loaded_goal.id, created_goal.id);
    assert_eq!(loaded_goal.status, GoalStatus::Active);

    let completed_goal = service
        .complete_goal(created.id)
        .await
        .expect("complete goal");
    assert_eq!(completed_goal.id, created_goal.id);
    assert_eq!(completed_goal.status, GoalStatus::Completed);
    assert!(completed_goal.completed_at.is_some());

    let cleared = service.clear_goal(created.id).await.expect("clear goal");
    assert!(cleared);
    assert!(
        service
            .get_goal(created.id)
            .await
            .expect("get cleared goal")
            .is_none()
    );

    let mut statuses = Vec::new();
    let drain = async {
        while let Some(item) = subscription.recv().await {
            if let SubscriptionItem::Event(event) = item
                && let EventKind::SessionGoalUpdated(payload) = &event.kind
                && payload.session_id == created.id
            {
                statuses.push(payload.status.clone());
                if statuses.len() == 3 {
                    break;
                }
            }
        }
    };
    let _ = tokio::time::timeout(std::time::Duration::from_millis(200), drain).await;

    assert_eq!(
        statuses.len(),
        3,
        "expected 3 goal events, got {statuses:?}"
    );
    assert_eq!(statuses[0].as_deref(), Some("active"));
    assert_eq!(statuses[1].as_deref(), Some("completed"));
    assert_eq!(statuses[2], None);
}

#[tokio::test]
async fn create_goal_persists_objective_updated_runtime_state() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "goal-runtime-state".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let goal = service
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "ship the feature".to_string(),
        })
        .await
        .expect("create goal");

    let session = service
        .get_session(created.id)
        .await
        .expect("load session after goal create");
    let pending = session
        .runtime
        .goal
        .pending_steering()
        .expect("goal runtime should queue objective-updated steering");
    assert_eq!(pending.goal_id, goal.id);
    assert_eq!(pending.kind, GoalSteeringKind::ObjectiveUpdated);
}

#[tokio::test]
async fn goal_turn_directive_only_allows_hidden_continuation_when_enabled() {
    let workspace = TempWorkspace::new();
    let service = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "goal-continuation-gate".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    service
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "keep going".to_string(),
        })
        .await
        .expect("create goal");

    let mut session = service
        .get_session(created.id)
        .await
        .expect("load session after goal create");
    session.runtime.goal.clear_pending_steering();

    assert!(
        service.goal_turn_directive(&session, false).is_none(),
        "ordinary user turns should not auto-continue indefinitely"
    );
    let continuation = service
        .goal_turn_directive(&session, true)
        .expect("continue_session should unlock one hidden continuation");
    assert_eq!(continuation.kind, GoalTurnDirectiveKind::Continuation);
    assert!(
        continuation
            .prompt
            .contains("Continue working toward the active runtime goal.")
    );
}

#[tokio::test]
async fn run_model_turn_uses_persisted_goal_context_message() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "goal-hidden-context".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    service
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "finish the migration".to_string(),
        })
        .await
        .expect("create goal");

    let options = recording_run_options();
    let state = service.execution_state();
    let mut session = service
        .get_session(created.id)
        .await
        .expect("load session before manual model turn");
    let ids = service
        .store
        .reserve_message_ids(1)
        .await
        .expect("reserve ids");
    let user_message = build_message(
        ids,
        Role::User,
        MessageStatus::Completed,
        vec![PartContent::text("start working")],
        MessageMetadata {
            source: MessageSource::User,
            parent_message_id: None,
            generated_by_call_id: None,
            model_provider_id: options.model.provider_id.to_string(),
            model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
            model_id: options.model.model_id.to_string(),
            model_thinking_mode: options.thinking_mode.clone(),
            model_speed_mode: options.speed_mode.clone(),
            model_verbosity: options.verbosity.clone(),
            model_parallel_tool_calls: options.request_override.parallel_tool_calls(),
            provider_metadata: None,
            tags: Vec::new(),
        },
    );
    session.messages.push(user_message.clone());
    session = service
        .persist_session_changes(session, vec![user_message], Vec::new(), None, state.clone())
        .await
        .expect("persist manual user message");

    let directive = service
        .goal_turn_directive(&session, false)
        .expect("objective-updated directive should be queued");
    assert_eq!(directive.kind, GoalTurnDirectiveKind::ObjectiveUpdated);
    session = service
        .append_goal_turn_directive_message(session, &directive, &options, state.clone())
        .await
        .expect("persist goal context message");
    let (control, _steer_rx) = service.turn_registry.register(created.id).await;
    let completed = service
        .run_model_turn(session, &options, state, control.clone())
        .await
        .expect("run one model turn");
    service
        .turn_registry
        .unregister_if_matches(created.id, &control)
        .await;

    let requests = requests
        .lock()
        .expect("recording provider request lock should succeed");
    let request = requests.last().expect("recorded request");
    let hidden_goal_message = request
        .messages
        .iter()
        .find(|message| message.as_text_lossy().contains("<goal_context>"))
        .expect("provider request should include persisted goal context");
    assert_eq!(hidden_goal_message.role, Role::User);
    assert!(
        hidden_goal_message
            .as_text_lossy()
            .contains("finish the migration")
    );
    assert!(
        completed
            .messages
            .iter()
            .any(|message| message.as_text_lossy().contains("<goal_context>")),
        "goal context should be persisted once so future prompts remain append-only"
    );
}

#[tokio::test]
async fn canceling_a_running_turn_pauses_an_active_goal() {
    let workspace = TempWorkspace::new();
    let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let manager = Arc::new(
        build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            InterruptibleProvider {
                call_count: Arc::clone(&call_count),
            },
        )
        .await,
    );

    let created = manager
        .create_session(SessionCreateRequest {
            title: "goal-pause-on-cancel".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    manager
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "keep working".to_string(),
        })
        .await
        .expect("create goal");

    cancel_running_turn(Arc::clone(&manager), created.id, call_count.as_ref()).await;

    let goal = manager
        .get_goal(created.id)
        .await
        .expect("load goal after cancel")
        .expect("goal should remain present");
    assert_eq!(goal.status, GoalStatus::Paused);
}

#[tokio::test]
async fn continue_session_resumes_a_paused_goal_to_active() {
    let workspace = TempWorkspace::new();
    let call_count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let manager = Arc::new(
        build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            InterruptibleProvider {
                call_count: Arc::clone(&call_count),
            },
        )
        .await,
    );

    let created = manager
        .create_session(SessionCreateRequest {
            title: "goal-resume-after-cancel".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    manager
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "keep working".to_string(),
        })
        .await
        .expect("create goal");

    cancel_running_turn(Arc::clone(&manager), created.id, call_count.as_ref()).await;
    let paused = manager
        .get_goal(created.id)
        .await
        .expect("load paused goal")
        .expect("goal should remain present");
    assert_eq!(paused.status, GoalStatus::Paused);

    let resumed = manager
        .continue_session(SessionContinueRequest {
            session_id: created.id,
            options: interruptible_options(),
        })
        .await
        .expect("continue paused session");
    let resumed_goal = resumed.goal.expect("goal should remain present");
    assert_eq!(resumed_goal.status, GoalStatus::Active);

    let persisted = manager
        .get_goal(created.id)
        .await
        .expect("reload resumed goal")
        .expect("goal should remain present");
    assert_eq!(persisted.status, GoalStatus::Active);
}

#[tokio::test]
async fn create_goal_on_idle_session_starts_one_persisted_goal_turn() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let manager = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;

    let created = manager
        .create_session(SessionCreateRequest {
            title: "goal-auto-start".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let state = manager.execution_state();
    let mut seeded = manager
        .get_session(created.id)
        .await
        .expect("reload session to seed execution overrides");
    seeded.runtime.execution.system_prompt_override = Some("system".to_string());
    seeded.runtime.execution.agent_run = crate::agent::AgentRunConfig {
        temperature: Some(crate::agent::AgentTemperature(0.2)),
        max_output_tokens: Some(256),
        steps: None,
    };
    let _ = manager
        .persist_session_changes(seeded, Vec::new(), Vec::new(), None, state)
        .await
        .expect("persist seeded execution overrides");

    manager
        .create_goal(SessionGoalCreateRequest {
            session_id: created.id,
            objective: "keep shipping".to_string(),
        })
        .await
        .expect("create goal");

    let started = async {
        for _ in 0..500 {
            if !requests
                .lock()
                .expect("recording provider request lock should succeed")
                .is_empty()
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        false
    }
    .await;
    assert!(started, "idle goal creation should start one hidden turn");

    let final_session = async {
        for _ in 0..500 {
            let session = manager
                .get_session(created.id)
                .await
                .expect("reload session during goal turn");
            if session.status() == SessionStatus::Idle
                && !manager.is_turn_active(created.id).await
                && session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant)
            {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("goal turn should settle within 10s");
    }
    .await;

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed");
    assert_eq!(
        recorded.len(),
        1,
        "goal creation should trigger exactly one goal turn"
    );
    let request = &recorded[0];
    assert_eq!(request.system.as_deref(), Some("system"));
    assert_eq!(request.temperature, Some(0.2));
    assert_eq!(request.max_output_tokens, Some(256));

    let goal_directive_message = request
        .messages
        .iter()
        .find(|message| message.role == Role::User)
        .expect("provider request should include persisted goal context");
    let goal_directive_text = goal_directive_message.as_text_lossy();
    assert!(
        goal_directive_text.contains("An active runtime goal has been set or updated."),
        "unexpected goal prompt: {}",
        goal_directive_text
    );
    assert!(
        goal_directive_text.contains("keep shipping"),
        "goal prompt should include the objective"
    );

    let persisted_goal_messages = final_session
        .messages
        .iter()
        .filter(|message| {
            message.role == Role::User
                && message.metadata.source == MessageSource::System
                && message
                    .as_text_lossy()
                    .contains("An active runtime goal has been set or updated.")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        persisted_goal_messages.len(),
        1,
        "goal context must be persisted so future provider prompts stay append-only"
    );
    assert!(
        persisted_goal_messages[0]
            .as_text_lossy()
            .contains("keep shipping")
    );
    assert_eq!(
        final_session
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .count(),
        1
    );
    assert_eq!(
        final_session
            .messages
            .iter()
            .filter(|message| message.role == Role::Assistant)
            .count(),
        1
    );
}

#[tokio::test]
async fn goal_runtime_external_goal_set_is_visible_to_next_continue_turn() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "goal-external-set.db").await;
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let first = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    let created = first
        .create_session(SessionCreateRequest {
            title: "external-set".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    resume_event_sequence(&second).await;
    persist_goal_without_auto_run(
        &second,
        created.id,
        "Externally supplied objective",
        Some(100),
    )
    .await;

    let _ = first
        .continue_session(SessionContinueRequest {
            session_id: created.id,
            options: recording_run_options(),
        })
        .await
        .expect("continue should observe externally created goal");

    let recorded = requests
        .lock()
        .expect("recording requests lock should succeed");
    let request = recorded
        .last()
        .expect("goal continuation request should be recorded");
    assert!(request.messages.iter().any(|message| {
        message
            .as_text_lossy()
            .contains("Externally supplied objective")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn goal_runtime_resumed_session_can_continue_active_goal_after_restart() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "goal-resume.db").await;
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let first = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    let created = first
        .create_session(SessionCreateRequest {
            title: "goal-resume".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let session_id = created.id;
    persist_goal_without_auto_run(
        &first,
        session_id,
        "Resume this goal after restart",
        Some(100),
    )
    .await;
    drop(first);

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    resume_event_sequence(&second).await;

    let _ = second
        .continue_session(SessionContinueRequest {
            session_id,
            options: recording_run_options(),
        })
        .await
        .expect("continue after restart should observe persisted goal");

    let recorded = requests
        .lock()
        .expect("recording requests lock should succeed");
    let request = recorded
        .last()
        .expect("goal continuation request should be recorded after restart");
    assert!(request.messages.iter().any(|message| {
        message
            .as_text_lossy()
            .contains("Resume this goal after restart")
    }));
}

#[tokio::test]
async fn goal_runtime_external_goal_clear_stops_next_continue_turn() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "goal-external-clear.db").await;
    let requests = Arc::new(Mutex::new(Vec::<CompletionRequest>::new()));
    let first = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    let created = first
        .create_session(SessionCreateRequest {
            title: "external-clear".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    persist_goal_without_auto_run(
        &first,
        created.id,
        "This should be cleared before continuation",
        Some(100),
    )
    .await;

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(Arc::clone(&requests)),
    )
    .await;
    resume_event_sequence(&second).await;
    assert!(
        second
            .clear_goal(created.id)
            .await
            .expect("external clear goal should succeed")
    );

    let _ = first
        .continue_session(SessionContinueRequest {
            session_id: created.id,
            options: recording_run_options(),
        })
        .await
        .expect("continue after external clear should stop cleanly");

    assert!(
        requests
            .lock()
            .expect("recording requests lock should succeed")
            .is_empty(),
        "cleared goal should prevent idle continuation"
    );
}

#[tokio::test]
async fn goal_runtime_external_goal_set_refreshes_cached_session() {
    let workspace = TempWorkspace::new();
    let db = open_temp_database(&workspace.root, "goal-external-cache-refresh.db").await;
    let first = build_manager_with_provider_on_db(
        &workspace.root,
        db.clone(),
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
    )
    .await;
    let created = first
        .create_session(SessionCreateRequest {
            title: "goal-cache-refresh".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create session");
    let cached = first
        .get_session(created.id)
        .await
        .expect("prime cached session");
    assert!(cached.goal.is_none(), "session should start without a goal");

    let second = build_manager_with_provider_on_db(
        &workspace.root,
        db,
        PermissionPolicy::allow_all(),
        ToolPermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        ScriptedProvider,
    )
    .await;
    resume_event_sequence(&second).await;
    persist_goal_without_auto_run(
        &second,
        created.id,
        "Refresh the cached session goal",
        Some(100),
    )
    .await;

    let refreshed = first
        .get_session(created.id)
        .await
        .expect("reload cached session");
    let goal = refreshed
        .goal
        .expect("cached session should refresh its goal");
    assert_eq!(goal.objective, "Refresh the cached session goal");
}

/// Cancel a turn while the provider stream is still pending. The
/// processor must observe the cancellation token and surface a
/// terminal error rather than running to completion.
///
/// Currently flaky under heavy parallel test load (the cancel can race
/// with the manager's stream consumer in non-deterministic ways).
/// Tracked separately; runs reliably in isolation.
#[ignore = "flaky under cargo test --workspace; passes with -p agena --lib"]
#[tokio::test]
async fn cancel_active_turn_aborts_a_running_turn() {
    struct SlowProvider;

    #[async_trait]
    impl ModelRuntime for SlowProvider {
        fn id(&self) -> &str {
            "slow"
        }
        fn default_model(&self) -> &ModelId {
            static M: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("slow-model"));
            &M
        }
        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new("slow", "slow-model")])
        }
        async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".into()))
        }
        async fn complete_stream(
            &self,
            _: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let s = async_stream::stream! {
                // First chunk arrives quickly so the turn is "live".
                yield Ok(CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("slow"),
                    model: ModelId::new("slow-model"),
                    delta: "thinking".to_string(),
                });
                // Then we stall — long enough that the test can issue
                // the cancel before the next chunk would have arrived.
                tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                yield Ok(CompletionStreamEvent::TextDelta {
                    provider_id: ProviderId::new("slow"),
                    model: ModelId::new("slow-model"),
                    delta: "should never arrive".to_string(),
                });
            };
            Ok(Box::pin(s))
        }
    }

    fn slow_options() -> SessionRunOptions {
        SessionRunOptions {
            model: ModelRef::new("slow", "slow-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: Some(64),
            agent_profile: None,
            max_turn_loops: None,
        }
    }

    let workspace = TempWorkspace::new();
    let manager = Arc::new(
        build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            SlowProvider,
        )
        .await,
    );

    let created = manager
        .create_session(SessionCreateRequest {
            title: "cancel-test".to_string(),
            parent_session_id: None,
        })
        .await
        .expect("create");
    let session_id = created.id;

    let mgr = Arc::clone(&manager);
    let submit = tokio::spawn(async move {
        mgr.submit_user_turn(SessionUserTurnRequest {
            session_id,
            options: slow_options(),
            parts: vec![PartContent::text("ping")],
        })
        .await
    });

    // Poll until the turn registers with TurnRegistry rather than
    // sleeping a fixed duration — the original 80 ms was flaky under
    // load. Use a generous budget (10s) so concurrent cargo test runs
    // don't race even on heavily loaded CI runners.
    let registered = async {
        for _ in 0..500 {
            if manager.is_turn_active(session_id).await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        false
    }
    .await;
    assert!(registered, "turn should register within 10s");
    // Try cancel; if it races with turn-registry teardown we retry once.
    for attempt in 0..3 {
        match manager.cancel_active_turn(session_id).await {
            Ok(()) => break,
            Err(_) if attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(err) => panic!("cancel should find active turn: {err}"),
        }
    }

    // The submit future should resolve quickly now (not after 60s).
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), submit)
        .await
        .expect("submit should complete after cancel")
        .expect("join");
    // The session run reports an error because the turn was aborted.
    assert!(
        result.is_err(),
        "expected turn to be reported as failed/cancelled"
    );
}

/// `cancel_active_turn` for a session with no in-flight turn returns
/// the corresponding error, never panics.
#[tokio::test]
async fn cancel_with_no_active_turn_is_a_clean_error() {
    let workspace = TempWorkspace::new();
    let manager = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;
    let err = manager.cancel_active_turn(1234).await.unwrap_err();
    assert!(matches!(err, AppError::Internal(_)));
}

/// `steer_input` against a session with no active turn surfaces the
/// "no in-flight turn" error so callers can fall back gracefully.
#[tokio::test]
async fn steer_with_no_active_turn_is_a_clean_error() {
    let workspace = TempWorkspace::new();
    let manager = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;
    let err = manager
        .steer_input(99, vec![PartContent::text("late")])
        .await
        .unwrap_err();
    assert!(matches!(err, AppError::Internal(_)));
}
