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
