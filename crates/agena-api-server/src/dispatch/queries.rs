use super::*;
use crate::session_support::{session_execution_resource, session_goal_resource_for_session};

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

pub async fn dispatch_query(state: &AppState, query: Query) -> Result<QueryResult, ServerError> {
    let manager = state.session_manager()?;
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
        Query::ListMessages(ListMessagesParams {
            session_id,
            cursor,
            limit,
            parts,
        }) => Ok(QueryResult::Messages(
            http_page_result(state.service().list_messages(
                manager.as_ref(),
                session_id,
                MessageListQuery {
                    pagination: cursor_pagination(cursor, limit),
                    parts: parts.into(),
                },
            ))
            .await?,
        )),
        Query::GetMessage(GetMessageParams { message_id, parts }) => Ok(QueryResult::Message(
            http_optional_result(
                state
                    .service()
                    .get_message(manager.as_ref(), message_id, parts.into()),
                || format!("message {message_id} not found"),
            )
            .await?,
        )),
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
            api_key_env,
            adapter_ids,
        }) => Ok(QueryResult::ProviderAdapterModels(
            crate::provider_queries::list_provider_adapter_models_response(
                state,
                ListProviderAdapterModelsParams {
                    provider_id,
                    base_url,
                    protocol_paths,
                    api_key,
                    api_key_env,
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
            let session = manager.get_session(session_id).await?;
            let state_resource =
                session_execution_resource(state, manager.as_ref(), &session).await?;
            Ok(QueryResult::SessionState(state_resource.into()))
        }
        Query::GetSessionGoal(GetSessionParams { session_id }) => {
            let session = manager.get_session(session_id).await?;
            let goal = match session.goal.as_ref() {
                Some(goal) => Some(
                    session_goal_resource_for_session(state, manager.as_ref(), &session, goal)
                        .await?
                        .into(),
                ),
                None => None,
            };
            Ok(QueryResult::SessionGoal(goal))
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
