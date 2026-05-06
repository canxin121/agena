//! Command/Query dispatch into the underlying [`agena::session::SessionManager`].
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport.

use std::collections::HashMap;

use agena::event::{EventStore, StoreRange};
use agena::{
    event::EventKind,
    model::ModelRef,
    session::{
        SessionContinueRequest, SessionPermissionReplyRequest, SessionRunOptions,
        SessionUserInputReplyRequest, SessionUserTurnRequest,
    },
};
use agena_api::{
    commands::{
        CancelTurnParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RewindSessionParams, SubmitTurnParams, UpdateSessionParams, UpdateWorkspaceParams,
        UpsertPermissionRuleParams,
    },
    pagination::{PageInfo, PaginatedResponse, normalize_limit},
    queries::{
        GetMessageParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
        ListEventsParams, ListMessagesParams, ListPermissionRulesParams, ListProviderModelsParams,
        ListSessionsParams, ListWorkspacesParams, PaginatedEvents, Query, QueryResult,
    },
    resource::{
        ProviderModelsResponse, ProviderSummaryResource, RunOptions, RuntimeAutomationResource,
        RuntimeLspResource, RuntimeLspServerResource, RuntimeMcpResource, RuntimeMcpServerResource,
        RuntimeOperatorResource, RuntimeSessionCacheResource, RuntimeSkillResource,
        RuntimeSkillsResource, RuntimeStatusResponse, RuntimeTaskResource,
        SessionAutomationResource, SessionExecutionContextResource, SessionExecutionResource,
        SessionResource, SessionRunState, WorkspaceResource,
    },
};
use crate::local_api::{
    PermissionRuleListQuery, PermissionRuleResource as HttpPermissionRuleResource,
    PermissionRuleWriteRequest, SessionAutomationResource as HttpSessionAutomationResource,
    SessionCreateRequest as HttpSessionCreateRequest,
    SessionExecutionContextResource as HttpSessionExecutionContextResource,
    SessionExecutionResource as HttpSessionExecutionResource, SessionListQuery,
    SessionReplaceRequest, SessionResource as HttpSessionResource, WorkspaceListQuery,
    WorkspaceResolveRequest, WorkspaceResource as HttpWorkspaceResource, WorkspaceWriteRequest,
};

use crate::{error::ServerError, state::AppState};

const DEFAULT_MODEL_REF: &str = "openai/gpt-4o-mini";

fn run_options_to_core(options: &RunOptions) -> SessionRunOptions {
    let model = options.model.clone().unwrap_or_else(|| {
        let parts: Vec<&str> = DEFAULT_MODEL_REF.split('/').collect();
        ModelRef::new(parts[0], parts.get(1).copied().unwrap_or("gpt-4o-mini"))
    });
    SessionRunOptions {
        model,
        system: options.system.clone(),
        temperature: options.temperature,
        max_output_tokens: options.max_output_tokens,
    }
}

fn server_error_from_http(_error: crate::local_api::ApiError) -> ServerError {
    ServerError::Internal("internal API call failed".into())
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
        workspace_id: value.workspace_id,
        title: value.title,
        version: value.version,
        created_at: value.created_at,
        updated_at: value.updated_at,
        message_count: value.message_count,
        child_session_count: value.child_session_count,
        last_message_at: value.last_message_at,
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

fn execution_context_from_http(
    value: HttpSessionExecutionContextResource,
) -> SessionExecutionContextResource {
    SessionExecutionContextResource {
        agent_profile: value.agent_profile,
        active_skill_name: value.active_skill_name,
        system_prompt_override: value.system_prompt_override,
        allowed_tools: value.allowed_tools,
        model_provider_id: value.model_provider_id,
        model_id: value.model_id,
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
    }
}

fn permission_rule_from_http(
    value: HttpPermissionRuleResource,
) -> agena_api::resource::PermissionRuleResource {
    agena_api::resource::PermissionRuleResource {
        id: value.id,
        action_key: value.action_key,
        mode: value.mode,
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

    let skills = if let Some(manager) = snapshot.skills_manager() {
        let mut skills = manager.list();
        skills.sort_by(|left, right| left.frontmatter.name.cmp(&right.frontmatter.name));
        let mut commands = manager.list_commands();
        commands.sort_by(|left, right| left.frontmatter.name.cmp(&right.frontmatter.name));
        RuntimeSkillsResource {
            skill_count: skills.len(),
            command_count: commands.len(),
            skills: skills
                .into_iter()
                .map(|skill| RuntimeSkillResource {
                    name: skill.frontmatter.name,
                    description: skill.frontmatter.description,
                    aliases: skill.frontmatter.aliases,
                    source_path: skill.source_path.map(|path| path.display().to_string()),
                })
                .collect(),
            commands: commands
                .into_iter()
                .map(|skill| RuntimeSkillResource {
                    name: skill.frontmatter.name,
                    description: skill.frontmatter.description,
                    aliases: skill.frontmatter.aliases,
                    source_path: skill.source_path.map(|path| path.display().to_string()),
                })
                .collect(),
        }
    } else {
        RuntimeSkillsResource {
            skill_count: 0,
            command_count: 0,
            skills: Vec::new(),
            commands: Vec::new(),
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
        active_mode: resolution
            .meta
            .active_mode
            .as_ref()
            .map(ToString::to_string),
        active_mode_source: resolution.meta.active_mode_source,
        auth_store_path: resolution.config.auth.store_path.display().to_string(),
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
        automation,
        operator: RuntimeOperatorResource { mcp, lsp, skills },
    }
}

fn list_providers_response(state: &AppState) -> Vec<ProviderSummaryResource> {
    let snapshot = state.runtime().current_snapshot();
    let registry = snapshot.provider_registry();
    let mut providers = registry
        .provider_ids()
        .into_iter()
        .filter_map(|provider_id| {
            registry
                .get(provider_id.as_str())
                .map(|provider| ProviderSummaryResource {
                    default_model_ref: format!("{provider_id}/{}", provider.default_model()),
                    default_model: provider.default_model().to_string(),
                    provider_id,
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
        Command::SubmitTurn(SubmitTurnParams {
            session_id,
            options,
            parts,
        }) => {
            let request = SessionUserTurnRequest {
                session_id,
                options: run_options_to_core(&options),
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
                options: run_options_to_core(&options),
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
            if let Some(expected_version) = expected_version {
                state
                    .service()
                    .assert_session_version(session_id, expected_version)
                    .await
                    .map_err(server_error_from_http)?;
            }
            let session = manager
                .rewind_session(agena::session::SessionRewindRequest {
                    session_id,
                    message_id,
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
        Command::ReplyPermission(ReplyPermissionParams {
            session_id,
            options,
            reply,
        }) => {
            let request = SessionPermissionReplyRequest {
                session_id,
                options: run_options_to_core(&options),
                reply,
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
                options: run_options_to_core(&options),
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
        Command::UpsertPermissionRule(UpsertPermissionRuleParams { action_key, mode }) => {
            let rule = state
                .service()
                .create_permission_rule(PermissionRuleWriteRequest { action_key, mode })
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
        Query::ListMessages(ListMessagesParams { session_id, .. }) => {
            let session = manager.get_session(session_id).await?;
            let items: Vec<_> = session
                .messages
                .iter()
                .map(|m| agena_api::resource::MessageResource {
                    id: m.id,
                    session_id: session.id,
                    role: m.role,
                    state: m.state,
                    created_at: m.created_at,
                    updated_at: m.created_at,
                    metadata: m.metadata.clone(),
                    usage: m.usage.clone(),
                    finish: m.finish.clone(),
                    part_count: m.parts.len() as u64,
                    parts: Some(m.parts.clone()),
                })
                .collect();
            let returned = items.len() as u64;
            Ok(QueryResult::Messages(PaginatedResponse {
                items,
                page: PageInfo {
                    next_cursor: None,
                    has_more: false,
                    returned,
                },
            }))
        }
        Query::GetMessage(GetMessageParams { message_id, .. }) => {
            let session_id = manager
                .find_session_id_for_message(message_id)
                .await?
                .ok_or_else(|| ServerError::NotFound(format!("message {message_id} not found")))?;
            let session = manager.get_session(session_id).await?;
            let m = session
                .messages
                .iter()
                .find(|m| m.id == message_id)
                .ok_or_else(|| ServerError::NotFound(format!("message {message_id}")))?;
            Ok(QueryResult::Message(agena_api::resource::MessageResource {
                id: m.id,
                session_id: session.id,
                role: m.role,
                state: m.state,
                created_at: m.created_at,
                updated_at: m.created_at,
                metadata: m.metadata.clone(),
                usage: m.usage.clone(),
                finish: m.finish.clone(),
                part_count: m.parts.len() as u64,
                parts: Some(m.parts.clone()),
            }))
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
