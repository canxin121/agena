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
    recover_mutex, recover_read, recover_write,
};
use crate::session::prompt_window;
use agena_domain::Role;

/// Output budget for one classifier verdict when the approval model does not
/// advertise its own output ceiling.
///
/// The verdict itself is tiny (a tool call with one rule name and one
/// sentence), but the approval model is allowed to reason first and that
/// reasoning is billed against this same output budget. 256 and 2 048 both
/// truncated the thinking before the verdict existed on models that reason
/// before they answer, and a truncated attempt returns nothing at all - which is
/// the empty response that fails the classification closed into an interactive
/// ask. The budget therefore leaves real thinking headroom rather than just
/// enough room for the verdict.
const AUTO_APPROVAL_DEFAULT_OUTPUT_TOKENS: u32 = 16_384;

/// Upper bound for the derived budget: one verdict must not turn into a minute
/// of generation on a model that advertises a very large output window. The
/// classify deadline bounds the wait either way.
const AUTO_APPROVAL_MAX_OUTPUT_TOKENS: u32 = 32_768;

/// Verdict attempts per candidate: the first request, then one retry after
/// [`AUTO_APPROVAL_VERDICT_REMINDER`]. Two is deliberate — a third attempt
/// buys little and delays the fail-closed `Ask` the user is waiting on.
const AUTO_APPROVAL_ATTEMPTS: usize = 2;

/// Reminder appended as a user turn when an attempt produced no verdict at
/// all. It names the two tools rather than restating the whole policy, and
/// mirrors the wording of the runtime's other retry notes
/// (`append_empty_response_nudge`, `DOOM_LOOP_RECOVERY_PROMPT`).
const AUTO_APPROVAL_VERDICT_REMINDER: &str = "Trusted Agena runtime note: the previous response did not submit a verdict. Submit your verdict now as a tool call: call approve_action to allow the action, or call block_action with the exact name of the BLOCK rule that matched to block it. Do not answer with prose.";

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

/// Group current tool policies into global → workspace → session chains.
pub(crate) fn group_snapshot_rules(
    rules: &[agena_storage::PersistedPermissionRule],
) -> HashMap<String, Vec<agena_permission::RuleEntry>> {
    let mut grouped: HashMap<String, Vec<agena_permission::RuleEntry>> = HashMap::new();
    for rule in rules {
        let key = serde_json::from_str::<agena_domain::PermissionAction>(&rule.action_key)
            .ok()
            .and_then(|action| super::permission_action_key(&action).ok())
            .unwrap_or_else(|| rule.action_key.clone());
        grouped
            .entry(key)
            .or_default()
            .push(rule_entry_from_persisted(rule));
    }
    // Alias spellings can produce several persisted rows for one subject.
    // Keep the latest within each scope.
    for chain in grouped.values_mut() {
        chain.sort_by_key(|entry| {
            (
                scope_rank(entry.scope),
                std::cmp::Reverse(entry.revision_ms),
                std::cmp::Reverse(entry.id),
            )
        });
        chain.dedup_by_key(|entry| entry.scope);
    }
    for chain in grouped.values_mut() {
        chain.sort_by_key(|entry| scope_rank(entry.scope));
    }
    grouped
}

fn scope_rank(scope: agena_domain::PermissionScope) -> u8 {
    match scope {
        agena_domain::PermissionScope::Global => 0,
        agena_domain::PermissionScope::Workspace => 1,
        agena_domain::PermissionScope::Session => 2,
    }
}

impl SessionManager {
    pub(in crate::session::manager) async fn rule_snapshot(
        &self,
        state: &SessionManagerState,
        session_id: Option<i64>,
    ) -> Result<Arc<RuleSnapshot>, AppError> {
        if let Some(snapshot) = recover_read(
            state.rule_snapshots.as_ref(),
            "read cached permission-rule snapshot",
        )
        .get(&session_id)
        .cloned()
        {
            return Ok(snapshot);
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
        recover_write(
            state.rule_snapshots.as_ref(),
            "cache resolved permission-rule snapshot",
        )
        .insert(session_id, Arc::clone(&snapshot));
        Ok(snapshot)
    }

    pub(crate) fn invalidate_rule_snapshots(&self) {
        let state = self.execution_state();
        recover_write(
            state.rule_snapshots.as_ref(),
            "invalidate permission-rule snapshots",
        )
        .clear();
    }

    pub(crate) fn auto_budget(&self, session_id: Option<i64>) -> DenialBudget {
        let state = self.execution_state();
        recover_mutex(
            state.auto_approval.as_ref(),
            "read automatic-approval denial budget",
        )
        .get(&session_id)
        .cloned()
        .unwrap_or_default()
    }

    pub(crate) fn record_auto_decision(&self, session_id: Option<i64>, allowed: bool) {
        let state = self.execution_state();
        recover_mutex(
            state.auto_approval.as_ref(),
            "record automatic-approval decision",
        )
        .entry(session_id)
        .or_default()
        .record_decision(allowed);
    }

    /// Resolve the approval model: the configured `approval_model`, falling
    /// back to the session model. `None` only when no model exists anywhere.
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
            None => recover_read(
                state.shared_permission.as_ref(),
                "read shared automatic-approval permission",
            )
            .approval_model
            .clone(),
        };
        match approval_model {
            Some(selection) => self
                .resolve_approval_model_selection(&selection, state)
                .map(|model| Some((model, Some(selection)))),
            None => match session {
                Some(session) => self
                    .model_from_session_or_error(session, state)
                    .map(|model| Some((model, None))),
                None => Ok(None),
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
            .provider_registry
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
    /// the candidates with their [`agena_permission::ClassifierVerdict`] filled
    /// in; a candidate whose verdict is `None` fell back to interactive `ask`
    /// (fail closed) because automatic approval could not resolve, and carries
    /// its [`agena_permission::ClassifyFailure`] for the user.
    pub(in crate::session::manager) async fn classify_auto_candidates(
        &self,
        session: Option<&Session>,
        state: &SessionManagerState,
        session_id: Option<i64>,
        candidates: Vec<agena_permission::ClassifierCandidate>,
    ) -> Vec<agena_permission::ClassifiedCandidate> {
        if candidates.is_empty() {
            return Vec::new();
        }
        let fail_all = |candidates: Vec<agena_permission::ClassifierCandidate>,
                        failure: agena_permission::ClassifyFailure| {
            candidates
                .into_iter()
                .map(|candidate| agena_permission::ClassifiedCandidate {
                    candidate,
                    verdict: None,
                    failure: Some(failure.clone()),
                })
                .collect::<Vec<_>>()
        };
        let Some((model, selection)) = (match self.resolve_approval_model(session, state) {
            Ok(Some(resolved)) => Some(resolved),
            Ok(None) => {
                let reason =
                    "no approval model is configured and no session model could be resolved"
                        .to_owned();
                return fail_all(
                    candidates,
                    agena_permission::ClassifyFailure::ApprovalModelUnavailable(reason),
                );
            }
            Err(error) => {
                return fail_all(
                    candidates,
                    agena_permission::ClassifyFailure::ApprovalModelUnavailable(error.to_string()),
                );
            }
        }) else {
            return Vec::new();
        };

        // The model reasons before it answers, and that reasoning shares this
        // output budget with the verdict. Follow the approval model's own output
        // ceiling when it advertises one, so a thinking model is never cut off
        // before it can submit the verdict.
        let verdict_output_tokens = state
            .provider_registry
            .model_metadata(&model)
            .ok()
            .and_then(|metadata| metadata.limits.max_output_tokens)
            .map_or(AUTO_APPROVAL_DEFAULT_OUTPUT_TOKENS, |tokens| {
                tokens.min(AUTO_APPROVAL_MAX_OUTPUT_TOKENS)
            });

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
            max_output_tokens: Some(verdict_output_tokens),
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
            return fail_all(
                candidates,
                agena_permission::ClassifyFailure::ModeUnavailable(error.to_string()),
            );
        }

        let transcript_budget_chars = state
            .provider_registry
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
            let cached = recover_mutex(
                state.auto_projection.as_ref(),
                "read automatic-approval transcript projection cache",
            )
            .get(&session_id)
            .cloned();
            match cached {
                Some((len, text)) if len == part_count => text,
                _ => {
                    let text = prompt_window::project_transcript(parts, transcript_budget_chars);
                    recover_mutex(
                        state.auto_projection.as_ref(),
                        "cache automatic-approval transcript projection",
                    )
                    .insert(session_id, (part_count, text.clone()));
                    text
                }
            }
        });
        let recent_decisions = self.auto_budget(session_id).recent_decision_labels();
        let context_message = agena_permission::build_classifier_context_message(
            transcript.as_deref(),
            &recent_decisions,
        );

        // A route whose `agena_tools.mode` is not `provider_protocol` strips
        // every declared tool before the request leaves the registry, so the
        // verdict tools never reach that model and it can only answer in text.
        // The text recovery path in the classifier still resolves those
        // verdicts; the log makes the degraded route visible when one is
        // misconfigured.
        if state
            .provider_registry
            .agena_tool_mode(&model)
            .is_ok_and(|mode| mode.is_disabled())
        {
            tracing::debug!(
                model_id = model.model_id.as_ref(),
                "automatic approval route disables Agena tools; verdict tool calls are stripped and only the text recovery path can resolve a verdict"
            );
        }

        let decision_tools = auto_approval_decision_tools();
        // The prompt's path promises are conditional on the compiled policy: a
        // class the sandbox does not approve outright must be judged by the
        // model itself, so the prompt stops telling it not to. The probe asks
        // the same policy the executor enforces, which is what keeps the two
        // from drifting apart.
        let system_prompt = agena_permission::auto_approval_system_prompt(
            &state.tool_executor.path_class_prompt_flags(),
        );
        let mut futures = Vec::with_capacity(candidates.len());
        for candidate in candidates {
            let model_ref = model.clone();
            let context = context_message.clone();
            let thinking = options.thinking.clone();
            let verbosity = options.verbosity.clone();
            let request_override = options.request_override.clone();
            let decision_tools = decision_tools.clone();
            let system_prompt = system_prompt.clone();
            futures.push(async move {
                let requested = candidate.clone();
                let action = serde_json::to_string(&candidate.action)
                    .unwrap_or_else(|_| r#"{"action":"unserializable"}"#.to_owned());
                let action_message = agena_permission::build_classifier_action_message(
                    &action,
                    &candidate.policy_reason,
                );
                let build_request = |reminder: Option<&str>| {
                    let mut turns = Vec::with_capacity(3);
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
                            text: action_message.clone(),
                        }],
                        provider_state: Default::default(),
                    });
                    if let Some(reminder) = reminder {
                        turns.push(agena_provider::CompletionInputRun {
                            role: Role::User,
                            parts: vec![agena_provider::CompletionInputPart::Text {
                                text: reminder.to_owned(),
                            }],
                            provider_state: Default::default(),
                        });
                    }
                    agena_provider::CompletionRequest {
                        model: model_ref.model_id.clone(),
                        system: Some(system_prompt.clone()),
                        turns,
                        tool_api_functions: decision_tools.clone(),
                        provider_native_tools: Default::default(),
                        // The verdict tools must survive the registry
                        // boundary: `disable_tools` (and a disabled route)
                        // would strip every declaration and then reject the
                        // very tool call the model is asked to make.
                        disable_tools: false,
                        temperature: Some(0.0),
                        max_output_tokens: Some(verdict_output_tokens),
                        prompt_cache_key: Some(format!("agena:auto:{}", model_ref.model_id)),
                        previous_response_id: None,
                        prompt_window_generation: None,
                        provider_compaction: None,
                        stop_sequences: Vec::new(),
                        top_p: None,
                        top_k: None,
                        seed: None,
                        // The approval model keeps its own reasoning mode: the
                        // budget above is sized so the thinking and the verdict
                        // both fit.
                        thinking: thinking.clone(),
                        verbosity: verbosity.clone(),
                        // The verdict is a tool call now, not a JSON body, so
                        // there is no response schema to constrain.
                        response_format: None,
                        responses_api_metadata: None,
                        request_override: request_override.clone(),
                    }
                };

                // One deadline for every attempt, so a slow first attempt
                // cannot double the time the user waits before the fail-closed
                // `Ask`.
                let deadline = tokio::time::Instant::now()
                    + agena_permission::AUTO_APPROVAL_CLASSIFY_TIMEOUT;
                let mut verdict = None;
                let mut failure = None;
                let mut last_text = String::new();
                for attempt in 0..AUTO_APPROVAL_ATTEMPTS {
                    let reminder =
                        (attempt > 0).then_some(AUTO_APPROVAL_VERDICT_REMINDER);
                    let request = build_request(reminder);
                    match tokio::time::timeout_at(
                        deadline,
                        state.provider_registry.complete(&model_ref, request),
                    )
                    .await
                    {
                        Ok(Ok(response)) => {
                            last_text = response.text.clone();
                            match classifier_verdict_from_response(&response) {
                                Some(resolved) => {
                                    verdict = Some(resolved);
                                    break;
                                }
                                None if attempt + 1 < AUTO_APPROVAL_ATTEMPTS => {
                                    // No verdict at all: the model answered
                                    // with prose the recovery parser could not
                                    // read. Remind it and ask once more rather
                                    // than interrupting the user.
                                    tracing::debug!(
                                        model_id = model_ref.model_id.as_ref(),
                                        "automatic approval attempt produced no verdict; retrying with a reminder"
                                    );
                                }
                                None => {
                                    failure = Some(if last_text.trim().is_empty() {
                                        agena_permission::ClassifyFailure::EmptyResponse
                                    } else {
                                        agena_permission::ClassifyFailure::UnparseableVerdict(
                                            truncate_classifier_text(last_text.as_str()),
                                        )
                                    });
                                }
                            }
                        }
                        // A provider error or an expired deadline is not a
                        // formatting problem a reminder can fix, and the
                        // registry already retried the transport call.
                        Ok(Err(error)) => {
                            failure = Some(agena_permission::ClassifyFailure::Provider(
                                error.to_string(),
                            ));
                            break;
                        }
                        Err(_elapsed) => {
                            failure = Some(agena_permission::ClassifyFailure::Timeout);
                            break;
                        }
                    }
                }

                let Some(verdict) = verdict else {
                    return agena_permission::ClassifiedCandidate {
                        candidate: requested,
                        verdict: None,
                        failure: Some(failure.unwrap_or(
                            agena_permission::ClassifyFailure::UnparseableVerdict(
                                truncate_classifier_text(last_text.as_str()),
                            ),
                        )),
                    };
                };
                // The denial budget counts classifier *decisions*. An uncited
                // block is not honored as a denial (the caller falls back to
                // confirmation), so recording it as one would trip the
                // "disabled after repeated denials" cutoff on verdicts that
                // never denied anything. One record per candidate, on the
                // final verdict, so a retry cannot double-count a decision.
                let honored_denial = !verdict.allowed && verdict.cites_block_rule();
                self.record_auto_decision(session_id, !honored_denial);
                let failure = (!verdict.cites_block_rule()).then(|| {
                    agena_permission::ClassifyFailure::UncitedBlock(truncate_classifier_text(
                        verdict.reason.as_str(),
                    ))
                });
                agena_permission::ClassifiedCandidate {
                    candidate: requested,
                    verdict: Some(verdict),
                    failure,
                }
            });
        }
        futures_util::future::join_all(futures).await
    }
}

/// The two verdict tools as provider-facing declarations, built from the pure
/// contract in `agena-permission`.
///
/// The binding fields are fixed strings: these tools are never executed by the
/// runtime — the verdict is read off the call and the call is dropped — so they
/// deliberately share one identity that no execution tool can claim.
fn auto_approval_decision_tools() -> Vec<agena_provider::ToolApiDefinition> {
    agena_permission::auto_approval_decision_tools()
        .into_iter()
        .map(
            |(name, description, input_schema)| agena_provider::ToolApiDefinition {
                handler_key: format!("agena.auto_approval.{name}"),
                plugin_name: "agena.auto_approval".to_owned(),
                name: name.to_owned(),
                description: description.to_owned(),
                input_schema,
                output_schema: serde_json::json!({}),
                definition_identity: format!("agena-auto-approval:{name}"),
            },
        )
        .collect()
}

/// Resolve one completion into a verdict: the tool call the model submitted
/// first, then the text recovery path.
///
/// The tool call is the primary contract — the decision is carried by the tool
/// *name* and the cited rule by a schema-constrained field — and prose is only
/// read for routes that cannot carry tool calls at all.
fn classifier_verdict_from_response(
    response: &agena_provider::CompletionResponse,
) -> Option<agena_permission::ClassifierVerdict> {
    for call in &response.tool_calls {
        let agena_provider::CompletionToolCall::Function {
            name,
            arguments_json,
            ..
        } = call;
        if let Some(verdict) =
            agena_permission::ClassifierVerdict::from_tool_call(name, arguments_json)
        {
            return Some(verdict);
        }
    }
    if let Some(verdict) = agena_permission::ClassifierVerdict::parse(response.text.as_str()) {
        return Some(verdict);
    }
    // A model that reasons its way to the verdict and then stops without an
    // answer still submitted one; the strict contract is what keeps a stray
    // mention in the reasoning from counting as a decision.
    response
        .reasoning_text
        .as_deref()
        .and_then(agena_permission::ClassifierVerdict::parse)
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
