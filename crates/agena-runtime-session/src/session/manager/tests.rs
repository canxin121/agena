#[cfg(test)]
use super::*;

#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::session::store::MessageCheckpoint;
    use crate::{AppError, RuntimeSessionManagerConfig};
    use std::{
        collections::{HashMap, HashSet},
        sync::Arc,
        time::Duration,
    };

    use agena_domain::{
        ExecutionStatus, FinishReason, PermissionAction, PermissionDecision, PermissionReplyKind,
        StructuredObject, TimeRange, UserInputQuestion, UserInputReplyKind,
    };
    use chrono::Utc;
    use sea_orm::{ConnectionTrait, Database, Statement};
    use tokio::sync::Notify;

    use super::{ExecutionConversationTarget, SessionManager, build_message, merge_system_prompts};
    use crate::session::history::{
        AssistantMessageFinished, RunCompleted, RunStarted, TranscriptContent, UserMessageAppended,
    };
    use crate::session_execution_service::SessionExecutionCommandService;
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
        SessionCreateRequest, SessionExecutionReplyRequest, SessionExecutionRequest,
        SessionPermissionReplyRequest, SessionPluginCommandRequest, SessionPluginCommandService,
        SessionRewindRequest, SessionRunOptions, SessionToolExecutionService,
    };
    use agena_tool::ToolPermissionCheck;

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
    static REPLY_PROBE_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);

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
            REPLY_PROBE_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            REPLY_PROBE_STARTED.notify_one();
            REPLY_PROBE_CONTINUE.notified().await;
            "reply-probe-complete".to_string()
        }
    }

    #[derive(Default)]
    struct ApprovedFailureTool;

    static APPROVED_SUCCESS_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    static BATCH_SLOW_STARTED: Notify = Notify::const_new();
    static BATCH_SLOW_RELEASE: Notify = Notify::const_new();
    static PERMISSION_BATCH_FAST_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    static PERMISSION_BATCH_SLOW_CALLS: std::sync::atomic::AtomicUsize =
        std::sync::atomic::AtomicUsize::new(0);
    static PERMISSION_BATCH_SLOW_STARTED: Notify = Notify::const_new();
    static PERMISSION_BATCH_SLOW_RELEASE: Notify = Notify::const_new();

    #[derive(Default)]
    struct ApprovedSuccessTool;

    #[derive(Default)]
    struct BatchBarrierTool;

    #[derive(Default)]
    struct PermissionBatchTool;

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "batch_barrier",
        version = "0.1.0",
        summary = "Concurrent tool-batch barrier regression fixture."
    )]
    impl BatchBarrierTool {
        #[tool(
            name = "fast",
            summary = "Complete immediately.",
            read_only,
            concurrency_safe
        )]
        async fn fast(&self) -> serde_json::Value {
            serde_json::json!({ "result": "fast-complete" })
        }

        #[tool(
            name = "slow",
            summary = "Wait for the batch-barrier test.",
            read_only,
            concurrency_safe
        )]
        async fn slow(&self) -> serde_json::Value {
            BATCH_SLOW_STARTED.notify_one();
            BATCH_SLOW_RELEASE.notified().await;
            serde_json::json!({ "result": "slow-complete" })
        }
    }

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "permission_batch",
        version = "0.1.0",
        summary = "Approved concurrent tool-batch regression fixture."
    )]
    impl PermissionBatchTool {
        #[tool(
            name = "fast",
            summary = "Complete immediately after batch approval.",
            read_only,
            concurrency_safe
        )]
        async fn fast(&self) -> serde_json::Value {
            PERMISSION_BATCH_FAST_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            serde_json::json!({ "result": "fast-approved" })
        }

        #[tool(
            name = "slow",
            summary = "Wait after batch approval.",
            read_only,
            concurrency_safe
        )]
        async fn slow(&self) -> serde_json::Value {
            PERMISSION_BATCH_SLOW_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            PERMISSION_BATCH_SLOW_STARTED.notify_one();
            PERMISSION_BATCH_SLOW_RELEASE.notified().await;
            serde_json::json!({ "result": "slow-approved" })
        }
    }

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "approved_success",
        version = "0.1.0",
        summary = "Approved execution success regression fixture."
    )]
    impl ApprovedSuccessTool {
        #[tool(name = "run", summary = "Complete after approval.", read_only)]
        async fn run(&self) -> serde_json::Value {
            APPROVED_SUCCESS_CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            serde_json::json!({ "approved": true })
        }
    }

    #[agena_plugin_host::sdk::agena_plugin(
        namespace = "test",
        name = "approved_failure",
        version = "0.1.0",
        summary = "Approved execution failure regression fixture."
    )]
    impl ApprovedFailureTool {
        #[tool(name = "run", summary = "Fail after approval.")]
        async fn run(&self) -> agena_plugin_host::sdk::Result<String> {
            Err(agena_plugin_host::sdk::PluginError::internal(
                "approved fixture failed with its real diagnostic",
            ))
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

    struct ApprovalTestProvider {
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
            Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.default_model.clone(),
                text: "continued after approved tool".to_owned(),
                reasoning_text: None,
                finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }
    }

    #[async_trait::async_trait]
    impl ModelRuntime for ApprovalTestProvider {
        fn id(&self) -> &str {
            "approval-test-provider"
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
            Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.default_model.clone(),
                text: "ALLOW".to_owned(),
                reasoning_text: None,
                finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }
    }

    async fn test_manager() -> SessionManager {
        test_manager_with_tool_policy(ToolPermissionPolicy::allow_all()).await
    }

    async fn test_manager_with_tool_policy(tool_policy: ToolPermissionPolicy) -> SessionManager {
        test_manager_with_permission(tool_policy, Default::default()).await
    }

    async fn test_manager_with_permission(
        tool_policy: ToolPermissionPolicy,
        permission: agena_domain::PermissionConfig,
    ) -> SessionManager {
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
        plugins_config.list.insert(
            "test.approved_failure".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.approved_success".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.batch_barrier".to_string(),
            ConfiguredPlugin::static_default(),
        );
        plugins_config.list.insert(
            "test.permission_batch".to_string(),
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
                StaticPluginRegistration::new(
                    "test.approved_failure"
                        .parse()
                        .expect("valid approved failure plugin key"),
                    ApprovedFailureTool,
                ),
                StaticPluginRegistration::new(
                    "test.approved_success"
                        .parse()
                        .expect("valid approved success plugin key"),
                    ApprovedSuccessTool,
                ),
                StaticPluginRegistration::new(
                    "test.batch_barrier"
                        .parse()
                        .expect("valid batch barrier plugin key"),
                    BatchBarrierTool,
                ),
                StaticPluginRegistration::new(
                    "test.permission_batch"
                        .parse()
                        .expect("valid permission batch plugin key"),
                    PermissionBatchTool,
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
        providers.register(ApprovalTestProvider {
            default_model: ModelId::new("approval-test-model"),
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
            RuntimeSessionManagerConfig {
                permission,
                ..RuntimeSessionManagerConfig::default()
            },
        )
    }

    #[tokio::test]
    async fn get_session_rebuilds_stale_effective_permission_after_reload() {
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "permission refresh".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create permission refresh session");

        let mut stale = manager
            .store
            .load_session(session.id, manager.execution_state().cache_policy())
            .await
            .expect("load permission refresh session");
        stale.runtime.execution.effective_permission = agena_domain::PermissionConfig {
            path: Some(agena_domain::PathPermissionConfig {
                workspace: Some(agena_domain::PathAccessModes {
                    read: Some(agena_domain::PermissionMode::Ask),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            ..Default::default()
        };
        manager
            .persist_session_changes(
                stale,
                Vec::new(),
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist stale permission snapshot");

        let refreshed = manager
            .get_session(session.id)
            .await
            .expect("reload permission refresh session");
        assert_eq!(
            refreshed
                .runtime
                .execution
                .effective_permission
                .path
                .and_then(|path| path.workspace)
                .and_then(|modes| modes.read),
            Some(agena_domain::PermissionMode::Allow),
            "a persisted old Ask snapshot must not survive a current config reload"
        );
    }

    #[tokio::test]
    async fn automatic_permission_uses_the_configured_model_and_returns_allow() {
        let mut permission = agena_domain::PermissionConfig::global_default();
        permission.approval_model = Some(agena_domain::ApprovalModelSelection {
            provider_id: "approval-test-provider".to_owned(),
            adapter_id: None,
            model_id: "approval-test-model".to_owned(),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            parallel_tool_calls: None,
        });
        let manager =
            test_manager_with_permission(ToolPermissionPolicy::allow_all(), permission).await;
        let outcome = manager
            .aggregate_permission_outcome(
                None,
                &[ToolPermissionCheck {
                    action: PermissionAction::PathAccess {
                        access_kind: "write".to_owned(),
                        workspace_root: "/workspace".to_owned(),
                        target_path: "/workspace/file.txt".to_owned(),
                    },
                    decision: PermissionDecision::Auto {
                        reason: "workspace write is eligible for automatic approval".to_owned(),
                    },
                    contract: agena_domain::ToolPermissionContract::default(),
                }],
            )
            .await
            .expect("automatic approval should resolve");

        assert!(matches!(
            outcome,
            super::replies::AggregatedPermissionOutcome::Allow
        ));
    }

    #[tokio::test]
    async fn automatic_permission_falls_back_to_session_model_without_approval_model() {
        // The default permission config has no approval_model; auto must still
        // resolve through the classifier using the session model instead of
        // asking interactively.
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "auto fallback".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create auto fallback session");
        manager
            .update_session_selection(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("approval-test-provider", "approval-test-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
            )
            .await
            .expect("select approval-test model for the session");

        let outcome = manager
            .aggregate_permission_outcome(
                Some(&session),
                &[ToolPermissionCheck {
                    action: PermissionAction::Tool {
                        tool_name: "fs.write".to_owned(),
                        qualifier: None,
                    },
                    decision: PermissionDecision::Auto {
                        reason: "tool is eligible for automatic approval".to_owned(),
                    },
                    contract: agena_domain::ToolPermissionContract {
                        input_paths: vec![agena_domain::InputPathSpec {
                            jsonpath: "$.path".to_owned(),
                            kind: agena_domain::PathKind::Write,
                            fallback: None,
                            optional: false,
                        }],
                        ..agena_domain::ToolPermissionContract::default()
                    },
                }],
            )
            .await
            .expect("automatic approval should resolve without an approval model");

        assert!(matches!(
            outcome,
            super::replies::AggregatedPermissionOutcome::Allow
        ));
    }

    #[tokio::test]
    async fn path_granted_tool_ask_is_allowed_when_every_path_check_allows() {
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let outcome = manager
            .aggregate_permission_outcome(
                None,
                &[
                    ToolPermissionCheck {
                        action: PermissionAction::Tool {
                            tool_name: "fs.apply_patch".to_owned(),
                            qualifier: None,
                        },
                        decision: PermissionDecision::Ask {
                            reason: "tool 'fs.apply_patch' requires confirmation by policy"
                                .to_owned(),
                        },
                        contract: agena_domain::ToolPermissionContract {
                            input_paths: vec![agena_domain::InputPathSpec {
                                jsonpath: "$.path".to_owned(),
                                kind: agena_domain::PathKind::Write,
                                fallback: None,
                                optional: false,
                            }],
                            ..agena_domain::ToolPermissionContract::default()
                        },
                    },
                    ToolPermissionCheck {
                        action: PermissionAction::PathAccess {
                            access_kind: "write".to_owned(),
                            workspace_root: "/workspace".to_owned(),
                            target_path: "/workspace/crates/x.rs".to_owned(),
                        },
                        decision: PermissionDecision::Allow,
                        contract: agena_domain::ToolPermissionContract::default(),
                    },
                ],
            )
            .await
            .expect("path-granted tool ask should resolve");
        assert!(matches!(
            outcome,
            super::replies::AggregatedPermissionOutcome::Allow
        ));
    }

    #[tokio::test]
    async fn path_granted_override_requires_all_path_checks_to_allow() {
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let outcome = manager
            .aggregate_permission_outcome(
                None,
                &[
                    ToolPermissionCheck {
                        action: PermissionAction::Tool {
                            tool_name: "fs.apply_patch".to_owned(),
                            qualifier: None,
                        },
                        decision: PermissionDecision::Ask {
                            reason: "tool 'fs.apply_patch' requires confirmation by policy"
                                .to_owned(),
                        },
                        contract: agena_domain::ToolPermissionContract {
                            input_paths: vec![agena_domain::InputPathSpec {
                                jsonpath: "$.path".to_owned(),
                                kind: agena_domain::PathKind::Write,
                                fallback: None,
                                optional: false,
                            }],
                            ..agena_domain::ToolPermissionContract::default()
                        },
                    },
                    ToolPermissionCheck {
                        action: PermissionAction::PathAccess {
                            access_kind: "write".to_owned(),
                            workspace_root: "/workspace".to_owned(),
                            target_path: "/workspace/inside.rs".to_owned(),
                        },
                        decision: PermissionDecision::Allow,
                        contract: agena_domain::ToolPermissionContract::default(),
                    },
                    ToolPermissionCheck {
                        action: PermissionAction::PathAccess {
                            access_kind: "write".to_owned(),
                            workspace_root: "/workspace".to_owned(),
                            target_path: "/tmp/outside.rs".to_owned(),
                        },
                        decision: PermissionDecision::Ask {
                            reason: "external write requires confirmation".to_owned(),
                        },
                        contract: agena_domain::ToolPermissionContract::default(),
                    },
                ],
            )
            .await
            .expect("mixed path outcome should resolve");
        assert!(
            matches!(
                outcome,
                super::replies::AggregatedPermissionOutcome::Request(_)
            ),
            "an external path that still needs confirmation must keep the tool ask"
        );
    }

    #[tokio::test]
    async fn path_granted_override_never_lifts_tool_deny() {
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let outcome = manager
            .aggregate_permission_outcome(
                None,
                &[
                    ToolPermissionCheck {
                        action: PermissionAction::Tool {
                            tool_name: "shell.run".to_owned(),
                            qualifier: None,
                        },
                        decision: PermissionDecision::Deny {
                            reason:
                                "bash command matches deny pattern and is unconditionally blocked"
                                    .to_owned(),
                        },
                        contract: agena_domain::ToolPermissionContract {
                            shell: true,
                            ..agena_domain::ToolPermissionContract::default()
                        },
                    },
                    ToolPermissionCheck {
                        action: PermissionAction::PathAccess {
                            access_kind: "write".to_owned(),
                            workspace_root: "/workspace".to_owned(),
                            target_path: "/workspace/out".to_owned(),
                        },
                        decision: PermissionDecision::Allow,
                        contract: agena_domain::ToolPermissionContract::default(),
                    },
                ],
            )
            .await
            .expect("deny must stay authoritative");
        assert!(matches!(
            outcome,
            super::replies::AggregatedPermissionOutcome::Deny(_)
        ));
    }

    #[tokio::test]
    async fn path_granted_override_never_applies_to_arbitrary_execution_tools() {
        let manager = test_manager_with_permission(
            ToolPermissionPolicy::allow_all(),
            agena_domain::PermissionConfig::global_default(),
        )
        .await;
        let outcome = manager
            .aggregate_permission_outcome(
                None,
                &[
                    ToolPermissionCheck {
                        action: PermissionAction::Tool {
                            tool_name: "shell.run".to_owned(),
                            qualifier: None,
                        },
                        decision: PermissionDecision::Ask {
                            reason: "tool 'shell.run' requires confirmation by policy".to_owned(),
                        },
                        contract: agena_domain::ToolPermissionContract {
                            shell: true,
                            input_paths: vec![agena_domain::InputPathSpec {
                                jsonpath: "$.path".to_owned(),
                                kind: agena_domain::PathKind::Write,
                                fallback: None,
                                optional: false,
                            }],
                            ..agena_domain::ToolPermissionContract::default()
                        },
                    },
                    ToolPermissionCheck {
                        action: PermissionAction::PathAccess {
                            access_kind: "write".to_owned(),
                            workspace_root: "/workspace".to_owned(),
                            target_path: "/workspace/out.txt".to_owned(),
                        },
                        decision: PermissionDecision::Allow,
                        contract: agena_domain::ToolPermissionContract::default(),
                    },
                ],
            )
            .await
            .expect("shell tools must keep their tool-level ask");
        assert!(
            matches!(
                outcome,
                super::replies::AggregatedPermissionOutcome::Request(_)
            ),
            "allowing writes inside the workspace must not authorize arbitrary shell execution"
        );
    }

    async fn seed_canonical_assistant_reply(
        manager: &SessionManager,
        session_id: i64,
    ) -> (
        agena_domain::ExecutionId,
        agena_domain::TurnId,
        agena_domain::AssistantReplyId,
    ) {
        let execution_id = agena_domain::ExecutionId::new();
        let turn_id = agena_domain::TurnId::new();
        let reply_id = agena_domain::AssistantReplyId::new();
        manager
            .store
            .append_lifecycle_events(
                session_id,
                vec![EventKind::ExecutionStarted(
                    agena_domain::ExecutionStartedEvent {
                        session_id,
                        execution_id,
                        turn_id,
                        reply_id,
                        source: ExecutionSource::User,
                        ts_ms: 1,
                    },
                )],
            )
            .await
            .expect("seed canonical assistant reply");
        (execution_id, turn_id, reply_id)
    }

    async fn checkpoint_seeded_assistant_message(
        manager: &SessionManager,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
        message: &crate::message::Message,
    ) {
        let mut events = message
            .parts
            .iter()
            .cloned()
            .map(|part| {
                EventKind::MessagePartCheckpointed(crate::event::MessagePartCheckpointedEvent {
                    session_id,
                    execution_id: Some(execution_id),
                    run_id: None,
                    turn_id: Some(turn_id),
                    reply_id: Some(reply_id),
                    message_id: message.id,
                    message_role: message.role,
                    message_state: message.state,
                    message_created_at: message.created_at,
                    message_metadata: message.metadata.clone(),
                    part,
                    ts_ms: Utc::now().timestamp_millis(),
                })
            })
            .collect::<Vec<_>>();
        events.push(EventKind::ExecutionFinished(
            agena_domain::ExecutionFinishedEvent {
                session_id,
                execution_id,
                reply_id,
                outcome: agena_domain::ExecutionOutcome::Completed,
                ts_ms: Utc::now().timestamp_millis(),
            },
        ));
        manager
            .store
            .append_lifecycle_events(session_id, events)
            .await
            .expect("checkpoint seeded canonical assistant message");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approved_execution_failure_terminalizes_the_original_permission_once() {
        let manager = test_manager().await;
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
                title: "approved failure terminalization".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create approved failure session");
        let (seed_execution_id, seed_turn_id, seed_reply_id) =
            seed_canonical_assistant_reply(&manager, session.id).await;
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve approved failure message ids");
        let operation_id = "approved-failure-operation";
        let operation = OperationPart::pending(
            41,
            ToolInvocation::new("test.approved_failure.run", StructuredObject::default()),
            "Run approved_failure.run",
            TimeRange::default(),
        );
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(operation)],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: options.model.provider_id.to_string(),
                model_id: options.model.model_id.to_string(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some(operation_id.to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist approved failure operation");
        let pending = session
            .next_pending_tool()
            .expect("approved failure pending tool");
        let action = PermissionAction::Tool {
            tool_name: "test.approved_failure.run".to_owned(),
            qualifier: None,
        };
        for _ in 0..2 {
            session = manager
                .apply_permission_request_with_id(
                    session,
                    &pending,
                    operation_id.to_owned(),
                    action.clone(),
                    vec![action.clone()],
                    vec![action.clone()],
                    "approved failure regression".to_owned(),
                    "The fixture requires explicit approval.".to_owned(),
                    Some("static_policy".to_owned()),
                    None,
                    None,
                    Vec::new(),
                    manager.execution_state(),
                )
                .await
                .expect("upsert one permission request");
        }
        checkpoint_seeded_assistant_message(
            &manager,
            session.id,
            seed_execution_id,
            seed_turn_id,
            seed_reply_id,
            session.messages.last().expect("seeded assistant message"),
        )
        .await;

        let permission_record_count = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .filter_map(|part| match part.content.as_ref() {
                Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                    operation,
                ))) => Some(operation.authorization.permissions.len()),
                _ => None,
            })
            .sum::<usize>();
        assert_eq!(permission_record_count, 1);
        let message = session
            .messages
            .iter()
            .find(|message| {
                message
                    .parts
                    .iter()
                    .any(|part| part.operation_id.as_deref() == Some(operation_id))
            })
            .expect("message owning approved failure operation");
        assert_eq!(
            message
                .parts
                .iter()
                .map(|part| part.part_index)
                .collect::<Vec<_>>(),
            vec![0],
            "permission must remain data on the single operation part"
        );
        let events = manager
            .list_session_events(session.id)
            .await
            .expect("load permission request events");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::PermissionRequested(_)))
                .count(),
            1
        );

        let session = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                options,
                PermissionReply {
                    request_id: operation_id.to_owned(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            ))
            .await
            .expect("approved failure becomes a normal failed operation");

        assert!(session.pending_interactive_requests().is_empty());
        assert!(session.next_pending_tool().is_none());
        assert!(session.has_finished_operation(operation_id));
        let parts = session
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .collect::<Vec<_>>();
        assert!(parts.iter().any(|part| {
            part.operation_id.as_deref() == Some(operation_id)
                && part.status == ExecutionStatus::Failed
                && matches!(
                    part.content.as_ref(),
                    Some(PartContent::Activity(
                        crate::message::RuntimeActivity::Operation(operation)
                    ))
                        if operation.authorization.permissions.len() == 1
                            && operation.authorization.permissions[0].reply.is_some()
                            && operation.title == "test.approved_failure.run"
                            && !operation.title.contains("Awaiting permission")
                )
        }));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn durable_permission_reply_resumes_the_existing_execution_without_partial_failure() {
        let manager = test_manager().await;
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
                title: "active permission reply".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create session");
        let (seed_execution_id, turn_id, reply_id) =
            seed_canonical_assistant_reply(&manager, session.id).await;
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve operation ids");
        let operation_id = "active-permission-operation";
        let operation = OperationPart::pending(
            43,
            ToolInvocation::new("test.active_permission.run", StructuredObject::default()),
            "test.active_permission.run",
            TimeRange::default(),
        );
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(operation)],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: options.model.provider_id.to_string(),
                model_id: options.model.model_id.to_string(),
                ..Default::default()
            },
        );
        message.parts[0].operation_id = Some(operation_id.to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist operation");
        let pending = session.next_pending_tool().expect("pending operation");
        let action = PermissionAction::Tool {
            tool_name: "test.active_permission.run".to_owned(),
            qualifier: None,
        };
        session = manager
            .apply_permission_request_with_id(
                session,
                &pending,
                operation_id.to_owned(),
                action.clone(),
                Vec::new(),
                vec![action],
                "active execution approval".to_owned(),
                String::new(),
                Some("static_policy".to_owned()),
                None,
                None,
                Vec::new(),
                manager.execution_state(),
            )
            .await
            .expect("persist permission request");
        checkpoint_seeded_assistant_message(
            &manager,
            session.id,
            seed_execution_id,
            turn_id,
            reply_id,
            session.messages.last().expect("assistant message"),
        )
        .await;

        let release = Arc::new(Notify::new());
        let release_execution = Arc::clone(&release);
        let active = manager
            .start_registered(
                session.id,
                ExecutionSource::Continue,
                ExecutionConversationTarget::ExistingReply(super::ConversationIdentity {
                    turn_id,
                    reply_id,
                }),
                "active permission waiter",
                move |_manager, _control, _steer_rx| async move {
                    release_execution.notified().await;
                    Ok::<_, AppError>(())
                },
            )
            .await
            .expect("start active execution");
        let active_receipt = active.receipt.expect("active receipt");

        let reply_outcome = manager
            .start_reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                options,
                PermissionReply {
                    request_id: operation_id.to_owned(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: Some("approved".to_owned()),
                    scope: None,
                },
                None,
            ))
            .await
            .expect("durable reply must not fail after persistence");
        let reply_receipt = reply_outcome.receipt.expect("resumed receipt");
        assert_eq!(reply_receipt.execution_id, active_receipt.execution_id);
        assert_eq!(reply_receipt.turn_id, turn_id);
        assert_eq!(reply_receipt.reply_id, reply_id);

        let stored = manager
            .get_session(session.id)
            .await
            .expect("load replied session");
        let authorization = stored
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find_map(|part| match part.content.as_ref() {
                Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(
                    operation,
                ))) if part.operation_id.as_deref() == Some(operation_id) => {
                    Some(&operation.authorization)
                }
                _ => None,
            })
            .expect("operation authorization");
        assert_eq!(authorization.permissions.len(), 1);
        assert_eq!(
            authorization.permissions[0]
                .reply
                .as_ref()
                .map(|reply| reply.kind),
            Some(PermissionReplyKind::AllowOnce)
        );

        release.notify_one();
        tokio::time::timeout(Duration::from_secs(5), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("active execution finished");
    }

    #[tokio::test]
    async fn session_commit_checkpoints_only_the_explicitly_changed_part() {
        let manager = test_manager().await;
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "checkpoint delta".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create session");
        let ids = manager
            .store
            .reserve_message_ids(3)
            .await
            .expect("reserve message ids");
        let message = build_message(
            ids,
            Role::User,
            ExecutionStatus::Completed,
            vec![
                PartContent::text("first"),
                PartContent::text("second"),
                PartContent::text("third"),
            ],
            MessageMetadata::default(),
        );
        let message_id = message.id;
        let changed_part_id = message.parts[1].id;
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist initial document");
        let before = manager
            .list_session_events(session.id)
            .await
            .expect("initial events")
            .into_iter()
            .filter(|event| {
                matches!(
                    &event.kind,
                    EventKind::MessagePartCheckpointed(checkpoint)
                        if checkpoint.message_id == message_id
                )
            })
            .count();
        assert_eq!(before, 3);

        let changed = session
            .messages
            .iter_mut()
            .find(|message| message.id == message_id)
            .and_then(|message| {
                message
                    .parts
                    .iter_mut()
                    .find(|part| part.id == changed_part_id)
            })
            .expect("changed part");
        changed.set_content(PartContent::text("second updated"));
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::part(message_id, changed_part_id)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist one part delta");

        let checkpoints = manager
            .list_session_events(session.id)
            .await
            .expect("updated events")
            .into_iter()
            .filter_map(|event| match event.kind {
                EventKind::MessagePartCheckpointed(checkpoint)
                    if checkpoint.message_id == message_id =>
                {
                    Some(checkpoint.part)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(checkpoints.len(), 4);
        assert_eq!(
            checkpoints.last().map(|part| part.id),
            Some(changed_part_id)
        );
        assert_eq!(
            checkpoints.last().and_then(|part| part.text()),
            Some("second updated")
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn approved_provider_tool_executes_once_then_continues_the_same_turn() {
        APPROVED_SUCCESS_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        let manager = test_manager().await;
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
                title: "approved provider continuation".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create approved continuation session");
        let (seed_execution_id, seed_turn_id, seed_reply_id) =
            seed_canonical_assistant_reply(&manager, session.id).await;
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve approved continuation message ids");
        let operation_id = "approved-provider-operation";
        let target_input = StructuredObject::default();
        let invocation = ToolInvocation {
            tool_api_call: Some(agena_domain::ToolApiCall {
                function: agena_domain::ToolApiFunction::Call,
                arguments: StructuredObject::try_from(serde_json::json!({
                    "tool": "test.approved_success.run",
                    "input": {}
                }))
                .expect("valid provider tool envelope"),
            }),
            name: "test.approved_success.run".to_owned(),
            plugin_name: None,
            input: target_input,
        };
        let operation =
            OperationPart::pending(42, invocation, "Run stream.object", TimeRange::default());
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(operation)],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: options.model.provider_id.to_string(),
                model_id: options.model.model_id.to_string(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some(operation_id.to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist approved continuation operation");
        manager
            .store
            .append_lifecycle_events(
                session.id,
                vec![EventKind::MessagePartCheckpointed(
                    crate::event::MessagePartCheckpointedEvent {
                        session_id: session.id,
                        execution_id: Some(seed_execution_id),
                        run_id: None,
                        turn_id: Some(seed_turn_id),
                        reply_id: Some(seed_reply_id),
                        message_id: message.id,
                        message_role: message.role,
                        message_state: message.state,
                        message_created_at: message.created_at,
                        message_metadata: message.metadata.clone(),
                        part: message.parts[0].clone(),
                        ts_ms: Utc::now().timestamp_millis(),
                    },
                )],
            )
            .await
            .expect("associate operation message with canonical reply execution");
        let pending = session
            .next_pending_tool()
            .expect("approved continuation pending tool");
        let action = PermissionAction::Tool {
            tool_name: "test.approved_success.run".to_owned(),
            qualifier: None,
        };
        session = manager
            .apply_permission_request_with_id(
                session,
                &pending,
                operation_id.to_owned(),
                action.clone(),
                vec![action.clone()],
                vec![action],
                "approved continuation regression".to_owned(),
                "The fixture requires explicit approval.".to_owned(),
                Some("static_policy".to_owned()),
                None,
                None,
                Vec::new(),
                manager.execution_state(),
            )
            .await
            .expect("persist approved continuation permission");
        let pending_permission = session
            .find_pending_permission_by_request_id(operation_id)
            .expect("pending permission on the operation");
        assert_eq!(pending_permission.request_id, operation_id);
        let operation = session
            .part(&pending_permission.tool.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                    Some(operation)
                }
                _ => None,
            })
            .expect("operation owns permission authorization");
        assert_eq!(operation.authorization.permissions.len(), 1);
        assert!(operation.authorization.permissions[0].reply.is_none());
        assert_eq!(
            manager
                .conversation_identity_for_message(
                    session.id,
                    pending_permission.tool.part.message_id,
                )
                .await
                .expect("resolve reply identity from durable model-message ownership"),
            super::ConversationIdentity {
                turn_id: seed_turn_id,
                reply_id: seed_reply_id,
            }
        );
        let user_messages_before = session
            .messages
            .iter()
            .filter(|message| message.role == Role::User)
            .count();

        let session = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                options,
                PermissionReply {
                    request_id: operation_id.to_owned(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            ))
            .await
            .expect("approved provider target continues the model turn");

        manager
            .store
            .append_lifecycle_events(
                session.id,
                vec![EventKind::ExecutionFinished(
                    agena_domain::ExecutionFinishedEvent {
                        session_id: session.id,
                        execution_id: seed_execution_id,
                        reply_id: seed_reply_id,
                        outcome: agena_domain::ExecutionOutcome::Completed,
                        ts_ms: Utc::now().timestamp_millis(),
                    },
                )],
            )
            .await
            .expect("close seeded origin execution");

        assert_eq!(
            APPROVED_SUCCESS_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one approval must execute one target call"
        );
        assert!(session.pending_interactive_requests().is_empty());
        assert!(session.next_pending_tool().is_none());
        assert!(session.has_finished_operation(operation_id));
        assert_eq!(
            session
                .messages
                .iter()
                .filter(|message| message.role == Role::User)
                .count(),
            user_messages_before,
            "a permission reply must not synthesize an empty user message"
        );
        assert_eq!(
            session.last_assistant_text().as_deref(),
            Some("continued after approved tool")
        );
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
                ExecutionConversationTarget::NewTurn,
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

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_tool_batch_resolution_waits_for_every_tool() {
        let manager = Arc::new(test_manager().await);
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "tool batch barrier".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create batch barrier session");
        let ids = manager
            .store
            .reserve_message_ids(2)
            .await
            .expect("reserve batch message ids");
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![
                PartContent::operation(OperationPart::pending(
                    71,
                    ToolInvocation::new("test.batch_barrier.fast", StructuredObject::default()),
                    "Run batch_barrier.fast",
                    TimeRange::default(),
                )),
                PartContent::operation(OperationPart::pending(
                    72,
                    ToolInvocation::new("test.batch_barrier.slow", StructuredObject::default()),
                    "Run batch_barrier.slow",
                    TimeRange::default(),
                )),
            ],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: "reply-test-provider".to_owned(),
                model_id: "reply-test-model".to_owned(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some("batch-fast".to_owned());
        message.parts[1].operation_id = Some("batch-slow".to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist pending tool batch");
        let pending_tools = session.pending_tools();
        assert_eq!(pending_tools.len(), 2);

        let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
        request_override.set_parallel_tool_calls(Some(true));
        let options = SessionRunOptions {
            model: ModelRef::new("reply-test-provider", "reply-test-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        let runner = manager.background_handle();
        let state = manager.execution_state();
        let mut batch = tokio::spawn(async move {
            runner
                .resolve_pending_tools(session, pending_tools, &options, state)
                .await
        });

        tokio::select! {
            _ = BATCH_SLOW_STARTED.notified() => {}
            result = &mut batch => panic!("tool batch finished before the slow tool started: {result:?}"),
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("slow batch tool did not start before the timeout")
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(
            !batch.is_finished(),
            "batch resolution must not return while one concurrent tool is still running"
        );

        BATCH_SLOW_RELEASE.notify_one();
        let session = tokio::time::timeout(Duration::from_secs(5), batch)
            .await
            .expect("batch execution completed")
            .expect("join batch execution")
            .expect("resolve complete batch");
        assert!(session.next_pending_tool().is_none());
        assert!(session.has_finished_operation("batch-fast"));
        assert!(session.has_finished_operation("batch-slow"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_tool_batch_materializes_every_permission_before_blocking() {
        PERMISSION_BATCH_FAST_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        PERMISSION_BATCH_SLOW_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
        let manager = Arc::new(
            test_manager_with_tool_policy(ToolPermissionPolicy::new(
                agena_domain::PermissionMode::Ask,
            ))
            .await,
        );
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "parallel permission batch".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create parallel permission session");
        let (seed_execution_id, seed_turn_id, seed_reply_id) =
            seed_canonical_assistant_reply(manager.as_ref(), session.id).await;
        let ids = manager
            .store
            .reserve_message_ids(2)
            .await
            .expect("reserve permission batch message ids");
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![
                PartContent::operation(OperationPart::pending(
                    81,
                    ToolInvocation {
                        tool_api_call: Some(agena_domain::ToolApiCall {
                            function: agena_domain::ToolApiFunction::Call,
                            arguments: StructuredObject::try_from(serde_json::json!({
                                "tool": "test.permission_batch.fast",
                                "input": {}
                            }))
                            .expect("valid fast provider tool envelope"),
                        }),
                        name: "test.permission_batch.fast".to_owned(),
                        plugin_name: None,
                        input: StructuredObject::default(),
                    },
                    "Run batch_barrier.fast",
                    TimeRange::default(),
                )),
                PartContent::operation(OperationPart::pending(
                    82,
                    ToolInvocation {
                        tool_api_call: Some(agena_domain::ToolApiCall {
                            function: agena_domain::ToolApiFunction::Call,
                            arguments: StructuredObject::try_from(serde_json::json!({
                                "tool": "test.permission_batch.slow",
                                "input": {}
                            }))
                            .expect("valid slow provider tool envelope"),
                        }),
                        name: "test.permission_batch.slow".to_owned(),
                        plugin_name: None,
                        input: StructuredObject::default(),
                    },
                    "Run batch_barrier.slow",
                    TimeRange::default(),
                )),
            ],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: "reply-test-provider".to_owned(),
                model_id: "reply-test-model".to_owned(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some("permission-batch-fast".to_owned());
        message.parts[1].operation_id = Some("permission-batch-slow".to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist permission batch");
        let pending_tools = session.pending_tools();
        assert_eq!(pending_tools.len(), 2);

        let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
        request_override.set_parallel_tool_calls(Some(true));
        let options = SessionRunOptions {
            model: ModelRef::new("reply-test-provider", "reply-test-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        let mut session = manager
            .background_handle()
            .resolve_pending_tools(session, pending_tools, &options, manager.execution_state())
            .await
            .expect("resolve permission batch");
        session.refresh_derived();

        let pending_permissions = session.pending_interactive_requests();
        assert_eq!(
            pending_permissions.len(),
            2,
            "every Ask member of one concurrent provider batch must be visible before the session blocks"
        );
        assert!(
            session
                .find_pending_permission_by_request_id("permission-batch-fast")
                .is_some()
        );
        assert!(
            session
                .find_pending_permission_by_request_id("permission-batch-slow")
                .is_some()
        );
        assert!(
            session.pending_tools().is_empty(),
            "operations with unresolved authorization remain interaction-blocked, not re-dispatched as ordinary tools"
        );
        assert!(
            session
                .messages
                .iter()
                .flat_map(|message| message.parts.iter())
                .filter(|part| part.operation_id.as_deref().is_some_and(|id| {
                    id == "permission-batch-fast" || id == "permission-batch-slow"
                }))
                .all(|part| part.status == ExecutionStatus::Pending)
        );
        let events = manager
            .list_session_events(session.id)
            .await
            .expect("load permission batch events");
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::PermissionRequested(_)))
                .count(),
            2,
            "every Operation-owned authorization request must have its own durable audit event"
        );

        checkpoint_seeded_assistant_message(
            manager.as_ref(),
            session.id,
            seed_execution_id,
            seed_turn_id,
            seed_reply_id,
            session.messages.last().expect("permission batch message"),
        )
        .await;

        let first_reply = manager
            .reply_permission(SessionPermissionReplyRequest::new(
                session.id,
                options.clone(),
                PermissionReply {
                    request_id: "permission-batch-fast".to_owned(),
                    kind: PermissionReplyKind::AllowOnce,
                    reason: None,
                    scope: None,
                },
                None,
            ))
            .await
            .expect("persist first batch approval");
        assert_eq!(
            first_reply.pending_interactive_requests().len(),
            1,
            "one approval must leave its sibling request pending"
        );
        assert!(
            !manager.is_run_active(session.id).await,
            "an incomplete approval batch must not register a continuation execution"
        );
        assert_eq!(
            PERMISSION_BATCH_FAST_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an approved member must wait for the batch approval barrier"
        );
        assert_eq!(
            PERMISSION_BATCH_SLOW_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "an unresolved member must not execute"
        );
        let events_after_first = manager
            .list_session_events(session.id)
            .await
            .expect("load events after first batch approval");
        assert_eq!(
            events_after_first
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ExecutionStarted(_)))
                .count(),
            1,
            "the first approval must not create an isolated execution"
        );

        let second_manager = Arc::clone(&manager);
        let second_options = options.clone();
        let session_id = session.id;
        let mut second_reply = tokio::spawn(async move {
            second_manager
                .reply_permission(SessionPermissionReplyRequest::new(
                    session_id,
                    second_options,
                    PermissionReply {
                        request_id: "permission-batch-slow".to_owned(),
                        kind: PermissionReplyKind::AllowOnce,
                        reason: None,
                        scope: None,
                    },
                    None,
                ))
                .await
        });

        tokio::select! {
            _ = PERMISSION_BATCH_SLOW_STARTED.notified() => {}
            result = &mut second_reply => {
                panic!("approved batch finished before the slow tool reached its barrier: {result:?}")
            }
            _ = tokio::time::sleep(Duration::from_secs(5)) => {
                panic!("approved slow batch member did not start")
            }
        }
        tokio::time::timeout(Duration::from_secs(5), async {
            while PERMISSION_BATCH_FAST_CALLS.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("approved fast batch member did not run beside the slow member");
        assert_eq!(
            PERMISSION_BATCH_FAST_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert_eq!(
            PERMISSION_BATCH_SLOW_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1
        );
        assert!(
            !second_reply.is_finished(),
            "the canonical reply must wait for every concurrent tool result"
        );

        PERMISSION_BATCH_SLOW_RELEASE.notify_one();
        let completed = tokio::time::timeout(Duration::from_secs(5), second_reply)
            .await
            .expect("approved batch continuation timed out")
            .expect("join approved batch continuation")
            .expect("complete approved batch continuation");
        assert!(completed.pending_interactive_requests().is_empty());
        assert!(completed.next_pending_tool().is_none());
        assert!(completed.has_finished_operation("permission-batch-fast"));
        assert!(completed.has_finished_operation("permission-batch-slow"));
        assert_eq!(
            PERMISSION_BATCH_FAST_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an approval token must execute its immutable invocation exactly once"
        );
        assert_eq!(
            PERMISSION_BATCH_SLOW_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "an approval token must execute its immutable invocation exactly once"
        );

        let final_events = manager
            .list_session_events(session.id)
            .await
            .expect("load completed batch events");
        assert_eq!(
            final_events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ExecutionStarted(_)))
                .count(),
            2,
            "the batch needs only its seed execution and one shared continuation"
        );
        assert_eq!(
            final_events
                .iter()
                .filter(|event| matches!(event.kind, EventKind::PermissionRequested(_)))
                .count(),
            2,
            "durable approval Activities must suppress duplicate Ask evaluation"
        );

        let backend = manager.store.db.get_database_backend();
        let reply_row = manager
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                "SELECT status FROM agena_assistant_replies WHERE reply_id = ?",
                [seed_reply_id.to_string().into()],
            ))
            .await
            .expect("query completed canonical reply")
            .expect("canonical reply row");
        assert_eq!(
            reply_row
                .try_get::<String>("", "status")
                .expect("canonical reply status"),
            "completed"
        );
        let failed_activities = manager
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                backend,
                                "SELECT COUNT(*) AS count FROM agena_content_nodes WHERE owner_kind = 'assistant_reply' AND owner_id = ? AND state = 'failed'",
                [seed_reply_id.to_string().into()],
            ))
            .await
            .expect("count failed canonical activities")
            .expect("failed activity count row")
            .try_get::<i64>("", "count")
            .expect("failed activity count");
        assert_eq!(
            failed_activities, 0,
            "a successful approved batch must not manufacture failed Activities"
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn live_host_user_input_reply_does_not_require_canonical_activity_projection() {
        let manager = test_manager().await;
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
                title: "live host input projection lag".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create host input session");
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve host input message ids");
        let call_id = 93;
        let operation_id = "host-input-operation";
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(OperationPart::pending(
                call_id,
                ToolInvocation::new("test.reply_probe.run", StructuredObject::default()),
                "Run reply_probe.run",
                TimeRange::default(),
            ))],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: options.model.provider_id.to_string(),
                model_id: options.model.model_id.to_string(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some(operation_id.to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist host input operation");
        let pending = session
            .next_pending_tool()
            .expect("pending host input tool");
        let request_id = format!("host-input:{}:{call_id}:0", session.id);
        session = manager
            .apply_user_input_request_with_id(
                session,
                &pending,
                crate::message::AskUserToolInput {
                    title: "Continue?".to_owned(),
                    body_markdown: "Host tool is waiting.".to_owned(),
                    kind: "single".to_owned(),
                    submit_label: String::new(),
                    cancel_label: String::new(),
                    auto_resolution_ms: Some(60_000),
                    questions: vec![UserInputQuestion {
                        id: "continue".to_owned(),
                        header: String::new(),
                        question: "Continue?".to_owned(),
                        options: Vec::new(),
                        multiple: false,
                        allow_custom: true,
                    }],
                },
                request_id.clone(),
                manager.execution_state(),
            )
            .await
            .expect("persist host user-input activity");
        let pending_request = session
            .find_pending_user_input_by_request_id(request_id.as_str())
            .expect("pending host user-input request");
        assert!(
            manager
                .conversation_identity_for_message(session.id, pending_request.request.message_id,)
                .await
                .is_err(),
            "fixture must reproduce a live host request without a canonical continuation identity"
        );

        let response_rx = manager
            .install_host_user_input_waiter(session.id, request_id.clone())
            .await;
        let completed = manager
            .reply_user_input(SessionExecutionReplyRequest::new(
                session.id,
                options,
                agena_domain::UserInputReply {
                    request_id,
                    kind: UserInputReplyKind::Timeout,
                    answers: Default::default(),
                    reason: Some("automatic timeout".to_owned()),
                },
            ))
            .await
            .expect("live host reply must wake its waiter without canonical lookup");
        let response = response_rx.await.expect("host waiter received response");
        assert!(response.timed_out);
        assert!(!response.cancelled);
        assert!(completed.pending_interactive_requests().is_empty());
        assert!(
            completed.next_pending_tool().is_some(),
            "the already-running host tool, not a second session execution, owns continuation"
        );
        let events = manager
            .list_session_events(session.id)
            .await
            .expect("load host input events");
        assert!(
            events
                .iter()
                .all(|event| !matches!(event.kind, EventKind::ExecutionStarted(_)))
        );
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
        let agena_runtime::SessionToolExecutionOutcome::Completed(_) = tool_outcome else {
            panic!("unexpected tool outcome: {tool_outcome:?}");
        };
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn explicit_application_tool_does_not_consult_model_permission_policy() {
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
            .expect("application tool execution");

        let agena_runtime::SessionToolExecutionOutcome::Completed(_) = outcome else {
            panic!("unexpected tool outcome: {outcome:?}");
        };
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
    async fn sessionless_application_tool_does_not_consult_model_permission_policy() {
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
            .expect("sessionless application tool execution");
        let agena_runtime::SessionToolExecutionOutcome::Completed(_) = outcome else {
            panic!("unexpected sessionless outcome: {outcome:?}");
        };
    }

    async fn append_completed_text_message(
        manager: &SessionManager,
        mut session: Session,
        role: Role,
        text: &str,
        turn_id: Option<i64>,
        parent_message_id: Option<i64>,
    ) -> (Session, i64, agena_domain::TurnId) {
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
                model_turn_id: Some(turn_id.unwrap_or(message_id)),
                parent_message_id,
                ..Default::default()
            },
        );
        session.messages.push(message.clone());
        let session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist rewind regression message");
        let session_id = session.id;
        let execution_id = ExecutionId::new();
        let transcript_turn_id = agena_domain::TurnId::new();
        let response_id = agena_domain::AssistantReplyId::new();
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
                        reply_id: response_id,
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
                        reply_id: response_id,
                        outcome: agena_domain::ExecutionOutcome::Completed,
                        ts_ms: message.created_at.timestamp_millis(),
                    }),
                ],
                manager.execution_state().cache_policy(),
            )
            .await
            .expect("append current rewind regression history");
        (session, message_id, transcript_turn_id)
    }

    #[tokio::test]
    async fn process_restart_terminalizes_hanging_tool_batch_without_resuming_the_model() {
        let manager = test_manager().await;
        let mut session = manager
            .create_session(SessionCreateRequest {
                title: "interrupted tool batch".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create interrupted session");
        let session_id = session.id;
        let (execution_id, turn_id, reply_id) =
            seed_canonical_assistant_reply(&manager, session_id).await;
        let run_id = RunId::new();
        let ids = manager
            .store
            .reserve_message_ids(1)
            .await
            .expect("reserve interrupted tool ids");
        let mut message = build_message(
            ids,
            Role::Assistant,
            ExecutionStatus::InProgress,
            vec![PartContent::operation(OperationPart::pending(
                701,
                ToolInvocation::new("test.batch_barrier.slow", StructuredObject::default()),
                "Wait for batch_barrier.slow",
                TimeRange::default(),
            ))],
            MessageMetadata {
                model_turn_id: Some(1),
                model_provider_id: "reply-test-provider".to_owned(),
                model_id: "reply-test-model".to_owned(),
                ..MessageMetadata::default()
            },
        );
        message.parts[0].operation_id = Some("interrupted-tool-operation".to_owned());
        session.messages.push(message.clone());
        session = manager
            .persist_session_changes(
                session,
                vec![MessageCheckpoint::all(&message)],
                Vec::new(),
                None,
                manager.execution_state(),
            )
            .await
            .expect("persist pending tool");
        assert_eq!(
            session.workflow_state(),
            agena_domain::WorkflowState::ToolPending
        );

        manager
            .store
            .append_lifecycle_events(
                session_id,
                vec![
                    EventKind::RunStarted(RunStarted {
                        execution_id,
                        run_id,
                        source: ExecutionSource::User,
                        model_id: "reply-test-model".into(),
                        provider_id: "reply-test-provider".into(),
                        request_digest: None,
                    }),
                    EventKind::MessagePartCheckpointed(
                        crate::event::MessagePartCheckpointedEvent {
                            session_id,
                            execution_id: Some(execution_id),
                            run_id: Some(run_id),
                            turn_id: Some(turn_id),
                            reply_id: Some(reply_id),
                            message_id: message.id,
                            message_role: message.role,
                            message_state: message.state,
                            message_created_at: message.created_at,
                            message_metadata: message.metadata.clone(),
                            part: message.parts[0].clone(),
                            ts_ms: Utc::now().timestamp_millis(),
                        },
                    ),
                ],
            )
            .await
            .expect("persist hanging execution history");

        manager
            .reconcile_interrupted_executions()
            .await
            .expect("reconcile process restart");

        let recovered = manager
            .get_session(session_id)
            .await
            .expect("load recovered session");
        assert_eq!(
            recovered.workflow_state(),
            agena_domain::WorkflowState::Quiescent
        );
        assert!(recovered.pending_tools().is_empty());
        let recovered_part = recovered
            .messages
            .iter()
            .flat_map(|message| message.parts.iter())
            .find(|part| part.operation_id.as_deref() == Some("interrupted-tool-operation"))
            .expect("recovered tool operation");
        assert_eq!(recovered_part.status, ExecutionStatus::Failed);

        let snapshot = manager
            .transcript_snapshot(session_id)
            .await
            .expect("load recovered canonical transcript");
        assert_eq!(snapshot.turns.len(), 1);
        assert_eq!(
            snapshot.turns[0].reply.status,
            agena_domain::AssistantReplyStatus::Failed
        );
        assert!(
            snapshot.turns[0].reply.content.nodes().iter().any(|node| {
                matches!(
                    node,
                    agena_domain::ContentNode::Activity { activity }
                        if activity.state == agena_domain::ActivityState::Failed
                )
            }),
            "recovered reply content: {:#?}",
            snapshot.turns[0].reply.content.nodes(),
        );

        let events_after_first_recovery = manager
            .store
            .list_session_events(session_id)
            .await
            .expect("list recovered events");
        assert_eq!(
            events_after_first_recovery
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ExecutionStarted(_)))
                .count(),
            1
        );
        assert_eq!(
            events_after_first_recovery
                .iter()
                .filter(|event| matches!(event.kind, EventKind::ExecutionFinished(_)))
                .count(),
            1
        );
        assert_eq!(
            events_after_first_recovery
                .iter()
                .filter(|event| matches!(
                    event.kind,
                    EventKind::RunAborted(crate::session::history::RunAborted {
                        reason: agena_domain::RunAbortReason::ProcessRestart,
                        ..
                    })
                ))
                .count(),
            1
        );

        manager
            .reconcile_interrupted_executions()
            .await
            .expect("repeat recovery is idempotent");
        assert_eq!(
            manager
                .store
                .list_session_events(session_id)
                .await
                .expect("list events after repeated recovery")
                .len(),
            events_after_first_recovery.len(),
            "restart recovery must never enqueue a new execution or provider continuation",
        );
    }

    /// A crashed process leaves a stale execution lease behind. `reap_stale_leases`
    /// must reclaim it and terminalize the interrupted run so the session becomes
    /// usable again instead of permanently reporting "already running a response".
    #[tokio::test]
    async fn reap_stale_leases_reclaims_crashed_lease_and_recovers_session() {
        use agena_domain::AssistantReplyStatus;

        let manager = test_manager().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "crashed lease session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create crashed-lease session");
        let session_id = session.id;
        seed_canonical_assistant_reply(&manager, session_id).await;

        // A different, now-crashed process owns the session's lease with a stale
        // heartbeat (well past LEASE_STALENESS_MS).
        let now = agena_runtime_session_core::db::leases::lease_now_ms();
        manager
            .store
            .db
            .execute(sea_orm::Statement::from_sql_and_values(
                manager.store.db.get_database_backend(),
                "INSERT INTO agena_execution_leases \
                 (session_id, owner_id, run_id, lease_started_at_ms, heartbeat_at_ms) \
                 VALUES (?, ?, NULL, ?, ?)",
                [
                    session_id.into(),
                    "crashed-process".into(),
                    (now - 60_000).into(),
                    (now - 60_000).into(),
                ],
            ))
            .await
            .expect("insert stale crashed lease");

        // Startup-style reclamation: reclaim the stale lease and reconcile the
        // interrupted run.
        manager
            .reap_stale_leases()
            .await
            .expect("reap stale leases");

        // The stale lease row is gone.
        let lease_row =
            agena_runtime_session_core::db::leases::lease(&manager.store.db, session_id)
                .await
                .expect("read lease after reap");
        assert!(lease_row.is_none(), "stale lease must be reclaimed");

        // The interrupted execution was terminalized: the assistant reply is failed.
        let snapshot = manager
            .transcript_snapshot(session_id)
            .await
            .expect("load recovered canonical transcript");
        let terminal_reply = snapshot
            .turns
            .iter()
            .any(|turn| turn.reply.status == AssistantReplyStatus::Failed);
        assert!(
            terminal_reply,
            "interrupted reply must be terminalized as failed"
        );

        // The session is usable again: a fresh execution can register.
        let registry = Arc::clone(&manager.execution_registry);
        let registered = registry
            .register(
                session_id,
                agena_domain::TurnId::new(),
                agena_domain::AssistantReplyId::new(),
            )
            .await;
        assert!(
            registered.is_ok(),
            "session must be usable after lease reclamation"
        );
        // Clean up the lease this registration acquired.
        if let Ok((control, _rx)) = &registered {
            registry.unregister_if_matches(session_id, control).await;
        }
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
        let (session, first_user_id, _) = append_completed_text_message(
            &manager,
            session,
            Role::User,
            "first prompt",
            None,
            None,
        )
        .await;
        let (session, assistant_id, _) = append_completed_text_message(
            &manager,
            session,
            Role::Assistant,
            "first response",
            Some(first_user_id),
            Some(first_user_id),
        )
        .await;
        let (_session, rewind_target_id, rewind_target_turn_id) = append_completed_text_message(
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
                turn_id: rewind_target_turn_id,
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
            child.messages[1].metadata.model_turn_id,
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
    async fn permission_reply_releases_session_lock_before_tool_continuation() {
        let manager = Arc::new(test_manager().await);
        REPLY_PROBE_CALLS.store(0, std::sync::atomic::Ordering::SeqCst);
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
        let (seed_execution_id, canonical_turn_id, canonical_reply_id) =
            seed_canonical_assistant_reply(manager.as_ref(), session.id).await;
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
            model_turn_id: Some(1),
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
                vec![MessageCheckpoint::all(&message)],
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
                Vec::new(),
                manager.execution_state(),
            )
            .await
            .expect("persist reply probe permission request");
        let operation_activity_id = session
            .find_pending_permission_by_request_id(request_id.as_str())
            .and_then(|pending| session.part(&pending.tool.part))
            .and_then(|part| part.activity_id)
            .expect("operation activity identity");
        checkpoint_seeded_assistant_message(
            manager.as_ref(),
            session.id,
            seed_execution_id,
            canonical_turn_id,
            canonical_reply_id,
            session.messages.last().expect("seeded assistant message"),
        )
        .await;

        // A later canonical turn must not steal an older interactive reply.
        // The Operation owns the continuation identity explicitly, so
        // resolving it never depends on the session's newest turn.
        let (newer_execution_id, newer_turn_id, newer_reply_id) =
            seed_canonical_assistant_reply(manager.as_ref(), session.id).await;
        manager
            .store
            .append_lifecycle_events(
                session.id,
                vec![EventKind::ExecutionFinished(
                    agena_domain::ExecutionFinishedEvent {
                        session_id: session.id,
                        execution_id: newer_execution_id,
                        reply_id: newer_reply_id,
                        outcome: agena_domain::ExecutionOutcome::Completed,
                        ts_ms: Utc::now().timestamp_millis(),
                    },
                )],
            )
            .await
            .expect("finish newer canonical reply");
        assert_ne!(newer_turn_id, canonical_turn_id);
        assert_ne!(newer_reply_id, canonical_reply_id);

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
        let receipt = outcome
            .receipt
            .expect("permission continuation must return an execution receipt");
        assert_eq!(receipt.turn_id, canonical_turn_id);
        assert_eq!(receipt.reply_id, canonical_reply_id);
        let canonical_operation = manager
            .store
            .db
            .query_one(Statement::from_sql_and_values(
                manager.store.db.get_database_backend(),
                "SELECT payload_json FROM agena_content_nodes WHERE node_id = ?",
                [operation_activity_id.to_string().into()],
            ))
            .await
            .expect("query canonical operation activity")
            .expect("canonical operation activity");
        let payload = canonical_operation
            .try_get::<serde_json::Value>("", "payload_json")
            .expect("activity payload");
        assert!(
            payload
                .pointer("/authorization/permissions/0/reply")
                .is_some_and(|reply| !reply.is_null()),
            "canonical Operation must contain the durable permission reply: {payload}"
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
        assert_eq!(
            REPLY_PROBE_CALLS.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one approval must execute the original target exactly once"
        );
        let completed = manager
            .get_session(session_id)
            .await
            .expect("load completed approved operation");
        assert!(completed.pending_interactive_requests().is_empty());
        assert!(completed.next_pending_tool().is_none());
        assert!(completed.has_finished_operation("reply-lock-operation"));
    }

    // A provider that returns exactly one tool-call turn followed by a plain
    // text turn. Lets an end-to-end test observe whether the session loop
    // keeps requesting the model after a tool call finishes.
    #[derive(Clone)]
    struct StreamingNoIdToolProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for StreamingNoIdToolProvider {
        fn id(&self) -> &str {
            "streaming-no-id-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        fn agena_tool_mode_for_adapter(
            &self,
            _adapter_id: Option<&agena_domain::AdapterId>,
            _model: &ModelId,
        ) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "let me run the tool".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::ToolCalls),
                    tool_calls: vec![agena_provider::CompletionToolCall::Function {
                        id: "call_1".to_owned(),
                        name: "test.approved_success.run".to_owned(),
                        arguments_json: "{}".to_owned(),
                    }],
                    usage: None,
                    provider_metadata: None,
                })
            } else {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "done".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    tool_calls: Vec::new(),
                    usage: None,
                    provider_metadata: None,
                })
            }
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<
                            Item = Result<
                                agena_provider::CompletionStreamEvent,
                                agena_runtime_provider::ProviderError,
                            >,
                        > + Send,
                >,
            >,
            agena_runtime_provider::ProviderError,
        > {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(agena_provider::CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        stream_key: "idx:0".to_owned(),
                        id: None,
                        name: Some("test.approved_success.run".to_owned()),
                        arguments_delta: "{}".to_owned(),
                    }),
                    Ok(agena_provider::CompletionStreamEvent::Completed {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        finish_reason: Some(agena_provider::CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                        end_turn: None,
                    }),
                ])))
            } else {
                Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(agena_provider::CompletionStreamEvent::TextDelta {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        delta: "done".to_owned(),
                    }),
                    Ok(agena_provider::CompletionStreamEvent::Completed {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                        end_turn: None,
                    }),
                ])))
            }
        }
    }

    struct TwoTurnToolProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for TwoTurnToolProvider {
        fn id(&self) -> &str {
            "two-turn-provider"
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
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "let me run the tool".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::ToolCalls),
                    tool_calls: vec![agena_provider::CompletionToolCall::Function {
                        id: "call_1".to_owned(),
                        name: "test.approved_success.run".to_owned(),
                        arguments_json: "{}".to_owned(),
                    }],
                    usage: None,
                    provider_metadata: None,
                })
            } else {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "done".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    tool_calls: Vec::new(),
                    usage: None,
                    provider_metadata: None,
                })
            }
        }
    }

    async fn test_manager_with_two_turn_provider()
    -> (SessionManager, Arc<std::sync::atomic::AtomicUsize>) {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.approved_success".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "test.approved_success"
                    .parse()
                    .expect("valid test plugin key"),
                ApprovedSuccessTool,
            )],
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(TwoTurnToolProvider {
            default_model: ModelId::new("two-turn-model"),
            calls: Arc::clone(&calls),
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        (manager, calls)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn model_tool_call_turn_continues_until_final_text() {
        let (manager, calls) = test_manager_with_two_turn_provider().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "two turn provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create two-turn session");
        manager
            .update_session_selection(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("two-turn-provider", "two-turn-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
            )
            .await
            .expect("select two-turn model");
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "please work".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("two-turn-provider", "two-turn-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");

        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "the loop must request the model again after the tool call finishes"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    async fn test_manager_with_streaming_no_id_provider()
    -> (SessionManager, Arc<std::sync::atomic::AtomicUsize>) {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.approved_success".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "test.approved_success"
                    .parse()
                    .expect("valid test plugin key"),
                ApprovedSuccessTool,
            )],
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(StreamingNoIdToolProvider {
            default_model: ModelId::new("streaming-no-id-model"),
            calls: Arc::clone(&calls),
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        (manager, calls)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn streaming_tool_call_without_provider_id_still_continues() {
        let (manager, calls) = test_manager_with_streaming_no_id_provider().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "streaming no id provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create streaming session");
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "please work".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("streaming-no-id-provider", "streaming-no-id-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");

        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "streaming tool call without provider id must still continue the loop"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    // A provider that streams plain text with an explicit `end_turn=false`
    // signal on the first Completed event, then finishes with a normal
    // `end_turn` absent on the second. Lets an end-to-end test verify that
    // the session loop honors the protocol-level follow-up signal even when
    // the model produces no tool call.
    #[derive(Clone)]
    struct EndTurnFalseProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for EndTurnFalseProvider {
        fn id(&self) -> &str {
            "end-turn-false-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        fn agena_tool_mode_for_adapter(
            &self,
            _adapter_id: Option<&agena_domain::AdapterId>,
            _model: &ModelId,
        ) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (text, end_turn) = if call == 0 {
                ("first turn, not done".to_owned(), Some(false))
            } else {
                ("done".to_owned(), None)
            };
            Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.default_model.clone(),
                text,
                reasoning_text: None,
                finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
            .map(|response| {
                // The non-streaming port has no end_turn field; this provider
                // is exercised through complete_stream below.
                let _ = end_turn;
                response
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<
                            Item = Result<
                                agena_provider::CompletionStreamEvent,
                                agena_runtime_provider::ProviderError,
                            >,
                        > + Send,
                >,
            >,
            agena_runtime_provider::ProviderError,
        > {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (text, end_turn) = if call == 0 {
                ("first turn, not done".to_owned(), Some(false))
            } else {
                ("done".to_owned(), None)
            };
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(agena_provider::CompletionStreamEvent::TextDelta {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    delta: text,
                }),
                Ok(agena_provider::CompletionStreamEvent::Completed {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                    end_turn,
                }),
            ])))
        }
    }

    async fn test_manager_with_end_turn_false_provider()
    -> (SessionManager, Arc<std::sync::atomic::AtomicUsize>) {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![],
            config: PluginsConfig::default(),
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(EndTurnFalseProvider {
            default_model: ModelId::new("end-turn-false-model"),
            calls: Arc::clone(&calls),
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        (manager, calls)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn end_turn_false_keeps_loop_alive_without_tool_call() {
        let (manager, calls) = test_manager_with_end_turn_false_provider().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "end turn false provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create end-turn session");
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "please work".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("end-turn-false-provider", "end-turn-false-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");

        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "an explicit end_turn=false must request another model turn even without a tool call"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    // A provider that always streams plain text with no tool call and no
    // explicit end_turn. A normal user submission stops after one turn; a
    // continue-driven run exercises the bounded goal-continuation driver.
    #[derive(Clone)]
    struct PlainTextProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for PlainTextProvider {
        fn id(&self) -> &str {
            "plain-text-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        fn agena_tool_mode_for_adapter(
            &self,
            _adapter_id: Option<&agena_domain::AdapterId>,
            _model: &ModelId,
        ) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            let _ = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.default_model.clone(),
                text: "plain".to_owned(),
                reasoning_text: None,
                finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<
                            Item = Result<
                                agena_provider::CompletionStreamEvent,
                                agena_runtime_provider::ProviderError,
                            >,
                        > + Send,
                >,
            >,
            agena_runtime_provider::ProviderError,
        > {
            let _ = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(agena_provider::CompletionStreamEvent::TextDelta {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    delta: "plain".to_owned(),
                }),
                Ok(agena_provider::CompletionStreamEvent::Completed {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                    end_turn: None,
                }),
            ])))
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn continue_session_goal_continuation_drives_plain_text_turns() {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![],
            config: PluginsConfig::default(),
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(PlainTextProvider {
            default_model: ModelId::new("plain-text-model"),
            calls: Arc::clone(&calls),
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        let session = manager
            .create_session(SessionCreateRequest {
                title: "plain text provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create plain session");
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "please work".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("plain-text-provider", "plain-text-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "a plain-text user submission without end_turn must stop after one turn"
        );

        manager
            .continue_session(SessionExecutionRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("plain-text-provider", "plain-text-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
            ))
            .await
            .expect("continue session");
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("continue run did not finish");

        let total = calls.load(std::sync::atomic::Ordering::SeqCst);
        // One user submission, then the continue run: one main model turn
        // plus GOAL_CONTINUATION_LIMIT extra continuation turns.
        assert_eq!(
            total,
            1 + 1 + 8,
            "continue must drive one main turn plus GOAL_CONTINUATION_LIMIT extra plain-text turns (got {total})"
        );
    }

    // A provider that emits a scripted sequence of plain-text turns ending
    // with dangling connectors (or an output-limit truncation) before a final
    // complete turn. Lets end-to-end tests observe whether the stable-run
    // loop keeps requesting the model for unfinished turns.
    #[derive(Clone)]
    struct UnfinishedPlainTextProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
        /// Number of turns that end with a dangling connector (colon).
        dangling_turns: usize,
        /// When true, the first turn reports `Length` (output limit) instead
        /// of `Stop`.
        truncate_first: bool,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for UnfinishedPlainTextProvider {
        fn id(&self) -> &str {
            "unfinished-plain-text-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        fn agena_tool_mode_for_adapter(
            &self,
            _adapter_id: Option<&agena_domain::AdapterId>,
            _model: &ModelId,
        ) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (text, finish_reason) = if self.truncate_first && call == 0 {
                (
                    "partial output".to_owned(),
                    agena_provider::CompletionFinishReason::Length,
                )
            } else if call < self.dangling_turns {
                (
                    "先查看 provider_for_adapter_with_mode 和 1390-1420 区域：".to_owned(),
                    agena_provider::CompletionFinishReason::Stop,
                )
            } else {
                (
                    "完成。".to_owned(),
                    agena_provider::CompletionFinishReason::Stop,
                )
            };
            Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.default_model.clone(),
                text,
                reasoning_text: None,
                finish_reason: Some(finish_reason),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<
                            Item = Result<
                                agena_provider::CompletionStreamEvent,
                                agena_runtime_provider::ProviderError,
                            >,
                        > + Send,
                >,
            >,
            agena_runtime_provider::ProviderError,
        > {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (text, finish_reason) = if self.truncate_first && call == 0 {
                (
                    "partial output".to_owned(),
                    agena_provider::CompletionFinishReason::Length,
                )
            } else if call < self.dangling_turns {
                (
                    "先查看 provider_for_adapter_with_mode 和 1390-1420 区域：".to_owned(),
                    agena_provider::CompletionFinishReason::Stop,
                )
            } else {
                (
                    "完成。".to_owned(),
                    agena_provider::CompletionFinishReason::Stop,
                )
            };
            Ok(Box::pin(futures_util::stream::iter(vec![
                Ok(agena_provider::CompletionStreamEvent::TextDelta {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    delta: text,
                }),
                Ok(agena_provider::CompletionStreamEvent::Completed {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    finish_reason: Some(finish_reason),
                    usage: None,
                    provider_metadata: None,
                    end_turn: None,
                }),
            ])))
        }
    }

    async fn test_manager_with_unfinished_plain_text_provider(
        dangling_turns: usize,
        truncate_first: bool,
    ) -> (SessionManager, Arc<std::sync::atomic::AtomicUsize>) {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![],
            config: PluginsConfig::default(),
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(UnfinishedPlainTextProvider {
            default_model: ModelId::new("unfinished-plain-text-model"),
            calls: Arc::clone(&calls),
            dangling_turns,
            truncate_first,
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        (manager, calls)
    }

    async fn submit_plain_text_user_message(manager: &SessionManager, session_id: i64) {
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "please work".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session_id,
                SessionRunOptions {
                    model: ModelRef::new(
                        "unfinished-plain-text-provider",
                        "unfinished-plain-text-model",
                    ),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn plain_text_trailing_colon_keeps_loop_alive_without_tool_call() {
        let (manager, calls) = test_manager_with_unfinished_plain_text_provider(2, false).await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "trailing colon provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create trailing-colon session");
        submit_plain_text_user_message(&manager, session.id).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "plain-text turns ending on a colon must keep the loop alive until a complete turn"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn truncated_plain_text_forces_continuation() {
        let (manager, calls) = test_manager_with_unfinished_plain_text_provider(0, true).await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "truncated provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create truncated session");
        submit_plain_text_user_message(&manager, session.id).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "an output-limit truncated turn must request another model turn"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn unfinished_continuation_budget_is_bounded() {
        // A degenerate model that never completes must not loop forever: one
        // initial turn plus the bounded unfinished-continuation budget.
        let (manager, calls) =
            test_manager_with_unfinished_plain_text_provider(usize::MAX, false).await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "unfinished budget provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create budget session");
        submit_plain_text_user_message(&manager, session.id).await;
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1 + 4,
            "unfinished continuation must be bounded by its per-run budget (1 main + 4 continuations)"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }

    // A provider that reproduces the real-world failure mode: a tool-calling
    // turn, then a plain-text turn that ends on a dangling colon while the
    // task is still mid-work, then a final complete turn. The stable-run
    // loop must keep requesting the model through the dangling turn instead
    // of stopping mid-task.
    #[derive(Clone)]
    struct AgenticDanglingProvider {
        default_model: ModelId,
        calls: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl ModelRuntime for AgenticDanglingProvider {
        fn id(&self) -> &str {
            "agentic-dangling-provider"
        }

        fn default_model(&self) -> &ModelId {
            &self.default_model
        }

        async fn list_models(&self) -> Result<Vec<Model>, agena_runtime_provider::ProviderError> {
            Ok(Vec::new())
        }

        fn agena_tool_mode_for_adapter(
            &self,
            _adapter_id: Option<&agena_domain::AdapterId>,
            _model: &ModelId,
        ) -> agena_provider::AgenaToolMode {
            agena_provider::AgenaToolMode::ProviderProtocol
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, agena_runtime_provider::ProviderError> {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if call == 0 {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "let me run the tool".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::ToolCalls),
                    tool_calls: vec![agena_provider::CompletionToolCall::Function {
                        id: "call_1".to_owned(),
                        name: "test.approved_success.run".to_owned(),
                        arguments_json: "{}".to_owned(),
                    }],
                    usage: None,
                    provider_metadata: None,
                })
            } else if call == 1 {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "现在看 provider_for_adapter_with_mode 和 1390-1420 区域：".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    tool_calls: Vec::new(),
                    usage: None,
                    provider_metadata: None,
                })
            } else {
                Ok(CompletionResponse {
                    provider_id: agena_domain::ProviderId::new(self.id()),
                    model: self.default_model.clone(),
                    text: "完成。".to_owned(),
                    reasoning_text: None,
                    finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                    tool_calls: Vec::new(),
                    usage: None,
                    provider_metadata: None,
                })
            }
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            std::pin::Pin<
                Box<
                    dyn futures_util::Stream<
                            Item = Result<
                                agena_provider::CompletionStreamEvent,
                                agena_runtime_provider::ProviderError,
                            >,
                        > + Send,
                >,
            >,
            agena_runtime_provider::ProviderError,
        > {
            let call = self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            match call {
                0 => Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(agena_provider::CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        stream_key: "idx:0".to_owned(),
                        id: None,
                        name: Some("test.approved_success.run".to_owned()),
                        arguments_delta: "{}".to_owned(),
                    }),
                    Ok(agena_provider::CompletionStreamEvent::Completed {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        finish_reason: Some(agena_provider::CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                        end_turn: None,
                    }),
                ]))),
                1 => Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(agena_provider::CompletionStreamEvent::TextDelta {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        delta: "现在看 provider_for_adapter_with_mode 和 1390-1420 区域："
                            .to_owned(),
                    }),
                    Ok(agena_provider::CompletionStreamEvent::Completed {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                        end_turn: None,
                    }),
                ]))),
                _ => Ok(Box::pin(futures_util::stream::iter(vec![
                    Ok(agena_provider::CompletionStreamEvent::TextDelta {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        delta: "完成。".to_owned(),
                    }),
                    Ok(agena_provider::CompletionStreamEvent::Completed {
                        provider_id: agena_domain::ProviderId::new(self.id()),
                        model: self.default_model.clone(),
                        finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                        end_turn: None,
                    }),
                ]))),
            }
        }
    }

    async fn test_manager_with_agentic_dangling_provider()
    -> (SessionManager, Arc<std::sync::atomic::AtomicUsize>) {
        let workspace_root = std::env::current_dir().expect("resolve test workspace");
        let mut plugins_config = PluginsConfig::default();
        plugins_config.list.insert(
            "test.approved_success".to_string(),
            ConfiguredPlugin::static_default(),
        );
        let plugins = PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "test.approved_success"
                    .parse()
                    .expect("valid test plugin key"),
                ApprovedSuccessTool,
            )],
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
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            Arc::clone(&plugins),
            None,
            None,
            None,
            ToolPresentationConfig::default(),
        );
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut providers = ProviderRegistry::new();
        providers.register(AgenticDanglingProvider {
            default_model: ModelId::new("agentic-dangling-model"),
            calls: Arc::clone(&calls),
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
        let manager = SessionManager::new(
            database,
            processor,
            executor,
            RuntimeSessionManagerConfig::default(),
        );
        (manager, calls)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn agentic_dangling_text_continues_after_tool_work() {
        let (manager, calls) = test_manager_with_agentic_dangling_provider().await;
        let session = manager
            .create_session(SessionCreateRequest {
                title: "agentic dangling provider".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("create agentic dangling session");
        let document = agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
            text: "remove the prompt envelope".to_owned(),
        }]);
        manager
            .submit_user_message(agena_runtime::SessionUserMessageRequest::new(
                session.id,
                SessionRunOptions {
                    model: ModelRef::new("agentic-dangling-provider", "agentic-dangling-model"),
                    thinking_mode: None,
                    speed_mode: None,
                    verbosity: None,
                    thinking: None,
                    request_override: Default::default(),
                    system: None,
                    temperature: None,
                    max_output_tokens: None,
                },
                document,
            ))
            .await
            .expect("submit user message");
        tokio::time::timeout(Duration::from_secs(10), async {
            while manager.is_run_active(session.id).await {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("session run did not finish");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            3,
            "a tool-calling run whose next turn dangles on a colon must not stop mid-task"
        );
        let finished = manager
            .get_session(session.id)
            .await
            .expect("load finished session");
        assert!(finished.next_pending_tool().is_none());
        assert!(finished.pending_interactive_requests().is_empty());
    }
}
