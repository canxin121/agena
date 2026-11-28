use super::*;
use crate::provider::{
    ModelAttemptHooks, ModelAttemptObserver, ModelAttemptOutcome, ModelAttemptStatus,
};
use agena_plugin_host::sdk::{
    HookSubscription, ImportedModelAttemptInput, ImportedModelAttemptOutcome,
    ImportedModelAttemptStatus, ImportedModelTokenUsage, ImportedSamplingKind,
};
use agena_provider::{BillableUsageItem, CompletionUsage};

struct ImportedSamplingObserver {
    executor: ToolExecutor,
    session_id: i64,
    root_session_id: i64,
    turn_id: String,
    request_id: String,
    kind: ImportedSamplingKind,
    agent_type: Option<String>,
}

impl SessionManager {
    pub(super) async fn imported_model_attempt_hooks(
        &self,
        control: &RunControl,
        executor: &ToolExecutor,
        context: &crate::session::model::SessionExecutionContext,
        round: &RoundHandle,
        kind: ImportedSamplingKind,
    ) -> Option<ModelAttemptHooks> {
        let host = executor.plugin_manager();
        if !host.has_subscription(HookSubscription::IMPORTED_MODEL_ATTEMPT_BEFORE)
            && !host.has_subscription(HookSubscription::IMPORTED_MODEL_ATTEMPT_AFTER)
        {
            return None;
        }
        let session_id = control.session.get();
        let (root_session_id, agent_type) = if context.relation_kind.is_subagent() {
            let root = match self.kernel().reader.summary(control.session).await {
                Ok(summary) => summary.root_session_id.get(),
                Err(error) => {
                    tracing::warn!(session_id, %error, "could not resolve sampling hook thread ancestry");
                    session_id
                }
            };
            let profile = control
                .state
                .config
                .effective_profile_name(context.profile.as_deref(), context.relation_kind);
            (root, Some(host.imported_subagent_type(profile)))
        } else {
            (session_id, None)
        };
        Some(ModelAttemptHooks::new(Arc::new(ImportedSamplingObserver {
            executor: executor.clone(),
            session_id,
            root_session_id,
            turn_id: control.execution.to_string(),
            request_id: round.id().get().to_string(),
            kind,
            agent_type,
        })))
    }
}

impl ImportedSamplingObserver {
    fn input(&self, model: &ModelRef, number: u32) -> ImportedModelAttemptInput {
        ImportedModelAttemptInput {
            session_id: self.session_id,
            root_session_id: self.root_session_id,
            turn_id: self.turn_id.clone(),
            request_id: self.request_id.clone(),
            attempt_number: number,
            kind: self.kind,
            model: model.model_id.to_string(),
            agent_type: self.agent_type.clone(),
            outcome: None,
        }
    }
}

#[async_trait::async_trait]
impl ModelAttemptObserver for ImportedSamplingObserver {
    async fn before(&self, model: &ModelRef, number: u32) {
        self.executor
            .plugin_manager()
            .dispatch_imported_model_attempt(self.input(model, number), |key| {
                self.executor.plugin_is_enabled(key)
            })
            .await;
    }

    async fn after(
        &self,
        model: &ModelRef,
        number: u32,
        outcome: ModelAttemptOutcome,
    ) -> Option<BillableUsageItem> {
        let mut input = self.input(model, number);
        input.outcome = Some(ImportedModelAttemptOutcome {
            status: match outcome.status {
                ModelAttemptStatus::Completed => ImportedModelAttemptStatus::Completed,
                ModelAttemptStatus::Failed => ImportedModelAttemptStatus::Failed,
                ModelAttemptStatus::Cancelled => ImportedModelAttemptStatus::Cancelled,
            },
            duration_ms: outcome.duration_ms,
            error: outcome.error,
            response_id: outcome.response_id,
            token_usage: outcome.usage.as_ref().map(token_usage),
            usage: outcome
                .usage
                .and_then(|usage| serde_json::to_value(usage).ok()),
        });
        let charge = self
            .executor
            .plugin_manager()
            .dispatch_imported_model_attempt(input, |key| self.executor.plugin_is_enabled(key))
            .await;
        (charge.extra_codex_rollout_budget_units > 0.0).then(|| BillableUsageItem {
            kind: "extra_codex_rollout_budget_units".into(),
            quantity: charge.extra_codex_rollout_budget_units,
            unit: "codex_rollout_budget_unit".into(),
            pricing_source: Some("codex_post_model_request".into()),
            ..Default::default()
        })
    }
}

fn token_usage(usage: &CompletionUsage) -> ImportedModelTokenUsage {
    ImportedModelTokenUsage {
        input_tokens: usage.own_cache_input_tokens(),
        output_tokens: usage.output_tokens.saturating_add(usage.reasoning_tokens),
        cached_input_tokens: usage.cache_read_tokens,
        cache_write_input_tokens: usage.cache_write_tokens,
        reasoning_output_tokens: usage.reasoning_tokens,
        total_tokens: usage.own_total_tokens(),
    }
}
