use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};

fn options() -> agena_runtime::SessionRunOptions {
    agena_runtime::SessionRunOptions {
        model: ModelRef::new("failure-recovery", "fixture"),
        model_supports_parallel_tool_calls: None,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: Some(0.0),
        max_output_tokens: Some(64),
    }
}

struct RecoveringProvider(AtomicUsize);
#[async_trait::async_trait]
impl ModelRuntime for RecoveringProvider {
    fn id(&self) -> &str {
        "failure-recovery"
    }
    fn default_model(&self) -> &ModelId {
        static MODEL: std::sync::LazyLock<ModelId> =
            std::sync::LazyLock::new(|| ModelId::new("fixture"));
        &MODEL
    }
    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(vec![])
    }
    async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        unreachable!()
    }
    async fn complete_stream(
        &self,
        _: CompletionRequest,
    ) -> Result<
        std::pin::Pin<
            Box<
                dyn futures_util::Stream<Item = Result<CompletionStreamEvent, ProviderError>>
                    + Send,
            >,
        >,
        ProviderError,
    > {
        if self.0.fetch_add(1, Ordering::SeqCst) == 0 {
            return Err(ProviderError::Provider(
                "recoverable provider failure".to_owned(),
            ));
        }
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new(self.id()),
                model: self.default_model().clone(),
                delta: "Recovered answer".to_owned(),
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: ProviderId::new(self.id()),
                model: self.default_model().clone(),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: Some(CompletionUsage::default()),
                provider_metadata: None,
                end_turn: None,
            }),
        ])))
    }
}

#[tokio::test]
async fn failure_parts_hook_retry_reuses_the_run_and_retains_its_error() {
    let provider = Arc::new(RecoveringProvider(AtomicUsize::new(0)));
    let manager =
        manager_with_stop_counter(provider.clone(), Arc::new(AtomicUsize::new(0)), true, None)
            .await;
    let session = create(&manager, "stable retry ownership").await;
    let saved = manager
        .submit_subtask_user_message(
            SessionUserRunRequest::new(
                session.id,
                options(),
                vec![TypedContent::Text(text_content("answer"))],
            ),
            None,
        )
        .await
        .expect("hook retry recovers");
    assert_eq!(provider.0.load(Ordering::SeqCst), 2);
    let model_runs: Vec<_> = saved
        .parts()
        .iter()
        .filter(|part| {
            part.kind == "run"
                && part.role == PartRole::Assistant
                && part.content["run_kind"] == "continue"
        })
        .collect();
    assert_eq!(
        model_runs.len(),
        1,
        "an internal retry must not allocate another model run"
    );
    assert_eq!(model_runs[0].state, PartState::Completed);
    let errors: Vec<_> = saved
        .parts()
        .iter()
        .filter(|part| part.kind == "error")
        .collect();
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0].run_id, Some(model_runs[0].part_id));
    assert!(
        errors[0]
            .content
            .to_string()
            .contains("recoverable provider failure")
    );
    let reloaded = manager.store.load_session(session.id).await.unwrap();
    assert!(
        reloaded
            .parts()
            .iter()
            .any(|part| part.part_id == errors[0].part_id && part.kind == "error")
    );
}

#[tokio::test]
async fn failure_parts_repeated_and_concurrent_cleanup_is_idempotent() {
    let manager = test_manager().await;
    let session = create(&manager, "idempotent errors").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .unwrap();
    let failure = crate::AppError::Provider("provider failed".to_owned()).failure();
    let (first, second) = tokio::join!(
        manager
            .store
            .record_failure_part(session.id, run_id, &failure),
        manager
            .store
            .record_failure_part(session.id, run_id, &failure)
    );
    let first = first.unwrap();
    let second = second.unwrap();
    assert_eq!(first.part_id, second.part_id);
    assert_eq!(first.revision, second.revision, "a replay changes no facts");
    let changed = manager
        .store
        .record_failure_part(
            session.id,
            run_id,
            &crate::AppError::Provider("retries exhausted".to_owned()).failure(),
        )
        .await
        .unwrap();
    assert_eq!(changed.part_id, first.part_id);
    assert_eq!(
        changed.content["problem"]["id"],
        first.content["problem"]["id"]
    );
    assert!(changed.revision > first.revision);
    let saved = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(
        saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .count(),
        1
    );
}

#[tokio::test]
async fn failure_parts_restart_and_old_marker_repair_are_durable_and_idempotent() {
    let manager = test_manager().await;
    let session = create(&manager, "restart errors").await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .unwrap();
    manager
        .store
        .reconcile(session.id, &[run_id])
        .await
        .unwrap();
    let first = manager.store.load_session(session.id).await.unwrap();
    let error = first
        .parts()
        .iter()
        .find(|part| part.kind == "error")
        .unwrap();
    assert_eq!(error.run_id, Some(run_id));
    assert_eq!(error.content["problem"]["code"], "execution.interrupted");
    manager.store.reconcile(session.id, &[]).await.unwrap();
    let repeated = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(first.parts(), repeated.parts());

    let old_run = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content("continue", None, None, None, None),
        )
        .await
        .unwrap();
    manager
        .store
        .complete_run(
            session.id,
            old_run,
            agena_storage::store::RunOutcome {
                status: PartState::Failed,
                abort_reason: Some("provider_error".to_owned()),
                content: None,
                provider_state: None,
            },
        )
        .await
        .unwrap();
    manager.store.reconcile(session.id, &[]).await.unwrap();
    let repaired = manager.store.load_session(session.id).await.unwrap();
    assert!(
        repaired
            .parts()
            .iter()
            .any(|part| part.kind == "error" && part.run_id == Some(old_run))
    );
}

#[tokio::test]
async fn failure_parts_failed_native_operation_keeps_its_started_part() {
    let manager = manager_with_provider(Arc::new(ProviderNativeFixtureProvider {
        model: ModelId::new("provider-native-model"),
        mode: ProviderNativeFixtureMode::FailedCompletion,
    }))
    .await;
    let session = create(&manager, "native tool failure").await;
    let mut request_options = options();
    request_options.model = ModelRef::new("provider-native-fixture", "provider-native-model");
    let saved = manager
        .submit_subtask_user_message(
            SessionUserRunRequest::new(
                session.id,
                request_options,
                vec![TypedContent::Text(text_content("draw"))],
            ),
            None,
        )
        .await
        .unwrap();
    let tools: Vec<_> = saved
        .parts()
        .iter()
        .filter(|part| part.kind == "tool_call")
        .collect();
    assert_eq!(
        tools.len(),
        1,
        "duplicate terminal events update the started operation once"
    );
    assert_eq!(tools[0].state, PartState::Failed);
    let operation = operation_from_part(tools[0]).unwrap();
    assert_eq!(operation.status(), agena_domain::ExecutionStatus::Failed);
    assert!(
        operation
            .error
            .unwrap()
            .failure
            .user
            .fallback
            .contains("Image server unavailable")
    );
    assert_eq!(
        saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .count(),
        0,
        "the tool Part already owns this failure"
    );
}

#[tokio::test]
async fn failure_parts_unexpected_task_failures_get_a_durable_owner() {
    let manager = test_manager().await;
    let session = create(&manager, "unexpected task failure").await;
    let result = manager
        .execute_registered(
            session.id,
            agena_domain::ExecutionSource::User,
            ExecutionConversationTarget::NewTurn,
            "panic fixture",
            |_manager, _control, _steer| async move {
                panic!("unexpected fixture failure");
                #[allow(unreachable_code)]
                Ok::<(), crate::AppError>(())
            },
        )
        .await;
    assert!(result.is_err());
    assert!(!manager.execution_registry.is_active(session.id).await);
    let saved = manager.store.load_session(session.id).await.unwrap();
    let error = saved
        .parts()
        .iter()
        .find(|part| part.kind == "error")
        .expect("panic has a real error Part");
    assert!(saved.parts().iter().any(|part| part.kind == "run"
        && Some(part.part_id) == error.run_id
        && part.state == PartState::Failed));
    assert_eq!(
        saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .count(),
        1
    );
}

#[tokio::test]
async fn failure_parts_compaction_keeps_the_failure_on_its_existing_activity() {
    let manager = test_manager().await;
    let session = create(&manager, "compaction failure ownership").await;
    let result = manager.execute_registered(session.id, agena_domain::ExecutionSource::User, ExecutionConversationTarget::NewTurn, "compaction failure fixture", move |manager, control, _steer| async move {
        let run_id = manager.store.start_run(session.id, "compaction", serde_json::json!({"reply_id": control.reply_id(), "turn_id": control.turn_id()})).await?;
        let error = crate::AppError::Provider("compaction failed".to_owned());
        let mut part = NewPart::pending("compaction", PartRole::Assistant, serde_json::json!({"problem": agena_failure::UserProblem::from(&error.failure()), "attempt_failure": true}));
        part.state = PartState::Failed;
        manager.store.append_parts(session.id, run_id, vec![part]).await?;
        manager.store.complete_run(session.id, run_id, agena_storage::store::RunOutcome { status: PartState::Failed, abort_reason: Some("compaction_failed".to_owned()), content: None, provider_state: None }).await?;
        Err::<(), _>(error)
    }).await;
    assert!(result.is_err());
    let saved = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(
        saved.parts().len(),
        2,
        "one run and its failed compaction Part are sufficient"
    );
    assert_eq!(
        saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .count(),
        0
    );
}

#[tokio::test]
async fn failure_parts_checkpoint_retry_keeps_the_original_error_identity() {
    let manager = test_manager().await;
    let session = create(&manager, "checkpoint retry failure").await;
    let reply_id = AssistantReplyId::new();
    let content = run_marker_content("continue", None, None, None, Some(reply_id));
    let first_run = manager
        .store
        .start_run(session.id, "continue", content.clone())
        .await
        .unwrap();
    let failure = crate::AppError::Provider("first attempt".to_owned()).failure();
    let first = manager
        .store
        .record_failure_part(session.id, first_run, &failure)
        .await
        .unwrap();
    let next_run = manager
        .store
        .start_run(session.id, "continue", content)
        .await
        .unwrap();
    let next = manager
        .store
        .record_failure_part(
            session.id,
            next_run,
            &crate::AppError::Provider("retry attempt".to_owned()).failure(),
        )
        .await
        .unwrap();
    assert_eq!(first.part_id, next.part_id);
    assert_eq!(
        next.run_id,
        Some(first_run),
        "historical ownership stays intact across a checkpoint boundary"
    );
    let saved = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(
        saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .count(),
        1
    );
}

#[tokio::test]
async fn failure_parts_fork_recovery_preserves_shared_history() {
    let manager = test_manager().await;
    let parent = create(&manager, "parent failure history").await;
    let marker_content = run_marker_content(
        "continue",
        None,
        None,
        Some(TurnId::new()),
        Some(AssistantReplyId::new()),
    );
    let parent_run = manager
        .store
        .start_run(parent.id, "continue", marker_content.clone())
        .await
        .unwrap();
    manager
        .store
        .complete_run(
            parent.id,
            parent_run,
            agena_storage::store::RunOutcome {
                status: PartState::Failed,
                abort_reason: Some("provider_error".to_owned()),
                content: None,
                provider_state: None,
            },
        )
        .await
        .unwrap();
    let marker_only_fork = manager
        .store
        .fork(parent.id, parent_run, "legacy marker branch".to_owned())
        .await
        .unwrap();
    let inherited = manager.store.load_session(marker_only_fork).await.unwrap();
    manager
        .store
        .reconcile(marker_only_fork, &[])
        .await
        .unwrap();
    assert_eq!(
        manager
            .store
            .load_session(marker_only_fork)
            .await
            .unwrap()
            .parts(),
        inherited.parts(),
        "opening a fork must not repair or mutate an ancestor's marker"
    );

    let parent_error = manager
        .store
        .record_provider_retry(
            parent.id,
            parent_run,
            &agena_domain::ProviderRetryStatus {
                attempt: 1,
                max_retries: 3,
                next_at_ms: 10_000,
                message: "The provider request timed out.".to_owned(),
            },
        )
        .await
        .unwrap();
    let branch = manager
        .store
        .fork(parent.id, parent_error.part_id, "retry branch".to_owned())
        .await
        .unwrap();
    manager.store.reconcile(branch, &[]).await.unwrap();
    let branch_run = manager
        .store
        .start_run(branch, "continue", marker_content)
        .await
        .unwrap();
    let branch_error = manager
        .store
        .record_failure_part(
            branch,
            branch_run,
            &crate::AppError::Provider("branch request failed".to_owned()).failure(),
        )
        .await
        .unwrap();
    assert_ne!(branch_error.part_id, parent_error.part_id);
    assert_eq!(branch_error.origin_session_id, branch);
    assert_eq!(branch_error.run_id, Some(branch_run));
    let saved_parent = manager.store.load_session(parent.id).await.unwrap();
    assert_eq!(
        saved_parent
            .parts()
            .iter()
            .find(|part| part.kind == "error"),
        Some(&parent_error),
        "branch repair and retries leave the shared parent error untouched"
    );
}
