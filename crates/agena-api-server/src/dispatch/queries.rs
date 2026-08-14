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
            exclude_subagents,
            search,
        }) => Ok(QueryResult::Sessions(
            http_page_result(state.service().list_sessions(SessionListQuery {
                pagination: search_pagination(cursor, limit, search),
                workspace_id,
                parent_id,
                roots,
                exclude_subagents,
            }))
            .await?,
        )),
        Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
            http_optional_result(state.service().get_session(session_id), || {
                format!("session {session_id} not found")
            })
            .await?,
        )),
        Query::Health => Ok(QueryResult::Health(agena_api::resource::HealthResponse {
            status: "ok".into(),
            generation: 0,
            loaded_at: chrono::Utc::now(),
            database_connected: true,
        })),
        Query::Runtime => Ok(QueryResult::Runtime(state.runtime_status_response().await)),
        Query::ListActivities(params) => {
            let service = state.runtime_activities()?;
            let filter = agena_application::service::activity_filter_from_params(&params);
            let activities = service.list_activities(&filter);
            Ok(QueryResult::Activities(
                activities
                    .iter()
                    .map(agena_api::resource::BackgroundActivityResource::from)
                    .collect(),
            ))
        }
        Query::GetActivity(GetActivityParams { activity_id }) => {
            let service = state.runtime_activities()?;
            let activity = service
                .get_activity(&activity_id)
                .map_err(activity_control_error)?;
            Ok(QueryResult::Activity(
                agena_api::resource::BackgroundActivityResource::from(&activity),
            ))
        }
        Query::ActivityLogs(ActivityLogsParams {
            activity_id,
            since_seq,
            limit,
            wait_ms,
        }) => {
            let service = state.runtime_activities()?;
            let read = service
                .activity_logs(&activity_id, since_seq, limit, wait_ms)
                .await
                .map_err(activity_control_error)?;
            Ok(QueryResult::ActivityLogs(read.into()))
        }
        Query::ListProviders => Ok(QueryResult::Providers(
            agena_application::provider_queries::list_providers_response(state),
        )),
        Query::ListProviderModels(ListProviderModelsParams { provider_id }) => {
            Ok(QueryResult::ProviderModels(
                agena_application::provider_queries::list_provider_models_response(
                    state,
                    provider_id,
                )
                .await?,
            ))
        }
        Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
            provider_id,
            base_url,
            protocol_paths,
            api_key,
            adapter_ids,
        }) => Ok(QueryResult::ProviderAdapterModels(
            agena_application::provider_queries::list_provider_adapter_models_response(
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
            agena_application::provider_queries::list_saved_provider_adapter_models_response(
                state,
                ListSavedProviderAdapterModelsParams {
                    provider_id,
                    adapter_ids,
                },
            )
            .await?,
        )),
        Query::GetSessionState(GetSessionParams { session_id }) => Ok(QueryResult::SessionState(
            state.session_execution_resource(session_id).await?,
        )),
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
    ActivityLogsParams, Application, ApplicationError, CursorPaginationQuery, GetActivityParams,
    GetOperationDetailParams, GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams,
    ListPermissionRulesParams, ListProviderAdapterModelsParams, ListProviderModelsParams,
    ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams,
    OperationDetailResource, Query, QueryResult, SearchPaginationQuery, SessionListQuery,
    WorkspaceListQuery, http_optional_result, http_page_result,
};

fn activity_control_error(error: agena_runtime::ActivityControlError) -> ApplicationError {
    ApplicationError::bad_request_with_diagnostic(
        "The background activity operation failed.",
        error.to_string(),
    )
}
