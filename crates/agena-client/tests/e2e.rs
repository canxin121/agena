//! End-to-end test: spin up `agena-api-server` against an in-memory
//! `SessionManager`, drive the bus through the publisher, and consume
//! events via the SDK over both REST and WebSocket.

use std::{
    fs,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use agena::{
    agent::Agent,
    config::LoadConfigRequest,
    event::{EventKind, PublishContext},
    message::{PartContent, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput},
    permission::{
        PermissionMode, PermissionPolicy, PermissionReplyKind, PermissionScope,
        ToolPermissionPolicy,
    },
    provider::{
        CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
        CompletionToolCall, ModelRuntime, OpenAiAdapter, ProviderRegistry,
    },
    role::Role,
    session::{ContextGovernor, ContextPolicy, SessionManager, SessionProcessor},
    tool::ToolExecutor,
};
use agena_api::{
    Scope,
    commands::{
        Command, CommandResult, CreateSessionParams, ReplyPermissionParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, SubmitTurnParams, UpsertPermissionRuleParams,
    },
    pagination::PaginatedResponse,
    queries::{
        GetSessionParams, GetWorkspaceParams, ListEventsParams, ListMessagesParams,
        ListSessionsParams, ListWorkspacesParams, Query, QueryResult,
    },
    resource::{PartLoadMode, RunOptions},
    subscribe::SubscribeRequest,
};
use agena_api_server::{AppState, router};
use agena_client::{AgenaClient, WsClient, ws::SubscriptionEvent};

struct PermissionTestProvider;
use mockito::ServerGuard;
use sea_orm::Database;

struct PermissionTestHostClient;

#[async_trait::async_trait]
impl agena::plugin::sdk::host_api::HostClient for PermissionTestHostClient {
    async fn log(
        &self,
        _level: agena::plugin::sdk::host_api::LogLevel,
        _message: String,
        _fields: serde_json::Value,
    ) {
    }

    async fn publish_event(
        &self,
        _env: agena::plugin::EventEnvelope,
    ) -> agena::plugin::sdk::Result<()> {
        Err(agena::plugin::PluginError::new(
            "publish_event is not used by this test host",
        ))
    }

    async fn subscribe_events(
        &self,
        _filter: agena::plugin::EventFilter,
    ) -> agena::plugin::sdk::Result<agena::plugin::sdk::host_api::EventSubscription> {
        Err(agena::plugin::PluginError::new(
            "subscribe_events is not used by this test host",
        ))
    }

    async fn ask_permission(
        &self,
        _req: agena::plugin::PermissionAskInput,
    ) -> agena::plugin::sdk::Result<agena::plugin::PermissionDecision> {
        Err(agena::plugin::PluginError::new(
            "ask_permission is not used by this test host",
        ))
    }

    async fn read_config(
        &self,
        _path: Option<String>,
    ) -> agena::plugin::sdk::Result<serde_json::Value> {
        Err(agena::plugin::PluginError::new(
            "read_config is not used by this test host",
        ))
    }

    async fn invoke_tool(
        &self,
        tool: String,
        _input: serde_json::Value,
    ) -> agena::plugin::sdk::Result<agena::plugin::ToolInvokeOutput> {
        Err(agena::plugin::PluginError::new(format!(
            "invoke_tool is not used by this test host: {tool}",
        )))
    }

    async fn todo_write(
        &self,
        req: agena::plugin::sdk::host_api::HostTodoWriteRequest,
    ) -> agena::plugin::sdk::Result<agena::plugin::ToolInvokeOutput> {
        let items = serde_json::to_value(&req.items)
            .map_err(|err| agena::plugin::PluginError::new(err.to_string()))?;
        Ok(agena::plugin::ToolInvokeOutput {
            title: "Todo write".to_string(),
            output_text: format!("Updated todo list with {} item(s):", req.items.len()),
            payload: Some(serde_json::json!({
                "output": {
                    "tool": "todo_write",
                    "items": items,
                },
                "apply_patch": null,
            })),
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }
}

#[async_trait::async_trait]
impl ModelRuntime for PermissionTestProvider {
    fn id(&self) -> &str {
        "permission-test"
    }

    fn default_model(&self) -> &agena::model::ModelId {
        static DEFAULT_MODEL: std::sync::LazyLock<agena::model::ModelId> =
            std::sync::LazyLock::new(|| agena::model::ModelId::new("permission-test-model"));
        &DEFAULT_MODEL
    }

    async fn list_models(&self) -> Result<Vec<agena::provider::ProviderModel>, agena::AppError> {
        Ok(vec![agena::provider::ProviderModel::new(
            "permission-test",
            self.default_model().as_str(),
        )])
    }

    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, agena::AppError> {
        let last_user_text = request
            .messages
            .iter()
            .rev()
            .find(|message| message.role == Role::User)
            .map(agena::message::Message::as_text_lossy)
            .unwrap_or_default();
        let todo_done = request.messages.iter().any(|message| {
            message
                .parts
                .iter()
                .any(|part| part.operation_id.as_deref() == Some("call_todo_1"))
        });

        if last_user_text.contains("permission todo") && !todo_done {
            return Ok(CompletionResponse {
                provider_id: agena::model::ProviderId::new("permission-test"),
                model: self.default_model().clone(),
                text: String::new(),
                reasoning_text: None,
                finish_reason: Some(CompletionFinishReason::ToolCalls),
                tool_calls: vec![CompletionToolCall::Function {
                    id: "call_todo_1".to_string(),
                    name: "todo_write".to_string(),
                    arguments_json: serde_json::to_string(&TodoWriteToolInput {
                        items: vec![TodoItem {
                            content: "confirm permission sdk".to_string(),
                            status: TodoStatus::Completed,
                            priority: TodoPriority::Low,
                        }],
                    })
                    .expect("serialize todo input"),
                }],
                usage: None,
                provider_metadata: None,
            });
        }

        Ok(CompletionResponse {
            provider_id: agena::model::ProviderId::new("permission-test"),
            model: self.default_model().clone(),
            text: "permission todo done".to_string(),
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
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, agena::AppError>>
                    + Send,
            >,
        >,
        agena::AppError,
    > {
        let response = self.complete(request).await?;
        let events = if response.tool_calls.is_empty() {
            vec![
                Ok(CompletionStreamEvent::TextDelta {
                    provider_id: response.provider_id.clone(),
                    model: response.model.clone(),
                    delta: response.text.clone(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: response.provider_id,
                    model: response.model,
                    finish_reason: response.finish_reason,
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        } else {
            vec![
                Ok(CompletionStreamEvent::ToolCallDelta {
                    provider_id: response.provider_id.clone(),
                    model: response.model.clone(),
                    stream_key: "call_todo_1".to_string(),
                    id: Some("call_todo_1".to_string()),
                    name: Some("todo_write".to_string()),
                    arguments_delta: response
                        .tool_calls
                        .first()
                        .map(|call| match call {
                            CompletionToolCall::Function { arguments_json, .. } => {
                                arguments_json.clone()
                            }
                        })
                        .unwrap_or_default(),
                }),
                Ok(CompletionStreamEvent::Completed {
                    provider_id: response.provider_id,
                    model: response.model,
                    finish_reason: response.finish_reason,
                    usage: None,
                    provider_metadata: None,
                }),
            ]
        };
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

async fn spawn_server() -> (String, String, Arc<SessionManager>) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let registry = ProviderRegistry::new();
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        std::env::temp_dir(),
        Agent::new("client-e2e", PermissionPolicy::allow_all()),
    );
    let manager = Arc::new(SessionManager::new(db.clone(), processor, executor));
    let config_path =
        std::env::temp_dir().join(format!("agena-client-e2e-{}.toml", uuid::Uuid::new_v4()));
    std::fs::write(&config_path, "").expect("test config should be written");

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(agena::config::LoadConfigRequest {
            config_path: Some(config_path),
            ..agena::config::LoadConfigRequest::default()
        })
        .with_workspace_root(std::env::temp_dir())
        .with_database_connection(db.clone())
        .build()
        .await
        .expect("runtime build");
    let shared_db = Arc::new(db.clone());
    let state =
        AppState::new(runtime, Arc::clone(&shared_db)).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager)
}

fn write_temp_config(content: &str) -> std::path::PathBuf {
    let suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time should move forward")
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("agena-client-e2e-{suffix}"));
    fs::create_dir_all(&dir).expect("temp dir should be created");
    let path = dir.join("config.toml");
    fs::write(&path, content).expect("config should be written");
    path
}

async fn spawn_permission_server() -> (String, String, Arc<SessionManager>) {
    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let mut registry = ProviderRegistry::new();
    registry.register(PermissionTestProvider);
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let workspace_root = std::env::temp_dir().join(format!(
        "agena-client-e2e-permission-{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos()
    ));
    fs::create_dir_all(&workspace_root).expect("permission temp dir should be created");
    let config_path = write_temp_config(
        r#"
[providers."permission-test"]
default_model = "anthropic/permission-test-model"

[providers."permission-test".auth]
mode = "api"
base_url = "https://example.invalid/v1"
api_key = "test"

[providers."permission-test".adapters.anthropic]
enabled = true
"#,
    );
    let plugins = agena::tool::default_tool_host(workspace_root.clone()).expect("plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        Agent::new("client-e2e-permission", PermissionPolicy::allow_all()).with_tool_policy(
            ToolPermissionPolicy::allow_all().with_tool_mode("todo_write", PermissionMode::Ask),
        ),
    )
    .with_plugin_manager(Arc::clone(&plugins));
    let manager = Arc::new(SessionManager::new(
        db.clone(),
        processor.with_plugin_host(Arc::clone(&plugins)),
        executor.clone(),
    ));

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(workspace_root)
        .with_database_connection(db.clone())
        .build()
        .await
        .expect("runtime build");
    plugins
        .host_handle()
        .install_client(Arc::new(PermissionTestHostClient))
        .await;
    let shared_db = Arc::new(db.clone());
    let state =
        AppState::new(runtime, Arc::clone(&shared_db)).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager)
}

async fn spawn_server_with_provider() -> (String, String, Arc<SessionManager>, ServerGuard) {
    let mut provider = mockito::Server::new_async().await;
    let _models = provider
        .mock("GET", "/models")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "data": [{ "id": "gpt-4o-mini" }]
            })
            .to_string(),
        )
        .create_async()
        .await;
    let _chat = provider
        .mock("POST", "/chat/completions")
        .with_status(200)
        .with_header("content-type", "application/json")
        .with_body(
            serde_json::json!({
                "id": "chatcmpl_test",
                "choices": [{
                    "message": { "content": "ack" },
                    "finish_reason": "stop"
                }],
                "usage": {
                    "prompt_tokens": 3,
                    "completion_tokens": 1,
                    "total_tokens": 4
                }
            })
            .to_string(),
        )
        .create_async()
        .await;

    let config_path = write_temp_config(&format!(
        r#"
[providers.openai]
default_model = "openai/gpt-4o-mini"

[providers.openai.auth]
mode = "api"
base_url = "{base_url}"
api_key = "test"

[providers.openai.adapters.openai]
"#,
        base_url = provider.url()
    ));
    let workspace_root = config_path
        .parent()
        .expect("config should have parent")
        .to_path_buf();

    let db = Database::connect("sqlite::memory:").await.unwrap();
    agena::db::init_schema(&db).await.unwrap();

    let mut registry = ProviderRegistry::new();
    registry.register(OpenAiAdapter::new_with_id(
        "openai",
        reqwest::Client::new(),
        "test",
        provider.url(),
        "gpt-4o-mini",
    ));
    let processor = SessionProcessor::new(
        Arc::new(registry),
        ContextGovernor::new(ContextPolicy::default()),
    );
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        Agent::new("client-e2e-provider", PermissionPolicy::allow_all()),
    );
    let manager = Arc::new(SessionManager::new(db.clone(), processor, executor));

    let runtime = agena::runtime::AgenaRuntime::builder()
        .with_load_request(LoadConfigRequest {
            config_path: Some(config_path),
            ..LoadConfigRequest::default()
        })
        .with_workspace_root(workspace_root)
        .with_database_connection(db.clone())
        .build()
        .await
        .expect("runtime build");
    let shared_db = Arc::new(db.clone());
    let state =
        AppState::new(runtime, Arc::clone(&shared_db)).with_manager_override(Arc::clone(&manager));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let http_url = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/api/v1/ws");
    (http_url, ws_url, manager, provider)
}

#[tokio::test]
async fn rest_health_via_sdk() {
    let (http_url, _, _) = spawn_server().await;
    let client = AgenaClient::new(http_url).unwrap();
    let health = client.health().await.unwrap();
    assert_eq!(health.status, "ok");
}

#[tokio::test]
async fn rest_list_events_returns_published() {
    let (http_url, _, manager) = spawn_server().await;

    manager
        .event_publisher()
        .publish(
            PublishContext::for_session(1),
            EventKind::PluginEvent(agena::event::PluginEventPayload {
                plugin_id: "test".into(),
                kind_label: "test_event".into(),
                payload: serde_json::json!({}),
            }),
        )
        .await
        .unwrap();

    let client = AgenaClient::new(http_url).unwrap();
    let page = client
        .list_events(ListEventsParams {
            scope: Scope::Global,
            ..Default::default()
        })
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert_eq!(page.items[0].kind.tag_str(), "plugin_event");
}

#[tokio::test]
async fn rest_command_and_query_cover_workspace_session_and_message_routes() {
    let (http_url, _ws_url, _manager, _provider) = spawn_server_with_provider().await;
    let client = AgenaClient::new(http_url).unwrap();

    let workspace = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: std::env::temp_dir().display().to_string(),
            create_if_missing: true,
        }))
        .await
        .unwrap();
    let CommandResult::Workspace(workspace) = workspace else {
        panic!("expected workspace command result");
    };

    let workspaces = client
        .query(Query::ListWorkspaces(ListWorkspacesParams {
            cursor: None,
            limit: Some(20),
            search: None,
            include_session_count: true,
        }))
        .await
        .unwrap();
    let QueryResult::Workspaces(workspaces) = workspaces else {
        panic!("expected workspaces query result");
    };
    assert!(workspaces.items.iter().any(|item| item.id == workspace.id));

    let workspace_result = client
        .query(Query::GetWorkspace(GetWorkspaceParams {
            workspace_id: workspace.id,
        }))
        .await
        .unwrap();
    let QueryResult::Workspace(workspace_detail) = workspace_result else {
        panic!("expected workspace query result");
    };
    assert_eq!(workspace_detail.id, workspace.id);

    let created = client
        .command(Command::CreateSession(CreateSessionParams {
            workspace_id: workspace.id,
            title: "sdk session".to_string(),
            parent_id: None,
        }))
        .await
        .unwrap();
    let CommandResult::Session(session) = created else {
        panic!("expected session command result");
    };

    let sessions = client
        .query(Query::ListSessions(ListSessionsParams {
            cursor: None,
            limit: Some(20),
            workspace_id: Some(workspace.id),
            parent_id: None,
            roots: false,
            search: None,
        }))
        .await
        .unwrap();
    let QueryResult::Sessions(PaginatedResponse { items, .. }) = sessions else {
        panic!("expected sessions query result");
    };
    assert!(items.iter().any(|item| item.id == session.id));

    let get_session = client
        .query(Query::GetSession(GetSessionParams {
            session_id: session.id,
        }))
        .await
        .unwrap();
    let QueryResult::Session(session_detail) = get_session else {
        panic!("expected session detail result");
    };
    assert_eq!(session_detail.id, session.id);

    let turn = client
        .command(Command::SubmitTurn(SubmitTurnParams {
            session_id: session.id,
            options: RunOptions::default(),
            parts: vec![PartContent::text("hello from sdk")],
        }))
        .await
        .unwrap();
    let CommandResult::Execution(execution) = turn else {
        panic!("expected execution result");
    };
    assert_eq!(execution.session.id, session.id);

    let state = client
        .query(Query::GetSessionState(GetSessionParams {
            session_id: session.id,
        }))
        .await
        .unwrap();
    let QueryResult::SessionState(state) = state else {
        panic!("expected session state result");
    };
    assert_eq!(state.session.id, session.id);

    let messages = client
        .query(Query::ListMessages(ListMessagesParams {
            session_id: session.id,
            cursor: None,
            limit: Some(20),
            parts: PartLoadMode::Full,
        }))
        .await
        .unwrap();
    let QueryResult::Messages(PaginatedResponse {
        items: messages, ..
    }) = messages
    else {
        panic!("expected messages query result");
    };
    assert!(!messages.is_empty());
    let message = messages
        .iter()
        .find(|message| message.session_id == session.id)
        .expect("expected a message belonging to the created session")
        .clone();
    assert_eq!(message.session_id, session.id);
    assert!(
        message
            .parts
            .as_ref()
            .is_some_and(|parts| !parts.is_empty())
    );
}

#[tokio::test]
async fn rest_permission_rule_and_reply_flows_round_trip_via_sdk() {
    let (http_url, _ws_url, _manager) = spawn_permission_server().await;
    let client = AgenaClient::new(http_url).unwrap();

    let workspace = client
        .command(Command::ResolveWorkspace(ResolveWorkspaceParams {
            path: std::env::temp_dir().display().to_string(),
            create_if_missing: true,
        }))
        .await
        .unwrap();
    let CommandResult::Workspace(workspace) = workspace else {
        panic!("expected workspace command result");
    };

    let created_rule = client
        .command(Command::UpsertPermissionRule(UpsertPermissionRuleParams {
            action_key: None,
            subject_kind: Some("tool".to_string()),
            tool_name: Some("bash".to_string()),
            qualifier: Some("git status*".to_string()),
            path_access_kind: None,
            workspace_root: None,
            target_path: None,
            network_target: None,
            network_host: None,
            network_port: None,
            scope: Some("global".to_string()),
            session_id: None,
            mode: agena_api::resource::PermissionMode::Allow,
        }))
        .await
        .unwrap();
    let CommandResult::PermissionRule(created_rule) = created_rule else {
        panic!("expected permission rule result");
    };
    assert_eq!(created_rule.operator.as_deref(), Some("http_api"));
    assert_eq!(created_rule.scope, "global");

    let revoked_rule = client
        .command(Command::RevokePermissionRule(RevokePermissionRuleParams {
            rule_id: created_rule.id,
            reason: Some("cleanup".to_string()),
        }))
        .await
        .unwrap();
    let CommandResult::PermissionRule(revoked_rule) = revoked_rule else {
        panic!("expected permission rule revoke result");
    };
    assert_eq!(revoked_rule.revoked_by.as_deref(), Some("http_api"));

    let session = client
        .command(Command::CreateSession(CreateSessionParams {
            workspace_id: workspace.id,
            title: "sdk permission flow".to_string(),
            parent_id: None,
        }))
        .await
        .unwrap();
    let CommandResult::Session(session) = session else {
        panic!("expected session command result");
    };

    let execution = client
        .command(Command::SubmitTurn(SubmitTurnParams {
            session_id: session.id,
            options: RunOptions {
                model: Some(agena::model::ModelRef::new(
                    "permission-test",
                    "permission-test-model",
                )),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                parallel_tool_calls: None,
                system: None,
                temperature: None,
                max_output_tokens: Some(128),
                agent_profile: None,
                max_turn_loops: None,
            },
            parts: vec![PartContent::text("permission todo")],
        }))
        .await
        .unwrap();
    let CommandResult::Execution(execution) = execution else {
        panic!("expected execution result");
    };
    let request_id = execution
        .pending_permission_requests
        .first()
        .map(|request| request.request_id.clone())
        .expect("pending permission request");

    let replied = client
        .reply_permission(ReplyPermissionParams {
            session_id: session.id,
            options: RunOptions {
                model: Some(agena::model::ModelRef::new(
                    "permission-test",
                    "permission-test-model",
                )),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                parallel_tool_calls: None,
                system: None,
                temperature: None,
                max_output_tokens: Some(128),
                agent_profile: None,
                max_turn_loops: None,
            },
            reply: agena_api::resource::PermissionReply {
                request_id,
                kind: PermissionReplyKind::AllowAlways,
                reason: None,
                scope: Some(PermissionScope::Global),
            },
        })
        .await
        .unwrap();
    assert!(replied.pending_permission_requests.is_empty());

    let rules = client
        .query(Query::ListPermissionRules(
            agena_api::queries::ListPermissionRulesParams {
                cursor: None,
                limit: Some(20),
                search: None,
            },
        ))
        .await
        .unwrap();
    let QueryResult::PermissionRules(rules) = rules else {
        panic!("expected permission rules query result");
    };
    assert!(rules.items.iter().any(|rule| {
        rule.source == "permission_reply"
            && rule.operator.as_deref() == Some("http_api")
            && rule.scope == "global"
    }));
}

#[tokio::test]
async fn ws_subscription_delivers_event() {
    let (_, ws_url, manager) = spawn_server().await;

    let client = WsClient::connect(&ws_url).await.unwrap();
    let mut sub = client
        .subscribe(SubscribeRequest {
            scope: Scope::Global,
            kinds: None,
            since_seq_global: None,
        })
        .await
        .unwrap();

    // Give the server a moment to register the subscription before we
    // publish.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    manager
        .event_publisher()
        .publish(
            PublishContext::for_session(2),
            EventKind::RunStarted(agena::event::RunStartedEvent {
                session_id: 2,
                ts_ms: 0,
            }),
        )
        .await
        .unwrap();

    let item = tokio::time::timeout(std::time::Duration::from_millis(500), sub.recv())
        .await
        .expect("ws should deliver event")
        .expect("subscription not closed");
    let SubscriptionEvent::Event(event) = item else {
        panic!("expected event, got lagged");
    };
    assert_eq!(event.kind.tag_str(), "run_started");
    assert_eq!(event.meta.session_id, Some(2));
}
