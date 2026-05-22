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

fn run_async_with_large_stack<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    let handle = std::thread::Builder::new()
        .name("agena-session-test".to_string())
        .stack_size(32 * 1024 * 1024)
        .spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("session test runtime");
            runtime.block_on(fut);
        })
        .expect("spawn large-stack session test thread");
    match handle.join() {
        Ok(()) => {}
        Err(err) => std::panic::resume_unwind(err),
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
                    .is_some_and(|loaded_tools| loaded_tools.iter().any(|name| name == "fs"))
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
                        "action": "write",
                        "items": TodoWriteToolInput {
                            items: vec![TodoItem {
                                content: "confirm permission recovery".to_string(),
                                status: TodoStatus::Completed,
                                priority: TodoPriority::Low,
                            }],
                        }.items,
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
                        "action": "search",
                        "query": ToolSearchToolInput {
                            query: "patch file".to_string(),
                            load: vec!["fs".to_string()],
                            limit: None,
                        }.query,
                        "load": ["fs"],
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
                        "action": "request_input",
                        "questions": AskUserToolInput {
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
                        }.questions,
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
                        name: Some("fs".to_string()),
                        arguments_delta: serde_json::json!({
                            "action": "apply_patch",
                            "patch": ApplyPatchToolInput {
                                patch: "*** Begin Patch\n*** Add File: result.txt\n+approved\n*** End Patch"
                                    .to_string(),
                            }.patch,
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
                            "action": "write",
                            "items": "bad",
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
                                "action": "search",
                                "query": 123,
                            })
                            .to_string(),
                        ),
                        (
                            "call_bad_tools_2",
                            "tools",
                            serde_json::json!({
                                "action": "search",
                                "query": "todo",
                                "limit": 1,
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

    tokio::time::timeout(std::time::Duration::from_secs(5), chunk_ready)
        .await
        .expect("streaming chunk should be emitted");

    let partial_output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
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

#[test]
fn blocked_permission_survives_restart_and_reply_continues() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "permission-resume.db").await;
        let tool_policy =
            ToolPermissionPolicy::allow_all().with_tool_mode("todo", PermissionMode::Ask);
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
    });
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

#[test]
fn goal_runtime_resumed_session_can_continue_active_goal_after_restart() {
    run_async_with_large_stack(async move {
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
    });
}
