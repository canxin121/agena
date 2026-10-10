use super::*;
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering};

struct ContextProvider {
    model: ModelId,
    requests: std::sync::Mutex<Vec<CompletionRequest>>,
    summaries: std::sync::Mutex<VecDeque<(&'static str, CompletionFinishReason)>>,
    native_attempts: AtomicUsize,
    prompt_tokens: u64,
    repaired: bool,
    limits: agena_domain::ModelTokenLimits,
    overflow_first: bool,
}

impl ContextProvider {
    fn new(summaries: Vec<(&'static str, CompletionFinishReason)>) -> Self {
        Self {
            model: ModelId::new("large-context"),
            requests: Default::default(),
            summaries: std::sync::Mutex::new(summaries.into()),
            native_attempts: AtomicUsize::new(0),
            prompt_tokens: 320_000,
            repaired: false,
            limits: agena_domain::ModelTokenLimits {
                context_window_tokens: Some(1_000_000),
                max_input_tokens: Some(1_000_000),
                max_output_tokens: Some(384_000),
            },
            overflow_first: false,
        }
    }
}

#[async_trait::async_trait]
impl ModelRuntime for ContextProvider {
    fn id(&self) -> &str {
        "context-fixture"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    fn model_metadata(&self, _model: &ModelId) -> agena_domain::ModelMetadata {
        agena_domain::ModelMetadata {
            limits: self.limits.clone(),
            ..Default::default()
        }
    }
    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn compact_conversation(
        &self,
        _request: CompletionRequest,
    ) -> Result<Option<agena_provider::ProviderCompactionOutput>, ProviderError> {
        self.native_attempts.fetch_add(1, Ordering::SeqCst);
        Err(ProviderError::HttpStatus {
            provider: self.id().to_owned(),
            status: reqwest::StatusCode::NOT_FOUND,
            body: "Not Found".to_owned(),
            kind: agena_provider::ProviderErrorKind::InvalidRequest,
            retryable: false,
        })
    }
    async fn complete(
        &self,
        request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        self.requests.lock().unwrap().push(request);
        if self.overflow_first && self.requests.lock().unwrap().len() == 1 {
            return Err(ProviderError::ProviderClassified {
                provider: self.id().to_owned(),
                message: "context length exceeded".to_owned(),
                kind: agena_provider::ProviderErrorKind::ContextOverflow,
                retryable: false,
            });
        }
        let (text, finish_reason) = self
            .summaries
            .lock()
            .unwrap()
            .pop_front()
            .expect("expected compaction attempt");
        Ok(CompletionResponse {
            provider_id: ProviderId::new(self.id()),
            model: self.model.clone(),
            text: text.to_owned(),
            reasoning_text: text
                .is_empty()
                .then(|| "Still reasoning about the checkpoint.".to_owned()),
            finish_reason: Some(finish_reason),
            tool_calls: Vec::new(),
            usage: Some(CompletionUsage {
                input_tokens: 320_000,
                reasoning_tokens: 4_096,
                ..Default::default()
            }),
            provider_metadata: None,
        })
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
        self.requests.lock().unwrap().push(request);
        Ok(Box::pin(futures_util::stream::iter(vec![
            Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new(self.id()),
                model: self.model.clone(),
                delta: "done".to_owned(),
            }),
            Ok(CompletionStreamEvent::Completed {
                provider_id: ProviderId::new(self.id()),
                model: self.model.clone(),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: Some(CompletionUsage {
                    input_tokens: self.prompt_tokens * if self.repaired { 2 } else { 1 },
                    output_tokens: 1,
                    request_usage: self.repaired.then_some(
                        agena_domain::PromptTokenUsageSnapshot {
                            input_tokens: self.prompt_tokens,
                            output_tokens: 1,
                            ..Default::default()
                        },
                    ),
                    ..Default::default()
                }),
                provider_metadata: None,
                end_turn: None,
            }),
        ])))
    }
}

fn options() -> agena_runtime::SessionRunOptions {
    agena_runtime::SessionRunOptions {
        model: ModelRef::new("context-fixture", "large-context"),
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: None,
        max_output_tokens: None,
    }
}

async fn dense_history(manager: &SessionManager) -> Session {
    let mut session = create_with_model(
        manager,
        "session 126 regression",
        "context-fixture",
        "large-context",
    )
    .await;
    // More than the old 616k admission limit; every resource remains below
    // its model-preview bound so this exercises the complete request.
    for _ in 0..32 {
        session = append_message(
            manager,
            session,
            Role::User,
            vec![TypedContent::Text(text_content("x".repeat(80_000)))],
        )
        .await;
    }
    session = append_message(
        manager,
        session,
        Role::User,
        vec![TypedContent::Text(text_content(
            "Keep the permission fix and finish its verification.",
        ))],
    )
    .await;
    let run_id = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("context-fixture"),
                Some("large-context"),
                Some(TurnId::new()),
                Some(AssistantReplyId::new()),
            ),
        )
        .await
        .unwrap();
    manager
        .store
        .append_parts(
            session.id,
            run_id,
            vec![
                new_part_from_content(
                    "text",
                    PartRole::Assistant,
                    &TypedContent::Text(text_content("Prior work awaits verification.")),
                    PartState::Completed,
                )
                .unwrap(),
            ],
        )
        .await
        .unwrap();
    manager
        .store
        .complete_run(
            session.id,
            run_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .unwrap();
    manager.get_session(session.id).await.unwrap()
}

#[tokio::test]
async fn large_output_capability_does_not_reject_the_session_126_window() {
    let provider = Arc::new(ContextProvider::new(Vec::new()));
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let before = manager.session_usage_async(&session).await.unwrap();
    assert!(before.current_tokens > 616_000);
    assert_eq!(before.input_limit_tokens, Some(968_000));
    assert_eq!(before.limit_tokens, Some(948_000));
    assert_eq!(before.model_max_output_tokens, Some(384_000));
    let completed = manager
        .continue_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    assert!(
        completed
            .parts()
            .iter()
            .any(|part| part.kind == "text" && part.role == PartRole::Assistant)
    );
    assert_eq!(
        provider.requests.lock().unwrap()[0].max_output_tokens,
        Some(32_000)
    );

    let loaded = manager.get_session(session.id).await.unwrap();
    let usage = manager.session_usage_async(&loaded).await.unwrap();
    assert_eq!(usage.measured_prompt_tokens, Some(320_000));
    assert_eq!(usage.projected_tokens, Some(320_001));
    assert_eq!(
        usage.current_tokens, 320_001,
        "the measured prefix must survive reload and outrank its rough estimate"
    );

    let loaded = append_message(
        &manager,
        loaded,
        Role::User,
        vec![TypedContent::Text(text_content("a".repeat(4_000)))],
    )
    .await;
    let usage = manager.session_usage_async(&loaded).await.unwrap();
    assert_eq!(usage.current_tokens, 321_001, "only new input is estimated");

    let tool_run = manager
        .store
        .start_run(
            session.id,
            "continue",
            run_marker_content(
                "continue",
                Some("context-fixture"),
                Some("large-context"),
                Some(TurnId::new()),
                Some(AssistantReplyId::new()),
            ),
        )
        .await
        .unwrap();
    let mut invocation = ToolInvocation::new("fixture.large_result", StructuredObject::default());
    invocation.tool_api_call = Some(agena_domain::ToolApiCall {
        function: agena_domain::ToolApiFunction::Call,
        arguments: StructuredObject::try_from(serde_json::json!({
            "tool": "fixture.large_result", "input": {},
        }))
        .unwrap(),
    });
    let operation = agena_runtime_contracts::part::OperationPart::completed(
        7,
        invocation,
        agena_domain::RawOutput::text("z".repeat(400_000)),
        TimeRange {
            start_ms: 1,
            end_ms: Some(2),
        },
    );
    manager
        .store
        .append_parts(
            session.id,
            tool_run,
            vec![
                new_part_from_content(
                    "tool_call",
                    PartRole::Assistant,
                    &TypedContent::ToolCall(Box::new(tool_call_from_operation(&operation))),
                    PartState::Completed,
                )
                .unwrap(),
            ],
        )
        .await
        .unwrap();
    manager
        .store
        .complete_run(
            session.id,
            tool_run,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: None,
                provider_state: None,
            },
        )
        .await
        .unwrap();
    let loaded = manager.get_session(session.id).await.unwrap();
    let tool_usage = manager.session_usage_async(&loaded).await.unwrap();
    assert!(tool_usage.projected_tokens.is_some());
    assert!(tool_usage.current_tokens > usage.current_tokens);
    assert!(
        tool_usage.current_tokens < usage.current_tokens + 15_000,
        "only the bounded model preview, not the retained tool body, counts as new input"
    );
    let switched = manager
        .set_session_model_override(
            session.id,
            ModelRef::new("context-fixture", "another-model"),
        )
        .await
        .unwrap();
    let usage = manager.session_usage_async(&switched).await.unwrap();
    assert!(
        usage.projected_tokens.is_none(),
        "a different model invalidates the measurement"
    );
    assert!(usage.current_tokens > 616_000);
}

#[tokio::test]
async fn compaction_recovers_empty_reasoning_and_remembers_a_missing_native_endpoint() {
    let provider = Arc::new(ContextProvider::new(vec![
        ("", CompletionFinishReason::Length),
        (
            "Permission fix is in progress. Run the pending regression tests.",
            CompletionFinishReason::Stop,
        ),
        (
            "Permission fix is verified; continue with the current user request.",
            CompletionFinishReason::Stop,
        ),
    ]));
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let mut run_options = options();
    run_options.thinking = Some(agena_domain::ThinkingRequest::Budget {
        budget_tokens: 8_192,
    });
    run_options
        .request_override
        .body_patch
        .insert("max_output_tokens".into(), serde_json::json!(384_000));
    let request = agena_runtime::SessionExecutionRequest::new(session.id, run_options);
    let compacted = manager.compact_session(request.clone()).await.unwrap();
    assert!(compacted.runtime.prompt_window.compaction.is_some());
    assert!(!compacted.runtime.prompt_window.auto_compaction_disabled);
    let requests = provider.requests.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert_eq!(requests[0].max_output_tokens, Some(32_000));
    assert_eq!(requests[1].max_output_tokens, Some(64_000));
    assert_eq!(
        requests[0].request_override.body_patch["max_output_tokens"],
        32_000
    );
    assert_eq!(
        requests[1].request_override.body_patch["max_output_tokens"],
        64_000
    );
    assert_eq!(requests[0].thinking, request.options.thinking);
    assert_eq!(requests[1].thinking, request.options.thinking);
    assert!(
        requests
            .iter()
            .all(|request| request.disable_tools && request.tool_api_functions.is_empty())
    );
    let reloaded = manager.get_session(session.id).await.unwrap();
    assert!(
        !reloaded
            .runtime
            .prompt_window
            .remote_compaction_disabled_models
            .is_empty()
    );
    let reloaded = append_message(
        &manager,
        reloaded,
        Role::User,
        vec![TypedContent::Text(text_content("z".repeat(80_000)))],
    )
    .await;
    manager
        .compact_session(agena_runtime::SessionExecutionRequest::new(
            reloaded.id,
            request.options,
        ))
        .await
        .unwrap();
    assert_eq!(
        provider.native_attempts.load(Ordering::SeqCst),
        1,
        "404 must not be retried after a reload"
    );
}

#[tokio::test]
async fn repaired_compaction_policy_recovers_sessions_disabled_by_legacy_failures() {
    let manager = test_manager().await;
    let session = create(&manager, "old compaction failures").await;
    for _ in 0..3 {
        let run = manager
            .store
            .start_run(
                session.id,
                "compaction",
                serde_json::json!({
                    "summary": null, "checkpoint": null, "attempt_failure": true,
                    "compaction_policy_version": 2,
                }),
            )
            .await
            .unwrap();
        manager
            .store
            .complete_run(
                session.id,
                run,
                agena_storage::store::RunOutcome {
                    status: PartState::Failed,
                    abort_reason: Some("compaction_failed".into()),
                    content: None,
                    provider_state: None,
                },
            )
            .await
            .unwrap();
    }
    let loaded = manager.get_session(session.id).await.unwrap();
    assert_eq!(
        loaded.runtime.prompt_window.consecutive_compaction_failures,
        0
    );
    assert!(!loaded.runtime.prompt_window.auto_compaction_disabled);
    for _ in 0..3 {
        let loaded = manager.get_session(session.id).await.unwrap();
        manager.persist_compaction_failure(loaded).await.unwrap();
    }
    let loaded = manager.get_session(session.id).await.unwrap();
    assert_eq!(
        loaded.runtime.prompt_window.consecutive_compaction_failures,
        3
    );
    assert!(
        loaded.runtime.prompt_window.auto_compaction_disabled,
        "the repaired strategy still bounds repeated failures"
    );
}

#[tokio::test]
async fn measured_overflow_automatically_compacts_before_the_next_request() {
    let mut fixture = ContextProvider::new(vec![(
        "Keep the permission fix and finish verification.",
        CompletionFinishReason::Stop,
    )]);
    fixture.prompt_tokens = 980_000;
    let provider = Arc::new(fixture);
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    manager
        .continue_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    let loaded = manager.get_session(session.id).await.unwrap();
    assert!(
        manager
            .session_usage_async(&loaded)
            .await
            .unwrap()
            .current_tokens
            > 968_000
    );
    let compacted = manager
        .continue_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    let checkpoint = compacted.runtime.prompt_window.compaction.as_ref().unwrap();
    assert_eq!(
        checkpoint.trigger,
        agena_domain::PromptCompactionTrigger::Auto
    );
    assert!(checkpoint.after_tokens < 968_000);
    assert_eq!(provider.requests.lock().unwrap().len(), 3);
}

#[tokio::test]
async fn old_aggregated_prompt_measurements_are_reestimated_on_reload() {
    let provider = Arc::new(ContextProvider::new(Vec::new()));
    let manager = manager_with_provider(provider).await;
    let session = dense_history(&manager).await;
    manager
        .continue_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    let loaded = manager.get_session(session.id).await.unwrap();
    let marker = loaded
        .parts()
        .iter()
        .rev()
        .find(|part| part.is_run_marker())
        .unwrap();
    let mut content = marker.content.clone();
    let measurement = &mut content["rounds"][0]["prompt_usage"];
    measurement
        .as_object_mut()
        .unwrap()
        .remove("accounting_version");
    measurement["last_successful_usage"]["input_tokens"] = serde_json::json!(1_370_000);
    manager
        .store
        .complete_run(
            session.id,
            marker.part_id,
            agena_storage::store::RunOutcome {
                status: PartState::Completed,
                abort_reason: None,
                content: Some(content),
                provider_state: None,
            },
        )
        .await
        .unwrap();
    let loaded = manager.get_session(session.id).await.unwrap();
    let usage = manager.session_usage_async(&loaded).await.unwrap();
    assert!(usage.measured_prompt_tokens.is_none());
    assert!(usage.current_tokens > 616_000 && usage.current_tokens < 968_000);
}

#[tokio::test]
async fn protocol_repair_billing_does_not_double_the_persisted_context_usage() {
    let mut fixture = ContextProvider::new(Vec::new());
    fixture.prompt_tokens = 680_000;
    fixture.repaired = true;
    let provider = Arc::new(fixture);
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    for _ in 0..2 {
        manager
            .continue_session(agena_runtime::SessionExecutionRequest::new(
                session.id,
                options(),
            ))
            .await
            .unwrap();
        let loaded = manager.get_session(session.id).await.unwrap();
        let usage = manager.session_usage_async(&loaded).await.unwrap();
        assert_eq!(usage.measured_prompt_tokens, Some(680_000));
        assert_eq!(usage.current_tokens, 680_001);
        assert!(loaded.runtime.prompt_window.compaction.is_none());
    }
    assert_eq!(provider.requests.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn output_override_is_used_by_admission_and_the_provider_request() {
    let provider = Arc::new(ContextProvider::new(Vec::new()));
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let mut run_options = options();
    run_options.max_output_tokens = Some(32_000);
    run_options
        .request_override
        .body_patch
        .insert("max_output_tokens".into(), serde_json::json!(64_000));
    manager
        .apply_selection_modes_to_run_options(&session, &mut run_options)
        .unwrap();
    let usage = manager
        .session_usage_for_run_options(
            &session,
            Some(run_options.clone()),
            manager.execution_state(),
        )
        .await
        .unwrap();
    assert_eq!(usage.input_limit_tokens, Some(936_000));
    assert_eq!(usage.limit_tokens, Some(916_000));
    manager
        .continue_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            run_options,
        ))
        .await
        .unwrap();
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests[0].max_output_tokens, Some(64_000));
    assert_eq!(
        requests[0].request_override.body_patch["max_output_tokens"],
        64_000
    );
}

#[tokio::test]
async fn failed_summary_recovery_keeps_history_and_native_capability_state() {
    let provider = Arc::new(ContextProvider::new(vec![
        ("", CompletionFinishReason::Stop),
        ("", CompletionFinishReason::Length),
    ]));
    let manager = manager_with_provider(provider).await;
    let session = dense_history(&manager).await;
    let error = manager
        .compact_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap_err();
    assert!(error.to_string().contains("reasoning_tokens"));
    let loaded = manager.get_session(session.id).await.unwrap();
    assert!(loaded.runtime.prompt_window.compaction.is_none());
    assert!(
        !loaded
            .runtime
            .prompt_window
            .remote_compaction_disabled_models
            .is_empty()
    );
    assert!(loaded.active_window_parts().len() >= session.parts().len());
    assert_eq!(
        loaded.runtime.prompt_window.consecutive_compaction_failures,
        1
    );
}

#[tokio::test]
async fn small_model_compactor_keeps_history_within_its_input_ceiling() {
    let mut fixture = ContextProvider::new(vec![(
        "Keep working on the permission fix.",
        CompletionFinishReason::Stop,
    )]);
    fixture.limits = agena_domain::ModelTokenLimits {
        context_window_tokens: Some(8_192),
        max_input_tokens: Some(6_000),
        max_output_tokens: Some(2_048),
    };
    let provider = Arc::new(fixture);
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let compacted = manager
        .compact_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    assert!(compacted.runtime.prompt_window.compaction.is_some());
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 1);
    assert!(serde_json::to_vec(&requests[0]).unwrap().len() <= 6_000 * 4);
    assert_eq!(requests[0].max_output_tokens, Some(1_024));
}

#[tokio::test]
async fn compactor_recovers_provider_overflow_by_reducing_history_once() {
    let mut fixture = ContextProvider::new(vec![(
        "Continue the permission fix and run its tests.",
        CompletionFinishReason::Stop,
    )]);
    fixture.overflow_first = true;
    fixture.limits = agena_domain::ModelTokenLimits {
        context_window_tokens: Some(65_536),
        max_input_tokens: Some(32_768),
        max_output_tokens: Some(16_384),
    };
    let provider = Arc::new(fixture);
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let compacted = manager
        .compact_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await
        .unwrap();
    assert!(compacted.runtime.prompt_window.compaction.is_some());
    let requests = provider.requests.lock().unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        serde_json::to_vec(&requests[1]).unwrap().len()
            < serde_json::to_vec(&requests[0]).unwrap().len()
    );
    assert_eq!(requests[0].thinking, requests[1].thinking);
}

#[tokio::test]
async fn unrepresentable_compaction_preserves_history_without_sending_an_oversized_request() {
    let mut fixture = ContextProvider::new(Vec::new());
    fixture.limits = agena_domain::ModelTokenLimits {
        context_window_tokens: Some(1_024),
        max_input_tokens: Some(512),
        max_output_tokens: Some(128),
    };
    let provider = Arc::new(fixture);
    let manager = manager_with_provider(provider.clone()).await;
    let session = dense_history(&manager).await;
    let original = session.parts().to_vec();
    let result = manager
        .compact_session(agena_runtime::SessionExecutionRequest::new(
            session.id,
            options(),
        ))
        .await;
    assert!(result.is_err());
    assert!(provider.requests.lock().unwrap().is_empty());
    let loaded = manager.get_session(session.id).await.unwrap();
    assert!(loaded.runtime.prompt_window.compaction.is_none());
    for part in original {
        assert_eq!(
            loaded
                .parts()
                .iter()
                .find(|current| current.part_id == part.part_id),
            Some(&part)
        );
    }
}
