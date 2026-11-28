use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::time::Duration;

use super::*;
use crate::provider::core::CompletionEventStream;
use agena_domain::{Model, ModelId, ProviderId};

struct EmptyThenText {
    model: ModelId,
    calls: Arc<AtomicUsize>,
    always_empty: bool,
}

#[async_trait::async_trait]
impl ModelRuntime for EmptyThenText {
    fn id(&self) -> &str {
        "performance-fixture"
    }
    fn default_model(&self) -> &ModelId {
        &self.model
    }
    async fn list_models(&self) -> Result<Vec<Model>, ProviderError> {
        Ok(Vec::new())
    }
    async fn complete(&self, _: CompletionRequest) -> Result<CompletionResponse, ProviderError> {
        unreachable!("the fixture only serves streaming requests")
    }
    fn supports_native_compaction(&self, model: &ModelId) -> bool {
        model == &self.model
    }
    async fn compact_conversation(
        &self,
        request: CompletionRequest,
    ) -> Result<Option<ProviderCompactionOutput>, ProviderError> {
        if !self.supports_native_compaction(&request.model) {
            return Ok(None);
        }
        Ok(Some(ProviderCompactionOutput::OpenAiResponses {
            items: vec![serde_json::json!({"type":"compaction","encrypted_content":"fixture"})],
            response_id: Some("cmp_fixture".into()),
            usage: Some(CompletionUsage {
                requests: 1,
                input_tokens: 23,
                output_tokens: 4,
                ..Default::default()
            }),
        }))
    }
    async fn complete_stream(
        &self,
        _: CompletionRequest,
    ) -> Result<CompletionEventStream, ProviderError> {
        let attempt = self.calls.fetch_add(1, Ordering::SeqCst);
        let provider_id = ProviderId::new(self.id());
        let model = self.model.clone();
        let mut events = Vec::new();
        if attempt > 0 && !self.always_empty {
            events.push(Ok(CompletionStreamEvent::TextDelta {
                provider_id: provider_id.clone(),
                model: model.clone(),
                delta: "answer".into(),
            }));
        }
        events.push(Ok(CompletionStreamEvent::Completed {
            provider_id,
            model,
            finish_reason: Some(agena_provider::CompletionFinishReason::Stop),
            usage: Some(CompletionUsage {
                input_tokens: 10 + attempt as u64,
                output_tokens: if attempt > 0 && !self.always_empty {
                    20
                } else {
                    0
                },
                ..Default::default()
            }),
            provider_metadata: None,
            end_turn: None,
        }));
        Ok(Box::pin(futures_util::stream::iter(events)))
    }
}

async fn empty_retry_events(always_empty: bool) -> (usize, Vec<CompletionStreamEvent>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut registry = ProviderRegistry::new();
    registry.retry_policy = super::super::RequestRetryPolicy {
        max_retries: 1,
        base_delay: Duration::ZERO,
        max_delay: Duration::ZERO,
    };
    registry.register(EmptyThenText {
        model: ModelId::new("test-model"),
        calls: calls.clone(),
        always_empty,
    });
    let request = serde_json::from_value(serde_json::json!({
        "model": "test-model", "messages": [],
    }))
    .unwrap();
    let mut stream = registry
        .complete_stream(&ModelRef::new("performance-fixture", "test-model"), request)
        .await
        .unwrap();
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event.unwrap());
    }
    (calls.load(Ordering::SeqCst), events)
}

#[tokio::test]
async fn performance_empty_retry_preserves_every_sample_and_only_one_terminal_event() {
    let (calls, events) = empty_retry_events(false).await;
    assert_eq!(calls, 2);
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event, CompletionStreamEvent::ProviderRetry { .. }))
            .count(),
        1
    );
    let terminal = events
        .iter()
        .filter_map(|event| match event {
            CompletionStreamEvent::Completed { usage, .. } => usage.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(terminal.len(), 1);
    let usage = terminal[0];
    assert_eq!(usage.requests, 2);
    assert_eq!(usage.input_tokens, 21);
    assert_eq!(usage.output_tokens, 20);
    assert_eq!(usage.performance.len(), 2);
    assert_eq!(
        usage
            .performance
            .iter()
            .filter(|sample| sample.generated_tokens == Some(0))
            .count(),
        1
    );
    assert_eq!(
        usage.request_performance.unwrap().generated_tokens,
        Some(20)
    );
    assert_eq!(usage.request_usage_snapshot().input_tokens, 11);
    assert_eq!(usage.request_usage_snapshot().output_tokens, 20);
}

#[tokio::test]
async fn performance_exhausted_empty_retry_keeps_usage_and_final_terminal_event() {
    let (calls, events) = empty_retry_events(true).await;
    assert_eq!(calls, 2);
    let terminal = events
        .iter()
        .filter_map(|event| match event {
            CompletionStreamEvent::Completed { usage, .. } => usage.as_ref(),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(terminal.len(), 1);
    let usage = terminal[0];
    assert_eq!(usage.requests, 2);
    assert_eq!(usage.input_tokens, 21);
    assert_eq!(usage.performance.len(), 2);
    assert_eq!(usage.request_performance.unwrap().generated_tokens, Some(0));
    assert_eq!(usage.request_usage_snapshot().input_tokens, 11);
}

#[tokio::test]
async fn native_compaction_reports_response_usage_and_retains_observer_charge() {
    use crate::provider::{ModelAttemptObserver, ModelAttemptOutcome, ModelAttemptStatus};
    use agena_provider::BillableUsageItem;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Observer(Mutex<Vec<(u32, Option<String>)>>);

    #[async_trait::async_trait]
    impl ModelAttemptObserver for Observer {
        async fn before(&self, _: &ModelRef, attempt: u32) {
            self.0.lock().unwrap().push((attempt, None));
        }

        async fn after(
            &self,
            _: &ModelRef,
            attempt: u32,
            outcome: ModelAttemptOutcome,
        ) -> Option<BillableUsageItem> {
            assert!(matches!(outcome.status, ModelAttemptStatus::Completed));
            assert_eq!(outcome.usage.unwrap().input_tokens, 23);
            self.0.lock().unwrap().push((attempt, outcome.response_id));
            Some(BillableUsageItem {
                kind: "fixture_units".into(),
                quantity: 1.25,
                ..Default::default()
            })
        }
    }

    let mut registry = ProviderRegistry::new();
    registry.register(EmptyThenText {
        model: ModelId::new("test-model"),
        calls: Arc::new(AtomicUsize::new(0)),
        always_empty: false,
    });
    let observer = Arc::new(Observer::default());
    let hooks = ModelAttemptHooks::new(observer.clone());
    let request: CompletionRequest =
        serde_json::from_value(serde_json::json!({"model":"test-model","messages":[]})).unwrap();
    let unsupported = registry
        .compact_conversation_observed(
            &ModelRef::new("performance-fixture", "portable-model"),
            request.clone(),
            Some(&hooks),
        )
        .await
        .unwrap();
    assert!(unsupported.is_none());
    assert!(observer.0.lock().unwrap().is_empty());

    let compact = registry
        .compact_conversation_observed(
            &ModelRef::new("performance-fixture", "test-model"),
            request,
            Some(&hooks),
        )
        .await
        .unwrap()
        .unwrap();
    assert!(compact.has_checkpoint());
    let ProviderCompactionOutput::OpenAiResponses { usage, .. } = compact;
    let usage = usage.unwrap();
    assert_eq!(usage.input_tokens, 23);
    assert_eq!(usage.billable_items[0].quantity, 1.25);
    assert_eq!(
        observer.0.lock().unwrap().as_slice(),
        [(1, None), (1, Some("cmp_fixture".into()))]
    );
}
