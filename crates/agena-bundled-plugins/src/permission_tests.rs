use agena_domain::PathKind;
use agena_plugin_sdk::hooks::{EventEnvelope, EventFilter};
use agena_plugin_sdk::host_api::*;
use agena_plugin_sdk::{
    InitContext, Plugin, PluginError, PluginErrorKind, Result, ToolInvokeInput, ToolInvokeOutput,
};
use serde_json::{Value, json};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Host {
    deny_suffix: Option<String>,
    deny_read: bool,
    deny_write: bool,
    requests: Mutex<Vec<PathPermissionQuery>>,
    network: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl HostClient for Host {
    async fn log(&self, _: LogLevel, _: String, _: Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Ok(())
    }
    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Ok(EventSubscription { id: "test".into() })
    }
    async fn read_config(&self, _: Option<String>) -> Result<Value> {
        Ok(json!({}))
    }
    async fn invoke_tool(&self, _: String, _: Value) -> Result<ToolInvokeOutput> {
        unreachable!()
    }
    async fn check_path_permission(&self, _: PathPermissionQuery) -> Result<PermissionQuery> {
        Ok(PermissionQuery::Ask {
            reason: "review required".into(),
        })
    }
    async fn require_path_permission(&self, request: PathPermissionQuery) -> Result<()> {
        let denied = self
            .deny_suffix
            .as_ref()
            .is_some_and(|suffix| request.path.ends_with(suffix))
            || match request.kind {
                PathKind::Read => self.deny_read,
                PathKind::Write => self.deny_write,
            };
        self.requests.lock().unwrap().push(request);
        if denied {
            Err(PluginError::from_kind(
                PluginErrorKind::PolicyDenied,
                "test path denied",
            ))
        } else {
            Ok(())
        }
    }
    async fn require_network_permission(&self, target: String) -> Result<()> {
        self.network.lock().unwrap().push(target);
        Err(PluginError::from_kind(
            PluginErrorKind::UserDeclined,
            "test network declined",
        ))
    }
}

#[tokio::test]
async fn structural_directory_search_checks_children_and_preserves_denial() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("secret.rs"), "fn private() {}\n").unwrap();
    let host = Arc::new(Host {
        deny_suffix: Some("secret.rs".into()),
        ..Default::default()
    });
    let plugin = crate::plugins::provided::code::new_plugin();
    initialize(&plugin, root.path(), host.clone()).await;
    let error = invoke(
        &plugin,
        root.path(),
        "search_ast",
        json!({"path":".", "language":"rust", "pattern":"fn $NAME() { $$$ }"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.path.ends_with("secret.rs"))
    );
}

#[tokio::test]
async fn memory_index_permission_is_checked_before_creating_a_record() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_suffix: Some("MEMORY.md".into()),
        ..Default::default()
    });
    let plugin = crate::memory::MemoryPlugin::new();
    initialize(&plugin, root.path(), host).await;
    let error = invoke(
        &plugin,
        root.path(),
        "write",
        json!({"name":"permission-probe", "content":"private record"}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    let store = agena_storage::MemoryStore::for_workspace(root.path());
    assert!(!store.dir().join("permission-probe.md").exists());
}

#[tokio::test]
async fn memory_authorization_uses_the_invoking_workspace_instead_of_initial_state_root() {
    let initial = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::memory::MemoryPlugin::new();
    initialize(&plugin, initial.path(), host.clone()).await;
    let context = HostCallbackContext {
        workspace_root: Some(workspace.path().to_string_lossy().into_owned()),
        ..Default::default()
    };
    let error = run_in_isolated_host_callback_context(
        context,
        invoke(
            &plugin,
            workspace.path(),
            "write",
            json!({"name":"workspace-check", "content":"private"}),
        ),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    let expected = agena_storage::MemoryStore::for_workspace(workspace.path())
        .dir()
        .join("workspace-check.md");
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| Path::new(&request.path) == expected)
    );
}

async fn initialize(plugin: &dyn Plugin, root: &Path, host: Arc<Host>) {
    plugin
        .init(
            InitContext {
                agena_version: "test".into(),
                workspace_root: root.to_owned(),
                plugin_id: agena_plugin_sdk::PluginKey::new(
                    &plugin.manifest().namespace,
                    &plugin.manifest().name,
                )
                .unwrap(),
                host_callback_url: None,
                host_callback_token: None,
                settings: json!({}),
                protocol_version: agena_plugin_sdk::rpc::PROTOCOL_VERSION,
            },
            host,
        )
        .await
        .unwrap();
}

async fn invoke(
    plugin: &dyn Plugin,
    root: &Path,
    name: &str,
    input: Value,
) -> Result<ToolInvokeOutput> {
    plugin
        .tool_invoke(ToolInvokeInput {
            tool_name: name.into(),
            session_id: 1,
            call_id: 2,
            workspace_root: root.to_string_lossy().into_owned(),
            input,
        })
        .await
}

#[tokio::test]
async fn filesystem_mutation_requires_write_and_does_not_touch_denied_files() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::fs::new_plugin();
    initialize(&plugin, root.path(), host.clone()).await;
    let result = invoke(
        &plugin,
        root.path(),
        "write",
        json!({"path":"blocked/child.txt","content":"forbidden","create_parents":true}),
    )
    .await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::PolicyDenied);
    assert!(!root.path().join("blocked").exists());
    assert_eq!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .map(|r| r.kind)
            .collect::<Vec<_>>(),
        [PathKind::Read, PathKind::Write]
    );
}

#[tokio::test]
async fn filesystem_reads_authorize_before_metadata_or_conversion() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_read: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::fs::new_plugin();
    initialize(&plugin, root.path(), host.clone()).await;
    for (name, input) in [
        ("stat", json!({"path":"missing.txt"})),
        ("read_many", json!({"paths":["missing.txt","other.txt"]})),
        ("document", json!({"path":"missing.pdf"})),
    ] {
        assert_eq!(
            invoke(&plugin, root.path(), name, input)
                .await
                .unwrap_err()
                .kind,
            PluginErrorKind::PolicyDenied,
            "{name}"
        );
    }
    assert_eq!(host.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn ast_preview_only_needs_read_but_application_needs_write() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("source.rs");
    std::fs::write(&path, "fn main() { let x = 1; }\n").unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::code::new_plugin();
    initialize(&plugin, root.path(), host.clone()).await;
    let input = json!({"path":"source.rs","pattern":"let $A = $B;","replacement":"let $A = 2;"});
    let preview = invoke(&plugin, root.path(), "rewrite_ast", input.clone())
        .await
        .unwrap();
    let hash = preview.payload.as_ref().unwrap()["before_sha256"].clone();
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .all(|r| r.kind == PathKind::Read)
    );
    let mut apply = input;
    apply["apply"] = json!(true);
    apply["expected_sha256"] = hash;
    assert_eq!(
        invoke(&plugin, root.path(), "rewrite_ast", apply)
            .await
            .unwrap_err()
            .kind,
        PluginErrorKind::PolicyDenied
    );
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        "fn main() { let x = 1; }\n"
    );
}

#[tokio::test]
async fn notebook_denial_precedes_file_access() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::notebook::NotebookPlugin::new();
    initialize(&plugin, root.path(), host).await;
    let result = invoke(
        &plugin,
        root.path(),
        "edit_cell",
        json!({"path":"missing.ipynb","action":"delete","cell_index":0,"expected_sha256":"abc"}),
    )
    .await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::PolicyDenied);
}

#[tokio::test]
async fn provider_requests_preserve_declines_without_sending_or_exposing_query_secrets() {
    let host = Arc::new(Host::default());
    let client: Arc<dyn HostClient> = host.clone();
    let result = crate::plugins::provided::official_service::post_json(
        &client,
        "http://127.0.0.1:1/request?key=secret",
        &Default::default(),
        json!({}),
        &BuiltinProviderConnection {
            provider_id: "test".into(),
            base_url: "https://provider.example".into(),
            messages_url: None,
            realtime_ws_url: None,
            headers: Default::default(),
            model: None,
            timeout_secs: 1,
            connect_timeout_secs: 1,
            receipt_key: String::new(),
        },
        "test",
        "test",
    )
    .await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::UserDeclined);
    assert_eq!(
        *host.network.lock().unwrap(),
        ["http://127.0.0.1:1/request"]
    );
}

#[tokio::test]
async fn memory_mutation_honors_path_denial_before_creating_a_record() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::memory::MemoryPlugin::new();
    initialize(&plugin, root.path(), host.clone()).await;
    let result = invoke(&plugin, root.path(), "write", json!({"name":"permission-regression", "content":"must not persist", "description":"test", "memory_type":"project"})).await;
    assert_eq!(result.unwrap_err().kind, PluginErrorKind::PolicyDenied);
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .any(|request| request.kind == PathKind::Write)
    );
}
