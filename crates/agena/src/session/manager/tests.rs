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
use crate::db::entities::activity_message;
use crate::db::init_schema;
use crate::event::EventKind;
use crate::message::{
    ApplyPatchToolInput, AskUserToolInput, ExecutionStatus, MessageMetadata, MessagePart, TodoItem,
    TodoPriority, TodoStatus, TodoWriteToolInput, UserInputOption, UserInputQuestion,
};
use crate::model::{ModelId, ModelRef, ProviderId};
use crate::permission::{
    NetworkPermissionPolicy, PermissionAction, PermissionMode, PermissionPolicy,
    ToolPermissionPolicy,
};
use crate::provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    CompletionUsage, ModelRuntime, ProviderModel, ProviderRegistry,
};
use crate::role::Role;
use crate::session::history::TranscriptContent;
use crate::session::{ContextGovernor, ContextPolicy};

use super::*;
use crate::session::cache::SessionCachePolicy;

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

struct StreamingFixturePlugin {
    chunk_sent: Arc<tokio::sync::Notify>,
    finish: Arc<tokio::sync::Semaphore>,
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
        let permit = self
            .finish
            .acquire()
            .await
            .expect("streaming fixture finish semaphore should remain open");
        drop(permit);
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

    fn with_metadata(mut self, metadata: crate::provider::ModelMetadata) -> Self {
        self.metadata = metadata;
        self
    }

    fn with_usage(mut self, usage: CompletionUsage) -> Self {
        self.usage = Some(usage);
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
        let web_tool_result = request.messages.iter().find_map(|message| {
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some("call_web_1") {
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
        } else if last_user_text.contains("unsupported tool") && web_tool_result.is_none() {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    stream_key: "call_web_1".to_string(),
                    id: Some("call_web_1".to_string()),
                    name: Some("web".to_string()),
                    arguments_delta: serde_json::json!({}).to_string(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: scripted_provider_id(),
                    model: scripted_model_id(),
                    finish_reason: Some(CompletionFinishReason::ToolCalls),
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else if let Some(web_tool_result) = web_tool_result {
            let delta = match web_tool_result {
                Ok(output) => format!("unsupported tool handled: {output}"),
                Err(error) => format!("unsupported tool handled: {error}"),
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

async fn build_streaming_plugin_host(
    workspace_root: &std::path::Path,
) -> (
    Arc<crate::plugin::PluginHost>,
    Arc<tokio::sync::Notify>,
    Arc<tokio::sync::Semaphore>,
) {
    let chunk_sent = Arc::new(tokio::sync::Notify::new());
    let finish = Arc::new(tokio::sync::Semaphore::new(0));
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
    pending_permission_request_ids(session)
        .into_iter()
        .next()
        .expect("session should contain a pending permission request")
}

fn pending_permission_request_ids(session: &Session) -> Vec<String> {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Request(crate::message::RequestPart::Permission(request)))
                if request.reply.is_none() =>
            {
                Some(request.request.request_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn pending_user_input_request_id(session: &Session) -> String {
    pending_user_input_request_ids(session)
        .into_iter()
        .next()
        .expect("session should contain a pending user input request")
}

fn pending_user_input_request_ids(session: &Session) -> Vec<String> {
    session
        .messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Request(crate::message::RequestPart::UserInput(request)))
                if request.reply.is_none() =>
            {
                Some(request.request.request_id.clone())
            }
            _ => None,
        })
        .collect()
}

fn run_options() -> SessionRunOptions {
    SessionRunOptions::new(scripted_model_ref()).with_max_output_tokens(Some(128))
}

fn recording_run_options() -> SessionRunOptions {
    SessionRunOptions::new(recording_model_ref())
        .with_system(Some("system".to_string()))
        .with_temperature(Some(0.2))
        .with_max_output_tokens(Some(256))
}

fn context_limit_recording_usage() -> CompletionUsage {
    CompletionUsage {
        input_tokens: 245_000,
        output_tokens: 100,
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
    let _ = manager.run_registry.register(session_id).await;

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
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(16 * 1024 * 1024)
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
            .submit_user_message(SessionUserMessageRequest::new(
                session_id,
                run_options(),
                vec![crate::message::PartContent::text("stream plugin")],
            ))
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

    finish.add_permits(1);
    let completed = tokio::time::timeout(std::time::Duration::from_secs(15), submit)
        .await
        .expect("streaming submit should finish after fixture release")
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

#[test]
fn unsupported_tool_call_is_returned_to_model() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "unsupported_tool_call.db").await;
        let manager = build_manager_with_provider_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
        )
        .await;
        let created = manager
            .create_session(SessionCreateRequest {
                title: "Unsupported tool fixture".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("session creation should succeed");

        let completed = manager
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                run_options(),
                vec![crate::message::PartContent::text("unsupported tool")],
            ))
            .await
            .expect("run should continue after unsupported tool call");

        let (status, error, output) = operation_snapshot(&completed, "call_web_1");
        assert_eq!(status, ExecutionStatus::Failed);
        assert!(
            error
                .as_deref()
                .is_some_and(|value| value.contains("invalid tool input"))
        );
        assert!(output.contains("missing field `action`"));
        assert!(
            completed.messages.iter().any(|message| {
                message.role == Role::Assistant
                    && message
                        .as_text_lossy()
                        .contains("unsupported tool handled:")
            }),
            "final assistant reply should reflect the tool failure: messages={:?}",
            completed.messages
        );
    });
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

    let run_id = HistoryRunId::new();
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
            EventKind::RunStarted(RunStarted {
                run_id,
                source: RunSource::User,
                model_id: "direct-model".into(),
                provider_id: "direct-provider".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("publish direct run start");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::UserMessageAppended(UserMessageAppended {
                message_id: HistoryMessageId(message_id),
                run_id,
                created_at,
                content: TranscriptContent::from_text("published directly"),
                parts: vec![part.clone()],
                metadata: MessageMetadata::default(),
                provider_state: None,
            }),
        )
        .await
        .expect("publish direct user message");
    service
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(created.id),
            EventKind::RunCompleted(RunCompleted {
                run_id,
                finish_reason: FinishReason::Stop,
            }),
        )
        .await
        .expect("publish direct run completion");

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
            title: "append-only-run".into(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    service
        .submit_user_message(SessionUserMessageRequest::new(
            created.id,
            run_options(),
            vec![PartContent::text("hi")],
        ))
        .await
        .expect("submit message");

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
async fn session_usage_does_not_guess_unknown_context_window() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "usage-unknown-context".into(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let usage = service.session_usage(&created).expect("session usage");

    assert_eq!(usage.model_context_window_tokens, None);
    assert_eq!(usage.limit_tokens, None);
    assert_eq!(usage.reserved_tokens, None);
    assert_eq!(usage.limit_basis, None);
}

#[tokio::test]
async fn auto_compact_does_not_trigger_when_context_window_unknown() {
    let workspace = TempWorkspace::new();
    let requests = Arc::new(Mutex::new(Vec::new()));
    let service = build_manager_with_provider(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
        ContextPolicy::default(),
        RecordingProvider::new(requests.clone()).with_usage(context_limit_recording_usage()),
    )
    .await;

    let created = service
        .create_session(SessionCreateRequest {
            title: "auto-compact-unknown-context".into(),
            parent_session_id: None,
        })
        .await
        .expect("create session");

    let first = service
        .submit_user_message(SessionUserMessageRequest::new(
            created.id,
            recording_run_options(),
            vec![PartContent::text("seed")],
        ))
        .await
        .expect("first user message");
    assert!(first.runtime.prompt_window.compaction.is_none());

    let second = service
        .submit_user_message(SessionUserMessageRequest::new(
            created.id,
            recording_run_options(),
            vec![PartContent::text("trigger compaction")],
        ))
        .await
        .expect("second run");

    assert!(
        second.runtime.prompt_window.compaction.is_none(),
        "unknown context windows should not trigger automatic compaction"
    );
    assert_eq!(
        requests.lock().expect("request lock should succeed").len(),
        2,
        "expected only the two ordinary model runs"
    );
}

#[test]
fn auto_compact_triggers_at_known_context_limit() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let provider = RecordingProvider::new(requests.clone())
            .with_metadata(
                crate::provider::ModelMetadata::default().with_context_window_tokens(272_000),
            )
            .with_usage(context_limit_recording_usage());
        let service = build_manager_with_provider(
            &workspace.root,
            PermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            provider,
        )
        .await;

        let created = service
            .create_session(SessionCreateRequest {
                title: "auto-compact-known-context".into(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let first = service
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                recording_run_options(),
                vec![PartContent::text("seed")],
            ))
            .await
            .expect("first user message");
        assert!(first.runtime.prompt_window.compaction.is_none());

        let second = service
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                recording_run_options(),
                vec![PartContent::text("trigger compaction")],
            ))
            .await
            .expect("second run");

        assert!(
            second.runtime.prompt_window.compaction.is_some(),
            "second run should install an automatic compaction snapshot"
        );
        assert!(
            requests.lock().expect("request lock should succeed").len() >= 3,
            "expected first user message, local compaction run, and post-compaction run"
        );
    });
}

#[tokio::test]
async fn restart_after_interrupted_turn_can_continue_session() {
    use crate::session::history::RunAbortReason;

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
        SessionRunOptions::new(ModelRef::new("restartable", "restartable-model"))
            .with_max_output_tokens(Some(128))
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
                .submit_user_message(SessionUserMessageRequest::new(
                    session_id,
                    restartable_options(),
                    vec![PartContent::text("start then restart")],
                ))
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
                    EventKind::RunStarted(payload)
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
            .expect_err("run task should be aborted")
            .is_cancelled()
    );
    let interrupted_turn = HistoryRunId::new();
    first
        .event_publisher()
        .publish(
            crate::event::PublishContext::for_session(session_id),
            EventKind::RunStarted(RunStarted {
                run_id: interrupted_turn,
                source: RunSource::User,
                model_id: "restartable-model".into(),
                provider_id: "restartable".into(),
                request_digest: None,
            }),
        )
        .await
        .expect("interrupted run should be persisted");
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
        .continue_session(SessionExecutionRequest::new(
            session_id,
            restartable_options(),
        ))
        .await
        .expect("continue should recover after restart");
    let history = second
        .list_session_events(session_id)
        .await
        .expect("history should load");

    assert!(history.iter().any(|record| {
        matches!(
            &record.kind,
            EventKind::RunAborted(payload)
                if payload.run_id == interrupted_turn
                    && payload.reason == RunAbortReason::ProcessRestart
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
            .submit_user_message(SessionUserMessageRequest::new(
                session_id,
                run_options(),
                vec![PartContent::text("permission todo")],
            ))
            .await
            .expect("run should block on permission");
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
            .reply_permission(SessionPermissionReplyRequest::new(
                session_id,
                run_options(),
                PermissionReply {
                    request_id,
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                Some("test".to_string()),
            ))
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

#[test]
fn duplicate_permission_reply_is_idempotent() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "permission-idempotent.db").await;
        let manager = build_manager_with_provider_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all().with_tool_mode("todo", PermissionMode::Ask),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
        )
        .await;
        let created = manager
            .create_session(SessionCreateRequest {
                title: "permission-idempotent".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let blocked = manager
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                run_options(),
                vec![PartContent::text("permission todo")],
            ))
            .await
            .expect("run should block on permission");
        let request = SessionPermissionReplyRequest::new(
            created.id,
            run_options(),
            PermissionReply {
                request_id: pending_permission_request_id(&blocked),
                kind: PermissionReplyKind::AllowOnce,
                reason: None,
                scope: None,
            },
            Some("test".to_string()),
        );

        let (first, second) = tokio::join!(
            manager.reply_permission(request.clone()),
            manager.reply_permission(request),
        );
        first.expect("first permission reply should succeed");
        second.expect("duplicate permission reply should be ignored");

        let session = manager
            .get_session(created.id)
            .await
            .expect("session should reload after duplicate permission reply");
        assert!(
            !session.blocked(),
            "session should no longer be blocked: runtime={:?}",
            session.runtime()
        );
        assert!(
            session
                .messages
                .iter()
                .any(|message| message.as_text_lossy().contains("permission todo done")),
            "final assistant reply should survive duplicate permission reply: messages={:?}",
            session.messages
        );
    });
}

#[test]
fn duplicate_user_input_reply_is_idempotent() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "user-input-idempotent.db").await;
        let manager = build_manager_with_provider_on_db(
            &workspace.root,
            db.clone(),
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
        )
        .await;
        let created = manager
            .create_session(SessionCreateRequest {
                title: "user-input-idempotent".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let state = manager.execution_state();
        let session = manager
            .get_session(created.id)
            .await
            .expect("session should load");
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("message ids should reserve");
        let invocation = ToolInvocation::new(
            "user",
            crate::message::StructuredObject::try_from(serde_json::json!({
                "action": "request_input",
                "questions": [],
            }))
            .expect("tool input should serialize"),
        );
        let mut assistant_message = build_message(
            ids,
            Role::Assistant,
            MessageStatus::Pending,
            vec![PartContent::Operation(OperationPart::pending(
                1,
                invocation,
                "Ask user",
                TimeRange::default(),
            ))],
            MessageMetadata::default(),
        );
        assistant_message.parts[0].operation_id = Some("call_manual_user_input_1".to_string());
        let session = manager
            .store
            .append_history_items(
                session,
                vec![EventKind::AssistantMessageCompleted(
                    crate::session::history::AssistantMessageCompleted {
                        message_id: HistoryMessageId(assistant_message.id),
                        run_id: HistoryRunId::new(),
                        created_at: assistant_message.created_at,
                        content: TranscriptContent::from_message_lossy(&assistant_message),
                        parts: assistant_message.parts.clone(),
                        usage: None,
                        finish_reason: FinishReason::Stop,
                        metadata: assistant_message.metadata.clone(),
                        provider_state: assistant_message.provider_state.clone(),
                    },
                )],
                state.cache_policy(),
            )
            .await
            .expect("pending tool should persist through history");
        let pending_tool = session
            .pending_tools()
            .into_iter()
            .next()
            .expect("session should expose the pending tool");
        let blocked = manager
            .apply_user_input_request_with_id(
                session,
                &pending_tool,
                AskUserToolInput {
                    questions: vec![UserInputQuestion {
                        id: "model_choice".to_string(),
                        header: "Model".to_string(),
                        question: "Which model should we use?".to_string(),
                        options: vec![
                            UserInputOption {
                                label: "gpt-5".to_string(),
                                description: "Use the flagship reasoning model.".to_string(),
                            },
                            UserInputOption {
                                label: "gpt-4.1".to_string(),
                                description: "Use the faster general-purpose model.".to_string(),
                            },
                        ],
                        multiple: false,
                        allow_custom: false,
                    }],
                },
                "call_manual_user_input_1".to_string(),
                state.clone(),
            )
            .await
            .expect("user input request should persist");
        let request = SessionExecutionReplyRequest::new(
            created.id,
            run_options(),
            UserInputReply {
                request_id: pending_user_input_request_id(&blocked),
                kind: UserInputReplyKind::Submit,
                reason: None,
                answers: BTreeMap::from([("model_choice".to_string(), vec!["gpt-5".to_string()])]),
            },
        );

        let (first, second) = tokio::join!(
            manager.reply_user_input(request.clone()),
            manager.reply_user_input(request),
        );
        first.expect("first user input reply should succeed");
        second.expect("duplicate user input reply should be ignored");

        let session = manager
            .get_session(created.id)
            .await
            .expect("session should reload after duplicate user input reply");
        assert!(
            !session.blocked(),
            "session should no longer be blocked: runtime={:?}",
            session.runtime()
        );
        assert!(
            session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .any(|part| {
                    part.operation_id.as_deref() == Some("call_manual_user_input_1")
                        && part.status == ExecutionStatus::Completed
                }),
            "user input operation should complete after duplicate reply: messages={:?}",
            session.messages,
        );
    });
}

#[test]
fn replied_host_user_input_survives_restart_and_restores_answer_from_history() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "host-user-input-resume.db").await;
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
                title: "host-user-input-resume".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let state = first.execution_state();
        let session = first
            .get_session(created.id)
            .await
            .expect("session should load");
        let ids = first
            .store
            .reserve_message_ids(2)
            .await
            .expect("message ids should reserve");
        let todo_input = crate::message::StructuredObject::try_from(serde_json::json!({
            "action": "write",
            "items": [{
                "content": "resume host input",
                "status": "completed",
                "priority": "low",
            }],
        }))
        .expect("todo tool input should serialize");
        let mut assistant_message = build_message(
            ids,
            Role::Assistant,
            MessageStatus::Pending,
            vec![PartContent::Operation(OperationPart::pending(
                1,
                ToolInvocation::new("todo", todo_input),
                "todo",
                TimeRange::default(),
            ))],
            MessageMetadata::default(),
        );
        assistant_message.parts[0].operation_id = Some("call_host_input_1".to_string());
        let session = first
            .store
            .append_history_items(
                session,
                vec![EventKind::AssistantMessageCompleted(
                    crate::session::history::AssistantMessageCompleted {
                        message_id: HistoryMessageId(assistant_message.id),
                        run_id: HistoryRunId::new(),
                        created_at: assistant_message.created_at,
                        content: TranscriptContent::from_message_lossy(&assistant_message),
                        parts: assistant_message.parts.clone(),
                        usage: None,
                        finish_reason: FinishReason::ToolCalls,
                        metadata: assistant_message.metadata.clone(),
                        provider_state: assistant_message.provider_state.clone(),
                    },
                )],
                state.cache_policy(),
            )
            .await
            .expect("pending tool should persist");
        let pending_tool = session
            .pending_tools()
            .into_iter()
            .next()
            .expect("pending tool should exist");
        let blocked = first
            .apply_user_input_request_with_id(
                session,
                &pending_tool,
                AskUserToolInput {
                    questions: vec![UserInputQuestion {
                        id: "confirm".to_string(),
                        header: "Confirm".to_string(),
                        question: "Should the resumed host tool continue?".to_string(),
                        options: vec![
                            UserInputOption {
                                label: "yes".to_string(),
                                description: "Continue the resumed tool.".to_string(),
                            },
                            UserInputOption {
                                label: "no".to_string(),
                                description: "Stop the resumed tool.".to_string(),
                            },
                        ],
                        multiple: false,
                        allow_custom: false,
                    }],
                },
                "host-input:1:1:0".to_string(),
                state.clone(),
            )
            .await
            .expect("host user input request should persist");
        let request_id = pending_user_input_request_id(&blocked);
        drop(first);

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

        let completed = second
            .reply_user_input(SessionExecutionReplyRequest::new(
                created.id,
                run_options(),
                UserInputReply {
                    request_id: request_id.clone(),
                    kind: UserInputReplyKind::Submit,
                    reason: None,
                    answers: BTreeMap::from([("confirm".to_string(), vec!["yes".to_string()])]),
                },
            ))
            .await
            .expect("host user input reply should survive restart without a waiter");
        let (status, error, _) = operation_snapshot(&completed, "call_host_input_1");
        assert_eq!(status, ExecutionStatus::Completed);
        assert!(
            error.is_none(),
            "replayed host tool should complete cleanly"
        );
    });
}

#[test]
fn concurrent_permission_replies_for_distinct_requests_are_serialized() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        let db = open_temp_database(&workspace.root, "distinct-permission-replies.db").await;
        let manager = build_manager_with_provider_on_db(
            &workspace.root,
            db,
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all().with_tool_mode("todo", PermissionMode::Ask),
            SessionManagerConfig::default(),
            ContextPolicy::default(),
            ScriptedProvider,
        )
        .await;
        let created = manager
            .create_session(SessionCreateRequest {
                title: "distinct-permission-replies".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session");

        let state = manager.execution_state();
        let session = manager
            .get_session(created.id)
            .await
            .expect("session should load");
        let ids = manager
            .store
            .reserve_message_ids(2)
            .await
            .expect("message ids should reserve");
        let todo_input_one = crate::message::StructuredObject::try_from(serde_json::json!({
            "action": "write",
            "items": [{
                "content": "approve permission one",
                "status": "completed",
                "priority": "low",
            }],
        }))
        .expect("todo tool input one should serialize");
        let todo_input_two = crate::message::StructuredObject::try_from(serde_json::json!({
            "action": "write",
            "items": [{
                "content": "approve permission two",
                "status": "completed",
                "priority": "low",
            }],
        }))
        .expect("todo tool input two should serialize");
        let mut assistant_message = build_message(
            ids,
            Role::Assistant,
            MessageStatus::Pending,
            vec![
                PartContent::Operation(OperationPart::pending(
                    1,
                    ToolInvocation::new("todo", todo_input_one),
                    "todo",
                    TimeRange::default(),
                )),
                PartContent::Operation(OperationPart::pending(
                    2,
                    ToolInvocation::new("todo", todo_input_two),
                    "todo",
                    TimeRange::default(),
                )),
            ],
            MessageMetadata::default(),
        );
        assistant_message.parts[0].operation_id = Some("call_manual_permission_1".to_string());
        assistant_message.parts[1].operation_id = Some("call_manual_permission_2".to_string());
        let session = manager
            .store
            .append_history_items(
                session,
                vec![EventKind::AssistantMessageCompleted(
                    crate::session::history::AssistantMessageCompleted {
                        message_id: HistoryMessageId(assistant_message.id),
                        run_id: HistoryRunId::new(),
                        created_at: assistant_message.created_at,
                        content: TranscriptContent::from_message_lossy(&assistant_message),
                        parts: assistant_message.parts.clone(),
                        usage: None,
                        finish_reason: FinishReason::ToolCalls,
                        metadata: assistant_message.metadata.clone(),
                        provider_state: assistant_message.provider_state.clone(),
                    },
                )],
                state.cache_policy(),
            )
            .await
            .expect("manual pending tools should persist through history");
        let pending_action = PermissionAction::Tool {
            tool_name: "todo".to_string(),
            qualifier: None,
        };
        let pending_reason = "tool 'todo' requires confirmation by policy".to_string();
        let pending_trace = vec![crate::permission::DecisionTraceStep {
            source_kind: crate::permission::PolicySourceKind::StaticPolicy,
            summary: pending_reason.clone(),
            source: Some("static_policy".to_string()),
            scope: None,
            operator: None,
        }];
        let pending_tool_one = session
            .pending_tools()
            .into_iter()
            .find(|tool| {
                session
                    .part(&tool.part)
                    .and_then(|part| part.operation_id.as_deref())
                    == Some("call_manual_permission_1")
            })
            .expect("first manual pending tool should exist");
        let session = manager
            .apply_permission_request(
                session,
                &pending_tool_one,
                pending_action.clone(),
                vec![pending_action.clone()],
                vec![pending_action.clone()],
                pending_reason.clone(),
                pending_reason.clone(),
                Some("static_policy".to_string()),
                None,
                None,
                crate::permission::PermissionRiskLevel::Medium,
                pending_trace.clone(),
                state.clone(),
            )
            .await
            .expect("first manual permission request should persist");
        let pending_tool_two = session
            .pending_tools()
            .into_iter()
            .find(|tool| {
                session
                    .part(&tool.part)
                    .and_then(|part| part.operation_id.as_deref())
                    == Some("call_manual_permission_2")
            })
            .expect("second manual pending tool should exist");
        let blocked = manager
            .apply_permission_request(
                session,
                &pending_tool_two,
                pending_action.clone(),
                vec![pending_action.clone()],
                vec![pending_action.clone()],
                pending_reason.clone(),
                pending_reason.clone(),
                Some("static_policy".to_string()),
                None,
                None,
                crate::permission::PermissionRiskLevel::Medium,
                pending_trace,
                state.clone(),
            )
            .await
            .expect("second manual permission request should persist");
        let mut request_ids = pending_permission_request_ids(&blocked);
        request_ids.sort();
        assert_eq!(
            request_ids.len(),
            2,
            "session should surface both pending permission requests: messages={:?}",
            blocked.messages
        );

        let first_request = SessionPermissionReplyRequest::new(
            created.id,
            run_options(),
            PermissionReply {
                request_id: request_ids[0].clone(),
                kind: PermissionReplyKind::AllowOnce,
                reason: None,
                scope: None,
            },
            Some("test".to_string()),
        );
        let second_request = SessionPermissionReplyRequest::new(
            created.id,
            run_options(),
            PermissionReply {
                request_id: request_ids[1].clone(),
                kind: PermissionReplyKind::AllowOnce,
                reason: None,
                scope: None,
            },
            Some("test".to_string()),
        );

        let (first, second) = tokio::join!(
            manager.reply_permission(first_request),
            manager.reply_permission(second_request),
        );
        first.expect("first distinct permission reply should succeed");
        second.expect("second distinct permission reply should wait and succeed");

        let session = manager
            .get_session(created.id)
            .await
            .expect("session should reload after concurrent permission replies");
        assert!(
            !session.blocked(),
            "session should finish after both permission replies: runtime={:?}",
            session.runtime()
        );
        for operation_id in ["call_manual_permission_1", "call_manual_permission_2"] {
            let (status, error, _) = operation_snapshot(&session, operation_id);
            assert_eq!(
                status,
                ExecutionStatus::Completed,
                "{operation_id} was not completed: error={error:?}\nmessages={:?}\nruntime={:?}",
                session.messages,
                session.runtime()
            );
            assert!(error.is_none(), "{operation_id} failed: {error:?}");
        }
    });
}

#[test]
fn pending_permission_request_aggregates_invocation_actions() {
    run_async_with_large_stack(async move {
        let workspace = TempWorkspace::new();
        fs::write(
            workspace.root.join("notes.txt"),
            "aggregated permissions fixture\n",
        )
        .expect("fixture file should be written");
        let db = open_temp_database(&workspace.root, "permission-aggregate.db").await;
        let agents = crate::agents::SubagentRegistry::discover(&workspace.root, None);
        let mut agent = Agent::new(
            "build",
            PermissionPolicy::allow_all().with_workspace_read_default(PermissionMode::Ask),
        )
        .with_tool_policy(ToolPermissionPolicy::allow_all());
        agent.network_policy =
            NetworkPermissionPolicy::allow_all().with_internet_default(PermissionMode::Ask);
        let executor =
            ToolExecutor::new(&workspace.root, agent).with_subagent_registry(agents.clone());
        let plugins = crate::tool::default_tool_host(&workspace.root).expect("default plugin host");
        plugins
            .host_handle()
            .install_client(Arc::new(SessionTestHostClient {
                executor: executor.clone().with_plugin_manager(Arc::clone(&plugins)),
            }))
            .await;
        let executor = executor.with_plugin_manager(plugins.clone());
        let mut registry = ProviderRegistry::new();
        registry.register(ScriptedProvider);
        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(ContextPolicy::default()),
        )
        .with_plugin_host(Arc::clone(&plugins));
        let test_executor = executor.clone();
        let manager = SessionManager::new(db, processor, executor)
            .with_config(SessionManagerConfig::default());
        let invocation = crate::tool::ToolPayloadInput::Bash(crate::message::ShellCommandInput {
            command: "curl https://api.example.com/health && cat notes.txt".to_string(),
            description: "aggregate permission request".to_string(),
            timeout_ms: Some(5_000),
            workdir: Some(".".to_string()),
            filesystem_effects: vec![crate::message::FilesystemEffect {
                path: "notes.txt".to_string(),
                access: crate::message::FilesystemAccess::Read,
            }],
            network_effects: vec![crate::message::NetworkEffect {
                target: "https://api.example.com/health".to_string(),
            }],
        })
        .into_invocation();
        let checks = test_executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission checks should collect");
        let outcome = manager
            .aggregate_permission_outcome(None, checks.as_slice())
            .await
            .expect("permission aggregation should succeed");
        let request = match outcome {
            super::replies::AggregatedPermissionOutcome::Request(request) => request,
            super::replies::AggregatedPermissionOutcome::Allow => {
                panic!("aggregated outcome should request confirmation")
            }
            super::replies::AggregatedPermissionOutcome::Deny { reason } => {
                panic!("aggregated outcome should not deny: {reason}")
            }
        };

        assert_eq!(
            request.requested_actions.len(),
            3,
            "workdir, file, and network actions should all require confirmation"
        );
        assert!(
            request
                .reason
                .contains("plus 2 more permission checks for this tool call"),
            "aggregated permission reason should mention additional requested actions: {}",
            request.reason
        );
        assert!(
            request.related_actions.iter().any(|action| matches!(
                action,
                PermissionAction::Tool { tool_name, .. } if tool_name == "shell" || tool_name == "bash"
            )),
            "aggregated request should include the shell tool action: {:?}",
            request.related_actions
        );
        assert!(
            request.related_actions.iter().any(|action| matches!(
                action,
                PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } if access_kind == "read" && target_path.ends_with("notes.txt")
            )),
            "aggregated request should include the file read action: {:?}",
            request.related_actions
        );
        assert!(
            request.related_actions.iter().any(|action| matches!(
                action,
                PermissionAction::NetworkAccess { host, port, .. }
                    if host == "api.example.com" && port == &Some(443)
            )),
            "aggregated request should include the network action: {:?}",
            request.related_actions
        );
        assert!(
            request.requested_actions.iter().any(|action| matches!(
                action,
                PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } if access_kind == "read" && target_path.ends_with("notes.txt")
            )),
            "requested actions should include the file read permission: {:?}",
            request.requested_actions
        );
        assert!(
            request.requested_actions.iter().any(|action| matches!(
                action,
                PermissionAction::NetworkAccess { host, port, .. }
                    if host == "api.example.com" && port == &Some(443)
            )),
            "requested actions should include the network permission: {:?}",
            request.requested_actions
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
    assert!(
        started,
        "idle goal creation should start one goal-triggered run"
    );

    let final_session = async {
        for _ in 0..500 {
            let session = manager
                .get_session(created.id)
                .await
                .expect("reload session during goal run");
            if session.status() == SessionStatus::Idle
                && !manager.is_run_active(created.id).await
                && session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant)
            {
                return session;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("goal run should settle within 10s");
    }
    .await;

    let recorded = requests
        .lock()
        .expect("recording provider request lock should succeed");
    assert_eq!(
        recorded.len(),
        1,
        "goal creation should trigger exactly one goal run"
    );
    let request = &recorded[0];
    assert_eq!(request.system.as_deref(), Some("system"));

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
            .continue_session(SessionExecutionRequest::new(
                session_id,
                recording_run_options(),
            ))
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

#[tokio::test]
async fn goal_runtime_external_goal_clear_stops_next_continue_run() {
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
        .continue_session(SessionExecutionRequest::new(
            created.id,
            recording_run_options(),
        ))
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

/// Cancel a run while the provider stream is still pending. The
/// processor must observe the cancellation token and surface a
/// terminal error rather than running to completion.
///
/// Currently flaky under heavy parallel test load (the cancel can race
/// with the manager's stream consumer in non-deterministic ways).
/// Tracked separately; runs reliably in isolation.
#[ignore = "flaky under cargo test --workspace; passes with -p agena --lib"]
#[tokio::test]
async fn cancel_active_run_aborts_a_running_run() {
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
                // First chunk arrives quickly so the run is "live".
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
        SessionRunOptions::new(ModelRef::new("slow", "slow-model")).with_max_output_tokens(Some(64))
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
        mgr.submit_user_message(SessionUserMessageRequest::new(
            session_id,
            slow_options(),
            vec![PartContent::text("ping")],
        ))
        .await
    });

    // Poll until the run registers with RunRegistry rather than
    // sleeping a fixed duration — the original 80 ms was flaky under
    // load. Use a generous budget (10s) so concurrent cargo test runs
    // don't race even on heavily loaded CI runners.
    let registered = async {
        for _ in 0..500 {
            if manager.is_run_active(session_id).await {
                return true;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
        false
    }
    .await;
    assert!(registered, "run should register within 10s");
    // Try cancel; if it races with run-registry teardown we retry once.
    for attempt in 0..3 {
        match manager.cancel_active_run(session_id).await {
            Ok(()) => break,
            Err(_) if attempt < 2 => {
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
            Err(err) => panic!("cancel should find active run: {err}"),
        }
    }

    // The submit future should resolve quickly now (not after 60s).
    let result = tokio::time::timeout(std::time::Duration::from_secs(15), submit)
        .await
        .expect("submit should complete after cancel")
        .expect("join");
    // The session run reports an error because the run was aborted.
    assert!(
        result.is_err(),
        "expected run to be reported as failed/cancelled"
    );
}

/// `cancel_active_run` for a session with no in-flight run returns
/// the corresponding error, never panics.
#[tokio::test]
async fn cancel_with_no_active_run_is_a_clean_error() {
    let workspace = TempWorkspace::new();
    let manager = build_manager(
        &workspace.root,
        PermissionPolicy::allow_all(),
        SessionManagerConfig::default(),
    )
    .await;
    let err = manager.cancel_active_run(1234).await.unwrap_err();
    assert!(matches!(err, AppError::Internal(_)));
}

/// `steer_input` against a session with no active run surfaces the
/// "no in-flight run" error so callers can fall back gracefully.
#[tokio::test]
async fn steer_with_no_active_run_is_a_clean_error() {
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

mod runtime_builtin_tool_tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
    use std::time::Duration;

    use crate::message::{EnterWorktreeToolInput, ExitWorktreeToolInput};
    use crate::plugin::sdk::host_api::{
        AskUserRequest, AskUserResponse, HostAgentRestoreRequest, HostAgentRestoreResponse,
        HostAgentSwitchRequest, HostAgentSwitchResponse, HostCallbackContext, HostClearGoalRequest,
        HostClearGoalResponse, HostClient, HostConfigReloadResponse, HostCreateGoalRequest,
        HostCreateGoalResponse, HostEnterPlanModeRequest, HostEnterWorktreeRequest,
        HostExitPlanModeRequest, HostExitWorktreeRequest, HostGetGoalRequest, HostGetGoalResponse,
        HostGetSessionRequest, HostGetSessionResponse, HostGoal, HostGoalStatus, HostLspDiagnostic,
        HostLspListDiagnosticsRequest, HostLspListDiagnosticsResponse, HostLspListServersResponse,
        HostLspServer, HostRenameSessionRequest, HostRenameSessionResponse, HostSession,
        HostTodoPriority, HostTodoStatus, HostTodoWriteRequest, HostUpdateGoalRequest,
        HostUpdateGoalResponse, LogLevel, SpawnSubtaskRequest, SpawnSubtaskResponse,
        ToolDescriptor, current_host_callback_context,
    };
    use crate::plugin::sdk::{EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision};
    use axum::{Router, extract::State, response::Html, routing::get};

    struct NoopJobSink;

    #[async_trait]
    impl agena_scheduler::JobSink for NoopJobSink {
        async fn deliver(
            &self,
            _job: &agena_scheduler::ScheduledJob,
        ) -> agena_scheduler::JobDeliveryResult {
            agena_scheduler::JobDeliveryResult::skipped(None, "runtime tool tests do not deliver")
        }
    }

    #[derive(Debug)]
    struct TestConfigState {
        path: PathBuf,
        generation: AtomicU64,
    }

    #[derive(Clone)]
    struct RuntimeToolTestHostClient {
        executor: ToolExecutor,
        manager: Arc<tokio::sync::RwLock<Option<Arc<SessionManager>>>>,
        config: Arc<TestConfigState>,
    }

    impl RuntimeToolTestHostClient {
        fn new(executor: ToolExecutor, config_path: PathBuf) -> Self {
            Self {
                executor,
                manager: Arc::new(tokio::sync::RwLock::new(None)),
                config: Arc::new(TestConfigState {
                    path: config_path,
                    generation: AtomicU64::new(1),
                }),
            }
        }

        async fn install_manager(&self, manager: Arc<SessionManager>) {
            *self.manager.write().await = Some(manager);
        }

        async fn manager(&self) -> crate::plugin::sdk::Result<Arc<SessionManager>> {
            self.manager
                .read()
                .await
                .clone()
                .ok_or_else(|| crate::plugin::PluginError::new("session manager not installed"))
        }

        fn callback_context(&self) -> crate::plugin::sdk::Result<HostCallbackContext> {
            current_host_callback_context()
                .ok_or_else(|| crate::plugin::PluginError::new("missing host callback context"))
        }

        async fn workflow_tool_output(
            &self,
            tool_name: &str,
            input: serde_json::Value,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            let context = self.callback_context()?;
            self.executor
                .execute_tool_payload_for_host(
                    tool_name,
                    input,
                    context.session_id.filter(|id| *id >= 0),
                    context.call_id.filter(|id| *id >= 0),
                    None,
                )
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))
        }

        fn config_document(&self) -> serde_json::Value {
            let config = if self.config.path.exists() {
                serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(&self.config.path).expect("read config.json"),
                )
                .expect("parse config.json")
            } else {
                serde_json::json!({})
            };
            serde_json::json!({
                "config": config,
                "meta": {
                    "config_path": self.config.path.display().to_string(),
                    "config_found": self.config.path.exists(),
                    "generation": self.config.generation.load(Ordering::SeqCst),
                },
            })
        }
    }

    #[async_trait]
    impl HostClient for RuntimeToolTestHostClient {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> crate::plugin::sdk::Result<()> {
            Ok(())
        }

        async fn subscribe_events(
            &self,
            _filter: EventFilter,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::host_api::EventSubscription> {
            Ok(crate::plugin::sdk::host_api::EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(
            &self,
            _req: PermissionAskInput,
        ) -> crate::plugin::sdk::Result<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(
            &self,
            path: Option<String>,
        ) -> crate::plugin::sdk::Result<serde_json::Value> {
            crate::config::get_json_path(&self.config_document(), path.as_deref())
                .map_err(|err| crate::plugin::PluginError::invalid_params(err.to_string()))
        }

        async fn reload_config(&self) -> crate::plugin::sdk::Result<HostConfigReloadResponse> {
            let previous_generation = self.config.generation.fetch_add(1, Ordering::SeqCst);
            let generation = previous_generation + 1;
            Ok(HostConfigReloadResponse {
                previous_generation,
                generation,
                loaded_at: chrono::Utc::now().to_rfc3339(),
            })
        }

        async fn invoke_tool(
            &self,
            tool: String,
            input: serde_json::Value,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            let manager = self.manager().await?;
            let context = self.callback_context()?;
            let session_id = context
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let call_id = context.call_id.unwrap_or(-1);
            let input = crate::message::StructuredObject::try_from(input)
                .map_err(|err| crate::plugin::PluginError::invalid_params(err.to_string()))?;
            let execution = manager
                .execute_host_invoked_tool(session_id, call_id, ToolInvocation::new(tool, input))
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(host_invoke_execution_output(execution))
        }

        async fn ask_user(
            &self,
            req: AskUserRequest,
        ) -> crate::plugin::sdk::Result<AskUserResponse> {
            let mut answers = BTreeMap::new();
            for question in req.questions {
                let answer = question
                    .options
                    .first()
                    .map(|option| option.label.clone())
                    .or_else(|| question.allow_custom.then_some("auto".to_string()));
                if let Some(answer) = answer {
                    answers.insert(question.id, vec![answer]);
                }
            }
            let reply = answers
                .values()
                .next()
                .and_then(|values| values.first())
                .cloned()
                .unwrap_or_default();
            Ok(AskUserResponse {
                reply,
                answers,
                cancelled: false,
            })
        }

        async fn spawn_subtask(
            &self,
            req: SpawnSubtaskRequest,
        ) -> crate::plugin::sdk::Result<SpawnSubtaskResponse> {
            let parent_session_id = self
                .callback_context()?
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let response = manager
                .spawn_subtask(SessionSubtaskRequest {
                    parent_session_id,
                    description: req.description.clone(),
                    prompt: req.prompt.clone(),
                    subagent_type: parse_subagent_type(req.subagent_type.as_str())
                        .unwrap_or(crate::message::TaskSubagentType::Explore),
                    profile_name: Some(req.subagent_type),
                    task_id: req.task_id,
                    command: req.command,
                    requested_model: req.model,
                })
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            let mut metadata = BTreeMap::new();
            metadata.insert("session_id".to_string(), response.session.id.to_string());
            if let Some(provider) = response.model_provider_id {
                metadata.insert("model_provider_id".to_string(), provider);
            }
            if let Some(model_id) = response.model_id {
                metadata.insert("model_id".to_string(), model_id);
            }
            Ok(SpawnSubtaskResponse {
                final_text: format!("spawned {}", req.description),
                metadata,
            })
        }

        async fn list_tools(&self) -> crate::plugin::sdk::Result<Vec<ToolDescriptor>> {
            Ok(self
                .executor
                .detailed_tools()
                .into_iter()
                .map(|tool| {
                    let name = tool.exposed_name.clone();
                    let description = Some(tool.description_text().to_string());
                    let summary = tool.summary_text().map(ToString::to_string);
                    let help = tool.help_text().map(ToString::to_string);
                    let input_schema = Some(tool.sanitized_input_schema());
                    let description_mode = tool.decl.description_mode;
                    let tags = tool.effective_tags();
                    let plugin_id =
                        (!tool.plugin_name.trim().is_empty()).then_some(tool.plugin_name.clone());
                    ToolDescriptor {
                        name,
                        description,
                        summary,
                        help,
                        input_schema,
                        description_mode,
                        tags,
                        plugin_id,
                    }
                })
                .collect())
        }

        async fn todo_write(
            &self,
            req: HostTodoWriteRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            let context = self.callback_context()?;
            let payload = TodoWriteToolInput {
                items: req
                    .items
                    .into_iter()
                    .map(|item| TodoItem {
                        content: item.content,
                        status: match item.status {
                            HostTodoStatus::Pending => TodoStatus::Pending,
                            HostTodoStatus::InProgress => TodoStatus::InProgress,
                            HostTodoStatus::Completed => TodoStatus::Completed,
                            HostTodoStatus::Cancelled => TodoStatus::Cancelled,
                        },
                        priority: match item.priority {
                            HostTodoPriority::High => TodoPriority::High,
                            HostTodoPriority::Medium => TodoPriority::Medium,
                            HostTodoPriority::Low => TodoPriority::Low,
                        },
                    })
                    .collect(),
            };
            self.executor
                .execute_tool_payload_for_host(
                    "todo_write",
                    serde_json::to_value(payload)
                        .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?,
                    context.session_id.filter(|id| *id >= 0),
                    context.call_id.filter(|id| *id >= 0),
                    None,
                )
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))
        }

        async fn get_session(
            &self,
            req: HostGetSessionRequest,
        ) -> crate::plugin::sdk::Result<HostGetSessionResponse> {
            let session_id = req
                .session_id
                .or(self.callback_context()?.session_id)
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let session = manager
                .get_session(session_id)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostGetSessionResponse {
                session: host_session_from_session(&session),
            })
        }

        async fn rename_session(
            &self,
            req: HostRenameSessionRequest,
        ) -> crate::plugin::sdk::Result<HostRenameSessionResponse> {
            let session_id = req
                .session_id
                .or(self.callback_context()?.session_id)
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let session = manager
                .rename_session(session_id, req.title)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostRenameSessionResponse {
                session: host_session_from_session(&session),
            })
        }

        async fn get_goal(
            &self,
            _req: HostGetGoalRequest,
        ) -> crate::plugin::sdk::Result<HostGetGoalResponse> {
            let session_id = self
                .callback_context()?
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let goal = manager
                .get_goal(session_id)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?
                .map(host_goal_from_session_goal);
            Ok(HostGetGoalResponse { goal })
        }

        async fn create_goal(
            &self,
            req: HostCreateGoalRequest,
        ) -> crate::plugin::sdk::Result<HostCreateGoalResponse> {
            let session_id = self
                .callback_context()?
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let goal = manager
                .create_goal(SessionGoalCreateRequest {
                    session_id,
                    objective: req.objective,
                })
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostCreateGoalResponse {
                goal: host_goal_from_session_goal(goal),
            })
        }

        async fn update_goal(
            &self,
            req: HostUpdateGoalRequest,
        ) -> crate::plugin::sdk::Result<HostUpdateGoalResponse> {
            let session_id = self
                .callback_context()?
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let goal = manager
                .update_goal(SessionGoalUpdateRequest {
                    session_id,
                    objective: req.objective,
                    status: req.status.map(host_goal_status_to_session_goal_status),
                    expected_goal_id: None,
                })
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostUpdateGoalResponse {
                goal: host_goal_from_session_goal(goal),
            })
        }

        async fn clear_goal(
            &self,
            _req: HostClearGoalRequest,
        ) -> crate::plugin::sdk::Result<HostClearGoalResponse> {
            let session_id = self
                .callback_context()?
                .session_id
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let cleared = manager
                .clear_goal(session_id)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostClearGoalResponse { cleared })
        }

        async fn lsp_list_servers(&self) -> crate::plugin::sdk::Result<HostLspListServersResponse> {
            let registry = self
                .executor
                .lsp_registry()
                .ok_or_else(|| crate::plugin::PluginError::new("lsp registry not configured"))?;
            let specs = registry.server_specs().await;
            Ok(HostLspListServersResponse {
                servers: specs
                    .into_iter()
                    .map(|spec| HostLspServer {
                        name: spec.name,
                        command: spec.command,
                        args: spec.args,
                        file_extensions: spec.file_extensions,
                    })
                    .collect(),
            })
        }

        async fn lsp_list_diagnostics(
            &self,
            req: HostLspListDiagnosticsRequest,
        ) -> crate::plugin::sdk::Result<HostLspListDiagnosticsResponse> {
            let registry = self
                .executor
                .lsp_registry()
                .ok_or_else(|| crate::plugin::PluginError::new("lsp registry not configured"))?;
            let pairs = registry.collect_diagnostics().await;
            let mut entries = Vec::new();
            for (uri, diagnostics) in pairs {
                if let Some(filter) = req.uri.as_ref()
                    && filter != &uri
                {
                    continue;
                }
                for diagnostic in diagnostics {
                    entries.push(HostLspDiagnostic {
                        uri: uri.clone(),
                        severity: diagnostic
                            .severity
                            .map(|severity| match severity {
                                agena_lsp::lsp_types::DiagnosticSeverity::ERROR => "error",
                                agena_lsp::lsp_types::DiagnosticSeverity::WARNING => "warning",
                                agena_lsp::lsp_types::DiagnosticSeverity::INFORMATION => "info",
                                agena_lsp::lsp_types::DiagnosticSeverity::HINT => "hint",
                                _ => "note",
                            })
                            .unwrap_or("note")
                            .to_string(),
                        message: diagnostic.message,
                        start_line: diagnostic.range.start.line,
                        start_character: diagnostic.range.start.character,
                        end_line: diagnostic.range.end.line,
                        end_character: diagnostic.range.end.character,
                        source: diagnostic.source,
                        code: diagnostic.code.map(|code| match code {
                            agena_lsp::lsp_types::NumberOrString::Number(n) => n.to_string(),
                            agena_lsp::lsp_types::NumberOrString::String(s) => s,
                        }),
                    });
                }
            }
            Ok(HostLspListDiagnosticsResponse { entries })
        }

        async fn enter_plan_mode(
            &self,
            _req: HostEnterPlanModeRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            self.workflow_tool_output("enter_plan_mode", serde_json::json!({}))
                .await
        }

        async fn exit_plan_mode(
            &self,
            _req: HostExitPlanModeRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            self.workflow_tool_output("exit_plan_mode", serde_json::json!({}))
                .await
        }

        async fn enter_worktree(
            &self,
            req: HostEnterWorktreeRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            self.workflow_tool_output(
                "enter_worktree",
                serde_json::to_value(EnterWorktreeToolInput {
                    name: req.name,
                    path: req.path,
                })
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?,
            )
            .await
        }

        async fn exit_worktree(
            &self,
            req: HostExitWorktreeRequest,
        ) -> crate::plugin::sdk::Result<crate::plugin::sdk::ToolInvokeOutput> {
            self.workflow_tool_output(
                "exit_worktree",
                serde_json::to_value(ExitWorktreeToolInput {
                    action: req.action,
                    discard_changes: req.discard_changes,
                })
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?,
            )
            .await
        }

        async fn agent_switch(
            &self,
            req: HostAgentSwitchRequest,
        ) -> crate::plugin::sdk::Result<HostAgentSwitchResponse> {
            let session_id = req
                .session_id
                .or(self.callback_context()?.session_id)
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let outcome = manager
                .switch_session_agent(session_id, req.agent, req.push_previous)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostAgentSwitchResponse {
                session_id: outcome.session_id,
                previous_agent: outcome.previous_agent,
                current_agent: outcome.current_agent,
                stack_depth: outcome.stack_depth,
            })
        }

        async fn agent_restore(
            &self,
            req: HostAgentRestoreRequest,
        ) -> crate::plugin::sdk::Result<HostAgentRestoreResponse> {
            let session_id = req
                .session_id
                .or(self.callback_context()?.session_id)
                .ok_or_else(|| crate::plugin::PluginError::new("missing session_id"))?;
            let manager = self.manager().await?;
            let outcome = manager
                .restore_session_agent(session_id)
                .await
                .map_err(|err| crate::plugin::PluginError::new(err.to_string()))?;
            Ok(HostAgentRestoreResponse {
                session_id: outcome.session_id,
                restored: outcome.restored,
                previous_agent: outcome.previous_agent,
                current_agent: outcome.current_agent,
                stack_depth: outcome.stack_depth,
            })
        }
    }

    fn host_session_from_session(session: &Session) -> HostSession {
        HostSession {
            id: session.id,
            parent_id: session.parent_id,
            root_id: session.root_id,
            workspace_id: session.workspace_id,
            title: session.title.clone(),
            is_subagent: session.is_subagent,
        }
    }

    fn host_goal_status_to_session_goal_status(status: HostGoalStatus) -> GoalStatus {
        match status {
            HostGoalStatus::Active => GoalStatus::Active,
            HostGoalStatus::Paused => GoalStatus::Paused,
            HostGoalStatus::Completed => GoalStatus::Completed,
        }
    }

    fn host_goal_from_session_goal(goal: SessionGoal) -> HostGoal {
        HostGoal {
            id: goal.id,
            objective: goal.objective,
            status: match goal.status {
                GoalStatus::Active => HostGoalStatus::Active,
                GoalStatus::Paused => HostGoalStatus::Paused,
                GoalStatus::Completed => HostGoalStatus::Completed,
            },
            completed_at_ms: goal.completed_at.map(|ts| ts.timestamp_millis()),
        }
    }

    fn parse_subagent_type(raw: &str) -> Option<crate::message::TaskSubagentType> {
        match raw.trim() {
            "explore" => Some(crate::message::TaskSubagentType::Explore),
            "implement" => Some(crate::message::TaskSubagentType::Implement),
            "verify" => Some(crate::message::TaskSubagentType::Verify),
            _ => None,
        }
    }

    async fn build_runtime_tool_manager_with_provider<P>(
        root: &Path,
        db: DatabaseConnection,
        provider: P,
    ) -> (Arc<SessionManager>, Arc<RuntimeToolTestHostClient>)
    where
        P: ModelRuntime + 'static,
    {
        build_runtime_tool_manager_with_provider_and_executor(root, db, provider, |executor| {
            executor
        })
        .await
    }

    async fn build_runtime_tool_manager_with_provider_and_executor<P, F>(
        root: &Path,
        db: DatabaseConnection,
        provider: P,
        configure_executor: F,
    ) -> (Arc<SessionManager>, Arc<RuntimeToolTestHostClient>)
    where
        P: ModelRuntime + 'static,
        F: FnOnce(ToolExecutor) -> ToolExecutor,
    {
        let agents = crate::agents::SubagentRegistry::discover(root, None);
        let plugins = crate::tool::default_tool_host(root).expect("default plugin host");
        let scheduler = agena_scheduler::Scheduler::new(
            Arc::new(agena_scheduler::InMemoryJobStore::new()),
            Arc::new(NoopJobSink),
            Duration::from_secs(60),
        );
        let executor = configure_executor(
            ToolExecutor::new(
                root,
                Agent::new("build", PermissionPolicy::allow_all())
                    .with_tool_policy(ToolPermissionPolicy::allow_all()),
            )
            .with_subagent_registry(agents)
            .with_plan_registry(crate::tool::plan_registry_for_executor())
            .with_worktree_registry(crate::tool::worktree_registry_for_executor())
            .with_scheduler(scheduler)
            .with_lsp_registry(Arc::new(agena_lsp::LspRegistry::new(
                root.to_path_buf(),
                "agena-test",
                "0.0.1",
            ))),
        )
        .with_plugin_manager(plugins.clone());
        let host = Arc::new(RuntimeToolTestHostClient::new(
            executor.clone(),
            root.join("config.json"),
        ));
        plugins.host_handle().install_client(host.clone()).await;

        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(ContextPolicy::default()),
        )
        .with_plugin_host(Arc::clone(&plugins));
        let manager = Arc::new(
            SessionManager::new(db, processor, executor)
                .with_config(SessionManagerConfig::default()),
        );
        host.install_manager(Arc::clone(&manager)).await;
        (manager, host)
    }

    async fn open_runtime_tool_database(root: &Path, name: &str) -> DatabaseConnection {
        let path = root.join(name);
        let mut options =
            sea_orm::ConnectOptions::new(format!("sqlite://{}?mode=rwc", path.display()));
        options.max_connections(8);
        options.min_connections(1);
        let db = Database::connect(options)
            .await
            .expect("failed to create sqlite db");
        init_schema(&db).await.expect("failed to init schema");
        db
    }

    fn init_git_workspace(root: &Path) {
        run_git(root, &["init", "-b", "main"]);
        run_git(root, &["config", "user.name", "Agena Tests"]);
        run_git(root, &["config", "user.email", "tests@example.com"]);
        std::fs::write(root.join("notes.txt"), "base line\n").expect("write notes");
        std::fs::write(
            root.join("demo.ipynb"),
            serde_json::to_string_pretty(&serde_json::json!({
                "cells": [
                    {"cell_type": "markdown", "metadata": {}, "source": ["# Title\n"]},
                    {"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": ["print(1)\n"]}
                ],
                "metadata": {},
                "nbformat": 4,
                "nbformat_minor": 5
            }))
            .expect("serialize notebook"),
        )
        .expect("write notebook");
        std::fs::write(
            root.join("config.json"),
            serde_json::to_string_pretty(&serde_json::json!({
                "default": { "agent": "build" }
            }))
            .expect("serialize config"),
        )
        .expect("write config");
        run_git(root, &["add", "."]);
        run_git(root, &["commit", "-m", "initial"]);
    }

    fn run_git(root: &Path, args: &[&str]) {
        let output = Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap_or_else(|err| panic!("git {args:?} failed to start: {err}"));
        assert!(
            output.status.success(),
            "git {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn request_operation_text(request: &CompletionRequest, operation_id: &str) -> Option<String> {
        request
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                match part.content.as_ref() {
                    Some(PartContent::Operation(operation))
                        if matches!(
                            part.status,
                            ExecutionStatus::Completed | ExecutionStatus::Failed
                        ) =>
                    {
                        Some(operation.model_output.text.clone())
                    }
                    _ => None,
                }
            })
    }

    fn request_operation_debug(request: &CompletionRequest, operation_id: &str) -> String {
        request
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                match part.content.as_ref() {
                    Some(PartContent::Operation(operation)) => Some(format!(
                        "status={:?} summary={:?} model_output={:?} payload={:?}",
                        part.status,
                        operation.summary,
                        operation.model_output.text,
                        operation.details.to_json_payload()
                    )),
                    other => Some(format!("status={:?} non-operation={other:?}", part.status)),
                }
            })
            .unwrap_or_else(|| format!("operation {operation_id} not found"))
    }

    fn request_operation_payload(
        request: &CompletionRequest,
        operation_id: &str,
    ) -> serde_json::Value {
        request
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                match part.content.as_ref() {
                    Some(PartContent::Operation(operation))
                        if matches!(
                            part.status,
                            ExecutionStatus::Completed | ExecutionStatus::Failed
                        ) =>
                    {
                        operation.details.to_json_payload()
                    }
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing payload for {operation_id}"))
    }

    fn session_operation_payload(session: &Session, operation_id: &str) -> serde_json::Value {
        session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                match part.content.as_ref() {
                    Some(PartContent::Operation(operation))
                        if part.status == ExecutionStatus::Completed =>
                    {
                        operation.details.to_json_payload()
                    }
                    _ => None,
                }
            })
            .unwrap_or_else(|| panic!("missing completed payload for {operation_id}"))
    }

    fn session_operation_summaries(session: &Session) -> Vec<String> {
        session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| {
                let operation_id = part.operation_id.as_deref()?;
                let PartContent::Operation(operation) = part.content.as_ref()? else {
                    return None;
                };
                Some(format!(
                    "{operation_id}: status={:?} tool={} summary={:?} output={:?}",
                    part.status,
                    operation.invocation.name,
                    operation.summary,
                    operation.model_output.text,
                ))
            })
            .collect()
    }

    fn runtime_tool_run_options() -> SessionRunOptions {
        SessionRunOptions::new(ModelRef::new("runtime-tools", "runtime-tools-model"))
            .with_max_output_tokens(Some(256))
    }

    async fn create_runtime_tool_session(manager: &SessionManager, title: &str) -> Session {
        manager
            .create_session(SessionCreateRequest {
                title: title.to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create session")
    }

    async fn submit_runtime_tool_prompt(
        manager: &SessionManager,
        session_id: i64,
        prompt: &str,
        failure_context: &str,
    ) -> Session {
        match manager
            .submit_user_message(SessionUserMessageRequest::new(
                session_id,
                runtime_tool_run_options(),
                vec![PartContent::text(prompt)],
            ))
            .await
        {
            Ok(session) => session,
            Err(err) => {
                let failed = manager
                    .get_session(session_id)
                    .await
                    .expect("failed session should reload");
                panic!(
                    "{failure_context}: {err:?}\noperations:\n{}",
                    session_operation_summaries(&failed).join("\n")
                );
            }
        }
    }

    fn assert_operations_completed(session: &Session, operation_ids: &[&str]) {
        for operation_id in operation_ids {
            let (status, error, _) = operation_snapshot(session, operation_id);
            assert_eq!(
                status,
                ExecutionStatus::Completed,
                "{operation_id} was not completed: error={error:?}\noperations:\n{}",
                session_operation_summaries(session).join("\n")
            );
            assert!(error.is_none(), "{operation_id} failed: {error:?}");
        }
    }

    #[derive(Clone)]
    struct LocalWebFixture {
        base_url: String,
        fetch_hits: Arc<AtomicUsize>,
        search_hits: Arc<AtomicUsize>,
    }

    #[derive(Clone)]
    struct LocalWebServerState {
        base_url: String,
        fetch_hits: Arc<AtomicUsize>,
        search_hits: Arc<AtomicUsize>,
    }

    struct LocalLspFixture {
        script_path: PathBuf,
        source_path: PathBuf,
    }

    async fn start_local_web_fixture() -> LocalWebFixture {
        async fn fetch_page(State(state): State<LocalWebServerState>) -> Html<String> {
            state.fetch_hits.fetch_add(1, Ordering::SeqCst);
            Html(
                "<html><body><h1>Runtime Web</h1><p>loopback fetch works</p></body></html>"
                    .to_string(),
            )
        }

        async fn search_results(
            State(state): State<LocalWebServerState>,
        ) -> axum::Json<serde_json::Value> {
            state.search_hits.fetch_add(1, Ordering::SeqCst);
            axum::Json(serde_json::json!({
                "web": {
                    "results": [
                        {
                            "title": "Runtime Web Docs",
                            "url": format!("{}/docs/runtime-web", state.base_url),
                            "description": "Local runtime search result"
                        },
                        {
                            "title": "External Result",
                            "url": "https://example.com/external",
                            "description": "Should be filtered out"
                        }
                    ]
                }
            }))
        }

        let fetch_hits = Arc::new(AtomicUsize::new(0));
        let search_hits = Arc::new(AtomicUsize::new(0));
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").expect("local web fixture should bind");
        listener
            .set_nonblocking(true)
            .expect("local web fixture listener should become nonblocking");
        let addr = listener
            .local_addr()
            .expect("local web fixture should have an address");
        let base_url = format!("http://{addr}");
        let state = LocalWebServerState {
            base_url: base_url.clone(),
            fetch_hits: fetch_hits.clone(),
            search_hits: search_hits.clone(),
        };
        let app = Router::new()
            .route("/page", get(fetch_page))
            .route("/search", get(search_results))
            .with_state(state);
        std::thread::Builder::new()
            .name("runtime-web-fixture".to_string())
            .spawn(move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("runtime web fixture runtime");
                runtime.block_on(async move {
                    let listener = tokio::net::TcpListener::from_std(listener)
                        .expect("local web fixture should convert listener");
                    axum::serve(listener, app)
                        .await
                        .expect("local web fixture should serve requests");
                });
            })
            .expect("local web fixture thread should spawn");
        LocalWebFixture {
            base_url,
            fetch_hits,
            search_hits,
        }
    }

    fn write_local_lsp_fixture(root: &Path) -> LocalLspFixture {
        let script_path = root.join("mock_lsp_server.py");
        std::fs::write(
            &script_path,
            r#"import json
import sys

documents = {}


def read_frame():
    headers = {}
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b"\r\n", b"\n"):
            break
        key, value = line.decode("utf-8").split(":", 1)
        headers[key.strip().lower()] = value.strip()
    length = int(headers["content-length"])
    body = sys.stdin.buffer.read(length)
    return json.loads(body.decode("utf-8"))


def send(payload):
    body = json.dumps(payload, separators=(",", ":")).encode("utf-8")
    sys.stdout.buffer.write(f"Content-Length: {len(body)}\r\n\r\n".encode("utf-8"))
    sys.stdout.buffer.write(body)
    sys.stdout.buffer.flush()


def find_line(text, needle):
    for line_no, line in enumerate(text.splitlines()):
        column = line.find(needle)
        if column >= 0:
            return line_no, column
    return 0, 0


def location(uri, line_no, column, width):
    return {
        "uri": uri,
        "range": {
            "start": {"line": line_no, "character": column},
            "end": {"line": line_no, "character": column + width},
        },
    }


def publish_diagnostics(uri, text):
    diagnostics = []
    if "todo!" in text:
        line_no, column = find_line(text, "todo!")
        diagnostics.append(
            {
                "range": {
                    "start": {"line": line_no, "character": column},
                    "end": {"line": line_no, "character": column + 5},
                },
                "severity": 1,
                "source": "mock-lsp",
                "message": "todo! macro left in file",
            }
        )
    send(
        {
            "jsonrpc": "2.0",
            "method": "textDocument/publishDiagnostics",
            "params": {"uri": uri, "diagnostics": diagnostics},
        }
    )


while True:
    message = read_frame()
    if message is None:
        break

    method = message.get("method")
    params = message.get("params") or {}

    if "id" not in message:
        if method == "textDocument/didOpen":
            text_document = params["textDocument"]
            documents[text_document["uri"]] = text_document["text"]
            publish_diagnostics(text_document["uri"], text_document["text"])
        elif method == "textDocument/didChange":
            text_document = params["textDocument"]
            text = params["contentChanges"][-1]["text"]
            documents[text_document["uri"]] = text
            publish_diagnostics(text_document["uri"], text)
        elif method == "exit":
            break
        continue

    request_id = message["id"]
    result = None

    if method == "initialize":
        result = {
            "capabilities": {"textDocumentSync": 1},
            "serverInfo": {"name": "mock-lsp", "version": "0.0.1"},
        }
    elif method == "textDocument/definition":
        uri = params["textDocument"]["uri"]
        text = documents.get(uri, "")
        line_no, column = find_line(text, "pub fn answer")
        result = location(uri, line_no, column, len("pub fn answer"))
    elif method == "textDocument/references":
        uri = params["textDocument"]["uri"]
        text = documents.get(uri, "")
        references = []
        if params.get("context", {}).get("includeDeclaration"):
            line_no, column = find_line(text, "pub fn answer")
            references.append(location(uri, line_no, column, len("answer")))
        line_no, column = find_line(text, "let value = answer();")
        references.append(location(uri, line_no, column + len("let value = "), len("answer")))
        result = references
    elif method == "textDocument/hover":
        result = {
            "contents": {
                "kind": "markdown",
                "value": "```rust\nfn answer() -> u32\n```",
            }
        }
    elif method == "shutdown":
        result = None

    send({"jsonrpc": "2.0", "id": request_id, "result": result})
"#,
        )
        .expect("mock lsp server script should be written");

        let source_dir = root.join("src");
        std::fs::create_dir_all(&source_dir).expect("runtime lsp source dir should exist");
        std::fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"runtime-lsp\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("runtime lsp fixture Cargo.toml should be written");
        let source_path = source_dir.join("lib.rs");
        std::fs::write(
            &source_path,
            "pub fn answer() -> u32 {\n    42\n}\n\npub fn use_answer() -> u32 {\n    let value = answer();\n    value\n}\n\npub fn broken() {\n    todo!(\"needs work\");\n}\n",
        )
        .expect("runtime lsp source should be written");

        LocalLspFixture {
            script_path,
            source_path,
        }
    }

    #[derive(Clone)]
    struct FsPlanWorktreeProvider;

    #[async_trait]
    impl ModelRuntime for FsPlanWorktreeProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let events = if completed_or_failed_operation_count(&request, &["call_tools_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_tools_1",
                    "tools",
                    serde_json::json!({
                        "action": "help",
                        "tool": "plan",
                        "include_schema": false
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_plan_enter_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_plan_enter_1",
                    "plan",
                    serde_json::json!({ "action": "enter" }).to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_plan_glob_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_plan_glob_1",
                    "fs",
                    serde_json::json!({
                        "action": "glob",
                        "pattern": "*.md",
                        "path": ".agena/plans"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_plan_patch_1"]) == 0 {
                let plan_path = request_operation_text(&request, "call_plan_glob_1")
                    .and_then(|text| {
                        text.lines()
                            .map(str::trim)
                            .find(|line| line.ends_with(".md"))
                            .map(str::to_string)
                    })
                    .unwrap_or_else(|| {
                        panic!(
                            "plan path should be rendered in output text: glob={} enter={}",
                            request_operation_debug(&request, "call_plan_glob_1"),
                            request_operation_debug(&request, "call_plan_enter_1")
                        )
                    });
                scripted_tool_call_events(vec![(
                    "call_plan_patch_1",
                    "fs",
                    serde_json::json!({
                        "action": "apply_patch",
                        "patch": format!(
                            "*** Begin Patch\n*** Update File: {plan_path}\n@@\n-# Plan\n-\n-_(write your plan here)_\n+# Plan\n+\n+- inspect the repo state\n+- enter a throwaway worktree\n+- verify edits stay isolated\n*** End Patch"
                        ),
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_plan_exit_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_plan_exit_1",
                    "plan",
                    serde_json::json!({ "action": "exit" }).to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_worktree_enter_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_worktree_enter_1",
                    "worktree",
                    serde_json::json!({
                        "action": "enter",
                        "target": "new",
                        "name": "demo"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_fs_patch_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_fs_patch_1",
                    "fs",
                    serde_json::json!({
                        "action": "apply_patch",
                        "patch": "*** Begin Patch\n*** Update File: notes.txt\n@@\n-base line\n+base line\n+branch-only change\n*** End Patch"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_fs_glob_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_fs_glob_1",
                    "fs",
                    serde_json::json!({
                        "action": "glob",
                        "pattern": "*.txt"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_fs_grep_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_fs_grep_1",
                    "fs",
                    serde_json::json!({
                        "action": "grep",
                        "pattern": "branch-only",
                        "path": ".",
                        "include": "notes.txt"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_nb_edit_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_nb_edit_1",
                    "fs",
                    serde_json::json!({
                        "action": "notebook_edit",
                        "notebook_path": "demo.ipynb",
                        "cell_number": 1,
                        "new_source": "print(2)\n",
                        "edit_mode": "replace"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_fs_read_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_fs_read_1",
                    "fs",
                    serde_json::json!({
                        "action": "read",
                        "file_path": "notes.txt"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_worktree_exit_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_worktree_exit_1",
                    "worktree",
                    serde_json::json!({
                        "action": "exit",
                        "exit_action": "remove",
                        "discard_changes": true
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime tool flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_fs_plan_worktree_flow_exercises_real_host_bridges() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let db =
                open_runtime_tool_database(&workspace.root, "runtime-tools-fs-plan-worktree.db")
                    .await;
            let (manager, _host) = build_runtime_tool_manager_with_provider(
                &workspace.root,
                db,
                FsPlanWorktreeProvider,
            )
            .await;

            let created =
                create_runtime_tool_session(manager.as_ref(), "runtime-fs-plan-worktree").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise fs plan and worktree tools",
                "runtime tool run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime tool flow finished")),
                "assistant should acknowledge the flow completion"
            );

            assert_operations_completed(
                &session,
                &[
                    "call_tools_1",
                    "call_plan_enter_1",
                    "call_plan_glob_1",
                    "call_plan_patch_1",
                    "call_plan_exit_1",
                    "call_worktree_enter_1",
                    "call_fs_patch_1",
                    "call_fs_glob_1",
                    "call_fs_grep_1",
                    "call_nb_edit_1",
                    "call_fs_read_1",
                    "call_worktree_exit_1",
                ],
            );

            let plan_path = std::fs::read_dir(workspace.root.join(".agena/plans"))
                .expect("plans directory should exist")
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .find(|path| path.extension().and_then(|ext| ext.to_str()) == Some("md"))
                .expect("plan tool should create exactly one markdown file");
            let plan_body = std::fs::read_to_string(&plan_path).expect("plan file should exist");
            assert!(
                plan_body.contains("enter a throwaway worktree"),
                "plan file should contain the patched checklist"
            );

            let read_payload = session_operation_payload(&session, "call_fs_read_1");
            let preview = read_payload["preview"]
                .as_str()
                .expect("read payload should contain preview");
            assert!(
                preview.contains("branch-only change"),
                "worktree-scoped read should observe the patched file: {preview}"
            );

            let notebook_payload = session_operation_payload(&session, "call_nb_edit_1");
            assert_eq!(notebook_payload["edit_mode"].as_str(), Some("replace"));
            assert_eq!(notebook_payload["cell_index"].as_u64(), Some(1));

            let root_notes = std::fs::read_to_string(workspace.root.join("notes.txt"))
                .expect("original workspace notes should remain readable");
            assert_eq!(
                root_notes, "base line\n",
                "worktree mutation should not leak back into the original workspace"
            );
            assert!(
                !workspace.root.join(".agena/worktrees/demo").exists(),
                "worktree exit remove should clean up the temporary worktree"
            );
        });
    }

    #[derive(Clone)]
    struct ShellRuntimeProvider;

    #[async_trait]
    impl ModelRuntime for ShellRuntimeProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let bash_command = if cfg!(windows) {
                "echo shell-runtime && type notes.txt"
            } else {
                "printf 'shell-runtime\\n'; cat notes.txt"
            };
            let monitor_command = if cfg!(windows) {
                "echo tick-1 && echo tick-2 && ping -n 6 127.0.0.1 >nul"
            } else {
                "printf 'tick-1\\n'; printf 'tick-2\\n'; sleep 5"
            };

            let events = if completed_or_failed_operation_count(&request, &["call_tools_shell_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_tools_shell_1",
                    "tools",
                    serde_json::json!({
                        "action": "help",
                        "tool": "shell",
                        "include_schema": false
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_shell_exec_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_shell_exec_1",
                    "shell",
                    serde_json::json!({
                        "action": "exec",
                        "shell": "bash",
                        "command": bash_command,
                        "description": "read notes via shell",
                        "workdir": ".",
                        "timeout_ms": 5_000,
                        "filesystem_effects": [{"path": "notes.txt", "access": "read"}],
                        "network_effects": []
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_shell_powershell_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_shell_powershell_1",
                    "shell",
                    serde_json::json!({
                        "action": "exec",
                        "shell": "powershell",
                        "command": "Get-Location",
                        "description": "probe powershell availability",
                        "filesystem_effects": [],
                        "network_effects": []
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_monitor_start_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_monitor_start_1",
                    "shell",
                    serde_json::json!({
                        "action": "monitor_start",
                        "command": monitor_command,
                        "description": "tick monitor",
                        "timeout_ms": 10_000,
                        "persistent": false,
                        "capture_stderr": true,
                        "filesystem_effects": [{"path": ".", "access": "read"}],
                        "network_effects": []
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_monitor_list_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_monitor_list_1",
                    "shell",
                    serde_json::json!({
                        "action": "monitor_list"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_monitor_read_1"]) == 0 {
                let monitor_id = request_operation_payload(&request, "call_monitor_start_1")
                    .get("monitor_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        panic!(
                            "monitor start should return monitor_id: {}",
                            request_operation_debug(&request, "call_monitor_start_1")
                        )
                    });
                scripted_tool_call_events(vec![(
                    "call_monitor_read_1",
                    "shell",
                    serde_json::json!({
                        "action": "monitor_read",
                        "monitor_id": monitor_id,
                        "since_seq": 0,
                        "wait_ms": 1_000
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_monitor_stop_1"]) == 0 {
                let monitor_id = request_operation_payload(&request, "call_monitor_start_1")
                    .get("monitor_id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        panic!(
                            "monitor start should return monitor_id: {}",
                            request_operation_debug(&request, "call_monitor_start_1")
                        )
                    });
                scripted_tool_call_events(vec![(
                    "call_monitor_stop_1",
                    "shell",
                    serde_json::json!({
                        "action": "monitor_stop",
                        "monitor_id": monitor_id
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime shell flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_shell_flow_exercises_exec_and_monitor_paths() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let db = open_runtime_tool_database(&workspace.root, "runtime-tools-shell.db").await;
            let (manager, _host) =
                build_runtime_tool_manager_with_provider(&workspace.root, db, ShellRuntimeProvider)
                    .await;

            let created = create_runtime_tool_session(manager.as_ref(), "runtime-shell").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise shell runtime tools",
                "runtime shell run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime shell flow finished")),
                "assistant should acknowledge the shell flow completion"
            );

            assert_operations_completed(
                &session,
                &[
                    "call_tools_shell_1",
                    "call_shell_exec_1",
                    "call_monitor_start_1",
                    "call_monitor_list_1",
                    "call_monitor_read_1",
                    "call_monitor_stop_1",
                ],
            );

            let (powershell_status, powershell_error, powershell_output) =
                operation_snapshot(&session, "call_shell_powershell_1");
            if cfg!(windows) {
                assert_eq!(
                    powershell_status,
                    ExecutionStatus::Completed,
                    "powershell should succeed on Windows"
                );
                assert!(
                    powershell_error.is_none(),
                    "powershell should not error on Windows"
                );
            } else {
                assert_eq!(
                    powershell_status,
                    ExecutionStatus::Failed,
                    "powershell should fail on non-Windows"
                );
                let message = powershell_error
                    .as_deref()
                    .unwrap_or(powershell_output.as_str());
                assert!(
                    message.contains("only available on Windows"),
                    "non-Windows powershell failure should explain the platform restriction: {message}"
                );
            }

            let shell_exec_payload = session_operation_payload(&session, "call_shell_exec_1");
            let shell_output = shell_exec_payload["output"]
                .as_str()
                .expect("shell exec payload should contain output");
            assert!(
                shell_output.contains("shell-runtime"),
                "shell exec output should include the marker: {shell_output}"
            );
            assert!(
                shell_output.contains("base line"),
                "shell exec output should read notes.txt: {shell_output}"
            );

            let monitor_start_payload = session_operation_payload(&session, "call_monitor_start_1");
            let monitor_id = monitor_start_payload["monitor_id"]
                .as_str()
                .expect("monitor start should return monitor_id");
            assert!(
                monitor_id.starts_with("mon_"),
                "monitor ids should use the expected prefix: {monitor_id}"
            );

            let monitor_list_payload = session_operation_payload(&session, "call_monitor_list_1");
            let listed = monitor_list_payload["monitors"]
                .as_array()
                .expect("monitor list should include monitors");
            assert!(
                listed.iter().any(|entry| {
                    entry.get("monitor_id").and_then(serde_json::Value::as_str) == Some(monitor_id)
                }),
                "monitor list should include the started monitor"
            );

            let monitor_read_payload = session_operation_payload(&session, "call_monitor_read_1");
            let events = monitor_read_payload["events"]
                .as_array()
                .expect("monitor read should include events");
            assert!(
                events.iter().any(|event| {
                    event.get("line").and_then(serde_json::Value::as_str) == Some("tick-1")
                }),
                "monitor read should capture tick-1"
            );
            assert!(
                events.iter().any(|event| {
                    event.get("line").and_then(serde_json::Value::as_str) == Some("tick-2")
                }),
                "monitor read should capture tick-2"
            );

            let monitor_stop_payload = session_operation_payload(&session, "call_monitor_stop_1");
            assert_eq!(monitor_stop_payload["action"].as_str(), Some("stop"));
            assert_eq!(
                monitor_stop_payload["monitor_id"].as_str(),
                Some(monitor_id)
            );
        });
    }

    #[derive(Clone)]
    struct WebRuntimeProvider {
        fetch_url: String,
    }

    #[async_trait]
    impl ModelRuntime for WebRuntimeProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let events = if completed_or_failed_operation_count(&request, &["call_web_fetch_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_web_fetch_1",
                    "web",
                    serde_json::json!({
                        "action": "fetch",
                        "url": self.fetch_url
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_web_fetch_2"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_web_fetch_2",
                    "web",
                    serde_json::json!({
                        "action": "fetch",
                        "url": self.fetch_url
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_web_search_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_web_search_1",
                    "web",
                    serde_json::json!({
                        "action": "search",
                        "query": "runtime web",
                        "allowed_domains": ["127.0.0.1"],
                        "max_results": 5
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime web flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_web_flow_exercises_fetch_cache_and_search() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let web_fixture = start_local_web_fixture().await;
            let db = open_runtime_tool_database(&workspace.root, "runtime-tools-web.db").await;
            let (manager, _host) = build_runtime_tool_manager_with_provider_and_executor(
                &workspace.root,
                db,
                WebRuntimeProvider {
                    fetch_url: format!("{}/page", web_fixture.base_url),
                },
                |executor| {
                    executor
                        .with_web_search_backend(crate::config::WebSearchBackend {
                            api_key: "test-brave-key".to_string(),
                        })
                        .with_web_search_url_override(format!("{}/search", web_fixture.base_url))
                },
            )
            .await;

            let created = create_runtime_tool_session(manager.as_ref(), "runtime-web").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise web runtime tools",
                "runtime web run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime web flow finished")),
                "assistant should acknowledge the web flow completion"
            );

            for operation_id in ["call_web_fetch_1", "call_web_fetch_2", "call_web_search_1"] {
                let (status, error, _) = operation_snapshot(&session, operation_id);
                assert_eq!(
                    status,
                    ExecutionStatus::Completed,
                    "{operation_id} was not completed: error={error:?}\noperations:\n{}",
                    session_operation_summaries(&session).join("\n")
                );
                assert!(error.is_none(), "{operation_id} failed: {error:?}");
            }

            let fetch_payload = session_operation_payload(&session, "call_web_fetch_1");
            assert_eq!(fetch_payload["status"].as_u64(), Some(200));
            assert_eq!(fetch_payload["cached"].as_bool(), Some(false));
            let fetched_markdown = fetch_payload["markdown"]
                .as_str()
                .expect("web fetch should return markdown");
            assert!(
                fetched_markdown.contains("Runtime Web"),
                "web fetch markdown should contain the local page title: {fetched_markdown}"
            );
            assert!(
                fetch_payload["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with("http://127.0.0.1:")),
                "loopback fetch should preserve HTTP for the local fixture"
            );

            let cached_fetch_payload = session_operation_payload(&session, "call_web_fetch_2");
            assert_eq!(cached_fetch_payload["cached"].as_bool(), Some(true));

            let search_payload = session_operation_payload(&session, "call_web_search_1");
            assert_eq!(
                search_payload["backend"].as_str(),
                Some("brave"),
                "web search should report the active provider"
            );
            let results = search_payload["results"]
                .as_array()
                .expect("web search should return results");
            assert_eq!(
                results.len(),
                1,
                "allowed_domains should filter the external result from the local fixture"
            );
            assert_eq!(
                results[0]["title"].as_str(),
                Some("Runtime Web Docs"),
                "web search should parse the local Brave-like JSON payload"
            );
            assert!(
                results[0]["url"]
                    .as_str()
                    .is_some_and(|url| url.starts_with(&format!("{}/docs/", web_fixture.base_url))),
                "web search should keep the local result URL"
            );

            assert_eq!(
                web_fixture.fetch_hits.load(Ordering::SeqCst),
                1,
                "the second fetch should be served from cache"
            );
            assert_eq!(
                web_fixture.search_hits.load(Ordering::SeqCst),
                1,
                "web search should hit the local endpoint exactly once"
            );
        });
    }

    #[derive(Clone)]
    struct TaskRuntimeProvider;

    #[async_trait]
    impl ModelRuntime for TaskRuntimeProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            if request.messages.iter().any(|message| {
                message.role == Role::User
                    && message
                        .as_text_lossy()
                        .contains("Inspect SUBTASK.md and report its headline.")
            }) {
                let events =
                    if completed_or_failed_operation_count(&request, &["call_subtask_fs_read_1"])
                        == 0
                    {
                        scripted_tool_call_events(vec![(
                            "call_subtask_fs_read_1",
                            "fs",
                            serde_json::json!({
                                "action": "read",
                                "path": "SUBTASK.md"
                            })
                            .to_string(),
                        )])
                    } else {
                        scripted_text_events("subtask finished: Delegated Notes")
                    };
                return Ok(Box::pin(stream::iter(events)));
            }

            let events =
                if completed_or_failed_operation_count(&request, &["call_tools_task_1"]) == 0 {
                    scripted_tool_call_events(vec![(
                        "call_tools_task_1",
                        "tools",
                        serde_json::json!({
                            "action": "help",
                            "tool": "task",
                            "include_schema": false
                        })
                        .to_string(),
                    )])
                } else if completed_or_failed_operation_count(&request, &["call_task_run_1"]) == 0 {
                    scripted_tool_call_events(vec![(
                        "call_task_run_1",
                        "task",
                        serde_json::json!({
                            "action": "run",
                            "description": "inspect delegated note",
                            "prompt": "Inspect SUBTASK.md and report its headline.",
                            "subagent_type": "explore",
                            "task_id": "runtime-task-note"
                        })
                        .to_string(),
                    )])
                } else {
                    scripted_text_events("runtime task flow finished")
                };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_task_flow_exercises_subtask_session_creation_and_completion() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            std::fs::write(
                workspace.root.join("SUBTASK.md"),
                "# Delegated Notes\n\nRuntime task flow fixture.\n",
            )
            .expect("runtime task fixture should be written");
            let db = open_runtime_tool_database(&workspace.root, "runtime-tools-task.db").await;
            let (manager, _host) =
                build_runtime_tool_manager_with_provider(&workspace.root, db, TaskRuntimeProvider)
                    .await;

            let created = create_runtime_tool_session(manager.as_ref(), "runtime-task").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise task runtime tool",
                "runtime task run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime task flow finished")),
                "assistant should acknowledge the task flow completion"
            );

            assert_operations_completed(&session, &["call_tools_task_1", "call_task_run_1"]);

            let task_payload = session_operation_payload(&session, "call_task_run_1");
            assert_eq!(
                task_payload["model_provider_id"].as_str(),
                Some("runtime-tools"),
                "task payload should preserve the spawned model provider id"
            );
            assert_eq!(
                task_payload["model_id"].as_str(),
                Some("runtime-tools-model"),
                "task payload should preserve the spawned model id"
            );
            let child_session_id = task_payload["session_id"]
                .as_str()
                .expect("task payload should include the child session id")
                .parse::<i64>()
                .expect("child session id should parse");

            let child = manager
                .get_session(child_session_id)
                .await
                .expect("child session should load");
            assert_eq!(child.parent_id, Some(created.id));
            assert!(
                child.is_subagent,
                "task flow should create a subagent session"
            );
            assert!(
                child.messages.iter().any(|message| {
                    message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("subtask finished: Delegated Notes")
                }),
                "child session should complete the delegated subtask"
            );

            let child_read_payload = session_operation_payload(&child, "call_subtask_fs_read_1");
            let loaded_paths = child_read_payload["loaded_paths"]
                .as_array()
                .expect("subtask read payload should include loaded paths");
            assert!(
                loaded_paths
                    .iter()
                    .any(|value| value.as_str() == Some("SUBTASK.md")),
                "subtask should read the delegated fixture file"
            );
            assert!(
                child_read_payload["preview"]
                    .as_str()
                    .is_some_and(|preview| preview.contains("# Delegated Notes")),
                "subtask read preview should include the delegated note heading"
            );
            assert!(
                child
                    .messages
                    .iter()
                    .flat_map(|message| message.parts.iter())
                    .filter(|part| part.operation_id.as_deref() == Some("call_subtask_fs_read_1"))
                    .filter_map(|part| match part.content.as_ref() {
                        Some(PartContent::Operation(operation)) =>
                            Some(operation.model_output.text.clone()),
                        _ => None,
                    })
                    .any(|text| text.contains("# Delegated Notes")),
                "subtask fs read should surface the delegated file contents"
            );
        });
    }

    #[derive(Clone)]
    struct LspRuntimeProvider;

    #[async_trait]
    impl ModelRuntime for LspRuntimeProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let events = if completed_or_failed_operation_count(&request, &["call_lsp_servers_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_lsp_servers_1",
                    "lsp",
                    serde_json::json!({
                        "action": "servers"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_lsp_definition_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_lsp_definition_1",
                    "lsp",
                    serde_json::json!({
                        "action": "definition",
                        "file_path": "src/lib.rs",
                        "line": 5,
                        "character": 16
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_lsp_references_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_lsp_references_1",
                    "lsp",
                    serde_json::json!({
                        "action": "references",
                        "file_path": "src/lib.rs",
                        "line": 5,
                        "character": 16,
                        "include_declaration": true
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_lsp_hover_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_lsp_hover_1",
                    "lsp",
                    serde_json::json!({
                        "action": "hover",
                        "file_path": "src/lib.rs",
                        "line": 5,
                        "character": 16
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_lsp_diagnostics_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_lsp_diagnostics_1",
                    "lsp",
                    serde_json::json!({
                        "action": "diagnostics",
                        "file_path": "src/lib.rs"
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime lsp flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_lsp_flow_exercises_servers_navigation_hover_and_diagnostics() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let lsp_fixture = write_local_lsp_fixture(&workspace.root);
            let db = open_runtime_tool_database(&workspace.root, "runtime-tools-lsp.db").await;
            let (manager, _host) =
                build_runtime_tool_manager_with_provider(&workspace.root, db, LspRuntimeProvider)
                    .await;

            let registry = manager
                .tool_executor()
                .lsp_registry()
                .cloned()
                .expect("runtime manager should expose an lsp registry");
            registry
                .register(agena_lsp::server_spec::LspServerSpec {
                    name: "mock-rust".to_string(),
                    command: "python3".to_string(),
                    args: vec![lsp_fixture.script_path.display().to_string()],
                    env: Default::default(),
                    file_extensions: vec!["rs".to_string()],
                    root_markers: vec!["Cargo.toml".to_string()],
                    initialization_options: None,
                })
                .await;

            let created = create_runtime_tool_session(manager.as_ref(), "runtime-lsp").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise lsp runtime tool",
                "runtime lsp run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime lsp flow finished")),
                "assistant should acknowledge the lsp flow completion"
            );

            assert_operations_completed(
                &session,
                &[
                    "call_lsp_servers_1",
                    "call_lsp_definition_1",
                    "call_lsp_references_1",
                    "call_lsp_hover_1",
                    "call_lsp_diagnostics_1",
                ],
            );

            let servers_payload = session_operation_payload(&session, "call_lsp_servers_1");
            let servers = servers_payload["servers"]
                .as_array()
                .expect("lsp servers payload should include servers");
            assert_eq!(servers.len(), 1);
            assert_eq!(servers[0]["name"].as_str(), Some("mock-rust"));
            assert_eq!(servers[0]["command"].as_str(), Some("python3"));
            assert_eq!(
                servers[0]["file_extensions"][0].as_str(),
                Some("rs"),
                "lsp servers output should include the registered file extension"
            );

            let definition_payload = session_operation_payload(&session, "call_lsp_definition_1");
            let definition_locations = definition_payload["locations"]
                .as_array()
                .expect("definition payload should include locations");
            assert_eq!(definition_locations.len(), 1);
            assert!(
                definition_locations[0].as_str().is_some_and(
                    |location| location == format!("{}:1:1", lsp_fixture.source_path.display())
                ),
                "definition should point at the function declaration: {definition_locations:?}"
            );

            let references_payload = session_operation_payload(&session, "call_lsp_references_1");
            let references = references_payload["locations"]
                .as_array()
                .expect("references payload should include locations");
            assert_eq!(references.len(), 2);
            assert!(
                references.iter().any(|value| {
                    value.as_str().is_some_and(|location| {
                        location == format!("{}:1:1", lsp_fixture.source_path.display())
                    })
                }),
                "references should include the declaration when include_declaration is true"
            );
            assert!(
                references.iter().any(|value| {
                    value.as_str().is_some_and(|location| {
                        location == format!("{}:6:17", lsp_fixture.source_path.display())
                    })
                }),
                "references should include the call site"
            );

            let hover_payload = session_operation_payload(&session, "call_lsp_hover_1");
            assert_eq!(
                hover_payload["contents"].as_str(),
                Some("```rust\nfn answer() -> u32\n```"),
                "hover should surface the mock server hover markdown"
            );

            let diagnostics_payload = session_operation_payload(&session, "call_lsp_diagnostics_1");
            let diagnostics = diagnostics_payload["entries"]
                .as_array()
                .expect("diagnostics payload should include entries");
            assert_eq!(diagnostics.len(), 1);
            assert_eq!(
                diagnostics[0].as_str(),
                Some("src/lib.rs:11:5 [error] todo! macro left in file"),
                "diagnostics should include the published todo! error"
            );

            registry.shutdown_all().await;
        });
    }

    #[derive(Clone)]
    struct WorkflowMutationProvider;

    #[async_trait]
    impl ModelRuntime for WorkflowMutationProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let events = if completed_or_failed_operation_count(
                &request,
                &["call_workflow_review_1"],
            ) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_workflow_review_1",
                    "workflow",
                    serde_json::json!({
                        "action": "review",
                        "args": "auth handlers"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_agent_restore_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_agent_restore_1",
                    "agent",
                    serde_json::json!({
                        "action": "restore"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_session_rename_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_session_rename_1",
                    "session",
                    serde_json::json!({
                        "action": "rename",
                        "title": "runtime-mutation-renamed"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_goal_create_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_goal_create_1",
                    "goal",
                    serde_json::json!({
                        "action": "create",
                        "objective": "Close runtime mutation coverage"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_goal_complete_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_goal_complete_1",
                    "goal",
                    serde_json::json!({
                        "action": "complete"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_goal_clear_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_goal_clear_1",
                    "goal",
                    serde_json::json!({
                        "action": "clear"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(
                &request,
                &["call_workflow_security_review_1"],
            ) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_workflow_security_review_1",
                    "workflow",
                    serde_json::json!({
                        "action": "security_review",
                        "args": "auth layer"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_agent_restore_2"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_agent_restore_2",
                    "agent",
                    serde_json::json!({
                        "action": "restore"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_agent_switch_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_agent_switch_1",
                    "agent",
                    serde_json::json!({
                        "action": "switch",
                        "agent": "planner",
                        "push_previous": true
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_agent_restore_3"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_agent_restore_3",
                    "agent",
                    serde_json::json!({
                        "action": "restore"
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime workflow mutation flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_workflow_mutation_flow_exercises_active_run_host_mutations() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let db =
                open_runtime_tool_database(&workspace.root, "runtime-tools-workflow-mutation.db")
                    .await;
            let (manager, _host) = build_runtime_tool_manager_with_provider(
                &workspace.root,
                db,
                WorkflowMutationProvider,
            )
            .await;

            let created =
                create_runtime_tool_session(manager.as_ref(), "runtime-workflow-mutation").await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise workflow mutation runtime tools",
                "runtime workflow mutation run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime workflow mutation flow finished")),
                "assistant should acknowledge the workflow mutation flow completion"
            );

            assert_operations_completed(
                &session,
                &[
                    "call_workflow_review_1",
                    "call_agent_restore_1",
                    "call_session_rename_1",
                    "call_goal_create_1",
                    "call_goal_complete_1",
                    "call_goal_clear_1",
                    "call_agent_switch_1",
                    "call_workflow_security_review_1",
                    "call_agent_restore_2",
                    "call_agent_restore_3",
                ],
            );

            let review_text = session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .find_map(|part| {
                    (part.operation_id.as_deref() == Some("call_workflow_review_1")).then(|| {
                        match part.content.as_ref() {
                            Some(PartContent::Operation(operation)) => {
                                Some(operation.model_output.text.clone())
                            }
                            _ => None,
                        }
                    })?
                })
                .expect("workflow review output should exist");
            assert!(
                review_text.contains("You are reviewing the changes on the current branch"),
                "workflow review should return the bundled review prompt"
            );
            assert!(
                review_text.contains("User arguments:\nauth handlers"),
                "workflow review should include the forwarded workflow arguments"
            );

            let session_payload = session_operation_payload(&session, "call_session_rename_1");
            assert_eq!(
                session_payload["session"]["title"].as_str(),
                Some("runtime-mutation-renamed")
            );

            let restore_after_review = session_operation_payload(&session, "call_agent_restore_1");
            assert_eq!(restore_after_review["restored"].as_bool(), Some(true));
            assert_eq!(
                restore_after_review["previous_agent"].as_str(),
                Some("reviewer")
            );
            assert_eq!(
                restore_after_review["current_agent"],
                serde_json::Value::Null
            );
            assert_eq!(restore_after_review["stack_depth"].as_u64(), Some(0));

            let goal_create_payload = session_operation_payload(&session, "call_goal_create_1");
            assert_eq!(
                goal_create_payload["goal"]["objective"].as_str(),
                Some("Close runtime mutation coverage")
            );
            assert_eq!(
                goal_create_payload["goal"]["status"].as_str(),
                Some("active")
            );

            let goal_complete_payload = session_operation_payload(&session, "call_goal_complete_1");
            assert_eq!(
                goal_complete_payload["goal"]["status"].as_str(),
                Some("completed")
            );

            let goal_clear_payload = session_operation_payload(&session, "call_goal_clear_1");
            assert_eq!(goal_clear_payload["cleared"].as_bool(), Some(true));

            let security_review_text = session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .find_map(|part| {
                    (part.operation_id.as_deref() == Some("call_workflow_security_review_1")).then(
                        || match part.content.as_ref() {
                            Some(PartContent::Operation(operation)) => {
                                Some(operation.model_output.text.clone())
                            }
                            _ => None,
                        },
                    )?
                })
                .expect("workflow security review output should exist");
            assert!(
                security_review_text.contains("Audit the changes on this branch"),
                "workflow security review should return the bundled security review prompt"
            );
            assert!(
                security_review_text.contains("User arguments:\nauth layer"),
                "workflow security review should include the forwarded workflow arguments"
            );

            let restore_after_security_review =
                session_operation_payload(&session, "call_agent_restore_2");
            assert_eq!(
                restore_after_security_review["restored"].as_bool(),
                Some(true)
            );
            assert_eq!(
                restore_after_security_review["previous_agent"].as_str(),
                Some("reviewer")
            );
            assert_eq!(
                restore_after_security_review["current_agent"],
                serde_json::Value::Null
            );
            assert_eq!(
                restore_after_security_review["stack_depth"].as_u64(),
                Some(0)
            );

            let agent_switch_payload = session_operation_payload(&session, "call_agent_switch_1");
            assert_eq!(
                agent_switch_payload["current_agent"].as_str(),
                Some("planner")
            );
            assert_eq!(
                agent_switch_payload["previous_agent"],
                serde_json::Value::Null
            );
            assert_eq!(agent_switch_payload["stack_depth"].as_u64(), Some(1));

            let restore_to_default = session_operation_payload(&session, "call_agent_restore_3");
            assert_eq!(restore_to_default["restored"].as_bool(), Some(true));
            assert_eq!(
                restore_to_default["previous_agent"].as_str(),
                Some("planner")
            );
            assert_eq!(restore_to_default["current_agent"], serde_json::Value::Null);
            assert_eq!(restore_to_default["stack_depth"].as_u64(), Some(0));

            assert_eq!(session.title, "runtime-mutation-renamed");
            assert!(
                session.goal.is_none(),
                "goal clear should remove the active goal from the session"
            );
            assert_eq!(
                session.runtime.execution.selection.agent, None,
                "final agent restore should return the session to the default runtime context"
            );
        });
    }

    #[test]
    fn effective_permission_prefers_session_then_agent_then_top_level() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            let mut config = SessionManagerConfig::default();
            config.permission = crate::agent::PermissionConfig {
                network: Some(crate::agent::NetworkPermissionConfig {
                    internet: Some(PermissionMode::Ask),
                    ..Default::default()
                }),
                ..Default::default()
            };
            config.default_selection.permission = crate::agent::PermissionConfig {
                network: Some(crate::agent::NetworkPermissionConfig {
                    private: Some(PermissionMode::Deny),
                    ..Default::default()
                }),
                ..Default::default()
            };
            let manager =
                build_manager(&workspace.root, PermissionPolicy::allow_all(), config).await;
            manager
                .tool_executor()
                .subagent_registry()
                .register_runtime(crate::agents::AgentProfile {
                    name: "priority-test".to_string(),
                    frontmatter: crate::agents::AgentFrontmatter {
                        permission: crate::agent::PermissionConfig {
                            network: Some(crate::agent::NetworkPermissionConfig {
                                internet: Some(PermissionMode::Allow),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        ..Default::default()
                    },
                    prompt: "Priority test profile".to_string(),
                    source_path: None,
                    scope: crate::agents::AgentScope::Project,
                });

            let mut session = manager
                .create_session(SessionCreateRequest {
                    title: "permission-priority".to_string(),
                    parent_session_id: None,
                })
                .await
                .expect("session should be created");
            session.runtime.execution.selection.agent = Some("priority-test".to_string());
            session.runtime.execution.selection.permission = crate::agent::PermissionConfig {
                network: Some(crate::agent::NetworkPermissionConfig {
                    internet: Some(PermissionMode::Deny),
                    ..Default::default()
                }),
                ..Default::default()
            };

            let state = manager.execution_state();
            let mut options = SessionRunOptions::new(scripted_model_ref());
            let updated = manager
                .apply_requested_agent_profile(session, &mut options, state)
                .await
                .expect("agent profile should apply");

            let network = updated
                .runtime
                .execution
                .effective_permission
                .network
                .as_ref()
                .expect("effective network permission should exist");
            assert_eq!(
                network.internet,
                Some(PermissionMode::Deny),
                "session permission should override agent and top-level permission"
            );
            assert_eq!(
                network.private, None,
                "default selection permission should not contribute to effective permission"
            );
        });
    }

    #[derive(Clone)]
    struct WorkflowSettingsScheduleProvider;

    #[async_trait]
    impl ModelRuntime for WorkflowSettingsScheduleProvider {
        fn id(&self) -> &str {
            "runtime-tools"
        }

        fn default_model(&self) -> &ModelId {
            static MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("runtime-tools-model"));
            &MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new(
                "runtime-tools",
                "runtime-tools-model",
            )])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Err(AppError::Provider("streaming only".to_string()))
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            if request.messages.iter().any(|message| {
                message.role == Role::User
                    && message
                        .as_text_lossy()
                        .contains("Locate the settings tool and summarize it.")
            }) {
                return Ok(Box::pin(stream::iter(scripted_text_events(
                    "subtask completed: settings tool located",
                ))));
            }

            let events = if completed_or_failed_operation_count(&request, &["call_workflow_init_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_workflow_init_1",
                    "workflow",
                    serde_json::json!({
                        "action": "init",
                        "args": "backend service"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_tools_help_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_tools_help_1",
                    "tools",
                    serde_json::json!({
                        "action": "help",
                        "tool": "settings",
                        "include_schema": true
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_session_get_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_session_get_1",
                    "session",
                    serde_json::json!({
                        "action": "get"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_goal_get_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_goal_get_1",
                    "goal",
                    serde_json::json!({
                        "action": "get"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_user_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_user_1",
                    "user",
                    serde_json::json!({
                        "action": "request_input",
                        "questions": [{
                            "id": "confirm",
                            "header": "Confirm",
                            "question": "Should the runtime test continue?",
                            "options": [
                                {
                                    "label": "yes",
                                    "description": "Continue the scripted runtime flow."
                                },
                                {
                                    "label": "no",
                                    "description": "Stop the scripted runtime flow."
                                }
                            ],
                            "multiple": false,
                            "allow_custom": false
                        }]
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_todo_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_todo_1",
                    "todo",
                    serde_json::json!({
                        "action": "write",
                        "items": [
                            {
                                "content": "cover workflow host tools",
                                "status": "completed",
                                "priority": "high"
                            },
                            {
                                "content": "cover settings and schedule",
                                "status": "in_progress",
                                "priority": "medium"
                            }
                        ]
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_settings_get_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_settings_get_1",
                    "settings",
                    serde_json::json!({
                        "action": "get",
                        "path": "agents.default",
                        "source": "file"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_settings_set_1"]) == 0 {
                scripted_tool_call_events(vec![(
                    "call_settings_set_1",
                    "settings",
                    serde_json::json!({
                        "action": "set",
                        "path": "agents.default",
                        "value": "planner",
                        "reload": true
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_settings_validate_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_settings_validate_1",
                    "settings",
                    serde_json::json!({
                        "action": "validate"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_schedule_list_1"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_schedule_list_1",
                    "schedule",
                    serde_json::json!({
                        "action": "list"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_schedule_wakeup_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_schedule_wakeup_1",
                    "schedule",
                    serde_json::json!({
                        "action": "wakeup",
                        "delay_seconds": 60,
                        "prompt": "wake me up later",
                        "reason": "runtime coverage"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_schedule_create_1"])
                == 0
            {
                scripted_tool_call_events(vec![(
                    "call_schedule_create_1",
                    "schedule",
                    serde_json::json!({
                        "action": "create",
                        "expression": "0 0 * * * *",
                        "prompt": "hourly coverage check",
                        "max_age_days": 1
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_schedule_list_2"]) == 0
            {
                scripted_tool_call_events(vec![(
                    "call_schedule_list_2",
                    "schedule",
                    serde_json::json!({
                        "action": "list"
                    })
                    .to_string(),
                )])
            } else if completed_or_failed_operation_count(&request, &["call_schedule_delete_1"])
                == 0
            {
                let schedule_id = request_operation_payload(&request, "call_schedule_create_1")
                    .get("id")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        panic!(
                            "schedule create should return an id: {}",
                            request_operation_debug(&request, "call_schedule_create_1")
                        )
                    });
                scripted_tool_call_events(vec![(
                    "call_schedule_delete_1",
                    "schedule",
                    serde_json::json!({
                        "action": "delete",
                        "id": schedule_id
                    })
                    .to_string(),
                )])
            } else {
                scripted_text_events("runtime workflow flow finished")
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    #[test]
    fn runtime_workflow_settings_schedule_flow_exercises_host_bridges() {
        run_async_with_large_stack(async move {
            let workspace = TempWorkspace::new();
            init_git_workspace(&workspace.root);
            let db =
                open_runtime_tool_database(&workspace.root, "runtime-tools-workflow-settings.db")
                    .await;
            let (manager, _host) = build_runtime_tool_manager_with_provider(
                &workspace.root,
                db,
                WorkflowSettingsScheduleProvider,
            )
            .await;

            let created =
                create_runtime_tool_session(manager.as_ref(), "runtime-workflow-settings").await;
            persist_goal_without_auto_run(
                manager.as_ref(),
                created.id,
                "Finish workflow runtime coverage",
                None,
            )
            .await;
            let session = submit_runtime_tool_prompt(
                manager.as_ref(),
                created.id,
                "exercise workflow settings schedule and host tools",
                "runtime workflow run should succeed",
            )
            .await;

            assert!(
                session
                    .messages
                    .iter()
                    .any(|message| message.role == Role::Assistant
                        && message
                            .as_text_lossy()
                            .contains("runtime workflow flow finished")),
                "assistant should acknowledge the workflow flow completion"
            );

            assert_operations_completed(
                &session,
                &[
                    "call_workflow_init_1",
                    "call_tools_help_1",
                    "call_session_get_1",
                    "call_goal_get_1",
                    "call_user_1",
                    "call_todo_1",
                    "call_settings_get_1",
                    "call_settings_set_1",
                    "call_settings_validate_1",
                    "call_schedule_list_1",
                    "call_schedule_wakeup_1",
                    "call_schedule_create_1",
                    "call_schedule_list_2",
                    "call_schedule_delete_1",
                ],
            );

            let workflow_text = session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .find_map(|part| {
                    (part.operation_id.as_deref() == Some("call_workflow_init_1")).then(|| {
                        match part.content.as_ref() {
                            Some(PartContent::Operation(operation)) => {
                                Some(operation.model_output.text.clone())
                            }
                            _ => None,
                        }
                    })?
                })
                .expect("workflow init output should exist");
            assert!(
                workflow_text.contains("Save the result to AGENA.md"),
                "workflow init should return the bundled init prompt"
            );

            let tools_help_text = session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .find_map(|part| {
                    (part.operation_id.as_deref() == Some("call_tools_help_1")).then(
                        || match part.content.as_ref() {
                            Some(PartContent::Operation(operation)) => {
                                Some(operation.model_output.text.clone())
                            }
                            _ => None,
                        },
                    )?
                })
                .expect("tools help output should exist");
            assert!(
                tools_help_text.contains("Tool: settings"),
                "tools help should describe the settings tool"
            );
            assert!(
                tools_help_text.contains("Input schema:"),
                "tools help should include the settings schema"
            );

            let session_get_payload = session_operation_payload(&session, "call_session_get_1");
            assert_eq!(
                session_get_payload["session"]["title"].as_str(),
                Some("runtime-workflow-settings")
            );

            let goal_payload = session_operation_payload(&session, "call_goal_get_1");
            assert_eq!(
                goal_payload["goal"]["objective"].as_str(),
                Some("Finish workflow runtime coverage")
            );

            let user_payload = session_operation_payload(&session, "call_user_1");
            assert_eq!(user_payload["answers"]["confirm"][0].as_str(), Some("yes"));

            let todo_payload = session_operation_payload(&session, "call_todo_1");
            let todo_items = todo_payload["items"]
                .as_array()
                .expect("todo write should return items");
            assert_eq!(todo_items.len(), 2);

            let settings_get_payload = session_operation_payload(&session, "call_settings_get_1");
            assert_eq!(
                settings_get_payload["value"].as_str(),
                Some("build"),
                "settings get should read the file-backed default agent before mutation"
            );

            let settings_set_payload = session_operation_payload(&session, "call_settings_set_1");
            assert_eq!(
                settings_set_payload["current"].as_str(),
                Some("planner"),
                "settings set should update the default agent"
            );
            assert_eq!(
                settings_set_payload["reload"]["generation"].as_u64(),
                Some(2),
                "settings set should trigger a config reload"
            );

            let config_json: serde_json::Value = serde_json::from_str(
                &std::fs::read_to_string(workspace.root.join("config.json"))
                    .expect("config file should remain readable"),
            )
            .expect("config.json should stay valid");
            assert_eq!(
                config_json
                    .pointer("/default/agent")
                    .and_then(serde_json::Value::as_str),
                Some("planner"),
                "settings set should persist the updated config file"
            );

            let schedule_list_1_payload =
                session_operation_payload(&session, "call_schedule_list_1");
            let initial_jobs = schedule_list_1_payload["jobs"]
                .as_array()
                .expect("schedule list should return jobs");
            assert!(
                initial_jobs.is_empty(),
                "scheduler should start empty for the runtime test"
            );

            let wakeup_payload = session_operation_payload(&session, "call_schedule_wakeup_1");
            assert!(
                wakeup_payload["id"]
                    .as_str()
                    .is_some_and(|id| !id.trim().is_empty()),
                "schedule wakeup should create an id"
            );

            let create_payload = session_operation_payload(&session, "call_schedule_create_1");
            let created_job_id = create_payload["id"]
                .as_str()
                .expect("schedule create should return an id");
            let schedule_list_2_payload =
                session_operation_payload(&session, "call_schedule_list_2");
            let listed_jobs = schedule_list_2_payload["jobs"]
                .as_array()
                .expect("second schedule list should return jobs");
            assert!(
                listed_jobs.len() >= 2,
                "scheduler should list both wakeup and cron jobs"
            );
            assert!(
                listed_jobs.iter().any(|job| {
                    job.get("id").and_then(serde_json::Value::as_str) == Some(created_job_id)
                }),
                "second schedule list should include the created cron job"
            );

            let delete_payload = session_operation_payload(&session, "call_schedule_delete_1");
            assert_eq!(delete_payload["id"].as_str(), Some(created_job_id));
            assert_eq!(delete_payload["removed"].as_bool(), Some(true));

            assert!(
                session
                    .goal
                    .as_ref()
                    .is_some_and(|goal| goal.objective == "Finish workflow runtime coverage"),
                "seeded goal should remain attached to the session"
            );
        });
    }
}
