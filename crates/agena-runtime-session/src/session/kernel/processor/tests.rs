use super::*;
use crate::provider::{ModelRuntime, ProviderError};
use agena_domain::{ComposerDocument, ComposerNode, ModelId, Role, StructuredObject};
use agena_plugin_host::{PluginHost, PluginHostBuildConfig};
use agena_storage_sqlite::{SqliteKernel, open_kernel_runtime};
use std::sync::Mutex;
use tokio::sync::{Notify, mpsc};

struct Provider {
    model: ModelId,
    events: Mutex<
        Option<mpsc::UnboundedReceiver<std::result::Result<CompletionStreamEvent, ProviderError>>>,
    >,
    requests: Mutex<Vec<CompletionRequest>>,
}
#[async_trait::async_trait]
impl ModelRuntime for Provider {
    fn id(&self) -> &str {
        "fixture"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    async fn list_models(&self) -> std::result::Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        _: CompletionRequest,
    ) -> std::result::Result<agena_provider::CompletionResponse, ProviderError> {
        unreachable!("streaming fixture")
    }
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> std::result::Result<CompletionEventStream, ProviderError> {
        self.requests.lock().unwrap().push(request);
        let receiver = self
            .events
            .lock()
            .unwrap()
            .take()
            .expect("provider must be invoked only once");
        Ok(Box::pin(futures_util::stream::unfold(
            receiver,
            |mut receiver| async { receiver.recv().await.map(|event| (event, receiver)) },
        )))
    }
}
fn model() -> ModelRef {
    ModelRef::new("fixture", "fixture-model")
}
fn text(delta: &str) -> CompletionStreamEvent {
    CompletionStreamEvent::TextDelta {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        delta: delta.into(),
    }
}
fn thinking(delta: &str) -> CompletionStreamEvent {
    CompletionStreamEvent::ThinkingDelta {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        delta: delta.into(),
    }
}
fn call(key: &str, id: Option<&str>, args: &str) -> CompletionStreamEvent {
    CompletionStreamEvent::ToolCallDelta {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        stream_key: key.into(),
        id: id.map(str::to_owned),
        name: Some("tools_call".into()),
        arguments_delta: args.into(),
    }
}
fn completed(metadata: Option<Value>, end_turn: Option<bool>) -> CompletionStreamEvent {
    CompletionStreamEvent::Completed {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        finish_reason: None,
        usage: None,
        provider_metadata: metadata,
        end_turn,
    }
}
fn invocation() -> ToolInvocation {
    ToolInvocation {
        tool_api_call: None,
        name: "chatgpt.web_search".into(),
        plugin_name: None,
        input: StructuredObject::default(),
    }
}
fn hosted_start() -> CompletionStreamEvent {
    CompletionStreamEvent::ProviderNativeToolCallStarted {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        stream_key: "idx:0".into(),
        id: Some("hosted-1".into()),
        invocation: invocation(),
        title: "searching".into(),
        raw: Some(json!({"opaque":[" \n\0",false]})),
    }
}
fn hosted_done() -> CompletionStreamEvent {
    CompletionStreamEvent::ProviderNativeToolCallCompleted {
        provider_id: agena_domain::ProviderId::new("fixture"),
        model: ModelId::new("fixture-model"),
        stream_key: "idx:9".into(),
        id: Some("hosted-1".into()),
        invocation: invocation(),
        title: "done".into(),
        summary: "search failed".into(),
        failure: Some(Box::new(
            AppError::Provider("hosted failure".into()).failure(),
        )),
        output_text: " exact output \n".into(),
        blocks: Vec::new(),
        details: Default::default(),
        raw: Some(json!({"final":true})),
    }
}

struct Fixture {
    kernel: SqliteKernel,
    root: tempfile::TempDir,
    session: SessionId,
    attempt: AttemptHandle,
    round: RoundHandle,
    run: KernelRoundRun,
    provider: Arc<Provider>,
    events: mpsc::UnboundedSender<std::result::Result<CompletionStreamEvent, ProviderError>>,
}
async fn fixture(memory: bool, cancellation: Option<CancellationToken>) -> Fixture {
    let root = tempfile::tempdir().unwrap();
    let url = if memory {
        "sqlite::memory:".into()
    } else {
        format!(
            "sqlite://{}?mode=rwc",
            root.path().join("runtime.db").display()
        )
    };
    let kernel = open_kernel_runtime(&url).await.unwrap();
    let workspace = kernel
        .workspaces
        .create(root.path().to_string_lossy().into())
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create(RequestId::new(), workspace.id, "processor".into())
        .await
        .unwrap()
        .value
        .id;
    let sent = kernel
        .ports
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "  exact user 世界\n\0".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = kernel
        .ports
        .executor
        .claim(RequestId::new(), session, sent.id)
        .await
        .unwrap()
        .value;
    let round = attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let prompt = crate::session::kernel::KernelPromptHydrator::isolated()
        .hydrate(
            &kernel.ports.reader,
            round.prompt().await.unwrap(),
            &model(),
            false,
        )
        .await
        .unwrap();
    let plugins = PluginHost::new(PluginHostBuildConfig {
        static_plugins: Vec::new(),
        config: Default::default(),
        workspace_root: root.path().into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let (events, receiver) = mpsc::unbounded_channel();
    let provider = Arc::new(Provider {
        model: model().model_id,
        events: Mutex::new(Some(receiver)),
        requests: Mutex::new(Vec::new()),
    });
    let mut registry = ProviderRegistry::new();
    registry.register_arc(provider.clone());
    let mut completion: CompletionRequest = serde_json::from_value(json!({"model":"ignored-model","messages":[{"role":"user","parts":[{"type":"text","text":"untrusted replacement"}]}],"stop_sequences":[]})).unwrap();
    completion
        .tool_api_functions
        .push(agena_provider::ToolApiDefinition {
            handler_key: "tools_call".into(),
            plugin_name: "agena.tools".into(),
            name: "tools_call".into(),
            description: "fixture".into(),
            input_schema: json!({"type":"object"}),
            output_schema: json!({}),
            definition_identity: "fixture".into(),
        });
    let request = KernelRoundRequest::new(round.clone(), prompt, completion, cancellation).unwrap();
    let run = SessionProcessor::new(plugins)
        .kernel_round(request, kernel.ports.reader.clone(), Arc::new(registry))
        .unwrap();
    Fixture {
        kernel,
        root,
        session,
        attempt,
        round,
        run,
        provider,
        events,
    }
}
async fn wait_entries(reader: &KernelReader, session: SessionId, count: usize) {
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            if reader
                .history(session, None, 200)
                .await
                .unwrap()
                .entries
                .len()
                >= count
            {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn actual_processor_streams_stable_kernel_entries_and_restores_replay_across_tool_rounds() {
    for memory in [true, false] {
        let mut fixture = fixture(memory, None).await;
        let ports = &fixture.kernel.ports;
        let reasoning = "  signed 世界🙂\n\0 reasoning\t";
        for event in [
            thinking(reasoning),
            text(" answer \n"),
            call(
                "idx:0",
                Some("call-z"),
                r#"{"tool":"fs.read","input":{"path":"one"}}"#,
            ),
            call(
                "idx:0",
                Some("call-a"),
                r#"{"tool":"fs.read","input":{"path":"two"}}"#,
            ),
            hosted_start(),
        ] {
            fixture.events.send(Ok(event)).unwrap();
        }
        // Drop only the drive future while the provider is still live. Both
        // pending argument drafts and hosted start already have durable IDs.
        tokio::select! {
            result = fixture.run.drive() => panic!("provider unexpectedly ended: {:?}", result.err()),
            _ = wait_entries(&ports.reader, fixture.session, 6) => {}
        }
        let history = ports
            .reader
            .history(fixture.session, None, 200)
            .await
            .unwrap();
        let tool_ids = history
            .entries
            .iter()
            .filter_map(|entry| match entry.payload {
                EntryPayload::Tool { tool_id } => Some(tool_id),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(tool_ids.len(), 3);
        assert_eq!(
            ports
                .reader
                .tool(fixture.session, tool_ids[0])
                .await
                .unwrap()
                .phase,
            ToolPhase::Preparing
        );
        assert_eq!(
            ports
                .reader
                .tool(fixture.session, tool_ids[2])
                .await
                .unwrap()
                .phase,
            ToolPhase::Running
        );
        let output = fixture
            .run
            .texts
            .iter()
            .find(|text| !text.reasoning)
            .unwrap()
            .output
            .as_ref()
            .unwrap();
        let accepted = ports
            .reader
            .text_range(fixture.session, output.id(), None, 1024)
            .await
            .unwrap();
        assert_eq!(accepted.bytes, b" answer \n");
        drop(accepted);
        fixture.events.send(Ok(hosted_done())).unwrap();
        fixture.events.send(Ok(hosted_done())).unwrap();
        let replay = json!({"response_id":"response-1","anthropic_thinking_blocks":[{"type":"thinking","thinking":reasoning,"signature":" AA+/==\n\0 "}],"future_provider_fact":{"unknown":[null,false," \n\0"]}});
        fixture
            .events
            .send(Ok(completed(Some(replay.clone()), Some(false))))
            .unwrap();
        fixture.events.send(Ok(completed(None, None))).unwrap();
        drop(fixture.events);
        let outcome = fixture.run.drive().await.unwrap().clone();
        assert!(outcome.failure.is_none());
        assert!(outcome.follow_up_requested);
        assert_eq!(outcome.tools.len(), 2);
        assert_eq!(
            outcome
                .tools
                .iter()
                .map(|tool| tool.tool.id())
                .collect::<Vec<_>>(),
            tool_ids[..2]
        );
        let hosted = ports
            .reader
            .tool(fixture.session, tool_ids[2])
            .await
            .unwrap();
        assert_eq!(hosted.execution_kind, ToolExecutionKind::ProviderHosted);
        assert_eq!(hosted.phase, ToolPhase::Failed);
        assert_eq!(
            hosted.result.as_ref().unwrap()["provider_metadata"],
            json!({"opaque":[" \n\0",false],"final":true})
        );
        let stored = ports
            .reader
            .round(fixture.session, fixture.round.id())
            .await
            .unwrap();
        let state = stored.provider_state.as_ref().unwrap();
        assert_eq!(
            state["replay"]["future_provider_fact"],
            replay["future_provider_fact"]
        );
        assert!(!state["replay"]["anthropic_thinking_blocks"][0]["thinking"].is_string());
        assert_eq!(state["follow_up_requested"], true);
        for tool in &outcome.tools {
            tool.tool
                .complete(
                    RequestId::new(),
                    ToolResult {
                        phase: ToolPhase::Succeeded,
                        value: json!({"ok":true}),
                    },
                )
                .await
                .unwrap();
        }
        let next = fixture
            .attempt
            .begin_current_round(RequestId::new())
            .await
            .unwrap()
            .value;
        let prompt = crate::session::kernel::KernelPromptHydrator::isolated()
            .hydrate(&ports.reader, next.prompt().await.unwrap(), &model(), false)
            .await
            .unwrap();
        assert_eq!(prompt.turns().len(), 2);
        assert_eq!(
            prompt.turns()[1].provider_state.anthropic_thinking_blocks,
            vec![replay["anthropic_thinking_blocks"][0].clone()]
        );
        {
            let requests = fixture.provider.requests.lock().unwrap();
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].model, model().model_id);
            assert_eq!(requests[0].turns[0].role, Role::User);
            assert_eq!(
                requests[0].turns[0].as_text_lossy(),
                "  exact user 世界\n\0"
            );
        }
        next.finish(RequestId::new(), None, true).await.unwrap();
        fixture
            .attempt
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
        fixture.kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn terminal_commit_rejection_retries_the_same_completion_without_provider_or_tool_effects() {
    let mut fixture = fixture(false, None).await;
    let admin = sea_orm::Database::connect(format!(
        "sqlite://{}?mode=rw",
        fixture.root.path().join("runtime.db").display()
    ))
    .await
    .unwrap();
    use sea_orm::{ConnectionTrait, DbBackend, Statement};
    admin.execute(Statement::from_string(DbBackend::Sqlite, "CREATE TABLE reject_commit (owner INTEGER REFERENCES agena_entries(id) DEFERRABLE INITIALLY DEFERRED)")).await.unwrap();
    admin.execute(Statement::from_string(DbBackend::Sqlite, "CREATE TRIGGER reject_round_commit AFTER UPDATE OF phase ON agena_rounds WHEN NEW.phase='succeeded' BEGIN INSERT INTO reject_commit VALUES (-1); END")).await.unwrap();
    fixture
        .events
        .send(Ok(text("  fully accepted 世界🙂\n")))
        .unwrap();
    fixture
        .events
        .send(Ok(call(
            "idx:0",
            Some("call-1"),
            r#"{"tool":"fs.read","input":{}}"#,
        )))
        .unwrap();
    fixture
        .events
        .send(Ok(completed(
            Some(json!({"response_id":"saved-response","future":{"unknown":1}})),
            None,
        )))
        .unwrap();
    drop(fixture.events);
    assert!(matches!(
        fixture.run.drive().await,
        Err(KernelError::Storage(_))
    ));
    assert!(fixture.run.outcome().is_none());
    let step = fixture.run.steps.front().unwrap();
    let request = step.request;
    let Action::Finish { state, succeeded } = &step.action else {
        panic!("completion must remain pending");
    };
    assert!(*succeeded);
    let exact = state.clone();
    assert_eq!(
        fixture
            .kernel
            .ports
            .reader
            .round(fixture.session, fixture.round.id())
            .await
            .unwrap()
            .phase,
        ProviderRoundPhase::Running
    );
    admin
        .execute(Statement::from_string(
            DbBackend::Sqlite,
            "DROP TRIGGER reject_round_commit",
        ))
        .await
        .unwrap();
    let outcome = fixture.run.drive().await.unwrap().clone();
    assert!(outcome.failure.is_none());
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    let saved = fixture
        .kernel
        .ports
        .reader
        .round(fixture.session, fixture.round.id())
        .await
        .unwrap();
    assert_eq!(saved.provider_state, Some(exact));
    assert!(fixture.run.drive().await.unwrap().failure.is_none());
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    let receipt: String = sea_orm::sqlx::query_scalar(
        "SELECT receipt_json FROM agena_command_results WHERE request_id=?",
    )
    .bind(request.to_string())
    .fetch_one(admin.get_sqlite_connection_pool())
    .await
    .unwrap();
    assert!(!receipt.is_empty());
    assert_eq!(outcome.tools.len(), 1);
    fixture.kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn dropping_a_drive_future_after_append_acceptance_keeps_the_in_flight_receipt() {
    let mut fixture = fixture(true, None).await;
    // The same actual append future is delayed after admission to reproduce a
    // caller abandoning its wait before observing the accepted cursor.
    fixture
        .run
        .text_delta(" once 世界🙂\n".into(), false)
        .unwrap();
    fixture.run.apply_front().await.unwrap();
    let pending = fixture.run.start_front().unwrap();
    let accepted = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let signal = accepted.clone();
    let wait = release.clone();
    fixture.run.in_flight = Some(Box::pin(async move {
        let value = pending.await?;
        signal.notify_one();
        wait.notified().await;
        Ok(value)
    }));
    tokio::select! {
        _ = fixture.run.drive() => panic!("append receipt should still be waiting"),
        _ = accepted.notified() => {}
    }
    release.notify_one();
    fixture.run.apply_front().await.unwrap();
    let output = fixture.run.texts[0].output.as_ref().unwrap();
    let body = fixture
        .kernel
        .ports
        .reader
        .text_range(fixture.session, output.id(), None, 1024)
        .await
        .unwrap();
    assert_eq!(body.bytes, " once 世界🙂\n".as_bytes());
    fixture.events.send(Ok(text("final"))).unwrap();
    fixture.events.send(Ok(completed(None, None))).unwrap();
    drop(fixture.events);
    assert!(fixture.run.drive().await.unwrap().failure.is_none());
    fixture.kernel.ports.shutdown().await.unwrap();
}

#[tokio::test]
async fn interrupted_or_cancelled_provider_round_settles_real_hosted_calls_and_unfinished_drafts() {
    for cancel in [false, true] {
        let token = CancellationToken::new();
        let mut fixture = fixture(true, Some(token.clone())).await;
        for event in [
            text("partial"),
            call("idx:0", Some("unfinished"), "{"),
            hosted_start(),
        ] {
            fixture.events.send(Ok(event)).unwrap();
        }
        tokio::select! {
            result = fixture.run.drive() => panic!("live stream ended: {:?}", result.err()),
            _ = wait_entries(&fixture.kernel.ports.reader, fixture.session, 4) => {}
        }
        if cancel {
            token.cancel();
        } else {
            drop(fixture.events);
        }
        let outcome = fixture.run.drive().await.unwrap().clone();
        assert!(outcome.failure.is_some());
        assert_eq!(outcome.cancelled, cancel);
        assert!(outcome.tools.is_empty());
        let ports = &fixture.kernel.ports;
        let history = ports
            .reader
            .history(fixture.session, None, 200)
            .await
            .unwrap();
        for entry in &history.entries {
            if let EntryPayload::Tool { tool_id } = entry.payload {
                let tool = ports.reader.tool(fixture.session, tool_id).await.unwrap();
                assert!(!matches!(
                    tool.phase,
                    ToolPhase::Preparing | ToolPhase::Running
                ));
            }
        }
        let round = ports
            .reader
            .round(fixture.session, fixture.round.id())
            .await
            .unwrap();
        assert_eq!(round.phase, ProviderRoundPhase::Failed);
        assert!(round.provider_state.as_ref().unwrap()["failure"].is_object());
        fixture
            .attempt
            .finish(
                RequestId::new(),
                if cancel {
                    ExecutionPhase::Cancelled
                } else {
                    ExecutionPhase::Failed
                },
            )
            .await
            .unwrap();
        fixture.kernel.ports.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn invalid_published_drafts_degrade_to_tool_visible_results() {
    unknown_published_draft_degrades_to_a_normal_tool_failure().await;
    malformed_tools_call_draft_becomes_a_diagnostic_tool_invocation().await;
}

async fn unknown_published_draft_degrades_to_a_normal_tool_failure() {
    let mut fixture = fixture(true, None).await;
    let mut unknown = call(
        "idx:0",
        Some("invalid-call"),
        r#"{"command":"never executed"}"#,
    );
    if let CompletionStreamEvent::ToolCallDelta { name, .. } = &mut unknown {
        *name = Some("shell_exec".into());
    }
    fixture.events.send(Ok(unknown)).unwrap();
    fixture.events.send(Ok(completed(None, None))).unwrap();
    drop(fixture.events);
    let outcome = fixture.run.drive().await.unwrap().clone();
    assert!(outcome.failure.is_none());
    assert_eq!(outcome.tools.len(), 1);
    assert_eq!(outcome.tools[0].invocation.name, "shell_exec");
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    let draft = fixture.run.drafts[0].handle.as_ref().unwrap();
    assert_eq!(outcome.tools[0].tool.id(), draft.id());
    assert_eq!(
        fixture
            .kernel
            .ports
            .reader
            .tool(fixture.session, draft.id())
            .await
            .unwrap()
            .phase,
        ToolPhase::Running
    );
    assert_eq!(
        fixture
            .kernel
            .ports
            .reader
            .round(fixture.session, fixture.round.id())
            .await
            .unwrap()
            .phase,
        ProviderRoundPhase::Succeeded
    );
    fixture.kernel.ports.shutdown().await.unwrap();
}

async fn malformed_tools_call_draft_becomes_a_diagnostic_tool_invocation() {
    let mut fixture = fixture(true, None).await;
    fixture
        .events
        .send(Ok(call(
            "idx:0",
            Some("invalid-json"),
            r#"{"tool":"fs.read","input":{"path":}}"#,
        )))
        .unwrap();
    fixture.events.send(Ok(completed(None, None))).unwrap();
    drop(fixture.events);

    let outcome = fixture.run.drive().await.unwrap().clone();
    assert!(outcome.failure.is_none());
    assert_eq!(fixture.provider.requests.lock().unwrap().len(), 1);
    assert_eq!(outcome.tools.len(), 1);
    let invocation = &outcome.tools[0].invocation;
    assert_eq!(invocation.name, "tools_call");
    let diagnostic = invocation
        .tool_api_call
        .as_ref()
        .and_then(|call| {
            call.arguments
                .get(agena_domain::TOOLS_CALL_ARGUMENTS_DIAGNOSTIC_FIELD)
        })
        .and_then(|value| value.as_text())
        .expect("malformed arguments must carry the executor diagnostic");
    assert!(diagnostic.contains("did not parse as valid JSON"));
    assert_eq!(
        fixture
            .kernel
            .ports
            .reader
            .round(fixture.session, fixture.round.id())
            .await
            .unwrap()
            .phase,
        ProviderRoundPhase::Succeeded
    );
    fixture.kernel.ports.shutdown().await.unwrap();
}
