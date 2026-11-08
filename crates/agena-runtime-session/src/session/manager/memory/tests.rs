use super::*;
use agena_domain::{Model, ModelId, ProviderId};
use agena_plugin_host::{
    ConfiguredPlugin, PluginHostBuildConfig, PluginsConfig, StaticPluginRegistration,
};
use agena_provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionUsage,
};
use agena_runtime_provider::{ProviderError, provider::ModelRuntime};
use agena_superagent::{AssistantPatch, AssistantUpdate, CompanionRepository};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

#[derive(Clone)]
struct MemoryProbe;
#[async_trait::async_trait]
impl agena_plugin_host::sdk::Plugin for MemoryProbe {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        use agena_plugin_host::sdk;
        let mut manifest = sdk::PluginManifest::new("agena", "memory", "test");
        manifest.settings = Some(sdk::macro_support::json_settings_contract());
        manifest.tools = vec![sdk::ToolDefinition {
            name: "write".into(),
            contract: sdk::ToolContract {
                input_schema: serde_json::json!({"type":"object"}),
                output_schema: serde_json::json!({"type":"null"}),
            },
            docs: Default::default(),
            runtime: Default::default(),
            tags: vec![sdk::ToolTag::Mutate, sdk::ToolTag::Filesystem],
        }];
        manifest
    }
    async fn init(
        &self,
        _: agena_plugin_host::sdk::InitContext,
        _: Arc<dyn agena_plugin_host::sdk::HostClient>,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::InitOutcome> {
        Ok(agena_plugin_host::sdk::InitOutcome::ack(self.manifest()))
    }
    async fn tool_invoke(
        &self,
        _: agena_plugin_host::sdk::ToolInvokeInput,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        panic!("memory maintenance only preflights the write capability")
    }
}

#[derive(Default)]
struct ProviderControl {
    calls: AtomicUsize,
    pause: AtomicBool,
    started: Notify,
    release: Notify,
    requests: StdMutex<Vec<serde_json::Value>>,
}
struct MemoryProvider {
    control: Arc<ProviderControl>,
    model: ModelId,
}
#[async_trait::async_trait]
impl ModelRuntime for MemoryProvider {
    fn id(&self) -> &str {
        "memory-lifecycle-test"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        assert!(request.disable_tools);
        assert!(request.max_output_tokens.is_some_and(|n| n <= 1536));
        assert!(
            request
                .system
                .as_deref()
                .is_some_and(|system| system.contains("historical episode"))
        );
        let context: serde_json::Value =
            serde_json::from_str(&request.turns[0].as_text_lossy()).unwrap();
        self.control.requests.lock().unwrap().push(context.clone());
        self.control.calls.fetch_add(1, Ordering::SeqCst);
        self.control.started.notify_one();
        if self.control.pause.load(Ordering::SeqCst) {
            self.control.release.notified().await;
        }
        let source = context["evidence"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["kind"] == "user")
            .unwrap()["entry_id"]
            .clone();
        let draft = serde_json::json!({"title":"Database migration task", "decisions":[{"text":"Use PostgreSQL for the project database.","evidence":[source]}],"outcomes":[],"open_work":[],"lessons":[]});
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: draft.to_string(),
            reasoning_text: None,
            finish_reason: Some(CompletionFinishReason::Stop),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
        })
    }
}

struct Fixture {
    manager: SessionManager,
    session: SessionId,
    workspace: i64,
    preferences: Arc<agena_storage_sqlite::SqliteCompanionRepository>,
    control: Arc<ProviderControl>,
    root: std::path::PathBuf,
    store: MemoryStore,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.manager.request_shutdown();
        let _ = std::fs::remove_dir_all(self.store.dir());
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
impl Fixture {
    async fn new(
        maintenance_enabled: bool,
        min_messages: u64,
        configuration: SessionConfiguration,
    ) -> Self {
        Self::with_cooldown(maintenance_enabled, min_messages, configuration, 0).await
    }
    async fn with_cooldown(
        maintenance_enabled: bool,
        min_messages: u64,
        configuration: SessionConfiguration,
        cooldown: u64,
    ) -> Self {
        let root =
            std::env::temp_dir().join(format!("agena-memory-lifecycle-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let store = MemoryStore::for_workspace(&root);
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let workspace = kernel
            .workspaces
            .ensure_id(&root.to_string_lossy())
            .await
            .unwrap();
        let session = kernel
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Memory lifecycle".into(),
                configuration,
            )
            .await
            .unwrap()
            .value
            .id;
        let preferences = Arc::new(agena_storage_sqlite::SqliteCompanionRepository::new(
            kernel.database.clone(),
        ));
        let mut plugin_config = PluginsConfig::default();
        plugin_config.list.insert("agena.memory".into(), ConfiguredPlugin::static_settings(serde_json::json!({
            "maintenance": {"enabled": maintenance_enabled, "idle_seconds": 5, "min_user_messages":min_messages,"cooldown_seconds":cooldown,"max_output_tokens":1536}
        })));
        let plugins = agena_plugin_host::PluginHost::new(PluginHostBuildConfig {
            static_plugins: vec![StaticPluginRegistration::new(
                "agena.memory".parse().unwrap(),
                MemoryProbe,
            )],
            config: plugin_config,
            workspace_root: root.clone(),
            agena_version: "test".into(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: Default::default(),
        })
        .await
        .unwrap();
        assert_eq!(
            plugins.plugins().len(),
            1,
            "memory fixture plugin failed: {:?}",
            plugins.plugin_statuses()
        );
        let executor = ToolExecutor::new(
            &root,
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            plugins.clone(),
            None,
            None,
        );
        let control = Arc::new(ProviderControl::default());
        let mut providers = ProviderRegistry::new();
        providers.register(MemoryProvider {
            control: control.clone(),
            model: ModelId::new("memory"),
        });
        let manager = SessionManager::new(
            kernel,
            Arc::new(providers),
            ContextGovernor::new(Default::default()),
            SessionProcessor::new(plugins),
            executor,
            RuntimeSessionManagerConfig {
                default_selection: ExecutionSelection {
                    provider: Some("memory-lifecycle-test".into()),
                    model: Some("memory".into()),
                    ..Default::default()
                },
                ..Default::default()
            },
        );
        manager.install_memory_preferences(preferences.clone());
        Self {
            manager,
            session,
            workspace,
            preferences,
            control,
            root,
            store,
        }
    }
    async fn message(&self, text: &str) -> EntryId {
        let execution = self
            .manager
            .kernel()
            .sessions
            .send(
                RequestId::new(),
                self.session,
                ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = self
            .manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), self.session, execution.id)
            .await
            .unwrap()
            .value;
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        execution.message_id
    }
    async fn tool_message(&self, text: &str, name: &str) {
        self.tool_message_with_input(
            text,
            name,
            serde_json::json!({"command":"cargo test migration"}),
        )
        .await;
    }
    async fn tool_message_with_input(&self, text: &str, name: &str, input: serde_json::Value) {
        let execution = self
            .manager
            .kernel()
            .sessions
            .send(
                RequestId::new(),
                self.session,
                ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            )
            .await
            .unwrap()
            .value;
        let attempt = self
            .manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), self.session, execution.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let tool = round
            .begin_tool(
                RequestId::new(),
                ToolStart {
                    call_id: uuid::Uuid::new_v4().to_string(),
                    name: name.into(),
                    input,
                },
            )
            .await
            .unwrap()
            .value;
        tool.complete(
            RequestId::new(),
            ToolResult {
                phase: ToolPhase::Succeeded,
                value: serde_json::json!({"ok":true,"tests_passed":12}),
            },
        )
        .await
        .unwrap();
        round.finish(RequestId::new(), None, true).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    async fn seed(&self) {
        self.message("项目决定使用 PostgreSQL").await;
        self.message("先保留迁移历史，增加针对数据库选择的回归测试")
            .await;
        self.message("继续这个项目，记得迁移决定仍然有效").await;
    }
    fn episode_name(&self) -> String {
        self.manager.memory_record_name("episode", self.session)
    }
    async fn disable_memory(&self) {
        let profile = self.preferences.profile(self.workspace).await.unwrap();
        self.preferences
            .update_profile(
                self.workspace,
                AssistantUpdate {
                    expected_revision: profile.revision,
                    request_key: uuid::Uuid::new_v4().to_string(),
                    patch: AssistantPatch {
                        memory_enabled: Some(false),
                        ..Default::default()
                    },
                },
                Utc::now().timestamp_millis(),
            )
            .await
            .unwrap();
    }
    async fn wait_worker(&self) {
        tokio::time::timeout(Duration::from_secs(12), async {
            loop {
                let dispatch = self.manager.session_dispatch(self.session);
                if !dispatch.memory.lock().unwrap().running {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
    }
}

#[tokio::test]
async fn idle_worker_coalesces_and_persists_source_backed_episode() {
    let fixture = Fixture::new(true, 3, Default::default()).await;
    fixture.seed().await;
    for _ in 0..8 {
        fixture.manager.schedule_memory(fixture.session);
    }
    fixture.wait_worker().await;
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 1);
    let episode = fixture.store.get(&fixture.episode_name()).unwrap();
    assert_eq!(episode.metadata.kind, MemoryKind::Episode);
    assert_eq!(episode.metadata.origin, MemoryOrigin::Derived);
    assert_eq!(episode.metadata.scope, MemoryScope::Workspace);
    assert_eq!(
        episode.metadata.source.as_ref().unwrap().session_id,
        Some(fixture.session.get())
    );
    assert!(episode.metadata.source.as_ref().unwrap().entry_id.is_some());
    assert!(episode.body.contains("entries"));
    assert!(episode.body.contains("PostgreSQL"));
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(
        fixture.control.calls.load(Ordering::SeqCst),
        1,
        "persisted source watermark prevents duplicate extraction"
    );
    fixture.manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn disabled_preferences_and_disabled_maintenance_never_learn() {
    for enabled in [true, false] {
        let fixture = Fixture::new(enabled, 3, Default::default()).await;
        fixture.seed().await;
        if enabled {
            fixture.disable_memory().await;
        }
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
        assert!(!fixture.store.dir().exists());
    }
}

#[tokio::test]
async fn short_conversation_and_denied_memory_capability_do_not_call_model() {
    let short = Fixture::new(true, 1, Default::default()).await;
    short.message("你好").await;
    short.manager.maintain_memory(short.session).await.unwrap();
    assert_eq!(short.control.calls.load(Ordering::SeqCst), 0);
    let denied = Fixture::new(
        true,
        1,
        SessionConfiguration {
            capability_denied_tool_names: ["memory.write".into()].into_iter().collect(),
            ..Default::default()
        },
    )
    .await;
    denied.seed().await;
    denied
        .manager
        .maintain_memory(denied.session)
        .await
        .unwrap();
    assert_eq!(denied.control.calls.load(Ordering::SeqCst), 0);
    assert!(!denied.store.dir().exists());
}

#[tokio::test]
async fn profile_disable_during_model_call_discards_result() {
    let fixture = Fixture::new(true, 3, Default::default()).await;
    fixture.seed().await;
    fixture.control.pause.store(true, Ordering::SeqCst);
    let manager = fixture.manager.clone();
    let session = fixture.session;
    let work = tokio::spawn(async move { manager.maintain_memory(session).await });
    tokio::time::timeout(Duration::from_secs(5), fixture.control.started.notified())
        .await
        .unwrap();
    fixture.disable_memory().await;
    fixture.control.release.notify_one();
    work.await.unwrap().unwrap();
    assert!(matches!(
        fixture.store.get(&fixture.episode_name()),
        Err(agena_storage::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn new_user_input_during_extraction_discards_old_task_snapshot() {
    let fixture = Fixture::new(true, 3, Default::default()).await;
    fixture.seed().await;
    fixture.control.pause.store(true, Ordering::SeqCst);
    let manager = fixture.manager.clone();
    let session = fixture.session;
    let work = tokio::spawn(async move { manager.maintain_memory(session).await });
    tokio::time::timeout(Duration::from_secs(5), fixture.control.started.notified())
        .await
        .unwrap();
    fixture
        .message("纠正：这个方案已经取消，等待重新选型")
        .await;
    fixture.control.release.notify_one();
    work.await.unwrap().unwrap();
    assert!(matches!(
        fixture.store.get(&fixture.episode_name()),
        Err(agena_storage::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn forgotten_episode_is_not_resurrected_by_in_flight_or_future_extraction() {
    let fixture = Fixture::new(true, 3, Default::default()).await;
    fixture.seed().await;
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    fixture.message("继续优化迁移流程").await;
    fixture.control.pause.store(true, Ordering::SeqCst);
    // Consume the permit left by the first completed model call.
    fixture.control.started.notified().await;
    let manager = fixture.manager.clone();
    let session = fixture.session;
    let work = tokio::spawn(async move { manager.maintain_memory(session).await });
    tokio::time::timeout(Duration::from_secs(5), fixture.control.started.notified())
        .await
        .unwrap();
    let record = fixture.store.get(&fixture.episode_name()).unwrap();
    fixture
        .store
        .forget_checked(&fixture.episode_name(), Some(&record.sha256))
        .unwrap();
    fixture.control.release.notify_one();
    assert_eq!(
        work.await.unwrap().unwrap(),
        MaintenanceOutcome::Complete,
        "durable source suppression discards a forgotten result before publication"
    );
    fixture.control.pause.store(false, Ordering::SeqCst);
    let calls = fixture.control.calls.load(Ordering::SeqCst);
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), calls);
    assert!(matches!(
        fixture.store.get(&fixture.episode_name()),
        Err(agena_storage::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn committed_compaction_becomes_session_only_task_memory_without_model_call() {
    let fixture = Fixture::new(true, 100, Default::default()).await;
    let through = fixture
        .message("目标：完成数据库迁移；下一步验证回归测试")
        .await;
    let execution = fixture
        .manager
        .kernel()
        .sessions
        .start_control_execution(
            RequestId::new(),
            fixture.session,
            ControlExecutionPurpose::Compaction,
        )
        .await
        .unwrap()
        .value;
    let attempt = fixture
        .manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), fixture.session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_compaction(RequestId::new(), 128)
        .await
        .unwrap()
        .value;
    let output = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    output.append("Accepted: PostgreSQL. Pending: validate migration tests. No test execution has been verified yet.").await.unwrap();
    let mut native = crate::session::kernel::KernelProviderState::new(
        agena_domain::ModelRef::try_new("memory-lifecycle-test", "memory").unwrap(),
        serde_json::Value::Null,
    );
    native.compaction = Some(agena_provider::ProviderCompactionContext::OpenAiResponses {
        items: vec![
            serde_json::json!({"type":"compaction","encrypted_content":"opaque-native-state-must-stay-in-kernel"}),
        ],
    });
    let checkpoint = round
        .commit_compaction(
            RequestId::new(),
            output.id(),
            through,
            Some(serde_json::to_value(native).unwrap()),
        )
        .await
        .unwrap()
        .value;
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    let task = fixture
        .store
        .get(&fixture.manager.memory_record_name("task", fixture.session))
        .unwrap();
    assert_eq!(task.metadata.kind, MemoryKind::Task);
    assert_eq!(
        task.metadata.source.as_ref().unwrap().entry_id,
        Some(checkpoint.get())
    );
    assert!(task.metadata.scope.includes_context(
        Some(fixture.session.get()),
        Some(fixture.manager.memory_instance_id())
    ));
    assert!(!task.metadata.scope.includes_context(
        Some(fixture.session.get() + 1),
        Some(fixture.manager.memory_instance_id())
    ));
    assert!(
        !task
            .metadata
            .scope
            .includes_context(Some(fixture.session.get()), Some("another-database"))
    );
    assert!(task.body.contains("Pending: validate migration tests"));
    assert!(!task.body.contains("opaque-native-state"));
    assert!(!task.body.contains("encrypted_content"));
}

#[tokio::test]
async fn older_cited_decisions_are_rehydrated_outside_the_recent_message_window() {
    let fixture = Fixture::new(true, 3, Default::default()).await;
    let first = fixture.message("任务目标：数据库迁移").await;
    let decision = fixture
        .message("已经确定 PostgreSQL，原因是事务一致性需求")
        .await;
    for _ in 0..15 {
        fixture.message("继续检查独立模块").await;
    }
    let name = fixture.episode_name();
    let snapshot = fixture.store.write_snapshot(&name).unwrap();
    let previous = fixture
        .store
        .save_derived(
            NewMemory {
                name: name.clone(),
                description: "Earlier decision".into(),
                memory_type: Some(MemoryType::Project),
                body: format!(
                    "- Keep PostgreSQL [session {}; entries {}]\n",
                    fixture.session.get(),
                    decision.get()
                ),
                index_line: None,
            },
            &snapshot,
            MemoryWriteOptions {
                kind: Some(MemoryKind::Episode),
                source: Some(MemorySource {
                    session_id: Some(fixture.session.get()),
                    instance_id: Some(fixture.manager.memory_instance_id().to_owned()),
                    entry_id: Some(decision.get()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    let context = fixture
        .manager
        .memory_evidence(fixture.session, Some(&previous))
        .await
        .unwrap()
        .unwrap();
    assert!(
        context
            .evidence
            .iter()
            .any(|source| source.entry_id == first.get())
    );
    assert!(
        context
            .evidence
            .iter()
            .any(|source| source.entry_id == decision.get() && source.text.contains("事务一致性"))
    );
}

#[test]
fn episode_requires_real_sources_and_tool_evidence_for_observed_outcomes() {
    let session = SessionId::try_from(1).unwrap();
    let evidence = vec![
        Evidence {
            entry_id: 10,
            source_session_id: 1,
            revision: 1,
            kind: EvidenceKind::User,
            text: "Use PostgreSQL".into(),
        },
        Evidence {
            entry_id: 11,
            source_session_id: 1,
            revision: 1,
            kind: EvidenceKind::Assistant,
            text: "All tests passed".into(),
        },
    ];
    assert!(
        parse_episode(
            r#"{"title":"Task","decisions":[{"text":"Use PostgreSQL","evidence":[999]}]}"#,
            session,
            &evidence
        )
        .is_err()
    );
    assert!(
        parse_episode(
            r#"{"title":"Task","outcomes":[{"text":"All tests passed","evidence":[11]}]}"#,
            session,
            &evidence
        )
        .is_err()
    );
    assert!(
        parse_episode(
            r#"{"title":"Task","decisions":[{"text":"Use PostgreSQL","evidence":[10]}]}"#,
            session,
            &evidence
        )
        .unwrap()
        .unwrap()
        .1
        .contains("entries 10")
    );
    assert!(
        parse_episode(r#"{"title":"No reusable experience"}"#, session, &evidence)
            .unwrap()
            .is_none()
    );
}

#[test]
fn excerpts_and_source_parsing_are_bounded_and_unicode_safe() {
    assert_eq!(truncate("数据库选型", 10), "数据库");
    let value = serde_json::json!({"text":"长文本".repeat(5000)});
    assert!(bounded_json(&value, 64).len() < 90);
    let session = SessionId::try_from(7).unwrap();
    assert_eq!(
        previous_sources(
            "- One [session 7; entries 3, 9]\n- Wrong [session 8; entries 6]\n- Two [session 7; entries 9, 12]",
            session
        ),
        vec![3, 9, 12]
    );
}

#[tokio::test]
async fn one_prompt_with_observed_tool_work_is_a_useful_episode() {
    let fixture = Fixture::new(true, 1, Default::default()).await;
    fixture
        .tool_message("数据库迁移使用 PostgreSQL 并执行测试", "agena.shell.exec")
        .await;
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 1);
    assert_eq!(
        fixture
            .store
            .get(&fixture.episode_name())
            .unwrap()
            .metadata
            .kind,
        MemoryKind::Episode
    );
}

#[tokio::test]
async fn memory_tool_echo_is_not_evidence_and_forget_suppresses_extraction() {
    let fixture = Fixture::new(true, 1, Default::default()).await;
    fixture
        .tool_message_with_input(
            "回忆数据库选择",
            "tools_call",
            serde_json::json!({"tool":"memory.get", "input":{"name":"previous-choice"}}),
        )
        .await;
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    let evidence = fixture
        .manager
        .memory_evidence(fixture.session, None)
        .await
        .unwrap()
        .unwrap();
    assert!(
        !evidence
            .evidence
            .iter()
            .any(|source| source.kind == EvidenceKind::ToolResult)
    );
    fixture.message("之前提到的选择需要删除").await;
    fixture
        .tool_message_with_input(
            "忘记之前的数据库选择",
            "tools_call",
            serde_json::json!({"tool":"memory.delete", "input":{"name":"previous-choice"}}),
        )
        .await;
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture.store.get(&fixture.episode_name()),
        Err(agena_storage::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn cooldown_retains_the_final_turn_until_eligible_without_another_event() {
    let fixture = Fixture::with_cooldown(true, 1, Default::default(), 8).await;
    fixture.seed().await;
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    let final_entry = fixture
        .message("最终迁移结果已经确认，下一步保留验证记录")
        .await;
    assert!(matches!(
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap(),
        MaintenanceOutcome::Defer(_)
    ));
    fixture.manager.schedule_memory(fixture.session);
    fixture.wait_worker().await;
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .store
            .get(&fixture.episode_name())
            .unwrap()
            .metadata
            .source
            .unwrap()
            .entry_id,
        Some(final_entry.get())
    );
    fixture.manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn unrelated_memory_mutation_retries_fresh_evidence_without_losing_episode() {
    let fixture = Fixture::new(true, 1, Default::default()).await;
    fixture.seed().await;
    fixture.control.pause.store(true, Ordering::SeqCst);
    fixture.manager.schedule_memory(fixture.session);
    tokio::time::timeout(Duration::from_secs(8), fixture.control.started.notified())
        .await
        .unwrap();
    fixture
        .store
        .save(NewMemory {
            name: "unrelated-project-fact".into(),
            description: "Explicit user correction elsewhere".into(),
            memory_type: Some(MemoryType::Project),
            body: "Use the updated build command.".into(),
            index_line: None,
        })
        .unwrap();
    fixture.control.pause.store(false, Ordering::SeqCst);
    fixture.control.release.notify_one();
    fixture.wait_worker().await;
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 2);
    assert!(fixture.store.get(&fixture.episode_name()).is_ok());
    assert_eq!(
        fixture
            .store
            .get("unrelated-project-fact")
            .unwrap()
            .metadata
            .origin,
        MemoryOrigin::Explicit
    );
    fixture.manager.shutdown().await.unwrap();
}

#[tokio::test]
async fn ask_and_denied_tool_policy_skip_automatic_learning() {
    for mode in [
        agena_domain::PermissionMode::Ask,
        agena_domain::PermissionMode::Deny,
    ] {
        let fixture = Fixture::new(
            true,
            1,
            SessionConfiguration {
                permission: agena_domain::PermissionConfig {
                    tools: Some(agena_domain::ToolPermissionConfig {
                        names: [("agena.memory.write".to_owned(), mode)]
                            .into_iter()
                            .collect(),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            },
        )
        .await;
        fixture.seed().await;
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
        assert!(!fixture.store.dir().exists());
    }
}

#[derive(Debug)]
struct MemoryChannelVerifier(Arc<AtomicBool>);
#[async_trait::async_trait]
impl agena_runtime_contracts::ChannelAccessVerifier for MemoryChannelVerifier {
    async fn authorize(&self, authority: &agena_domain::ChannelAuthority) -> Result<(), String> {
        if self.0.load(Ordering::SeqCst) && authority.active_at(Utc::now().timestamp_millis()) {
            Ok(())
        } else {
            Err("channel membership revoked".into())
        }
    }
}

#[tokio::test]
async fn private_channel_learning_uses_its_authorized_store_and_rechecks_revocation() {
    for enabled in [false, true] {
        let private_root = std::env::temp_dir().join(format!(
            "agena-memory-lifecycle-private-{}",
            uuid::Uuid::new_v4()
        ));
        let private_store = MemoryStore::for_workspace(&private_root);
        let authority = agena_domain::ChannelAuthority {
            binding_id: "memory-channel-test".into(),
            epoch: 1,
            actor: "member".into(),
            role: agena_domain::ChannelRole::Member,
            identity: agena_domain::ChannelIdentity {
                tenant: "test-tenant".into(),
                account: "test-account".into(),
                kind: agena_domain::ChannelKind::Direct,
            },
            resource: "private-conversation".into(),
            thread: None,
            session_mode: agena_domain::ChannelSessionMode::PerUser,
            grant: agena_domain::ChannelGrant {
                private_memory: enabled,
                ..Default::default()
            },
            expires_at_ms: i64::MAX,
            memory_workspace_root: private_root.clone(),
            private_paths: [private_store.dir().to_path_buf()].into_iter().collect(),
            revoked: false,
        };
        let fixture = Fixture::new(
            true,
            1,
            SessionConfiguration {
                channel_authority: Some(authority),
                ..Default::default()
            },
        )
        .await;
        let active = Arc::new(AtomicBool::new(true));
        fixture
            .manager
            .install_channel_verifier(Arc::new(MemoryChannelVerifier(active.clone())));
        fixture.seed().await;
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert!(
            !fixture.store.dir().exists(),
            "channel memory must never enter the source workspace"
        );
        if enabled {
            assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 1);
            let before = private_store.get(&fixture.episode_name()).unwrap();
            fixture.message("私有任务的后续步骤").await;
            active.store(false, Ordering::SeqCst);
            assert!(
                fixture
                    .manager
                    .maintain_memory(fixture.session)
                    .await
                    .is_err()
            );
            assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 1);
            assert_eq!(
                private_store.get(&fixture.episode_name()).unwrap().sha256,
                before.sha256
            );
        } else {
            assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
            assert!(!private_store.dir().exists());
        }
        let _ = std::fs::remove_dir_all(private_store.dir());
        let _ = std::fs::remove_dir_all(&private_root);
    }
}

#[tokio::test]
async fn shutdown_cancels_pending_idle_maintenance_without_a_model_call() {
    let fixture = Fixture::new(true, 1, Default::default()).await;
    fixture.seed().await;
    fixture.manager.schedule_memory(fixture.session);
    fixture.manager.shutdown().await.unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    assert!(!fixture.store.dir().exists());
}

#[tokio::test]
async fn remembered_rules_follow_the_owning_session_workspace() {
    let mut fixture = Fixture::new(true, 1, Default::default()).await;
    let original = fixture.session;
    let other_root = fixture.root.join("other-workspace");
    let workspace = fixture
        .manager
        .kernel_runtime
        .workspaces
        .ensure_id(&other_root.to_string_lossy())
        .await
        .unwrap();
    let session = fixture
        .manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Other workspace".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let state = fixture.manager.state.load_full();
    let context = fixture
        .manager
        .execution_context_with_state(session, &state)
        .await
        .unwrap();
    let executor = state.executor.for_session_context_async(&context).await;
    let checks = executor
        .collect_permission_checks_for_invocation_in_session(
            &agena_domain::ToolInvocation::new("agena.memory.write", Default::default()),
            Some(session.get()),
        )
        .await
        .unwrap();
    let check = checks.into_iter().next().unwrap();
    let action_key = super::super::execution::permission_key(&check.action).unwrap();
    fixture
        .manager
        .kernel_runtime
        .permissions
        .upsert(&agena_storage::PersistedPermissionRule {
            id: None,
            created_at_ms: None,
            updated_at_ms: None,
            action_key,
            mode: agena_domain::PermissionMode::Deny,
            scope: agena_domain::PermissionScope::Workspace,
            session_id: None,
            workspace_id: Some(workspace),
            source: "memory-workspace-policy-test".into(),
            reason: Some("Automatic writes are disabled in this workspace".into()),
            operator: None,
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        })
        .await
        .unwrap();
    assert!(matches!(
        fixture
            .manager
            .query_host_permission(Some(session.get()), check.clone())
            .await
            .unwrap(),
        PermissionDecision::Deny { .. }
    ));
    assert!(matches!(
        fixture
            .manager
            .query_host_permission(Some(original.get()), check.clone())
            .await
            .unwrap(),
        PermissionDecision::Allow
    ));
    assert!(matches!(
        fixture
            .manager
            .query_host_permission(None, check)
            .await
            .unwrap(),
        PermissionDecision::Allow
    ));
    fixture.session = session;
    fixture.seed().await;
    fixture.manager.maintain_memory(session).await.unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    assert!(!MemoryStore::for_workspace(&other_root).dir().exists());
}

#[tokio::test]
async fn durable_source_suppression_blocks_new_names_outside_recent_history() {
    let fixture = Fixture::new(true, 1, Default::default()).await;
    let entry = fixture.message("暂存数据库选型，之后可能撤销").await;
    let source = MemorySource {
        session_id: Some(fixture.session.get()),
        instance_id: Some(fixture.manager.memory_instance_id().to_owned()),
        entry_id: Some(entry.get()),
        ..Default::default()
    };
    let original = fixture
        .store
        .save_with_options(
            NewMemory {
                name: "explicit-database-choice".into(),
                description: "A decision the user later removed".into(),
                memory_type: Some(MemoryType::Project),
                body: "Use PostgreSQL for this project.".into(),
                index_line: None,
            },
            None,
            MemoryWriteOptions {
                source: Some(source.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    fixture
        .store
        .forget_checked("explicit-database-choice", Some(&original.sha256))
        .unwrap();
    // The deletion can come from UI and need not appear as a transcript tool.
    // Even after every old entry leaves the bounded recent window, its source
    // session cannot mint another derived name containing the removed material.
    for _ in 0..16 {
        fixture.message("继续处理其他模块").await;
    }
    assert!(fixture.store.source_is_suppressed(&source).unwrap());
    fixture
        .manager
        .maintain_memory(fixture.session)
        .await
        .unwrap();
    assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
    assert!(matches!(
        fixture.store.get(&fixture.episode_name()),
        Err(agena_storage::MemoryError::NotFound(_))
    ));
}

#[tokio::test]
async fn pending_memory_recovery_authorizes_every_actual_target_before_mutation() {
    for deny_target in [false, true] {
        let fixture = Fixture::new(true, 100, Default::default()).await;
        fixture.message("需要恢复之前的持久记录").await;
        let original = fixture
            .store
            .save(NewMemory {
                name: "recoverable-note".into(),
                description: "Previously committed note".into(),
                memory_type: Some(MemoryType::Project),
                body: "The committed version must survive interruption.".into(),
                index_line: None,
            })
            .unwrap();
        let journal = serde_json::json!({
            "schema_version": 1,
            "entries": [
                {"name":"recoverable-note.md","previous":std::fs::read_to_string(&original.path).unwrap()},
                {"name":"MEMORY.md","previous":std::fs::read_to_string(fixture.store.dir().join("MEMORY.md")).unwrap()},
                {"name":".memory-state.json","previous":std::fs::read_to_string(fixture.store.metadata_path()).unwrap()}
            ]
        });
        std::fs::write(
            fixture.store.dir().join(".memory-transaction.json"),
            serde_json::to_vec(&journal).unwrap(),
        )
        .unwrap();
        std::fs::write(&original.path, "uncommitted partial document").unwrap();
        std::fs::write(fixture.store.dir().join("MEMORY.md"), "uncommitted index").unwrap();
        std::fs::write(fixture.store.metadata_path(), "uncommitted state").unwrap();
        if deny_target {
            let state = fixture.manager.state.load_full();
            let mut config = state.config.clone();
            config.permission.path = Some(agena_domain::PathPermissionConfig {
                rules: [(
                    original.path.to_string_lossy().into_owned(),
                    agena_domain::PathAccessModes {
                        read: Some(agena_domain::PermissionMode::Allow),
                        write: Some(agena_domain::PermissionMode::Deny),
                    },
                )]
                .into_iter()
                .collect(),
                ..Default::default()
            });
            fixture.manager.reconfigure(
                state.providers.clone(),
                state.governor.clone(),
                state.processor.clone(),
                state.executor.clone(),
                config,
            );
        }
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
        if deny_target {
            assert!(
                fixture
                    .store
                    .dir()
                    .join(".memory-transaction.json")
                    .exists()
            );
            assert_eq!(
                std::fs::read_to_string(&original.path).unwrap(),
                "uncommitted partial document"
            );
        } else {
            assert!(
                !fixture
                    .store
                    .dir()
                    .join(".memory-transaction.json")
                    .exists()
            );
            assert_eq!(
                fixture.store.get("recoverable-note").unwrap().sha256,
                original.sha256
            );
        }
    }
}

async fn forget_known_source(fixture: &Fixture, source_session: SessionId, entry: EntryId) {
    let record = fixture
        .store
        .save_with_options(
            NewMemory {
                name: "parent-source-fact".into(),
                description: "Parent decision subsequently forgotten".into(),
                memory_type: Some(MemoryType::Project),
                body: "Earlier parent task selected PostgreSQL.".into(),
                index_line: None,
            },
            None,
            MemoryWriteOptions {
                source: Some(MemorySource {
                    session_id: Some(source_session.get()),
                    instance_id: Some(fixture.manager.memory_instance_id().to_owned()),
                    entry_id: Some(entry.get()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        )
        .unwrap();
    fixture
        .store
        .forget_checked("parent-source-fact", Some(&record.sha256))
        .unwrap();
}

async fn checkpoint_for_fixture(fixture: &Fixture, through: EntryId) {
    let execution = fixture
        .manager
        .kernel()
        .sessions
        .start_control_execution(
            RequestId::new(),
            fixture.session,
            ControlExecutionPurpose::Compaction,
        )
        .await
        .unwrap()
        .value;
    let attempt = fixture
        .manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), fixture.session, execution.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_compaction(RequestId::new(), 128)
        .await
        .unwrap()
        .value;
    let text = round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    text.append("Task checkpoint retaining the earlier PostgreSQL choice and current follow-up.")
        .await
        .unwrap();
    round
        .commit_compaction(RequestId::new(), text.id(), through, None)
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
}

#[tokio::test]
async fn inherited_fork_and_rewind_history_cannot_relabel_a_forgotten_source() {
    for rewind in [false, true] {
        let mut fixture = Fixture::new(true, 1, Default::default()).await;
        let parent = fixture.session;
        let original = fixture.message("父任务选择 PostgreSQL").await;
        fixture.message("父任务还有其他约束").await;
        let through = fixture.message("此处创建分支").await;
        forget_known_source(&fixture, parent, original).await;
        fixture.session = if rewind {
            fixture
                .manager
                .kernel()
                .sessions
                .rewind(RequestId::new(), parent, through)
                .await
                .unwrap()
                .value
                .session
                .id
        } else {
            fixture
                .manager
                .kernel()
                .sessions
                .fork(RequestId::new(), parent, through)
                .await
                .unwrap()
                .value
                .id
        };
        let config = fixture
            .manager
            .kernel()
            .reader
            .configuration(fixture.session)
            .await
            .unwrap();
        assert!(
            config.conversation.is_none(),
            "ordinary branches are not side conversations"
        );
        assert!(!config.relation_kind.is_subagent());
        drop(config);
        let mut latest = through;
        for _ in 0..16 {
            latest = fixture.message("分支内继续独立执行后续步骤").await;
        }
        let recent = fixture
            .manager
            .kernel()
            .reader
            .message_history(fixture.session, None, 12, 24)
            .await
            .unwrap();
        assert!(
            recent
                .entries
                .iter()
                .all(|entry| entry.origin_session_id == fixture.session)
        );
        drop(recent);
        // Even a newly generated checkpoint belongs to the branch while its
        // summary can still contain the inherited, suppressed parent material.
        checkpoint_for_fixture(&fixture, latest).await;
        assert!(
            !fixture
                .manager
                .memory_source_is_suppressed(fixture.session, &fixture.store)
                .await
                .unwrap(),
            "the new session identity alone does not carry the parent's suppression"
        );
        assert!(
            fixture
                .manager
                .memory_evidence(fixture.session, None)
                .await
                .unwrap()
                .is_none(),
            "mixed-origin evidence must not be relabeled with a single current session"
        );
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 0);
        for kind in ["episode", "task"] {
            assert!(matches!(
                fixture
                    .store
                    .get(&fixture.manager.memory_record_name(kind, fixture.session)),
                Err(agena_storage::MemoryError::NotFound(_))
            ));
        }
    }
}

#[tokio::test]
async fn independent_children_and_empty_rewinds_keep_own_learning_available() {
    for empty_rewind in [false, true] {
        let mut fixture = Fixture::new(true, 1, Default::default()).await;
        let parent = fixture.session;
        let first = fixture.message("父会话的旧决定").await;
        forget_known_source(&fixture, parent, first).await;
        fixture.session = if empty_rewind {
            fixture
                .manager
                .kernel()
                .sessions
                .rewind(RequestId::new(), parent, first)
                .await
                .unwrap()
                .value
                .session
                .id
        } else {
            fixture
                .manager
                .kernel()
                .sessions
                .create_configured(
                    RequestId::new(),
                    fixture.workspace,
                    "Independent child".into(),
                    SessionConfiguration {
                        parent_session_id: Some(parent),
                        relation_kind: agena_domain::SessionRelationKind::Child,
                        ..Default::default()
                    },
                )
                .await
                .unwrap()
                .value
                .id
        };
        fixture
            .tool_message(
                "新任务由当前用户独立指定 PostgreSQL 并完成验证",
                "agena.shell.exec",
            )
            .await;
        fixture
            .manager
            .maintain_memory(fixture.session)
            .await
            .unwrap();
        assert_eq!(fixture.control.calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            fixture
                .store
                .get(&fixture.episode_name())
                .unwrap()
                .metadata
                .source
                .unwrap()
                .session_id,
            Some(fixture.session.get())
        );
    }
}
