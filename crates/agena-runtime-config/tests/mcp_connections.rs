use std::{fs, path::Path};

use agena_domain::mcp_connections::{
    McpBridgeEnabledRequest, McpConnectionCreateRequest, McpConnectionEnabledRequest,
    McpConnectionScope, McpConnectionUpdateRequest, McpImportPreviewRequest, McpImportRequest,
};
use agena_runtime_config::{
    RawConfig,
    config::raw::RawProjectMergeKeys,
    mcp_config_from_plugins,
    mcp_connections::{McpConnectionStore, McpManagementErrorKind, REDACTED, connection_id},
};
use serde_json::{Value, json};
use tempfile::TempDir;

struct Fixture {
    _directory: TempDir,
    store: McpConnectionStore,
}

impl Fixture {
    fn new(global: Value, workspace: Value) -> Self {
        let directory = tempfile::tempdir().unwrap();
        let global_path = directory.path().join("global/config.json");
        let workspace_root = directory.path().join("workspace");
        let workspace_path = workspace_root.join("config.json");
        write_document(&global_path, &global);
        write_document(&workspace_path, &workspace);
        let store = McpConnectionStore::new(global_path, workspace_path, workspace_root);
        Self {
            _directory: directory,
            store,
        }
    }

    fn empty() -> Self {
        Self::new(json!({}), json!({}))
    }

    fn snapshot(&self) -> (Vec<u8>, Vec<u8>) {
        (
            fs::read(&self.store.global_path).unwrap(),
            fs::read(&self.store.workspace_path).unwrap(),
        )
    }

    fn create(&self, name: &str, scope: McpConnectionScope, config: Value) -> String {
        self.store
            .create(McpConnectionCreateRequest {
                name: name.into(),
                scope,
                config,
            })
            .unwrap()
    }
}

fn write_document(path: &Path, document: &Value) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_vec_pretty(document).unwrap()).unwrap();
}

fn read_document(path: &Path) -> Value {
    serde_json::from_slice(&fs::read(path).unwrap()).unwrap()
}

fn stdio(command: &str) -> Value {
    json!({"transport":"stdio", "process":{"command":command}})
}

fn http() -> Value {
    json!({"transport":"http", "endpoint":{"url":"https://example.test/mcp"}})
}

fn document(servers: Value) -> Value {
    json!({"plugins":{"list":{"agena.mcp":{
        "enabled":true,
        "package":{"kind":"static"},
        "settings":{"servers":servers}
    }}}})
}

fn merge_project(global: Value, workspace: Value) -> agena_runtime_config::ResolvedConfig {
    let mut raw = serde_json::from_value::<RawConfig>(global).unwrap();
    raw.merge_project_from_with_keys(
        serde_json::from_value(workspace).unwrap(),
        RawProjectMergeKeys::default(),
    );
    raw.resolve().unwrap()
}

#[test]
fn workspace_overrides_complete_connections_even_when_disabled_and_retains_other_global_servers() {
    let mut global = document(json!({
        "shared": {
            "transport":"http",
            "endpoint":{"url":"https://example.test/mcp"},
            "auth":{"kind":"bearer", "token":"global-private-token"},
            "tools":{"include":["global.*"]}
        },
        "global-only": stdio("global-mcp")
    }));
    global["plugins"]["list"]["agena.mcp"]["settings"]["runtime"] = json!({
        "token_store":{"enabled":false},
        "reconnect":{"initial_delay_ms":1250, "max_delay_ms":9000}
    });
    global["plugins"]["list"]["example.custom"] = json!({
        "package":{"kind":"static"}, "settings":{"global_only":true}
    });
    let mut workspace = document(json!({
        "shared":{"transport":"stdio", "process":{"command":"workspace-mcp"}, "enabled":false},
        "workspace-only":stdio("other-mcp")
    }));
    workspace["plugins"]["list"]["agena.mcp"]["settings"]["runtime"] =
        json!({"reconnect":{"enabled":false}});
    workspace["plugins"]["list"]["example.custom"] = json!({
        "package":{"kind":"static"}, "settings":{"workspace_only":true}
    });

    let resolved = merge_project(global.clone(), workspace.clone());
    let mcp = mcp_config_from_plugins(&resolved.plugins).unwrap();
    assert_eq!(mcp.servers.len(), 3);
    let shared = serde_json::to_value(&mcp.servers["shared"]).unwrap();
    assert_eq!(shared["transport"], "stdio");
    assert_eq!(shared["process"]["command"], "workspace-mcp");
    assert_eq!(shared["enabled"], false);
    assert!(shared.get("endpoint").is_none());
    assert!(shared.get("auth").is_none());
    assert_eq!(shared["tools"]["include"], json!([]));
    assert!(!mcp.runtime.token_store.enabled);
    assert!(!mcp.runtime.reconnect.enabled);
    assert_eq!(mcp.runtime.reconnect.initial_delay_ms, 1250);
    assert_eq!(mcp.runtime.reconnect.max_delay_ms, 9000);
    assert_eq!(
        resolved.plugins.list["example.custom"].settings(),
        &json!({"workspace_only":true})
    );

    let fixture = Fixture::new(global, workspace);
    let global_id = connection_id(McpConnectionScope::Global, "shared");
    let workspace_id = connection_id(McpConnectionScope::Workspace, "shared");
    let global = fixture.store.get(&global_id).unwrap();
    let workspace = fixture.store.get(&workspace_id).unwrap();
    assert!(!global.summary.effective);
    assert_eq!(global.summary.state, "shadowed");
    assert!(workspace.summary.effective);
    assert_eq!(workspace.summary.state, "disabled");
    assert_eq!(
        fixture.store.resolved(&global_id).unwrap_err().kind,
        McpManagementErrorKind::Invalid
    );
    assert!(!fixture.store.resolved(&workspace_id).unwrap().1.enabled());
    let catalog = fixture.store.list().unwrap();
    assert_eq!(catalog.connections.len(), 4);
    assert_eq!(
        catalog.workspace_root,
        fixture.store.workspace_root.to_string_lossy().into_owned()
    );
}

#[test]
fn scoped_crud_preserves_sibling_settings_and_removing_override_restores_global_connection() {
    let mut global = document(json!({"shared":stdio("global-mcp")}));
    global["session"] = json!({"additional_system_prompt":"keep global prompt"});
    let workspace = json!({
        "session":{"additional_system_prompt":"keep project prompt"},
        "plugins":{"list":{"example.custom":{
            "package":{"kind":"static"}, "settings":{"keep":[1,2,3]}
        }}}
    });
    let fixture = Fixture::new(global, workspace);
    let before = fixture.snapshot();
    let id = fixture.create(
        "shared",
        McpConnectionScope::Workspace,
        stdio("workspace-mcp"),
    );
    let saved = read_document(&fixture.store.workspace_path);
    assert_eq!(fs::read(&fixture.store.global_path).unwrap(), before.0);
    assert_eq!(
        saved["session"]["additional_system_prompt"],
        "keep project prompt"
    );
    assert_eq!(
        saved["plugins"]["list"]["example.custom"]["settings"],
        json!({"keep":[1,2,3]})
    );
    let detail = fixture.store.get(&id).unwrap();
    fixture.store.remove(&id, &detail.summary.revision).unwrap();
    assert_eq!(
        fixture.store.get(&id).unwrap_err().kind,
        McpManagementErrorKind::NotFound
    );
    let global = fixture
        .store
        .get(&connection_id(McpConnectionScope::Global, "shared"))
        .unwrap();
    assert!(global.summary.effective);
    assert_eq!(global.summary.state, "disconnected");
    assert_eq!(fs::read(&fixture.store.global_path).unwrap(), before.0);
    let saved = read_document(&fixture.store.workspace_path);
    assert_eq!(
        saved["session"]["additional_system_prompt"],
        "keep project prompt"
    );
    assert_eq!(
        saved["plugins"]["list"]["example.custom"]["settings"],
        json!({"keep":[1,2,3]})
    );
}

#[test]
fn stale_record_revisions_reject_update_enable_and_delete_without_changing_files() {
    let fixture = Fixture::empty();
    let id = fixture.create(
        "primary",
        McpConnectionScope::Workspace,
        stdio("before-mcp"),
    );
    let original = fixture.store.get(&id).unwrap();
    fixture
        .store
        .update(
            &id,
            McpConnectionUpdateRequest {
                config: stdio("after-mcp"),
                preserve_secrets: vec![],
                expected_revision: original.summary.revision.clone(),
            },
        )
        .unwrap();
    let before = fixture.snapshot();
    for revision in [original.summary.revision, String::new()] {
        let error = fixture
            .store
            .update(
                &id,
                McpConnectionUpdateRequest {
                    config: stdio("stale-mcp"),
                    preserve_secrets: vec![],
                    expected_revision: revision.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Conflict);
        let error = fixture
            .store
            .set_enabled(
                &id,
                McpConnectionEnabledRequest {
                    enabled: false,
                    expected_revision: revision.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Conflict);
        let error = fixture.store.remove(&id, &revision).unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Conflict);
        assert_eq!(fixture.snapshot(), before);
    }

    // A separate connection does not invalidate the revision of this record.
    let current = fixture.store.get(&id).unwrap();
    fixture.create(
        "sibling",
        McpConnectionScope::Workspace,
        stdio("sibling-mcp"),
    );
    fixture
        .store
        .set_enabled(
            &id,
            McpConnectionEnabledRequest {
                enabled: false,
                expected_revision: current.summary.revision,
            },
        )
        .unwrap();
    assert!(!fixture.store.get(&id).unwrap().summary.enabled);
    assert!(
        fixture
            .store
            .get(&connection_id(McpConnectionScope::Workspace, "sibling"))
            .unwrap()
            .summary
            .enabled
    );
}

#[test]
fn bridge_toggle_keeps_server_flags_and_does_not_copy_global_servers_into_workspace() {
    let fixture = Fixture::new(
        document(json!({
            "active":stdio("active-mcp"),
            "paused":{"transport":"stdio", "process":{"command":"paused-mcp"}, "enabled":false}
        })),
        json!({}),
    );
    let before_global = fixture.snapshot().0;
    fixture
        .store
        .set_bridge_enabled(McpBridgeEnabledRequest {
            enabled: false,
            scope: McpConnectionScope::Workspace,
        })
        .unwrap();
    let catalog = fixture.store.list().unwrap();
    assert!(!catalog.bridge_enabled);
    assert_eq!(
        fixture
            .store
            .get(&connection_id(McpConnectionScope::Global, "active"))
            .unwrap()
            .summary
            .state,
        "bridge_disabled"
    );
    assert_eq!(
        fixture
            .store
            .get(&connection_id(McpConnectionScope::Global, "paused"))
            .unwrap()
            .summary
            .state,
        "disabled"
    );
    fixture
        .store
        .set_bridge_enabled(McpBridgeEnabledRequest {
            enabled: true,
            scope: McpConnectionScope::Workspace,
        })
        .unwrap();
    assert!(fixture.store.list().unwrap().bridge_enabled);
    assert!(
        fixture
            .store
            .get(&connection_id(McpConnectionScope::Global, "active"))
            .unwrap()
            .summary
            .enabled
    );
    assert!(
        !fixture
            .store
            .get(&connection_id(McpConnectionScope::Global, "paused"))
            .unwrap()
            .summary
            .enabled
    );
    let saved = read_document(&fixture.store.workspace_path);
    assert_eq!(
        saved["plugins"]["list"]["agena.mcp"]["settings"]["servers"],
        json!({})
    );
    assert_eq!(fs::read(&fixture.store.global_path).unwrap(), before_global);
    let resolved = merge_project(read_document(&fixture.store.global_path), saved);
    assert_eq!(
        mcp_config_from_plugins(&resolved.plugins)
            .unwrap()
            .servers
            .len(),
        2
    );
}

#[test]
fn environment_secrets_can_be_kept_replaced_and_removed_with_escaped_json_pointers() {
    let fixture = Fixture::empty();
    let id = fixture.create(
        "local",
        McpConnectionScope::Workspace,
        json!({
            "transport":"stdio", "process":{"command":"fixture-mcp", "env":{
                "KEEP~/KEY":"keep-secret", "CHANGE":"old-secret", "REMOVE":"remove-secret"
            }}
        }),
    );
    let mut detail = fixture.store.get(&id).unwrap();
    let encoded = serde_json::to_string(&detail).unwrap();
    for secret in ["keep-secret", "old-secret", "remove-secret"] {
        assert!(!encoded.contains(secret));
    }
    assert!(
        detail
            .secret_paths
            .contains(&"/process/env/KEEP~0~1KEY".into())
    );
    detail.config["process"]["env"]["CHANGE"] = json!("replacement-secret");
    detail.config["process"]["env"]
        .as_object_mut()
        .unwrap()
        .remove("REMOVE");
    fixture
        .store
        .update(
            &id,
            McpConnectionUpdateRequest {
                config: detail.config,
                preserve_secrets: vec!["/process/env/KEEP~0~1KEY".into()],
                expected_revision: detail.summary.revision,
            },
        )
        .unwrap();
    let (_, _, raw) = fixture.store.raw(&id).unwrap();
    assert_eq!(raw["process"]["env"]["KEEP~/KEY"], "keep-secret");
    assert_eq!(raw["process"]["env"]["CHANGE"], "replacement-secret");
    assert!(raw["process"]["env"].get("REMOVE").is_none());
    assert_eq!(
        fixture.store.get(&id).unwrap().config["process"]["env"]["CHANGE"],
        REDACTED
    );
}

#[test]
fn http_credentials_and_secret_urls_are_redacted_preserved_and_explicitly_removable() {
    let fixture = Fixture::empty();
    let id = fixture.create("remote", McpConnectionScope::Global, json!({
        "transport":"http",
        "endpoint":{"url":"https://example.test/mcp?key=url-secret", "headers":{"X-Api-Key":"header-secret"}},
        "auth":{"kind":"bearer", "token":"bearer-secret"}
    }));
    let mut detail = fixture.store.get(&id).unwrap();
    let encoded = serde_json::to_string(&detail).unwrap();
    for secret in ["url-secret", "header-secret", "bearer-secret"] {
        assert!(!encoded.contains(secret));
    }
    assert_eq!(detail.config["endpoint"]["url"], REDACTED);
    assert_eq!(detail.config["auth"]["token"], REDACTED);
    detail.config["timeouts"]["request_ms"] = json!(4000);
    fixture
        .store
        .update(
            &id,
            McpConnectionUpdateRequest {
                config: detail.config,
                preserve_secrets: detail.secret_paths,
                expected_revision: detail.summary.revision,
            },
        )
        .unwrap();
    let (_, _, raw) = fixture.store.raw(&id).unwrap();
    assert_eq!(
        raw["endpoint"]["url"],
        "https://example.test/mcp?key=url-secret"
    );
    assert_eq!(raw["endpoint"]["headers"]["X-Api-Key"], "header-secret");
    assert_eq!(raw["auth"]["token"], "bearer-secret");
    assert_eq!(raw["timeouts"]["request_ms"], 4000);

    let detail = fixture.store.get(&id).unwrap();
    fixture
        .store
        .update(
            &id,
            McpConnectionUpdateRequest {
                config: http(),
                preserve_secrets: vec![],
                expected_revision: detail.summary.revision,
            },
        )
        .unwrap();
    let (_, _, raw) = fixture.store.raw(&id).unwrap();
    assert!(raw.get("auth").is_none());
    assert_eq!(raw["endpoint"]["headers"], json!({}));
    assert_eq!(raw["endpoint"]["url"], "https://example.test/mcp");
    assert!(fixture.store.get(&id).unwrap().secret_paths.is_empty());
}

#[test]
fn invalid_secret_preservation_never_saves_redaction_sentinels_or_changes_files() {
    let fixture = Fixture::empty();
    let id = fixture.create(
        "remote",
        McpConnectionScope::Workspace,
        json!({
            "transport":"http", "endpoint":{"url":"https://example.test/mcp"},
            "auth":{"kind":"bearer", "token":"keep-private-token"}
        }),
    );
    let detail = fixture.store.get(&id).unwrap();
    let before = fixture.snapshot();
    for (config, preserve_secrets) in [
        (detail.config.clone(), vec![]),
        (detail.config.clone(), vec!["/endpoint/url".to_owned()]),
        (http(), detail.secret_paths.clone()),
    ] {
        let error = fixture
            .store
            .update(
                &id,
                McpConnectionUpdateRequest {
                    config,
                    preserve_secrets,
                    expected_revision: detail.summary.revision.clone(),
                },
            )
            .unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Invalid);
        assert!(!error.message.contains("keep-private-token"));
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn import_preview_is_read_only_and_redacted_but_install_preserves_exact_input_values() {
    let fixture = Fixture::empty();
    let source = json!({"mcpServers":{
        "local":{
            "command":"fixture-mcp", "args":["a b", "", "$(literal)"],
            "cwd":"/project with spaces", "env":{"API_KEY":"import-env-secret"}, "disabled":true
        },
        "remote":{
            "type":"streamable-http", "url":"https://example.test/mcp?key=import-url-secret",
            "headers":{"Authorization":"Bearer import-header-secret"}
        }
    }});
    let before = fixture.snapshot();
    let preview = fixture
        .store
        .preview_import(&McpImportPreviewRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
        })
        .unwrap();
    assert_eq!(fixture.snapshot(), before);
    assert_eq!(preview.candidates.len(), 2);
    assert!(preview.errors.is_empty());
    assert!(
        preview
            .candidates
            .iter()
            .all(|item| item.valid && !item.conflict)
    );
    let serialized = serde_json::to_string(&preview).unwrap();
    for secret in [
        "import-env-secret",
        "import-url-secret",
        "import-header-secret",
    ] {
        assert!(!serialized.contains(secret));
    }
    let installed = fixture
        .store
        .import(McpImportRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
            selected: vec!["local".into(), "remote".into()],
            replace: false,
            expected_revision: preview.revision,
        })
        .unwrap();
    assert_eq!(installed.len(), 2);
    let (_, _, local) = fixture
        .store
        .raw(&connection_id(McpConnectionScope::Workspace, "local"))
        .unwrap();
    assert_eq!(
        local["process"]["args"],
        source["mcpServers"]["local"]["args"]
    );
    assert_eq!(local["process"]["env"]["API_KEY"], "import-env-secret");
    assert_eq!(local["process"]["cwd"], "/project with spaces");
    assert_eq!(local["enabled"], false);
    let (_, _, remote) = fixture
        .store
        .raw(&connection_id(McpConnectionScope::Workspace, "remote"))
        .unwrap();
    assert_eq!(
        remote["endpoint"]["url"],
        source["mcpServers"]["remote"]["url"]
    );
    assert_eq!(
        remote["endpoint"]["headers"]["Authorization"],
        "Bearer import-header-secret"
    );
    assert_eq!(fs::read(&fixture.store.global_path).unwrap(), before.0);
}

#[test]
fn imports_are_atomic_when_a_later_selection_is_invalid_or_missing() {
    let fixture = Fixture::empty();
    let source = json!({"servers":{
        "a-valid":stdio("valid-mcp"),
        "z-invalid":{"transport":"stdio", "process":{"command":"fixture-mcp"}, "unrecognized":true}
    }});
    let preview = fixture
        .store
        .preview_import(&McpImportPreviewRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
        })
        .unwrap();
    assert!(
        preview
            .candidates
            .iter()
            .find(|item| item.name == "a-valid")
            .unwrap()
            .valid
    );
    assert!(
        !preview
            .candidates
            .iter()
            .find(|item| item.name == "z-invalid")
            .unwrap()
            .valid
    );
    let before = fixture.snapshot();
    for selected in [
        vec!["a-valid".into(), "z-invalid".into()],
        vec!["a-valid".into(), "zz-missing".into()],
    ] {
        let error = fixture
            .store
            .import(McpImportRequest {
                scope: McpConnectionScope::Workspace,
                document: source.clone(),
                selected,
                replace: false,
                expected_revision: preview.revision.clone(),
            })
            .unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Invalid);
        assert_eq!(fixture.snapshot(), before);
    }
    fixture
        .store
        .import(McpImportRequest {
            scope: McpConnectionScope::Workspace,
            document: source,
            selected: vec!["a-valid".into()],
            replace: false,
            expected_revision: preview.revision,
        })
        .unwrap();
    let connections = fixture.store.list().unwrap().connections;
    assert_eq!(connections.len(), 1);
    assert_eq!(connections[0].name, "a-valid");
}

#[test]
fn import_conflicts_require_explicit_replacement_and_do_not_partially_install() {
    let fixture = Fixture::empty();
    let existing = fixture.create(
        "z-existing",
        McpConnectionScope::Workspace,
        stdio("old-mcp"),
    );
    let source = json!({"servers":{
        "a-new":stdio("new-mcp"), "z-existing":stdio("replacement-mcp")
    }});
    let preview = fixture
        .store
        .preview_import(&McpImportPreviewRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
        })
        .unwrap();
    assert!(
        preview
            .candidates
            .iter()
            .find(|item| item.name == "z-existing")
            .unwrap()
            .conflict
    );
    let before = fixture.snapshot();
    let request = McpImportRequest {
        scope: McpConnectionScope::Workspace,
        document: source,
        selected: vec!["a-new".into(), "z-existing".into()],
        replace: false,
        expected_revision: preview.revision,
    };
    let error = fixture.store.import(request.clone()).unwrap_err();
    assert_eq!(error.kind, McpManagementErrorKind::Conflict);
    assert_eq!(fixture.snapshot(), before);
    fixture
        .store
        .import(McpImportRequest {
            replace: true,
            ..request
        })
        .unwrap();
    assert_eq!(fixture.store.list().unwrap().connections.len(), 2);
    assert_eq!(
        fixture.store.raw(&existing).unwrap().2["process"]["command"],
        "replacement-mcp"
    );
}

#[test]
fn import_revision_covers_both_source_and_destination() {
    let fixture = Fixture::empty();
    let mut source = json!({"servers":{"imported":stdio("initial-mcp")}});
    let preview = fixture
        .store
        .preview_import(&McpImportPreviewRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
        })
        .unwrap();
    fixture.create("other", McpConnectionScope::Workspace, stdio("other-mcp"));
    let before = fixture.snapshot();
    let error = fixture
        .store
        .import(McpImportRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
            selected: vec!["imported".into()],
            replace: false,
            expected_revision: preview.revision,
        })
        .unwrap_err();
    assert_eq!(error.kind, McpManagementErrorKind::Conflict);
    assert_eq!(fixture.snapshot(), before);

    let preview = fixture
        .store
        .preview_import(&McpImportPreviewRequest {
            scope: McpConnectionScope::Workspace,
            document: source.clone(),
        })
        .unwrap();
    source["servers"]["imported"]["process"]["command"] = json!("changed-source-mcp");
    let error = fixture
        .store
        .import(McpImportRequest {
            scope: McpConnectionScope::Workspace,
            document: source,
            selected: vec!["imported".into()],
            replace: false,
            expected_revision: preview.revision,
        })
        .unwrap_err();
    assert_eq!(error.kind, McpManagementErrorKind::Conflict);
    assert_eq!(fixture.snapshot(), before);
}

#[test]
fn imports_reject_unknown_or_transport_incompatible_fields_instead_of_silently_dropping_them() {
    let fixture = Fixture::empty();
    let before = fixture.snapshot();
    let invalid = [
        json!({"command":"fixture-mcp", "unexpected":"private-import-value"}),
        json!({"command":"fixture-mcp", "headers":{"X-Key":"private-import-value"}}),
        json!({"command":"fixture-mcp", "auth":{"kind":"bearer", "token":"private-import-value"}}),
        json!({"url":"https://example.test/mcp", "args":["private-import-value"]}),
        json!({"url":"https://example.test/mcp", "env":{"TOKEN":"private-import-value"}}),
        json!({"url":"https://example.test/mcp", "cwd":"private-import-value"}),
        json!({"command":"fixture-mcp", "disabled":"false"}),
        json!({"url":"https://example.test/mcp", "type":false}),
        json!({"command":"fixture-mcp", "enabled":true, "disabled":true}),
        json!({"transport":"stdio", "process":{"command":"fixture-mcp", "unexpected":"private-import-value"}}),
    ];
    for config in invalid {
        let source = json!({"mcpServers":{"invalid":config}});
        let preview = fixture
            .store
            .preview_import(&McpImportPreviewRequest {
                scope: McpConnectionScope::Workspace,
                document: source.clone(),
            })
            .unwrap();
        assert_eq!(preview.candidates.len(), 1);
        assert!(!preview.candidates[0].valid);
        assert!(!preview.candidates[0].errors.is_empty());
        assert!(
            !serde_json::to_string(&preview)
                .unwrap()
                .contains("private-import-value")
        );
        let error = fixture
            .store
            .import(McpImportRequest {
                scope: McpConnectionScope::Workspace,
                document: source,
                selected: vec!["invalid".into()],
                replace: false,
                expected_revision: preview.revision,
            })
            .unwrap_err();
        assert_eq!(error.kind, McpManagementErrorKind::Invalid);
        assert!(!error.message.contains("private-import-value"));
        assert_eq!(fixture.snapshot(), before);
    }
}

#[test]
fn invalid_existing_record_remains_visible_and_can_be_repaired_without_losing_hidden_values() {
    let fixture = Fixture::new(
        document(json!({
            "broken":{"transport":"stdio", "process":{"command":42, "env":{"TOKEN":"repair-secret"}}},
            "healthy":stdio("healthy-mcp")
        })),
        json!({}),
    );
    let id = connection_id(McpConnectionScope::Global, "broken");
    let detail = fixture.store.get(&id).unwrap();
    assert_eq!(detail.summary.state, "invalid");
    assert!(detail.summary.effective);
    assert!(detail.summary.error.is_some());
    assert_eq!(detail.config["process"]["env"]["TOKEN"], REDACTED);
    assert!(
        !serde_json::to_string(&detail)
            .unwrap()
            .contains("repair-secret")
    );
    let healthy = fixture
        .store
        .raw(&connection_id(McpConnectionScope::Global, "healthy"))
        .unwrap()
        .2;
    let before_workspace = fixture.snapshot().1;
    fixture.store.update(&id, McpConnectionUpdateRequest {
        config:json!({"transport":"stdio", "process":{"command":"repaired-mcp", "env":{"TOKEN":REDACTED}}}),
        preserve_secrets:detail.secret_paths,
        expected_revision:detail.summary.revision,
    }).unwrap();
    assert_eq!(
        fixture.store.get(&id).unwrap().summary.state,
        "disconnected"
    );
    assert!(fixture.store.resolved(&id).is_ok());
    assert_eq!(
        fixture.store.raw(&id).unwrap().2["process"]["env"]["TOKEN"],
        "repair-secret"
    );
    assert_eq!(
        fixture
            .store
            .raw(&connection_id(McpConnectionScope::Global, "healthy"))
            .unwrap()
            .2,
        healthy
    );
    assert_eq!(
        fs::read(&fixture.store.workspace_path).unwrap(),
        before_workspace
    );
}
