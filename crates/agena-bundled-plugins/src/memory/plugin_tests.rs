use super::*;
use serde_json::json;

use agena_plugin_host::sdk::{
    EventEnvelope, EventFilter, HostClient,
    host_api::{
        EventSubscription, HostCallbackContext, LogLevel, PathPermissionQuery, PermissionQuery,
    },
};

struct RecoveryTestHost {
    workspace_root: PathBuf,
    policy_unavailable: bool,
    deny_target_write: std::sync::Mutex<bool>,
    checked_sessions: std::sync::Mutex<Vec<Option<i64>>>,
}

#[async_trait::async_trait]
impl HostClient for RecoveryTestHost {
    async fn log(&self, _: LogLevel, _: String, _: serde_json::Value) {}
    async fn publish_event(&self, _: EventEnvelope) -> SdkResult<()> {
        unreachable!()
    }
    async fn subscribe_events(&self, _: EventFilter) -> SdkResult<EventSubscription> {
        unreachable!()
    }
    async fn read_config(&self, _: Option<String>) -> SdkResult<serde_json::Value> {
        unreachable!()
    }
    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> SdkResult<ToolInvokeOutput> {
        unreachable!()
    }
    async fn companion_context(&self) -> SdkResult<serde_json::Value> {
        if self.policy_unavailable {
            return Err(PluginError::internal("workspace memory policy unavailable"));
        }
        Ok(json!({
            "profile": {"preferences": {"memory_enabled": true}},
            "memory_workspace_root": self.workspace_root,
            "memory_instance_id": "test-instance",
            "memory_checkpoint_entry_id": null
        }))
    }
    async fn check_path_permission(
        &self,
        request: PathPermissionQuery,
    ) -> SdkResult<PermissionQuery> {
        self.checked_sessions
            .lock()
            .unwrap()
            .push(current_memory_session_id());
        if *self.deny_target_write.lock().unwrap()
            && request.kind == agena_domain::PathKind::Write
            && request.path.ends_with("/protected.md")
        {
            Ok(PermissionQuery::Deny {
                reason: "protected target".to_string(),
            })
        } else {
            Ok(PermissionQuery::Allow {})
        }
    }
    async fn require_path_permission(&self, _: PathPermissionQuery) -> SdkResult<()> {
        if is_operator_memory_call() {
            return Err(PluginError::from_kind(
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied,
                "operator target gate",
            ));
        }
        panic!("recovery must not open an interactive approval");
    }
}

fn recovery_test_plugin(
    root: &std::path::Path,
    policy_unavailable: bool,
) -> (MemoryPlugin, Arc<RecoveryTestHost>) {
    let plugin = MemoryPlugin::new();
    let host = Arc::new(RecoveryTestHost {
        workspace_root: root.to_path_buf(),
        policy_unavailable,
        deny_target_write: std::sync::Mutex::new(true),
        checked_sessions: std::sync::Mutex::new(Vec::new()),
    });
    plugin.permissions.initialize(host.clone()).unwrap();
    assert!(plugin.host.set(host.clone()).is_ok());
    plugin.config.set(MemoryConfig::default()).unwrap();
    plugin.workspace_root.set(root.to_path_buf()).unwrap();
    (plugin, host)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recovery_preserves_callback_authority_and_requires_write_on_every_target() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().to_path_buf();
    let store = MemoryStore::at(root.join("memory"));
    std::fs::create_dir_all(store.dir()).unwrap();
    let target = store.dir().join("protected.md");
    let journal = store.dir().join(".memory-transaction.json");
    std::fs::write(&target, "interrupted publication").unwrap();
    std::fs::write(
        &journal,
        json!({
            "schema_version": 1,
            "entries": [{"name": "protected.md", "previous": "last committed revision"}]
        })
        .to_string(),
    )
    .unwrap();
    let (plugin, host) = recovery_test_plugin(&root, false);
    let callback = HostCallbackContext {
        session_id: Some(27),
        plugin_id: Some("agena.memory".to_string()),
        tool_name: Some("write".to_string()),
        ..Default::default()
    };
    let result = agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(
        callback.clone(),
        plugin.recover_memory_store(store.clone()),
    )
    .await;
    assert!(result.is_err());
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "interrupted publication"
    );
    assert!(journal.exists());
    assert!(!store.dir().join(".memory.lock").exists());
    *host.deny_target_write.lock().unwrap() = false;
    agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(
        callback,
        plugin.recover_memory_store(store.clone()),
    )
    .await
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(&target).unwrap(),
        "last committed revision"
    );
    assert!(!journal.exists());
    assert!(!host.checked_sessions.lock().unwrap().is_empty());
    assert!(
        host.checked_sessions
            .lock()
            .unwrap()
            .iter()
            .all(|session| *session == Some(27))
    );

    let untouched = MemoryStore::at(root.join("absent"));
    let checks = host.checked_sessions.lock().unwrap().len();
    plugin
        .recover_memory_store(untouched.clone())
        .await
        .unwrap();
    assert!(!untouched.dir().exists());
    assert_eq!(host.checked_sessions.lock().unwrap().len(), checks);
}

#[tokio::test]
async fn unavailable_workspace_policy_blocks_writes_and_automatic_recall() {
    let dir = tempfile::tempdir().unwrap();
    let (plugin, host) = recovery_test_plugin(dir.path(), true);
    let input: MemoryWriteInput = serde_json::from_value(json!({
        "name": "must-not-write", "kind": "fact", "content": "unknown policy must not enable memory"
    }))
    .unwrap();
    assert!(plugin.invoke_write(&input).await.is_err());
    assert!(plugin.workspace_root().await.is_err());
    let input = ChatMessagesTransformInput {
        session_id: 27,
        workspace_root: None,
        messages: vec![ChatMessage::user("我们之前数据库选型是什么")],
    };
    assert!(
        plugin
            .chat_messages_transform(input)
            .await
            .unwrap()
            .is_none()
    );
    assert!(host.checked_sessions.lock().unwrap().is_empty());
}

#[tokio::test]
async fn operator_calls_keep_their_authorized_workspace_without_a_kernel_session() {
    let dir = tempfile::tempdir().unwrap();
    let (plugin, _) = recovery_test_plugin(dir.path(), true);
    let root = dir.path().join("operator-workspace");
    let callback = HostCallbackContext {
        session_id: Some(-1),
        call_id: Some(9),
        workspace_root: Some(root.to_string_lossy().into_owned()),
        ..Default::default()
    };
    agena_plugin_host::sdk::host_api::run_in_isolated_host_callback_context(callback, async {
        assert_eq!(plugin.workspace_root().await.unwrap(), root);
        assert!(current_memory_session_id().is_none());
        let input: MemoryWriteInput = serde_json::from_value(json!({
            "name": "operator-note", "kind": "fact", "content": "an explicitly controlled operator note"
        }))
        .unwrap();
        let error = plugin.invoke_write(&input).await.unwrap_err();
        assert!(
            error.to_string().contains("operator target gate"),
            "{error}"
        );
        assert!(!root.exists());
    })
    .await;
}

fn stored_record(body: String) -> MemoryRecord {
    MemoryRecord {
        sha256: "ab".repeat(32),
        file_name: "paged-record.md".to_string(),
        path: PathBuf::from("/isolated/paged-record.md"),
        frontmatter: agena_storage::MemoryFrontmatter {
            schema_version: 2,
            name: "paged-record".to_string(),
            description: "A persisted memory".to_string(),
            revision: 1,
        },
        body,
        metadata: MemoryMetadata::default(),
    }
}

fn recalled(name: &str, body: String, reason: &'static str, query: &str) -> RecalledMemory {
    let document = MemorySearchDocument::new(
        name.to_string(),
        name.to_string(),
        String::new(),
        if reason == "core_preference" {
            MemoryKind::Preference
        } else {
            MemoryKind::Fact
        },
        body,
        format!("/isolated/{name}.md"),
    );
    let excerpt = document.excerpt(query, 800);
    RecalledMemory {
        hit: MemorySearchHit {
            document,
            score: 1.0,
            excerpt,
            matched_terms: if query.is_empty() {
                Vec::new()
            } else {
                vec![query.to_string()]
            },
        },
        metadata: MemoryMetadata {
            kind: if reason == "core_preference" {
                MemoryKind::Preference
            } else {
                MemoryKind::Fact
            },
            ..Default::default()
        },
        reason,
    }
}

#[test]
fn memory_inputs_require_canonical_kind_and_observed_delete_revision() {
    let create = json!({"name":"note", "content":"durable knowledge", "kind":"fact"});
    let input: MemoryWriteInput = serde_json::from_value(create.clone()).unwrap();
    assert_eq!(input.kind, MemoryKind::Fact);
    assert!(input.expected_sha256.is_none());
    for invalid in [
        json!({"name":"note", "content":"missing kind"}),
        json!({"name":"note", "content":"old kind", "memory_type":"project"}),
        json!({"name":"note", "content":"old field", "kind":"fact", "memory_type":"project"}),
        json!({"name":"note", "content":"old value", "kind":"project"}),
    ] {
        assert!(serde_json::from_value::<MemoryWriteInput>(invalid).is_err());
    }
    assert!(serde_json::from_value::<MemoryDeleteInput>(json!({"name":"note"})).is_err());
    assert!(
        serde_json::from_value::<MemoryDeleteInput>(json!({"name":"note", "expected_sha256":null}))
            .is_err()
    );
    let deleted: MemoryDeleteInput =
        serde_json::from_value(json!({"name":"note", "expected_sha256":"ab".repeat(32)})).unwrap();
    assert_eq!(deleted.expected_sha256, "ab".repeat(32));
}

#[test]
fn omitted_settings_use_bounded_defaults_and_invalid_budgets_are_rejected() {
    let config =
        parse_memory_config(json!({"retrieval":{"enabled":true,"limit":3,"min_query_chars":8}}))
            .unwrap();
    assert_eq!(config.retrieval.max_context_tokens, 2048);
    assert!(config.maintenance.enabled);
    assert_eq!(config.maintenance.min_user_messages, 1);
    assert!(parse_memory_config(json!({"retrieval":{"max_context_tokens":0}})).is_err());
    assert!(parse_memory_config(json!({"maintenance":{"idle_seconds":0}})).is_err());
    assert!(parse_memory_config(json!({"maintenance":{"max_output_tokens":100000}})).is_err());
}

#[test]
fn budgeted_context_keeps_the_matched_late_passage() {
    let body = format!(
        "{}\n数据库决策：使用 PostgreSQL，连接池大小为 16。\n{}",
        "无关背景资料。".repeat(400),
        "后续附录。".repeat(400)
    );
    let item = recalled("database", body, "query_match", "数据库");
    let config = MemoryRetrievalConfig {
        max_context_tokens: 256,
        core_context_tokens: 0,
        ..Default::default()
    };
    let context = render_memory_context(&[item], &config);
    assert!(context.contains("PostgreSQL"), "{context}");
    assert!(estimated_tokens(&context) <= 256);
    assert!(context.len() <= 1024);
    assert!(!context.contains(&"无关背景资料。".repeat(30)));
}

#[test]
fn more_than_three_concise_core_preferences_fit_and_evidence_keeps_a_budget() {
    let mut items = (0..4)
        .map(|n| {
            recalled(
                &format!("preference-{n}"),
                format!("preference-marker-{n}"),
                "core_preference",
                "",
            )
        })
        .collect::<Vec<_>>();
    items.push(recalled(
        "project",
        "project-evidence-marker".to_string(),
        "query_match",
        "project-evidence-marker",
    ));
    let context = render_memory_context(&items, &MemoryRetrievalConfig::default());
    for n in 0..4 {
        assert!(
            context.contains(&format!("preference-marker-{n}")),
            "{context}"
        );
    }
    assert!(context.contains("project-evidence-marker"));
    assert!(estimated_tokens(&context) <= 2048);
}

#[test]
fn search_returns_bounded_evidence_and_a_read_pointer_instead_of_full_documents() {
    let item = recalled(
        "large-note",
        format!("{}\nsearch-marker\n", "background ".repeat(2000)),
        "query_match",
        "search-marker",
    );
    let output = memory_search_output(&item);
    assert_eq!(output["read"]["name"], "large-note");
    assert!(
        output["excerpt"]["text"]
            .as_str()
            .unwrap()
            .contains("search-marker")
    );
    assert!(output.get("searchable_text").is_none());
    assert!(output.get("body").is_none());
    assert!(output["metadata"].is_object());
    assert!(output.to_string().len() < 6000);
}

#[test]
fn matching_core_memory_uses_query_budget_when_core_budget_is_zero_or_full() {
    let item = recalled(
        "matching-preference",
        "preferred-database PostgreSQL".to_string(),
        "core_preference",
        "preferred-database",
    );
    let config = MemoryRetrievalConfig {
        core_context_tokens: 0,
        ..Default::default()
    };
    let context = render_memory_context(&[item], &config);
    assert!(context.contains("PostgreSQL"), "{context}");
    assert!(context.contains("Relevant historical evidence"));
    assert!(estimated_tokens(&context) <= config.max_context_tokens as usize);

    let mut items = (0..5)
        .map(|n| {
            recalled(
                &format!("pref-{n}"),
                format!("preference-marker-{n}"),
                "core_preference",
                "",
            )
        })
        .collect::<Vec<_>>();
    items.push(recalled(
        "matched-sixth",
        "late-query-evidence".to_string(),
        "core_preference",
        "late-query-evidence",
    ));
    let config = MemoryRetrievalConfig {
        core_context_tokens: 384,
        ..Default::default()
    };
    let context = render_memory_context(&items, &config);
    assert!(context.contains("late-query-evidence"), "{context}");
    assert_eq!(context.matches("matched-sixth [").count(), 1, "{context}");
    assert!(estimated_tokens(&context) <= config.max_context_tokens as usize);
}

#[test]
fn suppressed_sources_stay_hidden_even_in_history_and_direct_reads() {
    let mut record = stored_record("forgotten-evidence".to_string());
    record.metadata.source_suppressed = true;
    record.metadata.status = MemoryStatus::Archived;
    let context = MemoryContext::default();
    assert!(!memory_record_is_visible(&record, Some(1), &context, false));
    assert!(!memory_record_is_visible(&record, Some(1), &context, true));
    assert!(require_model_record_read(&record, Some(1), None).is_err());

    record.metadata.source_suppressed = false;
    assert!(memory_record_is_visible(&record, Some(1), &context, true));
    assert!(require_model_record_read(&record, Some(1), None).is_ok());
    record.metadata.scope = MemoryScope::Session {
        session_id: 2,
        instance_id: "other".to_string(),
    };
    assert!(!memory_record_is_visible(&record, Some(1), &context, true));
    assert!(require_model_record_read(&record, Some(1), None).is_err());
}

#[test]
fn derived_task_projection_must_match_current_host_checkpoint() {
    let mut record = stored_record("current-task".to_string());
    record.metadata.kind = MemoryKind::Task;
    record.metadata.origin = MemoryOrigin::Derived;
    record.metadata.provenance_verified = true;
    record.metadata.scope = MemoryScope::Session {
        session_id: 7,
        instance_id: "database-a".to_string(),
    };
    record.metadata.source = Some(MemorySource {
        session_id: 7,
        instance_id: "database-a".to_string(),
        entry_id: Some(42),
        ..Default::default()
    });
    let mut context = MemoryContext {
        instance_id: Some("database-a".to_string()),
        checkpoint_entry_id: Some(42),
    };
    assert!(memory_record_is_visible(&record, Some(7), &context, false));
    assert!(!memory_record_is_visible(&record, Some(8), &context, false));
    context.checkpoint_entry_id = Some(43);
    assert!(!memory_record_is_visible(&record, Some(7), &context, false));
    assert!(memory_record_is_visible(&record, Some(7), &context, true));
    context.checkpoint_entry_id = None;
    assert!(!memory_record_is_visible(&record, Some(7), &context, false));
    context.checkpoint_entry_id = Some(42);
    context.instance_id = Some("database-b".to_string());
    assert!(!memory_record_is_visible(&record, Some(7), &context, true));
    context.instance_id = Some("database-a".to_string());
    record.metadata.provenance_verified = false;
    assert!(!memory_record_is_visible(&record, Some(7), &context, false));
    record.metadata.origin = MemoryOrigin::Explicit;
    assert!(memory_record_is_visible(&record, Some(7), &context, false));
}

#[test]
fn memory_get_pages_single_line_cjk_without_loss_and_keeps_full_record_revision() {
    let body = "数据库🙂e\u{301}PostgreSQL配置".repeat(3000);
    let record = stored_record(body.clone());
    let total = body.chars().count();
    let mut reconstructed = String::new();
    let mut offset = 0;
    loop {
        let page = memory_record_output(&record, offset, 997);
        assert_eq!(page.body_range.offset, offset);
        assert_eq!(page.body_range.total_chars, total);
        assert_eq!(page.body_range.returned_chars, page.body.chars().count());
        assert!(page.body_range.returned_chars <= 997);
        assert_eq!(page.sha256, record.sha256);
        let text = format_memory_entry(&page);
        assert!(text.len() < 8192);
        reconstructed.push_str(&page.body);
        if let Some(next) = page.body_range.next_offset {
            assert_eq!(next, offset + page.body_range.returned_chars);
            assert!(next > offset);
            assert!(text.contains("Continue with memory.get("));
            offset = next;
        } else {
            break;
        }
    }
    assert_eq!(reconstructed, body);
    let input: MemoryGetInput = serde_json::from_value(json!({"name":"paged-record"})).unwrap();
    assert_eq!(input.offset, 0);
    assert!(input.limit.is_none());
    let default_page = memory_record_output(&record, 0, DEFAULT_MEMORY_GET_CHARS);
    assert_eq!(default_page.body_range.next_offset, Some(8000));
    let capped = memory_record_output(&record, 0, usize::MAX);
    assert_eq!(capped.body_range.returned_chars, 32000);
    let after_end = memory_record_output(&record, usize::MAX, 8000);
    assert_eq!(after_end.body_range.offset, total);
    assert!(after_end.body.is_empty());
    assert!(after_end.body_range.next_offset.is_none());
}

#[test]
fn record_output_uses_canonical_metadata_and_large_write_echo_is_paged() {
    let small = stored_record("完整的小记忆".to_string());
    let output = memory_record_output(&small, 0, DEFAULT_MEMORY_GET_CHARS);
    let payload = serde_json::to_value(&output).unwrap();
    assert_eq!(payload["name"], "paged-record");
    assert_eq!(payload["file_name"], "paged-record.md");
    assert_eq!(payload["body"], small.body);
    assert_eq!(payload["sha256"], small.sha256);
    assert_eq!(payload["metadata"]["kind"], "fact");
    assert!(payload.get("memory_type").is_none());
    assert!(payload["body_range"]["next_offset"].is_null());
    assert!(body_page_note(&output).is_empty());

    let large = stored_record("大型记忆🙂".repeat(10000));
    let output = memory_record_output(&large, 0, DEFAULT_MEMORY_GET_CHARS);
    let payload = serde_json::to_value(&output).unwrap();
    assert_eq!(output.body_range.returned_chars, 8000);
    assert!(output.body_range.next_offset.is_some());
    assert_eq!(payload["sha256"], large.sha256);
    assert!(payload.to_string().len() < 40000);
}

#[test]
fn list_and_record_display_fields_are_bounded_even_with_huge_frontmatter() {
    let mut record = stored_record("body-marker".to_string());
    record.frontmatter.name = format!("{}PRIVATE_NAME_TAIL", "名称\n".repeat(10000));
    record.frontmatter.description = format!("{}PRIVATE_DESCRIPTION_TAIL", "背景\n".repeat(20000));
    let text = format_memory_list(std::slice::from_ref(&record));
    assert_eq!(text.lines().count(), 1);
    assert!(
        text.chars().count() <= MAX_MEMORY_DISPLAY_NAME_CHARS + MAX_MEMORY_DESCRIPTION_CHARS + 32
    );
    assert!(!text.contains("PRIVATE_NAME_TAIL"));
    assert!(!text.contains("PRIVATE_DESCRIPTION_TAIL"));
    let page = memory_record_output(&record, 0, 8000);
    assert!(page.name_truncated && page.description_truncated);
    assert!(format_memory_entry(&page).len() < 4096);
    assert_eq!(page.body, "body-marker");
    let navigation = memory_navigation_output(&record);
    assert!(navigation["name"].as_str().unwrap().chars().count() <= MAX_MEMORY_DISPLAY_NAME_CHARS);
    assert!(
        navigation["description"].as_str().unwrap().chars().count() <= MAX_MEMORY_DESCRIPTION_CHARS
    );
    assert_eq!(navigation["read"]["name"], "paged-record");
}
