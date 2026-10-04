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

async fn retry_transcript_case(fail: bool) {
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
        agena_runtime::SessionRunOptions {
            model: ModelRef::new("retry-fixture", "retry-model"),
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: Some(0.0),
            max_output_tokens: Some(64),
        },
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
            while !manager
                .retry_registry
                .snapshot(session.id)
                .is_some_and(|retry| retry.attempt == attempt)
            {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("retry status must be observable while the stream is waiting");

        let saved = manager.store.load_session(session.id).await.unwrap();
        assert!(
            saved.parts().iter().all(|part| {
                part.kind == "run" || (part.role == PartRole::User && part.kind == "text")
            }),
            "temporary retries must not create transcript content or error parts"
        );
        let part_ids: Vec<_> = saved.parts().iter().map(|part| part.part_id).collect();
        if let Some(initial) = &initial_part_ids {
            assert_eq!(&part_ids, initial, "retries must not append history rows");
        } else {
            initial_part_ids = Some(part_ids);
        }
    }

    if fail {
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
    assert_eq!(result.is_err(), fail);
    assert!(manager.retry_registry.snapshot(session.id).is_none());

    let saved = manager.store.load_session(session.id).await.unwrap();
    let errors: Vec<_> = saved
        .parts()
        .iter()
        .filter(|part| part.kind == "error")
        .collect();
    assert_eq!(errors.len(), usize::from(fail));
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
    } else {
        assert!(saved.parts().iter().any(|part| {
            part.role == PartRole::Assistant
                && part.kind == "text"
                && part.content.to_string().contains("Recovered successfully")
        }));
    }
}

#[tokio::test]
async fn provider_retry_success_does_not_write_error_parts() {
    retry_transcript_case(false).await;
}

#[tokio::test]
async fn provider_retry_failure_writes_one_final_error_part() {
    retry_transcript_case(true).await;
}
