//! Production manager contracts: waiter lifetime, frozen queued inputs,
//! cancellation fences, independent child settlement and durable deliveries.
use super::*;
use crate::provider::{ModelRuntime, ProviderError};
use agena_domain::{ModelId, ProviderId, SubtaskStatus};
use agena_provider::{
    CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
    PromptCacheShape, ProviderCompactionOutput,
};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};
use tokio::sync::{mpsc, oneshot};

type Event = std::result::Result<CompletionStreamEvent, ProviderError>;
type CompletionEventStream = std::pin::Pin<Box<dyn futures_util::Stream<Item = Event> + Send>>;
struct Provider {
    model: ModelId,
    streams: StdMutex<VecDeque<mpsc::UnboundedReceiver<Event>>>,
    requests: StdMutex<Vec<CompletionRequest>>,
    called: Notify,
    summary:
        StdMutex<Option<oneshot::Receiver<std::result::Result<CompletionResponse, ProviderError>>>>,
    summary_calls: AtomicUsize,
    native_calls: AtomicUsize,
    unchanged: std::sync::atomic::AtomicBool,
    input_limit: AtomicUsize,
    checkpoint_mode: AtomicUsize,
}
#[async_trait::async_trait]
impl ModelRuntime for Provider {
    fn id(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn model_metadata(&self, _: &ModelId) -> agena_domain::ModelMetadata {
        let limit = self.input_limit.load(Ordering::SeqCst);
        agena_domain::ModelMetadata {
            limits: agena_domain::ModelTokenLimits {
                max_input_tokens: (limit > 0).then_some(limit as u32),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    fn model_capabilities(&self, _: &ModelId) -> agena_domain::ModelCapabilities {
        agena_domain::ModelCapabilities {
            image_input: agena_domain::CapabilitySupport::Supported,
            ..Default::default()
        }
    }
    fn prompt_cache_shape(&self, _: &ModelId) -> Option<PromptCacheShape> {
        Some(PromptCacheShape::from_fields(
            "fixture",
            [("endpoint", "controlled")],
        ))
    }
    async fn list_models(&self) -> std::result::Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        _: CompletionRequest,
    ) -> std::result::Result<CompletionResponse, ProviderError> {
        self.summary_calls.fetch_add(1, Ordering::SeqCst);
        self.called.notify_waiters();
        let receiver = self.summary.lock().unwrap().take();
        match receiver {
            Some(receiver) => receiver.await.expect("controlled compactor sender"),
            None => Err(ProviderError::Config("synthetic compaction failure".into())),
        }
    }
    async fn compact_conversation(
        &self,
        _: CompletionRequest,
    ) -> std::result::Result<Option<ProviderCompactionOutput>, ProviderError> {
        self.native_calls.fetch_add(1, Ordering::SeqCst);
        let mode = self.checkpoint_mode.load(Ordering::SeqCst);
        if mode > 0 {
            let mut value = serde_json::json!({"type":"compaction","encrypted_content":" OPAQUE+/==\n\u{0000}","future":{"values":[null,false,7]}});
            if mode == 2 {
                value["summary"] = serde_json::json!([{"type":"summary_text","text":"portable instructions from native endpoint"}]);
            }
            return Ok(Some(ProviderCompactionOutput::OpenAiResponses {
                items: vec![value],
            }));
        }
        Ok(self
            .unchanged
            .load(Ordering::SeqCst)
            .then_some(ProviderCompactionOutput::OpenAiResponses { items: Vec::new() }))
    }
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionEventStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        self.called.notify_waiters();
        let receiver = self
            .streams
            .lock()
            .unwrap()
            .pop_front()
            .expect("provider call must have one controlled stream");
        Ok(Box::pin(futures_util::stream::unfold(
            (receiver, false),
            |(mut receiver, ended)| async move {
                if ended {
                    return None;
                }
                receiver.recv().await.map(|event| {
                    let ended = matches!(&event, Ok(CompletionStreamEvent::Completed { .. }));
                    (event, (receiver, ended))
                })
            },
        )))
    }
}
struct Fixture {
    manager: SessionManager,
    session: Session,
    provider: Arc<Provider>,
    events: Vec<mpsc::UnboundedSender<Event>>,
    _root: tempfile::TempDir,
}
struct EffectPlugin(Arc<AtomicUsize>);
struct ApiPlugin;
struct StartPatchPlugin;
struct StopPlugin {
    calls: Arc<AtomicUsize>,
    follow_up_once: bool,
}
#[async_trait::async_trait]
impl agena_plugin_host::sdk::Plugin for StopPlugin {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest = agena_plugin_host::sdk::PluginManifest::new("fixture", "stop", "1.0.0");
        manifest.hooks = agena_plugin_host::sdk::HookSubscription::AGENT_STOP;
        manifest
    }
    async fn agent_stop(
        &self,
        input: agena_plugin_host::sdk::AgentStopInput,
    ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::sdk::AgentStopPatch>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        assert!(input.run_error.is_none());
        Ok(
            (self.follow_up_once && call == 0).then(|| agena_plugin_host::sdk::AgentStopPatch {
                continue_with_message: Some("finish the stop-hook follow-up".into()),
                reason: Some("controlled continuation".into()),
            }),
        )
    }
}
fn tool_definition(name: &str) -> agena_plugin_host::sdk::ToolDefinition {
    agena_plugin_host::sdk::ToolDefinition {
        name: name.into(),
        contract: agena_plugin_host::sdk::ToolContract {
            input_schema: serde_json::json!({"type":"object"}),
            ..Default::default()
        },
        docs: agena_plugin_host::sdk::ToolDocs {
            summary: Some("Controlled effect.".into()),
            ..Default::default()
        },
        runtime: Default::default(),
        tags: vec![agena_plugin_host::sdk::ToolTag::ReadOnly],
    }
}
#[async_trait::async_trait]
impl agena_plugin_host::sdk::Plugin for EffectPlugin {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest =
            agena_plugin_host::sdk::PluginManifest::new("fixture", "effect", "1.0.0");
        manifest.tools = vec![tool_definition("run")];
        manifest
    }
    async fn tool_invoke(
        &self,
        _: agena_plugin_host::sdk::ToolInvokeInput,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(agena_plugin_host::sdk::ToolInvokeOutput::text(
            "one external effect",
        ))
    }
}
#[async_trait::async_trait]
impl agena_plugin_host::sdk::Plugin for ApiPlugin {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest = agena_plugin_host::sdk::PluginManifest::new("agena", "tools", "1.0.0");
        manifest.tools = vec![tool_definition("help")];
        manifest
    }
}
#[async_trait::async_trait]
impl agena_plugin_host::sdk::Plugin for StartPatchPlugin {
    fn manifest(&self) -> agena_plugin_host::sdk::PluginManifest {
        let mut manifest =
            agena_plugin_host::sdk::PluginManifest::new("fixture", "start_patch", "1.0.0");
        manifest.hooks = agena_plugin_host::sdk::HookSubscription::SESSION_START;
        manifest
    }

    async fn session_start(
        &self,
        _: agena_plugin_host::sdk::SessionStartInput,
    ) -> agena_plugin_host::sdk::Result<Option<agena_plugin_host::sdk::SessionStartPatch>> {
        Ok(Some(agena_plugin_host::sdk::SessionStartPatch {
            additional_context: Some("shared startup context".into()),
            initial_user_message: Some("shared startup message".into()),
        }))
    }
}
impl Fixture {
    async fn new(streams: usize) -> Self {
        Self::with_effect(streams, None).await
    }
    async fn with_effect(streams: usize, effect: Option<Arc<AtomicUsize>>) -> Self {
        Self::with_plugins(streams, effect, false).await
    }
    async fn with_plugins(
        streams: usize,
        effect: Option<Arc<AtomicUsize>>,
        start_patch: bool,
    ) -> Self {
        Self::with_controls(streams, effect, start_patch, None).await
    }
    async fn with_stop(streams: usize, calls: Arc<AtomicUsize>, follow_up_once: bool) -> Self {
        Self::with_controls(
            streams,
            None,
            false,
            Some(StopPlugin {
                calls,
                follow_up_once,
            }),
        )
        .await
    }
    async fn with_controls(
        streams: usize,
        effect: Option<Arc<AtomicUsize>>,
        start_patch: bool,
        stop: Option<StopPlugin>,
    ) -> Self {
        let root = tempfile::tempdir().unwrap();
        let workspace = root.path().canonicalize().unwrap();
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime(&format!(
                "sqlite://{}?mode=rwc",
                workspace.join("sessions.db").display()
            ))
            .await
            .unwrap(),
        );
        let mut config = agena_plugin_host::PluginsConfig::default();
        let mut static_plugins = Vec::new();
        if let Some(stop) = stop {
            config.list.insert(
                "fixture.stop".into(),
                agena_plugin_host::ConfiguredPlugin::default(),
            );
            static_plugins.push(agena_plugin_host::StaticPluginRegistration::new(
                "fixture.stop".parse().unwrap(),
                stop,
            ));
        }
        if start_patch {
            config.list.insert(
                "fixture.start_patch".into(),
                agena_plugin_host::ConfiguredPlugin::default(),
            );
            static_plugins.push(agena_plugin_host::StaticPluginRegistration::new(
                "fixture.start_patch".parse().unwrap(),
                StartPatchPlugin,
            ));
        }
        if let Some(effect) = effect {
            for id in ["agena.tools", "fixture.effect"] {
                config
                    .list
                    .insert(id.into(), agena_plugin_host::ConfiguredPlugin::default());
            }
            static_plugins.push(agena_plugin_host::StaticPluginRegistration::new(
                "agena.tools".parse().unwrap(),
                ApiPlugin,
            ));
            static_plugins.push(agena_plugin_host::StaticPluginRegistration::new(
                "fixture.effect".parse().unwrap(),
                EffectPlugin(effect),
            ));
        }
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                workspace_root: workspace.clone(),
                agena_version: "test".into(),
                config,
                static_plugins,
                callback_base_url: None,
                host_client: None,
                previous: None,
                previous_plugins: Default::default(),
            })
            .await
            .unwrap();
        let executor = ToolExecutor::new(
            workspace,
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            plugins.clone(),
            None,
            None,
        );
        let (events, receivers): (Vec<_>, VecDeque<_>) =
            (0..streams).map(|_| mpsc::unbounded_channel()).unzip();
        let provider = Arc::new(Provider {
            model: ModelId::new("model"),
            streams: StdMutex::new(receivers),
            requests: Default::default(),
            called: Notify::new(),
            summary: Default::default(),
            summary_calls: AtomicUsize::new(0),
            native_calls: AtomicUsize::new(0),
            unchanged: std::sync::atomic::AtomicBool::new(false),
            input_limit: AtomicUsize::new(0),
            checkpoint_mode: AtomicUsize::new(0),
        });
        let mut registry = ProviderRegistry::new();
        registry.register_arc(provider.clone());
        let config = RuntimeSessionManagerConfig {
            default_selection: if start_patch {
                ExecutionSelection::default()
            } else {
                ExecutionSelection {
                    provider: Some("fixture".into()),
                    model: Some("model".into()),
                    ..Default::default()
                }
            },
            ..Default::default()
        };
        let manager = SessionManager::new(
            kernel,
            Arc::new(registry),
            ContextGovernor::new(Default::default()),
            SessionProcessor::new(plugins),
            executor,
            config,
        );
        let session = manager
            .create_session(SessionCreateRequest {
                title: "supervision".into(),
                parent_session_id: None,
            })
            .await
            .unwrap();
        Self {
            manager,
            session,
            provider,
            events,
            _root: root,
        }
    }
    fn options(&self) -> SessionRunOptions {
        self.manager
            .options_for_selection(&self.session.execution.selection)
            .unwrap()
    }
    async fn submit(&self, text: &str) -> crate::SessionExecutionCommandOutcome {
        self.manager
            .submit(crate::SessionUserRunRequest::new(
                self.session.id,
                self.options(),
                document(text),
            ))
            .await
            .unwrap()
    }
    async fn calls(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let notified = self.provider.called.notified();
                if self.provider.requests.lock().unwrap().len() >= count {
                    break;
                }
                notified.await;
            }
        })
        .await
        .unwrap();
    }
    fn finish(&self, index: usize, value: &str) {
        self.events[index]
            .send(Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new("fixture"),
                model: ModelId::new("model"),
                delta: value.into(),
            }))
            .unwrap();
        self.events[index]
            .send(Ok(CompletionStreamEvent::Completed {
                provider_id: ProviderId::new("fixture"),
                model: ModelId::new("model"),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: None,
                provider_metadata: None,
                end_turn: Some(true),
            }))
            .unwrap();
    }
    async fn idle(&self, id: i64) {
        tokio::time::timeout(Duration::from_secs(5), self.manager.wait_session_idle(id))
            .await
            .unwrap();
    }
    async fn close(&self) {
        self.manager.shutdown().await.unwrap();
        self.manager.kernel().clone().shutdown().await.unwrap();
    }
}
fn document(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}
fn image_document() -> ComposerDocument {
    let mut input = document("inspect image");
    input.0.push(ComposerNode::Activity {
        activity: Box::new(agena_domain::ComposerActivity {
            id: agena_domain::ActivityId::new(),
            provenance: Default::default(),
            payload: agena_domain::ActivityPayload::Resource(agena_domain::ResourceActivity {
                delivery: agena_domain::ResourceDelivery::ModelInput,
                kind: agena_domain::ResourceKind::Image,
                reference: agena_domain::ResourceReference::WorkspacePath {
                    path: "input.png".into(),
                },
                name: "input.png".into(),
                media_type: Some("image/png".into()),
                size_bytes: None,
                width: None,
                height: None,
                duration_ms: None,
                page_count: None,
            }),
        }),
    });
    input
}
fn response(text: &str) -> CompletionResponse {
    CompletionResponse {
        provider_id: ProviderId::new("fixture"),
        model: ModelId::new("model"),
        text: text.into(),
        reasoning_text: None,
        finish_reason: Some(CompletionFinishReason::Stop),
        tool_calls: Vec::new(),
        usage: None,
        provider_metadata: None,
    }
}

#[tokio::test]
async fn selection_updates_group_commit_and_preserve_all_run_options() {
    let fixture = Fixture::new(0).await;
    let mut options = fixture.options();
    options.model =
        ModelRef::try_new_with_adapter("fixture", "selection-adapter", "selected-model").unwrap();
    options.thinking_mode = Some("high".into());
    options.speed_mode = Some("fast".into());
    options.verbosity = Some("verbose".into());
    options.model_supports_parallel_tool_calls = Some(false);

    for clear_overrides in [false, true] {
        if clear_overrides {
            options.thinking_mode = None;
            options.speed_mode = None;
            options.verbosity = None;
            options.model_supports_parallel_tool_calls = None;
        }
        let before = fixture.manager.kernel_runtime.diagnostics.snapshot();
        fixture
            .manager
            .update_session_selection(fixture.session.id, options.clone())
            .await
            .unwrap();
        let after = fixture.manager.kernel_runtime.diagnostics.snapshot();
        assert_eq!(after.command_results - before.command_results, 5);
        let transactions = after.command_transactions - before.command_transactions;
        assert!(
            (1..=2).contains(&transactions),
            "five field updates must share commits rather than wait serially: {transactions}"
        );
        let metadata = fixture
            .manager
            .kernel()
            .reader
            .metadata(session_id(fixture.session.id).unwrap())
            .await
            .unwrap();
        let selection = metadata.selection.as_ref().unwrap();
        assert_eq!(selection.model_ref().unwrap(), Some(options.model.clone()));
        assert_eq!(selection.thinking_mode, options.thinking_mode);
        assert_eq!(selection.speed_mode, options.speed_mode);
        assert_eq!(selection.verbosity, options.verbosity);
        assert_eq!(
            selection.parallel_tool_calls,
            options.model_supports_parallel_tool_calls
        );
    }
    assert!(fixture.provider.requests.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn session_start_context_and_message_deliveries_have_distinct_business_keys_per_session() {
    // No model is selected, so both patch fields must remain reliable
    // deliveries until a caller selects a model rather than starting a run.
    let fixture = Fixture::with_plugins(0, None, true).await;
    let second = fixture
        .manager
        .create_session(SessionCreateRequest {
            title: "another session with the same startup patch".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    assert_ne!(fixture.session.id, second.id);
    let mut delivery_ids = std::collections::BTreeSet::new();
    let mut business_keys = std::collections::BTreeSet::new();
    for id in [fixture.session.id, second.id] {
        let session = session_id(id).unwrap();
        let pending = fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(session, None, 64)
            .await
            .unwrap();
        assert_eq!(pending.len(), 2);
        for (kind, text) in [
            ("context", "shared startup context"),
            ("message", "shared startup message"),
        ] {
            let delivery = pending
                .iter()
                .find(|delivery| delivery.payload["text"] == text)
                .unwrap();
            assert_eq!(delivery.business_key, format!("session-start-{kind}:{id}"));
            assert_eq!(delivery.session, session);
            assert!(!delivery.consumed);
            assert!(delivery_ids.insert(delivery.id));
            assert!(business_keys.insert(delivery.business_key.clone()));
        }
    }
    assert_eq!(delivery_ids.len(), 4);
    assert_eq!(business_keys.len(), 4);
    assert!(fixture.provider.requests.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn dropping_submit_waiter_during_media_preparation_does_not_orphan_committed_input() {
    let fixture = Fixture::new(1).await;
    let path = fixture._root.path().join("input.png");
    use base64::Engine as _;
    let bytes=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==").unwrap();
    tokio::fs::write(&path, &bytes).await.unwrap();
    let held = super::media::MEDIA_PREPARATIONS
        .acquire_many(2)
        .await
        .unwrap();
    let input = image_document();
    let manager = fixture.manager.clone();
    let id = fixture.session.id;
    let options = fixture.options();
    let waiter = tokio::spawn(async move {
        manager
            .submit(crate::SessionUserRunRequest::new(id, options, input))
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.manager.admission.available_permits() == MAX_ACTIVE_EXECUTIONS {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    waiter.abort();
    drop(held);
    fixture.calls(1).await;
    tokio::fs::write(&path, b"different file after admission")
        .await
        .unwrap();
    let request = fixture.provider.requests.lock().unwrap()[0].clone();
    let encoded = serde_json::to_string(&request).unwrap();
    assert!(encoded.contains("iVBORw0KGgo"));
    assert!(!encoded.contains("different file after admission"));
    fixture.finish(0, "the frozen image");
    fixture.idle(id).await;
    let history = fixture
        .manager
        .kernel()
        .reader
        .history(session_id(id).unwrap(), None, 128)
        .await
        .unwrap();
    let user = history
        .entries
        .iter()
        .find(|e| matches!(e.payload, EntryPayload::User { .. }))
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .entry_blobs(session_id(id).unwrap(), user.id)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .manager
            .final_text(id, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some("the frozen image")
    );
    fixture.close().await;
}

#[tokio::test]
async fn shutdown_with_gated_media_and_persistent_commit_failure_is_bounded_and_can_settle() {
    use base64::Engine as _;
    use sea_orm::{ConnectionTrait, DbBackend, Statement};

    let fixture = Fixture::new(1).await;
    let active = fixture
        .submit("keep one genuine provider stream active")
        .await;
    fixture.calls(1).await;
    let active_execution = active.receipt.unwrap().execution_id;
    let active_session = session_id(fixture.session.id).unwrap();
    let media_session = fixture
        .manager
        .create_session(SessionCreateRequest {
            title: "media waiting during shutdown".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    let bytes=base64::engine::general_purpose::STANDARD.decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGMQVDL+DwACFAFmBODefwAAAABJRU5ErkJggg==").unwrap();
    tokio::fs::write(fixture._root.path().join("input.png"), &bytes)
        .await
        .unwrap();
    let held = super::media::MEDIA_PREPARATIONS
        .acquire_many(2)
        .await
        .unwrap();
    let key = format!("shutdown-media-{}", RequestId::new());
    let target = request_key(&format!("send:{}", media_session.id), &key);
    let (reached, waiting) = oneshot::channel();
    *super::media::PREPARATION_BARRIER.lock().unwrap() = Some((target, reached));
    let manager = fixture.manager.clone();
    let options = fixture.options();
    let media_id = media_session.id;
    let waiter = tokio::spawn(async move {
        manager
            .submit(
                crate::SessionUserRunRequest::new(media_id, options, image_document())
                    .with_idempotency_key(key),
            )
            .await
    });
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.manager.admission.available_permits(),
        MAX_ACTIVE_EXECUTIONS - 2
    );

    // This owned-file trigger rejects every terminal attempt COMMIT. A direct
    // public command proves the fault is active before shutdown is exercised.
    let admin = sea_orm::Database::connect(format!(
        "sqlite://{}?mode=rw",
        fixture._root.path().join("sessions.db").display()
    ))
    .await
    .unwrap();
    admin.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TABLE reject_shutdown_commit (owner INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)"))
        .await.unwrap();
    admin.execute(Statement::from_string(DbBackend::Sqlite,
        "CREATE TRIGGER reject_shutdown_terminal AFTER UPDATE OF phase ON agena_attempts WHEN NEW.phase IN ('cancelled','failed','succeeded','interrupted') BEGIN INSERT INTO reject_shutdown_commit VALUES (-1); END"))
        .await.unwrap();
    assert!(matches!(
        fixture
            .manager
            .kernel()
            .sessions
            .cancel(RequestId::new(), active_session, active_execution)
            .await,
        Err(KernelError::Storage(_))
    ));
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(active_session, active_execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Running
    );

    let failed_shutdown = tokio::time::timeout(Duration::from_secs(7), fixture.manager.shutdown())
        .await
        .expect("shutdown must report its fixed persistence deadline");
    assert!(matches!(failed_shutdown, Err(AppError::PersistenceBusy)));
    assert_eq!(
        fixture.manager.admission.available_permits(),
        MAX_ACTIVE_EXECUTIONS - 2
    );
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);

    // Release preparation while persistence is still failing. Its accepted
    // input is supervised by an already cancelled token, so no provider effect
    // starts. Neither run can pretend to have settled its durable terminal fact.
    drop(held);
    let media = tokio::time::timeout(Duration::from_secs(5), waiter)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    let media_execution = media.receipt.unwrap().execution_id;
    let media_session = session_id(media_id).unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fixture
                .manager
                .kernel()
                .reader
                .execution(media_session, media_execution)
                .await
                .unwrap()
                .phase
                == ExecutionPhase::Running
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    assert_eq!(
        fixture.manager.admission.available_permits(),
        MAX_ACTIVE_EXECUTIONS - 2
    );
    admin
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER reject_shutdown_terminal",
        ))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(7), fixture.manager.shutdown())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        fixture.manager.admission.available_permits(),
        MAX_ACTIVE_EXECUTIONS
    );
    assert_eq!(
        fixture.manager.task_settlements.available_permits(),
        MAX_ACTIVE_EXECUTIONS
    );
    assert!(fixture.manager.active.lock().unwrap().is_empty());
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    for (session, execution) in [
        (active_session, active_execution),
        (media_session, media_execution),
    ] {
        assert_eq!(
            fixture
                .manager
                .kernel()
                .reader
                .execution(session, execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Cancelled
        );
    }
    let history = fixture
        .manager
        .kernel()
        .reader
        .history(media_session, None, 16)
        .await
        .unwrap();
    let user = history
        .entries
        .iter()
        .find(|entry| matches!(entry.payload, EntryPayload::User { .. }))
        .unwrap();
    let blobs = fixture
        .manager
        .kernel()
        .reader
        .entry_blobs(media_session, user.id)
        .await
        .unwrap();
    assert_eq!(blobs.len(), 1);
    let mut copied = Vec::new();
    fixture
        .manager
        .kernel()
        .reader
        .open_blob(media_session, blobs[0].id)
        .await
        .unwrap()
        .copy_to(&mut copied)
        .await
        .unwrap();
    assert_eq!(copied, bytes);
    drop(admin);
    fixture.manager.kernel().clone().shutdown().await.unwrap();
}

#[tokio::test]
async fn steering_is_a_distinct_queued_execution_with_frozen_input_and_no_extra_provider_effect() {
    let fixture = Fixture::new(2).await;
    let first = fixture.submit("original user").await;
    fixture.calls(1).await;
    fixture
        .manager
        .steer_input(fixture.session.id, document("steered user"))
        .await
        .unwrap();
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    assert!(
        !serde_json::to_string(&fixture.provider.requests.lock().unwrap()[0])
            .unwrap()
            .contains("steered user")
    );
    fixture.finish(0, "original answer");
    fixture.calls(2).await;
    let second = serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1]).unwrap();
    assert!(second.contains("original user"));
    assert!(second.contains("steered user"));
    assert!(second.contains("original answer"));
    fixture.finish(1, "steered answer");
    fixture.idle(fixture.session.id).await;
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 2);
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(
                session_id(fixture.session.id).unwrap(),
                first.receipt.unwrap().execution_id
            )
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Succeeded
    );
    fixture.close().await;
}

#[tokio::test]
async fn cancellation_is_a_durable_fence_and_does_not_depend_on_private_registry_phase() {
    let fixture = Fixture::new(1).await;
    let sent = fixture.submit("cancel this").await;
    fixture.calls(1).await;
    let execution = sent.receipt.unwrap().execution_id;
    fixture
        .manager
        .cancel_execution_with_outcome(fixture.session.id, execution)
        .await
        .unwrap();
    assert!(
        fixture
            .manager
            .active_execution(fixture.session.id)
            .await
            .is_none()
    );
    fixture.idle(fixture.session.id).await;
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(session_id(fixture.session.id).unwrap(), execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn compaction_outlives_its_waiter_and_publishes_only_a_successful_checkpoint() {
    let fixture = Fixture::new(1).await;
    fixture.submit("retain these instructions").await;
    fixture.calls(1).await;
    fixture.finish(0, "recorded answer");
    fixture.idle(fixture.session.id).await;
    let (sender, receiver) = oneshot::channel();
    *fixture.provider.summary.lock().unwrap() = Some(receiver);
    let manager = fixture.manager.clone();
    let request = SessionExecutionRequest {
        session_id: fixture.session.id,
        options: fixture.options(),
    };
    let waiter = tokio::spawn(async move { manager.compact_session(request).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let notified = fixture.provider.called.notified();
            if fixture.provider.summary_calls.load(Ordering::SeqCst) > 0 {
                break;
            }
            notified.await;
        }
    })
    .await
    .unwrap();
    assert!(
        fixture
            .manager
            .compaction_status(fixture.session.id)
            .await
            .unwrap()
            .checkpoint_entry_id
            .is_none()
    );
    waiter.abort();
    sender
        .send(Ok(response(
            "Retain these instructions and the recorded answer.",
        )))
        .unwrap();
    fixture.idle(fixture.session.id).await;
    let status = fixture
        .manager
        .compaction_status(fixture.session.id)
        .await
        .unwrap();
    assert!(status.checkpoint_entry_id.is_some());
    assert!(status.through_entry_id.is_some());
    assert!(status.native_compaction_unavailable);
    assert_eq!(status.consecutive_compaction_failures, 0);
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn failed_and_unchanged_compactions_never_move_the_boundary_and_failures_disable_the_policy()
{
    let fixture = Fixture::new(1).await;
    fixture.submit("instructions").await;
    fixture.calls(1).await;
    fixture.finish(0, "answer");
    fixture.idle(fixture.session.id).await;
    for expected in 1..=3 {
        fixture
            .manager
            .compact_session(SessionExecutionRequest {
                session_id: fixture.session.id,
                options: fixture.options(),
            })
            .await
            .unwrap();
        let status = fixture
            .manager
            .compaction_status(fixture.session.id)
            .await
            .unwrap();
        assert!(status.checkpoint_entry_id.is_none());
        assert_eq!(status.consecutive_compaction_failures, expected);
        assert_eq!(status.auto_compaction_disabled, expected == 3);
    }
    fixture.provider.unchanged.store(true, Ordering::SeqCst);
    // A changed route/strategy re-enables native probing without changing the
    // authoritative checkpoint. The explicit attempt can safely be unchanged.
    let session = session_id(fixture.session.id).unwrap();
    let metadata = fixture
        .manager
        .kernel()
        .reader
        .metadata(session)
        .await
        .unwrap();
    let sent = fixture
        .manager
        .kernel()
        .sessions
        .continue_execution(
            RequestId::new(),
            session,
            fixture
                .manager
                .latest_origin_message(session)
                .await
                .unwrap()
                .unwrap(),
        )
        .await
        .unwrap()
        .value;
    let attempt = fixture
        .manager
        .kernel()
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    attempt
        .record_compaction(
            RequestId::new(),
            "new-policy".into(),
            CompactionPolicyOutcome::Unchanged,
        )
        .await
        .unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    fixture
        .manager
        .compact_session(SessionExecutionRequest {
            session_id: fixture.session.id,
            options: fixture.options(),
        })
        .await
        .unwrap();
    let status = fixture
        .manager
        .compaction_status(fixture.session.id)
        .await
        .unwrap();
    assert!(status.checkpoint_entry_id.is_none());
    assert_eq!(status.consecutive_compaction_failures, 0);
    assert!(!status.auto_compaction_disabled);
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 3);
    assert_eq!(metadata.title, "supervision");
    fixture.close().await;
}

#[tokio::test]
async fn active_background_releases_without_stop_or_polling_then_real_delivery_stops_once() {
    let stops = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::with_stop(2, stops.clone(), false).await;
    let session = session_id(fixture.session.id).unwrap();
    let ports = fixture.manager.kernel();
    let launch = ports
        .sessions
        .send(RequestId::new(), session, document("launch background"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, launch.id)
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
                call_id: "background".into(),
                name: "fixture".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let activity = tool
        .begin_activity(
            RequestId::new(),
            BackgroundActivityStart {
                kind: BackgroundActivityKind::Process,
                label: "waiting process".into(),
            },
        )
        .await
        .unwrap()
        .value;
    tool.complete(
        RequestId::new(),
        ToolResult {
            phase: ToolPhase::Succeeded,
            value: serde_json::json!({"launched":true}),
        },
    )
    .await
    .unwrap();
    round.finish(RequestId::new(), None, true).await.unwrap();
    attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    fixture.submit("observe background work").await;
    fixture.calls(1).await;
    fixture.finish(0, "waiting for the process");
    fixture.idle(fixture.session.id).await;
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        0
    );
    activity
        .end(
            RequestId::new(),
            BackgroundActivityOutcome::Succeeded {
                value: serde_json::json!({"exit_code":0}),
            },
        )
        .await
        .unwrap();
    ports.executor.deliver(RequestId::new(), session, "actual-background-completion".into(), serde_json::json!({"kind":"background","message_id":launch.message_id.get(),"text":"actual process result"})).await.unwrap();
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        1
    );
    fixture.calls(2).await;
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1])
            .unwrap()
            .contains("actual process result")
    );
    assert_eq!(stops.load(Ordering::SeqCst), 0);
    fixture.finish(1, "actual process result observed");
    fixture.idle(fixture.session.id).await;
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn stop_hook_followup_reopens_guidance_until_the_next_observed_stop_boundary() {
    let stops = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::with_stop(3, stops.clone(), true).await;
    let sent = fixture.submit("complete one run").await;
    fixture.calls(1).await;
    fixture.finish(0, "first completion");
    fixture.calls(2).await;
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1])
            .unwrap()
            .contains("finish the stop-hook follow-up")
    );
    let session = session_id(fixture.session.id).unwrap();
    let execution = sent.receipt.unwrap().execution_id;
    fixture.manager.kernel().executor.deliver_to_execution(RequestId::new(), session, execution, "after-stop-hook-guidance".into(), serde_json::json!({"kind":"task_guidance","text":"guidance after the hook resumed","wake":false})).await.unwrap();
    fixture.finish(1, "hook continuation answered");
    fixture.calls(3).await;
    assert_eq!(stops.load(Ordering::SeqCst), 1);
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[2])
            .unwrap()
            .contains("guidance after the hook resumed")
    );
    fixture.finish(2, "guidance fully observed");
    fixture.idle(fixture.session.id).await;
    assert_eq!(stops.load(Ordering::SeqCst), 2);
    let rejected = fixture
        .manager
        .kernel()
        .executor
        .deliver_to_execution(
            RequestId::new(),
            session,
            execution,
            "after-final-stop".into(),
            serde_json::json!({"text":"too late","wake":false}),
        )
        .await;
    assert!(matches!(rejected, Err(KernelError::Conflict(_))));
    fixture.close().await;
}

#[tokio::test]
async fn idle_background_delivery_wakes_once_and_keeps_its_originating_user_message() {
    let fixture = Fixture::new(2).await;
    let sent = fixture.submit("launch background work").await;
    fixture.calls(1).await;
    fixture.finish(0, "background launched");
    fixture.idle(fixture.session.id).await;
    let session = session_id(fixture.session.id).unwrap();
    let origin = fixture
        .manager
        .kernel()
        .reader
        .execution(session, sent.receipt.unwrap().execution_id)
        .await
        .unwrap()
        .message_id;
    let command = RequestId::new();
    let payload = serde_json::json!({"kind":"background","message_id":origin.get(),"text":"background completed"});
    fixture
        .manager
        .kernel()
        .executor
        .deliver(command, session, "background:one".into(), payload)
        .await
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        1
    );
    fixture.calls(2).await;
    fixture.finish(1, "completion observed");
    fixture.idle(fixture.session.id).await;
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        0
    );
    assert!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(session, None, 128)
            .await
            .unwrap()
            .is_empty()
    );
    let page = fixture
        .manager
        .kernel()
        .reader
        .history(session, None, 128)
        .await
        .unwrap();
    let notices=page.entries.iter().filter(|entry|matches!(&entry.payload,EntryPayload::Notice{text} if text=="background completed")).collect::<Vec<_>>();
    assert_eq!(notices.len(), 1);
    assert_eq!(notices[0].message_id, origin);
    fixture.close().await;
}

fn task_request(parent: i64) -> SessionSubtaskRequest {
    SessionSubtaskRequest {
        parent_session_id: parent,
        run_in_background: false,
        launch_call_id: None,
        description: "independent task".into(),
        prompt: "finish the delegated work".into(),
        commands: None,
        task_id: Some("task-one".into()),
        requested_model_selection: Default::default(),
        timeout_ms: Some(5_000),
        max_tokens: None,
        max_cost_microusd: None,
    }
}
#[tokio::test]
async fn child_settlement_survives_dropped_foreground_waiter_and_task_resume_uses_a_new_start() {
    let fixture = Fixture::new(3).await;
    let parent = fixture.submit("delegate this task").await;
    fixture.calls(1).await;
    fixture.finish(0, "ready");
    fixture.idle(fixture.session.id).await;
    let origin = fixture
        .manager
        .kernel()
        .reader
        .execution(
            session_id(fixture.session.id).unwrap(),
            parent.receipt.unwrap().execution_id,
        )
        .await
        .unwrap()
        .message_id;
    let manager = fixture.manager.clone();
    let request = task_request(fixture.session.id);
    let waiter = tokio::spawn(async move { manager.run_subtask(request).await });
    fixture.calls(2).await;
    waiter.abort();
    fixture.finish(1, "child result");
    let child = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let child = fixture
                .manager
                .task_session(fixture.session.id, "task-one")
                .await
                .unwrap();
            if child.subtask_status == Some(SubtaskStatus::Completed) {
                break child;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    fixture.idle(child.id).await;
    let manager = fixture.manager.clone();
    let request = task_request(fixture.session.id);
    let resumed = tokio::spawn(async move { manager.run_subtask(request).await });
    fixture.calls(3).await;
    fixture.finish(2, "resumed result");
    let resumed = resumed.await.unwrap().unwrap();
    assert!(resumed.resumed);
    assert_eq!(resumed.status, SubtaskStatus::Completed);
    assert_eq!(resumed.final_text.as_deref(), Some("resumed result"));
    assert!(resumed.session.subtask_started_at_ms > child.subtask_started_at_ms);
    let pending = fixture
        .manager
        .kernel()
        .reader
        .pending_deliveries(session_id(fixture.session.id).unwrap(), None, 128)
        .await
        .unwrap();
    assert_eq!(pending.len(), 2);
    assert!(
        pending
            .iter()
            .all(|delivery| delivery.payload["message_id"] == origin.get())
    );
    let output = fixture
        .manager
        .read_subtask_output(fixture.session.id, "task-one", 0, 128)
        .await
        .unwrap();
    assert!(
        output
            .chunks
            .iter()
            .any(|chunk| chunk.text == "child result")
    );
    assert!(
        output
            .chunks
            .iter()
            .any(|chunk| chunk.text == "resumed result")
    );
    fixture.close().await;
}

#[tokio::test]
async fn hard_prompt_budget_rejects_before_provider_effect_when_compaction_is_disabled() {
    let fixture = Fixture::new(0).await;
    fixture.provider.input_limit.store(512, Ordering::SeqCst);
    let mut state = (*fixture.manager.state.load_full()).clone();
    state.config.auto_compaction.enabled = false;
    fixture.manager.state.store(Arc::new(state));
    let sent = fixture.submit(&"user instruction ".repeat(1000)).await;
    fixture.idle(fixture.session.id).await;
    assert!(fixture.provider.requests.lock().unwrap().is_empty());
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 0);
    let view = fixture
        .manager
        .kernel()
        .reader
        .execution(
            session_id(fixture.session.id).unwrap(),
            sent.receipt.unwrap().execution_id,
        )
        .await
        .unwrap();
    assert_eq!(view.phase, ExecutionPhase::Failed);
    fixture.close().await;
}

#[tokio::test]
async fn proactive_unchanged_compaction_leaves_one_model_turn_available_below_the_hard_limit() {
    let fixture = Fixture::new(1).await;
    fixture.provider.unchanged.store(true, Ordering::SeqCst);
    let mut state = (*fixture.manager.state.load_full()).clone();
    state.config.max_turns = Some(1);
    state.governor = ContextGovernor::new(agena_domain::ContextPolicy {
        max_prompt_chars: 1,
        ..Default::default()
    });
    fixture.manager.state.store(Arc::new(state));
    let sent = fixture
        .submit("small prompt with proactive compaction")
        .await;
    fixture.calls(1).await;
    fixture.finish(0, "model was still called");
    fixture.idle(fixture.session.id).await;
    assert_eq!(fixture.provider.native_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(
                session_id(fixture.session.id).unwrap(),
                sent.receipt.unwrap().execution_id
            )
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Succeeded
    );
    assert!(
        fixture
            .manager
            .compaction_status(fixture.session.id)
            .await
            .unwrap()
            .checkpoint_entry_id
            .is_none()
    );
    fixture.close().await;
}

#[tokio::test]
async fn approval_classifier_allows_cited_denials_block_and_uncertain_denials_require_confirmation()
{
    let fixture = Fixture::new(0).await;
    let action = agena_domain::PermissionAction::Tool {
        tool_name: "fs.read".into(),
        qualifier: None,
    };
    let decision = agena_domain::PermissionDecision::Auto {
        reason: "review this operation".into(),
    };
    let state = fixture.manager.state.load_full();
    for (text, expected) in [
        ("allow", Some(true)),
        (
            r#"{"shouldBlock":true,"reason":"[Exfiltration] sends the repository to a paste site."}"#,
            Some(false),
        ),
        (r#"{"shouldBlock":true,"reason":"I am unsure."}"#, None),
        ("no classified decision", None),
    ] {
        let (sender, receiver) = oneshot::channel();
        *fixture.provider.summary.lock().unwrap() = Some(receiver);
        sender.send(Ok(response(text))).unwrap();
        assert_eq!(
            fixture
                .manager
                .classify_permission(
                    &action,
                    &decision,
                    session_id(fixture.session.id).unwrap(),
                    &state,
                    &state.executor
                )
                .await,
            expected
        );
    }
    assert!(fixture.provider.requests.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn committed_permission_reply_is_confirmed_when_follow_up_reads_are_exhausted() {
    let fixture = Fixture::new(0).await;
    let session = session_id(fixture.session.id).unwrap();
    let ports = fixture.manager.kernel();
    let execution = ports
        .sessions
        .send(
            RequestId::new(),
            session,
            document("confirm a durable reply"),
        )
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution.id)
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
                call_id: "confirmed-reply".into(),
                name: "fs.read".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: serde_json::to_string(&agena_domain::PermissionAction::Tool {
                    tool_name: "fs.read".into(),
                    qualifier: None,
                })
                .unwrap(),
                description: "confirm the committed decision independently of reads".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let reply = agena_domain::PermissionReply {
        request_id: interaction.get().to_string(),
        kind: agena_domain::PermissionReplyKind::AllowOnce,
        reason: Some("confirmed even when a projection cannot be admitted".into()),
        scope: None,
    };
    let request = crate::SessionPermissionReplyRequest::new(
        fixture.session.id,
        fixture.options(),
        reply.clone(),
        None,
    );

    // Retain actual views, clearing only the cache between reads, until the
    // session's allocation quota is exhausted. Writes have separate admission.
    // The old command path
    // committed this reply and then failed its redundant get_session read.
    let mut retained = Vec::new();
    let mut exhausted = false;
    for _ in 0..=2048 {
        fixture
            .manager
            .kernel_runtime
            .maintenance
            .invalidate_read_cache();
        match ports.reader.control_snapshot(session, 1).await {
            Ok(snapshot) => retained.push(snapshot),
            Err(KernelError::Busy) => {
                exhausted = true;
                break;
            }
            Err(error) => panic!("unexpected control snapshot failure: {error}"),
        }
    }
    assert!(
        exhausted,
        "the regression must force real read admission failure"
    );
    fixture
        .manager
        .get_session(fixture.session.id)
        .await
        .expect_err("the regression must reject the old command's follow-up read");
    let confirmed =
        crate::SessionExecutionCommandService::reply_permission(&fixture.manager, request)
            .await
            .expect("a committed decision must not require a follow-up read");
    assert_eq!(confirmed.session_id, fixture.session.id);

    drop(retained);
    let snapshot = ports.reader.control_snapshot(session, 128).await.unwrap();
    assert!(snapshot.pending_interactions.interactions.is_empty());
    assert!(matches!(
        snapshot.active_execution.as_ref(),
        Some(agena_domain::ExecutionLifecycle::Active { execution_id, .. })
            if *execution_id == execution.id
    ));
    for index in 0..8 {
        ports
            .sessions
            .update_selection(
                RequestId::new(),
                session,
                SelectionChange::Model(Some(ModelRef::new("fixture", format!("later-{index}")))),
            )
            .await
            .unwrap();
    }
    ports
        .sessions
        .cancel(RequestId::new(), session, execution.id)
        .await
        .unwrap();

    // The owner advanced repeatedly after the successful reply and captured
    // cut. Runtime projection uses that cut rather than reloading the model,
    // workflow, or root interactions from later committed state.
    let context = crate::SessionQueryService::execution_context(&fixture.manager, &snapshot)
        .expect("project the captured control cut without another read");
    assert_eq!(context.model_id.as_deref(), Some("model"));
    assert_eq!(
        context.workflow_state,
        agena_domain::WorkflowState::Quiescent
    );
    assert!(
        crate::SessionQueryService::pending_interactive_requests(&fixture.manager, &snapshot)
            .await
            .unwrap()
            .is_empty()
    );
    let stored = ports
        .reader
        .interaction(session, interaction)
        .await
        .unwrap();
    assert_eq!(
        stored.decision,
        Some(InteractionDecision::Permission {
            reply,
            allowed: true
        })
    );
    drop((stored, snapshot));
    fixture.close().await;
}

#[tokio::test]
async fn automatic_permission_replays_reuse_the_complete_winning_reply_without_reclassification() {
    let fixture = Fixture::new(0).await;
    let session = session_id(fixture.session.id).unwrap();
    let ports = fixture.manager.kernel();
    let execution = ports
        .sessions
        .send(RequestId::new(), session, document("review this action"))
        .await
        .unwrap()
        .value;
    let attempt = ports
        .executor
        .claim(RequestId::new(), session, execution.id)
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
                call_id: "automatic-review".into(),
                name: "fs.read".into(),
                input: serde_json::json!({}),
            },
        )
        .await
        .unwrap()
        .value;
    let interaction = tool
        .request_interaction(
            RequestId::new(),
            InteractionRequest::Permission {
                action_key: serde_json::to_string(&agena_domain::PermissionAction::Tool {
                    tool_name: "fs.read".into(),
                    qualifier: None,
                })
                .unwrap(),
                description: "review this action".into(),
            },
        )
        .await
        .unwrap()
        .value;
    let reply = agena_domain::PermissionReply {
        request_id: interaction.get().to_string(),
        kind: agena_domain::PermissionReplyKind::AutoApprove,
        reason: Some("the user's original automatic review request".into()),
        scope: None,
    };
    let request = crate::SessionPermissionReplyRequest::new(
        fixture.session.id,
        fixture.options(),
        reply.clone(),
        None,
    );
    let (allow, allow_result) = oneshot::channel();
    *fixture.provider.summary.lock().unwrap() = Some(allow_result);
    let manager = fixture.manager.clone();
    let first_request = request.clone();
    let first = tokio::spawn(async move { manager.reply_permission(first_request).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.provider.summary_calls.load(Ordering::SeqCst) < 1 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let (deny, deny_result) = oneshot::channel();
    *fixture.provider.summary.lock().unwrap() = Some(deny_result);
    let manager = fixture.manager.clone();
    let second_request = request.clone();
    let second = tokio::spawn(async move { manager.reply_permission(second_request).await });
    tokio::time::timeout(Duration::from_secs(5), async {
        while fixture.provider.summary_calls.load(Ordering::SeqCst) < 2 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    allow.send(Ok(response("allow"))).unwrap();
    first.await.unwrap().unwrap();
    let decided_at = ports
        .reader
        .interaction(session, interaction)
        .await
        .unwrap()
        .decided_at_ms;
    deny.send(Ok(response(
        r#"{"shouldBlock":true,"reason":"[Exfiltration] sends the repository to a paste site."}"#,
    )))
    .unwrap();
    second.await.unwrap().unwrap();
    // No classifier result remains: replay would fail if it called the model.
    fixture
        .manager
        .reply_permission(request.clone())
        .await
        .unwrap();
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 2);
    let stored = ports
        .reader
        .interaction(session, interaction)
        .await
        .unwrap();
    assert_eq!(
        stored.decision,
        Some(InteractionDecision::Permission {
            reply,
            allowed: true,
        })
    );
    assert_eq!(stored.decided_at_ms, decided_at);
    drop(stored);
    let mut changed = request;
    changed.request.reply.reason = Some("a different reply cannot replace the winner".into());
    assert!(matches!(
        fixture.manager.reply_permission(changed).await,
        Err(AppError::InteractiveRequestNotPending { .. })
    ));
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 2);
    ports
        .sessions
        .cancel(RequestId::new(), session, execution.id)
        .await
        .unwrap();
    fixture.close().await;
}

#[tokio::test]
async fn boundary_consumes_only_guidance_owned_by_its_execution() {
    let fixture = Fixture::new(0).await;
    let session = session_id(fixture.session.id).unwrap();
    let ports = fixture.manager.kernel();
    let mut executions = Vec::new();
    let mut attempts = Vec::new();
    for text in ["guidance owner", "another execution"] {
        let execution = ports
            .sessions
            .send(RequestId::new(), session, document(text))
            .await
            .unwrap()
            .value;
        attempts.push(
            ports
                .executor
                .claim(RequestId::new(), session, execution.id)
                .await
                .unwrap()
                .value,
        );
        executions.push(execution);
    }
    let owner = &executions[0];
    let other = &executions[1];
    ports
        .executor
        .deliver_to_execution(
            RequestId::new(),
            session,
            owner.id,
            "boundary-owner".into(),
            serde_json::json!({"kind":"task_guidance","text":"only this execution","wake":false}),
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .consume_boundary_deliveries(session, other.id, other.message_id)
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        ports
            .reader
            .pending_deliveries(session, None, 64)
            .await
            .unwrap()
            .len(),
        1
    );
    assert_eq!(
        fixture
            .manager
            .consume_boundary_deliveries(session, owner.id, owner.message_id)
            .await
            .unwrap(),
        1
    );
    for attempt in &attempts {
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    let history = ports.reader.history(session, None, 20).await.unwrap();
    assert_eq!(history.entries.len(), 3);
    let notice = history
        .entries
        .iter()
        .find(|entry| matches!(entry.payload, EntryPayload::Notice { .. }))
        .unwrap();
    assert_eq!(notice.execution_id, owner.id);
    assert_eq!(notice.message_id, owner.message_id);
    drop((history, attempts));
    fixture.close().await;
}

#[tokio::test]
async fn task_guidance_continues_the_supervised_execution_and_terminal_tasks_require_followup() {
    let fixture = Fixture::new(3).await;
    fixture.submit("delegate work").await;
    fixture.calls(1).await;
    fixture.finish(0, "ready");
    fixture.idle(fixture.session.id).await;
    let manager = fixture.manager.clone();
    let request = task_request(fixture.session.id);
    let task = tokio::spawn(async move { manager.run_subtask(request).await });
    fixture.calls(2).await;
    let child = fixture
        .manager
        .task_session(fixture.session.id, "task-one")
        .await
        .unwrap();
    let session = session_id(child.id).unwrap();
    let execution = fixture
        .manager
        .kernel()
        .reader
        .active_executions(session, None, 1)
        .await
        .unwrap()
        .executions[0]
        .id;
    fixture
        .manager
        .message_subtask(
            fixture.session.id,
            "task-one",
            "Keep the important instruction.".into(),
        )
        .await
        .unwrap();
    let pending = fixture
        .manager
        .kernel()
        .reader
        .pending_deliveries(session, None, 64)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].target_execution, Some(execution));
    drop(pending);
    fixture.finish(1, "initial answer");
    fixture.calls(3).await;
    assert_eq!(
        fixture
            .manager
            .task_session(fixture.session.id, "task-one")
            .await
            .unwrap()
            .subtask_status,
        Some(SubtaskStatus::Running)
    );
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[2].turns)
            .unwrap()
            .contains("Keep the important instruction.")
    );
    assert!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(session, None, 64)
            .await
            .unwrap()
            .is_empty()
    );
    fixture.finish(2, "guided answer");
    let response = task.await.unwrap().unwrap();
    assert_eq!(response.status, SubtaskStatus::Completed);
    assert_eq!(response.final_text.as_deref(), Some("guided answer"));
    let rounds = fixture
        .manager
        .kernel()
        .reader
        .rounds(session, None, 64)
        .await
        .unwrap();
    assert_eq!(rounds.len(), 2);
    assert!(rounds.iter().all(|round| round.execution_id == execution));
    drop(rounds);
    let output = fixture
        .manager
        .read_subtask_output(fixture.session.id, "task-one", 0, 128)
        .await
        .unwrap();
    for text in ["initial answer", "guided answer"] {
        assert!(output.chunks.iter().any(|chunk| chunk.text == text));
    }
    assert!(matches!(
        fixture
            .manager
            .message_subtask(fixture.session.id, "task-one", "too late".into())
            .await,
        Err(AppError::PersistenceConflict(_))
    ));
    fixture.close().await;
}

#[tokio::test]
async fn interrupted_task_recovery_repairs_terminal_projection_and_parent_delivery_once() {
    let fixture = Fixture::new(0).await;
    let parent = session_id(fixture.session.id).unwrap();
    let child = fixture
        .manager
        .kernel()
        .sessions
        .create_configured(
            RequestId::new(),
            fixture.session.workspace_id,
            "crashed task".into(),
            SessionConfiguration {
                parent_session_id: Some(parent),
                relation_kind: agena_domain::SessionRelationKind::Subagent,
                task: Some(SessionTaskConfiguration {
                    id: "task-recovered".into(),
                    status: SubtaskStatus::Running,
                    started_at_ms: Some(1),
                    finished_at_ms: None,
                    failure: None,
                }),
                ..Default::default()
            },
        )
        .await
        .unwrap()
        .value
        .id;
    assert_eq!(
        fixture.manager.reconcile_background_tasks(1).await.unwrap(),
        1
    );
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .configuration(child)
            .await
            .unwrap()
            .task
            .as_ref()
            .unwrap()
            .status,
        SubtaskStatus::Interrupted
    );
    assert_eq!(
        fixture.manager.reconcile_background_tasks(1).await.unwrap(),
        0
    );
    let pending = fixture
        .manager
        .kernel()
        .reader
        .pending_deliveries(parent, None, 64)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].payload["child_session_id"], child.get());
    drop(pending);
    assert!(fixture.provider.requests.lock().unwrap().is_empty());
    fixture.close().await;
}

#[tokio::test]
async fn scheduled_delivery_keeps_one_business_event_and_rejects_changed_replays() {
    let fixture = Fixture::new(1).await;
    let execution = fixture
        .submit("scheduled parent")
        .await
        .receipt
        .unwrap()
        .execution_id;
    fixture.calls(1).await;
    fixture.finish(0, "ready");
    fixture.idle(fixture.session.id).await;
    let session = session_id(fixture.session.id).unwrap();
    let message = fixture
        .manager
        .kernel()
        .reader
        .execution(session, execution)
        .await
        .unwrap()
        .message_id;
    assert!(
        fixture
            .manager
            .deliver_scheduled_job(
                fixture.session.id,
                "job".into(),
                "fire-1".into(),
                "scheduled instruction".into(),
                None
            )
            .await
            .unwrap()
    );
    assert!(
        fixture
            .manager
            .deliver_scheduled_job(
                fixture.session.id,
                "job".into(),
                "fire-1".into(),
                "scheduled instruction".into(),
                None
            )
            .await
            .unwrap()
    );
    let pending = fixture
        .manager
        .kernel()
        .reader
        .pending_deliveries(session, None, 64)
        .await
        .unwrap();
    assert_eq!(pending.len(), 1);
    let id = pending[0].id;
    drop(pending);
    fixture
        .manager
        .kernel()
        .executor
        .consume_delivery(
            RequestId::new(),
            session,
            id,
            message,
            "scheduled instruction".into(),
        )
        .await
        .unwrap();
    assert!(
        !fixture
            .manager
            .deliver_scheduled_job(
                fixture.session.id,
                "job".into(),
                "fire-1".into(),
                "scheduled instruction".into(),
                None
            )
            .await
            .unwrap()
    );
    assert!(
        fixture
            .manager
            .deliver_scheduled_job(
                fixture.session.id,
                "job".into(),
                "fire-1".into(),
                "changed".into(),
                None
            )
            .await
            .is_err()
    );
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn delegated_token_budget_bounds_provider_output_and_settles_the_task_with_usage() {
    let fixture = Fixture::new(2).await;
    fixture.submit("delegate bounded work").await;
    fixture.calls(1).await;
    fixture.finish(0, "ready");
    fixture.idle(fixture.session.id).await;
    let manager = fixture.manager.clone();
    let mut request = task_request(fixture.session.id);
    request.max_tokens = Some(10);
    let task = tokio::spawn(async move { manager.run_subtask(request).await });
    fixture.calls(2).await;
    assert_eq!(
        fixture.provider.requests.lock().unwrap()[1].max_output_tokens,
        Some(10)
    );
    fixture.events[1]
        .send(Ok(CompletionStreamEvent::TextDelta {
            provider_id: ProviderId::new("fixture"),
            model: ModelId::new("model"),
            delta: "bounded result".into(),
        }))
        .unwrap();
    fixture.events[1]
        .send(Ok(CompletionStreamEvent::Completed {
            provider_id: ProviderId::new("fixture"),
            model: ModelId::new("model"),
            finish_reason: Some(CompletionFinishReason::Stop),
            usage: Some(agena_provider::CompletionUsage {
                input_tokens: 10,
                output_tokens: 10,
                ..Default::default()
            }),
            provider_metadata: None,
            end_turn: Some(false),
        }))
        .unwrap();
    let task = task.await.unwrap().unwrap();
    assert!(task.budget_exceeded);
    assert_eq!(task.status, SubtaskStatus::Failed);
    assert_eq!(task.usage.total_tokens(), 20);
    assert!(task.failure.is_some());
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 2);
    fixture.close().await;
}

#[tokio::test]
async fn cancelling_one_session_does_not_fence_an_unrelated_running_session() {
    let fixture = Fixture::new(2).await;
    let first = fixture.submit("cancel only this session").await;
    fixture.calls(1).await;
    let second = fixture
        .manager
        .create_session(SessionCreateRequest {
            title: "independent".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    let other = fixture
        .manager
        .submit(crate::SessionUserRunRequest::new(
            second.id,
            fixture.options(),
            document("continue independently"),
        ))
        .await
        .unwrap();
    fixture.calls(2).await;
    fixture
        .manager
        .cancel_execution_with_outcome(fixture.session.id, first.receipt.unwrap().execution_id)
        .await
        .unwrap();
    fixture.finish(1, "independent success");
    fixture.idle(second.id).await;
    fixture.idle(fixture.session.id).await;
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(
                session_id(second.id).unwrap(),
                other.receipt.unwrap().execution_id
            )
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Succeeded
    );
    fixture.close().await;
}

#[tokio::test]
async fn active_boundary_consumes_background_deliveries_without_starting_a_competing_execution() {
    let fixture = Fixture::new(2).await;
    let sent = fixture
        .submit("observe completion at the next boundary")
        .await;
    fixture.calls(1).await;
    let session = session_id(fixture.session.id).unwrap();
    let execution = sent.receipt.unwrap().execution_id;
    let origin = fixture
        .manager
        .kernel()
        .reader
        .execution(session, execution)
        .await
        .unwrap()
        .message_id;
    fixture
        .manager
        .kernel()
        .executor
        .deliver(
            RequestId::new(),
            session,
            "active-completion".into(),
            serde_json::json!({"message_id":origin.get(),"text":"active background done"}),
        )
        .await
        .unwrap();
    assert_eq!(
        fixture
            .manager
            .recover_background_deliveries(128)
            .await
            .unwrap(),
        0
    );
    fixture.finish(0, "first answer");
    fixture.calls(2).await;
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1])
            .unwrap()
            .contains("active background done")
    );
    fixture.finish(1, "completion observed");
    fixture.idle(fixture.session.id).await;
    assert!(
        fixture
            .manager
            .kernel()
            .reader
            .pending_deliveries(session, None, 128)
            .await
            .unwrap()
            .is_empty()
    );
    let rounds = fixture
        .manager
        .kernel()
        .reader
        .rounds(session, None, 128)
        .await
        .unwrap();
    assert!(rounds.iter().all(|round| round.execution_id == execution));
    fixture.close().await;
}

#[tokio::test]
async fn native_checkpoint_keeps_exact_opaque_state_and_a_portable_summary_for_another_route() {
    for mode in [1, 2] {
        let fixture = Fixture::new(1).await;
        fixture.submit("portable instructions").await;
        fixture.calls(1).await;
        fixture.finish(0, "recorded outcome");
        fixture.idle(fixture.session.id).await;
        fixture
            .provider
            .checkpoint_mode
            .store(mode, Ordering::SeqCst);
        if mode == 1 {
            let (sender, receiver) = oneshot::channel();
            *fixture.provider.summary.lock().unwrap() = Some(receiver);
            sender
                .send(Ok(response(
                    "portable instructions retained through a local summary",
                )))
                .unwrap();
        }
        fixture
            .manager
            .compact_session(SessionExecutionRequest {
                session_id: fixture.session.id,
                options: fixture.options(),
            })
            .await
            .unwrap();
        let session = session_id(fixture.session.id).unwrap();
        let status = fixture
            .manager
            .compaction_status(fixture.session.id)
            .await
            .unwrap();
        assert!(status.checkpoint_entry_id.is_some());
        assert!(!status.native_compaction_unavailable);
        assert_eq!(
            fixture.provider.summary_calls.load(Ordering::SeqCst),
            usize::from(mode == 1)
        );
        assert_eq!(fixture.provider.native_calls.load(Ordering::SeqCst), 1);
        let execution = fixture
            .manager
            .kernel()
            .sessions
            .continue_execution(
                RequestId::new(),
                session,
                fixture
                    .manager
                    .latest_origin_message(session)
                    .await
                    .unwrap()
                    .unwrap(),
            )
            .await
            .unwrap()
            .value;
        let attempt = fixture
            .manager
            .kernel()
            .executor
            .claim(RequestId::new(), session, execution.id)
            .await
            .unwrap()
            .value;
        let round = attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let facts = round.prompt().await.unwrap();
        let model = ModelRef::new("fixture", "model");
        // This projection contract shares a fixture-local budget; unrelated
        // manager runs retain their own process-wide admission while streaming.
        let hydrator = crate::session::kernel::KernelPromptHydrator::isolated();
        let native = hydrator
            .hydrate(&fixture.manager.kernel().reader, facts, &model, true)
            .await
            .unwrap();
        let native_text = serde_json::to_string(native.compaction().unwrap()).unwrap();
        assert!(native_text.contains("OPAQUE+/=="));
        assert!(native_text.contains("future"));
        assert!(native_text.contains("\\u0000"));
        let other = hydrator
            .hydrate(
                &fixture.manager.kernel().reader,
                round.prompt().await.unwrap(),
                &ModelRef::new("another-route", "another-model"),
                false,
            )
            .await
            .unwrap();
        assert!(other.compaction().is_none());
        let text = serde_json::to_string(other.turns()).unwrap();
        assert!(text.contains("portable instructions"));
        assert!(!text.contains("OPAQUE+/=="));
        round.finish(RequestId::new(), None, false).await.unwrap();
        attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        fixture.close().await;
    }
}

#[tokio::test]
async fn a_native_checkpoint_without_a_portable_summary_does_not_commit_if_local_summary_fails() {
    let fixture = Fixture::new(1).await;
    fixture.submit("keep all original instructions").await;
    fixture.calls(1).await;
    fixture.finish(0, "answer");
    fixture.idle(fixture.session.id).await;
    fixture.provider.checkpoint_mode.store(1, Ordering::SeqCst);
    fixture
        .manager
        .compact_session(SessionExecutionRequest {
            session_id: fixture.session.id,
            options: fixture.options(),
        })
        .await
        .unwrap();
    let status = fixture
        .manager
        .compaction_status(fixture.session.id)
        .await
        .unwrap();
    assert!(status.checkpoint_entry_id.is_none());
    assert!(status.through_entry_id.is_none());
    assert_eq!(status.consecutive_compaction_failures, 1);
    assert_eq!(fixture.provider.native_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider.summary_calls.load(Ordering::SeqCst), 1);
    fixture.close().await;
}

#[tokio::test]
async fn tool_result_commit_rejection_preserves_the_observed_effect_instead_of_invoking_it_again() {
    let effects = Arc::new(AtomicUsize::new(0));
    let fixture = Fixture::with_effect(2, Some(effects.clone())).await;
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    let admin = sea_orm::Database::connect(format!(
        "sqlite://{}?mode=rw",
        fixture._root.path().join("sessions.db").display()
    ))
    .await
    .unwrap();
    admin.execute(Statement::from_string(DbBackend::Sqlite,"CREATE TABLE reject_tool_commit (owner INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)")).await.unwrap();
    admin.execute(Statement::from_string(DbBackend::Sqlite,"CREATE TRIGGER reject_tool_result AFTER UPDATE OF phase ON agena_tool_calls WHEN NEW.phase='succeeded' BEGIN INSERT INTO reject_tool_commit VALUES (-1); END")).await.unwrap();
    fixture.submit("invoke the controlled tool once").await;
    fixture.calls(1).await;
    fixture.events[0]
        .send(Ok(CompletionStreamEvent::ToolCallDelta {
            provider_id: ProviderId::new("fixture"),
            model: ModelId::new("model"),
            stream_key: "idx:0".into(),
            id: Some("effect-1".into()),
            name: Some("tools_call".into()),
            arguments_delta: r#"{"tool":"fixture.effect.run","input":{}}"#.into(),
        }))
        .unwrap();
    fixture.events[0]
        .send(Ok(CompletionStreamEvent::Completed {
            provider_id: ProviderId::new("fixture"),
            model: ModelId::new("model"),
            finish_reason: Some(CompletionFinishReason::ToolCalls),
            usage: None,
            provider_metadata: None,
            end_turn: Some(false),
        }))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while effects.load(Ordering::SeqCst) == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    let session = session_id(fixture.session.id).unwrap();
    let history = fixture
        .manager
        .kernel()
        .reader
        .history(session, None, 128)
        .await
        .unwrap();
    let tool_id = history
        .entries
        .iter()
        .find_map(|entry| match entry.payload {
            EntryPayload::Tool { tool_id } => Some(tool_id),
            _ => None,
        })
        .unwrap();
    assert!(
        fixture
            .manager
            .kernel()
            .reader
            .tool(session, tool_id)
            .await
            .unwrap()
            .result
            .is_none()
    );
    admin
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER reject_tool_result",
        ))
        .await
        .unwrap();
    fixture.calls(2).await;
    fixture.finish(1, "tool result observed");
    fixture.idle(fixture.session.id).await;
    let tool = fixture
        .manager
        .kernel()
        .reader
        .tool(session, tool_id)
        .await
        .unwrap();
    assert_eq!(tool.phase, ToolPhase::Succeeded);
    let result = tool.result.as_ref().unwrap();
    assert_eq!(result["tool_api_call"]["function"], "tools_call");
    assert!(result["raw_output"].is_object());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(
        serde_json::to_string(&fixture.provider.requests.lock().unwrap()[1])
            .unwrap()
            .contains("one external effect")
    );
    fixture.close().await;
}
