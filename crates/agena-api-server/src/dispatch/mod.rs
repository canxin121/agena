//! Command/Query dispatch into the underlying [`agena::session::SessionManager`].
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport.

use std::{collections::HashMap, future::Future};

use crate::local_api::{
    CursorPaginationQuery, MessageListQuery, ModelCatalogResponse as HttpModelCatalogResponse,
    PermissionRuleResource as HttpPermissionRuleResource, PermissionRuleWriteRequest,
    SearchPaginationQuery, SessionCreateRequest as HttpSessionCreateRequest,
    SessionHierarchyRequest, SessionListQuery, WorkspaceListQuery, WorkspacePathRequest,
    WorkspaceResolveRequest, WorkspaceResource as HttpWorkspaceResource,
};
use agena::event::EventKind;
use agena::event::{EventStore, StoreRange};
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
        ModelCatalogResponse, RuntimeAgentResource, RuntimeAgentsResource,
        RuntimeAutomationResource, RuntimeBackgroundTaskResource, RuntimeLspResource,
        RuntimeLspServerResource, RuntimeMcpResource, RuntimeMcpServerResource,
        RuntimeOperatorResource, RuntimeSessionCacheResource, RuntimeSkillResource,
        RuntimeSkillsResource, RuntimeStatusResponse, RuntimeTaskResource, WorkspaceResource,
    },
};

use crate::session_support::server_error_from_http;
use crate::{error::ServerError, state::AppState};

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

impl From<HttpModelCatalogResponse> for ModelCatalogResponse {
    fn from(value: HttpModelCatalogResponse) -> Self {
        Self {
            refreshing: value.refreshing,
            last_refresh_at: value.last_refresh_at,
            last_successful_source: value.last_successful_source,
            last_error: value.last_error,
            entry_count: value.entry_count,
        }
    }
}

impl From<crate::local_api::RuntimeBackgroundTaskResource> for RuntimeBackgroundTaskResource {
    fn from(value: crate::local_api::RuntimeBackgroundTaskResource) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            origin: value.origin,
            title: value.title,
            status: value.status,
            message: value.message,
            error_message: value.error_message,
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            cancellable: value.cancellable,
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

async fn http_page_result<T, U>(
    future: impl Future<
        Output = Result<crate::local_api::PaginatedResponse<T>, crate::local_api::ApiError>,
    >,
) -> Result<PaginatedResponse<U>, ServerError>
where
    T: Into<U>,
{
    Ok(page_from_http(future.await.server()?))
}

async fn http_optional_result<T, U>(
    future: impl Future<Output = Result<Option<T>, crate::local_api::ApiError>>,
    not_found: impl FnOnce() -> String,
) -> Result<U, ServerError>
where
    T: Into<U>,
{
    future
        .await
        .server()?
        .map(Into::into)
        .ok_or_else(|| ServerError::NotFound(not_found()))
}

async fn runtime_status_response(state: &AppState) -> RuntimeStatusResponse {
    let snapshot = state.runtime().current_snapshot();
    let resolution = snapshot.config_resolution();
    let mut provider_ids = snapshot.provider_registry().provider_ids();
    provider_ids.sort();
    let catalog = snapshot.model_catalog_response();
    let model_catalog: ModelCatalogResponse = crate::local_api::ModelCatalogResponse {
        refreshing: state.runtime().model_catalog_refresh_active(),
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_error: catalog.last_error,
        entry_count: catalog.entries.len(),
    }
    .into();
    let background_tasks = state
        .runtime()
        .background_tasks()
        .into_iter()
        .map(crate::local_api::RuntimeBackgroundTaskResource::from)
        .map(Into::into)
        .collect();
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
        background_tasks,
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
