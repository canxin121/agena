//! Exercise the bundled tools through real model, gateway and callback boundaries.
use super::*;
use agena_domain::{Role, session_kernel::SessionId};
use agena_provider::{CompletionFinishReason, CompletionRequest, CompletionStreamEvent};
use agena_storage::kernel::EntryPayload;

const CHILD_SCOPE: &str = "independent child scope marker";
const FINDING: &str = "finding sent through the child callback";
const FINAL: &str = "final child result marker";

struct AgentProvider {
    model: agena_domain::ModelId,
    requests: Mutex<Vec<CompletionRequest>>,
    parent_calls: AtomicUsize,
    child_calls: AtomicUsize,
    parent_wait_requested: tokio::sync::Notify,
    child_release: tokio_util::sync::CancellationToken,
}

#[async_trait::async_trait]
impl crate::provider::ModelRuntime for AgentProvider {
    fn id(&self) -> &str {
        "callback-fixture"
    }
    fn default_model(&self) -> &agena_domain::ModelId {
        &self.model
    }
    fn prompt_cache_shape(
        &self,
        _: &agena_domain::ModelId,
    ) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            "agent-fixture",
            [("endpoint", "controlled")],
        ))
    }
    async fn list_models(
        &self,
    ) -> Result<Vec<agena_domain::Model>, agena_runtime_provider::ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        _: CompletionRequest,
    ) -> Result<agena_provider::CompletionResponse, agena_runtime_provider::ProviderError> {
        unreachable!("streaming fixture")
    }
    async fn complete_stream(
        &self,
        request: CompletionRequest,
    ) -> Result<crate::provider::core::CompletionEventStream, agena_runtime_provider::ProviderError>
    {
        let child = request
            .turns
            .iter()
            .find(|turn| turn.role == Role::User)
            .unwrap()
            .as_text_lossy()
            == CHILD_SCOPE;
        let received_final = request.turns.iter().flat_map(|turn| &turn.parts).any(|part| {
            matches!(part, agena_provider::CompletionInputPart::SystemMessage { text } if text.contains(FINAL))
        });
        self.requests.lock().unwrap().push(request);
        let step = if child {
            self.child_calls.fetch_add(1, Ordering::SeqCst)
        } else {
            self.parent_calls.fetch_add(1, Ordering::SeqCst)
        };
        assert!(step < 8, "cooperation made no progress");
        let tool = if child && step == 0 {
            self.child_release.cancelled().await;
            Some(json!({"tool":"agent.message","input":{"target":"parent","message":FINDING}}))
        } else if !child && step == 0 {
            Some(json!({"tool":"agent.spawn","input":{"name":"worker","prompt":CHILD_SCOPE}}))
        } else if !child && !received_final {
            self.parent_wait_requested.notify_one();
            Some(json!({"tool":"agent.wait","input":{"timeout_ms":10000}}))
        } else {
            None
        };
        let provider = agena_domain::ProviderId::new(self.id());
        let event = match &tool {
            Some(input) => CompletionStreamEvent::ToolCallDelta {
                provider_id: provider.clone(),
                model: self.model.clone(),
                stream_key: format!("agent-{child}-{step}"),
                id: Some(format!("agent-{child}-{step}")),
                name: Some("tools_call".into()),
                arguments_delta: input.to_string(),
            },
            None => CompletionStreamEvent::TextDelta {
                provider_id: provider.clone(),
                model: self.model.clone(),
                delta: if child { FINAL } else { "cooperation complete" }.into(),
            },
        };
        Ok(Box::pin(futures_util::stream::iter([
            Ok(event),
            Ok(CompletionStreamEvent::Completed {
                provider_id: provider,
                model: self.model.clone(),
                finish_reason: Some(if tool.is_some() {
                    CompletionFinishReason::ToolCalls
                } else {
                    CompletionFinishReason::Stop
                }),
                usage: None,
                provider_metadata: None,
                end_turn: Some(tool.is_none()),
            }),
        ])))
    }
}

#[tokio::test]
async fn bundled_agents_exchange_mail_and_return_final_results_with_one_tool_slot() {
    let mut fixture = Fixture::new().await;
    fixture
        .observed
        .execution_fixture
        .store(true, Ordering::SeqCst);
    fixture.observed.session_end_release.cancel();
    fixture.write_config("en-US");
    let runtime = fixture.start().await;
    let manager = runtime.session_manager().unwrap();
    let provider = Arc::new(AgentProvider {
        model: agena_domain::ModelId::new("controlled"),
        requests: Default::default(),
        parent_calls: AtomicUsize::new(0),
        child_calls: AtomicUsize::new(0),
        parent_wait_requested: Default::default(),
        child_release: Default::default(),
    });
    let mut registry = crate::provider::ProviderRegistry::new();
    registry.register_arc(provider.clone());
    let host = runtime.current_snapshot().plugin_manager();
    let executor = crate::tool::ToolExecutor::new(
        runtime.workspace_root().to_owned(),
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        host.clone(),
        None,
        None,
    );
    manager.reconfigure(
        Arc::new(registry),
        crate::ContextGovernor::new(Default::default()),
        crate::session::SessionProcessor::new(host.clone()),
        executor,
        crate::RuntimeSessionManagerConfig {
            max_concurrent_tools: 1,
            ..Default::default()
        },
    );
    let parent = manager
        .create_session(crate::SessionCreateRequest {
            title: "agent cooperation".into(),
            parent_session_id: None,
        })
        .await
        .unwrap();
    crate::SessionExecutionCommandService::submit_user_run(
        manager.as_ref(),
        crate::SessionUserRunRequest::new(
            parent.id,
            fixture_run_options(),
            agena_domain::ComposerDocument(vec![agena_domain::ComposerNode::Text {
                text: "coordinate the agents".into(),
            }]),
        ),
    )
    .await
    .unwrap();
    tokio::time::timeout(WAIT, provider.parent_wait_requested.notified())
        .await
        .unwrap();
    provider.child_release.cancel();
    // A wait that retains the sole tool slot cannot reach this result before
    // its ten-second timeout: the child's message also needs that slot.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if !manager.is_run_active(parent.id).await {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("agent.wait must release gateway capacity for child messages");
    assert_eq!(
        manager
            .final_text(parent.id, 1024)
            .await
            .unwrap()
            .as_deref(),
        Some("cooperation complete")
    );
    let parent_id = SessionId::try_from(parent.id).unwrap();
    let history = manager
        .kernel()
        .reader
        .history(parent_id, None, 128)
        .await
        .unwrap();
    for marker in [FINDING, FINAL] {
        assert_eq!(history.entries.iter().filter(|entry| matches!(&entry.payload, EntryPayload::Notice { text } if text.contains(marker))).count(), 1, "{marker} must be delivered exactly once");
    }
    let child = manager
        .kernel()
        .reader
        .find_agent(parent_id, "worker")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        manager
            .kernel()
            .reader
            .configuration(child)
            .await
            .unwrap()
            .agent
            .as_ref()
            .unwrap()
            .status,
        agena_domain::AgentStatus::Completed
    );
    assert!(provider.child_calls.load(Ordering::SeqCst) >= 2);
    drop(history);
    manager.shutdown().await.unwrap();
    host.shutdown().await;
}
