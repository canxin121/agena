use super::*;

async fn fixture() -> (SessionManager, i64, i64) {
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let root = std::env::temp_dir();
    let workspace = kernel
        .workspaces
        .ensure_id(&root.to_string_lossy())
        .await
        .unwrap();
    let plugins = agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins: vec![],
        config: Default::default(),
        workspace_root: root.clone(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        root,
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let preferences = Arc::new(agena_storage_sqlite::SqliteCompanionRepository::new(
        kernel.database.clone(),
    ));
    let manager = SessionManager::new(
        kernel,
        Arc::new(ProviderRegistry::default()),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        Default::default(),
    );
    manager.install_memory_preferences(preferences);
    let bot = manager.ensure_bot_session().await.unwrap();
    (manager, bot, workspace)
}
fn request(value: Value) -> BotRequest {
    serde_json::from_value(value).unwrap()
}
async fn discover(manager: &SessionManager, source: i64, value: Value) -> Value {
    manager
        .bot_discovery(source, &request(value))
        .await
        .unwrap()
}
async fn create(
    manager: &SessionManager,
    workspace: i64,
    title: &str,
    config: SessionConfiguration,
) -> SessionId {
    manager
        .kernel()
        .sessions
        .create_configured(RequestId::new(), workspace, title.into(), config)
        .await
        .unwrap()
        .value
        .id
}
async fn text(
    manager: &SessionManager,
    session: SessionId,
    value: &str,
) -> (EntryId, AttemptHandle, RoundHandle) {
    let queued = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "actual user question".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, queued.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![queued.message_id])
        .await
        .unwrap()
        .value;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    let id = output.entry_id();
    output.append(value).await.unwrap();
    output.finish(RequestId::new()).await.unwrap();
    (id, attempt, round)
}

#[tokio::test]
async fn native_title_flags_parent_and_state_filters_and_cursor_ownership() {
    let (manager, bot, workspace) = fixture().await;
    let parent = create(&manager, workspace, "parent", Default::default()).await;
    let selected = create(
        &manager,
        workspace,
        "needle duplicate",
        SessionConfiguration {
            parent_session_id: Some(parent),
            relation_kind: agena_domain::SessionRelationKind::Child,
            ..Default::default()
        },
    )
    .await;
    manager
        .kernel()
        .sessions
        .update_metadata(
            RequestId::new(),
            selected,
            MetadataChange {
                favorite: Some(true),
                pinned: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    create(&manager, workspace, "needle duplicate", Default::default()).await;
    let result=discover(&manager,bot,json!({"action":"sessions","workspace_id":workspace,"query":"needle","favorite":true,"pinned":true,"parent_session_id":parent.get(),"states":["ready"]})).await;
    assert_eq!(result["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(result["sessions"][0]["session_id"], selected.get());
    assert_eq!(result["sessions"][0]["favorite"], true);
    assert_eq!(result["fulltext_search"], false);
    assert_eq!(result["has_more"], false);
    let first = discover(
        &manager,
        bot,
        json!({"action":"sessions","query":"needle","limit":1}),
    )
    .await;
    let token = first["next_cursor"].as_str().unwrap();
    for changed in [
        json!({"action":"sessions","query":"other","cursor":token}),
        json!({"action":"sessions","query":"needle","favorite":false,"cursor":token}),
        json!({"action":"read","session_id":parent.get(),"cursor":token}),
        json!({"action":"sessions","query":"needle","workspace_id":workspace,"cursor":token}),
    ] {
        assert!(manager.bot_discovery(bot, &request(changed)).await.is_err());
    }
    let second = discover(
        &manager,
        bot,
        json!({"action":"sessions","query":"needle","limit":1,"cursor":token}),
    )
    .await;
    assert_ne!(
        first["sessions"][0]["session_id"],
        second["sessions"][0]["session_id"]
    );
    assert_eq!(second["has_more"], false);
    assert!(
        manager
            .bot_discovery(
                bot + 100,
                &request(json!({"action":"sessions","query":"needle","cursor":token}))
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn global_registered_paging_filters_private_self_and_subagent_ancestry_without_losing_next() {
    let (manager, bot, workspace) = fixture().await;
    let other = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/tmp/discovery-global-other")
        .await
        .unwrap();
    let first = create(&manager, workspace, "visible first", Default::default()).await;
    let second = create(&manager, other, "visible second", Default::default()).await;
    let bot_child = manager
        .kernel()
        .sessions
        .create_global(
            RequestId::new(),
            "operator private continuation".into(),
            SessionConfiguration {
                parent_session_id: Some(session_id(bot).unwrap()),
                relation_kind: agena_domain::SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    assert!(
        manager
            .bot_discovery(
                bot,
                &request(json!({"action":"read","session_id":bot_child.get()}))
            )
            .await
            .is_err()
    );
    let private = agena_storage_layout::StorageLayout::current()
        .root()
        .join("channel-scopes/discovery-private");
    let excluded_workspace = manager
        .kernel_runtime
        .workspaces
        .ensure_id(&private.to_string_lossy())
        .await
        .unwrap();
    create(
        &manager,
        excluded_workspace,
        "private must not leak",
        Default::default(),
    )
    .await;
    let agent = create(
        &manager,
        workspace,
        "subagent secret",
        SessionConfiguration {
            parent_session_id: Some(first),
            relation_kind: agena_domain::SessionRelationKind::Subagent,
            ..Default::default()
        },
    )
    .await;
    // Ordinary-looking descendants cannot escape a Subagent boundary. Put more
    // than one scan budget before the visible targets to verify empty-page truth.
    for _ in 0..130 {
        create(
            &manager,
            workspace,
            "excluded descendant",
            SessionConfiguration {
                parent_session_id: Some(agent),
                relation_kind: agena_domain::SessionRelationKind::Child,
                ..Default::default()
            },
        )
        .await;
    }
    // A private parent's tombstone must not erase its descendant restrictions.
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), agent)
        .await
        .unwrap();
    let mut cursor = None;
    let mut ids = Vec::new();
    let mut saw_empty_continuation = false;
    for _ in 0..8 {
        let result = discover(
            &manager,
            bot,
            json!({"action":"sessions","limit":1,"cursor":cursor}),
        )
        .await;
        let rows = result["sessions"].as_array().unwrap();
        let raw = serde_json::to_string(&result).unwrap();
        assert!(!raw.contains("private must not leak"));
        assert!(!raw.contains("excluded descendant"));
        assert!(!raw.contains("subagent secret"));
        assert!(!raw.contains("operator private continuation"));
        if rows.is_empty() && result["has_more"] == true {
            saw_empty_continuation = true;
            assert!(result["next_cursor"].is_string());
        }
        ids.extend(rows.iter().map(|s| s["session_id"].as_i64().unwrap()));
        if result["has_more"] == false {
            break;
        }
        cursor = result["next_cursor"].as_str().map(str::to_owned);
    }
    ids.sort();
    let mut expected = vec![first.get(), second.get()];
    expected.sort();
    assert_eq!(ids, expected);
    assert!(saw_empty_continuation);
}

#[tokio::test]
async fn duplicate_project_names_are_candidates_with_paths_and_exact_empty_detail() {
    let (manager, bot, _) = fixture().await;
    let one = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/tmp/discovery-a/same-project")
        .await
        .unwrap();
    let two = manager
        .kernel_runtime
        .workspaces
        .ensure_id("/tmp/discovery-b/same-project")
        .await
        .unwrap();
    let result = discover(
        &manager,
        bot,
        json!({"action":"projects","query":"same-project"}),
    )
    .await;
    assert_eq!(result["resolution"], "ambiguous");
    let rows = result["projects"].as_array().unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| r["path"].as_str().unwrap().ends_with("same-project"))
    );
    assert!(rows.iter().any(|r| r["workspace_id"] == one));
    assert!(rows.iter().any(|r| r["workspace_id"] == two));
    let exact = discover(
        &manager,
        bot,
        json!({"action":"projects","query":"/tmp/discovery-a/same-project"}),
    )
    .await;
    assert_eq!(exact["projects"][0]["match_kind"], "exact_path");
    let detail = discover(
        &manager,
        bot,
        json!({"action":"project","workspace_id":one}),
    )
    .await;
    assert_eq!(detail["project"]["workspace_id"], one);
    assert_eq!(detail["sessions"], json!([]));
    assert_eq!(detail["has_more"], false);
    let first = discover(
        &manager,
        bot,
        json!({"action":"projects","query":"same-project","limit":1}),
    )
    .await;
    let second = discover(
        &manager,
        bot,
        json!({"action":"projects","query":"same-project","limit":1,"cursor":first["next_cursor"]}),
    )
    .await;
    assert_ne!(
        first["projects"][0]["workspace_id"],
        second["projects"][0]["workspace_id"]
    );
    assert_eq!(second["has_more"], false);
}

#[tokio::test]
async fn read_text_exact_utf8_chunks_cap_foreign_rejection_and_inherited_membership() {
    let (manager, bot, workspace) = fixture().await;
    let session = create(&manager, workspace, "transcript", Default::default()).await;
    let original = "你好🦀 abc\n".repeat(2000);
    let (entry, _, _) = text(&manager, session, &original).await;
    let mut offset = 0;
    let mut actual = String::new();
    loop {
        let result=discover(&manager,bot,json!({"action":"read_text","session_id":session.get(),"entry_id":entry.get(),"byte_offset":offset,"max_bytes":1001})).await;
        let chunk = result["text"].as_str().unwrap();
        assert!(chunk.len() <= 1001);
        actual.push_str(chunk);
        let next = result["next_byte_offset"].as_u64().unwrap();
        assert!(next > offset);
        offset = next;
        if result["has_more"] == false {
            break;
        }
    }
    assert_eq!(actual, original);
    let cap=discover(&manager,bot,json!({"action":"read_text","session_id":session.get(),"entry_id":entry.get(),"max_bytes":1000000})).await;
    assert!(cap["text"].as_str().unwrap().len() <= TEXT_BYTES);
    assert_eq!(cap["has_more"], true);
    assert!(manager.bot_discovery(bot,&request(json!({"action":"read_text","session_id":session.get(),"entry_id":entry.get(),"byte_offset":1}))).await.unwrap_err().to_string().contains("UTF-8"));
    assert!(serde_json::from_value::<BotRequest>(json!({"action":"read_text","session_id":session.get(),"entry_id":entry.get(),"content_id":"foreign-stream-bypass"})).is_err());
    let foreign = create(&manager, workspace, "foreign", Default::default()).await;
    assert!(
        manager
            .bot_discovery(
                bot,
                &request(
                    json!({"action":"read_text","session_id":foreign.get(),"entry_id":entry.get()})
                )
            )
            .await
            .is_err()
    );
    let fork = manager
        .kernel()
        .sessions
        .fork(RequestId::new(), session, entry)
        .await
        .unwrap()
        .value
        .id;
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), session)
        .await
        .unwrap();
    let mut offset = 0;
    let mut inherited = String::new();
    loop {
        let result=discover(&manager,bot,json!({"action":"read_text","session_id":fork.get(),"entry_id":entry.get(),"byte_offset":offset,"max_bytes":1001})).await;
        assert_eq!(result["origin_session_id"], session.get());
        assert_eq!(result["session_id"], fork.get());
        assert_eq!(result["url"], link(fork.get()));
        inherited.push_str(result["text"].as_str().unwrap());
        offset = result["next_byte_offset"].as_u64().unwrap();
        if result["has_more"] == false {
            break;
        }
        assert_eq!(result["next"]["session_id"], fork.get());
    }
    assert_eq!(inherited, original);
    let listed = discover(
        &manager,
        bot,
        json!({"action":"sessions","query":"transcript"}),
    )
    .await;
    assert!(
        listed["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["session_id"] == fork.get())
    );
}

#[tokio::test]
async fn read_keeps_user_assistant_tool_errors_and_status_explains_human_blocker() {
    let (manager, bot, workspace) = fixture().await;
    let session = create(&manager, workspace, "blocked project", Default::default()).await;
    let (_, _, round) = text(
        &manager,
        session,
        "Actual assistant response: build failed because test X.",
    )
    .await;
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "call-test".into(),
                name: "shell.run".into(),
                input: json!({"command":"cargo test"}),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Failed,
            value: json!({"error":"exit 101","stderr":"test X failed"}),
        },
    )
    .await
    .unwrap();
    let pending = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "permission-test".into(),
                name: "shell.run".into(),
                input: json!({"command":"deploy"}),
            },
        )
        .await
        .unwrap()
        .value;
    pending
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: "deploy production".into(),
                description: "Human must authorize publishing release".into(),
            },
        )
        .await
        .unwrap();
    let read = discover(
        &manager,
        bot,
        json!({"action":"read","session_id":session.get(),"limit":8}),
    )
    .await;
    let raw = serde_json::to_string(&read).unwrap();
    assert!(raw.contains("actual user question"));
    assert!(raw.contains("Actual assistant response"));
    assert!(raw.contains("test X failed"));
    assert!(raw.contains("Human must authorize"));
    assert!(
        read["entries"]
            .as_array()
            .unwrap()
            .iter()
            .all(|e| e["origin_session_id"] == session.get() && e["url"] == link(session.get()))
    );
    let status = discover(
        &manager,
        bot,
        json!({"action":"status","session_id":session.get()}),
    )
    .await;
    assert!(status["version"].is_i64());
    assert!(status["control_version"].is_i64());
    assert!(status["entry_count"].as_u64().unwrap() > 0);
    assert_eq!(status["state"], "awaiting_interaction");
    assert_eq!(status["active_count"], 1);
    assert_eq!(
        status["pending_interactions"][0]["detail"]["type"],
        "permission"
    );
    assert_eq!(status["pending_interactions"][0]["requires_human"], true);
    assert_eq!(
        status["pending_interactions"][0]["url"],
        link(session.get())
    );
    assert!(
        status["last_response"]["text"]
            .as_str()
            .unwrap()
            .contains("build failed")
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .pending_interactions(session, None, 10)
            .await
            .unwrap()
            .interactions
            .len(),
        1
    );
}

#[tokio::test]
async fn cursor_staleness_and_large_escaped_text_stay_explicit_and_bounded() {
    let (manager, bot, workspace) = fixture().await;
    let session = create(
        &manager,
        workspace,
        "unique cursor title",
        Default::default(),
    )
    .await;
    create(
        &manager,
        workspace,
        "unique cursor title two",
        Default::default(),
    )
    .await;
    let first = discover(
        &manager,
        bot,
        json!({"action":"sessions","query":"unique cursor title","limit":1}),
    )
    .await;
    let anchor = session_id(first["sessions"][0]["session_id"].as_i64().unwrap()).unwrap();
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), anchor)
        .await
        .unwrap();
    assert!(manager.bot_discovery(bot,&request(json!({"action":"sessions","query":"unique cursor title","cursor":first["next_cursor"]}))).await.unwrap_err().to_string().contains("stale"));
    let remaining = if anchor == session {
        create(&manager, workspace, "escaped", Default::default()).await
    } else {
        session
    };
    let (entry, _, _) = text(&manager, remaining, &"\u{1}".repeat(20000)).await;
    let result=discover(&manager,bot,json!({"action":"read_text","session_id":remaining.get(),"entry_id":entry.get(),"max_bytes":16000})).await;
    assert!(serde_json::to_vec(&result).unwrap().len() < RESPONSE_BUDGET);
    assert_eq!(result["has_more"], true);
    let bad = request(json!({"action":"sessions","cursor":"bd1.not-real"}));
    assert!(manager.bot_discovery(bot, &bad).await.is_err());
}

#[tokio::test]
async fn user_and_native_tool_text_are_exact_and_hidden_reasoning_is_omitted() {
    let (manager, bot, workspace) = fixture().await;
    let session = create(&manager, workspace, "long instructions", Default::default()).await;
    let instruction = "中文 user instruction 🦀\n".repeat(700);
    let split = instruction
        .char_indices()
        .map(|(offset, _)| offset)
        .find(|offset| *offset >= 3000)
        .unwrap();
    let queued = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![
                ComposerNode::Text {
                    text: instruction[..split].to_owned(),
                },
                ComposerNode::Text {
                    text: instruction[split..].to_owned(),
                },
            ]),
        )
        .await
        .unwrap()
        .value;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, queued.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_round(RequestId::new(), vec![queued.message_id])
        .await
        .unwrap()
        .value;
    let reasoning = round
        .begin_text(RequestId::new(), true)
        .await
        .unwrap()
        .value;
    reasoning.append("private hidden reasoning").await.unwrap();
    reasoning.finish(RequestId::new()).await.unwrap();
    let tool = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "long-output".into(),
                name: "native_tool".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let output = tool.begin_text(RequestId::new()).await.unwrap().value;
    output
        .append("native streamed tool evidence 🦀")
        .await
        .unwrap();
    output.finish(RequestId::new()).await.unwrap();
    let tool_entry = output.entry_id();
    let tool_text = "tool result 中文🦀\n".repeat(1300);
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: json!({"raw_output":{"payload":{"text":tool_text}}}),
        },
    )
    .await
    .unwrap();
    let plain = round
        .begin_tool(
            RequestId::new(),
            ToolStart {
                call_id: "plain-output".into(),
                name: "native_tool".into(),
                input: json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    plain
        .complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: json!({"raw_output":{"payload":{"text":tool_text}}}),
            },
        )
        .await
        .unwrap();
    let plain_entry = manager
        .kernel()
        .reader
        .tool(session, plain.id())
        .await
        .unwrap()
        .entry_id;
    for (entry, expected) in [
        (queued.message_id, instruction),
        (tool_entry, "native streamed tool evidence 🦀".to_owned()),
        (plain_entry, tool_text),
    ] {
        let mut offset = 0;
        let mut actual = String::new();
        loop {
            let result=discover(&manager,bot,json!({"action":"read_text","session_id":session.get(),"entry_id":entry.get(),"byte_offset":offset,"max_bytes":997})).await;
            actual.push_str(result["text"].as_str().unwrap());
            offset = result["next_byte_offset"].as_u64().unwrap();
            if result["has_more"] == false {
                break;
            }
        }
        assert_eq!(actual, expected);
    }
    let read = discover(
        &manager,
        bot,
        json!({"action":"read","session_id":session.get(),"limit":8}),
    )
    .await;
    let raw = serde_json::to_string(&read).unwrap();
    assert!(!raw.contains("private hidden reasoning"));
    assert!(raw.contains("native streamed tool evidence"));
    assert!(manager.bot_discovery(bot,&request(json!({"action":"read_text","session_id":session.get(),"entry_id":reasoning.entry_id().get()}))).await.is_err());
}

#[tokio::test]
async fn status_keeps_subagent_states_native_execution_and_input_pages() {
    let (manager, bot, workspace) = fixture().await;
    let session = create(&manager, workspace, "paged status", Default::default()).await;
    let (_, _, round) = text(&manager, session, "I need your choice.").await;
    let mut expected = Vec::new();
    for index in 0..3 {
        let child = create(
            &manager,
            workspace,
            "delegated",
            SessionConfiguration {
                parent_session_id: Some(session),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                agent: Some(SessionAgentConfiguration {
                    name: format!("agent_{index}"),
                    status: agena_domain::AgentStatus::Created,
                    started_at_ms: None,
                    finished_at_ms: None,
                    failure: None,
                    limits: Default::default(),
                }),
                ..Default::default()
            },
        )
        .await;
        expected.push(child.get());
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: format!("question-{index}"),
                    name: "ask_user".into(),
                    input: json!({}),
                },
            )
            .await
            .unwrap()
            .value;
        let request:agena_domain::UserInputRequest=serde_json::from_value(json!({"request_id":format!("q-{index}"),"session_id":session.get(),"title":"Choose release","kind":"question","source":"host","questions":[{"question":"Which release should we publish?","options":[{"label":"Stable"},{"label":"Preview"}]}],"created_at":Utc::now()})).unwrap();
        tool.request_interaction(RequestId::new(), InteractionRequest::Input { request })
            .await
            .unwrap();
    }
    let mut cursor = None;
    let mut agents = Vec::new();
    let mut interactions = Vec::new();
    let mut first_cursor = None;
    for _ in 0..8 {
        let result = discover(
            &manager,
            bot,
            json!({"action":"status","session_id":session.get(),"limit":1,"cursor":cursor}),
        )
        .await;
        agents.extend(
            result["subagents"]
                .as_array()
                .unwrap()
                .iter()
                .map(|a| a["session_id"].as_i64().unwrap()),
        );
        for item in result["pending_interactions"].as_array().unwrap() {
            assert_eq!(item["detail"]["type"], "input");
            assert_eq!(
                item["detail"]["questions"][0]["question"],
                "Which release should we publish?"
            );
            interactions.push(item["interaction_id"].clone());
        }
        if result["has_more"] == false {
            break;
        }
        cursor = result["next_cursor"].as_str().map(str::to_owned);
        if first_cursor.is_none() {
            first_cursor = cursor.clone();
        }
    }
    agents.sort();
    expected.sort();
    assert_eq!(agents, expected);
    assert_eq!(interactions.len(), 3);
    assert!(
        manager
            .bot_discovery(
                bot,
                &request(
                    json!({"action":"status","session_id":session.get()+100,"cursor":first_cursor})
                )
            )
            .await
            .is_err()
    );
}

#[tokio::test]
async fn opaque_cursor_rejects_other_database_and_expiration() {
    let (manager, bot, workspace) = fixture().await;
    for _ in 0..2 {
        create(&manager, workspace, "cursor-owner", Default::default()).await;
    }
    let page = discover(
        &manager,
        bot,
        json!({"action":"sessions","query":"cursor-owner","limit":1}),
    )
    .await;
    let token = page["next_cursor"].as_str().unwrap().to_owned();
    let (other, other_bot, _) = fixture().await;
    assert!(
        other
            .bot_discovery(
                other_bot,
                &request(json!({"action":"sessions","query":"cursor-owner","cursor":token}))
            )
            .await
            .unwrap_err()
            .to_string()
            .contains("different")
    );
    {
        let mut registry = continuations().lock().unwrap();
        registry.get_mut(&token).unwrap().created = Instant::now() - Duration::from_secs(3600);
    }
    let error = manager
        .bot_discovery(
            bot,
            &request(json!({"action":"sessions","query":"cursor-owner","cursor":token})),
        )
        .await
        .unwrap_err()
        .to_string();
    assert!(error.contains("expired") || error.contains("stale"));
}
