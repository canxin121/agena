//! REST handlers for the Studio/Web surface. These endpoints intentionally
//! return the plain JSON resources the current web client already consumes,
//! while WS/SSE protocol traffic continues to route through `dispatch`.

use std::{collections::{BTreeSet, HashMap}, convert::Infallible, sync::Arc};

use agena::event::{EventStore, StoreRange};
use agena_api::queries::{ListEventsParams, Query, QueryResult};
use agena_http_api::{
    AuthApiKeyWriteRequest, AuthCredentialType, AuthProviderResource, HealthResponse,
    MessageListQuery, PermissionRuleListQuery,
    PermissionRuleWriteRequest, PluginInspectResponse, PluginLogListQuery,
    PluginLogListResponse, PluginStatusListResponse, RuntimeReloadResponse,
    SessionContinueRequestBody, SessionCreateRequest, SessionEventStreamQuery,
    SessionListQuery, SessionPermissionReplyRequestBody, SessionReplaceRequest,
    SessionRewindRequestBody, SessionRunOptionsRequest, SessionTurnRequest,
    SessionUserInputReplyRequestBody, WorkspaceFileTreeQuery, WorkspaceListQuery,
    WorkspaceResolveRequest, WorkspaceWriteRequest,
};
use async_stream::stream;
use axum::{
    Json,
    extract::{Path, Query as AxumQuery, State},
    http::{HeaderMap, header::IF_MATCH},
    response::{IntoResponse, sse::{Event, Sse}},
};
use serde::Deserialize;

use crate::{dispatch, error::ServerError, state::AppState};

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionEventListCompatQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub after_seq: Option<i64>,
}

pub async fn health(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    Ok(Json(HealthResponse {
        status: "ok",
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at(),
        database_connected: true,
    }))
}

pub async fn get_runtime_status(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(&state, Query::Runtime).await? {
        QueryResult::Runtime(runtime) => Ok(Json(runtime)),
        _ => unreachable!("runtime query returned unexpected result"),
    }
}

pub async fn reload_runtime(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let report = state.runtime().reload().await.map_err(ServerError::Core)?;
    Ok(Json(RuntimeReloadResponse {
        cause: "manual",
        previous_generation: report.previous_generation,
        generation: report.generation,
        loaded_at: report.loaded_at,
    }))
}

pub async fn list_plugins(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(PluginStatusListResponse {
        entries: state.runtime().current_snapshot().plugin_manager().plugin_statuses(),
    }))
}

pub async fn get_plugin(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin = state
        .runtime()
        .current_snapshot()
        .plugin_manager()
        .plugin_inspect(plugin_id.as_str())
        .ok_or_else(|| ServerError::NotFound(format!("plugin not found: {plugin_id}")))?;
    Ok(Json(PluginInspectResponse { plugin }))
}

pub async fn list_plugin_logs(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    AxumQuery(query): AxumQuery<PluginLogListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let plugin_manager = state.runtime().current_snapshot().plugin_manager();
    if plugin_manager.plugin_status(plugin_id.as_str()).is_none() {
        return Err(ServerError::NotFound(format!("plugin not found: {plugin_id}")));
    }
    Ok(Json(PluginLogListResponse {
        plugin_id: plugin_id.clone(),
        entries: plugin_manager.plugin_logs(
            plugin_id.as_str(),
            query.after_seq,
            query.limit.unwrap_or(50),
        ),
    }))
}

pub async fn list_auth_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let configured_ids = configured_provider_ids(&state);
    let auth_map = state.runtime().auth_manager().all().map_err(ServerError::Core)?;
    let provider_ids = public_auth_provider_ids(&configured_ids, &auth_map);
    let items = provider_ids
        .into_iter()
        .map(|provider_id| {
            let auth = auth_map.get(provider_id.as_str());
            auth_provider_resource(configured_ids.contains(provider_id.as_str()), provider_id, auth)
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(items))
}

pub async fn get_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

pub async fn set_auth_provider_api_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<AuthApiKeyWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .set_api_key(provider_id.as_str(), request.api_key)
        .map_err(ServerError::Core)?;
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

pub async fn delete_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .remove(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if provider_id == "gitlab" {
        state
            .runtime()
            .auth_manager()
            .remove("gitlab-instance")
            .map_err(ServerError::Core)?;
    }
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        None,
    )?))
}

pub async fn refresh_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state
        .runtime()
        .auth_manager()
        .get(provider_id.as_str())
        .map_err(ServerError::Core)?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    let auth = match provider_id.as_str() {
        "openai" => state
            .runtime()
            .auth_manager()
            .refresh_openai_login()
            .await
            .map_err(ServerError::Core)?,
        "gitlab" => state
            .runtime()
            .auth_manager()
            .refresh_gitlab_login()
            .await
            .map_err(ServerError::Core)?,
        _ => {
            return Err(ServerError::BadRequest(format!(
                "credential refresh is not supported for provider '{provider_id}'"
            )));
        }
    };

    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        Some(&auth),
    )?))
}

pub async fn list_providers(State(state): State<AppState>) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(&state, Query::ListProviders).await? {
        QueryResult::Providers(providers) => Ok(Json(providers)),
        _ => unreachable!("providers query returned unexpected result"),
    }
}

pub async fn list_provider_models(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_query(
        &state,
        Query::ListProviderModels(agena_api::queries::ListProviderModelsParams { provider_id }),
    )
    .await?
    {
        QueryResult::ProviderModels(models) => Ok(Json(models)),
        _ => unreachable!("provider models query returned unexpected result"),
    }
}

pub async fn list_workspaces(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<WorkspaceListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspaces(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let workspace = state
        .service()
        .get_workspace(workspace_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("workspace not found: {workspace_id}")))?;
    Ok(Json(workspace))
}

pub async fn create_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn resolve_workspace(
    State(state): State<AppState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .resolve_workspace(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_workspace(workspace_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_workspace(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_workspace(workspace_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_workspace_files(
    State(state): State<AppState>,
    Path(workspace_id): Path<i64>,
    AxumQuery(query): AxumQuery<WorkspaceFileTreeQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_workspace_files(workspace_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<SessionListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_sessions(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let session = state
        .service()
        .get_session(session_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("session not found: {session_id}")))?;
    Ok(Json(session))
}

pub async fn get_session_state(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    let session = manager.get_session(session_id).await.map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn create_session(
    State(state): State<AppState>,
    Json(request): Json<SessionCreateRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_session(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplaceRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .replace_session(session_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }
    Ok(Json(
        state
            .service()
            .delete_session(session_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventListCompatQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    if let Some(after_seq) = query.after_seq {
        let limit = query.limit.unwrap_or(100).clamp(1, 1000);
        let items = state
            .service()
            .list_session_events_after(manager.as_ref(), session_id, after_seq, Some(limit))
            .await
            .map_err(server_error_from_http)?;
        let returned = items.len() as u64;
        let next_cursor = items.last().map(|event| event.meta.seq_global.to_string());
        return Ok(Json(serde_json::json!({
            "items": items,
            "page": {
                "limit": limit,
                "returned": returned,
                "has_more": returned >= limit,
                "next_cursor": next_cursor,
                "order": "asc"
            }
        })));
    }

    let page = state
        .service()
        .list_session_events(
            manager.as_ref(),
            session_id,
            agena_http_api::SessionEventListQuery {
                cursor: query.cursor,
                limit: query.limit,
            },
        )
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(serde_json::to_value(page).map_err(|error| ServerError::Internal(error.to_string()))?))
}

pub async fn stream_session_events(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<SessionEventStreamQuery>,
) -> Result<impl IntoResponse, ServerError> {
    use agena::event::{EventFilter, Scope, bus::SubscriptionItem};

    let manager = state.session_manager()?;
    let service = state.service().clone();
    let backfill_after = query.after_seq.unwrap_or(0);
    let backfill_limit = query.limit.unwrap_or(100).clamp(1, 1000);
    let initial = service
        .list_session_events_after(
            manager.as_ref(),
            session_id,
            backfill_after,
            Some(backfill_limit),
        )
        .await
        .map_err(server_error_from_http)?;

    let bus = manager.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));

    let stream = stream! {
        for event in &initial {
            match Event::default()
                .event("session_event")
                .id(event.meta.seq_global.to_string())
                .json_data(event)
            {
                Ok(ev) => yield Ok::<Event, Infallible>(ev),
                Err(error) => {
                    yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                    return;
                }
            }
        }
        let mut last_seen = initial
            .last()
            .map(|event| event.meta.seq_global)
            .unwrap_or(backfill_after);

        loop {
            match subscription.recv().await {
                Some(SubscriptionItem::Event(arc_event)) => {
                    if arc_event.meta.seq_global <= last_seen {
                        continue;
                    }
                    last_seen = arc_event.meta.seq_global;
                    match Event::default()
                        .event("session_event")
                        .id(arc_event.meta.seq_global.to_string())
                        .json_data(arc_event.as_ref())
                    {
                        Ok(ev) => yield Ok::<Event, Infallible>(ev),
                        Err(error) => {
                            yield Ok::<Event, Infallible>(sse_error_event(format!("failed to encode session event: {error}")));
                            return;
                        }
                    }
                }
                Some(SubscriptionItem::Lagged(skipped)) => {
                    yield Ok::<Event, Infallible>(Event::default().event("lagged").data(skipped.to_string()));
                }
                None => return,
            }
        }
    };

    Ok(Sse::new(stream))
}

pub async fn submit_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionTurnRequest>,
) -> Result<impl IntoResponse, ServerError> {
    if request.parts.is_empty() {
        return Err(ServerError::BadRequest(
            "session turn requires at least one part".into(),
        ));
    }
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .submit_user_turn(agena::session::SessionUserTurnRequest {
            session_id,
            options,
            parts: request.parts,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn continue_run(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionContinueRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .continue_session(agena::session::SessionContinueRequest { session_id, options })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn cancel_turn(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    match dispatch::dispatch_command(
        &state,
        agena_api::commands::Command::CancelTurn(agena_api::commands::CancelTurnParams { session_id }),
    )
    .await?
    {
        agena_api::commands::CommandResult::Ack => Ok(Json(serde_json::json!({ "ok": true }))),
        _ => unreachable!("cancel turn returned unexpected result"),
    }
}

pub async fn reply_permission(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionPermissionReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_permission(agena::session::SessionPermissionReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn reply_user_input(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionUserInputReplyRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let manager = state.session_manager()?;
    let session = manager
        .reply_user_input(agena::session::SessionUserInputReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn rewind_session(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionRewindRequestBody>,
) -> Result<impl IntoResponse, ServerError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await
            .map_err(server_error_from_http)?;
    }

    let manager = state.session_manager()?;
    let session = manager
        .rewind_session(agena::session::SessionRewindRequest {
            session_id,
            message_id: request.message_id,
        })
        .await
        .map_err(ServerError::Core)?;
    let resource = state
        .service()
        .session_execution_resource(manager.as_ref(), &session)
        .await
        .map_err(server_error_from_http)?;
    Ok(Json(resource))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(session_id): Path<i64>,
    AxumQuery(query): AxumQuery<MessageListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_messages(manager.as_ref(), session_id, query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_permission_rules(
    State(state): State<AppState>,
    AxumQuery(query): AxumQuery<PermissionRuleListQuery>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .list_permission_rules(query)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn get_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    let rule = state
        .service()
        .get_permission_rule(rule_id)
        .await
        .map_err(server_error_from_http)?
        .ok_or_else(|| ServerError::NotFound(format!("permission rule not found: {rule_id}")))?;
    Ok(Json(rule))
}

pub async fn create_permission_rule(
    State(state): State<AppState>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .create_permission_rule(request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn replace_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .replace_permission_rule(rule_id, request)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn delete_permission_rule(
    State(state): State<AppState>,
    Path(rule_id): Path<i64>,
) -> Result<impl IntoResponse, ServerError> {
    Ok(Json(
        state
            .service()
            .delete_permission_rule(rule_id)
            .await
            .map_err(server_error_from_http)?,
    ))
}

pub async fn list_events(
    State(state): State<AppState>,
    AxumQuery(params): AxumQuery<ListEventsParams>,
) -> Result<impl IntoResponse, ServerError> {
    let publisher = state.event_publisher()?;
    let store: &Arc<dyn EventStore<agena::event::EventKind>> = publisher.store();
    let filter = agena::event::EventFilter {
        scope: params.scope,
        kinds: params.kinds,
        since_seq_global: params.since_seq_global,
    };
    let limit = params.limit.unwrap_or(100).clamp(1, 1000) as usize;
    let range = StoreRange {
        after_seq_global: params.since_seq_global.unwrap_or(0),
        limit,
    };
    let events = store
        .range(&filter, range)
        .await
        .map_err(|error| ServerError::Internal(error.to_string()))?;
    let returned = events.len() as u64;
    let next_cursor = events.last().map(|event| event.meta.seq_global.to_string());
    Ok(Json(serde_json::json!({
        "items": events,
        "page": {
            "next_cursor": next_cursor,
            "has_more": (returned as usize) >= limit,
            "returned": returned,
        }
    })))
}

pub async fn plugin_rpc(
    State(state): State<AppState>,
    Path(plugin_id): Path<String>,
    headers: HeaderMap,
    Json(req): Json<agena::plugin::sdk::rpc::Request>,
) -> Result<impl IntoResponse, ServerError> {
    let host = state.runtime().current_snapshot().plugin_manager();
    let response = plugin_rpc_response(host, plugin_id.as_str(), bearer_token(&headers), req).await?;
    Ok(Json(response))
}

fn server_error_from_http(error: agena_http_api::ApiError) -> ServerError {
    let status = error.into_response().status();
    match status {
        axum::http::StatusCode::BAD_REQUEST => ServerError::BadRequest("legacy API bad request".into()),
        axum::http::StatusCode::NOT_FOUND => ServerError::NotFound("legacy API resource not found".into()),
        axum::http::StatusCode::CONFLICT => ServerError::Conflict("legacy API conflict".into()),
        axum::http::StatusCode::SERVICE_UNAVAILABLE => {
            ServerError::ServiceUnavailable("legacy API service unavailable".into())
        }
        _ => ServerError::Internal("legacy API internal error".into()),
    }
}

async fn resolve_run_options(
    state: &AppState,
    session_id: i64,
    request: SessionRunOptionsRequest,
) -> Result<agena::session::SessionRunOptions, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let manager = state.session_manager()?;
    state
        .service()
        .resolve_run_options(
            snapshot.provider_registry().as_ref(),
            manager.as_ref(),
            session_id,
            request,
        )
        .await
        .map_err(server_error_from_http)
}

fn if_match_version(headers: &HeaderMap) -> Result<Option<i64>, ServerError> {
    let Some(value) = headers.get(IF_MATCH) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|error| ServerError::BadRequest(format!("invalid If-Match header: {error}")))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ServerError::BadRequest("If-Match header cannot be empty".into()));
    }
    if trimmed == "*" {
        return Err(ServerError::BadRequest(
            "If-Match '*' is not supported for session version checks".into(),
        ));
    }
    if trimmed.contains(',') {
        return Err(ServerError::BadRequest(
            "If-Match must contain exactly one session version".into(),
        ));
    }

    let version_text = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let version = version_text.parse::<i64>().map_err(|error| {
        ServerError::BadRequest(format!("If-Match must be a numeric session version: {error}"))
    })?;

    Ok(Some(version))
}

fn sse_error_event(message: impl Into<String>) -> Event {
    Event::default().event("error").data(message.into())
}

fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(axum::http::header::AUTHORIZATION)?.to_str().ok()?.trim();
    let mut parts = raw.split_whitespace();
    let scheme = parts.next()?;
    let token = parts.next()?;
    if !scheme.eq_ignore_ascii_case("bearer") || parts.next().is_some() {
        return None;
    }
    Some(token.to_string())
}

fn callback_context_present(params: &serde_json::Value) -> bool {
    params
        .as_object()
        .and_then(|object| object.get("context"))
        .and_then(|value| {
            serde_json::from_value::<agena::plugin::sdk::host_api::HostCallbackContext>(
                value.clone(),
            )
            .ok()
        })
        .is_some()
}

async fn plugin_rpc_response(
    host: Arc<agena::plugin::PluginHost>,
    plugin_id: &str,
    callback_token: Option<String>,
    req: agena::plugin::sdk::rpc::Request,
) -> Result<agena::plugin::sdk::rpc::Response, ServerError> {
    use agena::plugin::sdk::rpc::{ErrorObject, JsonRpcVersion, Response, ResponsePayload, codes};

    if !host
        .host_handle()
        .validate_callback_token(plugin_id, callback_token.as_deref())
        .await
    {
        return Err(ServerError::BadRequest(
            "invalid or missing plugin callback bearer token".into(),
        ));
    }

    let id = req.id.clone();
    if host.plugins().iter().all(|plugin| plugin.id != plugin_id) {
        return Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::HOST_UNAVAILABLE,
                    message: format!("unknown plugin id: {plugin_id}"),
                    data: None,
                },
            },
        });
    }

    let params = req.params.unwrap_or(serde_json::Value::Null);
    if !callback_context_present(&params) {
        return Err(ServerError::BadRequest(
            "plugin callback request is missing callback context".into(),
        ));
    }

    let handle = host.host_handle();
    match handle
        .ingest_stream_event_for_plugin(plugin_id, &req.method, params.clone())
        .await
    {
        Ok(true) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Ok {
                    result: serde_json::Value::Object(Default::default()),
                },
            });
        }
        Ok(false) => {}
        Err(err) => {
            return Ok(Response {
                jsonrpc: JsonRpcVersion,
                id,
                payload: ResponsePayload::Err {
                    error: ErrorObject {
                        code: codes::PLUGIN_GENERIC,
                        message: err.message.clone(),
                        data: serde_json::to_value(&err).ok(),
                    },
                },
            });
        }
    }

    match handle.handle_call_for_plugin(plugin_id, &req.method, params).await {
        Ok(result) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result },
        }),
        Err(err) => Ok(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Err {
                error: ErrorObject {
                    code: codes::PLUGIN_GENERIC,
                    message: err.message.clone(),
                    data: serde_json::to_value(&err).ok(),
                },
            },
        }),
    }
}

fn configured_provider_ids(state: &AppState) -> BTreeSet<String> {
    state
        .runtime()
        .current_snapshot()
        .provider_registry()
        .provider_ids()
        .into_iter()
        .collect()
}

fn public_auth_provider_ids(
    configured_ids: &BTreeSet<String>,
    auth_map: &HashMap<String, agena::provider::auth::AuthData>,
) -> BTreeSet<String> {
    configured_ids
        .iter()
        .cloned()
        .chain(
            auth_map
                .keys()
                .filter(|provider_id| !is_internal_auth_provider_id(provider_id.as_str()))
                .cloned(),
        )
        .collect()
}

fn ensure_public_auth_provider_id(provider_id: &str) -> Result<(), ServerError> {
    if is_internal_auth_provider_id(provider_id) {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(())
}

fn is_internal_auth_provider_id(provider_id: &str) -> bool {
    provider_id == "gitlab-instance"
}

fn auth_provider_resource(
    configured: bool,
    provider_id: String,
    auth: Option<&agena::provider::auth::AuthData>,
) -> Result<AuthProviderResource, ServerError> {
    let mut resource = AuthProviderResource {
        provider_id,
        configured,
        credential_present: auth.is_some(),
        credential_type: None,
        key_preview: None,
        expires_at: None,
        expired: None,
        account_id: None,
        enterprise_url: None,
    };

    match auth {
        Some(agena::provider::auth::AuthData::Api { key }) => {
            resource.credential_type = Some(AuthCredentialType::Api);
            resource.key_preview = secret_preview(key);
        }
        Some(agena::provider::auth::AuthData::OAuth {
            expires_at_ms,
            account_id,
            enterprise_url,
            ..
        }) => {
            resource.credential_type = Some(AuthCredentialType::Oauth);
            resource.expires_at = if *expires_at_ms > 0 {
                Some(
                    chrono::DateTime::from_timestamp_millis(*expires_at_ms).ok_or_else(|| {
                        ServerError::Internal(format!(
                            "invalid oauth expiry millis: {expires_at_ms}"
                        ))
                    })?,
                )
            } else {
                None
            };
            resource.expired = resource
                .expires_at
                .map(|expires_at| expires_at <= chrono::Utc::now());
            resource.account_id = account_id.clone();
            resource.enterprise_url = enterprise_url.clone();
        }
        Some(agena::provider::auth::AuthData::WellKnown { key, .. }) => {
            resource.credential_type = Some(AuthCredentialType::WellKnown);
            resource.key_preview = Some(key.clone());
        }
        None => {}
    }

    Ok(resource)
}

fn secret_preview(secret: &str) -> Option<String> {
    let trimmed = secret.trim();
    if trimmed.is_empty() {
        return None;
    }
    if trimmed.len() <= 8 {
        return Some("*".repeat(trimmed.len()));
    }
    Some(format!(
        "{}...{}",
        &trimmed[..4],
        &trimmed[trimmed.len() - 4..]
    ))
}
