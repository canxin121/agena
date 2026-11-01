use super::*;
use agena_domain::{Model, ModelId};
use agena_runtime_provider::{
    ProviderError,
    provider::{CompletionResponse, ModelRuntime},
};

// Provider runs reserve 32 MiB from process-wide read budgets. Separate
// in-memory databases all allocate child ID 2, so running these fixtures in
// parallel would incorrectly charge their independent sessions to one owner.
static PROVIDER_FIXTURES: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

struct ProbeProvider {
    model: ModelId,
    requests: tokio::sync::mpsc::UnboundedSender<Option<u32>>,
    gate: Arc<Semaphore>,
    reply: Arc<StdMutex<Option<agena_provider::CompletionUsage>>>,
    panic_in_provider: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl ModelRuntime for ProbeProvider {
    fn id(&self) -> &str {
        "agent-lifecycle-test"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn prompt_cache_shape(&self, _: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id(),
            [("destination_revision", "agent-lifecycle-test")],
        ))
    }
    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(
        &self,
        request: agena_provider::CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.requests.send(request.max_output_tokens).unwrap();
        self.gate.acquire().await.unwrap().forget();
        assert!(
            !self
                .panic_in_provider
                .load(std::sync::atomic::Ordering::Relaxed),
            "intentional provider panic"
        );
        match self.reply.lock().unwrap().clone() {
            None => Err(ProviderError::Config(
                "intentional lifecycle probe failure".into(),
            )),
            Some(usage) => Ok(CompletionResponse {
                provider_id: agena_domain::ProviderId::new(self.id()),
                model: self.model.clone(),
                text: "Delegated task completed".into(),
                reasoning_text: None,
                finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: Some(usage),
                provider_metadata: None,
            }),
        }
    }
}

struct Fixture {
    _provider_admission: tokio::sync::MutexGuard<'static, ()>,
    manager: SessionManager,
    parent: SessionId,
    child: SessionId,
    options: SessionRunOptions,
    requests: tokio::sync::mpsc::UnboundedReceiver<Option<u32>>,
    gate: Arc<Semaphore>,
    reply: Arc<StdMutex<Option<agena_provider::CompletionUsage>>>,
    panic_in_provider: Arc<std::sync::atomic::AtomicBool>,
}

impl Fixture {
    async fn new(limits: AgentExecutionLimits) -> Self {
        let provider_admission = PROVIDER_FIXTURES.lock().await;
        let kernel = Arc::new(
            agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
                .await
                .unwrap(),
        );
        let workspace = kernel
            .workspaces
            .ensure_id("/agent-lifecycle-tests")
            .await
            .unwrap();
        let parent = kernel
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Parent".into(),
                Default::default(),
            )
            .await
            .unwrap()
            .value
            .id;
        let child = kernel
            .ports
            .sessions
            .create_configured(
                RequestId::new(),
                workspace,
                "Child".into(),
                SessionConfiguration {
                    parent_session_id: Some(parent),
                    relation_kind: agena_domain::SessionRelationKind::Subagent,
                    agent: Some(SessionAgentConfiguration {
                        name: "worker".into(),
                        status: AgentStatus::Completed,
                        started_at_ms: Some(1),
                        finished_at_ms: Some(2),
                        failure: None,
                        limits,
                    }),
                    ..Default::default()
                },
            )
            .await
            .unwrap()
            .value
            .id;
        let plugins =
            agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
                static_plugins: vec![],
                config: Default::default(),
                workspace_root: "/agent-lifecycle-tests".into(),
                agena_version: "test".into(),
                callback_base_url: None,
                host_client: None,
                previous: None,
                previous_plugins: Default::default(),
            })
            .await
            .unwrap();
        let executor = ToolExecutor::new(
            "/agent-lifecycle-tests",
            crate::authorization::ExecutionPrincipal::new(
                crate::permission::PermissionPolicy::allow_all(),
                crate::permission::ToolPermissionPolicy::allow_all(),
            ),
            plugins.clone(),
            None,
            None,
        );
        let (tx, requests) = tokio::sync::mpsc::unbounded_channel();
        let gate = Arc::new(Semaphore::new(0));
        let reply = Arc::new(StdMutex::new(None));
        let panic_in_provider = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut providers = ProviderRegistry::new();
        providers.register(ProbeProvider {
            model: ModelId::new("probe"),
            requests: tx,
            gate: gate.clone(),
            reply: reply.clone(),
            panic_in_provider: panic_in_provider.clone(),
        });
        let manager = SessionManager::new(
            kernel,
            Arc::new(providers),
            ContextGovernor::new(Default::default()),
            SessionProcessor::new(plugins),
            executor,
            Default::default(),
        );
        let options = SessionRunOptions {
            model: ModelRef::new("agent-lifecycle-test", "probe"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            model_supports_parallel_tool_calls: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        Self {
            _provider_admission: provider_admission,
            manager,
            parent,
            child,
            options,
            requests,
            gate,
            reply,
            panic_in_provider,
        }
    }

    async fn send(&self) -> ExecutionId {
        self.manager
            .submit(crate::SessionUserRunRequest::new(
                self.child.get(),
                self.options.clone(),
                ComposerDocument(vec![ComposerNode::Text {
                    text: "Direct user task".into(),
                }]),
            ))
            .await
            .unwrap()
            .receipt
            .unwrap()
            .execution_id
    }

    async fn wait_provider(&mut self) -> Option<u32> {
        match tokio::time::timeout(Duration::from_secs(5), self.requests.recv()).await {
            Ok(Some(request)) => request,
            other => {
                let controls = self
                    .manager
                    .active
                    .lock()
                    .unwrap()
                    .values()
                    .filter(|control| control.session == self.child)
                    .cloned()
                    .collect::<Vec<_>>();
                let failures = controls
                    .iter()
                    .map(|control| control.failure.lock().unwrap().clone())
                    .collect::<Vec<_>>();
                panic!(
                    "provider did not receive request: {other:?}; agent={:?}; live_failures={failures:?}",
                    self.agent().await
                );
            }
        }
    }

    async fn agent(&self) -> SessionAgentConfiguration {
        self.manager
            .kernel()
            .reader
            .configuration(self.child)
            .await
            .unwrap()
            .agent
            .clone()
            .unwrap()
    }

    async fn idle(&self) {
        tokio::time::timeout(
            Duration::from_secs(5),
            self.manager.wait_session_idle(self.child.get()),
        )
        .await
        .unwrap();
    }

    async fn deliveries(&self) -> ReadView<Vec<DeliveryView>> {
        self.manager
            .kernel()
            .reader
            .pending_deliveries(self.parent, None, 128)
            .await
            .unwrap()
    }

    async fn close(self) {
        self.manager
            .kernel_runtime
            .ports
            .clone()
            .shutdown()
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn direct_subagent_turns_and_retries_keep_identity_limits_and_distinct_results() {
    let mut fixture = Fixture::new(AgentExecutionLimits {
        max_tokens: Some(7),
        ..Default::default()
    })
    .await;
    let execution = fixture.send().await;
    assert_eq!(fixture.wait_provider().await, Some(7));
    let first = fixture.agent().await;
    assert_eq!(first.status, AgentStatus::Running);
    assert!(first.started_at_ms > Some(1));
    assert!(first.finished_at_ms.is_none());
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .active_executions(fixture.parent, None, 1)
            .await
            .unwrap()
            .executions
            .len(),
        0
    );
    fixture.gate.add_permits(1);
    fixture.idle().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Failed);
    let deliveries = fixture.deliveries().await;
    assert_eq!(deliveries.len(), 1);
    assert_eq!(deliveries[0].payload["attempt_generation"], 1);

    let retry = fixture
        .manager
        .start_continue_session(SessionExecutionRequest::new(
            fixture.child.get(),
            fixture.options.clone(),
        ))
        .await
        .unwrap()
        .receipt
        .unwrap()
        .execution_id;
    assert_eq!(retry, execution);
    assert_eq!(fixture.wait_provider().await, Some(7));
    let second = fixture.agent().await;
    assert_eq!(second.status, AgentStatus::Running);
    assert!(second.started_at_ms > first.started_at_ms);
    *fixture.reply.lock().unwrap() = Some(agena_provider::CompletionUsage {
        output_tokens: 1,
        ..Default::default()
    });
    fixture.gate.add_permits(1);
    fixture.idle().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Completed);
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(fixture.child, execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Succeeded
    );
    let deliveries = fixture.deliveries().await;
    assert_eq!(deliveries.len(), 2);
    let mut generations = deliveries
        .iter()
        .map(|delivery| delivery.payload["attempt_generation"].as_i64().unwrap())
        .collect::<Vec<_>>();
    generations.sort_unstable();
    assert_eq!(generations, [1, 2]);
    fixture.close().await;
}

#[tokio::test]
async fn direct_subagent_turns_enforce_stored_token_and_cost_limits() {
    for (limits, usage) in [
        (
            AgentExecutionLimits {
                max_tokens: Some(7),
                ..Default::default()
            },
            agena_provider::CompletionUsage {
                output_tokens: 8,
                ..Default::default()
            },
        ),
        (
            AgentExecutionLimits {
                max_cost_microusd: Some(10),
                ..Default::default()
            },
            agena_provider::CompletionUsage {
                recorded_cost: 0.00002,
                recorded_cost_available: true,
                ..Default::default()
            },
        ),
    ] {
        let mut fixture = Fixture::new(limits).await;
        *fixture.reply.lock().unwrap() = Some(usage);
        let execution = fixture.send().await;
        fixture.wait_provider().await;
        fixture.gate.add_permits(1);
        fixture.idle().await;
        let agent = fixture.agent().await;
        assert_eq!(agent.status, AgentStatus::Failed);
        let failure = agent.failure.unwrap();
        assert_eq!(failure.code.as_str(), "agent.budget_exceeded");
        assert_eq!(
            failure.category,
            agena_failure::FailureCategory::QuotaExceeded
        );
        assert_eq!(
            fixture
                .manager
                .kernel()
                .reader
                .execution(fixture.child, execution)
                .await
                .unwrap()
                .phase,
            ExecutionPhase::Failed
        );
        assert_eq!(fixture.deliveries().await.len(), 1);
        fixture.close().await;
    }
}

#[tokio::test]
async fn direct_subagent_timeout_settles_and_releases_its_execution() {
    let mut fixture = Fixture::new(AgentExecutionLimits {
        timeout_ms: Some(500),
        ..Default::default()
    })
    .await;
    let execution = fixture.send().await;
    fixture.wait_provider().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Running);
    fixture.idle().await;
    let agent = fixture.agent().await;
    assert_eq!(agent.status, AgentStatus::TimedOut);
    assert_eq!(
        agent.failure.unwrap().category,
        agena_failure::FailureCategory::Timeout
    );
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(fixture.child, execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    assert_eq!(fixture.deliveries().await.len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn provider_panic_does_not_leave_a_direct_subagent_running() {
    let mut fixture = Fixture::new(Default::default()).await;
    fixture
        .panic_in_provider
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let execution = fixture.send().await;
    fixture.wait_provider().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Running);
    fixture.gate.add_permits(1);
    fixture.idle().await;
    let agent = fixture.agent().await;
    assert_eq!(agent.status, AgentStatus::Cancelled);
    assert_eq!(
        agent.failure.as_ref().unwrap().category,
        agena_failure::FailureCategory::Internal
    );
    assert!(agent.finished_at_ms.is_some());
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(fixture.child, execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Cancelled
    );
    assert_eq!(fixture.deliveries().await.len(), 1);
    fixture.close().await;
}

#[tokio::test]
async fn abandoned_settlement_registration_cannot_stall_a_direct_subagent_turn() {
    let mut fixture = Fixture::new(Default::default()).await;
    let mut old = fixture.agent().await;
    old.status = AgentStatus::Running;
    old.started_at_ms = Some(3);
    old.finished_at_ms = None;
    fixture
        .manager
        .kernel()
        .sessions
        .configure(
            RequestId::new(),
            fixture.child,
            SessionConfigurationChange {
                agent: Some(Some(old)),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    let old_started = 3;
    let registration =
        fixture
            .manager
            .register_agent_settlement(fixture.child, old_started, ExecutionId::new());
    let execution = fixture.send().await;
    assert!(
        tokio::time::timeout(Duration::from_millis(30), fixture.requests.recv())
            .await
            .is_err()
    );
    drop(registration);
    fixture.wait_provider().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Running);
    fixture.gate.add_permits(1);
    fixture.idle().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Failed);
    assert_eq!(
        fixture
            .manager
            .kernel()
            .reader
            .execution(fixture.child, execution)
            .await
            .unwrap()
            .phase,
        ExecutionPhase::Failed
    );
    fixture.close().await;
}

#[tokio::test]
async fn manual_subagent_compaction_preserves_delegated_task_status_and_mailbox() {
    let mut fixture = Fixture::new(Default::default()).await;
    fixture.send().await;
    fixture.wait_provider().await;
    fixture.gate.add_permits(1);
    fixture.idle().await;
    let before = fixture.agent().await;
    let delivered = fixture.deliveries().await.len();
    fixture
        .manager
        .start_compact_session(SessionExecutionRequest::new(
            fixture.child.get(),
            fixture.options.clone(),
        ))
        .await
        .unwrap();
    // Compaction may need a model; release it without depending on how much
    // of this short history is eligible for a summary.
    fixture.gate.add_permits(1);
    fixture.idle().await;
    assert_eq!(fixture.agent().await, before);
    assert_eq!(fixture.deliveries().await.len(), delivered);
    fixture.close().await;
}

#[tokio::test]
async fn a_direct_turn_queued_after_delegation_waits_for_exactly_one_previous_settlement() {
    let mut fixture = Fixture::new(Default::default()).await;
    *fixture.reply.lock().unwrap() = Some(agena_provider::CompletionUsage::default());
    fixture
        .manager
        .launch_agent(AgentLaunch {
            parent_session_id: fixture.parent.get(),
            launch_call_id: None,
            name: "worker".into(),
            prompt: "Delegated first task".into(),
            commands: Vec::new(),
            selection: agena_domain::ModelSelectionConfig {
                provider: Some("agent-lifecycle-test".into()),
                model: Some("probe".into()),
                ..Default::default()
            },
            timeout_ms: None,
            max_tokens: None,
            max_cost_microusd: None,
            existing: Some(fixture.child),
        })
        .await
        .unwrap();
    fixture.wait_provider().await;
    let delegated = fixture.agent().await;
    assert_eq!(delegated.status, AgentStatus::Running);
    fixture.send().await;
    fixture.gate.add_permits(1);
    fixture.wait_provider().await;
    let direct = fixture.agent().await;
    assert_eq!(direct.status, AgentStatus::Running);
    assert!(direct.started_at_ms > delegated.started_at_ms);
    assert_eq!(fixture.deliveries().await.len(), 1);
    fixture.gate.add_permits(1);
    fixture.idle().await;
    assert_eq!(fixture.agent().await.status, AgentStatus::Completed);
    assert_eq!(fixture.deliveries().await.len(), 2);
    assert!(
        fixture
            .manager
            .pending_agent_settlements
            .lock()
            .unwrap()
            .is_empty()
    );
    fixture.close().await;
}
