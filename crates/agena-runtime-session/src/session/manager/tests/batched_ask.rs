//! Regression: two `ask` calls in one model batch.
//!
//! Batch fan-out no longer special-cases interactive tools, so both asks are in
//! flight at the same time and each owns an independent host user-input
//! request. When the batch reloads the session projection to apply its results,
//! the other tool's request/reply records must survive that merge.
use super::*;
use agena_plugin_host::sdk::host_api::{
    AskUserOption, AskUserQuestion, AskUserRequest, AskUserResponse, EventSubscription, HostClient,
    LogLevel,
};
use agena_plugin_host::sdk::{EventEnvelope, EventFilter, InitContext, InitOutcome, Plugin};

/// Routes a plugin's `ask_user` into the session manager through the callback
/// context the live runtime installs, so a batched ask is exercised end to end
/// without composing a whole `AgenaRuntime` in this crate.
#[derive(Default)]
struct AskRoutingHostClient {
    manager: std::sync::Mutex<Option<SessionManager>>,
}

impl AskRoutingHostClient {
    fn attach(&self, manager: &SessionManager) {
        *self.manager.lock().expect("ask routing lock") = Some(manager.background_handle());
    }
}

#[async_trait::async_trait]
impl HostClient for AskRoutingHostClient {
    async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

    async fn publish_event(&self, _env: EventEnvelope) -> agena_plugin_host::sdk::Result<()> {
        Ok(())
    }

    async fn subscribe_events(
        &self,
        _filter: EventFilter,
    ) -> agena_plugin_host::sdk::Result<EventSubscription> {
        Ok(EventSubscription {
            id: "unused".to_owned(),
        })
    }

    async fn read_config(
        &self,
        _path: Option<String>,
    ) -> agena_plugin_host::sdk::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }

    async fn invoke_tool(
        &self,
        _tool: String,
        _input: serde_json::Value,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        Err(agena_plugin_host::sdk::PluginError::internal(
            "the ask fixture does not invoke tools",
        ))
    }

    async fn ask_user(
        &self,
        req: AskUserRequest,
    ) -> agena_plugin_host::sdk::Result<AskUserResponse> {
        let context =
            agena_plugin_host::sdk::host_api::current_host_callback_context().unwrap_or_default();
        let (Some(session_id), Some(call_id)) = (context.session_id, context.call_id) else {
            return Err(agena_plugin_host::sdk::PluginError::internal(
                "the ask fixture ran outside a session tool call",
            ));
        };
        let request = ask_user_tool_input(req)?;
        // Clone the manager handle out of the lock before awaiting: holding a
        // std lock across an await would serialize the two batched asks.
        let handle = self
            .manager
            .lock()
            .expect("ask routing lock")
            .as_ref()
            .map(SessionManager::background_handle)
            .ok_or_else(|| {
                agena_plugin_host::sdk::PluginError::internal(
                    "the ask fixture has no session manager attached",
                )
            })?;
        handle
            .request_host_user_input(session_id, call_id, request)
            .await
            .map_err(|error| agena_plugin_host::sdk::PluginError::internal(error.to_string()))
    }
}

/// Project a plugin `AskUserRequest` onto the session-owned input type. The live
/// runtime keeps this conversion in its host-client mappers; the fixture needs
/// the same projection and the same validation.
fn ask_user_tool_input(
    req: AskUserRequest,
) -> agena_plugin_host::sdk::Result<crate::part::AskUserToolInput> {
    let questions = req
        .questions
        .into_iter()
        .map(|question| UserInputQuestion {
            header: question.header,
            question: question.question,
            options: question
                .options
                .into_iter()
                .map(|option| UserInputOption {
                    label: option.label,
                    description: option.description,
                })
                .collect(),
            multiple: question.multiple,
            allow_custom: question.allow_custom,
        })
        .collect::<Vec<_>>();
    crate::part::AskUserToolInput::parse_input(serde_json::json!({
        "title": req.title,
        "kind": req.kind,
        "body_markdown": req.body_markdown,
        "auto_resolution_ms": req.auto_resolution_ms,
        "questions": questions,
    }))
    .map_err(|error| agena_plugin_host::sdk::PluginError::invalid_params_error(&error))
}

/// Two ask tools in one plugin, each with its own question, so the two asks in
/// a batch can never be mistaken for one another.
struct AskFixture {
    host: std::sync::RwLock<Option<Arc<dyn HostClient>>>,
}

impl AskFixture {
    fn new() -> Self {
        Self {
            host: std::sync::RwLock::new(None),
        }
    }

    fn host(&self) -> agena_plugin_host::sdk::Result<Arc<dyn HostClient>> {
        self.host
            .read()
            .map_err(|_| {
                agena_plugin_host::sdk::PluginError::internal("ask fixture lock poisoned")
            })?
            .clone()
            .ok_or_else(|| {
                agena_plugin_host::sdk::PluginError::internal("ask fixture invoked before init")
            })
    }

    async fn ask(
        &self,
        title: &str,
        question: &str,
        option: &str,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        let response = self
            .host()?
            .ask_user(AskUserRequest {
                title: title.to_owned(),
                kind: "ask_user".to_owned(),
                body_markdown: String::new(),
                auto_resolution_ms: None,
                questions: vec![AskUserQuestion {
                    header: String::new(),
                    question: question.to_owned(),
                    options: vec![AskUserOption {
                        label: option.to_owned(),
                        description: String::new(),
                    }],
                    multiple: false,
                    allow_custom: false,
                }],
                prompt: String::new(),
                options: Vec::new(),
                allow_free_text: false,
            })
            .await?;
        if response.cancelled || response.timed_out {
            return Err(agena_plugin_host::sdk::PluginError::internal(
                "the ask fixture was not answered",
            ));
        }
        Ok(agena_plugin_host::sdk::ToolInvokeOutput::text(format!(
            "{title}: {}",
            response
                .answers
                .get("0")
                .cloned()
                .unwrap_or_default()
                .join(", ")
        )))
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "askfixture",
    version = "test",
    summary = "Batched ask regression fixture"
)]
impl AskFixture {
    #[hook(init)]
    async fn init(
        &self,
        _ctx: InitContext,
        host: Arc<dyn HostClient>,
    ) -> agena_plugin_host::sdk::Result<InitOutcome> {
        *self.host.write().map_err(|_| {
            agena_plugin_host::sdk::PluginError::internal("ask fixture lock poisoned")
        })? = Some(host);
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }

    #[tool(
        name = "ask_first",
        summary = "Ask the first question.",
        tags(interactive)
    )]
    async fn ask_first(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.ask("First Ask", "Which flavor?", "Vanilla").await
    }

    #[tool(
        name = "ask_second",
        summary = "Ask the second question.",
        tags(interactive)
    )]
    async fn ask_second(
        &self,
    ) -> agena_plugin_host::sdk::Result<agena_plugin_host::sdk::ToolInvokeOutput> {
        self.ask("Second Ask", "Which size?", "Large").await
    }
}

/// Streams one `tools_call` per ask in the first request, then the final answer
/// once both asks have been answered.
struct BatchedAskProvider {
    model: ModelId,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
}

impl BatchedAskProvider {
    fn new() -> Self {
        Self {
            model: ModelId::new("batched-ask-model"),
            requests: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn request_count(&self) -> usize {
        self.requests.lock().expect("request lock").len()
    }

    fn requests(&self) -> Vec<CompletionRequest> {
        self.requests.lock().expect("request lock").clone()
    }
}

#[async_trait::async_trait]
impl ModelRuntime for BatchedAskProvider {
    fn id(&self) -> &str {
        "batched-ask"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn agena_tool_mode(&self, _model: &ModelId) -> agena_provider::AgenaToolMode {
        agena_provider::AgenaToolMode::ProviderProtocol
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        panic!("the batched ask fixture drives streaming turns only")
    }

    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        let index = {
            let mut requests = self.requests.lock().expect("request lock");
            let index = requests.len();
            requests.push(request);
            index
        };
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let mut events = Vec::new();
        match index {
            0 => {
                for (position, target) in ["askfixture.ask_first", "askfixture.ask_second"]
                    .into_iter()
                    .enumerate()
                {
                    events.push(Ok(CompletionStreamEvent::ToolCallSnapshot {
                        provider_id: provider_id.clone(),
                        model: model.clone(),
                        stream_key: format!("call:{position}"),
                        id: Some(format!("batched_ask_{position}")),
                        name: Some("tools_call".to_owned()),
                        arguments_json: serde_json::json!({
                            "tool": target,
                            "input": {},
                        })
                        .to_string(),
                    }));
                }
            }
            1 => {
                events.push(Ok(CompletionStreamEvent::TextDelta {
                    provider_id: provider_id.clone(),
                    model: model.clone(),
                    delta: "BATCHED_ASK_OK".to_owned(),
                }));
            }
            other => panic!("the stable run made an unexpected provider request #{other}"),
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: Some(if index == 0 {
                CompletionFinishReason::ToolCalls
            } else {
                CompletionFinishReason::Stop
            }),
            usage: Some(CompletionUsage::default()),
            provider_metadata: None,
            end_turn: None,
        }));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_asks_in_one_batch_project_their_own_answers() {
    let host_client = Arc::new(AskRoutingHostClient::default());
    let provider = Arc::new(BatchedAskProvider::new());
    let workspace_root = std::env::current_dir().expect("resolve test workspace");
    let mut plugins_config = PluginsConfig::default();
    for plugin_id in ["agena.tools", "agena.askfixture"] {
        plugins_config
            .list
            .insert(plugin_id.to_owned(), ConfiguredPlugin::static_default());
    }
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: vec![
            StaticPluginRegistration::new(
                "agena.tools".parse().expect("valid Tool API plugin key"),
                ToolSearchFixture,
            ),
            StaticPluginRegistration::new(
                "agena.askfixture"
                    .parse()
                    .expect("valid ask fixture plugin key"),
                AskFixture::new(),
            ),
        ],
        config: plugins_config,
        workspace_root: workspace_root.clone(),
        agena_version: "test".to_owned(),
        callback_base_url: None,
        host_client: Some(Arc::clone(&host_client) as Arc<dyn HostClient>),
        previous: None,
        previous_plugins: HashMap::new(),
    })
    .await
    .expect("build batched ask plugin host");
    let executor = ToolExecutor::new(
        workspace_root.clone(),
        ExecutionPrincipal::new(
            PermissionPolicy::allow_all(),
            ToolPermissionPolicy::allow_all(),
        ),
        Arc::clone(&plugins),
        None,
        None,
    );
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider.clone());
    let database = Database::connect("sqlite::memory:")
        .await
        .expect("open test database");
    initialize(&database).await;
    let manager = Arc::new(SessionManager::new(
        database,
        Arc::new(registry),
        ContextGovernor::new(agena_domain::ContextPolicy::default()),
        SessionProcessor::new(plugins),
        executor,
        RuntimeSessionManagerConfig::default(),
    ));
    host_client.attach(&manager);
    let session = create(&manager, "batched ask projection").await;

    let mut request_override = agena_domain::ModelSpeedModeRequestOverride::default();
    request_override.set_parallel_tool_calls(Some(true));
    let request = SessionUserRunRequest::new(
        session.id,
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("batched-ask", "batched-ask-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override,
            system: Some("Ask both questions, then finish.".to_owned()),
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        },
        vec![TypedContent::Text(text_content("Ask the two questions."))],
    );
    let run_manager = Arc::clone(&manager);
    let execution =
        tokio::spawn(async move { run_manager.submit_subtask_user_message(request, None).await });

    // Both asks must be durable at the same time: neither may wait for the
    // other to finish first.
    let store = manager.session_store();
    let (first_part_id, second_part_id) = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        wait_for_two_pending_asks(&store, session.id),
    )
    .await
    .expect("both batched asks must become durable without waiting on each other");

    let pending = store.load(session.id).await.expect("reload batched asks");
    let ask_of = |part_id: i64| {
        pending
            .parts
            .iter()
            .find(|part| part.part_id == part_id)
            .and_then(tool_part_first_user_input)
            .unwrap_or_else(|| panic!("ask part {part_id} must carry a durable request"))
    };
    let first_request = ask_of(first_part_id);
    let second_request = ask_of(second_part_id);
    assert_ne!(
        first_request.request.request_id, second_request.request.request_id,
        "two asks in one batch own two independent host requests"
    );
    assert_eq!(
        (
            first_request.request.questions[0].question.as_str(),
            second_request.request.questions[0].question.as_str()
        ),
        ("Which flavor?", "Which size?"),
        "each ask keeps its own question across the concurrent projection"
    );

    let reply = |request_id: String, answer: &str| {
        crate::SessionExecutionReplyRequest::new(
            session.id,
            agena_runtime::SessionRunOptions {
                model: ModelRef::new("batched-ask", "batched-ask-model"),
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
            UserInputReply {
                request_id,
                kind: UserInputReplyKind::Submit,
                answers: BTreeMap::from([("0".to_owned(), vec![answer.to_owned()])]),
                reason: None,
            },
        )
    };
    manager
        .reply_user_input(reply(first_request.request.request_id.clone(), "Vanilla"))
        .await
        .expect("the first batched ask accepts its answer");
    manager
        .reply_user_input(reply(second_request.request.request_id.clone(), "Large"))
        .await
        .expect("the second batched ask accepts its answer");

    let completed = tokio::time::timeout(std::time::Duration::from_secs(15), execution)
        .await
        .expect("the batched asks must not wedge the run")
        .expect("run task joined")
        .expect("the stable run finishes after both asks are answered");

    // The batch reloads the session projection before applying each result.
    // Both asks must survive that merge with their own answer, rather than one
    // overwriting the other's record.
    let persisted = store.load(session.id).await.expect("reload answered batch");
    let answered = |part_id: i64, expected_question: &str, expected_answer: &str| {
        let part = persisted
            .parts
            .iter()
            .find(|part| part.part_id == part_id)
            .unwrap_or_else(|| panic!("ask part {part_id} survived the batch merge"));
        let record = tool_part_first_user_input(part).expect("the answered ask keeps its record");
        assert_eq!(
            record.request.questions[0].question, expected_question,
            "the record still belongs to its own ask"
        );
        let record_reply = record.reply.expect("the durable reply is recorded");
        assert_eq!(
            record_reply.answers.get("0").map(Vec::as_slice),
            Some(&[expected_answer.to_owned()][..]),
            "ask `{expected_question}` keeps its own answer"
        );
    };
    answered(first_part_id, "Which flavor?", "Vanilla");
    answered(second_part_id, "Which size?", "Large");

    let tool_states = completed
        .parts()
        .iter()
        .filter(|part| part.kind == "tool_call")
        .map(|part| part.state)
        .collect::<Vec<_>>();
    assert_eq!(tool_states.len(), 2);
    assert!(
        tool_states
            .iter()
            .all(|state| *state == PartState::Completed),
        "both answered asks complete: {tool_states:?}"
    );
    assert_eq!(
        provider.request_count(),
        2,
        "the answered batch returns to the model as exactly one follow-up turn"
    );
    let requests = provider.requests();
    let replay = serde_json::to_string(&requests[1]).expect("serialize follow-up request");
    assert!(
        replay.contains("First Ask: Vanilla") && replay.contains("Second Ask: Large"),
        "each answer reaches the model on its own tool result: {replay}"
    );
    assert!(completed.parts().iter().any(|part| {
        part.kind == "text"
            && part.content.get("text").and_then(serde_json::Value::as_str)
                == Some("BATCHED_ASK_OK")
    }));
}

/// Wait until both batched asks carry a durable user-input request and report
/// the two tool part ids.
async fn wait_for_two_pending_asks(
    store: &Arc<dyn agena_storage::store::SessionStore>,
    session_id: i64,
) -> (i64, i64) {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let persisted = store.load(session_id).await.expect("reload batched asks");
        let pending = persisted
            .parts
            .iter()
            .filter(|part| tool_part_first_user_input(part).is_some())
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        if pending.len() == 2 {
            return (pending[0], pending[1]);
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "a batched ask never became durable alongside its sibling: {:#?}",
            persisted
                .parts
                .iter()
                .map(|part| (part.part_id, part.state))
                .collect::<Vec<_>>()
        );
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
}
