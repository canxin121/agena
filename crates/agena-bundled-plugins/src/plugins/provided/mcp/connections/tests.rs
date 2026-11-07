//! Keep only the deterministic reload-queue failure injection.
//! Public connection behavior is exercised by TestAgena in
//! apps/agena/tests/bundled/protocols/mcp/connections.rs.
use std::{
    fs,
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_mcp_client::McpConnectionManager;
use agena_plugin_sdk::{
    InitContext, Plugin, PluginError, Result, ToolInvokeInput, ToolInvokeOutput,
    hooks::{EventEnvelope, EventFilter},
    host_api::{
        EventSubscription, HostClient, HostConfigReloadRequestResponse, HostMcpServerSpec,
        LogLevel, PathPermissionQuery,
    },
};
use serde_json::{Value, json};

use super::McpPlugin;

struct RejectReload {
    global_path: PathBuf,
    workspace_path: PathBuf,
    persisted_at_reload: Mutex<Option<Value>>,
}

#[async_trait::async_trait]
impl HostClient for RejectReload {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Ok(EventSubscription {
            id: "mcp-reload-fault".into(),
        })
    }
    async fn read_config(&self, path: Option<String>) -> Result<Value> {
        match path.as_deref() {
            Some("meta") => Ok(json!({
                "config_path": self.global_path,
                "project_config_path": self.workspace_path
            })),
            Some("config.plugins.list.\"agena.mcp\".settings") => Ok(json!({})),
            _ => Err(PluginError::internal(
                "Unexpected fault-fixture config request",
            )),
        }
    }
    async fn invoke_tool(&self, _: String, _: Value) -> Result<ToolInvokeOutput> {
        unreachable!("saving a disabled connection does not invoke another tool")
    }
    async fn require_path_permission(&self, _: PathPermissionQuery) -> Result<()> {
        Ok(())
    }
    async fn require_mcp_server_permission(&self, _: HostMcpServerSpec) -> Result<()> {
        unreachable!("a disabled connection must not request process authority")
    }
    async fn request_config_reload(&self) -> Result<HostConfigReloadRequestResponse> {
        let document = serde_json::from_slice(&fs::read(&self.workspace_path).unwrap()).unwrap();
        *self.persisted_at_reload.lock().unwrap() = Some(document);
        Err(PluginError::internal("Injected reload queue failure"))
    }
}

#[tokio::test]
async fn reload_queue_failure_reports_saved_state_without_losing_the_persisted_change() {
    let directory = tempfile::tempdir().unwrap();
    let workspace = directory.path().join("workspace");
    let global_path = directory.path().join("global.json");
    let workspace_path = workspace.join(".agena/agena.json");
    fs::create_dir_all(workspace_path.parent().unwrap()).unwrap();
    fs::write(&global_path, "{}").unwrap();
    fs::write(&workspace_path, "{}").unwrap();
    let host = Arc::new(RejectReload {
        global_path,
        workspace_path,
        persisted_at_reload: Mutex::new(None),
    });
    let plugin = McpPlugin::new(Arc::new(McpConnectionManager::new(
        "mcp-reload-fault",
        "test",
    )));
    Plugin::init(
        &plugin,
        InitContext {
            agena_version: "test".into(),
            workspace_root: workspace.clone(),
            plugin_id: agena_plugin_sdk::PluginKey::new("agena", "mcp").unwrap(),
            host_callback_url: None,
            host_callback_token: None,
            settings: json!({}),
            protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
        },
        host.clone(),
    )
    .await
    .unwrap();

    let output = tokio::time::timeout(
        Duration::from_secs(5),
        Plugin::tool_invoke(
            &plugin,
            ToolInvokeInput {
                tool_name: "connections.create".into(),
                session_id: 1,
                call_id: 2,
                workspace_root: workspace.to_string_lossy().into_owned(),
                input: json!({"name": "saved", "scope": "workspace", "config": {
                    "transport": "stdio", "enabled": false, "process": {"command": "fixture-mcp"}
                }}),
            },
        ),
    )
    .await
    .expect("queue failure must return after saving")
    .unwrap()
    .payload
    .unwrap();
    assert_eq!(output["saved"], true);
    assert!(output["reload_task_id"].is_null());
    assert!(
        output["warning"]
            .as_str()
            .unwrap()
            .contains("Reload from Diagnostics")
    );
    let stored: Value = serde_json::from_slice(&fs::read(&host.workspace_path).unwrap()).unwrap();
    assert_eq!(
        stored["plugins"]["list"]["agena.mcp"]["settings"]["servers"]["saved"]["process"]["command"],
        "fixture-mcp"
    );
    assert_eq!(
        *host.persisted_at_reload.lock().unwrap(),
        Some(stored),
        "the exact committed document must precede the failing reload queue call"
    );
    assert!(plugin.manager.statuses().await.is_empty());
}
