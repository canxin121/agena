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
