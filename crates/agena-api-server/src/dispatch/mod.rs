//! Command/Query dispatch into the underlying [`agena::session::SessionManager`].
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport.

use std::collections::HashMap;

use crate::local_api::{
    MessageListQuery, ModelCatalogResponse as HttpModelCatalogResponse, PermissionRuleListQuery,
    PermissionRuleResource as HttpPermissionRuleResource, PermissionRuleWriteRequest,
    SessionAutomationResource as HttpSessionAutomationResource,
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
        SessionCompactRequest, SessionContinueRequest, SessionPermissionReplyRequest,
        SessionUserInputReplyRequest, SessionUserMessageRequest,
    },
};
use agena_api::{
    commands::{
        CancelRunParams, ClearSessionGoalParams, Command, CommandResult, CompactSessionParams,
        CompleteSessionGoalParams, ContinueRunParams, CreateSessionGoalParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, ExportSessionParams, ForkSessionParams, ImportSessionParams,
        ListRewindCheckpointsParams, ListSessionTreeParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, SetSessionGoalParams, SubmitMessageParams,
        UpdateSessionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    pagination::{PageInfo, PaginatedResponse, normalize_limit},
    queries::{
        GetMessageParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
        ListEventsParams, ListMessagesParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams,
        PaginatedEvents, Query, QueryResult,
    },
    resource::{
        ModelCatalogResponse, RunOptions, RuntimeAgentResource, RuntimeAgentsResource,
        RuntimeAutomationResource, RuntimeLspResource, RuntimeLspServerResource,
        RuntimeMcpResource, RuntimeMcpServerResource, RuntimeOperatorResource,
        RuntimeSessionCacheResource, RuntimeSkillResource, RuntimeSkillsResource,
        RuntimeStatusResponse, RuntimeTaskResource, SessionAutomationResource,
        SessionExecutionContextResource, SessionExecutionResource, SessionGoalResource,
        SessionResource, SessionRunState, SessionUsageLimitBasis, SessionUsageResource,
        WorkspaceResource,
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
                thinking_mode: options.thinking_mode.clone(),
                speed_mode: options.speed_mode.clone(),
                verbosity: options.verbosity.clone(),
                parallel_tool_calls: options.parallel_tool_calls,
                agent_profile: options.agent_profile.clone(),
                system: options.system.clone(),
                temperature: options.temperature,
                max_output_tokens: options.max_output_tokens,
                max_run_loops: options.max_run_loops,
            },
        )
        .await
        .server()
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

trait HttpApiResultExt<T> {
    fn server(self) -> Result<T, ServerError>;
}

impl<T> HttpApiResultExt<T> for Result<T, crate::local_api::ApiError> {
    fn server(self) -> Result<T, ServerError> {
        self.map_err(server_error_from_http)
    }
}

impl From<HttpWorkspaceResource> for WorkspaceResource {
    fn from(value: HttpWorkspaceResource) -> Self {
        Self {
            id: value.id,
            path: value.path,
            created_at: value.created_at,
            updated_at: value.updated_at,
            session_count: value.session_count,
        }
    }
}

impl From<HttpSessionGoalResource> for SessionGoalResource {
    fn from(value: HttpSessionGoalResource) -> Self {
        Self {
            id: value.id,
            session_id: value.session_id,
            objective: value.objective,
            status: value.status,
            created_at: value.created_at,
            updated_at: value.updated_at,
            completed_at: value.completed_at,
        }
    }
}

impl From<HttpSessionResource> for SessionResource {
    fn from(value: HttpSessionResource) -> Self {
        Self {
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
            goal: value.goal.map(Into::into),
        }
    }
}

impl From<HttpSessionAutomationResource> for SessionAutomationResource {
    fn from(value: HttpSessionAutomationResource) -> Self {
        Self {
            job_count: value.job_count,
            latest_job: value.latest_job.map(Into::into),
        }
    }
}

impl From<crate::local_api::ScheduledJobRunResource>
    for agena_api::resource::ScheduledJobRunResource
{
    fn from(value: crate::local_api::ScheduledJobRunResource) -> Self {
        Self {
            triggered_at: value.triggered_at,
            finished_at: value.finished_at,
            status: value.status,
            session_id: value.session_id,
            error_message: value.error_message,
        }
    }
}

impl From<crate::local_api::ScheduledJobResource> for agena_api::resource::ScheduledJobResource {
    fn from(value: crate::local_api::ScheduledJobResource) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            expression: value.expression,
            at: value.at,
            prompt: value.prompt,
            owner_session_id: value.owner_session_id,
            next_fire_at: value.next_fire_at,
            last_fired_at: value.last_fired_at,
            last_run: value.last_run.map(Into::into),
        }
    }
}

impl From<HttpModelCatalogResponse> for ModelCatalogResponse {
    fn from(value: HttpModelCatalogResponse) -> Self {
        Self {
            last_refresh_at: value.last_refresh_at,
            last_successful_source: value.last_successful_source,
            last_error: value.last_error,
            entry_count: value.entry_count,
            official_entry_count: value.official_entry_count,
            custom_entry_count: value.custom_entry_count,
        }
    }
}

impl From<HttpSessionExecutionContextResource> for SessionExecutionContextResource {
    fn from(value: HttpSessionExecutionContextResource) -> Self {
        Self {
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
            model_thinking_mode: value.model_thinking_mode,
            model_speed_mode: value.model_speed_mode,
            model_verbosity: value.model_verbosity,
            model_parallel_tool_calls: value.model_parallel_tool_calls,
            agent_run: value.agent_run,
            effective_workspace_root: value.effective_workspace_root,
            task_id: value.task_id,
        }
    }
}

impl From<crate::local_api::SessionRunState> for SessionRunState {
    fn from(value: crate::local_api::SessionRunState) -> Self {
        match value {
            crate::local_api::SessionRunState::Idle => Self::Idle,
            crate::local_api::SessionRunState::AwaitingModel => Self::AwaitingModel,
        }
    }
}

impl From<crate::local_api::SessionUsageLimitBasis> for SessionUsageLimitBasis {
    fn from(value: crate::local_api::SessionUsageLimitBasis) -> Self {
        match value {
            crate::local_api::SessionUsageLimitBasis::ContextWindow => Self::ContextWindow,
            crate::local_api::SessionUsageLimitBasis::PromptThreshold => Self::PromptThreshold,
        }
    }
}

impl From<crate::local_api::SessionUsageResource> for SessionUsageResource {
    fn from(value: crate::local_api::SessionUsageResource) -> Self {
        Self {
            measured_prompt_tokens: value.measured_prompt_tokens,
            current_tokens: value.current_tokens,
            projected_tokens: value.projected_tokens,
            limit_tokens: value.limit_tokens,
            limit_basis: value.limit_basis.map(Into::into),
            reserved_tokens: value.reserved_tokens,
            model_context_window_tokens: value.model_context_window_tokens,
            model_max_input_tokens: value.model_max_input_tokens,
            model_max_output_tokens: value.model_max_output_tokens,
        }
    }
}

impl From<HttpSessionExecutionResource> for SessionExecutionResource {
    fn from(value: HttpSessionExecutionResource) -> Self {
        Self {
            session: value.session.into(),
            blocked: value.blocked,
            run_state: value.run_state.into(),
            latest_event_seq: value.latest_event_seq,
            automation: value.automation.map(Into::into),
            execution: value.execution.into(),
            pending_interactive_requests: value.pending_interactive_requests,
            pending_permission_requests: value.pending_permission_requests,
            pending_user_input_requests: value.pending_user_input_requests,
            goal: value.goal.map(Into::into),
            usage: value.usage.into(),
        }
    }
}

impl From<HttpPermissionRuleResource> for agena_api::resource::PermissionRuleResource {
    fn from(value: HttpPermissionRuleResource) -> Self {
        Self {
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
}

fn page_from_http<T, U>(value: crate::local_api::PaginatedResponse<T>) -> PaginatedResponse<U>
where
    T: Into<U>,
{
    PaginatedResponse {
        items: value.items.into_iter().map(Into::into).collect(),
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
    let model_catalog: ModelCatalogResponse = crate::local_api::ModelCatalogResponse {
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error,
        entry_count: catalog.entries.len(),
        official_entry_count: catalog
            .entries
            .iter()
            .filter(|entry| !entry.has_local_override)
            .count(),
        custom_entry_count: catalog
            .entries
            .iter()
            .filter(|entry| entry.has_local_override)
            .count(),
    }
    .into();
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
        let default_agent = resolution
            .config
            .default
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned)
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
                    default: agena_api::resource::RuntimeAgentDefaultResource {
                        provider: entry.default.provider,
                        adapter: entry.default.adapter,
                        model: entry.default.model,
                    },
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
                .map(Into::into)
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
            ui: snapshot.plugin_manager().ui_catalog(),
        },
    }
}

mod commands;
mod queries;

pub use commands::dispatch_command;
pub use queries::dispatch_query;
