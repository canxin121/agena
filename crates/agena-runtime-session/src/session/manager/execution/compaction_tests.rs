use super::*;
use agena_domain::{Model, ModelId, ModelMetadata};
use agena_runtime_provider::{
    ProviderError,
    provider::{CompletionResponse, ModelRuntime},
};

struct BudgetProvider {
    model: ModelId,
}

#[async_trait::async_trait]
impl ModelRuntime for BudgetProvider {
    fn id(&self) -> &str {
        "compaction-budget-test"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    fn prompt_cache_shape(&self, _: &ModelId) -> Option<agena_provider::PromptCacheShape> {
        Some(agena_provider::PromptCacheShape::from_fields(
            self.id(),
            [("destination_revision", "compaction-budget-test")],
        ))
    }

    fn model_metadata(&self, _: &ModelId) -> ModelMetadata {
        let mut metadata = ModelMetadata::default();
        metadata.limits.max_input_tokens = Some(1);
        metadata
    }

    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _: agena_provider::CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        panic!("an over-budget compaction must not invoke the provider")
    }
}

async fn fixture() -> (SessionManager, SessionId) {
    let kernel = Arc::new(
        agena_storage_sqlite::open_kernel_runtime("sqlite::memory:")
            .await
            .unwrap(),
    );
    let workspace = kernel
        .workspaces
        .ensure_id("/compaction-tests")
        .await
        .unwrap();
    let session = kernel
        .ports
        .sessions
        .create_configured(
            RequestId::new(),
            workspace,
            "Compaction test".into(),
            Default::default(),
        )
        .await
        .unwrap()
        .value
        .id;
    let plugins = agena_plugin_host::PluginHost::new(agena_plugin_host::PluginHostBuildConfig {
        static_plugins: vec![],
        config: Default::default(),
        workspace_root: "/compaction-tests".into(),
        agena_version: "test".into(),
        callback_base_url: None,
        host_client: None,
        previous: None,
        previous_plugins: Default::default(),
    })
    .await
    .unwrap();
    let executor = ToolExecutor::new(
        "/compaction-tests",
        crate::authorization::ExecutionPrincipal::new(
            crate::permission::PermissionPolicy::allow_all(),
            crate::permission::ToolPermissionPolicy::allow_all(),
        ),
        plugins.clone(),
        None,
        None,
    );
    let mut providers = ProviderRegistry::new();
    providers.register(BudgetProvider {
        model: ModelId::new("tiny"),
    });
    let manager = SessionManager::new(
        kernel,
        Arc::new(providers),
        ContextGovernor::new(Default::default()),
        SessionProcessor::new(plugins),
        executor,
        Default::default(),
    );
    (manager, session)
}

#[tokio::test]
async fn oversized_compaction_prefix_fails_with_budget_error_without_changing_context() {
    let (manager, session) = fixture().await;
    let execution = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "history that cannot fit one token".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    let attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, execution.id)
        .await
        .unwrap()
        .value;
    let options = SessionRunOptions {
        model: ModelRef::new("compaction-budget-test", "tiny"),
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
    let state = manager.state.load_full();
    let control = RunControl {
        session,
        execution: execution.id,
        model: options.model.clone(),
        state: state.clone(),
        cancel: CancellationToken::new(),
        phase: StdMutex::new(agena_domain::ExecutionPhase::PreparingModel),
        done: Notify::new(),
        budget: None,
        failure: Default::default(),
        settled: std::sync::atomic::AtomicBool::new(false),
    };
    let result = manager
        .perform_compaction(&attempt, &control, &options, &state, None)
        .await;
    assert!(
        matches!(
            result,
            Err(AppError::PromptBudgetExceeded {
                limit_tokens: 1,
                ..
            })
        ),
        "unexpected result: {result:?}"
    );
    let status = manager
        .kernel()
        .reader
        .prompt_window_status(session)
        .await
        .unwrap();
    assert!(status.checkpoint_entry_id.is_none());
    assert_eq!(
        manager
            .kernel()
            .reader
            .history(session, None, 10)
            .await
            .unwrap()
            .entries
            .len(),
        1
    );
    attempt
        .finish(RequestId::new(), ExecutionPhase::Failed)
        .await
        .unwrap();
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn control_anchors_are_local_boundaries_without_ancestor_authority() {
    let (manager, root) = fixture().await;
    for text in ["first owned request", "latest owned request"] {
        let execution = manager
            .kernel()
            .sessions
            .send(
                RequestId::new(),
                root,
                ComposerDocument(vec![ComposerNode::Text { text: text.into() }]),
            )
            .await
            .unwrap()
            .value;
        manager
            .kernel()
            .executor
            .claim_next(RequestId::new(), root, execution.id)
            .await
            .unwrap()
            .value
            .finish(RequestId::new(), ExecutionPhase::Succeeded)
            .await
            .unwrap();
    }
    let mut last_control = None;
    for purpose in [
        ControlExecutionPurpose::Compaction,
        ControlExecutionPurpose::Continuation,
    ] {
        let execution = manager
            .kernel()
            .sessions
            .start_control_execution(RequestId::new(), root, purpose)
            .await
            .unwrap()
            .value;
        assert_eq!(
            manager.latest_origin_message(root).await.unwrap(),
            Some(execution.message_id)
        );
        manager
            .kernel()
            .sessions
            .cancel(RequestId::new(), root, execution.id)
            .await
            .unwrap();
        last_control = Some(execution.message_id);
    }
    let fork = manager
        .kernel()
        .sessions
        .fork(RequestId::new(), root, last_control.unwrap())
        .await
        .unwrap()
        .value
        .id;
    assert_eq!(manager.latest_origin_message(fork).await.unwrap(), None);
    let continuation = manager
        .kernel()
        .sessions
        .start_control_execution(
            RequestId::new(),
            fork,
            ControlExecutionPurpose::Continuation,
        )
        .await
        .unwrap()
        .value;
    assert_eq!(
        manager.latest_origin_message(fork).await.unwrap(),
        Some(continuation.message_id)
    );
    let nested = manager
        .kernel()
        .sessions
        .fork(RequestId::new(), fork, continuation.message_id)
        .await
        .unwrap()
        .value
        .id;
    assert_eq!(manager.latest_origin_message(nested).await.unwrap(), None);
    for session in [root, fork, nested] {
        let page = manager
            .kernel()
            .reader
            .history(session, None, 100)
            .await
            .unwrap();
        assert_eq!(
            page.entries
                .iter()
                .filter(|entry| matches!(entry.payload, EntryPayload::User { .. }))
                .count(),
            2
        );
    }
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}

#[tokio::test]
async fn continuing_after_compaction_keeps_new_context_and_reports_the_latest_failure() {
    let (manager, session) = fixture().await;
    let seed = manager
        .kernel()
        .sessions
        .send(
            RequestId::new(),
            session,
            ComposerDocument(vec![ComposerNode::Text {
                text: "Original request".into(),
            }]),
        )
        .await
        .unwrap()
        .value;
    manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, seed.id)
        .await
        .unwrap()
        .value
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    let compact = manager
        .kernel()
        .sessions
        .start_control_execution(
            RequestId::new(),
            session,
            ControlExecutionPurpose::Compaction,
        )
        .await
        .unwrap()
        .value;
    let compact_attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, compact.id)
        .await
        .unwrap()
        .value;
    let compact_round = compact_attempt
        .begin_compaction(RequestId::new(), 128)
        .await
        .unwrap()
        .value;
    let summary = compact_round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    summary
        .append("Remember the original request.")
        .await
        .unwrap();
    compact_round
        .commit_compaction(RequestId::new(), summary.id(), seed.message_id, None)
        .await
        .unwrap();
    compact_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();
    assert_eq!(
        manager.latest_origin_message(session).await.unwrap(),
        Some(compact.message_id)
    );

    // Even a delayed caller referring to the old user request must not retry
    // behind a newer control boundary and lose subsequent context or status.
    let continuation = manager
        .kernel()
        .sessions
        .continue_execution(RequestId::new(), session, seed.message_id)
        .await
        .unwrap()
        .value;
    assert_eq!(continuation.purpose, ExecutionPurpose::Continuation);
    assert!(continuation.message_id > compact.message_id);
    let first_attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, continuation.id)
        .await
        .unwrap()
        .value;
    let first_round = first_attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    let response = first_round
        .begin_text(RequestId::new(), false)
        .await
        .unwrap()
        .value;
    response
        .append("Progress made by the first continuation.")
        .await
        .unwrap();
    first_round
        .finish(RequestId::new(), None, true)
        .await
        .unwrap();
    first_attempt
        .finish(RequestId::new(), ExecutionPhase::Succeeded)
        .await
        .unwrap();

    let next = manager
        .kernel()
        .sessions
        .continue_execution(
            RequestId::new(),
            session,
            manager
                .latest_origin_message(session)
                .await
                .unwrap()
                .unwrap(),
        )
        .await
        .unwrap()
        .value;
    assert_eq!(next.purpose, ExecutionPurpose::Continuation);
    assert!(next.message_id > continuation.message_id);
    let next_attempt = manager
        .kernel()
        .executor
        .claim_next(RequestId::new(), session, next.id)
        .await
        .unwrap()
        .value;
    let next_round = next_attempt
        .begin_current_round(RequestId::new())
        .await
        .unwrap()
        .value;
    assert!(
        next_round
            .prompt()
            .await
            .unwrap()
            .entries
            .iter()
            .any(|entry| entry.entry.id == response.entry_id())
    );
    next_round
        .finish(RequestId::new(), None, false)
        .await
        .unwrap();
    next_attempt
        .finish(RequestId::new(), ExecutionPhase::Failed)
        .await
        .unwrap();
    let state = manager
        .kernel()
        .reader
        .control_snapshot(session, 1)
        .await
        .unwrap();
    assert_eq!(state.summary.state, agena_domain::SessionStateKind::Failed);
    assert_eq!(state.summary.user_message_count, 1);
    assert_eq!(
        manager
            .kernel()
            .reader
            .execution(session, seed.id)
            .await
            .unwrap()
            .generation,
        seed.generation
    );
    manager
        .kernel_runtime
        .ports
        .clone()
        .shutdown()
        .await
        .unwrap();
}
