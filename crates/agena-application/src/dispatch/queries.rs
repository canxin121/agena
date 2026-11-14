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
    state: &super::Application,
    query: Query,
) -> Result<QueryResult, ApplicationError> {
    match query {
        Query::Server(query) => Ok(QueryResult::Data(state.dispatch_server_query(query).await?)),
        Query::Business(query) => Ok(QueryResult::Data(
            state.dispatch_business_query(query).await?,
        )),
        Query::Management(query) => Ok(QueryResult::Data(
            state.dispatch_management_query(query).await?,
        )),
        Query::ExtendedSession(query) => Ok(QueryResult::Data(
            state.dispatch_extended_session_query(query).await?,
        )),
        Query::ReadContentText(params) => Ok(QueryResult::ContentText(
            state.read_content_text(params).await?,
        )),
        Query::ReadParts(params) => Ok(QueryResult::Parts(state.read_parts(params).await?)),
        Query::ReadRuns(params) => Ok(QueryResult::Parts(state.read_runs(params).await?)),
        Query::ReadContent(params) => Ok(QueryResult::Content(
            state
                .read_content(
                    params.session_id,
                    params.resource_id,
                    params.after,
                    params.max_bytes,
                )
                .await?,
        )),
        Query::ListWorkspaces(ListWorkspacesParams {
            cursor,
            limit,
            search,
            include_session_count,
            offset,
        }) => Ok(QueryResult::Workspaces(
            page_result(state.service().list_workspaces(WorkspaceListQuery {
                pagination: search_pagination(cursor, limit, search),
                include_session_count,
                offset,
            }))
            .await?,
        )),
        Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => Ok(QueryResult::Workspace(
            optional_result(state.service().get_workspace(workspace_id), || {
                format!("workspace {workspace_id} not found")
            })
            .await?,
        )),
        Query::ListSessions(ListSessionsParams {
            cursor,
            limit,
            workspace_id,
            owner_bot_id,
            owner_surface,
            parent_id,
            roots,
            exclude_subagents,
            offset,
            bucket,
            include_total,
            count_only,
            search,
        }) => Ok(QueryResult::Sessions(
            page_result(state.service().list_sessions(SessionListQuery {
                pagination: search_pagination(cursor, limit, search),
                workspace_id,
                owner_bot_id,
                owner_surface,
                parent_id,
                roots,
                exclude_subagents,
                offset,
                bucket,
                include_total,
                count_only,
            }))
            .await?,
        )),
        Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
            optional_result(state.service().get_session(session_id), || {
                format!("session {session_id} not found")
            })
            .await?,
        )),
        Query::Health => {
            let snapshot = state.runtime_snapshot_summary().await;
            Ok(QueryResult::Health(agena_api::resource::HealthResponse {
                status: "ok".into(),
                generation: snapshot.generation,
                loaded_at: snapshot.loaded_at,
                database_connected: true,
                server: state.server_identity().clone(),
            }))
        }
        Query::Runtime => Ok(QueryResult::Runtime(state.runtime_status_response().await?)),
        Query::ListActivities(params) => {
            let service = state.runtime_activities()?;
            let filter = crate::service::activity_filter_from_params(&params);
            let activities = service
                .list_activities(&filter)
                .await
                .map_err(activity_control_error)?;
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
                .await
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
        Query::GetSessionState(GetSessionParams { session_id }) => Ok(QueryResult::SessionState(
            state.session_execution_resource(session_id).await?,
        )),
        Query::GetSessionCost(GetSessionParams { session_id }) => Ok(QueryResult::SessionCost(
            state
                .session_query_service()?
                .session_cost_summary(session_id)
                .await
                .map_err(|error| ApplicationError::internal_error(&error))?,
        )),
        Query::GetSessionTree(GetSessionParams { session_id }) => {
            let summaries = state
                .session_execution_services()?
                .queries
                .list_session_tree(session_id)
                .await
                .map_err(|error| ApplicationError::from_failure(*error.failure))?;
            Ok(QueryResult::SessionTree(
                state.session_resources_from_summaries(summaries).await?,
            ))
        }
        Query::GetSessionPlatforms(GetSessionParams { session_id }) => {
            Ok(QueryResult::SessionPlatforms(
                state
                    .assistant_service()?
                    .platform_inventory(session_id)
                    .await
                    .map_err(|error| ApplicationError::internal_error(&error))?,
            ))
        }
        Query::ListPermissionRules(ListPermissionRulesParams {
            cursor,
            limit,
            search,
        }) => Ok(QueryResult::PermissionRules(
            page_result(
                state
                    .service()
                    .list_permission_rules(search_pagination(cursor, limit, search)),
            )
            .await?,
        )),
        Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
            Ok(QueryResult::PermissionRule(
                optional_result(state.service().get_permission_rule(rule_id), || {
                    format!("permission rule {rule_id} not found")
                })
                .await?,
            ))
        }
    }
}
use super::{
    ActivityLogsParams, ApplicationError, CursorPaginationQuery, GetActivityParams,
    GetPermissionRuleParams, GetSessionParams, GetWorkspaceParams, ListPermissionRulesParams,
    ListProviderAdapterModelsParams, ListProviderModelsParams,
    ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
    QueryResult, SearchPaginationQuery, SessionListQuery, WorkspaceListQuery, optional_result,
    page_result,
};

fn activity_control_error(error: agena_runtime::ActivityControlError) -> ApplicationError {
    ApplicationError::bad_request_error(&error)
}
