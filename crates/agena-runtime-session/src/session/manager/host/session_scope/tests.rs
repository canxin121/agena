use super::*;
use agena_domain::StructuredObject;
use agena_plugin_host::sdk::{self, host_api, host_api::HostClient};
use serde_json::{Value, json};
use std::sync::OnceLock;

#[derive(Default)]
struct PermissionHost {
    manager: OnceLock<Arc<SessionManager>>,
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
            id: "session-scope-test".into(),
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
        let manager = self.manager.get().unwrap().clone();
        // A delegation carries a negative call id, which routes the
        // permission check to the session policy instead of a live tool slot.
        let call = context.call_id.expect("delegation carries a call id");
        assert!(call < 0, "delegation call id must be negative, got {call}");
        manager
            .require_host_path_permission(
                context.session_id.unwrap(),
                call,
                request.kind,
                request.path,
            )
            .await
            .map_err(|error| {
                sdk::PluginError::from_kind(sdk::PluginErrorKind::PolicyDenied, error.to_string())
            })
    }
}

#[derive(Default)]
struct ObserverState {
    host: OnceLock<Arc<dyn HostClient>>,
    contexts: StdMutex<Vec<host_api::HostCallbackContext>>,
}

/// Declares tools but is never invoked through the host; it only exercises
/// session-scoped callbacks reaching their own tools.
struct ObserverPlugin(Arc<ObserverState>);

#[sdk::async_trait]
impl sdk::Plugin for ObserverPlugin {
    fn manifest(&self) -> sdk::PluginManifest {
        let mut manifest = sdk::PluginManifest::new("test", "session_scope", "1.0.0");
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
                applies_to: Default::default(),
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
        self.0.contexts.lock().unwrap().push(context);
        let host = self.0.host.get().unwrap();
        match input.tool_name.as_str() {
            "read_many" => {
                let paths = input.input["paths"].as_array().unwrap();
                for path in paths {
                    host.require_path_permission(host_api::PathPermissionQuery::read(
                        path.as_str().unwrap(),
                    ))
                    .await?;
                }
                Ok(sdk::ToolInvokeOutput::text("ok"))
            }
            "write" => {
                let path = input.input["path"].as_str().unwrap();
                host.require_path_permission(host_api::PathPermissionQuery {
                    kind: agena_domain::PathKind::Write,
                    path: path.into(),
                })
                .await?;
                Ok(sdk::ToolInvokeOutput::text("written"))
            }
            _ => unreachable!(),
        }
    }
}

struct Fixture {
    manager: Arc<SessionManager>,
    plugin: Arc<ObserverState>,
    session: i64,
    root: std::path::PathBuf,
}

impl Fixture {
    async fn new(write_mode: &str) -> Self {
        let root =
            std::env::temp_dir().join(format!("agena-session-scope-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("workspace");
        tokio::fs::create_dir_all(&workspace).await.unwrap();
        let workspace = tokio::fs::canonicalize(workspace).await.unwrap();
        tokio::fs::write(workspace.join("one.txt"), "one")
            .await
            .unwrap();
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let workspace_id = kernel
            .workspaces
            .ensure_id(&workspace.to_string_lossy())
            .await
            .unwrap();
        let session = kernel
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace_id,
                "Session scope".into(),
                Default::default(),
            )
            .await
            .unwrap()
            .value
            .id
            .get();
        let host = Arc::new(PermissionHost::default());
        let plugin = Arc::new(ObserverState::default());
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                static_plugins: vec![agena_plugin_host::StaticPluginRegistration::new(
                    "test.session_scope".parse().unwrap(),
                    ObserverPlugin(plugin.clone()),
                )],
                config: agena_plugin_host::PluginsConfig {
                    list: [("test.session_scope".into(), Default::default())].into(),
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
                "workspace": if write_mode == "deny" { "readonly" } else { "readwrite" },
                "external": "deny"
            },
            "tools": {
                "default": "allow",
                "names": {
                    "test.session_scope.write": if matches!(write_mode, "ask" | "auto") { write_mode } else { "allow" }
                }
            }
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
        assert!(host.manager.set(manager.clone()).is_ok());
        Self {
            manager,
            plugin,
            session,
            root,
        }
    }

    async fn call(&self, tool: &str, input: Value) -> Result<ToolInvocationExecution, AppError> {
        self.manager
            .execute_session_scoped_tool(
                self.session,
                ToolInvocation::new(
                    format!("test.session_scope.{tool}"),
                    StructuredObject::try_from(input).unwrap(),
                ),
            )
            .await
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

#[tokio::test]
async fn registered_working_directories_compile_into_only_the_owning_sandbox() {
    use agena_domain::{AccessKind, RegisterSessionWorkingDirectory};

    let fixture = Fixture::new("allow").await;
    let extra = fixture.root.join("additional directory");
    let git_config = extra.join(".git/config");
    tokio::fs::create_dir_all(git_config.parent().unwrap())
        .await
        .unwrap();
    tokio::fs::write(&git_config, "[core]\n").await.unwrap();
    let extra = tokio::fs::canonicalize(extra).await.unwrap();
    let manager = &fixture.manager;
    let workspace = manager
        .kernel()
        .reader
        .metadata(session_id(fixture.session).unwrap())
        .await
        .unwrap()
        .workspace_id
        .unwrap();
    let other = manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Separate session".into(),
            SessionConfiguration {
                permission_ceiling: serde_json::from_value(json!({"path":{"external":"deny"}}))
                    .unwrap(),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id
        .get();
    let request = RegisterSessionWorkingDirectory {
        path: extra.clone(),
        access: agena_domain::SessionWorkingDirectoryAccess::Inherit,
    };
    let (left, right) = tokio::join!(
        manager.add_session_working_directory(fixture.session, request.clone()),
        manager.add_session_working_directory(fixture.session, request.clone()),
    );
    let (left, right) = (left.unwrap(), right.unwrap());
    assert_ne!(left.added, right.added);
    assert_eq!(left.directories, [extra.clone()]);
    assert_eq!(right.directories, [extra.clone()]);
    assert!(matches!(
        manager.add_session_working_directory(other, request).await,
        Err(AppError::PolicyDenied(_))
    ));

    let owner = manager
        .host_resource_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    let isolated = manager
        .host_resource_callback_executor(Some(other), None)
        .await
        .unwrap();
    let git_config = extra.join(".git/config");
    let snapshot = owner.sandbox_permission_snapshot();
    let native = snapshot.filesystem_policy().unwrap();
    assert!(
        native
            .readwrite_paths
            .iter()
            .any(|path| path.canonicalize().is_ok_and(|root| root == extra))
    );
    assert!(
        native
            .readonly_paths
            .iter()
            .any(|path| path.canonicalize().is_ok_and(|file| file == git_config))
    );
    assert_eq!(
        snapshot.authorized_additional_working_directories(),
        [extra.clone()]
    );
    let isolated = isolated.sandbox_permission_snapshot();
    assert!(matches!(
        isolated.authorize_path_access(AccessKind::Read, &extra),
        PermissionDecision::Deny { .. }
    ));
    assert!(
        !isolated
            .filesystem_policy()
            .unwrap()
            .readwrite_paths
            .iter()
            .any(|path| path.canonicalize().is_ok_and(|root| root == extra))
    );

    #[cfg(unix)]
    {
        let renamed = extra.with_file_name("moved directory");
        tokio::fs::rename(&extra, &renamed).await.unwrap();
        std::os::unix::fs::symlink(&renamed, &extra).unwrap();
        let refreshed = manager
            .host_resource_callback_executor(Some(fixture.session), None)
            .await
            .unwrap();
        assert!(
            refreshed
                .sandbox_permission_snapshot()
                .authorized_additional_working_directories()
                .is_empty()
        );
        assert!(matches!(
            refreshed
                .sandbox_permission_snapshot()
                .authorize_path_access(AccessKind::Read, &renamed),
            PermissionDecision::Deny { .. }
        ));
    }
    fixture.shutdown().await;
}

#[tokio::test]
async fn working_directory_access_modes_update_and_removal_enforce_sandbox_boundaries() {
    use agena_domain::{
        AccessKind, RegisterSessionWorkingDirectory, RemoveSessionWorkingDirectory,
        SessionWorkingDirectoryAccess,
    };

    let fixture = Fixture::new("allow").await;
    let extra_ro = fixture.root.join("extra-readonly");
    let extra_nested_rw = extra_ro.join("writable-sub");
    tokio::fs::create_dir_all(&extra_nested_rw).await.unwrap();
    let extra_ro = tokio::fs::canonicalize(&extra_ro).await.unwrap();
    let extra_nested_rw = tokio::fs::canonicalize(&extra_nested_rw).await.unwrap();
    let manager = &fixture.manager;

    let reg = manager
        .add_session_working_directory(
            fixture.session,
            RegisterSessionWorkingDirectory {
                path: extra_ro.clone(),
                access: SessionWorkingDirectoryAccess::ReadOnly,
            },
        )
        .await
        .unwrap();
    assert!(reg.added && !reg.updated);
    assert_eq!(reg.access, SessionWorkingDirectoryAccess::ReadOnly);
    assert_eq!(reg.entries.len(), 1);
    assert_eq!(reg.entries[0].access, SessionWorkingDirectoryAccess::ReadOnly);

    let executor = manager
        .host_resource_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    let snapshot = executor.sandbox_permission_snapshot();
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Read, &extra_ro),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Write, &extra_ro),
        PermissionDecision::Deny { .. }
    ));
    let fs_policy = snapshot.filesystem_policy().unwrap();
    assert!(
        fs_policy
            .readonly_paths
            .iter()
            .any(|p| p.canonicalize().is_ok_and(|c| c == extra_ro))
    );
    assert!(
        !fs_policy
            .readwrite_paths
            .iter()
            .any(|p| p.canonicalize().is_ok_and(|c| c == extra_ro))
    );

    let nested = manager
        .add_session_working_directory(
            fixture.session,
            RegisterSessionWorkingDirectory {
                path: extra_nested_rw.clone(),
                access: SessionWorkingDirectoryAccess::ReadWrite,
            },
        )
        .await
        .unwrap();
    assert!(nested.added && !nested.updated);
    let executor = manager
        .host_resource_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    let snapshot = executor.sandbox_permission_snapshot();
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Write, &extra_ro),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Write, &extra_nested_rw),
        PermissionDecision::Allow
    ));

    let upgraded = manager
        .add_session_working_directory(
            fixture.session,
            RegisterSessionWorkingDirectory {
                path: extra_ro.clone(),
                access: SessionWorkingDirectoryAccess::ReadWrite,
            },
        )
        .await
        .unwrap();
    assert!(!upgraded.added && upgraded.updated);
    let executor = manager
        .host_resource_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    let snapshot = executor.sandbox_permission_snapshot();
    assert!(matches!(
        snapshot.authorize_path_access(AccessKind::Write, &extra_ro),
        PermissionDecision::Allow
    ));

    let rm1 = manager
        .remove_session_working_directory(
            fixture.session,
            RemoveSessionWorkingDirectory {
                path: extra_nested_rw.clone(),
            },
        )
        .await
        .unwrap();
    assert!(rm1.removed);
    let rm2 = manager
        .remove_session_working_directory(
            fixture.session,
            RemoveSessionWorkingDirectory {
                path: extra_ro.clone(),
            },
        )
        .await
        .unwrap();
    assert!(rm2.removed);
    assert!(rm2.entries.is_empty());

    let rm_again = manager
        .remove_session_working_directory(
            fixture.session,
            RemoveSessionWorkingDirectory {
                path: extra_ro.clone(),
            },
        )
        .await
        .unwrap();
    assert!(!rm_again.removed);

    let after = manager
        .host_resource_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    assert!(matches!(
        after
            .sandbox_permission_snapshot()
            .authorize_path_access(AccessKind::Read, &extra_ro),
        PermissionDecision::Deny { .. }
    ));
    fixture.shutdown().await;
}

#[tokio::test]
async fn session_scoped_delegation_has_no_tool_call_authority() {
    let fixture = Fixture::new("allow").await;
    let execution = fixture
        .call("read_many", json!({"paths": ["one.txt"]}))
        .await
        .unwrap();
    assert_eq!(execution.view.output_text, "ok");
    let contexts = fixture.plugin.contexts.lock().unwrap().clone();
    assert_eq!(contexts.len(), 1);
    // The plugin sees the owning session and a negative delegation id, never
    // a real transcript tool id.
    assert_eq!(contexts[0].session_id, Some(fixture.session));
    let call = contexts[0].call_id.expect("delegation carries a call id");
    assert!(call < 0, "delegation call id must be negative, got {call}");
    assert!(contexts[0].authority_token.is_some());
    assert!(fixture.manager.tools.lock().unwrap().is_empty());
    fixture.shutdown().await;
}

#[tokio::test]
async fn session_scoped_delegation_never_promotes_ask_or_auto() {
    for mode in ["ask", "auto"] {
        let fixture = Fixture::new(mode).await;
        let error = fixture
            .call("write", json!({"path": "blocked.txt"}))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("no approval channel"), "{error}");
        assert!(
            !std::path::Path::new(&fixture.root)
                .join("workspace/blocked.txt")
                .exists()
        );
        fixture.shutdown().await;
    }
}

#[tokio::test]
async fn session_scoped_delegation_honours_a_workspace_rule() {
    let fixture = Fixture::new("deny").await;
    let error = fixture
        .call("write", json!({"path": "denied.txt"}))
        .await
        .unwrap_err();
    assert!(
        error.to_string().contains("permission policy"),
        "expected a policy denial, got {error}"
    );
    assert!(
        !std::path::Path::new(&fixture.root)
            .join("workspace/denied.txt")
            .exists()
    );
    fixture.shutdown().await;
}

fn callback_network_permission(
    network: agena_domain::NetworkPermissionConfig,
) -> agena_domain::PermissionConfig {
    agena_domain::PermissionConfig {
        network: Some(network),
        ..Default::default()
    }
}

fn enable_callback_network(manager: &SessionManager) {
    let state = manager.state.load_full();
    let permission =
        callback_network_permission(agena_domain::NetworkPermissionConfig::allow_all());
    let executor = state.executor.clone().with_permission_config(&permission);
    let mut config = state.config.clone();
    config.permission = permission;
    manager.reconfigure(
        state.providers.clone(),
        state.governor.clone(),
        state.processor.clone(),
        executor.clone(),
        config.clone(),
    );
    manager.reconfigure_global(
        state.providers.clone(),
        state.processor.clone(),
        executor,
        config,
    );
}

#[cfg(not(target_os = "macos"))]
async fn save_callback_network_rule(
    manager: &SessionManager,
    scope: agena_domain::PermissionScope,
    owner: Option<i64>,
    address: &str,
    mode: agena_domain::PermissionMode,
    port: Option<u16>,
) {
    use agena_domain::{PermissionAction, PermissionScope};
    manager
        .kernel_runtime
        .permissions
        .upsert(&agena_storage::PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key: serde_json::to_string(&PermissionAction::NetworkAccess {
                target: port.map_or_else(
                    || format!("tcp://{address}"),
                    |port| format!("{address}:{port}"),
                ),
                host: address.into(),
                port,
            })
            .unwrap(),
            mode,
            scope,
            session_id: (scope == PermissionScope::Session)
                .then_some(owner)
                .flatten(),
            workspace_id: (scope == PermissionScope::Workspace)
                .then_some(owner)
                .flatten(),
            source: "callback-network-test".into(),
            reason: None,
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        })
        .await
        .unwrap();
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn callback_network_snapshot_reloads_numeric_denies_in_the_owning_scopes() {
    use agena_domain::{PermissionDecision, PermissionMode, PermissionScope};

    let fixture = Fixture::new("allow").await;
    enable_callback_network(&fixture.manager);
    let workspace = fixture.manager.current_workspace_id().await.unwrap();
    let other_workspace = fixture
        .manager
        .kernel_runtime
        .workspaces
        .ensure_id(&fixture.root.join("other-workspace").to_string_lossy())
        .await
        .unwrap();
    let other_session = fixture
        .manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Other callback scope".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id
        .get();
    let global_session = fixture
        .manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "Global callback scope".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id
        .get();
    for (scope, owner, address) in [
        (PermissionScope::Global, None, "192.0.2.10"),
        (PermissionScope::Workspace, Some(workspace), "192.0.2.11"),
        (
            PermissionScope::Session,
            Some(fixture.session),
            "192.0.2.12",
        ),
        (
            PermissionScope::Workspace,
            Some(other_workspace),
            "192.0.2.13",
        ),
        (PermissionScope::Session, Some(other_session), "192.0.2.14"),
    ] {
        save_callback_network_rule(
            &fixture.manager,
            scope,
            owner,
            address,
            PermissionMode::Deny,
            Some(443),
        )
        .await;
    }

    let original = "example.test:443".parse().unwrap();
    for (session, denied) in [
        (Some(fixture.session), vec![10, 11, 12]),
        (Some(global_session), vec![10]),
        (None, vec![10, 11]),
    ] {
        let (executor, context) = fixture
            .manager
            .host_callback_executor(session, None)
            .await
            .unwrap();
        assert_eq!(context.and_then(|value| value.scope_session_id), session);
        let snapshot = executor.sandbox_permission_snapshot();
        for octet in 10..=15 {
            let resolved = format!("192.0.2.{octet}:443").parse().unwrap();
            let decision = snapshot.authorize_resolved_network_address(&original, &resolved);
            assert_eq!(
                matches!(decision, PermissionDecision::Deny { .. }),
                denied.contains(&octet),
                "session {session:?}, {resolved:?}: {decision:?}"
            );
        }
    }

    // A subsequent DNS callback must see a decision added after the earlier
    // callback, even though its hostname action key is entirely different.
    save_callback_network_rule(
        &fixture.manager,
        PermissionScope::Session,
        Some(fixture.session),
        "192.0.2.15",
        PermissionMode::Deny,
        Some(443),
    )
    .await;
    let (executor, _) = fixture
        .manager
        .host_callback_executor(Some(fixture.session), None)
        .await
        .unwrap();
    assert!(matches!(
        executor
            .sandbox_permission_snapshot()
            .authorize_resolved_network_address(&original, &"192.0.2.15:443".parse().unwrap()),
        PermissionDecision::Deny { .. }
    ));
    assert!(
        fixture
            .manager
            .host_callback_executor(Some(fixture.session), Some(999_999))
            .await
            .is_err()
    );
    fixture.shutdown().await;
}

#[tokio::test]
async fn callback_network_snapshot_retains_session_policy_and_permission_ceiling() {
    use agena_domain::{NetworkPermissionConfig, PermissionDecision};

    let fixture = Fixture::new("allow").await;
    enable_callback_network(&fixture.manager);
    let workspace = fixture.manager.current_workspace_id().await.unwrap();
    let allowed = callback_network_permission(NetworkPermissionConfig::allow_all());
    let denied = callback_network_permission(NetworkPermissionConfig::offline());
    for (permission, permission_ceiling) in
        [(denied.clone(), Default::default()), (allowed, denied)]
    {
        let session = fixture
            .manager
            .kernel()
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Restricted callback scope".into(),
                SessionConfiguration {
                    permission,
                    permission_ceiling,
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id
            .get();
        #[cfg(not(target_os = "macos"))]
        save_callback_network_rule(
            &fixture.manager,
            agena_domain::PermissionScope::Session,
            Some(session),
            "192.0.2.20",
            agena_domain::PermissionMode::Allow,
            Some(443),
        )
        .await;
        let (executor, _) = fixture
            .manager
            .host_callback_executor(Some(session), None)
            .await
            .unwrap();
        assert!(matches!(
            executor
                .sandbox_permission_snapshot()
                .authorize_resolved_network_address(
                    &"example.test:443".parse().unwrap(),
                    &"192.0.2.20:443".parse().unwrap()
                ),
            PermissionDecision::Deny { .. }
        ));
        assert!(
            fixture
                .manager
                .require_host_network_permission(
                    session,
                    host_api::SESSION_SCOPED_CALL_ID,
                    "192.0.2.20:443".into(),
                )
                .await
                .is_err()
        );
        assert!(matches!(
            fixture
                .manager
                .require_host_pinned_http_permission(
                    session,
                    host_api::SESSION_SCOPED_CALL_ID,
                    "https://192.0.2.20/".into(),
                )
                .await,
            Err(AppError::PolicyDenied(_)),
        ));
    }
    fixture.shutdown().await;
}

#[cfg(not(target_os = "macos"))]
#[tokio::test]
async fn callback_network_snapshot_enforces_portless_tcp_denies_before_io() {
    use agena_domain::{PermissionMode, PermissionScope};

    let fixture = Fixture::new("allow").await;
    enable_callback_network(&fixture.manager);
    let workspace = fixture.manager.current_workspace_id().await.unwrap();
    for (scope, owner, address) in [
        (PermissionScope::Global, None, "192.0.2.21"),
        (PermissionScope::Workspace, Some(workspace), "192.0.2.22"),
        (
            PermissionScope::Session,
            Some(fixture.session),
            "192.0.2.23",
        ),
    ] {
        save_callback_network_rule(
            &fixture.manager,
            scope,
            owner,
            address,
            PermissionMode::Deny,
            None,
        )
        .await;
        for target in [
            format!("{address}:80"),
            format!("https://[::ffff:{address}]/"),
        ] {
            assert!(
                fixture
                    .manager
                    .require_host_network_permission(
                        fixture.session,
                        host_api::SESSION_SCOPED_CALL_ID,
                        target.clone(),
                    )
                    .await
                    .is_err(),
                "persisted unqualified TCP deny must block {target}"
            );
        }
        fixture
            .manager
            .require_host_network_permission(
                fixture.session,
                host_api::SESSION_SCOPED_CALL_ID,
                format!("udp://{address}:443"),
            )
            .await
            .unwrap();
    }
    fixture.shutdown().await;
}
