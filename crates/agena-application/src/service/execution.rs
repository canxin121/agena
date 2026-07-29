impl ApplicationService {
    pub async fn assert_session_version(
        &self,
        session_id: i64,
        expected_version: i64,
    ) -> ApplicationResult<()> {
        let existing = self.ensure_session_model(session_id).await?;
        if existing.version == expected_version {
            return Ok(());
        }

        Err(ApplicationError::conflict(format!(
            "session version mismatch for {session_id}: expected {expected_version}, current {}",
            existing.version
        )))
    }

    pub async fn latest_session_event_seq(
        &self,
        session_queries: &dyn agena_runtime::SessionQueryService,
        session_id: i64,
    ) -> ApplicationResult<Option<i64>> {
        self.ensure_session_exists(session_id).await?;
        session_queries
            .latest_event_seq(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))
    }

    pub async fn list_session_events_after(
        &self,
        events: &dyn agena_runtime::RuntimeEventQueryService,
        session_id: i64,
        after_seq: i64,
        limit: Option<u64>,
    ) -> ApplicationResult<Vec<agena_runtime::RuntimeEvent>> {
        self.ensure_session_exists(session_id).await?;
        let limit = normalize_limit(limit) as usize;
        let events = events
            .list_events(
                &agena_domain::EventFilter {
                    scope: agena_domain::EventScope::Session { session_id },
                    kinds: None,
                    since_seq_global: None,
                },
                agena_runtime::RuntimeEventRange {
                    after_seq_global: after_seq,
                    limit,
                },
            )
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;
        Ok(events)
    }

    pub async fn resolve_run_options(
        &self,
        provider_catalog: &dyn ProviderCatalog,
        default_model: Option<ModelRef>,
        execution_control: &dyn agena_runtime::SessionExecutionControl,
        session_id: i64,
        request: SessionRunOptionsRequest,
    ) -> ApplicationResult<agena_runtime::SessionRunOptions> {
        self.ensure_session_exists(session_id).await?;

        let model = match request.model {
            Some(model) => {
                let model = model_ref_from_wire(model)?;
                ensure_provider_exists(provider_catalog, &model)?;
                model
            }
            None => {
                let selection = execution_control
                    .selected_model(session_id)
                    .await
                    .map_err(|error| ApplicationError::internal(error.to_string()))?;
                match selection {
                    Some(model) => {
                        ensure_provider_exists(provider_catalog, &model)?;
                        model
                    }
                    None => default_model.ok_or_else(|| {
                    ApplicationError::bad_request(
                        "model is required when neither the request, session, nor global default specifies one",
                    )
                })?,
                }
            }
        };

        if let Some(temperature) = request.temperature
            && !temperature.is_finite()
        {
            return Err(ApplicationError::bad_request(
                "temperature must be a finite number",
            ));
        }
        if matches!(request.max_output_tokens, Some(0)) {
            return Err(ApplicationError::bad_request(
                "max_output_tokens must be greater than zero",
            ));
        }
        let mut thinking_mode = non_empty(request.thinking_mode.as_deref()).map(ToOwned::to_owned);
        let mut speed_mode = non_empty(request.speed_mode.as_deref()).map(ToOwned::to_owned);
        let requested_verbosity =
            non_empty(request.verbosity.as_deref()).map(|value| value.trim().to_ascii_lowercase());
        let requested_parallel_tool_calls = request.parallel_tool_calls;

        let provider_options = provider_catalog
            .model_execution_options(&model)
            .map_err(provider_catalog_error)?;
        let resolved_adapter_id = model
            .adapter_id
            .clone()
            .or(provider_options.default_adapter);

        let thinking_modes = provider_options.thinking_modes;
        if thinking_mode.is_none() {
            thinking_mode = thinking_modes.iter().find_map(|mode| {
                mode.is_default
                    .then(|| mode.selector().map(|selector| selector.into_owned()))
                    .flatten()
            });
        }
        let (thinking, thinking_request_override) =
            if let Some(thinking_mode_name) = thinking_mode.as_deref() {
                let thinking_mode = thinking_modes
                    .iter()
                    .find(|mode| mode.selector().as_deref() == Some(thinking_mode_name))
                    .ok_or_else(|| {
                        ApplicationError::bad_request(format!(
                            "model `{}` has no think mode `{thinking_mode_name}`",
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

        let speed_modes = provider_options.speed_modes;
        if speed_mode.is_none() {
            speed_mode = speed_modes
                .iter()
                .find(|(_, mode)| mode.is_default)
                .map(|(name, _)| name.clone());
        }
        let speed_request_override = if let Some(speed_mode_name) = speed_mode.as_deref() {
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                ApplicationError::bad_request(format!(
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
        let metadata = provider_options.metadata;
        if requested_parallel_tool_calls.is_some()
            && !metadata.supports_parallel_tool_calls_for_model()
        {
            return Err(ApplicationError::bad_request(format!(
                "model `{}` does not support parallel tool calls",
                model
            )));
        }
        let supported_verbosity_levels =
            metadata.supported_verbosity_levels_for_model(&model.model_id);
        if let Some(verbosity) = requested_verbosity.as_deref()
            && !metadata.supports_verbosity_level_for_model(&model.model_id, verbosity)
        {
            return Err(ApplicationError::bad_request(format!(
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

        Ok(agena_runtime::SessionRunOptions {
            model,
            thinking_mode,
            speed_mode,
            verbosity,
            thinking,
            request_override,
            system: non_empty(request.system.as_deref()).map(ToOwned::to_owned),
            temperature,
            max_output_tokens: request.max_output_tokens,
        })
    }

    pub async fn session_execution_resource(
        &self,
        execution_control: &dyn agena_runtime::SessionExecutionControl,
        session_queries: &dyn agena_runtime::SessionQueryService,
        session_id: i64,
    ) -> ApplicationResult<SessionExecutionResource> {
        let session_resource = self.get_session(session_id).await?.ok_or_else(|| {
            ApplicationError::internal("session disappeared while loading execution state")
        })?;
        let context = session_queries
            .execution_context(session_id)
            .await
            .map_err(|error| ApplicationError::internal(error.to_string()))?;

        let scheduler_jobs = list_scheduled_jobs(execution_control).await;
        let pending_interactive_requests =
            pending_interactive_requests(session_queries, session_id).await?;
        // Workflow readiness and execution liveness are separate facts. A
        // rewind can be ready for a model while no task exists, so cancellation
        // and steering must consult the registry-backed lifecycle only.
        let active_execution = execution_control
            .active_execution(session_id)
            .await
            .and_then(|lifecycle| match lifecycle {
                agena_domain::ExecutionLifecycle::Active {
                    execution_id,
                    phase,
                } => Some(ActiveExecutionResource {
                    execution_id: execution_id.0,
                    phase: execution_phase_from_domain(phase),
                }),
                agena_domain::ExecutionLifecycle::Terminal { .. } => None,
            });

        Ok(SessionExecutionResource {
            session: session_resource,
            workflow_state: workflow_state_from_domain(context.workflow_state),
            active_execution,
            latest_event_seq: self
                .latest_session_event_seq(session_queries, session_id)
                .await?,
            automation: session_automation_resource(&scheduler_jobs, session_id),
            execution: SessionExecutionContextResource {
                agent_id: context.agent_id,
                execution_access: execution_access_from_domain(context.execution_access),
                effective_permission: permission_config_resource_from_domain(
                    &context.effective_permission,
                ),
                permission_ceiling: permission_config_resource_from_domain(
                    &context.permission_ceiling,
                ),
                model_provider_id: context.model_provider_id,
                model_adapter_id: context.model_adapter_id,
                model_id: context.model_id,
                model_thinking_mode: context.model_thinking_mode,
                model_speed_mode: context.model_speed_mode,
                model_verbosity: context.model_verbosity,
                model_parallel_tool_calls: context.model_parallel_tool_calls,
                effective_workspace_root: context.effective_workspace_root,
                task_id: context.task_id,
                subtask_status: context.subtask_status.map(subtask_status_from_domain),
                subtask_started_at: context.subtask_started_at,
                subtask_finished_at: context.subtask_finished_at,
                subtask_error: context.subtask_error,
            },
            pending_interactive_requests,
            usage: session_usage_resource(session_queries, session_id).await?,
        })
    }
}

fn model_ref_from_wire(value: agena_api::resource::ModelRef) -> ApplicationResult<ModelRef> {
    let result = match value.adapter_id {
        Some(adapter_id) => {
            ModelRef::try_new_with_adapter(value.provider_id, adapter_id, value.model_id)
        }
        None => ModelRef::try_new(value.provider_id, value.model_id),
    };
    result
        .map_err(|error| ApplicationError::bad_request(format!("invalid model reference: {error}")))
}

async fn session_usage_resource(
    session_queries: &dyn agena_runtime::SessionQueryService,
    session_id: i64,
) -> ApplicationResult<SessionUsageResource> {
    let usage = session_queries
        .session_usage(session_id)
        .await
        .map_err(|error| ApplicationError::internal(error.to_string()))?;
    Ok(SessionUsageResource {
        measured_prompt_tokens: usage.measured_prompt_tokens,
        current_tokens: usage.current_tokens,
        projected_tokens: usage.projected_tokens,
        limit_tokens: usage.limit_tokens,
        limit_basis: usage.limit_basis.map(|basis| match basis {
            agena_domain::SessionUsageLimitBasis::ContextWindow => {
                agena_api::resource::SessionUsageLimitBasis::ContextWindow
            }
            agena_domain::SessionUsageLimitBasis::PromptThreshold => {
                agena_api::resource::SessionUsageLimitBasis::PromptThreshold
            }
        }),
        reserved_tokens: usage.reserved_tokens,
        model_context_window_tokens: usage.model_context_window_tokens,
        model_max_input_tokens: usage.model_max_input_tokens,
        model_max_output_tokens: usage.model_max_output_tokens,
    })
}

const fn execution_phase_from_domain(
    value: agena_domain::ExecutionPhase,
) -> agena_api::resource::ExecutionPhase {
    match value {
        agena_domain::ExecutionPhase::Starting => agena_api::resource::ExecutionPhase::Starting,
        agena_domain::ExecutionPhase::PreparingModel => {
            agena_api::resource::ExecutionPhase::PreparingModel
        }
        agena_domain::ExecutionPhase::StreamingModel => {
            agena_api::resource::ExecutionPhase::StreamingModel
        }
        agena_domain::ExecutionPhase::ExecutingTools => {
            agena_api::resource::ExecutionPhase::ExecutingTools
        }
        agena_domain::ExecutionPhase::Cancelling => agena_api::resource::ExecutionPhase::Cancelling,
    }
}

const fn workflow_state_from_domain(
    value: agena_domain::WorkflowState,
) -> agena_api::resource::WorkflowState {
    match value {
        agena_domain::WorkflowState::Quiescent => agena_api::resource::WorkflowState::Quiescent,
        agena_domain::WorkflowState::ReadyForModel => {
            agena_api::resource::WorkflowState::ReadyForModel
        }
        agena_domain::WorkflowState::ToolPending => agena_api::resource::WorkflowState::ToolPending,
        agena_domain::WorkflowState::Blocked => agena_api::resource::WorkflowState::Blocked,
    }
}

fn resolve_mode_request_override(
    request_override: &ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<String, ModelSpeedModeRequestOverride>,
    resolved_adapter_id: Option<&AdapterId>,
) -> ModelSpeedModeRequestOverride {
    let mut merged = request_override.clone();
    if let Some(adapter_id) = resolved_adapter_id.map(AsRef::<str>::as_ref)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
}

fn ensure_provider_exists(
    provider_catalog: &dyn ProviderCatalog,
    model: &ModelRef,
) -> ApplicationResult<()> {
    if provider_catalog.contains_provider(&model.provider_id) {
        Ok(())
    } else {
        Err(ApplicationError::bad_request(format!(
            "provider not configured: {}",
            model.provider_id
        )))
    }
}

fn provider_catalog_error(error: ProviderCatalogError) -> ApplicationError {
    match error {
        ProviderCatalogError::InvalidRequest(message) => ApplicationError::BadRequest(message),
        ProviderCatalogError::NotFound(message) => ApplicationError::NotFound(message),
        ProviderCatalogError::Operation(message) => ApplicationError::Internal(message),
    }
}

pub async fn list_scheduled_jobs(
    execution_control: &dyn agena_runtime::SessionExecutionControl,
) -> Vec<agena_scheduler::ScheduledJob> {
    execution_control.list_scheduled_jobs().await
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
        status: match run.status {
            agena_scheduler::JobRunStatus::Submitted => {
                agena_api::resource::ScheduledJobRunStatus::Submitted
            }
            agena_scheduler::JobRunStatus::Skipped => {
                agena_api::resource::ScheduledJobRunStatus::Skipped
            }
            agena_scheduler::JobRunStatus::Failed => {
                agena_api::resource::ScheduledJobRunStatus::Failed
            }
        },
        session_id: run.session_id,
        error_message: run.error_message,
    }
}

async fn pending_interactive_requests(
    session_queries: &dyn agena_runtime::SessionQueryService,
    session_id: i64,
) -> ApplicationResult<Vec<PendingInteractiveRequestResource>> {
    let contexts = session_queries
        .pending_interactive_requests(session_id)
        .await
        .map_err(|error| ApplicationError::internal(error.to_string()))?;
    Ok(contexts
        .into_iter()
        .map(|context| PendingInteractiveRequestResource {
            session_id: context.session_id,
            parent_session_id: context.parent_session_id,
            task_id: context.task_id,
            request: pending_interactive_request_from_domain(context.request),
        })
        .collect())
}

pub(crate) const fn execution_access_from_domain(
    value: agena_domain::ExecutionAccess,
) -> agena_api::resource::ExecutionAccess {
    match value {
        agena_domain::ExecutionAccess::Inherit => agena_api::resource::ExecutionAccess::Inherit,
        agena_domain::ExecutionAccess::ReadOnly => agena_api::resource::ExecutionAccess::ReadOnly,
    }
}

/// Shared runtime-to-wire projection for interactive requests. Both the
/// session status surface and message-part history use this exact contract.
pub(crate) fn pending_interactive_request_from_domain(
    value: agena_domain::PendingInteractiveRequest,
) -> agena_api::resource::PendingInteractiveRequest {
    use agena_api::resource as wire;
    match value {
        agena_domain::PendingInteractiveRequest::Permission { request } => {
            wire::PendingInteractiveRequest::Permission {
                request: wire::PermissionRequest {
                    request_id: request.request_id,
                    session_id: request.session_id,
                    action: permission_action_from_domain(request.action),
                    related_actions: request
                        .related_actions
                        .into_iter()
                        .map(permission_action_from_domain)
                        .collect(),
                    requested_actions: request
                        .requested_actions
                        .into_iter()
                        .map(permission_action_from_domain)
                        .collect(),
                    reason: request.reason,
                    explanation: request.explanation,
                    source: request.source,
                    scope: request.scope.map(permission_scope_from_domain),
                    operator: request.operator,
                    risk: match request.risk {
                        agena_domain::PermissionRiskLevel::Low => wire::PermissionRiskLevel::Low,
                        agena_domain::PermissionRiskLevel::Medium => {
                            wire::PermissionRiskLevel::Medium
                        }
                        agena_domain::PermissionRiskLevel::High => wire::PermissionRiskLevel::High,
                        agena_domain::PermissionRiskLevel::Critical => {
                            wire::PermissionRiskLevel::Critical
                        }
                    },
                    trace: request
                        .trace
                        .into_iter()
                        .map(|step| wire::DecisionTraceStep {
                            source_kind: match step.source_kind {
                                agena_domain::PolicySourceKind::StaticPolicy => {
                                    wire::PolicySourceKind::StaticPolicy
                                }
                                agena_domain::PolicySourceKind::PersistedRule => {
                                    wire::PolicySourceKind::PersistedRule
                                }
                                agena_domain::PolicySourceKind::PluginAdvice => {
                                    wire::PolicySourceKind::PluginAdvice
                                }
                                agena_domain::PolicySourceKind::ManagedPolicy => {
                                    wire::PolicySourceKind::ManagedPolicy
                                }
                            },
                            summary: step.summary,
                            source: step.source,
                            scope: step.scope.map(permission_scope_from_domain),
                            operator: step.operator,
                        })
                        .collect(),
                    created_at: request.created_at,
                },
            }
        }
        agena_domain::PendingInteractiveRequest::UserInput { request } => {
            wire::PendingInteractiveRequest::UserInput {
                request: wire::UserInputRequest {
                    request_id: request.request_id,
                    session_id: request.session_id,
                    title: request.title,
                    body_markdown: request.body_markdown,
                    kind: request.kind,
                    submit_label: request.submit_label,
                    cancel_label: request.cancel_label,
                    auto_resolution_ms: request.auto_resolution_ms,
                    questions: request
                        .questions
                        .into_iter()
                        .map(|question| wire::UserInputQuestion {
                            id: question.id,
                            header: question.header,
                            question: question.question,
                            options: question
                                .options
                                .into_iter()
                                .map(|option| wire::UserInputOption {
                                    label: option.label,
                                    description: option.description,
                                    preview_markdown: option.preview_markdown,
                                })
                                .collect(),
                            multiple: question.multiple,
                            allow_custom: question.allow_custom,
                        })
                        .collect(),
                    created_at: request.created_at,
                },
            }
        }
    }
}

pub(crate) fn permission_action_from_domain(
    value: agena_domain::PermissionAction,
) -> agena_api::resource::PermissionActionResource {
    match value {
        agena_domain::PermissionAction::Tool {
            tool_name,
            qualifier,
        } => agena_api::resource::PermissionActionResource::Tool {
            tool_name,
            qualifier,
        },
        agena_domain::PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => agena_api::resource::PermissionActionResource::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        },
        agena_domain::PermissionAction::NetworkAccess { target, host, port } => {
            agena_api::resource::PermissionActionResource::NetworkAccess { target, host, port }
        }
    }
}

pub(crate) const fn permission_scope_from_domain(
    value: agena_domain::PermissionScope,
) -> agena_api::resource::PermissionScope {
    match value {
        agena_domain::PermissionScope::Session => agena_api::resource::PermissionScope::Session,
        agena_domain::PermissionScope::Workspace => agena_api::resource::PermissionScope::Workspace,
        agena_domain::PermissionScope::Global => agena_api::resource::PermissionScope::Global,
    }
}

pub(crate) fn permission_config_resource_from_domain(
    value: &agena_domain::PermissionConfig,
) -> agena_api::resource::PermissionConfigResource {
    use agena_api::resource::{
        NetworkPermissionConfigResource, PathAccessRuleResource, PathPermissionConfigResource,
        PermissionConfigResource, ToolPermissionConfigResource, ToolPermissionRulesResource,
    };

    PermissionConfigResource {
        path: value
            .path
            .as_ref()
            .map(|path| PathPermissionConfigResource {
                workspace: path
                    .workspace
                    .as_ref()
                    .map(path_access_modes_resource_from_domain),
                external: path
                    .external
                    .as_ref()
                    .map(path_access_modes_resource_from_domain),
                rules: path
                    .rules
                    .iter()
                    .map(|(pattern, rule)| {
                        let rule = match rule {
                            agena_domain::PathAccessRuleConfig::Modes(modes) => {
                                PathAccessRuleResource::Modes(
                                    path_access_modes_resource_from_domain(modes),
                                )
                            }
                            agena_domain::PathAccessRuleConfig::Shorthand(value) => {
                                PathAccessRuleResource::Shorthand(value.clone())
                            }
                        };
                        (pattern.clone(), rule)
                    })
                    .collect(),
            }),
        network: value
            .network
            .as_ref()
            .map(|network| NetworkPermissionConfigResource {
                internet: network.internet.map(permission_mode_resource_from_domain),
                private: network.private.map(permission_mode_resource_from_domain),
                loopback: network.loopback.map(permission_mode_resource_from_domain),
                rules: network
                    .rules
                    .iter()
                    .map(|(pattern, mode)| {
                        (pattern.clone(), permission_mode_resource_from_domain(*mode))
                    })
                    .collect(),
            }),
        tools: value
            .tools
            .as_ref()
            .map(|tools| ToolPermissionConfigResource {
                default: tools.default.map(permission_mode_resource_from_domain),
                tags: tools
                    .tags
                    .iter()
                    .map(|(name, mode)| (name.clone(), permission_mode_resource_from_domain(*mode)))
                    .collect(),
                names: tools
                    .names
                    .iter()
                    .chain(tools.plugin.iter())
                    .map(|(name, mode)| (name.clone(), permission_mode_resource_from_domain(*mode)))
                    .collect(),
                rules: tools
                    .rules
                    .iter()
                    .map(|(name, rules)| {
                        let rules = match rules {
                            agena_domain::ToolPermissionRules::Mode(mode) => {
                                ToolPermissionRulesResource::Mode(
                                    permission_mode_resource_from_domain(*mode),
                                )
                            }
                            agena_domain::ToolPermissionRules::Ordered(rules) => {
                                ToolPermissionRulesResource::Ordered(
                                    rules
                                        .iter()
                                        .map(|(pattern, mode)| {
                                            (
                                                pattern.clone(),
                                                permission_mode_resource_from_domain(*mode),
                                            )
                                        })
                                        .collect(),
                                )
                            }
                        };
                        (name.clone(), rules)
                    })
                    .collect(),
            }),
    }
}

fn path_access_modes_resource_from_domain(
    value: &agena_domain::PathAccessModes,
) -> agena_api::resource::PathAccessModesResource {
    agena_api::resource::PathAccessModesResource {
        read: value.read.map(permission_mode_resource_from_domain),
        write: value.write.map(permission_mode_resource_from_domain),
    }
}

const fn permission_mode_resource_from_domain(
    value: agena_domain::PermissionMode,
) -> agena_api::resource::PermissionMode {
    match value {
        agena_domain::PermissionMode::Allow => agena_api::resource::PermissionMode::Allow,
        agena_domain::PermissionMode::Ask => agena_api::resource::PermissionMode::Ask,
        agena_domain::PermissionMode::Deny => agena_api::resource::PermissionMode::Deny,
    }
}

use super::{
    ActiveExecutionResource, AdapterId, ApplicationError, ApplicationResult, ApplicationService,
    ModelRef, ModelSpeedModeRequestOverride, PendingInteractiveRequestResource,
    ScheduledJobResource, ScheduledJobRunResource, SessionAutomationResource,
    SessionExecutionContextResource, SessionExecutionResource, SessionRunOptionsRequest,
    SessionUsageResource, non_empty, normalize_limit, sessions::subtask_status_from_domain,
};
use agena_provider::{ProviderCatalog, ProviderCatalogError};
