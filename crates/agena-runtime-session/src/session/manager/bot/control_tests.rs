use super::*;
use agena_domain::{ActivityId, ComposerActivity};
use agena_runtime_provider::provider::{
    ManagedCredential, ModelRuntime, MultiAdapterProvider, ProviderModelRoute,
};
use agena_runtime_provider_adapters::{
    OpenAiChatCompletionsAdapter, OpenAiChatCompletionsAdapterOptions,
};
use std::collections::{BTreeMap, BTreeSet};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn model(name: &str) -> ModelRef {
    ModelRef::try_new_with_adapter("configured", "chat", name).unwrap()
}

fn providers(base: &str) -> ProviderRegistry {
    // Production provider and route implementations, with explicitly configured
    // inventory. No test ModelRuntime, fake manager or application HTTP routes.
    let adapter: Arc<dyn ModelRuntime> =
        Arc::new(OpenAiChatCompletionsAdapter::new_managed_with_options(
            "chat",
            reqwest::Client::new(),
            ManagedCredential::static_value("fixture", "CONTROL_SECRET_DO_NOT_EXPOSE"),
            base,
            "alpha",
            OpenAiChatCompletionsAdapterOptions::default(),
        ));
    let definition: agena_provider::ConfiguredModelDefinition = serde_json::from_value(json!({
        "supports_parallel_tool_calls":true,"supports_verbosity":true,
        "verbosity_levels":["low","high"],"default_verbosity":"low",
        "thinking_modes":{"default":"off","off":{"strategy":"disabled"}},
        "speed_modes":{"default":"normal","normal":{},"fast":{}}
    }))
    .unwrap();
    let routes = ["alpha", "beta", "disabled"]
        .into_iter()
        .map(|name| {
            (
                ("chat".into(), name.into()),
                ProviderModelRoute {
                    enabled: name != "disabled",
                    native_compaction: false,
                    agena_tool_mode: None,
                    provider_native_tools: Default::default(),
                    definition: definition.clone(),
                },
            )
        })
        .collect();
    let mut registry = ProviderRegistry::new();
    registry.register(MultiAdapterProvider::new(
        "configured",
        "chat",
        "alpha",
        BTreeMap::from([("chat".into(), adapter)]),
        routes,
        BTreeSet::from(["chat".into()]),
    ));
    registry
}

async fn fixture(base: &str) -> (SessionManager, i64, SessionId) {
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
    let target = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Target".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
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
        Arc::new(ProviderRegistry::new()),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        Default::default(),
    );
    manager.install_memory_preferences(preferences);
    let previous = manager.state.load_full();
    let default_selection = ExecutionSelection {
        provider: Some("configured".into()),
        adapter: Some("chat".into()),
        model: Some("alpha".into()),
        ..Default::default()
    };
    manager.state.store(Arc::new(ExecutionState {
        providers: Arc::new(providers(base)),
        config: RuntimeSessionManagerConfig {
            default_selection,
            ..previous.config.clone()
        },
        ..(*previous).clone()
    }));
    let source = manager.ensure_bot_session().await.unwrap();
    (manager, source, target)
}

async fn act(
    manager: &SessionManager,
    source: i64,
    target: SessionId,
    mut input: Value,
) -> Result<Value, AppError> {
    input["session_id"] = json!(target.get());
    let request = serde_json::from_value(input).unwrap();
    manager
        .bot_control(
            source,
            1,
            &request,
            RequestId::new(),
            Some(1),
            Some("fixture-operation"),
        )
        .await
}
async fn control_version(manager: &SessionManager, target: SessionId) -> i64 {
    *manager
        .kernel()
        .reader
        .control_version(target)
        .await
        .unwrap()
}
async fn version(manager: &SessionManager, target: SessionId) -> i64 {
    manager
        .kernel()
        .reader
        .metadata(target)
        .await
        .unwrap()
        .metadata_version
        .get()
}
fn document(text: &str) -> ComposerDocument {
    ComposerDocument(vec![ComposerNode::Text { text: text.into() }])
}
async fn human(manager: &SessionManager, target: SessionId, text: &str) -> ExecutionId {
    let metadata = manager.kernel().reader.metadata(target).await.unwrap();
    let options = manager
        .options_for_selection(&metadata.selection.clone().unwrap_or_default())
        .unwrap();
    manager
        .submit(crate::SessionUserRunRequest::new(
            target.get(),
            options,
            document(text),
        ))
        .await
        .unwrap()
        .receipt
        .unwrap()
        .execution_id
}
async fn queued(
    manager: &SessionManager,
    target: SessionId,
    document: ComposerDocument,
) -> ExecutionView {
    manager
        .kernel()
        .sessions
        .send(RequestId::new(), target, document)
        .await
        .unwrap()
        .value
}

#[tokio::test]
async fn models_are_actual_configured_inventory_with_effective_defaults_and_scoped_pagination() {
    // An unreachable endpoint proves that Models performs no network discovery.
    let (manager, source, target) = fixture("http://127.0.0.1:9").await;
    let first = act(
        &manager,
        source,
        target,
        json!({"action":"models","limit":1}),
    )
    .await
    .unwrap();
    assert_eq!(first["models"].as_array().unwrap().len(), 1);
    assert_eq!(first["models"][0]["model"], json!(model("alpha")));
    assert_eq!(first["effective_model"], json!(model("alpha")));
    assert_eq!(first["effective_modes"]["thinking_mode"], "off");
    assert_eq!(first["effective_modes"]["speed_mode"], "normal");
    assert_eq!(first["default_selection"]["model"], "alpha");
    assert_eq!(first["models"][0]["verbosity"], json!(["low", "high"]));
    let all = act(&manager, source, target, json!({"action":"models"}))
        .await
        .unwrap();
    let encoded = all.to_string();
    assert!(!encoded.contains("disabled"));
    assert!(!encoded.contains("CONTROL_SECRET"));
    assert!(!encoded.contains("127.0.0.1"));
    assert!(!encoded.contains("request_override"));
    assert_eq!(all["models"].as_array().unwrap().len(), 2);
    let next = act(
        &manager,
        source,
        target,
        json!({"action":"models","cursor":first["next_cursor"]}),
    )
    .await
    .unwrap();
    assert_eq!(next["models"][0]["model"], json!(model("beta")));
    assert!(
        act(
            &manager,
            source,
            target,
            json!({"action":"models","query":"beta","cursor":first["next_cursor"]})
        )
        .await
        .is_err()
    );
    let filtered = act(
        &manager,
        source,
        target,
        json!({"action":"models","query":"beta"}),
    )
    .await
    .unwrap();
    assert_eq!(filtered["models"].as_array().unwrap().len(), 1);
    assert_eq!(filtered["models"][0]["model"], json!(model("beta")));
}

#[tokio::test]
async fn configure_invalid_model_or_modes_roll_back_and_old_version_cannot_overwrite_manual_edits()
{
    let (manager, source, target) = fixture("http://127.0.0.1:9").await;
    let before = manager.kernel().reader.metadata(target).await.unwrap();
    for patch in [
        json!({"model":model("invented")}),
        json!({"model":model("disabled")}),
        json!({"speed_mode":"invented"}),
        json!({"thinking_mode":"invented"}),
        json!({"verbosity":"invented"}),
        json!({"model":model("beta"),"speed_mode":"invented"}),
        json!({"reset_model":true,"model":model("beta")}),
    ] {
        let mut input =
            json!({"action":"configure","expected_version":before.metadata_version.get()});
        input
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        assert!(act(&manager, source, target, input).await.is_err());
        assert_eq!(
            manager.kernel().reader.metadata(target).await.unwrap(),
            before
        );
    }
    manager
        .kernel()
        .sessions
        .update_metadata(
            RequestId::new(),
            target,
            MetadataChange {
                title: Some("Human title".into()),
                favorite: Some(true),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let conflict = act(&manager,source,target,json!({"action":"configure","expected_version":before.metadata_version.get(),"model":model("beta")})).await.unwrap_err();
    assert!(matches!(conflict, AppError::PersistenceConflict(_)));
    let updated_version = version(&manager, target).await;
    let configured = act(&manager,source,target,json!({"action":"configure","expected_version":updated_version,"model":model("beta"),"speed_mode":"fast","thinking_mode":"off","parallel_tool_calls":false})).await.unwrap();
    assert_eq!(configured["version"], updated_version + 1); // One atomic patch.
    assert_eq!(configured["in_flight_run_affected"], false);
    let metadata = manager.kernel().reader.metadata(target).await.unwrap();
    assert_eq!(metadata.title, "Human title");
    assert!(metadata.favorite);
    let selection = metadata.selection.clone().unwrap();
    assert_eq!(selection.model.as_deref(), Some("beta"));
    assert_eq!(selection.speed_mode.as_deref(), Some("fast"));
    let patch = act(&manager,source,target,json!({"action":"configure","expected_version":metadata.metadata_version.get(),"verbosity":"high"})).await.unwrap();
    let after = manager.kernel().reader.metadata(target).await.unwrap();
    assert_eq!(
        after.selection.as_ref().unwrap().speed_mode,
        selection.speed_mode
    );
    assert_eq!(
        after.selection.as_ref().unwrap().parallel_tool_calls,
        Some(false)
    );
    let reset = act(
        &manager,
        source,
        target,
        json!({"action":"configure","expected_version":patch["version"],"reset_model":true}),
    )
    .await
    .unwrap();
    assert_eq!(reset["status"], "configured");
    let models = act(&manager, source, target, json!({"action":"models"}))
        .await
        .unwrap();
    assert_eq!(models["effective_model"], json!(model("alpha")));
    assert_eq!(models["selection"]["speed_mode"], "fast");
    assert_eq!(models["selection"]["verbosity"], "high");
}

#[tokio::test]
async fn native_patch_and_queue_writer_replay_once_and_fence_manual_races() {
    let (manager, _, target) = fixture("http://127.0.0.1:9").await;
    let before = version(&manager, target).await;
    let operation = RequestId::new();
    let changes = vec![
        SelectionChange::Model(Some(model("beta"))),
        SelectionChange::SpeedMode(Some("fast".into())),
    ];
    let first = manager
        .kernel()
        .sessions
        .patch_selection(operation, target, before, changes.clone())
        .await
        .unwrap();
    manager
        .rename_session(target.get(), "Later human title".into())
        .await
        .unwrap();
    let retry = manager
        .kernel()
        .sessions
        .patch_selection(operation, target, before, changes.clone())
        .await
        .unwrap();
    assert_eq!(first, retry);
    let current = manager.kernel().reader.metadata(target).await.unwrap();
    assert!(
        manager
            .kernel()
            .sessions
            .patch_selection(RequestId::new(), target, before, changes)
            .await
            .is_err()
    );
    assert_eq!(
        manager.kernel().reader.metadata(target).await.unwrap(),
        current
    );
    let input = queued(&manager, target, document("do not repeat remove")).await;
    let guard = control_version(&manager, target).await;
    let key = RequestId::new();
    let remove = QueueChange::Remove {
        execution: input.id,
        generation: input.generation,
    };
    let first = manager
        .kernel()
        .sessions
        .control_queue(key, target, guard, remove.clone())
        .await
        .unwrap();
    let newer = queued(&manager, target, document("newer human input")).await;
    let retry = manager
        .kernel()
        .sessions
        .control_queue(key, target, guard, remove.clone())
        .await
        .unwrap();
    assert_eq!(first, retry);
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(target, newer.id)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Queued
    );
    assert!(
        manager
            .kernel()
            .sessions
            .control_queue(RequestId::new(), target, guard, remove)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn stale_generation_edit_and_remove_have_no_effect_on_newer_running_attempt() {
    let (manager, source, target) = fixture("http://127.0.0.1:9").await;
    let input = queued(&manager, target, document("keep me")).await;
    manager
        .kernel()
        .sessions
        .cancel_queued(RequestId::new(), target, input.id)
        .await
        .unwrap();
    let retried = manager
        .kernel()
        .sessions
        .retry(RequestId::new(), target, input.id)
        .await
        .unwrap()
        .value;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), target, input.id)
        .await
        .unwrap();
    let current = manager
        .kernel()
        .reader
        .execution(target, input.id)
        .await
        .unwrap();
    assert_ne!(current.generation, input.generation);
    assert_eq!(current.generation, retried.generation);
    let guard = control_version(&manager, target).await;
    for action in ["queue_edit", "queue_remove"] {
        let mut input_request = json!({"action":action,"expected_control_version":guard,"execution_id":input.id,"expected_generation":input.generation.get()});
        if action == "queue_edit" {
            input_request["instruction"] = json!("overwrite");
        }
        assert!(act(&manager, source, target, input_request).await.is_err());
        assert_eq!(
            manager
                .kernel()
                .reader
                .execution(target, input.id)
                .await
                .unwrap(),
            current
        );
        assert_eq!(control_version(&manager, target).await, guard);
    }
    let entry = manager
        .kernel()
        .reader
        .entry(target, input.message_id)
        .await
        .unwrap();
    assert!(
        matches!(&entry.payload,EntryPayload::User {document, ..} if document.text() == "keep me")
    );
}

struct WireFixture {
    requests: Arc<StdMutex<Vec<Value>>>,
    held: Arc<tokio::sync::Semaphore>,
    arrived: Arc<Notify>,
    task: tokio::task::JoinHandle<()>,
    base: String,
}
impl Drop for WireFixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl WireFixture {
    async fn start() -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let requests = Arc::new(StdMutex::new(Vec::new()));
        let held = Arc::new(tokio::sync::Semaphore::new(0));
        let arrived = Arc::new(Notify::new());
        let (captured, gate, ready) = (requests.clone(), held.clone(), arrived.clone());
        let task = tokio::spawn(async move {
            while let Ok((mut socket, _)) = listener.accept().await {
                let (requests, held, arrived) = (captured.clone(), gate.clone(), ready.clone());
                tokio::spawn(async move {
                    let mut bytes = Vec::new();
                    let (header_end, length) = loop {
                        let mut chunk = [0u8; 8192];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&chunk[..count]);
                        if let Some(end) = bytes.windows(4).position(|w| w == b"\r\n\r\n") {
                            let head = String::from_utf8_lossy(&bytes[..end]);
                            if head.starts_with("GET ") {
                                let body = json!({"object":"list","data":[{"id":"cached-real-model","object":"model","owned_by":"fixture"}]}).to_string();
                                let response = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                                    body.len()
                                );
                                socket.write_all(response.as_bytes()).await.unwrap();
                                return;
                            }
                            let length: usize = head
                                .lines()
                                .find_map(|line| {
                                    line.split_once(':')
                                        .filter(|(key, _)| {
                                            key.eq_ignore_ascii_case("content-length")
                                        })
                                        .map(|(_, value)| value.trim().parse().unwrap())
                                })
                                .unwrap();
                            break (end + 4, length);
                        }
                    };
                    while bytes.len() < header_end + length {
                        let mut chunk = [0u8; 8192];
                        let count = socket.read(&mut chunk).await.unwrap();
                        assert!(count > 0);
                        bytes.extend_from_slice(&chunk[..count]);
                    }
                    let value: Value =
                        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap();
                    let first = {
                        let mut all = requests.lock().unwrap();
                        all.push(value);
                        all.len() == 1
                    };
                    arrived.notify_one();
                    if first {
                        held.acquire().await.unwrap().forget();
                    }
                    let event = json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"alpha","choices":[{"index":0,"delta":{"content":"done"},"finish_reason":null}]});
                    let stop = json!({"id":"fixture","object":"chat.completion.chunk","created":1,"model":"alpha","choices":[{"index":0,"delta":{},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":2}});
                    let body = format!("data: {event}\n\ndata: {stop}\n\ndata: [DONE]\n\n");
                    let response = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    socket.write_all(response.as_bytes()).await.unwrap();
                });
            }
        });
        Self {
            requests,
            held,
            arrived,
            task,
            base,
        }
    }
    async fn wait_count(&self, count: usize) {
        tokio::time::timeout(Duration::from_secs(15), async {
            loop {
                let ready = self.arrived.notified();
                if self.requests.lock().unwrap().len() >= count {
                    break;
                }
                ready.await;
            }
        })
        .await
        .unwrap();
    }
}
async fn settled(
    manager: &SessionManager,
    target: SessionId,
    execution: ExecutionId,
) -> ExecutionPhase {
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let phase = manager
                .kernel()
                .reader
                .execution(target, execution)
                .await
                .unwrap()
                .phase;
            if !matches!(phase, ExecutionPhase::Queued | ExecutionPhase::Running) {
                return phase;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn held_native_run_pause_edit_remove_resume_preserves_attachments_fifo_and_human_continuation()
 {
    let wire = WireFixture::start().await;
    let (manager, source, target) = fixture(&wire.base).await;
    let first = human(&manager, target, "held first human input").await;
    wire.wait_count(1).await;
    act(&manager,source,target,json!({"action":"queue_pause","paused":true,"expected_control_version":control_version(&manager,target).await})).await.unwrap();
    let attachment:ComposerActivity = serde_json::from_value(json!({"id":ActivityId::new(),"payload":{"activity_type":"resource","delivery":"reference","kind":"file","reference":{"reference_type":"workspace_path","path":"kept.txt"},"name":"kept.txt"}})).unwrap();
    let with_attachment = ComposerDocument(vec![
        ComposerNode::Text {
            text: "old second text".into(),
        },
        ComposerNode::activity(attachment.clone()),
    ]);
    let bytes = b"immutable attached file bytes";
    let mut source_bytes = bytes.as_slice();
    let blob = manager
        .kernel()
        .sessions
        .prepare_blob(
            RequestId::new(),
            target,
            bytes.len() as u64,
            &mut source_bytes,
        )
        .await
        .unwrap();
    let blob_id = blob.descriptor().id;
    let second = manager
        .kernel()
        .sessions
        .send_with_blobs(RequestId::new(), target, with_attachment, &[blob])
        .await
        .unwrap()
        .value;
    let blobs_before = manager
        .kernel()
        .reader
        .entry_blobs(target, second.message_id)
        .await
        .unwrap();
    let removed = human(&manager, target, "remove this queued input").await;
    let fourth = human(&manager, target, "fourth human queued input").await;
    let queue = act(&manager, source, target, json!({"action":"queue"}))
        .await
        .unwrap();
    assert_eq!(queue["paused"], true);
    assert_eq!(queue["total"], 3);
    assert_eq!(queue["entries"][0]["execution_id"], json!(second.id));
    assert_eq!(queue["entries"][0]["attachments"][0]["name"], "kept.txt");
    let edit = act(&manager,source,target,json!({"action":"queue_edit","execution_id":second.id,"expected_generation":second.generation.get(),"expected_control_version":queue["control_version"],"instruction":"edited second text"})).await.unwrap();
    assert_eq!(edit["attachments_preserved"], true);
    let entry = manager
        .kernel()
        .reader
        .entry(target, second.message_id)
        .await
        .unwrap();
    assert!(
        matches!(&entry.payload,EntryPayload::User {document, ..} if document.text() == "edited second text" && document.0.contains(&ComposerNode::activity(attachment)))
    );
    let blobs_after = manager
        .kernel()
        .reader
        .entry_blobs(target, second.message_id)
        .await
        .unwrap();
    assert_eq!(blobs_before, blobs_after);
    let saved_file = manager
        .kernel()
        .reader
        .open_blob(target, blob_id)
        .await
        .unwrap();
    assert_eq!(saved_file.range(0, 1024).await.unwrap().bytes, bytes);
    let stale = act(&manager,source,target,json!({"action":"queue_remove","execution_id":removed,"expected_generation":1,"expected_control_version":queue["control_version"]})).await.unwrap_err();
    assert!(matches!(stale, AppError::PersistenceConflict(_)));
    let remove = act(&manager,source,target,json!({"action":"queue_remove","execution_id":removed,"expected_generation":1,"expected_control_version":control_version(&manager,target).await})).await.unwrap();
    assert_eq!(remove["status"], "removed");
    // Configuration while a provider request is held cannot mutate its options.
    act(&manager,source,target,json!({"action":"configure","expected_version":version(&manager,target).await,"model":model("beta"),"parallel_tool_calls":false})).await.unwrap();
    assert_eq!(wire.requests.lock().unwrap()[0]["model"], "alpha");
    wire.held.add_permits(1);
    assert_eq!(
        settled(&manager, target, first).await,
        ExecutionPhase::Succeeded
    );
    assert_eq!(wire.requests.lock().unwrap().len(), 1); // Pause still fences queued claims.
    act(&manager,source,target,json!({"action":"queue_pause","paused":false,"expected_control_version":control_version(&manager,target).await})).await.unwrap();
    assert_eq!(
        settled(&manager, target, second.id).await,
        ExecutionPhase::Succeeded
    );
    assert_eq!(
        settled(&manager, target, fourth).await,
        ExecutionPhase::Succeeded
    );
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(target, removed)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    let continuation = human(&manager, target, "human continues normally").await;
    assert_eq!(
        settled(&manager, target, continuation).await,
        ExecutionPhase::Succeeded
    );
    let requests = wire.requests.lock().unwrap();
    assert_eq!(requests.len(), 4);
    assert_eq!(requests[3]["model"], "beta");
    assert_eq!(requests[3]["parallel_tool_calls"], false);
    let latest_text = |request: &Value| {
        request["messages"]
            .as_array()
            .unwrap()
            .iter()
            .rev()
            .find(|message| message["role"] == "user")
            .unwrap()["content"]
            .to_string()
    };
    assert!(latest_text(&requests[0]).contains("held first human input"));
    assert!(latest_text(&requests[1]).contains("edited second text"));
    assert!(latest_text(&requests[2]).contains("fourth human queued input"));
    assert!(latest_text(&requests[3]).contains("human continues normally"));
}

#[tokio::test]
async fn prepared_control_retries_replay_after_new_manual_state_and_target_deletion() {
    let (manager, source, target) = fixture("http://127.0.0.1:9").await;
    let configure: BotRequest =
        serde_json::from_value(json!({"action":"configure","session_id":target.get(),
        "expected_version":version(&manager,target).await,"model":model("beta")}))
        .unwrap();
    let key = RequestId::new();
    let first = manager
        .bot_control(source, 1, &configure, key, Some(1), Some("configure"))
        .await
        .unwrap();
    manager
        .rename_session(target.get(), "new manual title".into())
        .await
        .unwrap();
    manager
        .kernel()
        .sessions
        .update_selection(
            RequestId::new(),
            target,
            SelectionChange::SpeedMode(Some("fast".into())),
        )
        .await
        .unwrap();
    let current = manager.kernel().reader.metadata(target).await.unwrap();
    let replay = manager
        .bot_control(source, 1, &configure, key, Some(1), Some("configure"))
        .await
        .unwrap();
    assert_eq!(first, replay);
    assert_eq!(
        manager.kernel().reader.metadata(target).await.unwrap(),
        current
    );
    let pause: BotRequest =
        serde_json::from_value(json!({"action":"queue_pause","session_id":target.get(),
        "expected_control_version":control_version(&manager,target).await,"paused":true}))
        .unwrap();
    let pause_key = RequestId::new();
    manager
        .bot_control(source, 1, &pause, pause_key, Some(1), Some("pause"))
        .await
        .unwrap();
    manager
        .set_message_queue_paused(target.get(), false)
        .await
        .unwrap();
    manager
        .bot_control(source, 1, &pause, pause_key, Some(1), Some("pause"))
        .await
        .unwrap();
    assert!(
        !manager
            .kernel()
            .reader
            .configuration(target)
            .await
            .unwrap()
            .message_queue_paused
    );
    manager
        .kernel()
        .sessions
        .delete(RequestId::new(), target)
        .await
        .unwrap();
    assert_eq!(
        manager
            .bot_control(source, 1, &configure, key, Some(1), Some("configure"))
            .await
            .unwrap(),
        first
    );
    let deleted = manager
        .bot_control(source, 1, &pause, pause_key, Some(1), Some("pause"))
        .await
        .unwrap();
    assert_eq!(deleted["accepted"], true);
    assert_eq!(deleted["target_deleted"], true);
    assert!(deleted["control_version"].is_null());
}

#[tokio::test]
async fn native_discovery_cache_exposes_observed_models_without_guessing_the_default() {
    let wire = WireFixture::start().await;
    let mut registry = ProviderRegistry::new();
    registry.register(OpenAiChatCompletionsAdapter::new_managed_with_options(
        "remote",
        reqwest::Client::new(),
        ManagedCredential::static_value("fixture", "cache-secret"),
        &wire.base,
        "unlisted-default",
        OpenAiChatCompletionsAdapterOptions::default(),
    ));
    assert!(registry.selection_model_inventory().is_empty());
    // Only the existing human/native inventory path performs discovery.
    let discovered = registry.list_models("remote").await.unwrap();
    assert_eq!(discovered.len(), 1);
    assert_eq!(
        registry.selection_model_inventory()[0].id.as_ref(),
        "cached-real-model"
    );
    assert!(
        !registry
            .selection_model_inventory()
            .iter()
            .any(|model| model.id.as_ref() == "unlisted-default")
    );
    // Removing/replacing a provider invalidates its observed inventory.
    registry.remove("remote");
    assert!(registry.selection_model_inventory().is_empty());
}
