use super::*;
use agena_domain::StructuredObject;
use agena_plugin_host::sdk::{self, host_api, host_api::HostClient};
use serde_json::{Value, json};
use std::sync::{OnceLock, Weak};

#[derive(Default)]
struct PermissionHost {
    manager: OnceLock<Weak<SessionManager>>,
}

#[sdk::async_trait]
impl HostClient for PermissionHost {
    async fn log(&self, _: host_api::LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: sdk::EventEnvelope) -> sdk::Result<()> {
        Ok(())
    }
    async fn subscribe_events(
        &self,
        _: sdk::EventFilter,
    ) -> sdk::Result<host_api::EventSubscription> {
        Ok(host_api::EventSubscription {
            id: "operator-test".into(),
        })
    }
    async fn read_config(&self, _: Option<String>) -> sdk::Result<Value> {
        Ok(json!({}))
    }
    async fn invoke_tool(&self, _: String, _: Value) -> sdk::Result<sdk::ToolInvokeOutput> {
        Err(sdk::PluginError::not_implemented("unused test callback"))
    }
    async fn require_path_permission(
        &self,
        request: host_api::PathPermissionQuery,
    ) -> sdk::Result<()> {
        let context = host_api::current_host_callback_context().unwrap();
        let manager = self.manager.get().unwrap().upgrade().unwrap();
        manager
            .require_host_path_permission(
                context.session_id.unwrap(),
                context.call_id.unwrap(),
                request.kind,
                request.path,
            )
            .await
            .map_err(|error| sdk::PluginError::internal(error.to_string()))
    }
}

#[derive(Default)]
struct FilePluginState {
    host: OnceLock<Arc<dyn HostClient>>,
    contexts: StdMutex<Vec<host_api::HostCallbackContext>>,
    waiting: Notify,
}

/// Exercise the same plugin-owned permission callbacks as fs.read_many and
/// fs.write, through a real PluginHost and its callback-token validation.
struct FilePlugin(Arc<FilePluginState>);

#[sdk::async_trait]
impl sdk::Plugin for FilePlugin {
    fn manifest(&self) -> sdk::PluginManifest {
        let mut manifest = sdk::PluginManifest::new("test", "operator", "1.0.0");
        manifest.tools = ["read_many", "write"]
            .into_iter()
            .map(|name| sdk::ToolDefinition {
                name: name.into(),
                contract: sdk::ToolContract {
                    input_schema: json!({"type": "object"}),
                    output_schema: json!({"type": "null"}),
                },
                docs: Default::default(),
                runtime: Default::default(),
                tags: vec![if name == "read_many" {
                    sdk::ToolTag::ReadOnly
                } else {
                    sdk::ToolTag::Mutate
                }],
            })
            .collect();
        manifest
    }

    async fn init(
        &self,
        _: sdk::InitContext,
        host: Arc<dyn HostClient>,
    ) -> sdk::Result<sdk::InitOutcome> {
        assert!(self.0.host.set(host).is_ok());
        Ok(sdk::InitOutcome::ack(self.manifest()))
    }

    async fn tool_invoke(&self, input: sdk::ToolInvokeInput) -> sdk::Result<sdk::ToolInvokeOutput> {
        let context = host_api::current_host_callback_context().unwrap();
        assert_eq!(context.session_id, Some(OPERATOR_SESSION));
        assert!(context.authority_token.is_some());
        self.0.contexts.lock().unwrap().push(context);
        let host = self.0.host.get().unwrap();
        if input.input["wait"].as_bool() == Some(true) {
            self.0.waiting.notify_one();
            std::future::pending::<()>().await;
        }
        match input.tool_name.as_str() {
            "read_many" => {
                let paths = input.input["paths"].as_array().unwrap();
                for path in paths {
                    host.require_path_permission(host_api::PathPermissionQuery::read(
                        path.as_str().unwrap(),
                    ))
                    .await?;
                }
                let content = paths
                    .iter()
                    .map(|path| {
                        std::fs::read_to_string(
                            std::path::Path::new(&input.workspace_root)
                                .join(path.as_str().unwrap()),
                        )
                        .map_err(|error| sdk::PluginError::internal(error.to_string()))
                    })
                    .collect::<sdk::Result<Vec<_>>>()?
                    .join("\n");
                Ok(sdk::ToolInvokeOutput::text(content))
            }
            "write" => {
                let path = input.input["path"].as_str().unwrap();
                host.require_path_permission(host_api::PathPermissionQuery::read(path))
                    .await?;
                host.require_path_permission(host_api::PathPermissionQuery {
                    kind: agena_domain::PathKind::Write,
                    path: path.into(),
                })
                .await?;
                std::fs::write(
                    std::path::Path::new(&input.workspace_root).join(path),
                    "written",
                )
                .map_err(|error| sdk::PluginError::internal(error.to_string()))?;
                Ok(sdk::ToolInvokeOutput::text("written"))
            }
            _ => unreachable!(),
        }
    }
}

struct Fixture {
    manager: Arc<SessionManager>,
    plugin: Arc<FilePluginState>,
    root: std::path::PathBuf,
    workspace: std::path::PathBuf,
}

impl Fixture {
    async fn new(write_mode: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("agena-operator-scope-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let workspace = tokio::fs::canonicalize(workspace).await.unwrap();
        tokio::fs::write(workspace.join("one.txt"), "one")
            .await
            .unwrap();
        tokio::fs::write(workspace.join("two.txt"), "two")
            .await
            .unwrap();
        tokio::fs::write(root.join("outside.txt"), "unchanged")
            .await
            .unwrap();
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let host = Arc::new(PermissionHost::default());
        let plugin = Arc::new(FilePluginState::default());
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                static_plugins: vec![agena_plugin_host::StaticPluginRegistration::new(
                    "test.operator".parse().unwrap(),
                    FilePlugin(plugin.clone()),
                )],
                config: agena_plugin_host::PluginsConfig {
                    list: [("test.operator".into(), Default::default())].into(),
                    ..Default::default()
                },
                workspace_root: workspace.clone(),
                agena_version: "test".into(),
                callback_base_url: None,
                host_client: Some(host.clone()),
                previous: None,
                previous_plugins: Default::default(),
            })
            .await
            .unwrap();
        let permissions: agena_domain::PermissionConfig = serde_json::from_value(json!({
            "path": {
                "workspace": {"read": "allow", "write": write_mode},
                "external": {"read": "deny", "write": "deny"}
            },
            "tools": {"default": "allow"}
        }))
        .unwrap();
        let executor = ToolExecutor::new(
            workspace.clone(),
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            plugins.clone(),
            None,
            None,
        )
        .with_permission_config(&permissions);
        let manager = Arc::new(SessionManager::new(
            kernel,
            Arc::new(ProviderRegistry::default()),
            ContextGovernor::new(Default::default()),
            SessionProcessor::new(plugins),
            executor,
            RuntimeSessionManagerConfig::default(),
        ));
        assert!(host.manager.set(Arc::downgrade(&manager)).is_ok());
        Self {
            manager,
            plugin,
            root,
            workspace,
        }
    }

    async fn call(
        &self,
        tool: &str,
        input: Value,
        id: i64,
    ) -> Result<crate::SessionToolExecutionOutcome, AppError> {
        self.manager
            .execute_unscoped_tool(invocation(tool, input), id)
            .await
    }

    async fn assert_revoked(&self, context: host_api::HostCallbackContext) {
        assert!(self.manager.operator_calls.lock().unwrap().is_empty());
        assert!(self.manager.tools.lock().unwrap().is_empty());
        let result = host_api::run_in_isolated_host_callback_context(
            context,
            self.plugin
                .host
                .get()
                .unwrap()
                .require_path_permission(host_api::PathPermissionQuery::read("one.txt")),
        )
        .await;
        assert_eq!(result.unwrap_err().kind, sdk::PluginErrorKind::PolicyDenied);
    }

    async fn shutdown(self) {
        self.manager
            .kernel_runtime
            .ports
            .clone()
            .shutdown()
            .await
            .unwrap();
        tokio::fs::remove_dir_all(self.root).await.unwrap();
    }
}

fn invocation(tool: &str, input: Value) -> ToolInvocation {
    ToolInvocation::new(
        format!("test.operator.{tool}"),
        StructuredObject::try_from(input).unwrap(),
    )
}

#[tokio::test]
async fn operator_plugin_read_many_and_write_use_live_policy_and_revoke_on_success() {
    let fixture = Fixture::new("allow").await;
    let read = fixture
        .call("read_many", json!({"paths": ["one.txt", "two.txt"]}), 1)
        .await
        .unwrap();
    assert!(read.into_summary().output_text.contains("one\ntwo"));
    fixture
        .call("write", json!({"path": "created.txt"}), 2)
        .await
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.join("created.txt")).unwrap(),
        "written"
    );
    let context = fixture
        .plugin
        .contexts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    fixture.assert_revoked(context).await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_plugin_io_denies_paths_outside_workspace_and_revokes_on_error() {
    let fixture = Fixture::new("allow").await;
    assert!(
        fixture
            .call(
                "read_many",
                json!({"paths": ["one.txt", "../outside.txt"]}),
                3
            )
            .await
            .is_err()
    );
    assert!(
        fixture
            .call("write", json!({"path": "../outside.txt"}), 4)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("outside.txt")).unwrap(),
        "unchanged"
    );
    let context = fixture
        .plugin
        .contexts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    fixture.assert_revoked(context).await;
    fixture.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn operator_plugin_io_denies_symlink_escape() {
    let fixture = Fixture::new("allow").await;
    std::os::unix::fs::symlink(
        fixture.root.join("outside.txt"),
        fixture.workspace.join("link.txt"),
    )
    .unwrap();
    assert!(
        fixture
            .call("read_many", json!({"paths": ["link.txt"]}), 5)
            .await
            .is_err()
    );
    assert!(
        fixture
            .call("write", json!({"path": "link.txt"}), 6)
            .await
            .is_err()
    );
    assert_eq!(
        std::fs::read_to_string(fixture.root.join("outside.txt")).unwrap(),
        "unchanged"
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_scope_rejects_duplicate_ids_and_is_revoked_when_request_is_dropped() {
    let fixture = Fixture::new("allow").await;
    let manager = fixture.manager.clone();
    let task = tokio::spawn(async move {
        manager
            .execute_unscoped_tool(invocation("read_many", json!({"wait": true})), 7)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), fixture.plugin.waiting.notified())
        .await
        .unwrap();
    let authority = fixture.manager.operator_invocation(7).unwrap();
    assert!(
        fixture
            .call("write", json!({"path": "unexpected.txt"}), 7)
            .await
            .is_err()
    );
    assert!(!authority.cancel.is_cancelled());
    assert!(fixture.manager.operator_invocation(7).is_ok());
    assert!(!fixture.workspace.join("unexpected.txt").exists());
    let context = fixture
        .plugin
        .contexts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    assert!(authority.cancel.is_cancelled());
    assert!(fixture.manager.operator_invocation(7).is_err());
    fixture.assert_revoked(context).await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_scope_never_promotes_ask_or_auto_to_allow() {
    for mode in ["ask", "auto"] {
        let fixture = Fixture::new(mode).await;
        let error = fixture
            .call("write", json!({"path": "blocked.txt"}), 8)
            .await
            .unwrap_err();
        assert!(
            error.to_string().contains("no session approval channel"),
            "{error}"
        );
        assert!(!fixture.workspace.join("blocked.txt").exists());
        assert!(fixture.manager.operator_calls.lock().unwrap().is_empty());
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn operator_scope_shutdown_cancels_pending_plugin_and_drains_authority() {
    let fixture = Fixture::new("allow").await;
    let manager = fixture.manager.clone();
    let task = tokio::spawn(async move {
        manager
            .execute_unscoped_tool(invocation("read_many", json!({"wait": true})), 9)
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), fixture.plugin.waiting.notified())
        .await
        .unwrap();
    let context = fixture
        .plugin
        .contexts
        .lock()
        .unwrap()
        .last()
        .unwrap()
        .clone();
    let authority = fixture.manager.operator_invocation(9).unwrap();
    fixture.manager.shutdown().await.unwrap();
    assert!(task.await.unwrap().is_err());
    assert!(authority.cancel.is_cancelled());
    fixture.assert_revoked(context).await;
    assert!(
        fixture
            .call("read_many", json!({"paths": ["one.txt"]}), 10)
            .await
            .is_err()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_scope_preserves_its_policy_when_runtime_is_reconfigured() {
    let fixture = Fixture::new("allow").await;
    let state = fixture.manager.state.load_full();
    let workspace = fixture.manager.current_workspace_id().await.unwrap();
    let first = fixture
        .manager
        .begin_operator_invocation(11, state.executor.clone(), workspace)
        .unwrap();

    let permissions: agena_domain::PermissionConfig = serde_json::from_value(json!({
        "path": {
            "workspace": {"read": "allow", "write": "deny"},
            "external": {"read": "deny", "write": "deny"}
        },
        "tools": {"default": "allow"}
    }))
    .unwrap();
    fixture.manager.reconfigure(
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        state.executor.clone().with_permission_config(&permissions),
        state.config.clone(),
    );
    let second = fixture
        .manager
        .begin_operator_invocation(12, fixture.manager.tool_executor(), workspace)
        .unwrap();
    // Each already admitted invocation keeps its captured static policy. The
    // workspace's persisted permission rules are still resolved on every check.
    fixture
        .manager
        .require_host_path_permission(
            OPERATOR_SESSION,
            11,
            agena_domain::PathKind::Write,
            "permitted.txt".into(),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .manager
            .require_host_path_permission(
                OPERATOR_SESSION,
                12,
                agena_domain::PathKind::Write,
                "denied.txt".into(),
            )
            .await
            .is_err()
    );
    drop(first);
    drop(second);
    assert!(fixture.manager.operator_calls.lock().unwrap().is_empty());
    fixture.shutdown().await;
}

#[tokio::test]
async fn operator_scope_reload_does_not_inherit_another_workspaces_permission_rule() {
    let fixture = Fixture::new("allow").await;
    // Populate the manager's original-workspace cache before reconfiguration.
    // Operator calls must derive their identity from their captured executor,
    // even while that unrelated cache still refers to the original workspace.
    let original_workspace = fixture.manager.current_workspace_id().await.unwrap();
    let state = fixture.manager.state.load_full();
    let permissions: agena_domain::PermissionConfig = serde_json::from_value(json!({
        "path": {
            "workspace": {"read": "allow", "write": "deny"},
            "external": {"read": "deny", "write": "deny"}
        },
        // ReadOnly tools have a built-in Allow class above tools.default.
        // An explicit tool name takes precedence and makes this test Ask.
        "tools": {"default": "allow", "names": {"test.operator.read_many": "ask"}}
    }))
    .unwrap();
    let original_executor = state.executor.clone().with_permission_config(&permissions);
    let request = invocation("read_many", json!({"paths": ["one.txt"]}));
    let checks = original_executor
        .collect_permission_checks_for_invocation_in_session(&request, None)
        .await
        .unwrap();
    assert!(matches!(
        checks[0].decision,
        agena_domain::PermissionDecision::Ask { .. }
    ));
    let action_key = super::super::super::execution::permission_key(&checks[0].action).unwrap();
    let mut rule = agena_storage::PersistedPermissionRule {
        id: None,
        created_at_ms: None,
        updated_at_ms: None,
        action_key: action_key.clone(),
        mode: agena_domain::PermissionMode::Allow,
        scope: agena_domain::PermissionScope::Workspace,
        session_id: None,
        workspace_id: Some(original_workspace),
        source: "operator-scope-test".into(),
        reason: Some("Allow only in the original workspace".into()),
        operator: None,
        revoked_at_ms: None,
        revoked_reason: None,
        revoked_by: None,
    };
    fixture
        .manager
        .kernel_runtime
        .permissions
        .upsert(&rule)
        .await
        .unwrap();
    fixture.manager.reconfigure(
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        original_executor,
        state.config.clone(),
    );
    let original = fixture
        .manager
        .execute_unscoped_tool(request.clone(), 13)
        .await
        .unwrap();
    assert!(original.into_summary().output_text.contains("one"));

    let other_path = fixture.root.join("other-workspace");
    tokio::fs::create_dir_all(&other_path).await.unwrap();
    let other_path = tokio::fs::canonicalize(other_path).await.unwrap();
    tokio::fs::write(other_path.join("one.txt"), "other workspace")
        .await
        .unwrap();
    let other_workspace = fixture
        .manager
        .kernel_runtime
        .workspaces
        .ensure_id(&other_path.to_string_lossy())
        .await
        .unwrap();
    assert_ne!(original_workspace, other_workspace);
    let other_executor = ToolExecutor::new(
        other_path,
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        state.executor.plugin_manager().clone(),
        None,
        None,
    )
    .with_permission_config(&permissions);
    let other_checks = other_executor
        .collect_permission_checks_for_invocation_in_session(&request, None)
        .await
        .unwrap();
    // Keep the action identical so a wrong workspace lookup would really leak
    // the first workspace's Allow, rather than fail on a different path key.
    assert_eq!(
        action_key,
        super::super::super::execution::permission_key(&other_checks[0].action).unwrap()
    );
    fixture.manager.reconfigure(
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        other_executor,
        state.config.clone(),
    );
    let before = fixture.plugin.contexts.lock().unwrap().len();
    let error = fixture
        .manager
        .execute_unscoped_tool(request.clone(), 14)
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("no session approval channel"),
        "{error}"
    );
    assert_eq!(fixture.plugin.contexts.lock().unwrap().len(), before);
    assert!(fixture.manager.operator_calls.lock().unwrap().is_empty());

    // An explicit grant in the new workspace permits the same real plugin
    // operation, confirming that the denial above was the authority boundary.
    rule.workspace_id = Some(other_workspace);
    rule.reason = Some("An independent grant in the new workspace".into());
    fixture
        .manager
        .kernel_runtime
        .permissions
        .upsert(&rule)
        .await
        .unwrap();
    let allowed = fixture
        .manager
        .execute_unscoped_tool(request, 15)
        .await
        .unwrap();
    assert!(
        allowed
            .into_summary()
            .output_text
            .contains("other workspace")
    );
    fixture.shutdown().await;
}
