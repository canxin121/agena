use agena_plugin_host::sdk::{ModelSwitchPricing, SessionStartCacheContext};
use agena_provider::CompletionUsage;
use serde::Deserialize;

use super::*;

pub(super) enum SessionStartPreparation {
    Ready(Option<String>),
    QueuePaused,
    Superseded,
}

impl SessionManager {
    pub(super) fn remember_session_start(
        &self,
        session: SessionId,
        stop_reason: Option<String>,
    ) -> Option<String> {
        let reason =
            stop_reason.map(|reason| agena_failure::diagnostic::scrubbed_preserve(&reason, 512));
        let mut starts = self.plugin_session_starts.lock();
        let pending = starts.entry(session).or_default();
        pending.extend(reason);
        pending.front().cloned()
    }

    pub(super) async fn session_start_cache_context(
        &self,
        session: SessionId,
        state: &ExecutionState,
        target: Option<&ModelRef>,
    ) -> Option<SessionStartCacheContext> {
        let facts = match read_with_busy_retry(session, "session_start_cache", || {
            self.kernel().reader.session_resume_facts(session)
        })
        .await
        {
            Ok(facts) => facts,
            Err(error) => {
                tracing::debug!(session_id = session.get(), %error, "session start cache data is unavailable");
                return None;
            }
        };
        let facts = facts.as_ref()?;
        let elapsed_ms = Utc::now()
            .timestamp_millis()
            .saturating_sub(facts.finished_at_ms)
            .max(0) as u64;
        let usage = facts
            .usage
            .as_ref()
            .and_then(|usage| CompletionUsage::deserialize(usage).ok());
        let cache_ttl = super::model_switch::observed_cache_ttl(usage.as_ref());
        let context_tokens = if facts.context_rewritten {
            facts.compacted_context_tokens
        } else {
            usage
                .as_ref()
                .filter(|usage| usage.has_own_usage())
                .map(crate::session::usage::context_tokens)
        };
        let estimated_cache_write_usd = target.zip(context_tokens).and_then(|(target, tokens)| {
            let (cost, pricing) =
                super::model_switch::model_switch_price(state, target, tokens, cache_ttl);
            (!matches!(pricing, ModelSwitchPricing::Default)).then_some(cost)
        });
        Some(SessionStartCacheContext {
            seconds_since_last_response: elapsed_ms / 1_000,
            context_tokens,
            prompt_cache_likely_expired: facts.context_rewritten
                || target.is_some_and(|target| {
                    facts
                        .model
                        .as_ref()
                        .is_some_and(|previous| previous != target)
                })
                || Duration::from_millis(elapsed_ms) >= cache_ttl.duration(),
            estimated_cache_write_usd,
        })
    }
}
