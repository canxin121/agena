use super::*;
use agena_domain::{ModelSwitchConfirmation, ModelSwitchInteraction, ModelSwitchSource};
use agena_plugin_host::sdk::{
    HookSubscription, ImportedModelSwitchDecision, ImportedModelSwitchInput, ModelSwitchCacheTtl,
    ModelSwitchPricing,
};
use serde::Deserialize;
use std::time::Instant;
use tokio::sync::watch;

use crate::SessionModelSwitchOutcome;
use crate::session::kernel::KernelProviderState;

const CONFIRMATION_LIFETIME: Duration = Duration::from_secs(300);
const POST_HOOK_PROMPT_WAIT: Duration = Duration::from_secs(5);

#[derive(Default)]
pub(super) struct SessionModelSwitch {
    confirmation: Option<PendingConfirmation>,
    context: Option<PendingContext>,
    effective_model: Option<ModelRef>,
}

struct PendingConfirmation {
    token: uuid::Uuid,
    created: Instant,
    version: i64,
    state: std::sync::Weak<ExecutionState>,
    changes: Vec<SelectionChange>,
}

#[derive(Clone)]
struct PendingContext {
    id: uuid::Uuid,
    model: ModelRef,
    result: watch::Receiver<Option<String>>,
}

pub(super) struct ModelSwitchContext {
    pub id: uuid::Uuid,
    pub text: String,
}

struct SelectionUpdate {
    changes: Vec<SelectionChange>,
    requested_model: Option<String>,
    interaction: ModelSwitchInteraction,
    check_fit: bool,
}

impl SessionManager {
    pub async fn switch_session_selection(
        &self,
        id: i64,
        options: SessionRunOptions,
    ) -> Result<(), AppError> {
        self.change_model_selection(id, Some(options), ModelSwitchInteraction::default())
            .await?;
        Ok(())
    }

    pub async fn clear_session_selection(&self, id: i64) -> Result<(), AppError> {
        self.change_model_selection(id, None, ModelSwitchInteraction::default())
            .await?;
        Ok(())
    }

    pub async fn set_session_model_override(
        &self,
        id: i64,
        model: ModelRef,
    ) -> Result<Session, AppError> {
        self.require_writable_bot_history(id).await?;
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let scope = agena_plugin_host::host::ImportedPromptScope::current()
            .acquire(&dispatch.submission)
            .await;
        let requested_model = Some(model.model_id.to_string());
        scope
            .run(self.apply_model_selection(
                session,
                SelectionUpdate {
                    changes: vec![SelectionChange::Model(Some(model))],
                    requested_model,
                    interaction: ModelSwitchInteraction::default(),
                    check_fit: true,
                },
            ))
            .await?;
        self.get_session(id).await
    }

    pub(super) async fn change_model_selection(
        &self,
        id: i64,
        options: Option<SessionRunOptions>,
        interaction: ModelSwitchInteraction,
    ) -> Result<SessionModelSwitchOutcome, AppError> {
        if matches!(
            interaction.source,
            ModelSwitchSource::Auto | ModelSwitchSource::Resume
        ) || (interaction.confirmation.is_some()
            && interaction.source != ModelSwitchSource::Command)
        {
            return Err(AppError::Config(
                "Invalid source for a model switch request".into(),
            ));
        }
        self.require_writable_bot_history(id).await?;
        let session = session_id(id)?;
        let dispatch = self.session_dispatch(session);
        let scope = agena_plugin_host::host::ImportedPromptScope::current()
            .acquire(&dispatch.submission)
            .await;
        let (changes, requested_model) = match options {
            Some(options) => selection_changes(options)?,
            None => (
                vec![
                    SelectionChange::Model(None),
                    SelectionChange::ThinkingMode(None),
                    SelectionChange::SpeedMode(None),
                    SelectionChange::Verbosity(None),
                    SelectionChange::ParallelToolCalls(None),
                ],
                None,
            ),
        };
        scope
            .run(self.apply_model_selection(
                session,
                SelectionUpdate {
                    changes,
                    requested_model,
                    interaction,
                    check_fit: true,
                },
            ))
            .await
    }

    /// Complete selection for internal callers that do not represent a user switch.
    pub async fn update_session_selection(
        &self,
        id: i64,
        options: SessionRunOptions,
    ) -> Result<(), AppError> {
        self.apply_model_selection(
            session_id(id)?,
            SelectionUpdate {
                changes: complete_selection(options),
                requested_model: None,
                interaction: ModelSwitchInteraction {
                    source: ModelSwitchSource::Auto,
                    confirmation: None,
                },
                check_fit: false,
            },
        )
        .await?;
        Ok(())
    }

    /// Explicit command options can also change the persisted model at submission.
    pub(super) async fn persist_submission_selection(
        &self,
        id: i64,
        options: SessionRunOptions,
    ) -> Result<(), AppError> {
        let (changes, requested_model) = selection_changes(options)?;
        self.apply_model_selection(
            session_id(id)?,
            SelectionUpdate {
                changes,
                requested_model,
                interaction: ModelSwitchInteraction::default(),
                check_fit: false,
            },
        )
        .await?;
        Ok(())
    }

    async fn apply_model_selection(
        &self,
        session: SessionId,
        update: SelectionUpdate,
    ) -> Result<SessionModelSwitchOutcome, AppError> {
        let SelectionUpdate {
            changes,
            requested_model,
            interaction,
            check_fit,
        } = update;
        let mut system_messages = Vec::new();
        let session_control = || self.kernel().reader.control_snapshot(session, 1);
        let snapshot = read_with_busy_retry(session, "selection", session_control).await?;
        let version = snapshot.metadata.metadata_version.get();
        let state = self.state_for_session(session).await?;
        let now = Instant::now();
        if let Some(token) = interaction.confirmation {
            let accepted = self
                .model_switches
                .lock()
                .get_mut(&session)
                .and_then(|pending| {
                    pending.confirmation.take_if(|confirmation| {
                        confirmation.token == token
                            && confirmation.version == version
                            && now.duration_since(confirmation.created) < CONFIRMATION_LIFETIME
                            && confirmation.changes == changes
                            && confirmation
                                .state
                                .upgrade()
                                .is_some_and(|previous| Arc::ptr_eq(&state, &previous))
                    })
                });
            if accepted.is_none() {
                return Err(AppError::InteractiveRequestNotPending {
                    request_kind: "model switch".into(),
                    request_id: token.to_string(),
                });
            }
        } else if (check_fit
            || changes
                .iter()
                .any(|change| matches!(change, SelectionChange::Model(_))))
            && let Some(pending) = self.model_switches.lock().get_mut(&session)
        {
            pending.confirmation = None;
        }

        if changes.is_empty() {
            return Ok(SessionModelSwitchOutcome::Applied { system_messages });
        }
        let plugins = state.executor.plugin_manager();
        let project =
            snapshot.metadata.workspace_id.is_some() && snapshot.configuration.owner.is_none();
        let pre = project
            && !matches!(
                interaction.source,
                ModelSwitchSource::Auto | ModelSwitchSource::Resume
            )
            && plugins.has_subscription(HookSubscription::IMPORTED_MODEL_SWITCH_BEFORE);
        let post =
            project && plugins.has_subscription(HookSubscription::IMPORTED_MODEL_SWITCH_AFTER);
        let model_change = changes.iter().find_map(|change| match change {
            SelectionChange::Model(model) => Some(model),
            _ => None,
        });
        let mut switch = None;
        let mut changed = false;
        let mut effective_model = None;
        let is_observed = self.model_switches.lock().contains_key(&session);
        if let Some(requested) = model_change
            && (check_fit || pre || post || is_observed)
        {
            let resolve = |override_selection: &ExecutionSelection| {
                let selection = state
                    .config
                    .session_selection(&snapshot.configuration, override_selection)
                    .map_err(AppError::Config)?;
                Ok::<_, AppError>(
                    state
                        .providers
                        .resolve_default_model_selection(&selection)?,
                )
            };
            let previous = resolve(
                snapshot
                    .metadata
                    .selection
                    .as_ref()
                    .unwrap_or(&ExecutionSelection::default()),
            )
            .ok()
            .flatten();
            let next = match requested {
                Some(model) => Some(state.providers.resolve_model_selection(
                    model.provider_id.as_ref(),
                    model.adapter_id.as_ref().map(AsRef::as_ref),
                    Some(model.model_id.as_ref()),
                )?),
                None => resolve(&ExecutionSelection::default())?,
            };
            changed = previous != next;
            effective_model.clone_from(&next);
            if let Some(next) = next
                && changed
            {
                if check_fit {
                    self.ensure_model_fits_session(session.get(), &next).await?;
                }
                if let Some(previous) = previous
                    && (pre || post)
                {
                    let context = self.execution_context_with_state(session, &state).await?;
                    let executor = state.executor.for_session_resource_context(&context);
                    let event = self
                        .model_switch_input(
                            session,
                            &state,
                            &previous,
                            &next,
                            requested_model,
                            interaction.source,
                        )
                        .await?;
                    if pre && interaction.confirmation.is_none() {
                        let decision = plugins
                            .dispatch_imported_model_switch(
                                event.clone(),
                                true,
                                |key| executor.plugin_is_enabled(key),
                                self.shutdown.child_token(),
                            )
                            .await;
                        system_messages = decision.system_messages;
                        match decision.decision {
                            Some(ImportedModelSwitchDecision::Ask(reason))
                                if interaction.source == ModelSwitchSource::Command =>
                            {
                                let token = uuid::Uuid::new_v4();
                                self.model_switches
                                    .lock()
                                    .entry(session)
                                    .or_default()
                                    .confirmation = Some(PendingConfirmation {
                                    token,
                                    created: Instant::now(),
                                    version,
                                    state: Arc::downgrade(&state),
                                    changes,
                                });
                                return Ok(SessionModelSwitchOutcome::ConfirmationRequired {
                                    request: ModelSwitchConfirmation {
                                        token,
                                        reason: public_switch_reason(&reason),
                                    },
                                    system_messages,
                                });
                            }
                            Some(
                                ImportedModelSwitchDecision::Ask(reason)
                                | ImportedModelSwitchDecision::Deny(reason),
                            ) => {
                                let reason = std::iter::once(reason)
                                    .chain(system_messages)
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                return Err(AppError::ModelSwitchBlocked(public_switch_reason(
                                    &reason,
                                )));
                            }
                            _ => {}
                        }
                    }
                    switch = Some((event, next, executor));
                }
            }
        }
        // Order the durable model write and its pending post-hook context with
        // observations of the session's effective model from the run driver.
        let dispatch = self.session_dispatch(session);
        let _selection = dispatch.model_selection.lock().await;
        if self.shutdown.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        let request = RequestId::new();
        persist(|| {
            self.kernel()
                .sessions
                .patch_selection(request, session, version, changes.clone())
        })
        .await
        .map_err(kernel_error)?;
        if let Some((event, next, executor)) = switch
            && post
        {
            if snapshot.configuration.pending_model_switch_context.is_some() {
                self.set_persisted_model_switch_context(session, None).await;
            }
            self.start_post_model_switch(session, event, next, executor);
        } else if changed {
            if let Some(pending) = self.model_switches.lock().get_mut(&session) {
                pending.effective_model = effective_model;
                pending.context = None;
            }
            if snapshot.configuration.pending_model_switch_context.is_some() {
                self.set_persisted_model_switch_context(session, None).await;
            }
        }
        Ok(SessionModelSwitchOutcome::Applied { system_messages })
    }

    pub(super) async fn observe_implicit_model_switch(
        &self,
        session: SessionId,
        state: &Arc<ExecutionState>,
        model: &ModelRef,
        executor: &ToolExecutor,
    ) {
        let subscribed = executor
            .plugin_manager()
            .has_subscription(HookSubscription::IMPORTED_MODEL_SWITCH_AFTER);
        if !subscribed && !self.model_switches.lock().contains_key(&session) {
            return;
        }
        if let Err(error) = self
            .observe_effective_model(session, state, model, executor, subscribed)
            .await
        {
            tracing::warn!(session_id = session.get(), %error, "could not observe the effective session model");
        }
    }

    async fn observe_effective_model(
        &self,
        session: SessionId,
        state: &Arc<ExecutionState>,
        model: &ModelRef,
        executor: &ToolExecutor,
        subscribed: bool,
    ) -> Result<(), AppError> {
        let dispatch = self.session_dispatch(session);
        let _selection = dispatch.model_selection.lock().await;
        if !Arc::ptr_eq(state, &self.state.load_full()) {
            return Ok(());
        }
        let snapshot = read_with_busy_retry(session, "effective_model", || {
            self.kernel().reader.control_snapshot(session, 1)
        })
        .await?;
        if snapshot.metadata.workspace_id.is_none()
            || snapshot.configuration.owner.is_some()
            || snapshot.configuration.conversation.as_ref().is_some_and(|view| view.is_read_only())
        {
            return Ok(());
        }
        let selected = snapshot.metadata.selection.clone().unwrap_or_default();
        let resolve = |selected: &ExecutionSelection| -> Result<Option<ModelRef>, AppError> {
            let selection = state.config.session_selection(&snapshot.configuration, selected)
                .map_err(AppError::Config)?;
            Ok(state.providers.resolve_default_model_selection(&selection)?)
        };
        // A request may have a one-off model override. The hook follows the
        // session's effective selection when that choice is about to be used.
        if resolve(&selected)?.as_ref() != Some(model) {
            return Ok(());
        }
        let mut previous = self.model_switches.lock().get(&session)
            .and_then(|pending| pending.effective_model.clone());
        if previous.as_ref() == Some(model) {
            return Ok(());
        }
        // Preserve an undelivered PostModelSwitch context saved before restart
        // when the session resumes on that exact target model.
        if previous.is_none()
            && snapshot
                .configuration
                .pending_model_switch_context
                .as_ref()
                .is_some_and(|pending| &pending.model == model)
        {
            if Arc::ptr_eq(state, &self.state.load_full()) {
                let mut switches = self.model_switches.lock();
                let pending = switches.entry(session).or_default();
                pending.effective_model = Some(model.clone());
            }
            return Ok(());
        }
        let mut source = ModelSwitchSource::Auto;
        let mut requested = None;
        if previous.is_none()
            && subscribed
            && snapshot.summary.created_at_ms < self.started_at_ms
            && (selected.provider.is_some() || selected.adapter.is_some() || selected.model.is_some())
        {
            // Restore the saved selection over the current ambient default.
            // A previous response may have used a per-request model override.
            previous = resolve(&ExecutionSelection::default())?;
            source = ModelSwitchSource::Resume;
            requested = selected.model;
        }
        let post = if subscribed && let Some(previous) = previous.filter(|previous| previous != model) {
            Some(self.model_switch_input(session, state, &previous, model, requested, source).await?)
        } else {
            None
        };
        // A queued run keeps its original provider snapshot. A reload during
        // this read must not overwrite the observation for the new snapshot.
        if Arc::ptr_eq(state, &self.state.load_full()) {
            if let Some(event) = post {
                if snapshot.configuration.pending_model_switch_context.is_some() {
                    self.set_persisted_model_switch_context(session, None).await;
                }
                self.start_post_model_switch(session, event, model.clone(), executor.clone());
            } else {
                {
                    let mut switches = self.model_switches.lock();
                    let pending = switches.entry(session).or_default();
                    pending.effective_model = Some(model.clone());
                    pending.context = None;
                }
                if snapshot
                    .configuration
                    .pending_model_switch_context
                    .as_ref()
                    .is_some_and(|saved| &saved.model != model)
                {
                    self.set_persisted_model_switch_context(session, None).await;
                }
            }
        }
        Ok(())
    }

    async fn model_switch_input(
        &self,
        session: SessionId,
        state: &ExecutionState,
        previous: &ModelRef,
        next: &ModelRef,
        requested_model: Option<String>,
        source: ModelSwitchSource,
    ) -> Result<ImportedModelSwitchInput, AppError> {
        let usage_round = read_with_busy_retry(session, "model_switch_usage", || {
            self.kernel().reader.latest_usage_round(session)
        })
        .await?;
        let usage = usage_round
            .as_ref()
            .and_then(|round| round.provider_state.as_ref())
            .and_then(|value| KernelProviderState::deserialize(value).ok());
        let latest_usage = usage.as_ref().and_then(|last| last.usage.as_ref());
        let context_tokens = latest_usage.map_or(0, crate::session::usage::context_tokens);
        let cache_ttl = observed_cache_ttl(latest_usage);
        let prompt_cache_warm = usage.as_ref().is_some_and(|last| &last.model == previous)
            && latest_usage
                .is_some_and(|usage| usage.cache_read_tokens > 0 || usage.cache_write_tokens > 0)
            && usage_round
                .as_ref()
                .and_then(|round| round.finished_at_ms)
                .is_some_and(|finished| {
                    Utc::now().timestamp_millis().saturating_sub(finished)
                        < cache_ttl.duration().as_millis() as i64
                });
        let (estimated_cache_write_usd, pricing) =
            model_switch_price(state, next, context_tokens, cache_ttl);
        Ok(ImportedModelSwitchInput {
            session_id: session.get(),
            from_model: previous.model_id.to_string(),
            to_model: next.model_id.to_string(),
            canonical_model: claude_matcher_model(next.model_id.as_ref()),
            requested_model,
            source,
            context_tokens,
            prompt_cache_warm,
            cache_ttl,
            estimated_cache_write_usd,
            pricing,
        })
    }

    fn start_post_model_switch(
        &self,
        session: SessionId,
        event: ImportedModelSwitchInput,
        model: ModelRef,
        executor: ToolExecutor,
    ) {
        let (sender, result) = watch::channel(None);
        let id = uuid::Uuid::new_v4();
        {
            let mut switches = self.model_switches.lock();
            let pending = switches.entry(session).or_default();
            pending.effective_model = Some(model.clone());
            pending.context = Some(PendingContext {
                id,
                model: model.clone(),
                result,
            });
        }
        let cancel = self.shutdown.child_token();
        let manager = self.clone();
        self.control_tasks.spawn(async move {
            let result = executor
                .plugin_manager()
                .dispatch_imported_model_switch(
                    event,
                    false,
                    |key| executor.plugin_is_enabled(key),
                    cancel,
                )
                .await;
            if !result.context.is_empty() {
                let mut text = result.context.join("\n");
                text.truncate(text.floor_char_boundary(text.len().min(16 * 1024)));
                if !text.trim().is_empty() {
                    let dispatch = manager.session_dispatch(session);
                    let _selection = dispatch.model_selection.lock().await;
                    let still_pending = manager
                        .model_switches
                        .lock()
                        .get(&session)
                        .and_then(|state| state.context.as_ref())
                        .is_some_and(|pending| pending.id == id);
                    if still_pending {
                        manager
                            .set_persisted_model_switch_context(
                                session,
                                Some(agena_storage::kernel::PendingModelSwitchContext {
                                    id,
                                    model,
                                    text: text.clone(),
                                }),
                            )
                            .await;
                    }
                    let _ = sender.send(Some(text));
                }
            }
        });
    }

    async fn set_persisted_model_switch_context(
        &self,
        session: SessionId,
        context: Option<agena_storage::kernel::PendingModelSwitchContext>,
    ) {
        let command = RequestId::new();
        let change = SessionConfigurationChange {
            pending_model_switch_context: Some(context),
            ..Default::default()
        };
        if let Err(error) = persist(|| {
            self.kernel()
                .sessions
                .configure(command, session, change.clone())
        })
        .await
        {
            tracing::debug!(
                session_id = session.get(),
                %error,
                "could not persist pending model switch context"
            );
        }
    }

    pub(super) async fn pending_model_switch_context(
        &self,
        session: SessionId,
        model: &ModelRef,
        cancellation: &CancellationToken,
    ) -> Option<ModelSwitchContext> {
        let in_memory = self
            .model_switches
            .lock()
            .get(&session)
            .and_then(|state| state.context.clone());
        if let Some(mut pending) = in_memory {
            if &pending.model != model {
                return None;
            }
            let outcome = {
                let result = tokio::select! {
                    biased;
                    _ = cancellation.cancelled() => return None,
                    result = tokio::time::timeout(POST_HOOK_PROMPT_WAIT, pending.result.wait_for(Option::is_some)) => result,
                };
                match result {
                    Ok(Ok(value)) => Ok(value.clone()),
                    Ok(Err(_)) => Err(true),
                    Err(_) => Err(false),
                }
            };
            return match outcome {
                Ok(value) => value.map(|text| ModelSwitchContext {
                    id: pending.id,
                    text,
                }),
                Err(closed) => {
                    if closed {
                        self.consume_model_switch_context(session, pending.id).await;
                    }
                    None
                }
            };
        }
        let configuration = read_with_busy_retry(session, "pending_model_switch", || {
            self.kernel().reader.configuration(session)
        })
        .await
        .ok()?;
        let saved = configuration.pending_model_switch_context.as_ref()?;
        (&saved.model == model).then(|| ModelSwitchContext {
            id: saved.id,
            text: saved.text.clone(),
        })
    }

    pub(super) async fn consume_model_switch_context(
        &self,
        session: SessionId,
        id: uuid::Uuid,
    ) -> bool {
        let dispatch = self.session_dispatch(session);
        let _selection = dispatch.model_selection.lock().await;
        let memory_consumed = self
            .model_switches
            .lock()
            .get_mut(&session)
            .and_then(|state| state.context.take_if(|context| context.id == id))
            .is_some();
        let persisted_matches = read_with_busy_retry(session, "consume_model_switch", || {
            self.kernel().reader.configuration(session)
        })
        .await
        .ok()
        .and_then(|config| config.pending_model_switch_context.clone())
        .is_some_and(|saved| saved.id == id);
        if persisted_matches {
            self.set_persisted_model_switch_context(session, None).await;
        }
        memory_consumed || persisted_matches
    }
}

fn public_switch_reason(reason: &str) -> String {
    let reason = agena_failure::diagnostic::scrubbed_preserve(reason, 220);
    if reason.trim().is_empty() {
        "Model switch policy".into()
    } else {
        reason
    }
}

fn complete_selection(options: SessionRunOptions) -> Vec<SelectionChange> {
    vec![
        SelectionChange::Model(Some(options.model)),
        SelectionChange::ThinkingMode(options.thinking_mode),
        SelectionChange::SpeedMode(options.speed_mode),
        SelectionChange::Verbosity(options.verbosity),
        SelectionChange::ParallelToolCalls(options.model_supports_parallel_tool_calls),
    ]
}

fn selection_changes(
    options: SessionRunOptions,
) -> Result<(Vec<SelectionChange>, Option<String>), AppError> {
    let Some(selection) = options.selection_override else {
        let requested = Some(options.model.model_id.to_string());
        return Ok((complete_selection(options), requested));
    };
    if selection.provider.is_some() || selection.adapter.is_some() || selection.model.is_some() {
        let model = selection
            .model_ref()
            .map_err(|error| AppError::Config(error.to_string()))?
            .unwrap_or(options.model);
        return Ok((
            vec![
                SelectionChange::Model(Some(model)),
                SelectionChange::ThinkingMode(selection.thinking_mode),
                SelectionChange::SpeedMode(selection.speed_mode),
                SelectionChange::Verbosity(selection.verbosity),
                SelectionChange::ParallelToolCalls(selection.parallel_tool_calls),
            ],
            selection.model,
        ));
    }
    let mut changes = Vec::new();
    changes.extend(
        selection
            .thinking_mode
            .map(|value| SelectionChange::ThinkingMode(Some(value))),
    );
    changes.extend(
        selection
            .speed_mode
            .map(|value| SelectionChange::SpeedMode(Some(value))),
    );
    changes.extend(
        selection
            .verbosity
            .map(|value| SelectionChange::Verbosity(Some(value))),
    );
    changes.extend(
        selection
            .parallel_tool_calls
            .map(|value| SelectionChange::ParallelToolCalls(Some(value))),
    );
    Ok((changes, None))
}

fn claude_matcher_model(id: &str) -> Option<String> {
    let base = id.trim();
    let base = base.strip_suffix("[1m]").unwrap_or(base);
    let id = agena_provider::normalized_catalog_model_id(base);
    let stable = id
        .rsplit_once('-')
        .filter(|(_, suffix)| suffix.len() == 8 && suffix.bytes().all(|byte| byte.is_ascii_digit()))
        .map_or(id.as_str(), |(base, _)| base);
    let version = stable.strip_prefix("claude-")?;
    version
        .bytes()
        .any(|byte| byte.is_ascii_digit())
        .then(|| stable.to_owned())
}

pub(super) fn observed_cache_ttl(
    usage: Option<&agena_provider::CompletionUsage>,
) -> ModelSwitchCacheTtl {
    if usage.is_some_and(|usage| usage.cache_write_1h_tokens > 0) {
        ModelSwitchCacheTtl::OneHour
    } else {
        ModelSwitchCacheTtl::FiveMinutes
    }
}

pub(super) fn model_switch_price(
    state: &ExecutionState,
    target: &ModelRef,
    tokens: u64,
    ttl: ModelSwitchCacheTtl,
) -> (f64, ModelSwitchPricing) {
    use agena_provider::CompletionUsage;

    let usage = CompletionUsage {
        cache_write_tokens: tokens,
        cache_write_1h_tokens: if matches!(ttl, ModelSwitchCacheTtl::OneHour) {
            tokens
        } else {
            0
        },
        ..Default::default()
    };
    let catalog = agena_provider::estimate_completion_usage_cost_usd(
        target.provider_id.as_ref(),
        target.model_id.as_ref(),
        &usage,
    );
    let metadata = state
        .providers
        .model_metadata(target)
        .ok()
        .and_then(|metadata| metadata.pricing);
    let price = metadata
        .and_then(|price| price.cache_write_usd_per_million_tokens)
        .and_then(|price| price.parse::<f64>().ok())
        .filter(|price| price.is_finite() && *price >= 0.0)
        .map(|rate| rate * (tokens as f64 / 1_000_000.0))
        .filter(|cost| cost.is_finite());
    match (price, catalog) {
        (_, Some(cost)) if matches!(ttl, ModelSwitchCacheTtl::OneHour) => {
            (cost, ModelSwitchPricing::Catalog)
        }
        (Some(cost), Some(catalog)) if cost == catalog => (cost, ModelSwitchPricing::Catalog),
        (Some(cost), _) => (cost, ModelSwitchPricing::Configured),
        (None, Some(cost)) => (cost, ModelSwitchPricing::Catalog),
        (None, None) => (0.0, ModelSwitchPricing::Default),
    }
}
