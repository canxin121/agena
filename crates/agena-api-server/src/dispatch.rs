//! Command/Query dispatch into the underlying [`agena::session::SessionManager`].
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport.

use std::collections::HashMap;

use crate::local_api::dto::MessageResource as HttpMessageResource;
use crate::local_api::{
    MessageListQuery, ModelCatalogEntryResource as HttpModelCatalogEntryResource,
    ModelCatalogResponse as HttpModelCatalogResponse, PartLoadMode as HttpPartLoadMode,
    PermissionRuleListQuery, PermissionRuleResource as HttpPermissionRuleResource,
    PermissionRuleWriteRequest, SessionAutomationResource as HttpSessionAutomationResource,
    SessionCreateRequest as HttpSessionCreateRequest,
    SessionExecutionContextResource as HttpSessionExecutionContextResource,
    SessionExecutionResource as HttpSessionExecutionResource,
    SessionGoalResource as HttpSessionGoalResource, SessionListQuery, SessionReplaceRequest,
    SessionResource as HttpSessionResource, SessionRunOptionsRequest, WorkspaceListQuery,
    WorkspaceResolveRequest, WorkspaceResource as HttpWorkspaceResource, WorkspaceWriteRequest,
};
use agena::event::{EventStore, StoreRange};
use agena::{
    event::EventKind,
    session::{
        SessionContinueRequest, SessionPermissionReplyRequest, SessionUserInputReplyRequest,
        SessionUserTurnRequest,
    },
};
use agena_api::{
    commands::{
        CancelTurnParams, ClearSessionGoalParams, Command, CommandResult,
        CompleteSessionGoalParams, ContinueRunParams, CreateSessionGoalParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams, ImportSessionParams,
        ListRewindCheckpointsParams, ListSessionTreeParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, SetSessionGoalParams, SubmitTurnParams,
        UnrewindSessionParams, UpdateSessionParams, UpdateWorkspaceParams,
        UpsertPermissionRuleParams,
    },
    pagination::{PageInfo, PaginatedResponse, normalize_limit},
    queries::{
        GetMessageParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
        ListEventsParams, ListMessagesParams, ListPermissionRulesParams, ListProviderModelsParams,
        ListSessionsParams, ListWorkspacesParams, PaginatedEvents, Query, QueryResult,
    },
    resource::{
        ModelCatalogEntryResource, ModelCatalogResponse, ProviderAdapterSummaryResource,
        ProviderModelsResponse, ProviderSummaryResource, RunOptions, RuntimeAgentResource,
        RuntimeAgentsResource, RuntimeAutomationResource, RuntimeLspResource,
        RuntimeLspServerResource, RuntimeMcpResource, RuntimeMcpServerResource,
        RuntimeOperatorResource, RuntimeSessionCacheResource, RuntimeSkillResource,
        RuntimeSkillsResource, RuntimeStatusResponse, RuntimeTaskResource,
        SessionAutomationResource, SessionExecutionContextResource, SessionExecutionResource,
        SessionGoalResource, SessionResource, SessionRunState, WorkspaceResource,
    },
};

use crate::{error::ServerError, state::AppState};

async fn run_options_to_core(
    state: &AppState,
    session_id: i64,
    options: &RunOptions,
) -> Result<agena::session::SessionRunOptions, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let default_model = configured_default_model(&snapshot)?;
    let manager = state.session_manager()?;
    state
        .service()
        .resolve_run_options(
            snapshot.provider_registry().as_ref(),
            default_model,
            manager.as_ref(),
            session_id,
            SessionRunOptionsRequest {
                model: options.model.clone(),
                variant: options.variant.clone(),
                agent_profile: options.agent_profile.clone(),
                system: options.system.clone(),
                temperature: options.temperature,
                max_output_tokens: options.max_output_tokens,
                max_turn_loops: options.max_turn_loops,
            },
        )
        .await
        .map_err(server_error_from_http)
}

fn configured_default_model(
    snapshot: &agena::runtime::RuntimeSnapshot,
) -> Result<Option<agena::model::ModelRef>, ServerError> {
    let default = &snapshot.config_resolution().config.default;
    let Some(provider_id) = default.provider.as_deref() else {
        return Ok(None);
    };
    let registry = snapshot.provider_registry();
    registry
        .resolve_model_selection(
            provider_id,
            default.adapter.as_deref(),
            default.model.as_deref(),
        )
        .map(Some)
        .map_err(ServerError::Core)
}

fn server_error_from_http(error: crate::local_api::ApiError) -> ServerError {
    match error.status_code() {
        axum::http::StatusCode::BAD_REQUEST => ServerError::BadRequest(error.message().to_owned()),
        axum::http::StatusCode::NOT_FOUND => ServerError::NotFound(error.message().to_owned()),
        axum::http::StatusCode::CONFLICT => ServerError::Conflict(error.message().to_owned()),
        axum::http::StatusCode::SERVICE_UNAVAILABLE => {
            ServerError::ServiceUnavailable(error.message().to_owned())
        }
        _ => ServerError::Internal(error.message().to_owned()),
    }
}

fn workspace_from_http(value: HttpWorkspaceResource) -> WorkspaceResource {
    WorkspaceResource {
        id: value.id,
        path: value.path,
        created_at: value.created_at,
        updated_at: value.updated_at,
        session_count: value.session_count,
    }
}

fn session_from_http(value: HttpSessionResource) -> SessionResource {
    SessionResource {
        id: value.id,
        parent_id: value.parent_id,
        depth: value.depth,
        root_id: value.root_id,
        workspace_id: value.workspace_id,
        title: value.title,
        version: value.version,
        is_subagent: value.is_subagent,
        created_at: value.created_at,
        updated_at: value.updated_at,
        message_count: value.message_count,
        child_session_count: value.child_session_count,
        last_message_at: value.last_message_at,
        goal: value.goal.map(session_goal_from_http),
    }
}

fn session_goal_from_http(value: HttpSessionGoalResource) -> SessionGoalResource {
    SessionGoalResource {
        id: value.id,
        session_id: value.session_id,
        objective: value.objective,
        status: value.status,
        token_budget: value.token_budget,
        tokens_used: value.tokens_used,
        time_used_seconds: value.time_used_seconds,
        created_at: value.created_at,
        updated_at: value.updated_at,
        completed_at: value.completed_at,
    }
}

fn message_resource_from_http(value: HttpMessageResource) -> agena_api::resource::MessageResource {
    agena_api::resource::MessageResource {
        id: value.id,
        session_id: value.session_id,
        role: value.role,
        state: value.state,
        created_at: value.created_at,
        updated_at: value.updated_at,
        metadata: value.metadata,
        usage: value.usage,
        finish: value.finish,
        part_count: value.part_count,
        parts: value.parts,
    }
}

fn part_load_mode_to_http(mode: agena_api::resource::PartLoadMode) -> HttpPartLoadMode {
    match mode {
        agena_api::resource::PartLoadMode::None => HttpPartLoadMode::None,
        agena_api::resource::PartLoadMode::Summary => HttpPartLoadMode::Summary,
        agena_api::resource::PartLoadMode::Full => HttpPartLoadMode::Full,
    }
}

fn session_automation_from_http(value: HttpSessionAutomationResource) -> SessionAutomationResource {
    SessionAutomationResource {
        job_count: value.job_count,
        latest_job: value.latest_job.map(scheduled_job_from_http),
    }
}

fn scheduled_job_from_http(
    value: crate::local_api::ScheduledJobResource,
) -> agena_api::resource::ScheduledJobResource {
    agena_api::resource::ScheduledJobResource {
        id: value.id,
        kind: value.kind,
        expression: value.expression,
        at: value.at,
        prompt: value.prompt,
        owner_session_id: value.owner_session_id,
        next_fire_at: value.next_fire_at,
        last_fired_at: value.last_fired_at,
        last_run: value.last_run.map(scheduled_job_run_from_http),
    }
}

fn scheduled_job_run_from_http(
    value: crate::local_api::ScheduledJobRunResource,
) -> agena_api::resource::ScheduledJobRunResource {
    agena_api::resource::ScheduledJobRunResource {
        triggered_at: value.triggered_at,
        finished_at: value.finished_at,
        status: value.status,
        session_id: value.session_id,
        error_message: value.error_message,
    }
}

fn model_catalog_entry_from_http(
    value: HttpModelCatalogEntryResource,
) -> ModelCatalogEntryResource {
    ModelCatalogEntryResource {
        model_id: value.model_id,
        kind: match value.kind {
            crate::local_api::dto::ModelCatalogEntryKind::Official => {
                agena_api::resource::ModelCatalogEntryKind::Official
            }
            crate::local_api::dto::ModelCatalogEntryKind::Custom => {
                agena_api::resource::ModelCatalogEntryKind::Custom
            }
        },
        source: match value.source {
            crate::local_api::dto::ModelCatalogSourceKind::Remote => {
                agena_api::resource::ModelCatalogSourceKind::Remote
            }
            crate::local_api::dto::ModelCatalogSourceKind::Fallback => {
                agena_api::resource::ModelCatalogSourceKind::Fallback
            }
            crate::local_api::dto::ModelCatalogSourceKind::Cache => {
                agena_api::resource::ModelCatalogSourceKind::Cache
            }
            crate::local_api::dto::ModelCatalogSourceKind::Custom => {
                agena_api::resource::ModelCatalogSourceKind::Custom
            }
        },
        source_label: value.source_label,
        has_local_override: value.has_local_override,
        display_name: value.display_name,
        origin: value.origin,
        lifecycle: value.lifecycle,
        context_window_tokens: value.context_window_tokens,
        max_output_tokens: value.max_output_tokens,
        description: value.description,
        variants: value.variants,
        capabilities: value.capabilities,
    }
}

fn model_catalog_from_http(value: HttpModelCatalogResponse) -> ModelCatalogResponse {
    ModelCatalogResponse {
        remote_url: value.remote_url,
        fallback_url: value.fallback_url,
        last_refresh_at: value.last_refresh_at,
        last_successful_source: value.last_successful_source,
        last_error: value.last_error,
        entries: value
            .entries
            .into_iter()
            .map(model_catalog_entry_from_http)
            .collect(),
    }
}

fn execution_context_from_http(
    value: HttpSessionExecutionContextResource,
) -> SessionExecutionContextResource {
    SessionExecutionContextResource {
        agent_profile: value.agent_profile,
        agent_mode: value.agent_mode,
        agent_hidden: value.agent_hidden,
        agent_color: value.agent_color,
        active_skill_name: value.active_skill_name,
        system_prompt_override: value.system_prompt_override,
        allowed_tools: value.allowed_tools,
        agent_permission: value.agent_permission,
        model_provider_id: value.model_provider_id,
        model_adapter_id: value.model_adapter_id,
        model_id: value.model_id,
        model_variant: value.model_variant,
        agent_run: value.agent_run,
        effective_workspace_root: value.effective_workspace_root,
        task_id: value.task_id,
    }
}

fn session_execution_from_http(value: HttpSessionExecutionResource) -> SessionExecutionResource {
    SessionExecutionResource {
        session: session_from_http(value.session),
        blocked: value.blocked,
        run_state: match value.run_state {
            crate::local_api::SessionRunState::Idle => SessionRunState::Idle,
            crate::local_api::SessionRunState::AwaitingModel => SessionRunState::AwaitingModel,
        },
        latest_event_seq: value.latest_event_seq,
        automation: value.automation.map(session_automation_from_http),
        execution: execution_context_from_http(value.execution),
        pending_permission_requests: value.pending_permission_requests,
        pending_user_input_requests: value.pending_user_input_requests,
        goal: value.goal.map(session_goal_from_http),
    }
}

fn permission_rule_from_http(
    value: HttpPermissionRuleResource,
) -> agena_api::resource::PermissionRuleResource {
    agena_api::resource::PermissionRuleResource {
        id: value.id,
        action_key: value.action_key,
        subject_kind: value.subject_kind,
        tool_name: value.tool_name,
        qualifier: value.qualifier,
        path_access_kind: value.path_access_kind,
        workspace_root: value.workspace_root,
        target_path: value.target_path,
        network_target: value.network_target,
        network_host: value.network_host,
        network_port: value.network_port,
        mode: value.mode,
        scope: value.scope,
        session_id: value.session_id,
        workspace_id: value.workspace_id,
        source: value.source,
        reason: value.reason,
        operator: value.operator,
        revoked_at: value.revoked_at,
        revoked_reason: value.revoked_reason,
        revoked_by: value.revoked_by,
        created_at: value.created_at,
        updated_at: value.updated_at,
    }
}

fn page_from_http<T, U>(
    value: crate::local_api::PaginatedResponse<T>,
    map: impl Fn(T) -> U,
) -> PaginatedResponse<U> {
    PaginatedResponse {
        items: value.items.into_iter().map(map).collect(),
        page: PageInfo {
            next_cursor: value.page.next_cursor,
            has_more: value.page.has_more,
            returned: value.page.returned as u64,
        },
    }
}

async fn runtime_status_response(state: &AppState) -> RuntimeStatusResponse {
    let snapshot = state.runtime().current_snapshot();
    let resolution = snapshot.config_resolution();
    let mut provider_ids = snapshot.provider_registry().provider_ids();
    provider_ids.sort();
    let catalog = snapshot.model_catalog_response();
    let model_catalog = model_catalog_from_http(crate::local_api::ModelCatalogResponse {
        remote_url: catalog.remote_url,
        fallback_url: catalog.fallback_url,
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error,
        entries: catalog
            .entries
            .into_iter()
            .map(|entry| {
                crate::local_api::ModelCatalogEntryResource::from_record(
                    entry,
                    catalog.last_successful_source,
                )
            })
            .collect(),
    });
    let session_cache = snapshot.session_manager().map(|manager| {
        let stats = manager.cache_stats();
        RuntimeSessionCacheResource {
            max_sessions: resolution.config.runtime.session_cache.max_sessions,
            ttl_secs: resolution.config.runtime.session_cache.ttl_secs,
            max_bytes: resolution.config.runtime.session_cache.max_bytes,
            entry_count: stats.entry_count,
            total_bytes: stats.total_bytes,
            hits: stats.hits,
            misses: stats.misses,
            inserts: stats.inserts,
            evictions: stats.evictions,
        }
    });

    let mcp = if let Some(manager) = snapshot.mcp_manager() {
        let mut servers = manager.server_names().await;
        servers.sort();
        let all_tools = manager.all_tools().await;
        let mut tool_counts = HashMap::<String, usize>::new();
        for (server_name, _) in &all_tools {
            *tool_counts.entry(server_name.clone()).or_default() += 1;
        }
        RuntimeMcpResource {
            server_count: servers.len(),
            tool_count: all_tools.len(),
            servers: servers
                .into_iter()
                .map(|name| RuntimeMcpServerResource {
                    tool_count: tool_counts.get(&name).copied().unwrap_or(0),
                    name,
                })
                .collect(),
        }
    } else {
        RuntimeMcpResource {
            server_count: 0,
            tool_count: 0,
            servers: Vec::new(),
        }
    };

    let lsp = if let Some(registry) = snapshot.lsp_registry() {
        let mut servers = registry.server_specs().await;
        servers.sort_by(|left, right| left.name.cmp(&right.name));
        let diagnostics = registry.collect_diagnostics().await;
        let diagnostics_count = diagnostics.iter().map(|(_, entries)| entries.len()).sum();
        RuntimeLspResource {
            server_count: servers.len(),
            diagnostics_count,
            files_with_diagnostics: diagnostics.len(),
            servers: servers
                .into_iter()
                .map(|server| RuntimeLspServerResource {
                    name: server.name,
                    command: server.command,
                    file_extensions: server.file_extensions,
                    root_markers: server.root_markers,
                })
                .collect(),
        }
    } else {
        RuntimeLspResource {
            server_count: 0,
            diagnostics_count: 0,
            files_with_diagnostics: 0,
            servers: Vec::new(),
        }
    };

    let skills = {
        let mut workflows = Vec::new();
        let mut commands = Vec::new();
        let entries = snapshot
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .filter(|entry| entry.plugin_name == "agena.skills")
            .collect::<Vec<_>>();
        let skill_key_for = |entry: &agena::plugin::registry::PluginEntry| {
            entry
                .decl
                .tags
                .iter()
                .find_map(|tag| match tag {
                    agena::plugin::sdk::ToolTag::Custom(value) => {
                        value.strip_prefix("skill:").map(str::to_string)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| entry.original_name.clone())
        };
        let has_custom_tag = |entry: &agena::plugin::registry::PluginEntry, expected: &str| {
            entry.decl.tags.iter().any(|tag| match tag {
                agena::plugin::sdk::ToolTag::Custom(value) => value == expected,
                _ => false,
            })
        };
        let mut aliases_by_skill = HashMap::<String, Vec<String>>::new();
        for entry in &entries {
            if !has_custom_tag(entry, "alias") {
                continue;
            }
            aliases_by_skill
                .entry(skill_key_for(entry))
                .or_default()
                .push(entry.exposed_name.clone());
        }
        for entry in entries {
            if has_custom_tag(&entry, "alias") {
                continue;
            }
            let skill_key = skill_key_for(&entry);
            let is_command = has_custom_tag(&entry, "command");
            let item = RuntimeSkillResource {
                name: entry.exposed_name,
                description: entry.decl.description.unwrap_or_default(),
                aliases: aliases_by_skill.remove(&skill_key).unwrap_or_default(),
                source_path: None,
            };
            if is_command {
                commands.push(item);
            } else {
                workflows.push(item);
            }
        }
        workflows.sort_by(|left, right| left.name.cmp(&right.name));
        commands.sort_by(|left, right| left.name.cmp(&right.name));
        RuntimeSkillsResource {
            skill_count: workflows.len(),
            command_count: commands.len(),
            skills: workflows,
            commands,
        }
    };

    let agents = {
        let mut entries = snapshot.agents().list_descriptors();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        let default_agent = Some(resolution.config.default.agent.trim().to_owned())
            .filter(|name| entries.iter().any(|entry| entry.name == *name))
            .or_else(|| {
                entries
                    .iter()
                    .find(|entry| entry.mode.allows_root() && !entry.hidden)
                    .map(|entry| entry.name.clone())
            })
            .unwrap_or_else(|| "none".to_string());
        let total_count = entries.len();
        let primary_count = entries
            .iter()
            .filter(|entry| entry.mode.allows_root())
            .count();
        let subagent_count = entries
            .iter()
            .filter(|entry| entry.mode.allows_subagent())
            .count();
        let hidden_count = entries.iter().filter(|entry| entry.hidden).count();
        RuntimeAgentsResource {
            default_agent,
            total_count,
            primary_count,
            subagent_count,
            hidden_count,
            agents: entries
                .into_iter()
                .map(|entry| RuntimeAgentResource {
                    name: entry.name,
                    description: entry.description,
                    mode: entry.mode,
                    hidden: entry.hidden,
                    color: entry.color,
                    temperature: entry.temperature.map(|value| value.0),
                    max_output_tokens: entry.max_output_tokens,
                    steps: entry.steps,
                    allowed_tools: entry.allowed_tools,
                    permission: entry.permission,
                    model: entry.model,
                    aliases: entry.aliases,
                    scope: entry.scope,
                    source_path: entry.source_path.map(|path| path.display().to_string()),
                })
                .collect(),
        }
    };

    let automation = if let Some(manager) = snapshot.session_manager() {
        let mut jobs = crate::local_api::list_scheduled_jobs(&manager).await;
        crate::local_api::sort_jobs_for_display(&mut jobs);
        RuntimeAutomationResource {
            enabled: manager.tool_executor().scheduler().is_some(),
            job_count: jobs.len(),
            recent_jobs: jobs
                .into_iter()
                .take(10)
                .map(crate::local_api::scheduled_job_resource)
                .map(scheduled_job_from_http)
                .collect(),
        }
    } else {
        RuntimeAutomationResource {
            enabled: false,
            job_count: 0,
            recent_jobs: Vec::new(),
        }
    };

    RuntimeStatusResponse {
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at(),
        workspace_root: state.runtime().workspace_root().display().to_string(),
        config_path: resolution.meta.config_path.display().to_string(),
        config_found: resolution.meta.config_found,
        provider_ids,
        plugin_count: snapshot.plugin_manager().plugins().len(),
        session_runtime_available: snapshot.session_manager().is_some(),
        watch_paths: snapshot
            .watch_paths()
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        reload: RuntimeTaskResource {
            enabled: snapshot.reload_enabled(),
            interval_secs: snapshot.reload_poll_interval().as_secs(),
        },
        janitor: RuntimeTaskResource {
            enabled: snapshot.janitor_enabled(),
            interval_secs: snapshot.janitor_interval().as_secs(),
        },
        session_cache,
        model_catalog: Some(model_catalog),
        automation,
        operator: RuntimeOperatorResource {
            mcp,
            lsp,
            agents,
            skills,
        },
    }
}

fn list_providers_response(state: &AppState) -> Vec<ProviderSummaryResource> {
    let snapshot = state.runtime().current_snapshot();
    let registry = snapshot.provider_registry();
    let mut providers = registry
        .provider_ids()
        .into_iter()
        .filter_map(|provider_id| {
            registry.get(provider_id.as_str()).map(|provider| {
                let provider_config = snapshot
                    .config_resolution()
                    .config
                    .providers
                    .get(provider_id.as_str());
                let adapters = provider_config
                    .map(|provider| {
                        provider
                            .adapters
                            .iter()
                            .map(|(adapter_id, adapter)| ProviderAdapterSummaryResource {
                                adapter_id: adapter_id.clone(),
                                enabled: adapter.enabled,
                                configured_model_count: provider
                                    .models
                                    .keys()
                                    .filter(|model_id| {
                                        model_id
                                            .split_once('/')
                                            .map(|(route_adapter_id, _)| {
                                                route_adapter_id == adapter_id
                                            })
                                            .unwrap_or(false)
                                    })
                                    .count(),
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();

                ProviderSummaryResource {
                    default_adapter: provider.default_adapter().map(ToString::to_string),
                    default_model: provider.default_model().to_string(),
                    adapters,
                    provider_id,
                }
            })
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    providers
}

// ─── Command dispatch ───────────────────────────────────────────────────

pub async fn dispatch_command(
    state: &AppState,
    command: Command,
) -> Result<CommandResult, ServerError> {
    let manager = state.session_manager()?;
    match command {
        Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
            let workspace = state
                .service()
                .create_workspace(WorkspaceWriteRequest { path })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
        }
        Command::UpdateWorkspace(UpdateWorkspaceParams {
            workspace_id, path, ..
        }) => {
            let workspace = state
                .service()
                .replace_workspace(workspace_id, WorkspaceWriteRequest { path })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
        }
        Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
            state
                .service()
                .delete_workspace(workspace_id)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::WorkspaceDeleted { id: workspace_id })
        }
        Command::ResolveWorkspace(ResolveWorkspaceParams {
            path,
            create_if_missing,
        }) => {
            let workspace = state
                .service()
                .resolve_workspace(WorkspaceResolveRequest {
                    path,
                    create_if_missing,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Workspace(workspace_from_http(workspace)))
        }
        Command::CreateSession(CreateSessionParams {
            workspace_id,
            title,
            parent_id,
        }) => {
            let session = state
                .service()
                .create_session(HttpSessionCreateRequest {
                    workspace_id,
                    title,
                    parent_id,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Session(session_from_http(session)))
        }
        Command::CreateSessionGoal(CreateSessionGoalParams {
            session_id,
            objective,
            token_budget,
        }) => {
            let goal = manager
                .create_goal(agena::session::SessionGoalCreateRequest {
                    session_id,
                    objective,
                    token_budget,
                })
                .await?;
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::SetSessionGoal(SetSessionGoalParams {
            session_id,
            objective,
            status,
            token_budget,
            clear,
        }) => {
            if clear {
                let cleared = manager.clear_goal(session_id).await?;
                if !cleared {
                    return Err(ServerError::NotFound(format!(
                        "session {session_id} goal not found"
                    )));
                }
                return Ok(CommandResult::SessionGoalCleared { session_id });
            }

            let goal = if manager.get_goal(session_id).await?.is_some() {
                manager
                    .update_goal(agena::session::SessionGoalUpdateRequest {
                        session_id,
                        objective,
                        status,
                        token_budget,
                        expected_goal_id: None,
                    })
                    .await?
            } else {
                if !matches!(status, None | Some(agena::session::GoalStatus::Active)) {
                    return Err(ServerError::BadRequest(format!(
                        "session {session_id} goal must be created with status active"
                    )));
                }
                let objective = objective.ok_or_else(|| {
                    ServerError::BadRequest(format!(
                        "session {session_id} goal objective is required when creating a goal"
                    ))
                })?;
                manager
                    .create_goal(agena::session::SessionGoalCreateRequest {
                        session_id,
                        objective,
                        token_budget: token_budget.flatten(),
                    })
                    .await?
            };
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::CompleteSessionGoal(CompleteSessionGoalParams { session_id }) => {
            let goal = manager.complete_goal(session_id).await?;
            let session = manager.get_session(session_id).await?;
            let resource = state
                .service()
                .session_goal_resource(manager.as_ref(), &session, &goal)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionGoal(session_goal_from_http(resource)))
        }
        Command::ClearSessionGoal(ClearSessionGoalParams { session_id }) => {
            let cleared = manager.clear_goal(session_id).await?;
            if !cleared {
                return Err(ServerError::NotFound(format!(
                    "session {session_id} goal not found"
                )));
            }
            Ok(CommandResult::SessionGoalCleared { session_id })
        }
        Command::SubmitTurn(SubmitTurnParams {
            session_id,
            options,
            parts,
        }) => {
            let request = SessionUserTurnRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                parts,
            };
            let session = manager.submit_user_turn(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ContinueRun(ContinueRunParams {
            session_id,
            options,
        }) => {
            let request = SessionContinueRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
            };
            let session = manager.continue_session(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::CancelTurn(CancelTurnParams { session_id }) => {
            // Best-effort: if the turn just finished moments before the
            // cancel arrived, NoActiveTurn is normal — surface as Ack so
            // the client doesn't spin on it.
            match manager.cancel_active_turn(session_id).await {
                Ok(()) => Ok(CommandResult::Ack),
                Err(_) => Ok(CommandResult::Ack),
            }
        }
        Command::RewindSession(RewindSessionParams {
            session_id,
            message_id,
            expected_version,
        }) => {
            let session = manager
                .rewind_session(agena::session::SessionRewindRequest {
                    session_id,
                    message_id,
                    expected_version,
                })
                .await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::UnrewindSession(UnrewindSessionParams {
            session_id,
            message_id,
            expected_version,
        }) => {
            let session = manager
                .unrewind_session(agena::session::SessionUnrewindRequest {
                    session_id,
                    message_id,
                    expected_version,
                })
                .await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ForkSession(ForkSessionParams {
            session_id,
            at_message_id,
            title,
        }) => {
            let session = manager
                .fork_session(agena::session::SessionForkRequest {
                    session_id,
                    at_message_id,
                    title,
                    expected_version: None,
                })
                .await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
            let summaries = manager.list_session_tree(root_id).await?;
            let resources: Vec<SessionResource> =
                summaries.into_iter().map(SessionResource::from).collect();
            Ok(CommandResult::SessionTree(resources))
        }
        Command::ListRewindCheckpoints(ListRewindCheckpointsParams { session_id }) => {
            let checkpoints = manager.list_rewind_checkpoints(session_id).await?;
            Ok(CommandResult::RewindCheckpoints(
                checkpoints.into_iter().map(Into::into).collect(),
            ))
        }
        Command::ExportSession(ExportSessionParams { session_id }) => {
            let jsonl = manager.export_session_jsonl(session_id).await?;
            Ok(CommandResult::SessionExport { jsonl })
        }
        Command::ImportSession(ImportSessionParams { jsonl }) => {
            let session = manager.import_session_jsonl(&jsonl).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionPermissionReplyRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                reply,
                operator: Some("jsonrpc".to_string()),
            };
            let session = manager.reply_permission(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::ReplyUserInput(ReplyUserInputParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionUserInputReplyRequest {
                session_id,
                options: run_options_to_core(state, session_id, &options).await?,
                reply,
            };
            let session = manager.reply_user_input(request).await?;
            let resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Execution(session_execution_from_http(
                resource,
            )))
        }
        Command::UpdateSession(UpdateSessionParams {
            session_id,
            title,
            parent_id,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .map_err(server_error_from_http)?;
            }
            let session = state
                .service()
                .replace_session(session_id, SessionReplaceRequest { title, parent_id })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::Session(session_from_http(session)))
        }
        Command::DeleteSession(DeleteSessionParams {
            session_id,
            expected_version,
        }) => {
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .map_err(server_error_from_http)?;
            }
            state
                .service()
                .delete_session(session_id)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::SessionDeleted { id: session_id })
        }
        Command::UpsertPermissionRule(UpsertPermissionRuleParams {
            action_key,
            subject_kind,
            tool_name,
            qualifier,
            path_access_kind,
            workspace_root,
            target_path,
            network_target,
            network_host,
            network_port,
            scope,
            session_id,
            mode,
        }) => {
            let rule = state
                .service()
                .create_permission_rule(PermissionRuleWriteRequest {
                    action_key,
                    subject_kind,
                    tool_name,
                    qualifier,
                    path_access_kind,
                    workspace_root,
                    target_path,
                    network_target,
                    network_host,
                    network_port,
                    scope,
                    session_id,
                    mode,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
        }
        Command::ReplacePermissionRule(ReplacePermissionRuleParams { rule_id, rule }) => {
            let rule = state
                .service()
                .replace_permission_rule(
                    rule_id,
                    PermissionRuleWriteRequest {
                        action_key: rule.action_key,
                        subject_kind: rule.subject_kind,
                        tool_name: rule.tool_name,
                        qualifier: rule.qualifier,
                        path_access_kind: rule.path_access_kind,
                        workspace_root: rule.workspace_root,
                        target_path: rule.target_path,
                        network_target: rule.network_target,
                        network_host: rule.network_host,
                        network_port: rule.network_port,
                        scope: rule.scope,
                        session_id: rule.session_id,
                        mode: rule.mode,
                    },
                )
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
        }
        Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
            let rule = state
                .service()
                .revoke_permission_rule(rule_id, reason)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRule(permission_rule_from_http(
                rule,
            )))
        }
        Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
            state
                .service()
                .delete_permission_rule(rule_id)
                .await
                .map_err(server_error_from_http)?;
            Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
        }
    }
}

// ─── Query dispatch ─────────────────────────────────────────────────────

pub async fn dispatch_query(state: &AppState, query: Query) -> Result<QueryResult, ServerError> {
    let manager = state.session_manager()?;
    match query {
        Query::ListWorkspaces(ListWorkspacesParams {
            cursor,
            limit,
            search,
            include_session_count,
        }) => {
            let page = state
                .service()
                .list_workspaces(WorkspaceListQuery {
                    cursor,
                    limit,
                    search,
                    include_session_count,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(QueryResult::Workspaces(page_from_http(
                page,
                workspace_from_http,
            )))
        }
        Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => {
            let workspace = state
                .service()
                .get_workspace(workspace_id)
                .await
                .map_err(server_error_from_http)?
                .ok_or_else(|| {
                    ServerError::NotFound(format!("workspace {workspace_id} not found"))
                })?;
            Ok(QueryResult::Workspace(workspace_from_http(workspace)))
        }
        Query::ListSessions(ListSessionsParams {
            cursor,
            limit,
            workspace_id,
            parent_id,
            roots,
            search,
        }) => {
            let page = state
                .service()
                .list_sessions(SessionListQuery {
                    cursor,
                    limit,
                    workspace_id,
                    parent_id,
                    roots,
                    search,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(QueryResult::Sessions(page_from_http(
                page,
                session_from_http,
            )))
        }
        Query::GetSession(GetSessionParams { session_id }) => {
            let session = state
                .service()
                .get_session(session_id)
                .await
                .map_err(server_error_from_http)?
                .ok_or_else(|| ServerError::NotFound(format!("session {session_id} not found")))?;
            Ok(QueryResult::Session(session_from_http(session)))
        }
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor,
            limit,
            parts,
        }) => {
            let page = state
                .service()
                .list_messages(
                    manager.as_ref(),
                    session_id,
                    MessageListQuery {
                        cursor,
                        limit,
                        parts: part_load_mode_to_http(parts),
                    },
                )
                .await
                .map_err(server_error_from_http)?;
            Ok(QueryResult::Messages(page_from_http(
                page,
                message_resource_from_http,
            )))
        }
        Query::GetMessage(GetMessageParams { message_id, parts }) => {
            let message = state
                .service()
                .get_message(manager.as_ref(), message_id, part_load_mode_to_http(parts))
                .await
                .map_err(server_error_from_http)?
                .map(message_resource_from_http)
                .ok_or_else(|| ServerError::NotFound(format!("message {message_id} not found")))?;
            Ok(QueryResult::Message(message))
        }
        Query::ListEvents(ListEventsParams {
            scope,
            kinds,
            since_seq_global,
            limit,
        }) => {
            let publisher = state.event_publisher()?;
            let store: &std::sync::Arc<dyn EventStore<EventKind>> = publisher.store();
            let filter = agena::event::EventFilter {
                scope,
                kinds,
                since_seq_global,
            };
            let limit = normalize_limit(limit) as usize;
            let range = StoreRange {
                after_seq_global: since_seq_global.unwrap_or(0),
                limit,
            };
            let events = store
                .range(&filter, range)
                .await
                .map_err(|e| ServerError::Internal(e.to_string()))?;
            let returned = events.len() as u64;
            let next_cursor = events.last().map(|e| e.meta.seq_global.to_string());
            Ok(QueryResult::Events(PaginatedEvents {
                items: events,
                page: PageInfo {
                    next_cursor,
                    has_more: returned as usize >= limit,
                    returned,
                },
            }))
        }
        Query::Health => Ok(QueryResult::Health(agena_api::resource::HealthResponse {
            status: "ok".into(),
            generation: 0,
            loaded_at: chrono::Utc::now(),
            database_connected: true,
        })),
        Query::Runtime => Ok(QueryResult::Runtime(runtime_status_response(state).await)),
        Query::ListProviders => Ok(QueryResult::Providers(list_providers_response(state))),
        Query::ListProviderModels(ListProviderModelsParams { provider_id }) => {
            let snapshot = state.runtime().current_snapshot();
            if snapshot
                .provider_registry()
                .get(provider_id.as_str())
                .is_none()
            {
                return Err(ServerError::NotFound(format!(
                    "provider {provider_id} not found"
                )));
            }

            let models = snapshot
                .list_provider_models(provider_id.as_str())
                .await
                .map_err(ServerError::Core)?;
            Ok(QueryResult::ProviderModels(ProviderModelsResponse {
                provider_id,
                models,
            }))
        }
        Query::GetSessionState(GetSessionParams { session_id }) => {
            let session = manager.get_session(session_id).await?;
            let state_resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .map_err(server_error_from_http)?;
            Ok(QueryResult::SessionState(session_execution_from_http(
                state_resource,
            )))
        }
        Query::GetSessionGoal(GetSessionParams { session_id }) => {
            let session = manager.get_session(session_id).await?;
            let goal = match session.goal.as_ref() {
                Some(goal) => Some(session_goal_from_http(
                    state
                        .service()
                        .session_goal_resource(manager.as_ref(), &session, goal)
                        .await
                        .map_err(server_error_from_http)?,
                )),
                None => None,
            };
            Ok(QueryResult::SessionGoal(goal))
        }
        Query::ListPermissionRules(ListPermissionRulesParams {
            cursor,
            limit,
            search,
        }) => {
            let page = state
                .service()
                .list_permission_rules(PermissionRuleListQuery {
                    cursor,
                    limit,
                    search,
                })
                .await
                .map_err(server_error_from_http)?;
            Ok(QueryResult::PermissionRules(page_from_http(
                page,
                permission_rule_from_http,
            )))
        }
        Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
            let rule = state
                .service()
                .get_permission_rule(rule_id)
                .await
                .map_err(server_error_from_http)?
                .ok_or_else(|| {
                    ServerError::NotFound(format!("permission rule {rule_id} not found"))
                })?;
            Ok(QueryResult::PermissionRule(permission_rule_from_http(rule)))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::local_api::SessionCreateRequest;
    use agena::config::LoadConfigRequest;
    use agena::db::entities::{activity_message, activity_part};
    use agena::message::{ExecutionStatus, MessageMetadata, PartContent, PartKind};
    use agena::model::ModelRef;
    use agena::runtime::AgenaRuntime;
    use agena_api::resource::RunOptions;
    use sea_orm::{ActiveModelTrait, ActiveValue::Set};
    use std::{
        fs,
        path::{Path, PathBuf},
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    fn unique_test_dir(label: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock should be after unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!("agena-api-server-{label}-{nanos}"))
    }

    async fn create_session(state: &AppState, workspace_root: &Path, title: &str) -> i64 {
        let workspace = state
            .service()
            .resolve_workspace(crate::local_api::WorkspaceResolveRequest {
                path: workspace_root.display().to_string(),
                create_if_missing: true,
            })
            .await
            .expect("workspace should resolve");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: title.to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");
        session.id
    }

    async fn test_state_with_config(config: &str, label: &str) -> (AppState, PathBuf) {
        let root = unique_test_dir(label);
        let workspace_root = root.join("workspace");
        fs::create_dir_all(&workspace_root).expect("create workspace root");
        let config_path = root.join("config.toml");
        fs::write(&config_path, config).expect("write config");

        let db = Arc::new(
            sea_orm::Database::connect("sqlite::memory:")
                .await
                .expect("in-memory sqlite should connect"),
        );
        agena::db::init_schema(db.as_ref())
            .await
            .expect("schema init should succeed");

        let runtime = AgenaRuntime::builder()
            .with_load_request(LoadConfigRequest {
                config_path: Some(config_path),
                overrides: Vec::new(),
            })
            .with_workspace_root(workspace_root.clone())
            .with_database_connection(db.as_ref().clone())
            .build()
            .await
            .expect("runtime build should succeed");

        (AppState::new(runtime, db), workspace_root)
    }

    async fn insert_projected_message_with_text_part(
        state: &AppState,
        session_id: i64,
        message_id: i64,
        part_id: i64,
        created_at_ms: i64,
        summary: &str,
        text: &str,
    ) {
        let db = state.service().clone_db();

        activity_message::ActiveModel {
            message_id: Set(message_id),
            session_id: Set(session_id),
            role: Set(agena::role::Role::Assistant),
            state: Set(ExecutionStatus::Completed),
            created_at_ms: Set(created_at_ms),
            updated_at_ms: Set(created_at_ms),
            metadata: Set(MessageMetadata::default()),
            usage: Set(None),
            finish: Set(None),
            part_count: Set(1),
            is_compacted: Set(false),
        }
        .insert(db.as_ref())
        .await
        .expect("activity message projection should insert");

        activity_part::ActiveModel {
            part_id: Set(part_id),
            message_id: Set(message_id),
            session_id: Set(session_id),
            part_index: Set(0),
            status: Set(ExecutionStatus::Completed),
            kind: Set(PartKind::Text),
            name: Set(None),
            summary: Set(Some(summary.to_string())),
            has_detail: Set(true),
            operation_id: Set(None),
            created_at_ms: Set(created_at_ms),
            content: Set(Some(PartContent::text(text))),
        }
        .insert(db.as_ref())
        .await
        .expect("activity part projection should insert");
    }

    #[tokio::test]
    async fn message_queries_respect_none_parts_mode() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "message-parts-none",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "message none").await;
        let db = state.service().clone_db();
        let created_at = chrono::Utc::now().timestamp_millis();
        let message_id = 7001;
        let part_id = 7101;

        activity_message::ActiveModel {
            message_id: Set(message_id),
            session_id: Set(session_id),
            role: Set(agena::role::Role::Assistant),
            state: Set(ExecutionStatus::Completed),
            created_at_ms: Set(created_at),
            updated_at_ms: Set(created_at),
            metadata: Set(MessageMetadata::default()),
            usage: Set(None),
            finish: Set(None),
            part_count: Set(1),
            is_compacted: Set(false),
        }
        .insert(db.as_ref())
        .await
        .expect("activity message projection should insert");

        activity_part::ActiveModel {
            part_id: Set(part_id),
            message_id: Set(message_id),
            session_id: Set(session_id),
            part_index: Set(0),
            status: Set(ExecutionStatus::Completed),
            kind: Set(PartKind::Text),
            name: Set(None),
            summary: Set(Some("hello from dispatch".to_string())),
            has_detail: Set(true),
            operation_id: Set(None),
            created_at_ms: Set(created_at),
            content: Set(Some(PartContent::text("hello from dispatch"))),
        }
        .insert(db.as_ref())
        .await
        .expect("activity part projection should insert");

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: None,
                parts: agena_api::resource::PartLoadMode::None,
            }),
        )
        .await
        .expect("list messages query should succeed");
        let QueryResult::Messages(page) = result else {
            panic!("expected message list result");
        };
        let message = page
            .items
            .first()
            .expect("message list should not be empty");
        assert_eq!(message.id, message_id);
        assert!(message.parts.is_none(), "none mode should omit parts");
        assert_eq!(message.part_count, 1);

        let result = dispatch_query(
            &state,
            Query::GetMessage(GetMessageParams {
                message_id,
                parts: agena_api::resource::PartLoadMode::None,
            }),
        )
        .await
        .expect("get message query should succeed");
        let QueryResult::Message(message) = result else {
            panic!("expected message result");
        };
        assert_eq!(message.id, message_id);
        assert!(message.parts.is_none(), "none mode should omit parts");
        assert_eq!(message.part_count, 1);
    }

    #[tokio::test]
    async fn message_queries_none_use_projected_part_count_without_part_rows() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "message-none-part-count",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "message none headers").await;
        let db = state.service().clone_db();
        let created_at = chrono::Utc::now().timestamp_millis();
        let message_id = 7002;

        activity_message::ActiveModel {
            message_id: Set(message_id),
            session_id: Set(session_id),
            role: Set(agena::role::Role::Assistant),
            state: Set(ExecutionStatus::Completed),
            created_at_ms: Set(created_at),
            updated_at_ms: Set(created_at),
            metadata: Set(MessageMetadata::default()),
            usage: Set(None),
            finish: Set(None),
            part_count: Set(3),
            is_compacted: Set(false),
        }
        .insert(db.as_ref())
        .await
        .expect("activity message projection should insert");

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: None,
                parts: agena_api::resource::PartLoadMode::None,
            }),
        )
        .await
        .expect("list messages query should succeed");
        let QueryResult::Messages(page) = result else {
            panic!("expected message list result");
        };
        let message = page
            .items
            .first()
            .expect("message list should not be empty");
        assert_eq!(message.id, message_id);
        assert!(message.parts.is_none(), "none mode should omit parts");
        assert_eq!(message.part_count, 3);

        let result = dispatch_query(
            &state,
            Query::GetMessage(GetMessageParams {
                message_id,
                parts: agena_api::resource::PartLoadMode::None,
            }),
        )
        .await
        .expect("get message query should succeed");
        let QueryResult::Message(message) = result else {
            panic!("expected message result");
        };
        assert_eq!(message.id, message_id);
        assert!(message.parts.is_none(), "none mode should omit parts");
        assert_eq!(message.part_count, 3);
    }

    #[tokio::test]
    async fn list_messages_query_uses_paginated_service_metadata() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "message-pagination-dispatch",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "message pagination").await;
        let created_at = chrono::Utc::now().timestamp_millis();

        insert_projected_message_with_text_part(
            &state,
            session_id,
            7201,
            7301,
            created_at,
            "older summary",
            "older body",
        )
        .await;
        insert_projected_message_with_text_part(
            &state,
            session_id,
            7202,
            7302,
            created_at + 1,
            "newer summary",
            "newer body",
        )
        .await;

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: Some(1),
                parts: agena_api::resource::PartLoadMode::Summary,
            }),
        )
        .await
        .expect("first list messages query should succeed");
        let QueryResult::Messages(first_page) = result else {
            panic!("expected message list result");
        };
        assert_eq!(first_page.items.len(), 1);
        assert_eq!(first_page.page.returned, 1);
        assert!(
            first_page.page.has_more,
            "first page should report more rows"
        );
        let next_cursor = first_page
            .page
            .next_cursor
            .clone()
            .expect("first page should include a cursor");
        assert_eq!(first_page.items[0].id, 7202);

        let first_part = first_page.items[0]
            .parts
            .as_ref()
            .and_then(|parts| parts.first())
            .expect("summary mode should include part headers");
        assert_eq!(first_part.summary.as_deref(), Some("newer summary"));
        assert!(
            first_part.content.is_none(),
            "summary mode should omit full content"
        );

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: Some(next_cursor),
                limit: Some(1),
                parts: agena_api::resource::PartLoadMode::Summary,
            }),
        )
        .await
        .expect("second list messages query should succeed");
        let QueryResult::Messages(second_page) = result else {
            panic!("expected message list result");
        };
        assert_eq!(second_page.items.len(), 1);
        assert_eq!(second_page.items[0].id, 7201);
        assert!(
            !second_page.page.has_more,
            "cursor should advance to the final page"
        );
    }

    #[tokio::test]
    async fn list_messages_query_preserves_parts_modes() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "message-parts-dispatch",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "message parts").await;
        let created_at = chrono::Utc::now().timestamp_millis();

        insert_projected_message_with_text_part(
            &state,
            session_id,
            7401,
            7501,
            created_at,
            "dispatch summary",
            "dispatch full text",
        )
        .await;

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: Some(1),
                parts: agena_api::resource::PartLoadMode::None,
            }),
        )
        .await
        .expect("none list messages query should succeed");
        let QueryResult::Messages(none_page) = result else {
            panic!("expected message list result");
        };
        assert!(
            none_page.items[0].parts.is_none(),
            "none mode should omit parts"
        );
        assert_eq!(none_page.items[0].part_count, 1);

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: Some(1),
                parts: agena_api::resource::PartLoadMode::Summary,
            }),
        )
        .await
        .expect("summary list messages query should succeed");
        let QueryResult::Messages(summary_page) = result else {
            panic!("expected message list result");
        };
        let summary_part = summary_page.items[0]
            .parts
            .as_ref()
            .and_then(|parts| parts.first())
            .expect("summary mode should include part headers");
        assert_eq!(summary_part.summary.as_deref(), Some("dispatch summary"));
        assert!(
            summary_part.content.is_none(),
            "summary mode should omit full content"
        );

        let result = dispatch_query(
            &state,
            Query::ListMessages(ListMessagesParams {
                session_id,
                cursor: None,
                limit: Some(1),
                parts: agena_api::resource::PartLoadMode::Full,
            }),
        )
        .await
        .expect("full list messages query should succeed");
        let QueryResult::Messages(full_page) = result else {
            panic!("expected message list result");
        };
        let full_part = full_page.items[0]
            .parts
            .as_ref()
            .and_then(|parts| parts.first())
            .expect("full mode should include full parts");
        assert_eq!(full_part.summary.as_deref(), Some("dispatch summary"));
        assert_eq!(full_part.text(), Some("dispatch full text"));
    }

    #[tokio::test]
    async fn run_options_to_core_uses_single_provider_default_when_model_absent() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "single-provider-default",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "single provider").await;
        let options = RunOptions {
            model: None,
            variant: None,
            agent_profile: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            max_turn_loops: None,
        };
        let core = run_options_to_core(&state, session_id, &options)
            .await
            .expect("single provider should resolve default model");
        assert_eq!(core.model.provider_id.as_str(), "openai");
        assert_eq!(core.model.model_id.as_str(), "openai/gpt-5.4");
    }

    #[tokio::test]
    async fn run_options_to_core_errors_when_model_absent_and_multiple_providers_exist() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.ollama]
default_model = "ollama/qwen3:14b"

[providers.ollama.adapters.ollama]
enabled = true
base_url = "http://localhost:11434"
"#,
            "multiple-provider-default",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "multiple providers").await;
        let options = RunOptions {
            model: None,
            variant: None,
            agent_profile: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            max_turn_loops: None,
        };
        let error = run_options_to_core(&state, session_id, &options)
            .await
            .expect_err("multiple providers should require an explicit or inferred model");
        assert!(
            matches!(error, ServerError::BadRequest(message) if message.contains("model is required"))
        );
    }

    #[tokio::test]
    async fn run_options_to_core_round_trips_explicit_model() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.ollama]
default_model = "ollama/qwen3:14b"

[providers.ollama.adapters.ollama]
enabled = true
base_url = "http://localhost:11434"
"#,
            "explicit-model",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "explicit model").await;
        let options = RunOptions {
            model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
            variant: None,
            agent_profile: None,
            system: Some("be concise".into()),
            temperature: Some(0.7),
            max_output_tokens: Some(256),
            max_turn_loops: None,
        };
        let core = run_options_to_core(&state, session_id, &options)
            .await
            .expect("explicit model should bypass default inference");
        assert_eq!(core.model.provider_id.as_str(), "openai");
        assert_eq!(core.model.model_id.as_str(), "openai/gpt-5.4");
        assert_eq!(core.system.as_deref(), Some("be concise"));
        assert_eq!(core.temperature, Some(0.7));
        assert_eq!(core.max_output_tokens, Some(256));
    }

    #[tokio::test]
    async fn run_options_to_core_resolves_model_variant_thinking() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".variants.light]
thinking = { type = "budget", budget_tokens = 3000 }

[providers.openai.adapters.openai.models."gpt-5.4".variants.deep]
thinking = { type = "budget", budget_tokens = 30000 }
"#,
            "model-variant",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "variant").await;
        let options = RunOptions {
            model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
            variant: Some("deep".to_string()),
            agent_profile: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            max_turn_loops: None,
        };

        let core = run_options_to_core(&state, session_id, &options)
            .await
            .expect("variant should resolve");

        assert_eq!(core.variant.as_deref(), Some("deep"));
        assert_eq!(
            core.thinking,
            Some(agena::provider::ThinkingRequest::Budget {
                budget_tokens: 30000
            })
        );
    }

    #[tokio::test]
    async fn run_options_to_core_rejects_unknown_model_variant() {
        let (state, workspace_root) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5.4".variants.light]
thinking = { type = "budget", budget_tokens = 3000 }
"#,
            "unknown-model-variant",
        )
        .await;
        let session_id = create_session(&state, &workspace_root, "variant").await;
        let options = RunOptions {
            model: Some(ModelRef::new("openai", "openai/gpt-5.4")),
            variant: Some("deep".to_string()),
            agent_profile: None,
            system: None,
            temperature: None,
            max_output_tokens: None,
            max_turn_loops: None,
        };

        let error = run_options_to_core(&state, session_id, &options)
            .await
            .expect_err("unknown variant should be rejected");

        assert!(
            matches!(error, ServerError::BadRequest(message) if message.contains("has no variant `deep`"))
        );
    }

    #[tokio::test]
    async fn runtime_query_includes_agent_inventory() {
        let (state, _) = test_state_with_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
            "runtime-agent-inventory",
        )
        .await;

        let result = dispatch_query(&state, Query::Runtime)
            .await
            .expect("runtime query should succeed");
        let QueryResult::Runtime(runtime) = result else {
            panic!("expected runtime query result");
        };

        assert_eq!(runtime.operator.agents.default_agent, "build");
        assert!(runtime.operator.agents.total_count >= 1);
        assert!(runtime.operator.agents.primary_count >= 1);
        assert!(
            runtime
                .operator
                .agents
                .agents
                .iter()
                .any(|agent| agent.name == "build" && agent.mode.allows_root())
        );
        assert!(
            runtime
                .operator
                .agents
                .agents
                .iter()
                .any(|agent| agent.name == "planner")
        );
        assert!(
            runtime
                .operator
                .agents
                .agents
                .iter()
                .any(|agent| agent.name == "scout")
        );
    }

    #[test]
    fn server_error_from_http_preserves_bad_request() {
        let err = crate::local_api::ApiError::bad_request("boom");
        let server_err = server_error_from_http(err);
        assert!(matches!(server_err, ServerError::BadRequest(message) if message == "boom"));
    }
}
