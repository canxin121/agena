use std::{
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use agena_domain::{PathKind, mcp_connections::McpConnectionScope};
use agena_mcp_client::McpConnectionManager;
use agena_plugin_sdk::{
    InitContext, Plugin, PluginError, PluginErrorKind, Result, ToolInvokeInput, ToolInvokeOutput,
    hooks::{EventEnvelope, EventFilter},
    host_api::{
        EventSubscription, HostClient, HostConfigReloadRequestResponse, HostMcpServerSpec,
        LogLevel, PathPermissionQuery,
    },
};
use agena_runtime_config::mcp_connections::{
    McpConnectionStore, connection_id, merge_mcp_settings,
};
use serde_json::{Value, json};
use tempfile::TempDir;

use super::{ConnectionUpdateInput, McpPlugin};

#[derive(Default)]
struct Policy {
    deny_read: bool,
    deny_write: bool,
    deny_mcp: bool,
    deny_reload: bool,
}

struct Host {
    global_path: PathBuf,
    workspace_path: PathBuf,
    active_settings: Value,
    policy: Policy,
    paths: Mutex<Vec<PathPermissionQuery>>,
    authority: Mutex<Vec<HostMcpServerSpec>>,
    config_reads: Mutex<Vec<String>>,
    reload_documents: Mutex<Vec<Value>>,
    events: Mutex<Vec<&'static str>>,
}

#[async_trait::async_trait]
impl HostClient for Host {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Ok(EventSubscription {
            id: "mcp-test".into(),
        })
    }
    async fn read_config(&self, path: Option<String>) -> Result<Value> {
        self.config_reads
            .lock()
            .unwrap()
            .push(path.clone().unwrap_or_default());
        match path.as_deref() {
            Some("meta") => Ok(json!({
                "config_path":self.global_path,
                "project_config_path":self.workspace_path
            })),
            Some("config.plugins.list.\"agena.mcp\".settings") => Ok(self.active_settings.clone()),
            _ => Err(PluginError::internal(
                "Unexpected test configuration request.",
            )),
        }
    }
    async fn invoke_tool(&self, _: String, _: Value) -> Result<ToolInvokeOutput> {
        panic!("Connection management must not invoke another tool implicitly");
    }
    async fn require_path_permission(&self, request: PathPermissionQuery) -> Result<()> {
        let denied = match request.kind {
            PathKind::Read => self.policy.deny_read,
            PathKind::Write => self.policy.deny_write,
        };
        self.events.lock().unwrap().push(match request.kind {
            PathKind::Read => "read",
            PathKind::Write => "write",
        });
        self.paths.lock().unwrap().push(request);
        if denied {
            Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "Test config path denied",
            ))
        } else {
            Ok(())
        }
    }
    async fn require_mcp_server_permission(&self, spec: HostMcpServerSpec) -> Result<()> {
        self.events.lock().unwrap().push("authority");
        self.authority.lock().unwrap().push(spec);
        if self.policy.deny_mcp {
            Err(PluginError::from_kind(
                PluginErrorKind::UserDeclined,
                "Test process execution declined",
            ))
        } else {
            Ok(())
        }
    }
    async fn request_config_reload(&self) -> Result<HostConfigReloadRequestResponse> {
        self.events.lock().unwrap().push("reload");
        // Observe persisted data at the reload boundary: no process is started.
        let document = read_document(&self.workspace_path);
        let mut reloads = self.reload_documents.lock().unwrap();
        reloads.push(document);
        if self.policy.deny_reload {
            Err(PluginError::internal("Test reload unavailable"))
        } else {
            Ok(HostConfigReloadRequestResponse {
                task_id: format!("reload-{}", reloads.len()),
                started: true,
            })
        }
    }
}

struct Fixture {
    _directory: TempDir,
    server_workspace: PathBuf,
    chat_workspace: PathBuf,
    host: Arc<Host>,
    plugin: McpPlugin,
}

impl Fixture {
    async fn new(global: Value, workspace: Value, policy: Policy) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let global_path = directory.path().join("global/config.json");
        let server_workspace = directory.path().join("server-workspace");
        let workspace_path = server_workspace.join(".agena/config.json");
        let chat_workspace = directory.path().join("selected-chat-workspace");
        fs::create_dir_all(&chat_workspace).unwrap();
        write_document(&global_path, &global);
        write_document(&workspace_path, &workspace);
        let empty = json!({});
        let settings = |value: &Value| {
            value
                .pointer("/plugins/list/agena.mcp/settings")
                .cloned()
                .unwrap_or_else(|| empty.clone())
        };
        let active_settings = merge_mcp_settings(&settings(&global), &settings(&workspace));
        let host = Arc::new(Host {
            global_path,
            workspace_path,
            active_settings,
            policy,
            paths: Mutex::new(vec![]),
            authority: Mutex::new(vec![]),
            config_reads: Mutex::new(vec![]),
            reload_documents: Mutex::new(vec![]),
            events: Mutex::new(vec![]),
        });
        let plugin = McpPlugin::new(Arc::new(McpConnectionManager::new(
            "mcp-management-test",
            "test",
        )));
        Plugin::init(
            &plugin,
            InitContext {
                agena_version: "test".into(),
                workspace_root: server_workspace.clone(),
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
        Self {
            _directory: directory,
            server_workspace,
            chat_workspace,
            host,
            plugin,
        }
    }

    async fn invoke(&self, name: &str, input: Value) -> Result<ToolInvokeOutput> {
        tokio::time::timeout(
            Duration::from_secs(5),
            Plugin::tool_invoke(
                &self.plugin,
                ToolInvokeInput {
                    tool_name: name.into(),
                    session_id: 1,
                    call_id: 2,
                    workspace_root: self.chat_workspace.to_string_lossy().into_owned(),
                    input,
                },
            ),
        )
        .await
        .expect("Management calls must finish without starting a transport")
    }

    fn snapshot(&self) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(&self.host.global_path).unwrap(),
            fs::read(&self.host.workspace_path).unwrap(),
        )
    }

    fn raw(&self, id: &str) -> Value {
        McpConnectionStore::new(
            self.host.global_path.clone(),
            self.host.workspace_path.clone(),
            self.server_workspace.clone(),
        )
        .raw(id)
        .unwrap()
        .2
    }
}

fn write_document(path: &Path, value: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
}
fn read_document(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}
fn document(servers: Value) -> Value {
    json!({"plugins":{"list":{"agena.mcp":{"enabled":true,"package":{"kind":"static"},"settings":{"servers":servers}}}}})
}
fn stdio() -> Value {
    json!({"transport":"stdio","process":{"command":"fixture-mcp","args":["two words","$(literal)"],"cwd":"services","env":{"TOKEN":"create-private-token"}}})
}

#[tokio::test]
async fn reads_require_both_config_paths_and_return_redacted_values_in_every_output_channel() {
    let fixture=Fixture::new(json!({}),document(json!({"remote":{
        "transport":"http","endpoint":{"url":"https://example.test/mcp?key=private-url","headers":{"X-Key":"private-header"}},
        "auth":{"kind":"bearer","token":"private-bearer"}
    }})),Policy::default()).await;
    let id = connection_id(McpConnectionScope::Workspace, "remote");
    for (name, input) in [
        ("connections.list", json!({})),
        ("connections.get", json!({"id":id})),
    ] {
        fixture.host.paths.lock().unwrap().clear();
        let output = fixture.invoke(name, input).await.unwrap();
        let serialized = serde_json::to_string(&output).unwrap();
        for secret in ["private-url", "private-header", "private-bearer"] {
            assert!(!serialized.contains(secret));
        }
        let paths = fixture.host.paths.lock().unwrap();
        assert_eq!(paths.len(), 2);
        assert!(paths.iter().all(|request| request.kind == PathKind::Read));
        assert!(
            paths
                .iter()
                .any(|request| Path::new(&request.path) == fixture.host.global_path)
        );
        assert!(
            paths
                .iter()
                .any(|request| Path::new(&request.path) == fixture.host.workspace_path)
        );
        if name == "connections.list" {
            assert_eq!(
                output.payload.unwrap()["workspace_root"],
                fixture.server_workspace.to_string_lossy().as_ref()
            );
        } else {
            let payload = output.payload.unwrap();
            assert_eq!(payload["config"]["auth"]["token"], "[REDACTED]");
            assert_eq!(payload["secret_paths"].as_array().unwrap().len(), 3);
        }
    }
    assert!(fixture.host.authority.lock().unwrap().is_empty());
    assert!(fixture.host.reload_documents.lock().unwrap().is_empty());
}

#[tokio::test]
async fn denied_config_reads_fail_before_parsing_files_and_preserve_policy_error() {
    let fixture = Fixture::new(
        json!({}),
        json!({}),
        Policy {
            deny_read: true,
            ..Default::default()
        },
    )
    .await;
    fs::write(&fixture.host.global_path, "invalid JSON must never be read").unwrap();
    let before = fixture.snapshot();
    for (name, input) in [
        ("connections.list", json!({})),
        ("connections.get", json!({"id":"unused"})),
    ] {
        let error = fixture.invoke(name, input).await.unwrap_err();
        assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
        assert_eq!(fixture.snapshot(), before);
    }
    assert!(
        fixture
            .host
            .config_reads
            .lock()
            .unwrap()
            .iter()
            .all(|path| path == "meta")
    );
    assert!(fixture.host.authority.lock().unwrap().is_empty());
    assert!(fixture.host.reload_documents.lock().unwrap().is_empty());
}

#[tokio::test]
async fn denied_process_authorization_leaves_config_unchanged_and_does_not_queue_reload() {
    let fixture = Fixture::new(
        json!({}),
        json!({}),
        Policy {
            deny_mcp: true,
            ..Default::default()
        },
    )
    .await;
    let before = fixture.snapshot();
    let error = fixture
        .invoke(
            "connections.create",
            json!({"name":"blocked","scope":"workspace","config":stdio()}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::UserDeclined);
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.host.authority.lock().unwrap().len(), 1);
    assert!(fixture.host.reload_documents.lock().unwrap().is_empty());
    assert!(fixture.plugin.manager.statuses().await.is_empty());
    assert!(
        fixture
            .host
            .paths
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.kind == PathKind::Write
                && Path::new(&request.path) == fixture.host.workspace_path)
    );
}

#[tokio::test]
async fn denied_config_write_never_requests_execution_authority_or_reload() {
    let fixture = Fixture::new(
        json!({}),
        json!({}),
        Policy {
            deny_write: true,
            ..Default::default()
        },
    )
    .await;
    let before = fixture.snapshot();
    let error = fixture
        .invoke(
            "connections.create",
            json!({"name":"blocked","scope":"workspace","config":stdio()}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert_eq!(fixture.snapshot(), before);
    assert!(fixture.host.authority.lock().unwrap().is_empty());
    assert!(fixture.host.reload_documents.lock().unwrap().is_empty());
}

#[tokio::test]
async fn accepted_mutation_authorizes_server_workspace_then_queues_reload_after_persisting() {
    let fixture = Fixture::new(json!({}), json!({}), Policy::default()).await;
    let before_global = fixture.snapshot().0;
    let output = fixture
        .invoke(
            "connections.create",
            json!({"name":"created","scope":"workspace","config":stdio()}),
        )
        .await
        .unwrap();
    assert!(
        !serde_json::to_string(&output)
            .unwrap()
            .contains("create-private-token")
    );
    let payload = output.payload.unwrap();
    assert_eq!(payload["saved"], true);
    assert_eq!(payload["reload_task_id"], "reload-1");
    assert!(payload["warning"].as_str().unwrap().contains("queued"));
    assert_eq!(payload["connection"]["state"], "pending_reload");
    {
        let reloads = fixture.host.reload_documents.lock().unwrap();
        assert_eq!(reloads.len(), 1);
        assert_eq!(
            reloads[0]["plugins"]["list"]["agena.mcp"]["settings"]["servers"]["created"]["process"]
                ["command"],
            "fixture-mcp"
        );
    }
    {
        let authority = fixture.host.authority.lock().unwrap();
        let HostMcpServerSpec::Stdio {
            command,
            args,
            env,
            cwd,
        } = &authority[0]
        else {
            panic!("Expected stdio authorization");
        };
        assert_eq!(command, "fixture-mcp");
        assert_eq!(args, &["two words".to_owned(), "$(literal)".to_owned()]);
        assert_eq!(env["TOKEN"], "create-private-token");
        assert_eq!(
            cwd.as_deref(),
            fixture.server_workspace.join("services").to_str()
        );
    }
    {
        let events = fixture.host.events.lock().unwrap();
        let write = events.iter().position(|event| *event == "write").unwrap();
        let authorize = events
            .iter()
            .position(|event| *event == "authority")
            .unwrap();
        let reload = events.iter().position(|event| *event == "reload").unwrap();
        assert!(write < authorize && authorize < reload);
    }
    assert_eq!(fs::read(&fixture.host.global_path).unwrap(), before_global);
    assert!(!fixture.chat_workspace.join(".agena").exists());
    assert!(fixture.plugin.manager.statuses().await.is_empty());
}

#[tokio::test]
async fn flat_update_input_keeps_only_requested_secrets_and_omitted_preservation_clears_removed_fields()
 {
    let fixture=Fixture::new(json!({}),document(json!({"editable":{
        "transport":"stdio","enabled":false,"process":{"command":"fixture-mcp","env":{"KEEP":"keep-private-token","REMOVE":"remove-private-token"}}
    }})),Policy::default()).await;
    let id = connection_id(McpConnectionScope::Workspace, "editable");
    let detail = fixture
        .invoke("connections.get", json!({"id":id}))
        .await
        .unwrap()
        .payload
        .unwrap();
    let mut config = detail["config"].clone();
    config["process"]["env"]
        .as_object_mut()
        .unwrap()
        .remove("REMOVE");
    let request = json!({"id":id,"config":config,"expected_revision":detail["revision"],"preserve_secrets":["/process/env/KEEP"]});
    let parsed: ConnectionUpdateInput = serde_json::from_value(request.clone()).unwrap();
    assert_eq!(parsed.id, id);
    assert_eq!(parsed.request.preserve_secrets, vec!["/process/env/KEEP"]);
    let output = fixture.invoke("connections.update", request).await.unwrap();
    for secret in ["keep-private-token", "remove-private-token"] {
        assert!(!serde_json::to_string(&output).unwrap().contains(secret));
    }
    let raw = fixture.raw(&id);
    assert_eq!(raw["process"]["env"], json!({"KEEP":"keep-private-token"}));

    let detail = fixture
        .invoke("connections.get", json!({"id":id}))
        .await
        .unwrap()
        .payload
        .unwrap();
    let mut config = detail["config"].clone();
    config["process"]["env"] = json!({});
    let request = json!({"id":id,"config":config,"expected_revision":detail["revision"]});
    let parsed: ConnectionUpdateInput = serde_json::from_value(request.clone()).unwrap();
    assert!(parsed.request.preserve_secrets.is_empty());
    fixture.invoke("connections.update", request).await.unwrap();
    assert_eq!(fixture.raw(&id)["process"]["env"], json!({}));
    assert_eq!(fixture.host.reload_documents.lock().unwrap().len(), 2);
    assert!(
        fixture.host.authority.lock().unwrap().is_empty(),
        "Disabled records do not start transports when saved"
    );

    let manifest = fixture.plugin.manifest();
    let schema = &manifest
        .tools
        .iter()
        .find(|tool| tool.name == "connections.update")
        .unwrap()
        .contract
        .input_schema;
    let properties = schema["properties"].as_object().unwrap();
    for name in ["id", "config", "expected_revision", "preserve_secrets"] {
        assert!(properties.contains_key(name));
    }
    assert!(!properties.contains_key("request"));
}

#[tokio::test]
async fn reload_queue_failure_reports_saved_state_without_losing_the_persisted_change() {
    let fixture = Fixture::new(
        json!({}),
        json!({}),
        Policy {
            deny_reload: true,
            ..Default::default()
        },
    )
    .await;
    let output = fixture
        .invoke(
            "connections.create",
            json!({"name":"saved","scope":"workspace","config":{
                "transport":"stdio","enabled":false,"process":{"command":"fixture-mcp"}
            }}),
        )
        .await
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
    assert_eq!(
        fixture.raw(&connection_id(McpConnectionScope::Workspace, "saved"))["process"]["command"],
        "fixture-mcp"
    );
    assert_eq!(fixture.host.reload_documents.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn deleting_a_workspace_override_requires_authorization_before_reactivating_global_process() {
    let fixture = Fixture::new(
        document(json!({"shared":stdio()})),
        document(json!({"shared":{
            "transport":"stdio","enabled":false,"process":{"command":"paused-mcp"}
        }})),
        Policy {
            deny_mcp: true,
            ..Default::default()
        },
    )
    .await;
    let id = connection_id(McpConnectionScope::Workspace, "shared");
    let detail = fixture
        .invoke("connections.get", json!({"id":id}))
        .await
        .unwrap()
        .payload
        .unwrap();
    let before = fixture.snapshot();
    let error = fixture
        .invoke(
            "connections.delete",
            json!({"id":id,"expected_revision":detail["revision"]}),
        )
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::UserDeclined);
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(fixture.host.authority.lock().unwrap().len(), 1);
    assert!(fixture.host.reload_documents.lock().unwrap().is_empty());
}
