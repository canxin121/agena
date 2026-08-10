//! Command/query dispatch through application and runtime service ports.
//! REST and WS handlers funnel through these helpers so semantics stay
//! identical regardless of transport without naming a concrete session manager.

use std::future::Future;

use crate::{
    Application, ApplicationError,
    dto::{
        CursorPaginationQuery, ModelCatalogResponse as ApplicationModelCatalogResponse,
        PermissionRuleWriteRequest, SearchPaginationQuery, SessionListQuery, WorkspaceListQuery,
        WorkspacePathRequest, WorkspaceResolveRequest,
    },
    pagination::PaginatedResponse as ApplicationPaginatedResponse,
    service::{scheduled_job_resource, sort_jobs_for_display},
};
use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, CompactSessionParams, ContinueRunParams,
        CreateSessionParams, CreateWorkspaceParams, DeletePermissionRuleParams,
        DeleteSessionParams, DeleteWorkspaceParams, DismissActivityParams, ExportSessionParams,
        ForkSessionParams, ImportSessionParams, ListSessionTreeParams,
        MarkInteractiveRequestPresentedParams, ReplacePermissionRuleParams, ReplyPermissionParams,
        ReplyUserInputParams, ResolveWorkspaceParams, RevokePermissionRuleParams,
        RewindSessionParams, StopActivityParams, SubmitRunParams, UpdateSessionParams,
        UpdateSessionSelectionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    pagination::{PageInfo, PaginatedResponse},
    queries::{
        ActivityLogsParams, GetActivityParams, GetOperationDetailParams, GetPermissionRuleParams,
        GetSessionParams, GetWorkspaceParams, ListActivitiesParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
        QueryResult,
    },
    resource::{
        DefaultSelectionResource, ModelCatalogResponse, ModelCatalogSourceKind,
        OperationDetailResource, RuntimeAutomationResource, RuntimeLspResource,
        RuntimeLspServerResource, RuntimeMcpResource, RuntimeMcpServerResource,
        RuntimeOperatorResource, RuntimePluginUiResource, RuntimeSessionCacheResource,
        RuntimeSkillResource, RuntimeSkillsResource, RuntimeStatusResponse, RuntimeTaskResource,
    },
};

const fn model_catalog_source_kind_from_domain(
    value: agena_provider::ModelCatalogSnapshotSourceKind,
) -> ModelCatalogSourceKind {
    match value {
        agena_provider::ModelCatalogSnapshotSourceKind::Generated => {
            ModelCatalogSourceKind::Generated
        }
        agena_provider::ModelCatalogSnapshotSourceKind::Cache => ModelCatalogSourceKind::Cache,
    }
}

trait ApplicationResultExt<T> {
    fn application(self) -> Result<T, ApplicationError>;
}

impl<T> ApplicationResultExt<T> for Result<T, ApplicationError> {
    fn application(self) -> Result<T, ApplicationError> {
        self
    }
}

trait IntoWire<T> {
    fn into_wire(self) -> T;
}

impl<T> IntoWire<T> for T {
    fn into_wire(self) -> T {
        self
    }
}

impl IntoWire<ModelCatalogResponse> for ApplicationModelCatalogResponse {
    fn into_wire(self) -> ModelCatalogResponse {
        let value = self;
        ModelCatalogResponse {
            refreshing: value.refreshing,
            last_refresh_at: value.last_refresh_at,
            last_successful_source: value
                .last_successful_source
                .map(model_catalog_source_kind_from_domain),
            last_failure: value.last_failure,
            model_count: value.model_count,
        }
    }
}

fn page_from_application<T, U>(value: ApplicationPaginatedResponse<T>) -> PaginatedResponse<U>
where
    T: IntoWire<U>,
{
    PaginatedResponse {
        items: value.items.into_iter().map(IntoWire::into_wire).collect(),
        page: PageInfo {
            next_cursor: value.page.next_cursor,
            has_more: value.page.has_more,
            returned: value.page.returned as u64,
        },
    }
}

async fn http_page_result<T, U>(
    future: impl Future<Output = Result<ApplicationPaginatedResponse<T>, ApplicationError>>,
) -> Result<PaginatedResponse<U>, ApplicationError>
where
    T: IntoWire<U>,
{
    Ok(page_from_application(future.await.application()?))
}

async fn http_optional_result<T, U>(
    future: impl Future<Output = Result<Option<T>, ApplicationError>>,
    not_found: impl FnOnce() -> String,
) -> Result<U, ApplicationError>
where
    T: IntoWire<U>,
{
    future
        .await
        .application()?
        .map(IntoWire::into_wire)
        .ok_or_else(|| {
            ApplicationError::not_found_with_diagnostic("The resource was not found.", not_found())
        })
}

async fn runtime_status_response(state: &Application) -> RuntimeStatusResponse {
    let status = state.runtime_status().runtime_status().await;
    let catalog = status.model_catalog;
    let model_catalog: ModelCatalogResponse = ApplicationModelCatalogResponse {
        refreshing: status.model_catalog_refreshing,
        last_refresh_at: catalog.last_refresh_at,
        last_successful_source: catalog.last_successful_source,
        last_failure: catalog.last_failure.map(Into::into),
        model_count: catalog.models.len(),
    }
    .into_wire();
    let background_tasks = status
        .background_tasks
        .into_iter()
        .map(crate::dto::runtime_background_task_resource)
        .collect();
    let session_cache = status
        .session_cache
        .map(|stats| RuntimeSessionCacheResource {
            max_sessions: agena_domain::SessionCacheLimits::default().max_sessions,
            ttl_secs: agena_domain::SessionCacheLimits::default().ttl_secs,
            max_bytes: agena_domain::SessionCacheLimits::default().max_bytes,
            session_count: stats.session_count,
            total_bytes: stats.total_bytes,
            hits: stats.hits,
            misses: stats.misses,
            inserts: stats.inserts,
            evictions: stats.evictions,
        });

    let mcp = RuntimeMcpResource {
        server_count: status.mcp.servers.len(),
        tool_count: status
            .mcp
            .servers
            .iter()
            .map(|server| server.tool_count)
            .sum(),
        servers: status
            .mcp
            .servers
            .into_iter()
            .map(|server| RuntimeMcpServerResource {
                name: server.name,
                tool_count: server.tool_count,
            })
            .collect(),
    };
    let lsp = RuntimeLspResource {
        server_count: status.lsp.servers.len(),
        diagnostics_count: status.lsp.diagnostics_count,
        files_with_diagnostics: status.lsp.files_with_diagnostics,
        servers: status
            .lsp
            .servers
            .into_iter()
            .map(|server| RuntimeLspServerResource {
                name: server.name,
                command: server.command,
                file_extensions: server.file_extensions,
                root_markers: server.root_markers,
            })
            .collect(),
    };
    let skills = RuntimeSkillsResource {
        skill_count: status.skills.skills.len(),
        command_count: status.skills.commands.len(),
        skills: status
            .skills
            .skills
            .into_iter()
            .map(|item| RuntimeSkillResource {
                name: item.name,
                description: item.description,
                aliases: item.aliases,
                source_path: item.source_path,
            })
            .collect(),
        commands: status
            .skills
            .commands
            .into_iter()
            .map(|item| RuntimeSkillResource {
                name: item.name,
                description: item.description,
                aliases: item.aliases,
                source_path: item.source_path,
            })
            .collect(),
    };
    let mut jobs = status.scheduled_jobs;
    sort_jobs_for_display(&mut jobs);
    let automation = RuntimeAutomationResource {
        enabled: status.automation_available,
        job_count: jobs.len(),
        recent_jobs: jobs
            .into_iter()
            .take(10)
            .map(scheduled_job_resource)
            .collect(),
    };
    let default_selection = {
        let selection = state.provider_catalog().default_selection();
        (!selection.is_empty()).then_some(DefaultSelectionResource {
            provider: selection.provider,
            adapter: selection.adapter,
            model: selection.model,
            thinking_mode: selection.thinking_mode,
            speed_mode: selection.speed_mode,
        })
    };

    RuntimeStatusResponse {
        generation: status.generation,
        loaded_at: status.loaded_at,
        workspace_root: status.workspace_root.display().to_string(),
        config_path: status.config_path.display().to_string(),
        config_found: status.config_found,
        provider_ids: status.provider_ids,
        plugin_count: status.plugin_count,
        session_runtime_available: status.session_runtime_available,
        watch_paths: status
            .watch_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        reload: RuntimeTaskResource {
            enabled: status.reload_enabled,
            interval_secs: status.reload_interval_secs,
        },
        session_gc: RuntimeTaskResource {
            enabled: status.session_gc_enabled,
            interval_secs: status.session_gc_interval_secs,
        },
        session_cache,
        model_catalog: Some(model_catalog),
        default_selection,
        background_tasks,
        automation,
        operator: RuntimeOperatorResource {
            mcp,
            lsp,
            agent_id: status.agent_id,
            skills,
            ui: RuntimePluginUiResource {
                catalog: plugin_ui_catalog_resource_from_domain(status.plugin_ui_catalog),
                tool_registry_generation: status.tool_registry_generation,
                tool_registry_last_event: status.tool_registry_last_event,
            },
        },
    }
}

fn plugin_ui_catalog_resource_from_domain(
    value: agena_plugin_host::PluginUiCatalog,
) -> agena_api::resource::PluginUiCatalogResource {
    use agena_api::resource::{
        PluginCommandResource, PluginDisplayContributionResource,
        PluginStudioControlOptionResource, PluginStudioControlResource,
        PluginStudioUiCatalogResource, PluginStudioViewResource, PluginThemeColorsResource,
        PluginThemePaletteResource, PluginTuiUiCatalogResource, PluginUiCatalogResource,
    };

    PluginUiCatalogResource {
        tui: PluginTuiUiCatalogResource {
            display: value
                .tui
                .display
                .into_iter()
                .map(|item| PluginDisplayContributionResource {
                    plugin_id: item.plugin_id.to_string(),
                    id: item.contribution.id,
                    kind: item.contribution.kind,
                    priority: item.contribution.priority,
                    content: item.contribution.content,
                })
                .collect(),
            themes: value
                .tui
                .themes
                .into_iter()
                .map(|theme| PluginThemePaletteResource {
                    id: theme.id,
                    plugin_id: theme.plugin_id.to_string(),
                    display_name: theme.display_name,
                    colors: PluginThemeColorsResource {
                        muted: theme.colors.muted.map(|color| color.as_str().to_owned()),
                        accent: theme.colors.accent.map(|color| color.as_str().to_owned()),
                        info: theme.colors.info.map(|color| color.as_str().to_owned()),
                        success: theme.colors.success.map(|color| color.as_str().to_owned()),
                        warning: theme.colors.warning.map(|color| color.as_str().to_owned()),
                        danger: theme.colors.danger.map(|color| color.as_str().to_owned()),
                        special: theme.colors.special.map(|color| color.as_str().to_owned()),
                        selection_fg: theme
                            .colors
                            .selection_fg
                            .map(|color| color.as_str().to_owned()),
                        selection_bg: theme
                            .colors
                            .selection_bg
                            .map(|color| color.as_str().to_owned()),
                    },
                })
                .collect(),
        },
        studio: PluginStudioUiCatalogResource {
            commands: value
                .studio
                .commands
                .into_iter()
                .map(|item| PluginCommandResource {
                    plugin_id: item.plugin_id.to_string(),
                    id: item.command.id,
                    title: item.command.title,
                    description: item.command.description,
                    category: item.command.category,
                    slash: item.command.slash,
                    aliases: item.command.aliases,
                    usage: item.command.usage,
                    location: item.command.location,
                    input_schema: item.command.input_schema,
                    handler: item.command.handler,
                    action: plugin_ui_action_resource_from_domain(item.command.action),
                })
                .collect(),
            controls: value
                .studio
                .controls
                .into_iter()
                .map(plugin_studio_control_resource_from_domain)
                .collect(),
            views: value
                .studio
                .views
                .into_iter()
                .map(|item| PluginStudioViewResource {
                    plugin_id: item.plugin_id.to_string(),
                    id: item.view.id,
                    title: item.view.title,
                    description: item.view.description,
                    location: item.view.location,
                    kind: item.view.kind,
                    content: item.view.content,
                    url: item.view.url,
                    controls: item
                        .view
                        .controls
                        .into_iter()
                        .map(|control| PluginStudioControlResource {
                            plugin_id: item.plugin_id.to_string(),
                            id: control.id,
                            title: control.title,
                            description: control.description,
                            location: control.location,
                            kind: control.kind,
                            options: control
                                .options
                                .into_iter()
                                .map(|option| PluginStudioControlOptionResource {
                                    label: option.label,
                                    value: option.value,
                                    description: option.description,
                                })
                                .collect(),
                            value: control.value,
                            action: plugin_ui_action_resource_from_domain(control.action),
                        })
                        .collect(),
                })
                .collect(),
        },
    }
}

fn plugin_studio_control_resource_from_domain(
    item: agena_plugin_host::PluginStudioControlCatalogItem,
) -> agena_api::resource::PluginStudioControlResource {
    agena_api::resource::PluginStudioControlResource {
        plugin_id: item.plugin_id.to_string(),
        id: item.control.id,
        title: item.control.title,
        description: item.control.description,
        location: item.control.location,
        kind: item.control.kind,
        options: item
            .control
            .options
            .into_iter()
            .map(
                |option| agena_api::resource::PluginStudioControlOptionResource {
                    label: option.label,
                    value: option.value,
                    description: option.description,
                },
            )
            .collect(),
        value: item.control.value,
        action: plugin_ui_action_resource_from_domain(item.control.action),
    }
}

fn plugin_ui_action_resource_from_domain(
    value: agena_plugin_host::sdk::PluginUiAction,
) -> agena_api::resource::PluginUiActionResource {
    match value {
        agena_plugin_host::sdk::PluginUiAction::None => {
            agena_api::resource::PluginUiActionResource::None
        }
        agena_plugin_host::sdk::PluginUiAction::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        } => agena_api::resource::PluginUiActionResource::InvokeTool {
            tool,
            input,
            submit_output_as_prompt,
        },
        agena_plugin_host::sdk::PluginUiAction::OpenPluginWorkbench { tab } => {
            agena_api::resource::PluginUiActionResource::OpenPluginWorkbench { tab }
        }
        agena_plugin_host::sdk::PluginUiAction::OpenUrl { url } => {
            agena_api::resource::PluginUiActionResource::OpenUrl { url }
        }
        agena_plugin_host::sdk::PluginUiAction::SubmitPrompt { prompt } => {
            agena_api::resource::PluginUiActionResource::SubmitPrompt { prompt }
        }
        agena_plugin_host::sdk::PluginUiAction::InvokeCommand { command, input } => {
            agena_api::resource::PluginUiActionResource::InvokeCommand { command, input }
        }
    }
}

mod commands;
mod queries;

pub use commands::dispatch_command;
pub use queries::dispatch_query;
