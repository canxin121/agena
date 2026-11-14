//! Shared session queries backed by the composed runtime's application services.

use super::*;
use agena_session_api::{
    activity::{BackgroundActivityLogResource, BackgroundActivityResource},
    pagination::{PageInfo, PaginatedResponse, normalize_resource_limit},
    queries::Query,
    resource::{PermissionMode, PermissionRuleResource, WorkspaceResource, WorkspaceSessionStats},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[path = "session_content.rs"]
mod session_content;

impl RuntimeHostClient {
    pub(super) async fn list_plugin_sessions(
        &self,
        params: agena_session_api::queries::ListSessionsParams,
    ) -> Result<Value, PluginError> {
        use agena_session_api::queries::SessionListBucket;
        use agena_storage::kernel::{SessionSummaryCursor, SummaryQuery};

        let before = params
            .cursor
            .as_deref()
            .map(|cursor| {
                let bytes = URL_SAFE_NO_PAD
                    .decode(cursor)
                    .map_err(|error| PluginError::invalid_params_error(&error))?;
                serde_json::from_slice::<SessionSummaryCursor>(&bytes)
                    .map_err(|error| PluginError::invalid_params_error(&error))
            })
            .transpose()?;
        if before.is_some() && params.offset > 0 {
            return Err(PluginError::invalid_params(
                "use either cursor or offset pagination",
            ));
        }
        let services = self.runtime()?.application_services();
        let kernel = services
            .session_kernel
            .as_ref()
            .ok_or_else(|| host_unavailable("session kernel is not enabled"))?;
        let page = kernel
            .reader
            .query_summaries(SummaryQuery {
                workspace_id: params.workspace_id,
                include_global: params.parent_id.is_some(),
                owner_bot_id: params.owner_bot_id.map(|id| id.to_string()),
                owner_surface: params
                    .owner_surface
                    .map(|surface| surface.as_str().to_owned()),
                parent_id: params.parent_id.map(kernel_session_id).transpose()?,
                roots_only: params.roots,
                exclude_subagents: params.exclude_subagents,
                temporary_only: false,
                agents_only: false,
                search: non_empty(params.search),
                before,
                offset: params.offset,
                limit: normalize_resource_limit(params.limit) as usize,
                favorite: (params.bucket == Some(SessionListBucket::Favorite)).then_some(true),
                pinned: (params.bucket == Some(SessionListBucket::Pinned)).then_some(true),
                states: match params.bucket {
                    Some(SessionListBucket::Running) => vec![
                        agena_domain::SessionStateKind::Running,
                        agena_domain::SessionStateKind::Creating,
                    ],
                    Some(SessionListBucket::Attention) => vec![
                        agena_domain::SessionStateKind::AwaitingInteraction,
                        agena_domain::SessionStateKind::Failed,
                    ],
                    Some(SessionListBucket::Recent) => vec![agena_domain::SessionStateKind::Ready],
                    _ => Vec::new(),
                },
            })
            .await
            .map_err(plugin_error)?;
        // Retain the plugin API's array response and summary fields. Filtering
        // happens in the kernel before pagination, across the requested workspace.
        let rows = page
            .sessions
            .iter()
            .map(|summary| {
                Ok(agena_domain::SessionSummary {
                    id: summary.id.get(),
                    parent_id: summary.parent_session_id.map(|id| id.get()),
                    depth: i64::from(summary.depth),
                    root_id: summary.root_session_id.get(),
                    workspace_id: summary.workspace_id,
                    title: summary.title.clone(),
                    favorite: summary.favorite,
                    pinned: summary.pinned,
                    version: summary.metadata_version.get(),
                    relation_kind: summary.relation_kind,
                    lifecycle_state: agena_domain::SessionLifecycleState::Ready,
                    source_cutoff_seq_global: None,
                    source_message_id: None,
                    agent_name: summary.agent_name.clone(),
                    agent_status: summary.agent_status,
                    created_at: timestamp_millis(summary.created_at_ms)?,
                    updated_at: timestamp_millis(summary.updated_at_ms)?,
                    message_count: summary.user_message_count,
                    child_session_count: summary.child_session_count,
                    last_message_at: summary
                        .last_message_at_ms
                        .map(timestamp_millis)
                        .transpose()?,
                })
            })
            .collect::<Result<Vec<_>, PluginError>>()?;
        serde_json::to_value(rows).map_err(plugin_error)
    }

    pub(super) async fn run_global_session_query(
        &self,
        query: Query,
    ) -> Result<Value, PluginError> {
        let services = self.runtime()?.application_services();
        match query {
            Query::Health => {
                let status = services.status.runtime_status().await;
                let database_connected = match services.repositories.as_ref() {
                    Some(repositories) => repositories
                        .workspace
                        .list(agena_storage::WorkspaceListQuery {
                            limit: 1,
                            ..Default::default()
                        })
                        .await
                        .is_ok(),
                    None => false,
                };
                Ok(json!({
                    "status": if database_connected { "ok" } else { "degraded" },
                    "generation": status.generation,
                    "loaded_at": status.loaded_at,
                    "database_connected": database_connected,
                    "instance_id": services.instance_id,
                }))
            }
            Query::Runtime => {
                let status = services.status.runtime_status().await;
                let (scheduled_jobs, scheduled_jobs_error) = match status.scheduled_jobs {
                    Ok(jobs) => (Some(jobs), None),
                    Err(error) => (None, Some(error.to_string())),
                };
                Ok(json!({
                    "generation": status.generation,
                    "loaded_at": status.loaded_at,
                    "workspace_root": status.workspace_root,
                    "config_path": status.config_path,
                    "config_found": status.config_found,
                    "provider_ids": status.provider_ids,
                    "plugin_count": status.plugin_count,
                    "session_runtime_available": status.session_runtime_available,
                    "watch_paths": status.watch_paths,
                    "reload_enabled": status.reload_enabled,
                    "reload_interval_secs": status.reload_interval_secs,
                    "session_gc_enabled": status.session_gc_enabled,
                    "session_gc_interval_secs": status.session_gc_interval_secs,
                    "model_catalog": status.model_catalog,
                    "model_catalog_refreshing": status.model_catalog_refreshing,
                    "background_tasks": status.background_tasks,
                    "automation_available": status.automation_available,
                    "scheduled_jobs": scheduled_jobs,
                    "scheduled_jobs_error": scheduled_jobs_error,
                    "mcp": status.mcp,
                    "lsp": {
                        "diagnostics_count": status.lsp.diagnostics_count,
                        "files_with_diagnostics": status.lsp.files_with_diagnostics,
                        "servers": status.lsp.servers.iter().map(|server| json!({
                            "name": server.name,
                            "command": server.command,
                            "file_extensions": server.file_extensions,
                            "root_markers": server.root_markers,
                        })).collect::<Vec<_>>(),
                    },
                    "commands": {
                        "commands": status.commands.commands.iter().map(|command| json!({
                            "name": command.name,
                            "slash": command.slash,
                            "description": command.description,
                            "aliases": command.aliases,
                            "group": command.group,
                            "category": command.category,
                            "plugin_id": command.plugin_id,
                            "target": command.target,
                        })).collect::<Vec<_>>(),
                    },
                    "agent_id": status.agent_id,
                    "plugin_surface_catalog": status.plugin_surface_catalog,
                    "tool_registry_generation": status.tool_registry_generation,
                    "tool_registry_last_event": status.tool_registry_last_event,
                }))
            }
            Query::ListProviders => Ok(Value::Array(
                services
                    .provider_catalog
                    .list_providers()
                    .into_iter()
                    .map(|provider| {
                        json!({
                            "provider_id": provider.provider_id,
                            "adapters": provider.adapters.into_iter().map(|adapter| json!({
                                "adapter_id": adapter.adapter_id,
                                "enabled": adapter.enabled,
                                "configured_model_count": adapter.configured_model_count,
                            })).collect::<Vec<_>>(),
                        })
                    })
                    .collect(),
            )),
            Query::ListProviderModels(params) => {
                let provider_id = agena_domain::ProviderId::new(params.provider_id);
                if !services.provider_catalog.contains_provider(&provider_id) {
                    return Err(PluginError::invalid_params(format!(
                        "provider {provider_id} not found"
                    )));
                }
                let discovery = services
                    .provider_catalog
                    .discover_models(&provider_id)
                    .await;
                let models = match discovery {
                    Ok(discovery) if !discovery.models.is_empty() => discovery
                        .models
                        .into_iter()
                        .map(|entry| entry.model)
                        .collect(),
                    Ok(discovery) => {
                        let configured = services
                            .provider_catalog
                            .configured_local_models(&provider_id)
                            .map_err(plugin_error)?;
                        if configured.is_empty() && !discovery.complete {
                            return Err(PluginError::internal(
                                "provider model discovery was incomplete",
                            ));
                        }
                        configured
                    }
                    Err(error) => {
                        let configured = services
                            .provider_catalog
                            .configured_local_models(&provider_id)
                            .map_err(plugin_error)?;
                        if configured.is_empty() {
                            return Err(plugin_error(error));
                        }
                        configured
                    }
                };
                Ok(json!({
                    "provider_id": provider_id,
                    "models": models.into_iter().map(provider_model_resource)
                        .collect::<Result<Vec<_>, _>>()?,
                }))
            }
            Query::ListProviderAdapterModels(params) => {
                use agena_provider::{
                    DraftHttpProviderAdapterModelsRequest, DraftProviderAdapterModelsRequest,
                    ProviderApiKeySource, ProviderProtocolPaths,
                };
                use agena_session_api::queries::ProviderSecretSource;
                let listing = services
                    .provider_catalog
                    .list_draft_adapter_models(DraftProviderAdapterModelsRequest::Http(
                        DraftHttpProviderAdapterModelsRequest {
                            provider_id: params.provider_id,
                            base_url: params.base_url,
                            protocol_paths: ProviderProtocolPaths {
                                openai: params.protocol_paths.openai,
                                anthropic: params.protocol_paths.anthropic,
                                gemini: params.protocol_paths.gemini,
                            },
                            api_key: params.api_key.map(|source| match source {
                                ProviderSecretSource::Inline(value) => {
                                    ProviderApiKeySource::Inline(value)
                                }
                                ProviderSecretSource::Env(name) => {
                                    ProviderApiKeySource::Environment(name)
                                }
                            }),
                            adapter_ids: params.adapter_ids,
                        },
                    ))
                    .await
                    .map_err(plugin_error)?;
                provider_adapter_models_resource(listing)
            }
            Query::ListSavedProviderAdapterModels(params) => {
                let listing = services
                    .provider_catalog
                    .list_saved_adapter_models(
                        &agena_domain::ProviderId::new(params.provider_id),
                        params.adapter_ids,
                    )
                    .await
                    .map_err(plugin_error)?;
                provider_adapter_models_resource(listing)
            }
            Query::ListWorkspaces(params) => {
                let repositories = services
                    .repositories
                    .as_ref()
                    .ok_or_else(|| host_unavailable("application repositories are not enabled"))?;
                let limit = normalize_resource_limit(params.limit);
                let cursor = decode_resource_cursor(params.cursor.as_deref())?;
                if cursor.is_some() && params.offset > 0 {
                    return Err(PluginError::invalid_params(
                        "use either cursor or offset pagination",
                    ));
                }
                let mut rows = repositories
                    .workspace
                    .list(agena_storage::WorkspaceListQuery {
                        offset: params.offset,
                        search: non_empty(params.search),
                        before_updated_at_ms: cursor.map(|cursor| cursor.updated_at_ms),
                        before_id: cursor.map(|cursor| cursor.id),
                        limit: limit + 1,
                    })
                    .await
                    .map_err(plugin_error)?;
                let has_more = rows.len() > limit as usize;
                rows.truncate(limit as usize);
                let stats = if params.include_session_count {
                    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
                    let kernel = services
                        .session_kernel
                        .as_ref()
                        .ok_or_else(|| host_unavailable("session kernel is not enabled"))?;
                    kernel
                        .reader
                        .workspace_statistics(&ids)
                        .await
                        .map_err(plugin_error)?
                        .iter()
                        .map(|(id, stats)| (*id, *stats))
                        .collect::<std::collections::BTreeMap<_, _>>()
                } else {
                    std::collections::BTreeMap::new()
                };
                let items = rows
                    .iter()
                    .map(|row| {
                        let statistics = stats.get(&row.id);
                        let mut item =
                            workspace_resource(row, statistics.map(|value| value.total))?;
                        item.session_stats = statistics.map(|value| WorkspaceSessionStats {
                            total: value.total,
                            roots: value.roots,
                            pinned: value.pinned,
                            running: value.running,
                            attention: value.attention,
                        });
                        Ok(item)
                    })
                    .collect::<Result<Vec<_>, PluginError>>()?;
                resource_page(
                    items,
                    has_more,
                    rows.last().map(|row| ResourceCursor {
                        updated_at_ms: row.updated_at_ms,
                        id: row.id,
                    }),
                )
            }
            Query::GetWorkspace(params) => {
                let repositories = services
                    .repositories
                    .as_ref()
                    .ok_or_else(|| host_unavailable("application repositories are not enabled"))?;
                let row = repositories
                    .workspace
                    .get(params.workspace_id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| {
                        PluginError::invalid_params(format!(
                            "workspace {} not found",
                            params.workspace_id
                        ))
                    })?;
                let kernel = services
                    .session_kernel
                    .as_ref()
                    .ok_or_else(|| host_unavailable("session kernel is not enabled"))?;
                let stats = kernel
                    .reader
                    .workspace_statistics(&[row.id])
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(workspace_resource(
                    &row,
                    stats.get(&row.id).map(|value| value.total),
                )?)
                .map_err(plugin_error)
            }
            Query::ListPermissionRules(params) => {
                let repositories = services
                    .repositories
                    .as_ref()
                    .ok_or_else(|| host_unavailable("application repositories are not enabled"))?;
                let limit = normalize_resource_limit(params.limit);
                let cursor = decode_resource_cursor(params.cursor.as_deref())?;
                let mut rows = repositories
                    .permission_rules
                    .list(agena_storage::PermissionRuleListQuery {
                        search: non_empty(params.search),
                        before_updated_at_ms: cursor.map(|cursor| cursor.updated_at_ms),
                        before_id: cursor.map(|cursor| cursor.id),
                        limit: limit + 1,
                    })
                    .await
                    .map_err(plugin_error)?;
                let has_more = rows.len() > limit as usize;
                rows.truncate(limit as usize);
                let items = rows
                    .iter()
                    .map(permission_rule_resource)
                    .collect::<Result<Vec<_>, _>>()?;
                resource_page(
                    items,
                    has_more,
                    rows.last().map(|row| ResourceCursor {
                        updated_at_ms: row.updated_at_ms,
                        id: row.id,
                    }),
                )
            }
            Query::GetPermissionRule(params) => {
                let repositories = services
                    .repositories
                    .as_ref()
                    .ok_or_else(|| host_unavailable("application repositories are not enabled"))?;
                let row = repositories
                    .permission_rules
                    .get(params.rule_id)
                    .await
                    .map_err(plugin_error)?
                    .ok_or_else(|| {
                        PluginError::invalid_params(format!(
                            "permission rule {} not found",
                            params.rule_id
                        ))
                    })?;
                serde_json::to_value(permission_rule_resource(&row)?).map_err(plugin_error)
            }
            Query::ListActivities(params) => {
                let activities = services
                    .activities
                    .as_ref()
                    .ok_or_else(|| host_unavailable("runtime activities are not enabled"))?;
                let filter = agena_domain::BackgroundActivityFilter {
                    kinds: parse_filters(params.kinds.as_deref()),
                    statuses: parse_filters(params.statuses.as_deref()),
                    session_id: params.session_id,
                    active_only: params.active_only,
                };
                let rows = activities
                    .list_activities(&filter)
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(
                    rows.iter()
                        .map(BackgroundActivityResource::from)
                        .collect::<Vec<_>>(),
                )
                .map_err(plugin_error)
            }
            Query::GetActivity(params) => {
                let activities = services
                    .activities
                    .as_ref()
                    .ok_or_else(|| host_unavailable("runtime activities are not enabled"))?;
                let activity = activities
                    .get_activity(&params.activity_id)
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(BackgroundActivityResource::from(&activity))
                    .map_err(plugin_error)
            }
            Query::ActivityLogs(params) => {
                let activities = services
                    .activities
                    .as_ref()
                    .ok_or_else(|| host_unavailable("runtime activities are not enabled"))?;
                let read = activities
                    .activity_logs(
                        &params.activity_id,
                        params.since_seq,
                        params.limit,
                        params.wait_ms,
                    )
                    .await
                    .map_err(plugin_error)?;
                serde_json::to_value(BackgroundActivityLogResource::from(read))
                    .map_err(plugin_error)
            }
            Query::ReadContentText(params) => {
                let kernel = services
                    .session_kernel
                    .as_ref()
                    .ok_or_else(|| host_unavailable("session kernel is not enabled"))?;
                serde_json::to_value(
                    session_content::read_content_text(&kernel.reader, params).await?,
                )
                .map_err(plugin_error)
            }
            _ => Err(PluginError::invalid_params(
                "query is not a global session query",
            )),
        }
    }
}

#[derive(Clone, Copy, Serialize, Deserialize)]
struct ResourceCursor {
    updated_at_ms: i64,
    id: i64,
}

fn decode_resource_cursor(cursor: Option<&str>) -> Result<Option<ResourceCursor>, PluginError> {
    cursor
        .map(|cursor| {
            let bytes = URL_SAFE_NO_PAD
                .decode(cursor)
                .map_err(|error| PluginError::invalid_params_error(&error))?;
            serde_json::from_slice(&bytes)
                .map_err(|error| PluginError::invalid_params_error(&error))
        })
        .transpose()
}

fn resource_page<T: Serialize>(
    items: Vec<T>,
    has_more: bool,
    cursor: Option<ResourceCursor>,
) -> Result<Value, PluginError> {
    let next_cursor = cursor
        .map(|cursor| serde_json::to_vec(&cursor).map(|bytes| URL_SAFE_NO_PAD.encode(bytes)))
        .transpose()
        .map_err(plugin_error)?;
    serde_json::to_value(PaginatedResponse {
        total: None,
        page: PageInfo {
            returned: items.len() as u64,
            has_more,
            next_cursor,
        },
        items,
    })
    .map_err(plugin_error)
}

fn non_empty(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

fn parse_filters<T: std::str::FromStr>(value: Option<&str>) -> Vec<T> {
    value
        .unwrap_or_default()
        .split(',')
        .filter_map(|value| value.trim().parse().ok())
        .collect()
}

fn timestamp_millis(value: i64) -> Result<chrono::DateTime<chrono::Utc>, PluginError> {
    chrono::DateTime::from_timestamp_millis(value)
        .ok_or_else(|| PluginError::internal(format!("invalid timestamp millis: {value}")))
}

pub(super) fn workspace_resource(
    row: &agena_storage::WorkspaceRecord,
    session_count: Option<u64>,
) -> Result<WorkspaceResource, PluginError> {
    Ok(WorkspaceResource {
        id: row.id,
        path: row.path.clone(),
        created_at: timestamp_millis(row.created_at_ms)?,
        updated_at: timestamp_millis(row.updated_at_ms)?,
        session_count,
        session_stats: None,
    })
}

pub(super) fn permission_rule_resource(
    row: &agena_storage::PermissionRuleRecord,
) -> Result<PermissionRuleResource, PluginError> {
    let action: agena_domain::PermissionAction =
        serde_json::from_str(&row.action_key).map_err(plugin_error)?;
    let (
        subject_kind,
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
    ) = match action {
        agena_domain::PermissionAction::Tool {
            tool_name,
            qualifier,
        } => (
            "tool",
            Some(tool_name),
            qualifier,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        agena_domain::PermissionAction::PathAccess {
            access_kind,
            workspace_root,
            target_path,
        } => (
            "path_access",
            None,
            None,
            Some(access_kind),
            Some(workspace_root),
            Some(target_path),
            None,
            None,
            None,
        ),
        agena_domain::PermissionAction::NetworkAccess { target, host, port } => (
            "network_access",
            None,
            None,
            None,
            None,
            None,
            Some(target),
            Some(host),
            port,
        ),
    };
    let mode = match row.mode.as_str() {
        "allow" => PermissionMode::Allow,
        "auto" => PermissionMode::Auto,
        "ask" => PermissionMode::Ask,
        "deny" => PermissionMode::Deny,
        mode => {
            return Err(PluginError::internal(format!(
                "invalid permission mode: {mode}"
            )));
        }
    };
    Ok(PermissionRuleResource {
        id: row.id,
        action_key: row.action_key.clone(),
        subject_kind: subject_kind.into(),
        tool_name,
        qualifier,
        path_access_kind,
        workspace_root,
        target_path,
        network_target,
        network_host,
        network_port,
        mode,
        scope: row.scope.clone(),
        session_id: row.session_id,
        workspace_id: row.workspace_id,
        source: row.source.clone(),
        reason: row.reason.clone(),
        operator: row.operator.clone(),
        revoked_at: row.revoked_at_ms.map(timestamp_millis).transpose()?,
        revoked_reason: row.revoked_reason.clone(),
        revoked_by: row.revoked_by.clone(),
        created_at: timestamp_millis(row.created_at_ms)?,
        updated_at: timestamp_millis(row.updated_at_ms)?,
    })
}

fn provider_model_resource(mut model: agena_domain::Model) -> Result<Value, PluginError> {
    model
        .thinking_modes
        .sort_by(agena_domain::compare_thinking_mode_strength);
    let mut value = serde_json::to_value(model).map_err(plugin_error)?;
    // The public provider resource flattens domain model limits into metadata.
    if let Some(metadata) = value.get_mut("metadata").and_then(Value::as_object_mut)
        && let Some(Value::Object(limits)) = metadata.remove("limits")
    {
        metadata.extend(limits);
    }
    Ok(value)
}

fn provider_adapter_models_resource(
    listing: agena_provider::ProviderAdapterModelsListing,
) -> Result<Value, PluginError> {
    let adapters = listing.adapters.into_iter().map(|adapter| {
        let mut models = adapter.models;
        models.sort_by(|left, right| left.id.as_ref().cmp(right.id.as_ref()));
        Ok(json!({
            "adapter_id": adapter.adapter_id,
            "enabled": adapter.enabled,
            "resolved_base_url": adapter.resolved_base_url,
            "models": models.into_iter().map(provider_model_resource).collect::<Result<Vec<_>, _>>()?,
            "failure": adapter.failure.map(agena_failure::UserProblem::from),
        }))
    }).collect::<Result<Vec<_>, PluginError>>()?;
    Ok(json!({ "provider_id": listing.provider_id, "adapters": adapters }))
}
