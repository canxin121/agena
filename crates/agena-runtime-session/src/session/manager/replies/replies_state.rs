use super::super::recover_read;
use super::{
    AppError, Arc, ModelRef, PathBuf, PersistedPermissionRule, SessionManager, SessionManagerState,
    SessionRunOptions, ToolInvocationExecution, custom_payload_value,
    managed_project_state_permission, mode_request_override_for_adapter, mpsc,
    payload_tool_name_for_invocation,
};
use crate::session::Session;
use crate::session::store::new_part_from_content;
use agena_domain::ToolInvocation;
use agena_runtime_contracts::part_content::TypedContent;
use agena_storage::store::{PartDelta, PartRole, PartState};

impl SessionManager {
    pub(in crate::session::manager) async fn persist_session_changes(
        &self,
        session: Session,
        changed_part_ids: Vec<i64>,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.persist_session_changes_with_rules(
            session,
            changed_part_ids,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    /// Flush in-memory part mutations to their durable rows and merge the
    /// authoritative engine rows back into the projection. v2 has no message
    /// checkpoints: each changed part id becomes one `update_part` facade call
    /// carrying the part's current content/state/summary. Run terminalization
    /// (complete_run/cancel_run) is owned by the turn/tool paths, never here.
    pub(in crate::session::manager) async fn persist_session_changes_with_rules(
        &self,
        mut session: Session,
        changed_part_ids: Vec<i64>,
        persisted_rules: Vec<PersistedPermissionRule>,
        _state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        // Rules may target any scope (session, workspace, or global). Persist
        // them through the kept permission-rule repository (design 19.1) and
        // invalidate every cached snapshot so no session keeps applying stale
        // rules. The session data itself flows through the sealed facade.
        self.invalidate_rule_snapshots();
        for rule in &persisted_rules {
            self.permission_rules
                .upsert(rule)
                .await
                .map_err(|error| AppError::Internal(format!("persist permission rule: {error}")))?;
        }
        if changed_part_ids.is_empty() {
            return Ok(session);
        }
        let session_id = session.id;
        let mut updated_parts = Vec::with_capacity(changed_part_ids.len());
        for part_id in changed_part_ids {
            let part = session
                .parts()
                .iter()
                .find(|part| part.part_id == part_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "part {part_id} missing from the session {session_id} projection while persisting"
                    ))
                })?;
            let updated = self
                .store
                .update_part(
                    session_id,
                    part_id,
                    PartDelta {
                        state: Some(part.state),
                        content: Some(part.content.clone()),
                        content_text_delta: None,
                        summary: part.summary.clone(),
                        provider_state: part.provider_state.clone(),
                        finished_at_ms: part.finished_at_ms,
                    },
                )
                .await?;
            updated_parts.push(updated);
        }
        let mut parts = session.parts().to_vec();
        for updated in updated_parts {
            if let Some(existing) = parts
                .iter_mut()
                .find(|part| part.part_id == updated.part_id)
            {
                *existing = updated;
            } else {
                parts.push(updated);
            }
        }
        session.install_projected_parts(parts);
        Ok(session)
    }

    pub(in crate::session::manager) fn apply_run_selection_to_session(
        &self,
        session: &mut Session,
        options: &SessionRunOptions,
    ) -> bool {
        let next_model_provider_id = options.model.provider_id.to_string();
        let next_model_adapter_id = options.model.adapter_id.as_ref().map(ToString::to_string);
        let next_model_id = options.model.model_id.to_string();
        let next_thinking_mode = options.thinking_mode.clone();
        let next_speed_mode = options.speed_mode.clone();
        let next_verbosity = options.verbosity.clone();
        let next_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let changed = session.runtime.execution.selection.provider.as_deref()
            != Some(next_model_provider_id.as_str())
            || session.runtime.execution.selection.adapter.as_deref()
                != next_model_adapter_id.as_deref()
            || session.runtime.execution.selection.model.as_deref() != Some(next_model_id.as_str())
            || session.runtime.execution.selection.thinking_mode != next_thinking_mode
            || session.runtime.execution.selection.speed_mode != next_speed_mode
            || session.runtime.execution.selection.verbosity != next_verbosity
            || session.runtime.execution.selection.parallel_tool_calls != next_parallel_tool_calls;
        session.runtime.set_model_override(
            Some(next_model_provider_id),
            next_model_adapter_id,
            Some(next_model_id),
        );
        session.runtime.set_model_mode_overrides(
            next_thinking_mode,
            next_speed_mode,
            next_verbosity,
            next_parallel_tool_calls,
        );
        changed
    }

    pub(in crate::session::manager) async fn apply_execution_context_to_run_options_async(
        &self,
        session: &Session,
        mut options: SessionRunOptions,
    ) -> Result<SessionRunOptions, AppError> {
        self.apply_selection_modes_to_run_options(session, &mut options)?;
        options.system = Some(
            self.assemble_session_system_prompt_async(session, options.system.as_deref())
                .await,
        );
        if options.temperature.is_none() {
            let execution = self.execution_state();
            let provider_registry = &execution.provider_registry;
            if let Ok(metadata) = provider_registry.model_metadata(&options.model) {
                options.temperature = metadata.parsed_default_temperature();
            }
        }
        Ok(options)
    }

    pub(in crate::session::manager) fn apply_selection_modes_to_run_options(
        &self,
        session: &Session,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let state = self.execution_state();
        let effective_selection = state
            .config
            .default_selection
            .overlay_with_cascade(&session.runtime.execution.selection);
        let selection_model = effective_selection.model_ref().map_err(|error| {
            AppError::Internal(format!(
                "session {} contains invalid execution model selection: {error}",
                session.id
            ))
        })?;
        let modes_belong_to_options_model = selection_model
            .as_ref()
            .is_some_and(|model| model == &options.model);
        if options.thinking_mode.is_none() {
            options.thinking_mode = modes_belong_to_options_model
                .then(|| effective_selection.thinking_mode.clone())
                .flatten();
        }
        if options.speed_mode.is_none() {
            options.speed_mode = modes_belong_to_options_model
                .then(|| effective_selection.speed_mode.clone())
                .flatten();
        }
        if options.request_override.parallel_tool_calls().is_none() {
            options.request_override.set_parallel_tool_calls(
                modes_belong_to_options_model
                    .then_some(effective_selection.parallel_tool_calls)
                    .flatten(),
            );
        }
        if options.verbosity.is_none() {
            options.verbosity = modes_belong_to_options_model
                .then(|| effective_selection.verbosity.clone())
                .flatten();
        }
        self.apply_model_mode_requests(options)
    }

    pub(in crate::session::manager) fn apply_model_mode_requests(
        &self,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let execution = self.execution_state();
        let provider_registry = &execution.provider_registry;
        let resolved_adapter_id = options.model.adapter_id.clone();

        let requested_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let mut merged_override = options.request_override.clone();
        merged_override.set_parallel_tool_calls(None);
        let thinking_modes = provider_registry.model_thinking_modes(&options.model)?;
        if options.thinking_mode.is_none() {
            options.thinking_mode = thinking_modes.iter().find_map(|mode| {
                mode.is_default
                    .then(|| mode.selector().map(|selector| selector.into_owned()))
                    .flatten()
            });
        }
        if let Some(thinking_mode_name) = options.thinking_mode.as_deref() {
            let thinking_mode = thinking_modes
                .iter()
                .find(|mode| mode.selector().as_deref() == Some(thinking_mode_name))
                .ok_or_else(|| {
                    AppError::Config(format!(
                        "model `{}` has no think mode `{thinking_mode_name}`",
                        options.model
                    ))
                })?;
            options.thinking = thinking_mode.thinking.clone();
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &thinking_mode.request_override,
                &thinking_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        let speed_modes = provider_registry.model_speed_modes(&options.model)?;
        if options.speed_mode.is_none() {
            options.speed_mode = speed_modes
                .iter()
                .find(|(_, mode)| mode.is_default)
                .map(|(name, _)| name.clone());
        }
        if let Some(speed_mode_name) = options.speed_mode.as_deref() {
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no speed mode `{speed_mode_name}`",
                    options.model
                ))
            })?;
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &speed_mode.request_override,
                &speed_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        if requested_parallel_tool_calls.is_some() {
            merged_override.set_parallel_tool_calls(requested_parallel_tool_calls);
        }
        options.request_override = merged_override;
        Ok(())
    }

    pub(in crate::session::manager) fn resolve_effective_session_permission(
        &self,
        session: &Session,
        state: &SessionManagerState,
    ) -> crate::authorization::PermissionConfig {
        let mut effective = recover_read(
            state.shared_permission.as_ref(),
            "resolve shared session permission",
        )
        .clone();
        effective.merge_from(managed_project_state_permission(
            state.tool_executor.workspace_root(),
        ));
        let session_permission = recover_read(
            state.shared_session_permissions.as_ref(),
            "resolve session-specific permission",
        )
        .get(&session.id)
        .cloned()
        .unwrap_or_else(|| session.runtime.execution.selection.permission.clone());
        effective.merge_from(session_permission);
        effective
    }

    pub(in crate::session::manager) fn refresh_execution_policy(
        &self,
        session: &mut Session,
        state: &SessionManagerState,
    ) {
        let effective = self.resolve_effective_session_permission(session, state);
        if session.is_subagent() {
            session.runtime.execution.capability_denied_tool_names =
                crate::session::manager::runs::non_recursive_subtask_capability_denials();
        } else {
            session.runtime.execution.permission_ceiling = Default::default();
            session
                .runtime
                .execution
                .capability_denied_tool_names
                .clear();
        }
        session.runtime.execution.effective_permission = effective;
    }

    pub(in crate::session::manager) fn model_from_session_selection(
        &self,
        session: &Session,
    ) -> Result<Option<ModelRef>, AppError> {
        session
            .runtime
            .execution
            .selection
            .model_ref()
            .map_err(|error| {
                AppError::Internal(format!(
                    "session {} contains invalid execution model selection: {error}",
                    session.id
                ))
            })
    }

    pub(in crate::session::manager) fn model_from_session_or_error(
        &self,
        session: &Session,
        state: &SessionManagerState,
    ) -> Result<ModelRef, AppError> {
        self.model_from_session_selection(session)?
            .map(Ok)
            .unwrap_or_else(|| {
                self.default_model_from_config(state)?.ok_or_else(|| {
                    AppError::Internal(format!(
                        "model is required for session {}; set a session model or global default model",
                        session.id
                    ))
                })
            })
    }

    pub(in crate::session::manager) fn default_model_from_config(
        &self,
        state: &SessionManagerState,
    ) -> Result<Option<ModelRef>, AppError> {
        Ok(state
            .provider_registry
            .resolve_default_model_selection(&state.config.default_selection)?)
    }

    pub(in crate::session::manager) async fn run_options_from_session_async(
        &self,
        session: &Session,
        state: Arc<SessionManagerState>,
    ) -> Result<SessionRunOptions, AppError> {
        let model = self.model_from_session_or_error(session, &state)?;

        self.apply_execution_context_to_run_options_async(
            session,
            SessionRunOptions {
                model,
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
            },
        )
        .await
    }

    pub(in crate::session::manager) fn resolve_model_selection_override(
        &self,
        provider_registry: &crate::provider::ProviderRegistry,
        base_model: &ModelRef,
        requested_selection: &agena_domain::ModelSelectionConfig,
    ) -> Result<ModelRef, AppError> {
        if requested_selection.is_empty() {
            return Ok(base_model.clone());
        }
        let requested_provider = requested_selection
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_adapter = requested_selection
            .adapter
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_model = requested_selection
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let provider_changed =
            requested_provider.is_some_and(|provider| provider != base_model.provider_id.as_ref());
        let provider_id = requested_provider.unwrap_or(base_model.provider_id.as_ref());
        let base_adapter = (!provider_changed)
            .then(|| base_model.adapter_id.as_ref().map(AsRef::<str>::as_ref))
            .flatten();
        let adapter_id = requested_adapter.or(base_adapter);
        let base_model_id = (!provider_changed && requested_adapter.is_none())
            .then(|| base_model.model_id.as_ref());
        let model_id = requested_model.or(base_model_id);
        Ok(provider_registry.resolve_model_selection(provider_id, adapter_id, model_id)?)
    }

    pub(in crate::session::manager) fn apply_tool_success_execution_context(
        &self,
        session: &mut Session,
        invocation: &ToolInvocation,
        execution: &ToolInvocationExecution,
    ) {
        let payload_tool_name = payload_tool_name_for_invocation(invocation);
        if let Some(output) = crate::tool::ToolPayloadOutput::from_tool_output(
            payload_tool_name.as_str(),
            &execution.output,
        ) {
            match output {
                crate::tool::ToolPayloadOutput::EnterSnapshot { path, .. } => {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                    return;
                }
                crate::tool::ToolPayloadOutput::ExitSnapshot { .. } => {
                    session.runtime.set_effective_workspace_root(None);
                    return;
                }
                _ => {}
            }
        }

        match execution
            .view
            .metadata
            .get("agena.effect")
            .map(String::as_str)
        {
            Some("enter_snapshot") => {
                if let Some(path) = custom_payload_value(&execution.output)
                    .and_then(|value| value.get("path").cloned())
                    .and_then(|value| value.as_str().map(str::to_string))
                {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                }
            }
            Some("exit_snapshot") => {
                session.runtime.set_effective_workspace_root(None);
            }
            _ => {}
        }
    }

    pub(in crate::session::manager) fn subtask_run_options(
        &self,
        parent: &Session,
        state: &SessionManagerState,
        requested_selection: &agena_domain::ModelSelectionConfig,
    ) -> Result<SessionRunOptions, AppError> {
        let parent_model = match self.model_from_session_selection(parent)? {
            Some(model) => model,
            None => self.default_model_from_config(state)?.ok_or_else(|| {
                AppError::Internal(
                    "subtask requires a parent or global default model before it can run"
                        .to_string(),
                )
            })?,
        };
        let model = self.resolve_model_selection_override(
            &state.provider_registry,
            &parent_model,
            requested_selection,
        )?;
        let parent_selection = state
            .config
            .default_selection
            .overlay_with_cascade(&parent.runtime.execution.selection);
        let inherit_parent_modes = parent_selection
            .model_ref()
            .map_err(|error| {
                AppError::Config(agena_failure::diagnostic::format_error_chain_with_context(
                    "decode the parent session model selection while preparing a subtask",
                    &error,
                ))
            })?
            .is_some_and(|selected| selected == model);
        let inherited = inherit_parent_modes.then_some(&parent_selection);
        let requested_mode = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned)
        };
        let mut options = SessionRunOptions {
            model,
            thinking_mode: requested_mode(&requested_selection.thinking_mode)
                .or_else(|| inherited.and_then(|value| value.thinking_mode.clone())),
            speed_mode: requested_mode(&requested_selection.speed_mode)
                .or_else(|| inherited.and_then(|value| value.speed_mode.clone())),
            verbosity: requested_mode(&requested_selection.verbosity)
                .or_else(|| inherited.and_then(|value| value.verbosity.clone())),
            thinking: None,
            request_override: Default::default(),
            system: None,
            temperature: None,
            max_output_tokens: None,
        };
        options.request_override.set_parallel_tool_calls(
            requested_selection
                .parallel_tool_calls
                .or_else(|| inherited.and_then(|value| value.parallel_tool_calls)),
        );
        self.apply_model_mode_requests(&mut options)?;
        Ok(options)
    }

    /// Drain every pending steer message (non-blocking) and append each as
    /// a run before the next model run. Ordinary inputs become a User run
    /// (marker + content parts). A background-operation notification steer is
    /// a pure re-trigger: `record_background_event` already appended the hook
    /// to its assistant launch run (or committed a Runtime ingress for
    /// launch-less scheduled work), so this only reloads the session. Because
    /// this drain runs between provider/tool parts, the hook becomes the next
    /// input the model sees without interrupting the active part.
    pub(in crate::session::manager) async fn drain_steer_input(
        &self,
        mut session: Session,
        steer_rx: &mut mpsc::Receiver<Vec<TypedContent>>,
        _options: &SessionRunOptions,
        _state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        loop {
            let parts = match steer_rx.try_recv() {
                Ok(parts) => parts,
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(session);
                }
            };
            let user_parts = parts
                .iter()
                .filter(|content| !matches!(content, TypedContent::SystemNotification(_)))
                .map(|content| {
                    new_part_from_content("text", PartRole::User, content, PartState::Completed)
                })
                .collect::<Result<Vec<_>, _>>()?;
            if !user_parts.is_empty() {
                let outcome = self
                    .store
                    .submit_user_run(session.id, user_parts, None)
                    .await?;
                if outcome.created {
                    let mut projected = session.parts().to_vec();
                    projected.extend(outcome.parts);
                    session.install_projected_parts(projected);
                }
            }
            if parts
                .iter()
                .any(|content| matches!(content, TypedContent::SystemNotification(_)))
            {
                // The settle committed the notification before this steer was
                // sent. Reload so it is in the projection; the notification
                // cursor requests the next provider round. Nothing is
                // committed here.
                session = self.store.load_session(session.id).await?;
            }
        }
    }
}
