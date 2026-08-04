use crate::session::session_execution_resource;

// ─── Query dispatch ─────────────────────────────────────────────────────

fn cursor_pagination(cursor: Option<String>, limit: Option<u64>) -> CursorPaginationQuery {
    CursorPaginationQuery { cursor, limit }
}

fn search_pagination(
    cursor: Option<String>,
    limit: Option<u64>,
    search: Option<String>,
) -> SearchPaginationQuery {
    SearchPaginationQuery {
        pagination: cursor_pagination(cursor, limit),
        search,
    }
}

pub async fn dispatch_query(
    state: &Application,
    query: Query,
) -> Result<QueryResult, ApplicationError> {
    match query {
        Query::ListWorkspaces(ListWorkspacesParams {
            cursor,
            limit,
            search,
            include_session_count,
        }) => Ok(QueryResult::Workspaces(
            http_page_result(state.service().list_workspaces(WorkspaceListQuery {
                pagination: search_pagination(cursor, limit, search),
                include_session_count,
            }))
            .await?,
        )),
        Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => Ok(QueryResult::Workspace(
            http_optional_result(state.service().get_workspace(workspace_id), || {
                format!("workspace {workspace_id} not found")
            })
            .await?,
        )),
        Query::ListSessions(ListSessionsParams {
            cursor,
            limit,
            workspace_id,
            parent_id,
            roots,
            search,
        }) => Ok(QueryResult::Sessions(
            http_page_result(state.service().list_sessions(SessionListQuery {
                pagination: search_pagination(cursor, limit, search),
                workspace_id,
                parent_id,
                roots,
            }))
            .await?,
        )),
        Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
            http_optional_result(state.service().get_session(session_id), || {
                format!("session {session_id} not found")
            })
            .await?,
        )),
        Query::ListEvents(ListEventsParams {
            scope,
            kinds,
            since_seq_global,
            limit,
        }) => {
            let events = state.event_query_service()?;
            let filter = agena_domain::EventFilter {
                scope: match scope {
                    agena_api::Scope::Global => agena_domain::EventScope::Global,
                    agena_api::Scope::Workspace { workspace_id } => {
                        agena_domain::EventScope::Workspace { workspace_id }
                    }
                    agena_api::Scope::Session { session_id } => {
                        agena_domain::EventScope::Session { session_id }
                    }
                },
                kinds,
                since_seq_global,
            };
            let limit = normalize_limit(limit) as usize;
            let range = agena_runtime::RuntimeEventRange {
                after_seq_global: since_seq_global.unwrap_or(0),
                limit,
            };
            let events = events
                .list_events(&filter, range)
                .await
                .map_err(|e| ApplicationError::internal(e.to_string()))?;
            let returned = events.len() as u64;
            let next_cursor = events.last().map(|e| e.meta.seq_global.to_string());
            let items = events
                .iter()
                .map(crate::event_projection::event_resource_from_runtime)
                .collect::<Vec<_>>();
            Ok(QueryResult::Events(PaginatedEvents {
                items,
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
        Query::ListProviders => Ok(QueryResult::Providers(
            crate::provider_queries::list_providers_response(state),
        )),
        Query::ListProviderModels(ListProviderModelsParams { provider_id }) => {
            Ok(QueryResult::ProviderModels(
                crate::provider_queries::list_provider_models_response(state, provider_id).await?,
            ))
        }
        Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
            provider_id,
            base_url,
            protocol_paths,
            api_key,
            adapter_ids,
        }) => Ok(QueryResult::ProviderAdapterModels(
            crate::provider_queries::list_provider_adapter_models_response(
                state,
                ListProviderAdapterModelsParams {
                    provider_id,
                    base_url,
                    protocol_paths,
                    api_key,
                    adapter_ids,
                },
            )
            .await?,
        )),
        Query::ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams {
            provider_id,
            adapter_ids,
        }) => Ok(QueryResult::ProviderAdapterModels(
            crate::provider_queries::list_saved_provider_adapter_models_response(
                state,
                ListSavedProviderAdapterModelsParams {
                    provider_id,
                    adapter_ids,
                },
            )
            .await?,
        )),
        Query::GetSessionState(GetSessionParams { session_id }) => {
            let session_services = state.session_execution_services()?;
            let state_resource = session_execution_resource(
                state,
                session_services.execution_control.as_ref(),
                session_services.queries.as_ref(),
                session_id,
            )
            .await?;
            Ok(QueryResult::SessionState(state_resource))
        }
        Query::GetOperationDetail(GetOperationDetailParams {
            session_id,
            activity_id,
        }) => {
            let session_services = state.session_execution_services()?;
            let detail = session_services
                .queries
                .as_ref()
                .operation_detail(session_id, activity_id)
                .await
                .map_err(|error| {
                    ApplicationError::internal(format!("operation detail query failed: {error}"))
                })?;
            let resource = detail
                .map(|detail| OperationDetailResource {
                    activity_id: detail.activity_id,
                    markdown: detail.markdown,
                    streaming: detail.streaming,
                })
                .unwrap_or(OperationDetailResource {
                    activity_id,
                    markdown: String::new(),
                    streaming: false,
                });
            Ok(QueryResult::OperationDetail(resource))
        }
        Query::ListPermissionRules(ListPermissionRulesParams {
            cursor,
            limit,
            search,
        }) => Ok(QueryResult::PermissionRules(
            http_page_result(
                state
                    .service()
                    .list_permission_rules(search_pagination(cursor, limit, search)),
            )
            .await?,
        )),
        Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
            Ok(QueryResult::PermissionRule(
                http_optional_result(state.service().get_permission_rule(rule_id), || {
                    format!("permission rule {rule_id} not found")
                })
                .await?,
            ))
        }
    }
}
use super::{
    Application, ApplicationError, CursorPaginationQuery, GetOperationDetailParams,
    GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams, ListEventsParams,
    ListPermissionRulesParams, ListProviderAdapterModelsParams, ListProviderModelsParams,
    ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams,
    OperationDetailResource, PageInfo, PaginatedEvents, Query, QueryResult, SearchPaginationQuery,
    SessionListQuery, WorkspaceListQuery, http_optional_result, http_page_result, normalize_limit,
    runtime_status_response,
};
