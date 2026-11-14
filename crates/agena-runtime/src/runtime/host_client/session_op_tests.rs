use super::*;
use agena_domain::{CommandOutputStream, ContentId, ContentTextPage, ContentTextPosition};
use agena_plugin_sdk::PluginErrorKind;
use agena_plugin_sdk::host_api::run_in_isolated_host_callback_context;
use agena_storage::kernel::{RequestId, RoundHandle};
use serde_json::{Value, json};

struct TestDirectory(PathBuf);

impl Drop for TestDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    runtime: Arc<AgenaRuntime>,
    client: Arc<RuntimeHostClient>,
    directory: TestDirectory,
}

impl Fixture {
    async fn new() -> Self {
        let root = std::env::temp_dir().join(format!("agena-session-op-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("runtime")).unwrap();
        let directory = TestDirectory(std::fs::canonicalize(root).unwrap());
        let workspace_root = directory.0.join("runtime");
        let config_path = directory.0.join("agena.json");
        let disabled_plugins = agena_bundled_plugins::plugins::sources::bundled_plugin_entries()
            .into_keys()
            .map(|id| (id, json!({"enabled": false, "package": {"kind": "static"}})))
            .collect::<serde_json::Map<String, Value>>();
        std::fs::write(
            &config_path,
            serde_json::to_vec(&json!({"plugins": {"list": disabled_plugins}})).unwrap(),
        )
        .unwrap();
        let runtime = AgenaRuntime::new_with_maintenance(
            crate::RuntimeCompositionConfig {
                load_request: crate::LoadConfigRequest {
                    workspace_root: Some(workspace_root.clone()),
                    config_path: Some(config_path),
                    ..Default::default()
                },
                workspace_root: Some(workspace_root),
                bootstrap_preflight: None,
                database_url: None,
                database_path: Some(directory.0.join("session-op.sqlite")),
                tracing_reload_handle: None,
            },
            false,
        )
        .await
        .unwrap();
        let snapshot = runtime.current_snapshot();
        assert!(snapshot.plugin_manager().plugins().is_empty());
        assert!(snapshot.global_plugin_manager().plugins().is_empty());
        let client = snapshot.host_client.clone();
        Self {
            runtime,
            client,
            directory,
        }
    }

    fn project(&self, name: &str) -> PathBuf {
        let path = self.directory.0.join(name);
        std::fs::create_dir_all(&path).unwrap();
        path
    }

    async fn command(&self, request: Value) -> Result<Value, PluginError> {
        let command = serde_json::from_value(request).unwrap();
        match HostClient::session_op(self.client.as_ref(), SessionOp::Command(command)).await? {
            SessionOpResponse::Command(value) => Ok(value),
            SessionOpResponse::Query(_) => panic!("command returned a query response"),
        }
    }

    async fn query(&self, request: Value) -> Result<Value, PluginError> {
        let query = serde_json::from_value(request).unwrap();
        match HostClient::session_op(self.client.as_ref(), SessionOp::Query(query)).await? {
            SessionOpResponse::Query(value) => Ok(value),
            SessionOpResponse::Command(_) => panic!("query returned a command response"),
        }
    }

    async fn shutdown(&self) {
        self.runtime.shutdown_and_wait().await.unwrap();
    }

    async fn content_round(&self) -> RoundHandle {
        let workspace = self
            .command(
                json!({"method": "create_workspace", "params": {"path": self.project("content")}}),
            )
            .await
            .unwrap();
        let session = self
            .command(json!({"method": "create_session", "params": {
                "workspace_id": workspace["data"]["id"], "title": "Content fixture"
            }}))
            .await
            .unwrap();
        let session_id =
            agena_storage::kernel::SessionId::try_from(session["data"]["id"].as_i64().unwrap())
                .unwrap();
        let kernel = self.runtime.application_services().session_kernel.unwrap();
        let execution = kernel
            .sessions
            .send(
                RequestId::new(),
                session_id,
                agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                    text: "Produce fixture content".into(),
                }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = kernel
            .executor
            .claim_next(RequestId::new(), session_id, execution.id)
            .await
            .unwrap()
            .value;
        attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value
    }

    async fn text_page(
        &self,
        session_id: i64,
        resource_id: ContentId,
        position: Option<ContentTextPosition>,
        max_bytes: usize,
    ) -> ContentTextPage {
        serde_json::from_value(
            self.query(json!({
                "method": "read_content_text", "params": {
                    "session_id": session_id, "resource_id": resource_id,
                    "position": position, "max_bytes": max_bytes,
                }
            }))
            .await
            .unwrap(),
        )
        .unwrap()
    }

    async fn cancel_round(&self, round: &RoundHandle) {
        self.runtime
            .application_services()
            .session_kernel
            .unwrap()
            .sessions
            .cancel(RequestId::new(), round.session_id(), round.execution_id())
            .await
            .unwrap();
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.runtime.shutdown();
    }
}

#[tokio::test]
async fn session_op_manages_workspaces_and_sessions_without_callback_context() {
    let fixture = Fixture::new().await;
    assert!(current_host_callback_context().is_none());
    let project = fixture.project("project");
    let created = fixture
        .command(json!({"method": "create_workspace", "params": {"path": project}}))
        .await
        .unwrap();
    let workspace_id = created["data"]["id"].as_i64().unwrap();
    assert!(created["session_id"].is_null());
    assert_eq!(created["execution_ids"], json!([]));
    assert_eq!(created["data"]["path"], json!(project));

    let resolved = fixture
        .command(json!({"method": "resolve_workspace", "params": {"path": project, "create_if_missing": false}}))
        .await
        .unwrap();
    assert_eq!(resolved["data"]["id"], workspace_id);
    let created_session = fixture
        .command(json!({"method": "create_session", "params": {"workspace_id": workspace_id, "title": "Plugin-created session"}}))
        .await
        .unwrap();
    let session_id = created_session["data"]["id"].as_i64().unwrap();
    assert_eq!(created_session["session_id"], session_id);
    assert_eq!(created_session["data"]["workspace_id"], workspace_id);
    assert_eq!(created_session["data"]["root_id"], session_id);
    let project_sessions = fixture
        .query(json!({"method": "list_sessions", "params": {
            "workspace_id": workspace_id, "roots": true,
            "exclude_subagents": true, "search": "Plugin-created"
        }}))
        .await
        .unwrap();
    assert!(
        project_sessions
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["id"] == session_id)
    );
    let runtime_workspace = fixture
        .command(json!({"method": "resolve_workspace", "params": {
            "path": fixture.directory.0.join("runtime"), "create_if_missing": true
        }}))
        .await
        .unwrap();
    assert_ne!(runtime_workspace["data"]["id"], workspace_id);
    let runtime_sessions = fixture
        .query(json!({"method": "list_sessions", "params": {
            "workspace_id": runtime_workspace["data"]["id"]
        }}))
        .await
        .unwrap();
    assert!(
        runtime_sessions
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["id"] != session_id)
    );
    let read_request = json!({"method": "get_session", "params": {"session_id": session_id}});
    let session = fixture.query(read_request.clone()).await.unwrap();
    assert_eq!(session["workspace_id"], workspace_id);
    assert_eq!(session["title"], "Plugin-created session");
    let version = session["version"].as_i64().unwrap();

    let stale_delete = fixture
        .command(json!({"method": "delete_session", "params": {"session_id": session_id, "expected_version": version + 1}}))
        .await
        .unwrap_err();
    assert_eq!(stale_delete.kind, PluginErrorKind::InvalidParams);
    let preserved = fixture.query(read_request.clone()).await.unwrap();
    assert_eq!(preserved["id"], session_id);
    assert_eq!(preserved["version"], version);
    let deleted = fixture
        .command(json!({"method": "delete_session", "params": {"session_id": session_id, "expected_version": version}}))
        .await
        .unwrap();
    assert_eq!(deleted["data"]["id"], session_id);
    assert!(fixture.query(read_request).await.is_err());

    let renamed_project = fixture.project("renamed-project");
    let updated = fixture
        .command(json!({"method": "update_workspace", "params": {"workspace_id": workspace_id, "path": renamed_project}}))
        .await
        .unwrap();
    assert_eq!(updated["data"]["id"], workspace_id);
    assert_eq!(updated["data"]["path"], json!(renamed_project));
    let workspace_request =
        json!({"method": "get_workspace", "params": {"workspace_id": workspace_id}});
    let workspace = fixture.query(workspace_request.clone()).await.unwrap();
    assert_eq!(workspace["path"], json!(renamed_project));
    let workspaces = fixture
        .query(json!({"method": "list_workspaces", "params": {"include_session_count": true}}))
        .await
        .unwrap();
    assert!(
        workspaces["items"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["id"] == workspace_id)
    );
    let resolved_project = fixture.project("resolved-project");
    let missing = json!({"method": "resolve_workspace", "params": {"path": resolved_project, "create_if_missing": false}});
    assert!(fixture.command(missing.clone()).await.is_err());
    let created_by_resolve = fixture
        .command(json!({"method": "resolve_workspace", "params": {"path": resolved_project, "create_if_missing": true}}))
        .await
        .unwrap();
    let resolved_again = fixture.command(missing).await.unwrap();
    assert_eq!(
        created_by_resolve["data"]["id"],
        resolved_again["data"]["id"]
    );
    // Deleted sessions retain their history and workspace reference. Exercise
    // workspace removal on a workspace that has no retained conversations.
    let empty_workspace_id = created_by_resolve["data"]["id"].as_i64().unwrap();
    fixture
        .command(
            json!({"method": "delete_workspace", "params": {"workspace_id": empty_workspace_id}}),
        )
        .await
        .unwrap();
    assert!(
        fixture
            .query(
                json!({"method": "get_workspace", "params": {"workspace_id": empty_workspace_id}})
            )
            .await
            .is_err()
    );
    assert!(resolved_project.is_dir());
    fixture.shutdown().await;
}

#[tokio::test]
async fn session_op_manages_permission_rules_and_inspects_runtime_with_plugin_only_context() {
    let fixture = Fixture::new().await;
    let context = HostCallbackContext {
        plugin_id: Some("fixture.connector".into()),
        ..Default::default()
    };
    run_in_isolated_host_callback_context(context, async {
        assert!(current_host_callback_context().unwrap().session_id.is_none());
        let status = fixture.query(json!({"method": "runtime"})).await.unwrap();
        assert_eq!(status["plugin_count"], 0);
        assert_eq!(status["workspace_root"], json!(fixture.directory.0.join("runtime")));
        let health = fixture.query(json!({"method": "health"})).await.unwrap();
        assert_eq!(health["status"], "ok");
        assert_eq!(health["database_connected"], true);
        assert_eq!(health["generation"], status["generation"]);
        assert_eq!(health["loaded_at"], status["loaded_at"]);
        assert!(fixture.query(json!({"method": "list_providers"})).await.unwrap().is_array());

        let created = fixture
            .command(json!({"method": "upsert_permission_rule", "params": {
                "subject_kind": "tool", "tool_name": "fixture.read", "scope": "global", "mode": "allow"
            }}))
            .await
            .unwrap();
        let rule_id = created["data"]["id"].as_i64().unwrap();
        assert!(created["session_id"].is_null());
        assert_eq!(created["data"]["mode"], "allow");
        assert_eq!(created["data"]["operator"], "fixture.connector");
        let rule_request = json!({"method": "get_permission_rule", "params": {"rule_id": rule_id}});
        let rule = fixture.query(rule_request.clone()).await.unwrap();
        assert_eq!(rule["tool_name"], "fixture.read");
        assert_eq!(rule["scope"], "global");
        let listed = fixture
            .query(json!({"method": "list_permission_rules", "params": {}}))
            .await
            .unwrap();
        assert!(listed["items"].as_array().unwrap().iter().any(|item| item["id"] == rule_id));

        let replaced = fixture
            .command(json!({"method": "replace_permission_rule", "params": {
                "rule_id": rule_id, "subject_kind": "tool", "tool_name": "fixture.write",
                "scope": "global", "mode": "deny"
            }}))
            .await
            .unwrap();
        assert_eq!(replaced["data"]["id"], rule_id);
        let changed = fixture.query(rule_request.clone()).await.unwrap();
        assert_eq!(changed["mode"], "deny");
        assert_eq!(changed["tool_name"], "fixture.write");
        let revoked = fixture
            .command(json!({"method": "revoke_permission_rule", "params": {"rule_id": rule_id, "reason": "fixture complete"}}))
            .await
            .unwrap();
        assert!(revoked["data"]["revoked_at"].is_string());
        assert_eq!(revoked["data"]["revoked_by"], "fixture.connector");
        let stored_revocation = fixture.query(rule_request.clone()).await.unwrap();
        assert_eq!(stored_revocation["revoked_reason"], "fixture complete");
        fixture
            .command(json!({"method": "delete_permission_rule", "params": {"rule_id": rule_id}}))
            .await
            .unwrap();
        assert!(fixture.query(rule_request).await.is_err());
        let after_delete = fixture
            .query(json!({"method": "list_permission_rules", "params": {}}))
            .await
            .unwrap();
        assert!(after_delete["items"].as_array().unwrap().iter().all(|item| item["id"] != rule_id));
        let cleared = fixture
            .command(json!({"method": "clear_finished_activities"}))
            .await
            .unwrap();
        assert!(cleared["session_id"].is_null());
        assert!(cleared["data"]["count"].as_u64().is_some());
    })
    .await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn session_op_reads_utf8_text_across_byte_pages_without_loss_or_repetition() {
    let fixture = Fixture::new().await;
    let round = fixture.content_round().await;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let expected = "开头🙂a\n中文🚀末尾\nsecond🌱行";
    output.append(expected).await.unwrap();
    output.finish(RequestId::new()).await.unwrap();

    for budget in [4, 5, 11] {
        let mut position = None;
        let mut actual = String::new();
        let mut finished = false;
        for _ in 0..=expected.len() {
            let page = fixture
                .text_page(round.session_id().get(), output.id(), position, budget)
                .await;
            assert!(!page.gap);
            assert_eq!(page.resource.state, agena_domain::ContentState::Complete);
            assert!(
                page.slices
                    .iter()
                    .map(|slice| slice.text.len())
                    .sum::<usize>()
                    <= budget
            );
            for slice in &page.slices {
                assert_eq!(slice.stream, None);
                actual.push_str(&slice.text);
            }
            assert_ne!(
                position,
                Some(page.next_position),
                "text cursor must advance"
            );
            assert_eq!(page.next_position.offset, 0);
            assert_eq!(page.next_position.after.sequence, actual.len() as u64);
            position = Some(page.next_position);
            if !page.has_more {
                finished = true;
                break;
            }
        }
        assert!(finished, "bounded text reads must terminate");
        assert_eq!(actual, expected, "UTF-8 text changed with budget {budget}");
        let exhausted = fixture
            .text_page(round.session_id().get(), output.id(), position, budget)
            .await;
        assert!(exhausted.slices.is_empty());
        assert!(!exhausted.has_more);
        assert_eq!(Some(exhausted.next_position), position);
    }

    fixture.cancel_round(&round).await;
    fixture.shutdown().await;
}

#[tokio::test]
async fn session_op_reads_large_log_records_across_offsets_and_event_pages() {
    let fixture = Fixture::new().await;
    let round = fixture.content_round().await;
    let draft = round
        .begin_tool_draft(RequestId::new(), "log-fixture".into(), "tools_call".into())
        .await
        .unwrap()
        .value;
    let tool = draft
        .finalize(
            RequestId::new(),
            json!({"tool": "fixture.log", "input": {}}),
        )
        .await
        .unwrap()
        .value;
    let output = tool.begin_log(RequestId::new()).await.unwrap().value;
    let mut expected = std::collections::BTreeMap::new();
    for index in 0..71 {
        let stream = if index % 2 == 0 {
            CommandOutputStream::Stdout
        } else {
            CommandOutputStream::Stderr
        };
        let text = if index == 0 {
            "长日志🙂\n".repeat(1024)
        } else {
            format!("事件{index}🚀\n")
        };
        let captured_at_ms = 1_000 + index;
        let cursor = output
            .append_at(captured_at_ms, stream.clone(), &text)
            .await
            .unwrap();
        expected.insert(cursor.sequence, (stream, captured_at_ms, text));
    }
    output.finish(RequestId::new()).await.unwrap();

    // The small budget splits one record; the large one reaches the kernel's
    // 64-record page boundary before exhausting the byte budget.
    for budget in [1021, 1024 * 1024] {
        let mut position = None;
        let mut actual = std::collections::BTreeMap::<u64, String>::new();
        let mut partial_record = false;
        let mut pages = 0;
        let mut finished = false;
        for _ in 0..100 {
            let page = fixture
                .text_page(round.session_id().get(), output.id(), position, budget)
                .await;
            pages += 1;
            assert!(!page.gap);
            assert!(
                page.slices
                    .iter()
                    .map(|slice| slice.text.len())
                    .sum::<usize>()
                    <= budget
            );
            for slice in &page.slices {
                let (stream, captured_at_ms, text) = &expected[&slice.cursor.sequence];
                assert_eq!(slice.stream.as_ref(), Some(stream));
                assert_eq!(slice.captured_at_ms, *captured_at_ms);
                let received = actual.entry(slice.cursor.sequence).or_default();
                assert_eq!(
                    slice.offset,
                    received.len(),
                    "record slices must be contiguous"
                );
                assert_eq!(
                    slice.text,
                    text[slice.offset..slice.offset + slice.text.len()]
                );
                received.push_str(&slice.text);
            }
            if let Some(previous) = position {
                assert!(
                    (page.next_position.after.sequence, page.next_position.offset)
                        > (previous.after.sequence, previous.offset),
                    "log cursor must advance",
                );
            }
            partial_record |= page.next_position.offset > 0;
            position = Some(page.next_position);
            if !page.has_more {
                finished = true;
                break;
            }
        }
        assert!(finished, "bounded log reads must terminate");
        assert!(pages > 1);
        if budget == 1021 {
            assert!(partial_record, "large records must continue with an offset");
        } else {
            assert_eq!(pages, 2, "71 records should cross the 64-record page limit");
        }
        let expected_text = expected
            .iter()
            .map(|(sequence, (_, _, text))| (*sequence, text.clone()))
            .collect::<std::collections::BTreeMap<_, _>>();
        assert_eq!(actual, expected_text);
        assert_eq!(
            position.unwrap().after.sequence,
            *expected.last_key_value().unwrap().0
        );
        assert_eq!(position.unwrap().offset, 0);
    }

    fixture.cancel_round(&round).await;
    fixture.shutdown().await;
}
