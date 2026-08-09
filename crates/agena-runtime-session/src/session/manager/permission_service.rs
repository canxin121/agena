//! Session-side permission orchestration: versioned rule snapshots, denial
//! budgets, and the automatic-approval classifier client.
//!
//! Decision *logic* lives in `agena-permission`; this module is the thin
//! adapter that supplies the snapshot, the budget, and the provider call.

use std::collections::HashMap;
use std::sync::Arc;

use agena_domain::ApprovalModelSelection;
use agena_permission::DenialBudget;

use super::{
    AppError, ModelRef, Session, SessionManager, SessionManagerState, SessionRunOptions,
};
use crate::session::prompt_window;
use agena_domain::Role;

/// Versioned per-session snapshot of persisted permission rules, grouped by
/// action key. Loaded once with a single query; invalidated on writes.
#[derive(Debug, Clone, Default)]
pub(crate) struct RuleSnapshot {
    pub(crate) rules: HashMap<String, Vec<agena_permission::RuleEntry>>,
}

impl RuleSnapshot {
    pub(crate) fn rules_for(&self, action_key: &str) -> &[agena_permission::RuleEntry] {
        self.rules.get(action_key).map(Vec::as_slice).unwrap_or(&[])
    }
}

pub(crate) fn rule_entry_from_persisted(
    rule: &agena_storage::PersistedPermissionRule,
) -> agena_permission::RuleEntry {
    agena_permission::RuleEntry {
        id: rule.id,
        revision_ms: rule.updated_at_ms,
        scope: rule.scope,
        source: rule.source.clone(),
        reason: rule.reason.clone(),
        operator: rule.operator.clone(),
        mode: rule.mode,
    }
}

/// Group a flat snapshot query result into per-action rule chains, keeping
/// the repository's global → workspace → session ordering.
pub(crate) fn group_snapshot_rules(
    rules: &[agena_storage::PersistedPermissionRule],
) -> HashMap<String, Vec<agena_permission::RuleEntry>> {
    let mut grouped: HashMap<String, Vec<agena_permission::RuleEntry>> = HashMap::new();
    for rule in rules {
        grouped
            .entry(rule.action_key.clone())
            .or_default()
            .push(rule_entry_from_persisted(rule));
    }
    grouped
}

impl SessionManager {
    pub(in crate::session::manager) async fn rule_snapshot(
        &self,
        state: &SessionManagerState,
        session_id: Option<i64>,
    ) -> Result<Arc<RuleSnapshot>, AppError> {
        if let Ok(snapshots) = state.rule_snapshots.read()
            && let Some(snapshot) = snapshots.get(&session_id)
        {
            return Ok(Arc::clone(snapshot));
        }
        let workspace_id = self.current_workspace_id().await?;
        let stored = self
            .permission_rules
            .resolve_snapshot(session_id, Some(workspace_id))
            .await
            .map_err(|error| {
                AppError::Internal(format!("resolve permission rule snapshot: {error}"))
            })?;
        let snapshot = Arc::new(RuleSnapshot {
            rules: group_snapshot_rules(stored.as_slice()),
        });
        if let Ok(mut snapshots) = state.rule_snapshots.write() {
            snapshots.insert(session_id, Arc::clone(&snapshot));
        }
        Ok(snapshot)
    }

    pub(crate) fn invalidate_rule_snapshots(&self) {
        if let Ok(mut snapshots) = self.execution_state().rule_snapshots.write() {
            snapshots.clear();
        }
    }

    pub(crate) fn auto_budget(&self, session_id: Option<i64>) -> DenialBudget {
        self.execution_state()
            .auto_approval
            .lock()
            .ok()
            .and_then(|budgets| budgets.get(&session_id).cloned())
            .unwrap_or_default()
    }

    pub(crate) fn record_auto_decision(&self, session_id: Option<i64>, allowed: bool) {
        if let Ok(mut budgets) = self.execution_state().auto_approval.lock() {
            budgets
                .entry(session_id)
                .or_default()
                .record_decision(allowed);
        }
    }

    /// Resolve the approval model: the configured `approval_model`, falling
    /// back to the session model, then the default model. `None` only when
    /// no model exists anywhere.
    pub(in crate::session::manager) fn resolve_approval_model(
        &self,
        session: Option<&Session>,
        state: &SessionManagerState,
    ) -> Result<Option<(ModelRef, Option<ApprovalModelSelection>)>, AppError> {
        let approval_model = match session {
            Some(session) => session
                .runtime
                .execution
                .effective_permission
                .approval_model
                .clone(),
            None => {
                let mut permission = state
                    .shared_permission
                    .read()
                    .map(|value| value.clone())
                    .unwrap_or_else(|_| state.config.permission.clone());
                permission.merge_from(super::replies::managed_project_state_permission(
                    state.tool_executor.workspace_root(),
                ));
                permission.approval_model
            }
        };
        match approval_model {
            Some(selection) => self
                .resolve_approval_model_selection(&selection, state)
                .map(|model| Some((model, Some(selection)))),
            None => match session {
                Some(session) => self
                    .model_from_session_or_default(session, state)
                    .map(|model| Some((model, None))),
                None => self
                    .default_model_from_config(state)
                    .map(|model| model.map(|model| (model, None))),
            },
        }
    }

    fn resolve_approval_model_selection(
        &self,
        selection: &ApprovalModelSelection,
        state: &SessionManagerState,
    ) -> Result<ModelRef, AppError> {
        let model = selection.model_ref().map_err(|error| {
            AppError::Internal(format!(
                "invalid automatic approval model reference: {error}"
            ))
        })?;
        state
            .processor
            .provider_registry()
            .resolve_model_selection(
                model.provider_id.as_ref(),
                model.adapter_id.as_ref().map(|adapter| adapter.as_ref()),
                Some(model.model_id.as_ref()),
            )
            .map_err(|error| {
                AppError::Internal(format!(
                    "automatic approval model is unavailable in the provider registry: {error}"
                ))
            })
    }

    /// Classify a batch of auto-approval candidates with one shared context:
    /// one model resolution, one variant resolution, one transcript. Returns
    /// per-candidate `Ok(allowed)`; `Err(failure)` means the candidate fell
    /// back to interactive `ask` (fail closed) because automatic approval
    /// could not resolve. The failure reason is surfaced to the user.
    pub(in crate::session::manager) async fn classify_auto_candidates(
        &self,
        session: Option<&Session>,
        state: &SessionManagerState,
        session_id: Option<i64>,
        candidates: Vec<agena_permission::ClassifierCandidate>,
    ) -> Vec<Result<bool, agena_permission::ClassifyFailure>> {
        if candidates.is_empty() {
            return Vec::new();
        }
        let Some((model, selection)) = (match self.resolve_approval_model(session, state) {
            Ok(Some(resolved)) => Some(resolved),
            Ok(None) => {
                let reason =
                    "no approval model is configured and no session/default model could be resolved"
                        .to_owned();
                return candidates
                    .into_iter()
                    .map(|_| {
                        Err(agena_permission::ClassifyFailure::ApprovalModelUnavailable(
                            reason.clone(),
                        ))
                    })
                    .collect();
            }
            Err(error) => {
                return candidates
                    .into_iter()
                    .map(|_| {
                        Err(agena_permission::ClassifyFailure::ApprovalModelUnavailable(
                            error.to_string(),
                        ))
                    })
                    .collect();
            }
        }) else {
            return Vec::new();
        };

        let mut options = SessionRunOptions {
            model: model.clone(),
            thinking_mode: selection
                .as_ref()
                .and_then(|selection| selection.thinking_mode.clone()),
            speed_mode: selection
                .as_ref()
                .and_then(|selection| selection.speed_mode.clone()),
            verbosity: selection
                .as_ref()
                .and_then(|selection| selection.verbosity.clone()),
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: Some(0.0),
            max_output_tokens: Some(256),
        };
        if let Some(parallel_tool_calls) = selection
            .as_ref()
            .and_then(|selection| selection.parallel_tool_calls)
        {
            options
                .request_override
                .set_parallel_tool_calls(Some(parallel_tool_calls));
        }
        if let Err(error) = self.apply_model_mode_requests(&mut options) {
            return candidates
                .into_iter()
                .map(|_| {
                    Err(agena_permission::ClassifyFailure::ModeUnavailable(
                        error.to_string(),
                    ))
                })
                .collect();
        }

        let transcript_budget_chars = state
            .processor
            .model_metadata(&model)
            .ok()
            .and_then(|metadata| metadata.limits.context_window_tokens)
            // The window is measured in tokens but the projection budget is
            // characters; cap it so a large model window (1M tokens) cannot
            // balloon the classifier transcript to megabytes per request.
            .map(|tokens| {
                (tokens as usize / 4)
                    .clamp(8_000, agena_permission::AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS)
            })
            .unwrap_or(agena_permission::AUTO_APPROVAL_TRANSCRIPT_FALLBACK_CHARS);
        let transcript = session.map(|session| {
            // v2: the transcript is the parts projection (the store is the
            // single durable source; the active-window part count doubles as
            // the cache key).
            let parts = session.active_window_parts();
            let part_count = parts.len();
            let cached = state
                .auto_projection
                .lock()
                .ok()
                .and_then(|cache| cache.get(&session_id).cloned());
            match cached {
                Some((len, text)) if len == part_count => text,
                _ => {
                    let text =
                        prompt_window::project_transcript(parts, transcript_budget_chars);
                    if let Ok(mut cache) = state.auto_projection.lock() {
                        cache.insert(session_id, (part_count, text.clone()));
                    }
                    text
                }
            }
        });
        let recent_decisions = self.auto_budget(session_id).recent_decision_labels();
        let context_message = agena_permission::build_classifier_context_message(
            transcript.as_deref(),
            &recent_decisions,
        );

        let mut futures = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let model_ref = model.clone();
            let context = context_message.clone();
            let thinking = options.thinking.clone();
            let verbosity = options.verbosity.clone();
            let request_override = options.request_override.clone();
            futures.push(async move {
                let action = serde_json::to_string(&candidate.action)
                    .unwrap_or_else(|_| r#"{"action":"unserializable"}"#.to_owned());
                let action_message = agena_permission::build_classifier_action_message(
                    &action,
                    &candidate.policy_reason,
                );
                let request = agena_provider::CompletionRequest {
                    model: model_ref.model_id.clone(),
                    system: Some(agena_permission::AUTO_APPROVAL_SYSTEM_PROMPT.to_owned()),
                    turns: {
                        let mut turns = Vec::with_capacity(2);
                        if let Some(context) = &context {
                            turns.push(agena_provider::CompletionInputRun {
                                role: Role::User,
                                parts: vec![agena_provider::CompletionInputPart::Text {
                                    text: context.clone(),
                                }],
                                provider_state: Default::default(),
                            });
                        }
                        turns.push(agena_provider::CompletionInputRun {
                            role: Role::User,
                            parts: vec![agena_provider::CompletionInputPart::Text {
                                text: action_message,
                            }],
                            provider_state: Default::default(),
                        });
                        turns
                    },
                    tool_api_functions: Vec::new(),
                    provider_native_tools: Default::default(),
                    disable_tools: true,
                    temperature: Some(0.0),
                    max_output_tokens: Some(256),
                    prompt_cache_key: Some(format!("agena:auto:{}", model_ref.model_id)),
                    previous_response_id: None,
                    prompt_window_generation: None,
                    provider_compaction: None,
                    stop_sequences: Vec::new(),
                    top_p: None,
                    top_k: None,
                    seed: None,
                    thinking,
                    verbosity,
                    response_format: Some(agena_provider::ResponseFormat::JsonSchema {
                        name: "permission_verdict".to_owned(),
                        schema: agena_permission::classifier_json_schema(),
                        strict: true,
                    }),
                    responses_api_metadata: None,
                    request_override,
                };
                match tokio::time::timeout(
                    agena_permission::AUTO_APPROVAL_CLASSIFY_TIMEOUT,
                    state
                        .processor
                        .provider_registry()
                        .complete(&model_ref, request),
                )
                .await
                {
                    Ok(Ok(response)) => {
                        if response.text.trim().is_empty() {
                            return Err(agena_permission::ClassifyFailure::EmptyResponse);
                        }
                        match agena_permission::parse_classifier_verdict(response.text.as_str()) {
                            Some(allowed) => {
                                self.record_auto_decision(session_id, allowed);
                                Ok(allowed)
                            }
                            None => Err(agena_permission::ClassifyFailure::UnparseableVerdict(
                                truncate_classifier_text(response.text.as_str()),
                            )),
                        }
                    }
                    Ok(Err(error)) => Err(agena_permission::ClassifyFailure::Provider(
                        error.to_string(),
                    )),
                    Err(_elapsed) => Err(agena_permission::ClassifyFailure::Timeout),
                }
            });
        }
        futures_util::future::join_all(futures).await
    }
}

/// Bound the classifier text echoed into a fallback `Ask` reason so a
/// pathological provider response cannot balloon the interactive prompt.
fn truncate_classifier_text(text: &str) -> String {
    const MAX: usize = 400;
    if text.chars().count() <= MAX {
        return text.to_owned();
    }
    let mut out = text.chars().take(MAX).collect::<String>();
    out.push('…');
    out
}
