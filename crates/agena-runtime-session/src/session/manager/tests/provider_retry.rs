use super::*;
use std::{pin::Pin, sync::Mutex, time::Duration};
use tokio::sync::mpsc;

type Event = Result<CompletionStreamEvent, ProviderError>;
type EventStream = Pin<Box<dyn futures_util::Stream<Item = Event> + Send>>;

struct RetryProvider {
    model: ModelId,
    events: Mutex<Option<mpsc::UnboundedReceiver<Event>>>,
}

#[async_trait::async_trait]
impl ModelRuntime for RetryProvider {
    fn id(&self) -> &str {
        "retry-fixture"
    }

    fn default_model(&self) -> &ModelId {
        &self.model
    }

    async fn list_models(&self) -> Result<Vec<agena_domain::Model>, ProviderError> {
        Ok(Vec::new())
    }

    async fn complete(
        &self,
        _request: CompletionRequest,
    ) -> Result<CompletionResponse, ProviderError> {
        unreachable!("retry fixture only supports streaming")
    }

    async fn complete_stream(
        &self,
        _request: CompletionRequest,
    ) -> Result<EventStream, ProviderError> {
        let events = self
            .events
            .lock()
            .unwrap()
            .take()
            .expect("one provider turn");
        Ok(Box::pin(futures_util::stream::unfold(
            events,
            |mut events| async move { events.recv().await.map(|event| (event, events)) },
        )))
    }
}

fn retry_options() -> agena_runtime::SessionRunOptions {
    agena_runtime::SessionRunOptions {
        model_supports_parallel_tool_calls: None,
        model: ModelRef::new("retry-fixture", "retry-model"),
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

async fn wait_retry_phase(
    manager: &SessionManager,
    session_id: i64,
    phase: &str,
) -> agena_storage::store::Part {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let saved = manager.store.load_session(session_id).await.unwrap();
            if let Some(part) = saved.parts().iter().find(|part| {
                part.kind == "error"
                    && part
                        .content
                        .pointer("/retry/phase")
                        .and_then(serde_json::Value::as_str)
                        == Some(phase)
            }) {
                return part.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("the durable retry phase must be visible without completion")
}

async fn retry_transcript_case(fail: bool, cancelled: bool) {
    let (sender, events) = mpsc::unbounded_channel();
    let provider_id = ProviderId::new("retry-fixture");
    let model = ModelId::new("retry-model");
    let manager = Arc::new(
        manager_with_provider(Arc::new(RetryProvider {
            model: model.clone(),
            events: Mutex::new(Some(events)),
        }))
        .await,
    );
    let session = create(&manager, "provider retry transcript").await;
    let request = SessionUserRunRequest::new(
        session.id,
        retry_options(),
        vec![TypedContent::Text(text_content("answer after retrying"))],
    );
    let task = tokio::spawn({
        let manager = manager.clone();
        async move { manager.submit_subtask_user_message(request, None).await }
    });
    let mut initial_part_ids = None;
    for attempt in 1..=3 {
        sender
            .send(Ok(CompletionStreamEvent::ProviderRetry {
                provider_id: provider_id.clone(),
                model: model.clone(),
                message: "fixture provider timed out".into(),
                retry_index: attempt - 1,
                attempt,
                max_retries: 3,
                delay_ms: 10_000,
            }))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let saved = manager.store.load_session(session.id).await.unwrap();
                if saved.parts().iter().any(|part| {
                    part.kind == "error"
                        && part
                            .content
                            .pointer("/retry/attempt")
                            .and_then(serde_json::Value::as_u64)
                            == Some(attempt as u64)
                }) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("retry status must be observable while the stream is waiting");

        let saved = manager.store.load_session(session.id).await.unwrap();
        let errors: Vec<_> = saved
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .collect();
        assert_eq!(errors.len(), 1, "all retries share one durable error Part");
        assert_eq!(errors[0].content["retry"]["outcome"], "retrying");
        assert_eq!(errors[0].content["retry"]["phase"], "backoff");
        assert!(errors[0].content["retry"]["next_at_ms"].is_i64());
        assert!(errors[0].content["retry"].get("started_at_ms").is_none());
        assert!(
            !errors[0].content.to_string().contains("fixture provider"),
            "raw diagnostics are not user content"
        );
        let part_ids: Vec<_> = saved.parts().iter().map(|part| part.part_id).collect();
        if let Some(initial) = &initial_part_ids {
            assert_eq!(&part_ids, initial, "retries must not append history rows");
        } else {
            initial_part_ids = Some(part_ids);
        }
        sender
            .send(Ok(CompletionStreamEvent::ProviderRetryStarted {
                provider_id: provider_id.clone(),
                model: model.clone(),
            }))
            .unwrap();
        let started = wait_retry_phase(&manager, session.id, "requesting").await;
        assert_eq!(started.part_id, errors[0].part_id);
        assert_eq!(started.content["retry"]["attempt"], attempt);
        assert!(started.content["retry"].get("next_at_ms").is_none());
        assert!(started.content["retry"]["started_at_ms"].is_i64());
    }

    if cancelled {
        manager
            .cancel_active_execution_with_outcome(session.id)
            .await
            .unwrap();
    } else if fail {
        sender
            .send(Err(ProviderError::Provider(
                "fixture retries exhausted".into(),
            )))
            .unwrap();
    } else {
        sender
            .send(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: "Recovered successfully".into(),
            }))
            .unwrap();
        let streaming = wait_retry_phase(&manager, session.id, "streaming").await;
        // A duplicated late start cannot move a live response backwards or
        // reset its elapsed-time origin.
        manager
            .store
            .advance_provider_retry(
                session.id,
                streaming.run_id.unwrap(),
                agena_failure::RetryPhase::Requesting,
            )
            .await
            .unwrap();
        assert_eq!(
            streaming,
            wait_retry_phase(&manager, session.id, "streaming").await
        );
        sender
            .send(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: "!".into(),
            }))
            .unwrap();
        // A second content event must not write another retry-phase revision.
        sender
            .send(Ok(CompletionStreamEvent::ThinkingDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: "done".into(),
            }))
            .unwrap();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let saved = manager.store.load_session(session.id).await.unwrap();
                if saved.parts().iter().any(|part| part.kind == "think") {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("later content must be processed before comparing the error revision");
        let unchanged = wait_retry_phase(&manager, session.id, "streaming").await;
        assert_eq!(streaming, unchanged);
        sender
            .send(Ok(CompletionStreamEvent::Completed {
                provider_id,
                model,
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: Some(CompletionUsage::default()),
                provider_metadata: None,
                end_turn: None,
            }))
            .unwrap();
    }
    drop(sender);
    let result = tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("provider turn must finish")
        .expect("provider task must not panic");
    assert_eq!(result.is_err(), fail || cancelled);
    assert!(manager.retry_registry.snapshot(session.id).is_none());

    let saved = manager.store.load_session(session.id).await.unwrap();
    let errors: Vec<_> = saved
        .parts()
        .iter()
        .filter(|part| part.kind == "error")
        .collect();
    assert_eq!(errors.len(), 1, "recovery must retain failure history");
    for field in ["next_at_ms", "phase", "started_at_ms"] {
        assert!(
            errors[0].content["retry"].get(field).is_none(),
            "terminal retries have no live clock"
        );
    }
    manager
        .store
        .advance_provider_retry(
            session.id,
            errors[0].run_id.unwrap(),
            agena_failure::RetryPhase::Requesting,
        )
        .await
        .unwrap();
    let unchanged = manager.store.load_session(session.id).await.unwrap();
    assert_eq!(
        Some(errors[0]),
        unchanged
            .parts()
            .iter()
            .find(|part| part.part_id == errors[0].part_id),
        "late progress cannot reopen terminal retry history"
    );
    assert!(
        initial_part_ids
            .as_ref()
            .unwrap()
            .contains(&errors[0].part_id)
    );
    assert_eq!(
        errors[0].content["retry"]["outcome"],
        if cancelled {
            "cancelled"
        } else if fail {
            "failed"
        } else {
            "recovered"
        }
    );
    if let Ok(returned) = result.as_ref() {
        let returned_errors: Vec<_> = returned
            .parts()
            .iter()
            .filter(|part| part.kind == "error")
            .collect();
        assert_eq!(
            returned_errors, errors,
            "the returned Session must include the same current error Part as storage"
        );
    }
    if fail {
        assert_eq!(errors[0].state, PartState::Failed);
        assert!(
            errors[0]
                .content
                .to_string()
                .contains("fixture retries exhausted")
        );
        let run_id = errors[0]
            .run_id
            .expect("failure belongs to the assistant run");
        assert!(saved.parts().iter().any(|part| {
            part.part_id == run_id && part.kind == "run" && part.state == PartState::Failed
        }));
    } else if !cancelled {
        let texts = super::resource_part_texts(&manager, saved.parts()).await;
        assert!(saved.parts().iter().any(|part| {
            part.role == PartRole::Assistant
                && texts
                    .get(&part.part_id)
                    .is_some_and(|text| text.contains("Recovered successfully"))
        }));
    }
}

async fn processor_retry_snapshot_case(fail: bool, checkpoint: bool) {
    let (sender, events) = mpsc::unbounded_channel();
    let manager = manager_with_provider(Arc::new(RetryProvider {
        model: ModelId::new("retry-model"),
        events: Mutex::new(Some(events)),
    }))
    .await;
    let mut session = create(&manager, "retry snapshot").await;
    let marker_content = run_marker_content(
        "continue",
        Some("retry-fixture"),
        Some("retry-model"),
        Some(TurnId::new()),
        Some(AssistantReplyId::new()),
    );
    let original_error = if checkpoint {
        let previous_run = manager
            .store
            .start_run(session.id, "continue", marker_content.clone())
            .await
            .unwrap();
        let error = manager
            .store
            .record_failure_part(
                session.id,
                previous_run,
                &crate::AppError::Provider("previous request failed".to_owned()).failure(),
            )
            .await
            .unwrap();
        manager
            .store
            .complete_run(
                session.id,
                previous_run,
                agena_storage::store::RunOutcome {
                    status: PartState::Failed,
                    abort_reason: Some("provider_error".to_owned()),
                    content: None,
                    provider_state: None,
                },
            )
            .await
            .unwrap();
        session = manager.store.load_session(session.id).await.unwrap();
        Some(error)
    } else {
        None
    };
    let run_id = manager
        .store
        .start_run(session.id, "continue", marker_content.clone())
        .await
        .unwrap();
    sender
        .send(Ok(CompletionStreamEvent::ProviderRetry {
            provider_id: ProviderId::new("retry-fixture"),
            model: ModelId::new("retry-model"),
            message: "provider timed out".to_owned(),
            retry_index: 0,
            attempt: 1,
            max_retries: 3,
            delay_ms: 10_000,
        }))
        .unwrap();
    if fail {
        sender
            .send(Err(ProviderError::Provider("retries exhausted".to_owned())))
            .unwrap();
    } else {
        sender
            .send(Ok(CompletionStreamEvent::TextDelta {
                provider_id: ProviderId::new("retry-fixture"),
                model: ModelId::new("retry-model"),
                delta: "Recovered answer".to_owned(),
            }))
            .unwrap();
        sender
            .send(Ok(CompletionStreamEvent::Completed {
                provider_id: ProviderId::new("retry-fixture"),
                model: ModelId::new("retry-model"),
                finish_reason: Some(CompletionFinishReason::Stop),
                usage: Some(CompletionUsage::default()),
                provider_metadata: None,
                end_turn: None,
            }))
            .unwrap();
    }
    drop(sender);
    let options = retry_options();
    let run = SessionRunRequest {
        content_writers: Default::default(),
        retry_registry: Default::default(),
        session_id: session.id,
        model: options.model.clone(),
        completion: super::super::completion_request(
            &options,
            None,
            Vec::new(),
            Vec::new(),
            None,
            None,
            None,
        ),
        next_message_id: run_id,
        marker_content,
        input_notification_part_ids: Vec::new(),
        prompt_tokens: None,
        part_ids: ProcessorPartIdAllocator,
        next_call_id: 0,
        store: manager.store.clone(),
        cancel: None,
    };
    let state = manager.execution_state();
    let result = state
        .processor
        .run_turn(run, &state.provider_registry)
        .await
        .unwrap();
    assert_eq!(
        matches!(result.termination, SessionRunTermination::Failed(_)),
        fail
    );
    let saved = manager.store.load_session(session.id).await.unwrap();
    let error = saved
        .parts()
        .iter()
        .find(|part| part.kind == "error")
        .unwrap();
    assert_eq!(
        error.content["retry"]["outcome"],
        if fail { "failed" } else { "recovered" }
    );
    assert_eq!(
        result
            .part_snapshot
            .parts()
            .iter()
            .find(|part| part.kind == "error"),
        Some(error),
        "the processor must return its durable failure row alongside ordinary content"
    );
    if let Some(original) = original_error {
        assert_eq!(error.part_id, original.part_id);
        assert_eq!(error.run_id, original.run_id);
        assert_eq!(
            error.content["problem"]["id"],
            original.content["problem"]["id"]
        );
    }
    session.apply_part_snapshot(result.part_snapshot).unwrap();
    assert_eq!(session.parts(), saved.parts());
}

#[tokio::test]
async fn processor_failed_retry_snapshot_includes_its_error_part() {
    processor_retry_snapshot_case(true, false).await;
}

#[tokio::test]
async fn processor_recovered_checkpoint_snapshot_updates_the_original_error_part() {
    processor_retry_snapshot_case(false, true).await;
}

#[tokio::test]
async fn provider_retry_success_preserves_the_same_error_part() {
    retry_transcript_case(false, false).await;
}

#[tokio::test]
async fn provider_retry_failure_updates_one_error_part() {
    retry_transcript_case(true, false).await;
}

#[tokio::test]
async fn provider_retry_cancel_preserves_history_and_seals_retry_progress() {
    retry_transcript_case(false, true).await;
}
