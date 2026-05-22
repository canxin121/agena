use super::*;

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
                .server()?;
            Ok(QueryResult::Workspaces(page_from_http(page)))
        }
        Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => {
            let workspace = state
                .service()
                .get_workspace(workspace_id)
                .await
                .server()?
                .ok_or_else(|| {
                    ServerError::NotFound(format!("workspace {workspace_id} not found"))
                })?;
            Ok(QueryResult::Workspace(workspace.into()))
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
                .server()?;
            Ok(QueryResult::Sessions(page_from_http(page)))
        }
        Query::GetSession(GetSessionParams { session_id }) => {
            let session = state
                .service()
                .get_session(session_id)
                .await
                .server()?
                .ok_or_else(|| ServerError::NotFound(format!("session {session_id} not found")))?;
            Ok(QueryResult::Session(session.into()))
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
                        parts: parts.into(),
                    },
                )
                .await
                .server()?;
            Ok(QueryResult::Messages(page_from_http(page)))
        }
        Query::GetMessage(GetMessageParams { message_id, parts }) => {
            let message = state
                .service()
                .get_message(manager.as_ref(), message_id, parts.into())
                .await
                .server()?
                .map(Into::into)
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
            let state_resource = state
                .service()
                .session_execution_resource(manager.as_ref(), &session)
                .await
                .server()?;
            Ok(QueryResult::SessionState(state_resource.into()))
        }
        Query::GetSessionGoal(GetSessionParams { session_id }) => {
            let session = manager.get_session(session_id).await?;
            let goal = match session.goal.as_ref() {
                Some(goal) => Some(
                    state
                        .service()
                        .session_goal_resource(manager.as_ref(), &session, goal)
                        .await
                        .server()?
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
        }) => {
            let page = state
                .service()
                .list_permission_rules(PermissionRuleListQuery {
                    cursor,
                    limit,
                    search,
                })
                .await
                .server()?;
            Ok(QueryResult::PermissionRules(page_from_http(page)))
        }
        Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
            let rule = state
                .service()
                .get_permission_rule(rule_id)
                .await
                .server()?
                .ok_or_else(|| {
                    ServerError::NotFound(format!("permission rule {rule_id} not found"))
                })?;
            Ok(QueryResult::PermissionRule(rule.into()))
        }
    }
}
