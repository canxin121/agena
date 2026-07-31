#[cfg(test)]
use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::{AppError, RuntimeSessionManagerConfig};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::Duration,
    };

    use agena_domain::{
        ExecutionStatus, FinishReason, PermissionAction, PermissionReplyKind, PermissionRiskLevel,
        StructuredObject, TimeRange,
    };
    use sea_orm::Database;
    use tokio::sync::Notify;

    use super::{
        HostPermissionGrantGuard, SessionManager, build_message,
        host_permission_grant_matches_action, merge_system_prompts,
    };
    use crate::session::history::{
        AssistantMessageFinished, RunCompleted, RunStarted, TranscriptContent, UserMessageAppended,
    };
    use crate::{
        authorization::ExecutionPrincipal,
        event::EventKind,
        message::{MessageMetadata, OperationPart, PartContent},
        permission::{PermissionPolicy, ToolPermissionPolicy},
        provider::{ModelRuntime, ProviderRegistry},
        session::{ContextGovernor, Session, SessionProcessor},
        tool::ToolExecutor,
    };
    use agena_domain::PermissionReply;
    use agena_domain::ToolInvocation;
    use agena_domain::{ExecutionId, ExecutionSource, Role, RunId};
    use agena_domain::{Model, ModelId, ModelRef};
    use agena_plugin_host::sdk::ToolStreamSink;
    use agena_plugin_host::{
        ConfiguredPlugin, PluginHost, PluginHostBuildConfig, PluginsConfig,
        StaticPluginRegistration, ToolPresentationConfig,
    };
    use agena_provider::CompletionRequest;
    use agena_provider::CompletionResponse;
    use agena_runtime::{
        SessionCreateRequest, SessionPermissionReplyRequest, SessionPluginCommandRequest,
        SessionPluginCommandService, SessionRewindRequest, SessionRunOptions,
        SessionToolExecutionService,
    };

    #[test]
    fn system_prompt_merge_is_idempotent_for_an_already_applied_identity_prompt() {
        assert_eq!(
            merge_system_prompts(Some("identity"), Some("identity\n\ncustom")),
            Some("identity\n\ncustom".to_string())
        );
        assert_eq!(
            merge_system_prompts(Some("identity"), Some("custom")),
            Some("identity\n\ncustom".to_string())
        );
    }

    #[derive(Default)]
    struct StreamingExecutionTool;

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "stream",
        version = "0.1.0",
        summary = "Streaming execution-tool regression fixture."
    )]
    impl StreamingExecutionTool {
        #[tool(
            name = "emit",
            summary = "Emit streaming chunks.",
            read_only,
            stream = emit_stream
        )]
        async fn emit(&self) -> String {
            "buffered-handler".to_string()
        }

        async fn emit_stream(&self, sink: ToolStreamSink) -> String {
            sink.text("stream-").await;
            sink.text("handler").await;
            "stream-terminal".to_string()
        }

        #[tool(name = "object", summary = "Return a structured object.", read_only)]
        async fn object(&self) -> serde_json::Value {
            serde_json::json!({ "approved": true })
        }
    }

    static REPLY_PROBE_STARTED: Notify = Notify::const_new();
    static REPLY_PROBE_CONTINUE: Notify = Notify::const_new();

    #[derive(Default)]
    struct ReplyLockProbeTool;

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "reply_probe",
        version = "0.1.0",
        summary = "Permission reply-lock regression fixture."
    )]
    impl ReplyLockProbeTool {
        #[tool(name = "run", summary = "Wait until the reply-lock test releases it.")]
        async fn run(&self) -> String {
            REPLY_PROBE_STARTED.notify_one();
            REPLY_PROBE_CONTINUE.notified().await;
            "reply-probe-complete".to_string()
        }
    }

    #[derive(Default)]
    struct CommandProbePlugin;

    #[async_trait::async_trait]
    impl agena_plugin_host::sdk::Plugin for CommandProbePlugin {
        fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
            let mut manifest =
                agena_plugin_host::sdk::PluginManifest::new("test", "command_probe", "0.1.0");
            manifest.summary =
                Some("Explicit plugin-command authorization regression fixture.".to_owned());
            manifest.commands.push(
                serde_json::from_value(serde_json::json!({
                    "id": "command_probe.run",
                    "title": "Run Command Probe",
                    "slash": "/command-probe",
                    "handler": "command_probe.run",
                    "action": {
                        "kind": "invoke_command",
                        "command": "command_probe.run"
                    }
                }))
                .expect("valid command probe definition"),
            );
            manifest
        }

        async fn command_invoke(
            &self,
            input: agena_plugin_host::sdk::PluginCommandInvokeInput,
        ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::PluginCommandOutput> {
            if input.command_id == "command_probe.run" {
                Ok(agena_plugin_host::sdk::PluginCommandOutput::None)
            } else {
                Err(agena_plugin_host::sdk::PluginError::not_implemented(
                    input.command_id,
                ))
            }
        }
    }

    struct ReplyTestProvider {
        default_model: ModelId,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for ReplyTestProvider {
        fn id(&self) -> &str {
            "reply-test-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            Err(agena_runtime_provider::ProviderError::Provider(
                "reply lock test provider does not complete".to_string(),
            ))
        }
    }

    async fn test_manager() -> SessionManager {
        test_manager_with_tool_policy(ToolPermissionPolicy::allow_all()).await
    }

    async fn test_manager_with_tool_policy(tool_policy: ToolPermissionPolicy) -> SessionManager {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.stream".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.reply_probe".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.command_probe".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![
                StaticPluginRegistration::new(
                    "test.stream".parse().expect("valid test plugin key"),
                    StreamingExecutionTool,
                ),
                StaticPluginRegistration::new(
                    "test.reply_probe"
                        .parse()
                        .expect("valid reply probe plugin key"),
                    ReplyLockProbeTool,
                ),
                StaticPluginRegistration::new(
                    "test.command_probe"
                        .parse()
                        .expect("valid command probe plugin key"),
                    CommandProbePlugin,
                ),
            ],
            config: plugins_config,
            workspace_root: workspace_root.clone(),
            agena_version: "test".to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: HashMap::new(),
        })
        .await
        .expect("build test plugin host");

        let executor = ToolExecutor::new(
            workspace_root.clone(),
            ExecutionPrincipal::new(PermissionPolicy::allow_all(), tool_policy),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let mut providers = ProviderRegistry::new();
        providers.register(ReplyTestProvider {
            default_model: ModelId::new("reply-test-model"),
        });
        let processor = SessionProcessor::new(
            Arc::new(providers),
            ContextGovernor::new(agena_domain::ContextPolicy::default()),
            plugins,
            workspace_root,
        );
        let database = Database::connect("sqlite::memory:")
            .await
            .expect("open in-memory database");
        agena_storage_sqlite::initialize_schema(&database)
            .await
            .expect("migrate in-memory database");
        SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn accepted_execution_returns_before_terminal_outcome_and_cancels_cleanly() {
        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "accepted execution".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create session");
        let never_finishes = Arc::new(Notify::new());
        let wait = Arc::clone(&never_finishes);
        let outcome = manager
            .start_registered(
                session.id,
                ExecutionSource::User,
                "acceptance regression",
                move |_manager, _control, _steer_rx| async move {
                    wait.notified().await;
                    Ok::<_, AppError>(())
                },
            )
            .await
            .expect("execution accepted");
        let receipt = outcome.receipt.expect("accepted receipt");

        assert!(manager.active_execution(session.id).await.is_some());
        assert_eq!(
            manager
                .cancel_execution(session.id, receipt.execution_id)
                .await
                .expect("request cancellation"),
            agena_domain::CancellationResult::CancellationRequested
        );

        tokio::time::timeout(Duration::from_secs(2), async {
            while manager.active_execution(session.id).await.is_some() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("execution terminalized after cancellation");

        let events = manager
            .list_session_events(session.id)
            .await
            .expect("load lifecycle events");
        let starts = events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ExecutionStarted(_)))
            .count();
        let finishes = events
            .iter()
            .filter(|event| matches!(event.kind, EventKind::ExecutionFinished(_)))
            .count();
        assert_eq!(starts, 1);
        assert_eq!(finishes, 1);
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            EventKind::ExecutionFinished(finished)
                if finished.outcome == agena_domain::ExecutionOutcome::Cancelled
        )));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_plugin_command_does_not_consult_tool_permission_policy() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Ask,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "command permission regression".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create command regression session");

        let output = manager
            .invoke_session_plugin_command(SessionPluginCommandRequest {
                session_id: session.id,
                plugin_id: "test.command_probe".to_owned(),
                command_id: "command_probe.run".to_owned(),
                input: serde_json::json!({}),
                slash: Some("/command-probe".to_owned()),
                raw: String::new(),
                workspace_root: None,
            })
            .await
            .expect("explicit command should not be evaluated as a tool");

        assert!(matches!(
            output,
            agena_plugin_host::sdk::PluginCommandOutput::None
        ));

        let tool_outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
            )
            .await
            .expect("authorization is a normal invocation outcome");
        let agena_runtime::SessionToolExecutionOutcome::ApprovalRequired { request_id, .. } =
            tool_outcome
        else {
            panic!("unexpected tool outcome: {tool_outcome:?}");
        };
        let request_id = request_id.expect("session-scoped Ask must have a request id");
        let reloaded = manager
            .get_session(session.id)
            .await
            .expect("reload approval session");
        assert!(
            reloaded
                .find_pending_permission_by_request_id(request_id.as_str())
                .is_some(),
            "external Ask must create a durable, actionable permission request"
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_tool_deny_is_a_normal_policy_outcome() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Deny,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "policy denial regression".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create policy denial session");

        let outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
            )
            .await
            .expect("policy denial must not use the error channel");

        let agena_runtime::SessionToolExecutionOutcome::PolicyDenied(denial) = outcome else {
            panic!("unexpected tool outcome: {outcome:?}");
        };
        assert_eq!(denial.denied_actions.len(), 1);
        assert!(denial.reason.contains("denied by policy"));
        assert_eq!(denial.source.as_deref(), Some("static_policy"));
        assert_eq!(
            denial.authority,
            agena_domain::PermissionAuthorityKind::StaticPolicy
        );
        assert!(denial.rule_id.is_none());
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn persisted_deny_reports_stable_rule_identity_and_revision() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Allow,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "persisted denial provenance".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create provenance session");
        let action = PermissionAction::Tool {
            tool_name: "test.stream.object".to_string(),
            qualifier: None,
        };
        manager
            .persist_session_changes_with_rules(
                session.clone(),
                Vec::new(),
                Vec::new(),
                vec![agena_storage::PersistedPermissionRule {
                    id: None,
                    created_at_ms: None,
                    updated_at_ms: None,
                    action_key: serde_json::to_string(&action).expect("serialize action"),
                    mode: agena_domain::PermissionMode::Deny,
                    scope: agena_domain::PermissionScope::Session,
                    session_id: Some(session.id),
                    workspace_id: None,
                    source: "permission_studio".to_string(),
                    reason: Some("blocked by saved user rule".to_string()),
                    operator: Some("test-user".to_string()),
                    revoked_at_ms: None,
                    revoked_reason: None,
                    revoked_by: None,
                }],
                manager.execution_state(),
            )
            .await
            .expect("persist deny rule");

        let outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
            )
            .await
            .expect("persisted denial is a normal outcome");
        let agena_runtime::SessionToolExecutionOutcome::PolicyDenied(denial) = outcome else {
            panic!("unexpected persisted denial outcome: {outcome:?}");
        };
        assert_eq!(
            denial.authority,
            agena_domain::PermissionAuthorityKind::PersistedRule
        );
        assert_eq!(denial.source.as_deref(), Some("permission_studio"));
        assert_eq!(denial.operator.as_deref(), Some("test-user"));
        assert!(denial.rule_id.is_some());
        assert!(denial.rule_revision_ms.is_some());
        let wire = serde_json::to_value(&denial).expect("serialize denial provenance");
        assert_eq!(wire["authority"], "persisted_rule");
        assert!(wire["rule_id"].is_number());
        assert!(wire["rule_revision_ms"].is_number());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_tool_ask_persists_and_approved_reply_executes_without_model_continuation() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Ask,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "external approval execution".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create external approval session");
        let outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
            )
            .await
            .expect("Ask is a normal outcome");
        let agena_runtime::SessionToolExecutionOutcome::ApprovalRequired { request_id, .. } =
            outcome
        else {
            panic!("unexpected external authorization outcome: {outcome:?}");
        };
        let request_id = request_id.expect("external Ask must have a request id");

        let session = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("unused-provider", "unused-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                PermissionReply {
                    request_id: request_id.clone(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            ))
            .await
            .expect("approved external tool executes without invoking a model");
        assert!(session.has_finished_operation(request_id.as_str()));
        let part = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find(|part| {
                part.operation_id.as_deref() == Some(request_id.as_str())
                    && matches!(
                        part.content.as_ref(),
                        Some(PartContent::Activity(
                            crate::message::RuntimeActivity::Operation(_)
                        ))
                    )
            })
            .expect("completed external tool operation");
        assert_eq!(part.status, ExecutionStatus::Completed, "part={part:?}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn external_tool_decline_finishes_as_user_declined_without_execution() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Ask,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "external approval decline".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create external decline session");
        let outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
            )
            .await
            .expect("Ask is a normal outcome");
        let agena_runtime::SessionToolExecutionOutcome::ApprovalRequired { request_id, .. } =
            outcome
        else {
            panic!("unexpected external authorization outcome: {outcome:?}");
        };
        let request_id = request_id.expect("external Ask must have a request id");

        let session = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("unused-provider", "unused-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                PermissionReply {
                    request_id: request_id.clone(),
                    kind: PermissionReplyKind::DenyOnce,
                    reason: Some("not now".to_owned()),
                    scope: None,
                },
                None,
            ))
            .await
            .expect("declining an external tool is a normal continuation");
        let part = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find(|part| {
                part.operation_id.as_deref() == Some(request_id.as_str())
                    && matches!(
                        part.content.as_ref(),
                        Some(PartContent::Activity(
                            crate::message::RuntimeActivity::Operation(_)
                        ))
                    )
            })
            .expect("declined external tool operation");
        assert_eq!(part.status, ExecutionStatus::UserDeclined, "part={part:?}");
        let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(operation))) =
            part.content.as_ref()
        else {
            panic!("declined part is not an operation: {part:?}");
        };
        assert_eq!(
            operation.result.state,
            agena_domain::ToolResultState::UserDeclined
        );
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn unavailable_tool_is_a_normal_structured_outcome() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Allow,
        ))
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "tool availability regression".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create tool availability session");

        let outcome = manager
            .execute_session_tool(
                session.id,
                ToolInvocation::new("missing.tool", StructuredObject::default()),
            )
            .await
            .expect("tool availability must not use the error channel");

        let agena_runtime::SessionToolExecutionOutcome::ToolUnavailable(unavailable) = outcome
        else {
            panic!("unexpected tool outcome: {outcome:?}");
        };
        assert_eq!(unavailable.tool_name, "missing.tool");
        assert_eq!(unavailable.source, "tool_registry");
        assert!(!unavailable.retryable);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn sessionless_tool_ask_is_a_normal_non_execution_outcome() {
        let manager = test_manager_with_tool_policy(ToolPermissionPolicy::new(
            agena_domain::PermissionMode::Ask,
        ))
        .await;
        let outcome = manager
            .execute_unscoped_tool(
                ToolInvocation::new("test.stream.object", StructuredObject::default()),
                77,
            )
            .await
            .expect("sessionless Ask must not use the error channel");
        let agena_runtime::SessionToolExecutionOutcome::ApprovalRequired { request_id, reason } =
            outcome
        else {
            panic!("unexpected sessionless outcome: {outcome:?}");
        };
        assert!(request_id.is_none());
        assert!(reason.contains("requires confirmation"));
    }

    async fn append_completed_text_message(
        manager: &SessionManager,
        mut session: Session,
        role: Role,
        text: &str,
        turn_id: Option<i64>,
        parent_message_id: Option<i64>,
    ) -> (Session, i64) {
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve rewind regression message ids");
        let message_id = ids.message_id;
        let message = build_message(
            ids,
            role,
            ExecutionStatus::Completed,
            vec![PartContent::text(text)],
            MessageMetadata {
                turn_id: Some(turn_id.unwrap_or(message_id)),
                parent_message_id,
                ..Default::default()
            },
        );
        session.messages.push(message.clone());
        let session = manager
            .persist_session_changes(
                session,
                vec![message.clone()],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist rewind regression message");
        let session_id = session.id;
        let execution_id = ExecutionId::new();
        let transcript_turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::ResponseId::new();
        let run_id = RunId::new();
        let message_event = match role {
            Role::User => EventKind::UserMessageAppended(UserMessageAppended {
                execution_id,
                message_id: agena_domain::MessageId(message.id),
                run_id,
                created_at: message.created_at,
                content: TranscriptContent::from_message_lossy(&message),
                parts: message.parts.clone(),
                metadata: message.metadata.clone(),
                provider_state: message.provider_state.clone(),
            }),
            Role::Assistant => EventKind::AssistantMessageFinished(AssistantMessageFinished {
                execution_id,
                message_id: agena_domain::MessageId(message.id),
                run_id,
                created_at: message.created_at,
                content: TranscriptContent::from_message_lossy(&message),
                status: message.state,
                parts: message.parts.clone(),
                usage: message.usage.clone(),
                finish_reason: FinishReason::Stop,
                metadata: message.metadata.clone(),
                provider_state: message.provider_state.clone(),
            }),
            role => panic!("unsupported rewind regression role: {role}"),
        };
        let session = manager
            .store
            .append_history_items(
                session,
                vec![
                    EventKind::ExecutionStarted(agena_domain::ExecutionStartedEvent {
                        session_id,
                        execution_id,
                        turn_id: transcript_turn_id,
                        response_id,
                        source: ExecutionSource::User,
                        ts_ms: message.created_at.timestamp_millis(),
                    }),
                    EventKind::RunStarted(RunStarted {
                        execution_id,
                        run_id,
                        source: ExecutionSource::User,
                        model_id: "test-model".into(),
                        provider_id: "test-provider".into(),
                        request_digest: None,
                    }),
                    message_event,
                    EventKind::RunCompleted(RunCompleted {
                        run_id,
                        finish_reason: FinishReason::Stop,
                    }),
                    EventKind::ExecutionFinished(agena_domain::ExecutionFinishedEvent {
                        session_id,
                        execution_id,
                        response_id,
                        outcome: agena_domain::ExecutionOutcome::Completed,
                        ts_ms: message.created_at.timestamp_millis(),
                    }),
                ],
                manager.execution_state().cache_policy(),
            )
            .await
            .expect("append current rewind regression history");
        (session, message_id)
    }

    async fn install_pending_tool_api_operation(
        manager: &SessionManager,
        mut session: Session,
        call_id: i64,
    ) -> Session {
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve Tool API operation message ids");
        let invocation = ToolInvocation::new(
            "agena.tools.call",
            StructuredObject::try_from(serde_json::json!({
                "tool": "stream.emit",
                "input": {}
            }))
            .expect("structured Tool API input"),
        );
        let operation =
            OperationPart::pending(call_id, invocation, "Tool tools.call", TimeRange::default());
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(operation)],
            MessageMetadata {
                turn_id: Some(1),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some("tool-api-stream-test".to_string());
        session.messages.push(message.clone());
        manager
            .persist_session_changes(
                session,
                vec![message],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist pending Tool API operation")
    }

    #[tokio::test]
    async fn updating_model_selection_is_immediate_and_session_local() {
        let manager = test_manager().await;
        let first = manager
            .create_session(SessionCreateRequest {
                title: "first session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create first session");
        let selected_model =
            ModelRef::new_with_adapter("selected-provider", "selected-adapter", "selected-model");

        manager
            .update_session_selection(
                first.id,
                SessionRunOptions {
                    model: selected_model.clone(),
                    thinking_mode: Some("high".to_owned()),
                    speed_mode: Some("fast".to_owned()),
                    verbosity: Some("high".to_owned()),
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
            )
            .await
            .expect("update first session model");

        let reloaded = manager
            .get_session(first.id)
            .await
            .expect("reload first session");
        assert_eq!(
            reloaded
                .runtime()
                .effective_model_ref()
                .expect("valid model reference"),
            Some(selected_model)
        );
        assert_eq!(
            reloaded.runtime().model_thinking_mode_override(),
            Some("high")
        );
        assert_eq!(reloaded.runtime().model_speed_mode_override(), Some("fast"));
        assert_eq!(reloaded.runtime().model_verbosity_override(), Some("high"));

        let second = manager
            .create_session(SessionCreateRequest {
                title: "second session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create second session");
        assert_eq!(
            second
                .runtime()
                .effective_model_ref()
                .expect("valid empty model selection"),
            None
        );
    }

    #[tokio::test]
    async fn rewind_copies_history_without_removing_it_from_the_source_session() {
        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "rewind source".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create rewind source");
        let source_id = session.id;
        let (session, first_user_id) = append_completed_text_message(
            &manager,
            session,
            Role::User,
            "first prompt",
            None,
            None,
        )
        .await;
        let (session, assistant_id) = append_completed_text_message(
            &manager,
            session,
            Role::Assistant,
            "first response",
            Some(first_user_id),
            Some(first_user_id),
        )
        .await;
        let (_session, rewind_target_id) = append_completed_text_message(
            &manager,
            session,
            Role::User,
            "rewrite this prompt",
            None,
            Some(assistant_id),
        )
        .await;

        let source_before = manager
            .store
            .list_projected_messages(source_id, true)
            .await
            .expect("load source projection before rewind");
        assert_eq!(source_before.len(), 3);

        let child = manager
            .rewind_session(SessionRewindRequest {
                session_id: source_id,
                message_id: rewind_target_id,
                expected_version: None,
            })
            .await
            .expect("rewind current-format session");

        let source_after = manager
            .store
            .list_projected_messages(source_id, true)
            .await
            .expect("reload source projection after rewind");
        assert_eq!(source_after, source_before);
        assert_eq!(
            source_after
                .iter()
                .map(|message| message.as_text_lossy())
                .collect::<Vec<_>>(),
            vec!["first prompt", "first response", "rewrite this prompt"]
        );

        assert_eq!(child.parent_id, Some(source_id));
        assert_eq!(
            child.relation_kind,
            agena_domain::SessionRelationKind::Rewind
        );
        assert_eq!(
            child.lifecycle_state,
            agena_domain::SessionLifecycleState::Ready
        );
        assert_eq!(child.source_message_id, Some(rewind_target_id));
        assert!(child.source_cutoff_seq_global.is_some());
        assert_eq!(
            child
                .messages
                .iter()
                .map(|message| message.as_text_lossy())
                .collect::<Vec<_>>(),
            vec!["first prompt", "first response"]
        );
        let source_message_ids = source_before
            .iter()
            .map(|message| message.id)
            .collect::<HashSet<_>>();
        let source_part_ids = source_before
            .iter()
            .flat_map(|message| message.parts.iter().map(|part| part.id))
            .collect::<HashSet<_>>();
        assert!(
            child
                .messages
                .iter()
                .all(|message| !source_message_ids.contains(&message.id))
        );
        assert!(
            child
                .messages
                .iter()
                .flat_map(|message| &message.parts)
                .all(|part| !source_part_ids.contains(&part.id))
        );
        assert_eq!(
            child.messages[1].metadata.turn_id,
            Some(child.messages[0].id)
        );
        assert_eq!(
            child.messages[1].metadata.parent_message_id,
            Some(child.messages[0].id)
        );

        let child_events = manager
            .store
            .list_session_events(child.id)
            .await
            .expect("load copied child events");
        assert!(child_events.iter().any(|event| {
            matches!(event.kind, crate::event::EventKind::UserMessageAppended(_))
        }));
        assert!(child_events.iter().all(|event| match &event.kind {
            crate::event::EventKind::MessagePartCheckpointed(payload) => {
                payload.session_id == child.id
            }
            _ => true,
        }));
    }

    #[tokio::test]
    async fn session_export_uses_one_current_unversioned_format() {
        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "current export format".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create export source");
        let bundle = manager
            .export_session_jsonl(session.id)
            .await
            .expect("export current session");
        let header = bundle.lines().next().expect("export header");
        let mut header_value = serde_json::from_str::<serde_json::Value>(header)
            .expect("decode current export header");
        assert!(
            header_value.get("schema").is_none(),
            "current-only exports must not carry a schema generation"
        );
        manager
            .import_session_jsonl(bundle.as_str())
            .await
            .expect("import matching current export");

        header_value
            .as_object_mut()
            .expect("object export header")
            .insert("schema".to_owned(), serde_json::json!(1));
        let versioned_bundle = format!(
            "{}\n",
            serde_json::to_string(&header_value).expect("encode versioned header")
        );
        let error = manager
            .import_session_jsonl(versioned_bundle.as_str())
            .await
            .expect_err("versioned export headers are not accepted");
        assert!(error.to_string().contains("unknown field `schema`"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn host_invoked_streaming_tool_updates_outer_tool_api_operation() {
        let manager = test_manager().await;
        let call_id = 73;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "Tool API stream regression".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create test session");
        let session = install_pending_tool_api_operation(&manager, session, call_id).await;

        let execution = manager
            .execute_host_invoked_tool(
                session.id,
                call_id,
                ToolInvocation::new("test.stream.emit", StructuredObject::default()),
            )
            .await
            .expect("execute streaming tool");

        // The ordinary handler deliberately returns a different value. This
        // proves the host path called `tool_invoke_stream`, not `tool_invoke`.
        assert_eq!(execution.summary().output_text, "stream-terminal");

        let session = manager
            .get_session(session.id)
            .await
            .expect("reload streamed Tool API session");
        let part = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find(|part| part.operation_id.as_deref() == Some("tool-api-stream-test"))
            .expect("outer Tool API operation remains present");
        assert_eq!(part.status, ExecutionStatus::InProgress);
        let PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) =
            part.content.as_ref().expect("operation content")
        else {
            panic!("Tool API stream test part is not an operation");
        };
        assert_eq!(operation.model_output.text, "stream-handler");
    }

    async fn wait_for_host_permission_request(manager: &SessionManager, session_id: i64) -> String {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let session = manager
                    .get_session(session_id)
                    .await
                    .expect("reload host permission session");
                if let Some(request) = session
                    .pending_interactive_requests()
                    .into_iter()
                    .find(|request| request.request_id().starts_with("host-permission:"))
                {
                    return request.request_id().to_string();
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("host permission request was not persisted")
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn dynamic_host_permission_asks_resumes_and_scopes_the_exact_grant() {
        let manager = Arc::new(
            test_manager_with_tool_policy(ToolPermissionPolicy::new(
                agena_domain::PermissionMode::Allow,
            ))
            .await,
        );
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "dynamic host permission".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create dynamic permission session");
        let call_id = 501;
        session = install_pending_tool_api_operation(&manager, session, call_id).await;
        let action = PermissionAction::NetworkAccess {
            target: "example.com:443".to_string(),
            host: "example.com".to_string(),
            port: Some(443),
        };
        let guard = HostPermissionGrantGuard::install(
            Arc::clone(&manager.host_permission_grants),
            session.id,
            call_id,
            "test.dynamic".to_string(),
            "fetch".to_string(),
            Vec::new(),
        );
        let authorize_manager = Arc::clone(&manager);
        let authorize_action = action.clone();
        let authorize = tokio::spawn(async move {
            authorize_manager
                .authorize_host_action(
                    session.id,
                    call_id,
                    "test.dynamic",
                    "fetch",
                    agena_tool::ToolPermissionCheck {
                        action: authorize_action,
                        decision: agena_domain::PermissionDecision::Ask {
                            reason: "network access requires approval".to_string(),
                        },
                    },
                )
                .await
        });
        let request_id = wait_for_host_permission_request(&manager, session.id).await;
        let options = SessionRunOptions {
            model: ModelRef::new("permission-test-provider", "permission-test-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                options,
                PermissionReply {
                    request_id,
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            ))
            .await
            .expect("approve dynamic host action");
        assert!(matches!(
            authorize
                .await
                .expect("join authorization")
                .expect("authorize"),
            agena_runtime::HostActionAuthorization::Allowed
        ));
        assert!(manager.has_host_permission_grant(
            session.id,
            call_id,
            "test.dynamic",
            "fetch",
            &action,
        ));
        let second_action = PermissionAction::NetworkAccess {
            target: "different.example:443".to_string(),
            host: "different.example".to_string(),
            port: Some(443),
        };
        assert!(!manager.has_host_permission_grant(
            session.id,
            call_id,
            "test.dynamic",
            "fetch",
            &second_action,
        ));
        let second_manager = Arc::clone(&manager);
        let second_check_action = second_action.clone();
        let mut second_authorize = tokio::spawn(async move {
            second_manager
                .authorize_host_action(
                    session.id,
                    call_id,
                    "test.dynamic",
                    "fetch",
                    agena_tool::ToolPermissionCheck {
                        action: second_check_action,
                        decision: agena_domain::PermissionDecision::Ask {
                            reason: "a different host requires a new approval".to_string(),
                        },
                    },
                )
                .await
        });
        let second_request_id = tokio::select! {
            request_id = wait_for_host_permission_request(&manager, session.id) => request_id,
            result = &mut second_authorize => panic!("second authorization terminated before asking: {result:?}"),
        };
        manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("permission-test-provider", "permission-test-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                PermissionReply {
                    request_id: second_request_id.clone(),
                    kind: PermissionReplyKind::DenyOnce,
                    reason: Some("not this host".to_string()),
                    scope: None,
                },
                None,
            ))
            .await
            .expect("decline second dynamic action");
        let outcome = second_authorize
            .await
            .expect("join second authorization")
            .expect("authorize second action");
        let agena_runtime::HostActionAuthorization::UserDeclined(decline) = outcome else {
            panic!("unexpected second authorization outcome: {outcome:?}");
        };
        assert_eq!(decline.request_id, second_request_id);
        assert!(!manager.has_host_permission_grant(
            session.id,
            call_id,
            "test.dynamic",
            "fetch",
            &second_action,
        ));
        drop(guard);
        assert!(!manager.has_host_permission_grant(
            session.id,
            call_id,
            "test.dynamic",
            "fetch",
            &action,
        ));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn permission_reply_releases_session_lock_before_tool_continuation() {
        let manager = Arc::new(test_manager().await);
        let call_id = 91;
        let request_id = "reply-lock-probe".to_string();
        let options = SessionRunOptions {
            model: ModelRef::new("reply-test-provider", "reply-test-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "permission reply lock regression".to_string(),
                parent_session_id: None,
            })
            .await
            .expect("create test session");
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve reply probe message ids");
        let invocation = ToolInvocation::new("test.reply_probe.run", StructuredObject::default());
        let operation = OperationPart::pending(
            call_id,
            invocation,
            "Tool reply_probe.run",
            TimeRange::default(),
        );
        let metadata = MessageMetadata {
            turn_id: Some(1),
            model_provider_id: options.model.provider_id.to_string(),
            model_id: options.model.model_id.to_string(),
            ..MessageMetadata::default()
        };
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(operation)],
            metadata,
        );
        message.parts[0].operation_id = Some("reply-lock-operation".to_string());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![message],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist reply probe operation");
        let pending = session
            .next_pending_tool()
            .expect("pending reply probe tool");
        let action = PermissionAction::Tool {
            tool_name: "test.reply_probe.run".to_string(),
            qualifier: None,
        };
        session = manager
            .apply_permission_request_with_id(
                session,
                &pending,
                request_id.clone(),
                action.clone(),
                Vec::new(),
                vec![action],
                "reply lock regression".to_string(),
                String::new(),
                Some("static_policy".to_string()),
                None,
                None,
                PermissionRiskLevel::Medium,
                Vec::new(),
                manager.execution_state(),
            )
            .await
            .expect("persist reply probe permission request");

        let session_id = session.id;
        let outcome = tokio::time::timeout(
            Duration::from_secs(5),
            manager.start_reply_permission(SessionPermissionReplyRequest::new(
                session_id,
                options,
                PermissionReply {
                    request_id,
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            )),
        )
        .await
        .expect("permission reply was not accepted")
        .expect("start permission reply");
        assert!(
            outcome.receipt.is_some(),
            "permission continuation must return an execution receipt"
        );

        tokio::select! {
            _ = REPLY_PROBE_STARTED.notified() => {}
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("approved tool continuation did not start")
            }
        }
        let lock_is_available = tokio::time::timeout(Duration::from_secs(1), async {
            let lock = manager.reply_session_lock(session_id).await;
            let _guard = lock.lock().await;
        })
        .await
        .is_ok();
        REPLY_PROBE_CONTINUE.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.is_run_active(session_id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("permission reply continuation did not terminate");

        assert!(
            lock_is_available,
            "permission reply held the session lock while executing the approved tool"
        );
    }

    #[test]
    fn host_permission_grant_covers_only_public_dns_resolution() {
        let granted = vec![PermissionAction::NetworkAccess {
            target: "https://openai.com/".to_string(),
            host: "openai.com".to_string(),
            port: Some(443),
        }];
        let public_address = PermissionAction::NetworkAccess {
            target: "104.18.33.45:443".to_string(),
            host: "104.18.33.45".to_string(),
            port: Some(443),
        };
        let private_address = PermissionAction::NetworkAccess {
            target: "10.0.0.1:443".to_string(),
            host: "10.0.0.1".to_string(),
            port: Some(443),
        };

        assert!(host_permission_grant_matches_action(
            &granted,
            &public_address
        ));
        assert!(!host_permission_grant_matches_action(
            &granted,
            &private_address
        ));
    }
}
