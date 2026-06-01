use super::*;

impl ApiService {
    pub async fn assert_session_version(
        &self,
        session_id: i64,
        expected_version: i64,
    ) -> ApiResult<()> {
        let existing = self.ensure_session_model(session_id).await?;
        if existing.version == expected_version {
            return Ok(());
        }

        Err(ApiError::conflict(format!(
            "session version mismatch for {session_id}: expected {expected_version}, current {}",
            existing.version
        )))
    }

    pub async fn latest_session_event_seq(
        &self,
        manager: &SessionManager,
        session_id: i64,
    ) -> ApiResult<Option<i64>> {
        self.ensure_session_exists(session_id).await?;
        let events = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;
        Ok(events.iter().map(|e| e.meta.seq_global).max())
    }

    pub async fn list_session_events_after(
        &self,
        manager: &SessionManager,
        session_id: i64,
        after_seq: i64,
        limit: Option<u64>,
    ) -> ApiResult<Vec<agena::event::DomainEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(limit) as usize;
        let mut events = manager
            .list_session_events(session_id)
            .await
            .map_err(api_error_from_app)?;
        events.retain(|e| e.meta.seq_global > after_seq);
        events.truncate(limit);
        Ok(events)
    }

    pub async fn resolve_run_options(
        &self,
        provider_registry: &ProviderRegistry,
        default_model: Option<ModelRef>,
        manager: &SessionManager,
        session_id: i64,
        request: SessionRunOptionsRequest,
    ) -> ApiResult<agena::session::SessionRunOptions> {
        self.ensure_session_exists(session_id).await?;

        let model = match request.model {
            Some(model) => {
                ensure_provider_exists(provider_registry, &model)?;
                model
            }
            None => {
                let session = manager
                    .get_session(session_id)
                    .await
                    .map_err(api_error_from_app)?;
                let selection = session.runtime().effective_model_ref().map_err(|error| {
                    ApiError::internal(format!(
                        "session {session_id} contains invalid persisted model reference: {error}"
                    ))
                })?;
                match selection {
                    Some(model) => {
                        ensure_provider_exists(provider_registry, &model)?;
                        model
                    }
                    None => default_model.ok_or_else(|| {
                    ApiError::bad_request(
                        "model is required when neither the request, session, nor global default specifies one",
                    )
                })?,
                }
            }
        };

        if let Some(temperature) = request.temperature
            && !temperature.is_finite()
        {
            return Err(ApiError::bad_request("temperature must be a finite number"));
        }
        if matches!(request.max_output_tokens, Some(0)) {
            return Err(ApiError::bad_request(
                "max_output_tokens must be greater than zero",
            ));
        }
        let thinking_mode = non_empty(request.thinking_mode.as_deref()).map(ToOwned::to_owned);
        let speed_mode = non_empty(request.speed_mode.as_deref()).map(ToOwned::to_owned);
        let requested_verbosity =
            non_empty(request.verbosity.as_deref()).map(|value| value.trim().to_ascii_lowercase());
        let requested_parallel_tool_calls = request.parallel_tool_calls;

        let resolved_adapter_id = model.adapter_id.clone().or_else(|| {
            provider_registry
                .get(model.provider_id.as_str())
                .and_then(|provider| provider.default_adapter().cloned())
        });

        let (thinking, thinking_request_override) =
            if let Some(thinking_mode_name) = thinking_mode.as_deref() {
                let thinking_modes = provider_registry
                    .model_thinking_modes(&model)
                    .map_err(api_error_from_app)?;
                let thinking_mode = thinking_modes.get(thinking_mode_name).ok_or_else(|| {
                    ApiError::bad_request(format!(
                        "model `{}` has no thinking mode `{thinking_mode_name}`",
                        model
                    ))
                })?;
                (
                    thinking_mode.thinking.clone(),
                    resolve_mode_request_override(
                        &thinking_mode.request_override,
                        &thinking_mode.adapter_overrides,
                        resolved_adapter_id.as_ref(),
                    ),
                )
            } else {
                (None, ModelSpeedModeRequestOverride::default())
            };

        let speed_request_override = if let Some(speed_mode_name) = speed_mode.as_deref() {
            let speed_modes = provider_registry
                .model_speed_modes(&model)
                .map_err(api_error_from_app)?;
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                ApiError::bad_request(format!(
                    "model `{}` has no speed mode `{speed_mode_name}`",
                    model
                ))
            })?;
            resolve_mode_request_override(
                &speed_mode.request_override,
                &speed_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            )
        } else {
            ModelSpeedModeRequestOverride::default()
        };

        let mut request_override = thinking_request_override.merged_with(&speed_request_override);
        request_override.set_parallel_tool_calls(requested_parallel_tool_calls);
        let metadata = provider_registry
            .model_metadata(&model)
            .map_err(api_error_from_app)?;
        if requested_parallel_tool_calls.is_some()
            && !metadata.supports_parallel_tool_calls_for_model()
        {
            return Err(ApiError::bad_request(format!(
                "model `{}` does not support parallel tool calls",
                model
            )));
        }
        let supported_verbosity_levels =
            metadata.supported_verbosity_levels_for_model(&model.model_id);
        if let Some(verbosity) = requested_verbosity.as_deref()
            && !metadata.supports_verbosity_level_for_model(&model.model_id, verbosity)
        {
            return Err(ApiError::bad_request(format!(
                "model `{}` does not support verbosity `{verbosity}`; supported values: {}",
                model,
                if supported_verbosity_levels.is_empty() {
                    "none".to_owned()
                } else {
                    supported_verbosity_levels.join(", ")
                }
            )));
        }
        let verbosity = requested_verbosity.or_else(|| {
            metadata
                .default_verbosity
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_ascii_lowercase)
        });
        let temperature = request
            .temperature
            .or_else(|| metadata.parsed_default_temperature());

        Ok(agena::session::SessionRunOptions::new(model)
            .with_thinking_mode(thinking_mode)
            .with_speed_mode(speed_mode)
            .with_verbosity(verbosity)
            .with_thinking(thinking)
            .with_request_override(request_override)
            .with_system(non_empty(request.system.as_deref()).map(ToOwned::to_owned))
            .with_temperature(temperature)
            .with_max_output_tokens(request.max_output_tokens)
            .with_agent_profile(non_empty(request.agent_profile.as_deref()).map(ToOwned::to_owned)))
    }

    pub async fn session_execution_resource(
        &self,
        manager: &SessionManager,
        session: &Session,
    ) -> ApiResult<SessionExecutionResource> {
        let session_resource = self.get_session(session.id).await?.ok_or_else(|| {
            ApiError::internal("session disappeared while loading execution state")
        })?;

        let scheduler_jobs = list_scheduled_jobs(manager).await;
        let pending_interactive_requests = pending_interactive_requests(session);

        Ok(SessionExecutionResource {
            session: session_resource,
            blocked: session.blocked(),
            run_state: SessionRunState::from(session.status()),
            latest_event_seq: self.latest_session_event_seq(manager, session.id).await?,
            automation: session_automation_resource(&scheduler_jobs, session.id),
            execution: SessionExecutionContextResource {
                agent_profile: session.runtime().execution.selection.agent.clone(),
                active_skill_name: session.runtime().execution.active_skill_name.clone(),
                system_prompt_override: session.runtime().execution.system_prompt_override.clone(),
                effective_permission: session.runtime().execution.effective_permission.clone(),
                model_provider_id: session.runtime().execution.selection.provider.clone(),
                model_adapter_id: session.runtime().execution.selection.adapter.clone(),
                model_id: session.runtime().execution.selection.model.clone(),
                model_thinking_mode: session.runtime().execution.selection.thinking_mode.clone(),
                model_speed_mode: session.runtime().execution.selection.speed_mode.clone(),
                model_verbosity: session.runtime().execution.selection.verbosity.clone(),
                model_parallel_tool_calls: session
                    .runtime()
                    .execution
                    .selection
                    .parallel_tool_calls,
                effective_workspace_root: session
                    .runtime()
                    .effective_workspace_root()
                    .map(|path| path.display().to_string()),
                task_id: session.runtime().execution.task_id.clone(),
            },
            pending_interactive_requests: pending_interactive_requests.clone(),
            pending_permission_requests: pending_permission_requests(
                pending_interactive_requests.as_slice(),
            ),
            pending_user_input_requests: pending_user_input_requests(
                pending_interactive_requests.as_slice(),
            ),
            usage: session_usage_resource(manager, session).map_err(api_error_from_app)?,
        })
    }
}

fn session_usage_resource(
    manager: &SessionManager,
    session: &Session,
) -> Result<SessionUsageResource, AppError> {
    let usage = manager.session_usage(session)?;
    Ok(SessionUsageResource {
        measured_prompt_tokens: usage.measured_prompt_tokens,
        current_tokens: usage.current_tokens,
        projected_tokens: usage.projected_tokens,
        limit_tokens: usage.limit_tokens,
        limit_basis: usage.limit_basis.map(Into::into),
        reserved_tokens: usage.reserved_tokens,
        model_context_window_tokens: usage.model_context_window_tokens,
        model_max_input_tokens: usage.model_max_input_tokens,
        model_max_output_tokens: usage.model_max_output_tokens,
    })
}

fn resolve_mode_request_override(
    request_override: &ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<String, ModelSpeedModeRequestOverride>,
    resolved_adapter_id: Option<&AdapterId>,
) -> ModelSpeedModeRequestOverride {
    let mut merged = request_override.clone();
    if let Some(adapter_id) = resolved_adapter_id.map(AdapterId::as_str)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
}

fn ensure_provider_exists(provider_registry: &ProviderRegistry, model: &ModelRef) -> ApiResult<()> {
    if provider_registry.get(model.provider_id.as_str()).is_some() {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "provider not configured: {}",
            model.provider_id
        )))
    }
}

pub async fn list_scheduled_jobs(manager: &SessionManager) -> Vec<agena_scheduler::ScheduledJob> {
    let executor = manager.tool_executor();
    let Some(scheduler) = executor.scheduler().cloned() else {
        return Vec::new();
    };
    scheduler.list().await
}

fn session_automation_resource(
    jobs: &[agena_scheduler::ScheduledJob],
    session_id: i64,
) -> Option<SessionAutomationResource> {
    let mut jobs = jobs
        .iter()
        .filter(|job| job.owner_session_id == Some(session_id))
        .cloned()
        .collect::<Vec<_>>();
    if jobs.is_empty() {
        return None;
    }
    sort_jobs_for_display(&mut jobs);
    Some(SessionAutomationResource {
        job_count: jobs.len(),
        latest_job: jobs.into_iter().next().map(scheduled_job_resource),
    })
}

pub fn sort_jobs_for_display(jobs: &mut [agena_scheduler::ScheduledJob]) {
    jobs.sort_by(|left, right| {
        let left_last_run = left
            .last_run
            .as_ref()
            .map(|run| run.triggered_at.timestamp_millis());
        let right_last_run = right
            .last_run
            .as_ref()
            .map(|run| run.triggered_at.timestamp_millis());
        right_last_run
            .cmp(&left_last_run)
            .then_with(|| left.next_fire_at.cmp(&right.next_fire_at))
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });
}

pub fn scheduled_job_resource(job: agena_scheduler::ScheduledJob) -> ScheduledJobResource {
    let (kind, expression, at) = match job.kind {
        agena_scheduler::JobKind::Cron { expression, .. } => {
            ("cron".to_string(), Some(expression), None)
        }
        agena_scheduler::JobKind::Once { at } => ("once".to_string(), None, Some(at)),
    };
    ScheduledJobResource {
        id: job.id.to_string(),
        kind,
        expression,
        at,
        prompt: job.prompt,
        owner_session_id: job.owner_session_id,
        next_fire_at: job.next_fire_at,
        last_fired_at: job.last_fired_at,
        last_run: job.last_run.map(scheduled_job_run_resource),
    }
}

fn scheduled_job_run_resource(run: agena_scheduler::JobRunRecord) -> ScheduledJobRunResource {
    ScheduledJobRunResource {
        triggered_at: run.triggered_at,
        finished_at: run.finished_at,
        status: run.status,
        session_id: run.session_id,
        error_message: run.error_message,
    }
}

fn pending_interactive_requests(
    session: &Session,
) -> Vec<agena::message::PendingInteractiveRequest> {
    session.pending_interactive_requests()
}

fn pending_permission_requests(
    requests: &[agena::message::PendingInteractiveRequest],
) -> Vec<agena::permission::PermissionRequest> {
    requests
        .iter()
        .filter_map(|request| request.as_permission().cloned())
        .collect()
}

fn pending_user_input_requests(
    requests: &[agena::message::PendingInteractiveRequest],
) -> Vec<UserInputRequest> {
    requests
        .iter()
        .filter_map(|request| request.as_user_input().cloned())
        .collect()
}
