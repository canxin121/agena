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

fn skill_document(name: &str, instructions: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: A skill for plugin integration tests.\n---\n\n{instructions}\n"
    )
}

async fn invoke_skills(
    plugin: &dyn Plugin,
    workspace: &Path,
    name: &str,
    input: Value,
) -> Result<Value> {
    let context = HostCallbackContext {
        workspace_root: Some(workspace.to_string_lossy().into_owned()),
        ..Default::default()
    };
    run_in_isolated_host_callback_context(context, invoke(plugin, workspace, name, input))
        .await
        .map(|output| {
            output
                .payload
                .expect("skills tools return structured payloads")
        })
}

#[tokio::test]
async fn skills_denied_create_authorizes_the_invoking_workspace_without_creating_files() {
    let initial = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::skills::SkillsPlugin::new();
    initialize(&plugin, initial.path(), host.clone()).await;
    let error = invoke_skills(
        &plugin,
        workspace.path(),
        "create",
        json!({"scope":"workspace", "document":skill_document("blocked-skill", "Do not create this package.")}),
    )
    .await
    .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert!(!initial.path().join(".agena").exists());
    assert!(!workspace.path().join(".agena").exists());
    let requests = host.requests.lock().unwrap();
    assert!(
        requests
            .iter()
            .any(|request| request.kind == PathKind::Write)
    );
    assert!(requests.iter().all(|request| {
        Path::new(&request.path).starts_with(workspace.path().join(".agena/skills"))
    }));
}

#[tokio::test]
async fn skills_source_read_denial_precedes_folder_or_archive_access() {
    let initial = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let host = Arc::new(Host {
        deny_read: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::skills::SkillsPlugin::new();
    initialize(&plugin, initial.path(), host.clone()).await;
    for tool in ["preview_install", "install"] {
        for (source, relative) in [
            (
                json!({"kind":"folder", "path":"missing-package"}),
                "missing-package",
            ),
            (
                json!({"kind":"archive", "path":"missing.zip"}),
                "missing.zip",
            ),
        ] {
            host.requests.lock().unwrap().clear();
            let error = invoke_skills(
                &plugin,
                workspace.path(),
                tool,
                json!({"scope":"workspace", "source":source}),
            )
            .await
            .unwrap_err();
            assert_eq!(
                error.kind,
                PluginErrorKind::PolicyDenied,
                "{tool}: {relative}"
            );
            let requests = host.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].kind, PathKind::Read);
            assert_eq!(
                Path::new(&requests[0].path),
                workspace.path().join(relative)
            );
        }
    }
    assert!(host.network.lock().unwrap().is_empty());
    assert!(!workspace.path().join(".agena").exists());
}

#[tokio::test]
async fn skills_network_decline_is_preserved_before_downloading_any_archive() {
    let workspace = tempfile::tempdir().unwrap();
    let host = Arc::new(Host::default());
    let plugin = crate::plugins::provided::skills::SkillsPlugin::new();
    initialize(&plugin, workspace.path(), host.clone()).await;
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let archive_url = format!("http://{}/skills.zip", listener.local_addr().unwrap());
    for tool in ["preview_install", "install"] {
        for (source, expected_target) in [
            (
                json!({"kind":"archive", "url":archive_url}),
                archive_url.as_str(),
            ),
            (
                json!({"kind":"github", "url":"https://github.com/agena-fixtures/skills", "reference":"main"}),
                "https://codeload.github.com/agena-fixtures/skills/tar.gz/main",
            ),
        ] {
            host.network.lock().unwrap().clear();
            let result = tokio::time::timeout(
                std::time::Duration::from_secs(5),
                invoke_skills(
                    &plugin,
                    workspace.path(),
                    tool,
                    json!({"scope":"workspace", "source":source}),
                ),
            )
            .await
            .expect("denied downloads must return before opening a connection");
            assert_eq!(result.unwrap_err().kind, PluginErrorKind::UserDeclined);
            assert_eq!(*host.network.lock().unwrap(), [expected_target]);
            assert_eq!(
                listener.accept().unwrap_err().kind(),
                std::io::ErrorKind::WouldBlock
            );
        }
    }
    assert!(host.requests.lock().unwrap().is_empty());
    assert!(!workspace.path().join(".agena").exists());
}

#[tokio::test]
async fn skills_install_previews_without_write_but_denied_install_creates_no_package() {
    let workspace = tempfile::tempdir().unwrap();
    let source = workspace.path().join("source-package");
    std::fs::create_dir(&source).unwrap();
    let document = skill_document("install-check", "Keep the complete package together.");
    std::fs::write(source.join("SKILL.md"), &document).unwrap();
    std::fs::write(source.join("reference.txt"), "original resource").unwrap();
    let host = Arc::new(Host {
        deny_write: true,
        ..Default::default()
    });
    let plugin = crate::plugins::provided::skills::SkillsPlugin::new();
    initialize(&plugin, workspace.path(), host.clone()).await;
    let mut request =
        json!({"scope":"workspace", "source":{"kind":"folder", "path":"source-package"}});
    let preview = invoke_skills(
        &plugin,
        workspace.path(),
        "preview_install",
        request.clone(),
    )
    .await
    .unwrap();
    assert_eq!(preview["candidates"][0]["name"], "install-check");
    assert_eq!(preview["candidates"][0]["valid"], true);
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .all(|request| request.kind == PathKind::Read)
    );
    request["expected_source_revision"] = preview["revision"].clone();
    let error = invoke_skills(&plugin, workspace.path(), "install", request)
        .await
        .unwrap_err();
    assert_eq!(error.kind, PluginErrorKind::PolicyDenied);
    assert!(!workspace.path().join(".agena").exists());
    assert_eq!(
        std::fs::read_to_string(source.join("SKILL.md")).unwrap(),
        document
    );
    assert_eq!(
        std::fs::read_to_string(source.join("reference.txt")).unwrap(),
        "original resource"
    );
}

#[tokio::test]
async fn skills_crud_and_resource_contract_work_without_a_global_plugin_host() {
    let initial = tempfile::tempdir().unwrap();
    let workspace = tempfile::tempdir().unwrap();
    let host = Arc::new(Host::default());
    let plugin = crate::plugins::provided::skills::SkillsPlugin::new();
    initialize(&plugin, initial.path(), host.clone()).await;
    let document = skill_document("crud-check", "Initial instructions.");
    let created = invoke_skills(
        &plugin,
        workspace.path(),
        "create",
        json!({"scope":"workspace", "document":document}),
    )
    .await
    .unwrap();
    let id = created["id"].as_str().unwrap();
    let package = workspace.path().join(".agena/skills/crud-check");
    assert_eq!(created["name"], "crud-check");
    assert_eq!(created["editable"], true);
    assert_eq!(created["enabled"], true);
    assert_eq!(
        std::fs::read_to_string(package.join("SKILL.md")).unwrap(),
        document
    );
    assert!(!initial.path().join(".agena").exists());

    let read = invoke_skills(&plugin, workspace.path(), "get", json!({"id":id}))
        .await
        .unwrap();
    assert_eq!(read["document"], document);
    assert_eq!(read["revision"], created["revision"]);
    let updated_document = skill_document("crud-check", "Updated instructions.");
    let updated = invoke_skills(
        &plugin,
        workspace.path(),
        "update",
        json!({"id":id, "document":updated_document, "expected_revision":read["revision"]}),
    )
    .await
    .unwrap();
    assert_ne!(updated["revision"], created["revision"]);
    let stale = invoke_skills(
        &plugin,
        workspace.path(),
        "update",
        json!({"id":id, "document":document, "expected_revision":created["revision"]}),
    )
    .await;
    assert!(stale.is_err());
    assert_eq!(
        std::fs::read_to_string(package.join("SKILL.md")).unwrap(),
        updated_document
    );

    let resource = invoke_skills(
        &plugin,
        workspace.path(),
        "write_resource",
        json!({"id":id, "path":"references/guide.md", "content":"Use this reference."}),
    )
    .await
    .unwrap();
    let loaded = invoke_skills(
        &plugin,
        workspace.path(),
        "read_resource",
        json!({"id":id, "path":"references/guide.md"}),
    )
    .await
    .unwrap();
    assert_eq!(loaded, resource);
    let duplicate = invoke_skills(
        &plugin,
        workspace.path(),
        "write_resource",
        json!({"id":id, "path":"references/guide.md", "content":"Unreviewed replacement"}),
    )
    .await;
    assert!(duplicate.is_err());
    let detail = invoke_skills(
        &plugin,
        workspace.path(),
        "remove_resource",
        json!({"id":id, "path":"references/guide.md", "expected_revision":resource["revision"]}),
    )
    .await
    .unwrap();
    assert!(!package.join("references/guide.md").exists());

    let disabled = invoke_skills(
        &plugin,
        workspace.path(),
        "set_enabled",
        json!({"id":id, "enabled":false, "expected_revision":detail["revision"]}),
    )
    .await
    .unwrap();
    assert_eq!(disabled["enabled"], false);
    let active = invoke_skills(
        &plugin,
        workspace.path(),
        "list",
        json!({"scope":"workspace", "query":"crud-check"}),
    )
    .await
    .unwrap();
    assert!(
        !active["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["id"] == id)
    );
    let all = invoke_skills(
        &plugin,
        workspace.path(),
        "list",
        json!({"scope":"workspace", "query":"crud-check", "include_inactive":true}),
    )
    .await
    .unwrap();
    assert!(
        all["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|skill| skill["id"] == id)
    );
    let removed = invoke_skills(
        &plugin,
        workspace.path(),
        "remove",
        json!({"id":id, "expected_revision":disabled["revision"]}),
    )
    .await
    .unwrap();
    assert_eq!(removed, json!({"id":id, "removed":true}));
    assert!(!package.exists());
    assert!(!initial.path().join(".agena").exists());
    assert!(
        host.requests
            .lock()
            .unwrap()
            .iter()
            .filter(|request| request.kind == PathKind::Write)
            .all(|request| Path::new(&request.path).starts_with(workspace.path()))
    );
}
