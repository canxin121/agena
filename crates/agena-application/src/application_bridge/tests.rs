use super::*;
use agena_plugin_host::sdk::{
    CommandDefinition, CommandInvokeInput, CommandTarget, HookSubscription, PluginManifest,
    SettingsContract,
};
use serde_json::{Value, json};
use std::time::Duration;

struct Fixture {
    application: Application,
    runtime: agena_runtime::RuntimeBootstrapResult,
    directory: tempfile::TempDir,
}

impl Fixture {
    async fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        std::fs::create_dir(root.join("workspace")).unwrap();
        std::fs::write(root.join("fixture.py"), include_str!("fixture.py")).unwrap();
        let mut manifest = PluginManifest::new("test", "application-api", "1.0.0");
        manifest.hooks |= HookSubscription::APPLICATION_READY;
        manifest.commands = vec![CommandDefinition {
            id: "operation".into(),
            title: "Host operation".into(),
            group: "Test".into(),
            category: None,
            slash: None,
            aliases: vec![],
            applies_to: Default::default(),
            docs: Default::default(),
            input: SettingsContract::empty_object(
                "Operation",
                "Execute the isolated fixture operation",
            ),
            target: CommandTarget::Method {
                handler: "operation".into(),
            },
        }];
        std::fs::write(root.join("manifest.json"), serde_json::to_vec(&json!({
            "manifest": manifest, "protocol_version": agena_plugin_host::sdk::rpc::PROTOCOL_VERSION
        })).unwrap()).unwrap();
        let mut plugins = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .map(|id| (id, json!({"enabled": false, "package": {"kind": "static"}})))
            .collect::<serde_json::Map<String, Value>>();
        plugins.insert("test.application-api".into(), json!({"enabled": true, "package": {
            "kind": "stdio", "command": "python3", "args": ["-u", root.join("fixture.py"), root],
            "cwd": root
        }}));
        std::fs::write(
            root.join("agena.json"),
            serde_json::to_vec(&json!({"plugins": {"list": plugins}})).unwrap(),
        )
        .unwrap();
        let (application, runtime) =
            Application::bootstrap(agena_runtime::RuntimeBootstrapRequest {
                workspace_root: Some(root.join("workspace")),
                config_path: Some(root.join("agena.json")),
                database_path: Some(root.join("test.sqlite")),
                automatic_maintenance: Some(false),
                ..Default::default()
            })
            .await
            .unwrap();
        let ready: Value =
            serde_json::from_slice(&std::fs::read(root.join("ready.json")).unwrap()).unwrap();
        assert_eq!(ready["result"]["response"]["result"], "health");
        assert_eq!(ready["result"]["response"]["data"]["status"], "ok");
        Self {
            application,
            runtime,
            directory,
        }
    }

    async fn op(&self, kind: &str, request: Value) -> Result<Value, Value> {
        // Decode through the public contract before sending it over stdio.
        let operation: agena_plugin_host::sdk::host_api::SessionOp =
            serde_json::from_value(json!({"op":kind,"request":request})).unwrap();
        std::fs::write(
            self.directory.path().join("operation.json"),
            serde_json::to_vec(&operation).unwrap(),
        )
        .unwrap();
        let output = tokio::time::timeout(
            Duration::from_secs(8),
            self.application.plugin_runtime().invoke_plugin_command(
                "test.application-api",
                CommandInvokeInput {
                    command_id: "operation".into(),
                    input: json!({}),
                    session_id: None,
                    call_id: None,
                    workspace_root: None,
                    slash: None,
                    raw: String::new(),
                },
            ),
        )
        .await
        .expect("host callback must finish before a reload retires its caller")
        .unwrap()
        .output
        .unwrap();
        if let Some(error) = output.get("error") {
            return Err(error.clone());
        }
        assert_eq!(output["result"]["op"], kind);
        Ok(output["result"]["response"].clone())
    }

    async fn query(&self, request: Value) -> Value {
        let query = serde_json::from_value(request.clone()).unwrap();
        let actual = self.op("query", request).await.unwrap();
        let expected = serde_json::to_value(
            dispatch::dispatch_query(&self.application, query)
                .await
                .unwrap(),
        )
        .unwrap();
        assert_eq!(
            actual, expected,
            "Host and in-process query must share the complete envelope"
        );
        actual
    }

    async fn command(&self, request: Value) -> Value {
        self.op("command", request).await.unwrap()
    }

    async fn close(self) {
        self.runtime.shutdown_and_wait().await.unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

#[tokio::test]
async fn stdio_host_uses_shared_application_without_an_http_listener() {
    let fixture = Fixture::new().await;
    let path = fixture.directory.path().join("project");
    std::fs::create_dir(&path).unwrap();
    let workspace = fixture
        .command(json!({"method":"create_workspace","params":{"path":path}}))
        .await;
    let workspace_id = workspace["data"]["id"].as_i64().unwrap();
    assert_eq!(
        fixture
            .query(json!({"method":"get_workspace","params":{"workspace_id":workspace_id}}))
            .await,
        workspace
    );
    let prompts = json!({"mode":"replace","order":[],"disabled":[],"inline":[]});
    let created = fixture
        .command(json!({"method":"create_session","params":{
            "workspace_id":workspace_id,"title":"Shared session","prompts":prompts
        }}))
        .await;
    let id = created["data"]["id"].as_i64().unwrap();
    assert_eq!(created["data"]["workspace_id"], workspace_id);
    let state = fixture
        .query(json!({"method":"get_session_state","params":{"session_id":id}}))
        .await;
    assert_eq!(state["data"]["execution"]["prompts"], prompts);
    let counted = fixture
        .query(json!({"method":"list_sessions","params":{
            "workspace_id":workspace_id,"count_only":true,"include_total":true
        }}))
        .await;
    assert_eq!(counted["data"]["items"], json!([]));
    assert_eq!(counted["data"]["total"], 1);
    let paged = fixture
        .query(json!({"method":"list_sessions","params":{"workspace_id":workspace_id,"limit":1}}))
        .await;
    assert_eq!(paged["data"]["items"][0]["id"], id);
    let bad_delete = fixture
        .op(
            "command",
            json!({"method":"delete_session","params":{"session_id":id,"expected_version":-1}}),
        )
        .await;
    assert!(bad_delete.is_err());
    let bad_fork = fixture
        .op(
            "command",
            json!({"method":"fork_session","params":{"session_id":id,"expected_version":-1}}),
        )
        .await;
    assert!(bad_fork.is_err());
    fixture
        .query(json!({"method":"get_session","params":{"session_id":id}}))
        .await;
    let changed = fixture
        .command(
            json!({"method":"extended_session","params":{"method":"update_metadata","params":{
                "session_id":id,"title":"Renamed from Host","favorite":true,"pinned":true
            }}}),
        )
        .await;
    assert!(changed["data"].is_object());
    let renamed = fixture
        .query(json!({"method":"get_session","params":{"session_id":id}}))
        .await;
    assert_eq!(renamed["data"]["title"], "Renamed from Host");
    let bot = fixture
        .command(
            json!({"method":"business","params":{"method":"create_bot","params":{
                "request_key":"shared-backend-bot","name":"Shared Bot"
            }}}),
        )
        .await;
    let bot_id = bot["data"]["id"].clone();
    assert!(bot_id.is_string());
    let bots = fixture
        .query(json!({"method":"business","params":{"method":"list_bots","params":{}}}))
        .await;
    assert!(serde_json::to_string(&bots).unwrap().contains("Shared Bot"));
    fixture
        .query(
            json!({"method":"business","params":{"method":"get_bot","params":{"bot_id":bot_id}}}),
        )
        .await;
    let rule = fixture
        .command(json!({"method":"upsert_permission_rule","params":{
            "subject_kind":"tool","tool_name":"fixture.read","scope":"global","mode":"allow"
        }}))
        .await;
    let rule_id = rule["data"]["id"].as_i64().unwrap();
    let rule_query = json!({"method":"get_permission_rule","params":{"rule_id":rule_id}});
    fixture.query(rule_query.clone()).await;
    fixture.command(json!({"method":"replace_permission_rule","params":{
        "rule_id":rule_id,"subject_kind":"tool","tool_name":"fixture.write","scope":"global","mode":"deny"
    }})).await;
    assert_eq!(
        fixture.query(rule_query.clone()).await["data"]["mode"],
        "deny"
    );
    fixture.command(json!({"method":"revoke_permission_rule","params":{"rule_id":rule_id,"reason":"fixture complete"}})).await;
    assert!(fixture.query(rule_query.clone()).await["data"]["revoked_at"].is_string());
    fixture
        .command(json!({"method":"delete_permission_rule","params":{"rule_id":rule_id}}))
        .await;
    assert!(fixture.op("query", rule_query).await.is_err());
    fixture
        .command(json!({"method":"clear_finished_activities"}))
        .await;
    fixture.command(json!({"method":"delete_session","params":{"session_id":id,"expected_version":renamed["data"]["version"]}})).await;
    assert!(
        fixture
            .op(
                "query",
                json!({"method":"get_session","params":{"session_id":id}})
            )
            .await
            .is_err()
    );
    // Deleted conversations keep their workspace reference. Remove an empty
    // workspace to exercise deletion without violating retained history.
    let vacant = fixture.directory.path().join("vacant");
    std::fs::create_dir(&vacant).unwrap();
    let empty = fixture
        .command(
            json!({"method":"resolve_workspace","params":{"path":vacant,"create_if_missing":true}}),
        )
        .await;
    fixture
        .command(json!({"method":"delete_workspace","params":{"workspace_id":empty["data"]["id"]}}))
        .await;
    fixture.close().await;
}

#[tokio::test]
async fn plugin_settings_reload_waits_for_callback_and_keeps_backend_attached() {
    let fixture = Fixture::new().await;
    let backend = fixture.application.session_api_backend.backend().unwrap();
    let before = fixture
        .application
        .runtime_snapshot_summary()
        .await
        .generation;
    let get = json!({"method":"management","params":{"domain":"settings","request":{
        "method":"get","params":{"path":"runtime.providers.client_versions.auto_update","source":"file"}
    }}});
    let initial = fixture.query(get.clone()).await;
    let revision = initial["data"]["revision"].clone();
    let set = |value: bool, revision: Value, reload: bool| {
        json!({"method":"management","params":{
            "domain":"settings","request":{"method":"set","params":{
                "path":"runtime.providers.client_versions.auto_update","value":value,
                "expected_revision":revision,"reload":reload,"validate":true,"dry_run":false
            }}
        }})
    };
    fixture.command(set(false, revision.clone(), false)).await;
    assert!(
        fixture
            .op("command", set(true, revision, false))
            .await
            .is_err()
    );
    let saved = fixture.query(get.clone()).await;
    assert_eq!(saved["data"]["value"], false);
    let result = fixture
        .command(set(true, saved["data"]["revision"].clone(), true))
        .await;
    let tasks = result["deferred_reload_task_ids"]
        .as_array()
        .expect("a callback reload reports its real deferred task");
    assert_eq!(tasks.len(), 1);
    tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            if fixture
                .application
                .runtime_snapshot_summary()
                .await
                .generation
                > before
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the reload starts after the plugin callback completes");
    assert!(Arc::ptr_eq(
        &backend,
        &fixture.application.session_api_backend.backend().unwrap()
    ));
    let saved = fixture.query(get).await;
    assert_eq!(saved["data"]["value"], true);
    fixture
        .query(json!({"method":"business","params":{"method":"list_bots","params":{}}}))
        .await;
    fixture.close().await;
}
