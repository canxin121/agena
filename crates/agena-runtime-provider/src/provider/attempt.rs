use std::{
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
    },
    time::Instant,
};

use agena_domain::ModelRef;
use agena_provider::{BillableUsageItem, CompletionUsage};
use serde_json::Value;

use crate::ProviderError;

#[derive(Clone, Copy)]
pub enum ModelAttemptStatus {
    Completed,
    Failed,
    Cancelled,
}

pub struct ModelAttemptOutcome {
    pub status: ModelAttemptStatus,
    pub duration_ms: u64,
    pub error: Option<String>,
    pub response_id: Option<String>,
    pub usage: Option<CompletionUsage>,
}

#[async_trait::async_trait]
pub trait ModelAttemptObserver: Send + Sync {
    async fn before(&self, model: &ModelRef, attempt_number: u32);

    async fn after(
        &self,
        model: &ModelRef,
        attempt_number: u32,
        outcome: ModelAttemptOutcome,
    ) -> Option<BillableUsageItem>;
}

/// Shared only by the physical requests that implement one generation.
#[derive(Clone)]
pub struct ModelAttemptHooks {
    observer: Arc<dyn ModelAttemptObserver>,
    next_attempt: Arc<AtomicU32>,
}

impl ModelAttemptHooks {
    pub fn new(observer: Arc<dyn ModelAttemptObserver>) -> Self {
        Self {
            observer,
            next_attempt: Arc::new(AtomicU32::new(1)),
        }
    }
}

pub(super) struct ModelAttempt(Option<ActiveAttempt>);

struct ActiveAttempt {
    observer: Arc<dyn ModelAttemptObserver>,
    model: ModelRef,
    number: u32,
    started: Instant,
}

impl ModelAttempt {
    pub(super) async fn begin(hooks: Option<&ModelAttemptHooks>, model: &ModelRef) -> Self {
        let Some(hooks) = hooks else {
            return Self(None);
        };
        let number = hooks.next_attempt.fetch_add(1, Ordering::Relaxed);
        let mut attempt = Self(Some(ActiveAttempt {
            observer: hooks.observer.clone(),
            model: model.clone(),
            number,
            started: Instant::now(),
        }));
        hooks.observer.before(model, number).await;
        attempt.0.as_mut().expect("active provider attempt").started = Instant::now();
        attempt
    }

    pub(super) async fn complete(
        &mut self,
        usage: &mut Option<CompletionUsage>,
        metadata: Option<&Value>,
    ) {
        let Some(attempt) = self.0.take() else { return };
        let (response_id, incomplete) = match metadata {
            Some(metadata) => (
                metadata["response_id"].as_str(),
                metadata["response_incomplete_reason"].as_str(),
            ),
            None => (None, None),
        };
        let status = if incomplete.is_some() {
            ModelAttemptStatus::Failed
        } else {
            ModelAttemptStatus::Completed
        };
        attempt
            .finish(
                status,
                incomplete
                    .map(|reason| safe_error(&format!("provider response incomplete: {reason}"))),
                response_id,
                usage,
            )
            .await;
    }

    pub(super) async fn fail(&mut self, error: &ProviderError) {
        let Some(attempt) = self.0.take() else { return };
        // Transport errors may include whole HTTP bodies or URLs with
        // credentials. Hooks receive the useful classification instead.
        let error = match error {
            ProviderError::HttpStatus {
                provider,
                status,
                kind,
                ..
            } => {
                format!("{provider} returned HTTP {} ({kind:?})", status.as_u16())
            }
            ProviderError::Http(error) if error.is_timeout() => "provider request timed out".into(),
            ProviderError::Http(error) if error.is_connect() => {
                "could not connect to the provider".into()
            }
            ProviderError::Http(_) => "provider HTTP request failed".into(),
            _ => safe_error(&error.to_string()),
        };
        attempt
            .finish(ModelAttemptStatus::Failed, Some(error), None, &mut None)
            .await;
    }
}

fn safe_error(error: &str) -> String {
    agena_failure::diagnostic::user_message_with_context(error, 2048)
}

impl Drop for ModelAttempt {
    fn drop(&mut self) {
        if let (Some(attempt), Ok(runtime)) = (self.0.take(), tokio::runtime::Handle::try_current())
        {
            runtime.spawn(async move {
                attempt
                    .finish(ModelAttemptStatus::Cancelled, None, None, &mut None)
                    .await;
            });
        }
    }
}

impl ActiveAttempt {
    async fn finish(
        self,
        status: ModelAttemptStatus,
        error: Option<String>,
        response_id: Option<&str>,
        usage: &mut Option<CompletionUsage>,
    ) {
        let billable = matches!(status, ModelAttemptStatus::Completed)
            || (matches!(status, ModelAttemptStatus::Failed) && usage.is_some());
        let outcome = ModelAttemptOutcome {
            status,
            duration_ms: self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
            error,
            response_id: response_id.map(ToOwned::to_owned),
            usage: usage.clone(),
        };
        if let Some(item) = self.observer.after(&self.model, self.number, outcome).await
            && billable
        {
            usage
                .get_or_insert_with(|| CompletionUsage {
                    cost_estimate_incomplete: true,
                    ..Default::default()
                })
                .billable_items
                .push(item);
        }
    }
}
