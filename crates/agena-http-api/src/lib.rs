mod dto;
mod error;
mod pagination;
mod service;

use std::{
    collections::{BTreeSet, HashMap},
    convert::Infallible,
    future::Future,
    sync::Arc,
    time::Duration,
};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, header::IF_MATCH},
    response::{
        IntoResponse,
        sse::{Event, Sse},
    },
    routing::{get, post},
};
use chrono::DateTime;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::time::{Instant, sleep};

use agena::{AppError, runtime::AgenaRuntime, session::SessionManager};

pub use dto::{
    AuthApiKeyWriteRequest, AuthBrowserStartRequest, AuthBrowserStartResource,
    AuthCopilotDevicePollRequest, AuthCopilotDeviceStartRequest, AuthCredentialType,
    AuthDeviceStartResource, AuthGitLabBrowserFinishRequest, AuthGitLabBrowserStartRequest,
    AuthLoginResultResource, AuthOpenAiBrowserFinishRequest, AuthOpenAiDevicePollRequest,
    AuthProviderResource, HealthResponse, MessageListQuery, MessageResource, PartLoadMode,
    PermissionRuleListQuery, PermissionRuleResource, PermissionRuleWriteRequest,
    ProviderModelsResponse, ProviderSummaryResource, RuntimeReloadResponse, RuntimeStatusResponse,
    RuntimeTaskResource, SessionContinueRequestBody, SessionCreateRequest, SessionEventListQuery,
    SessionEventStreamQuery, SessionExecutionResource, SessionPermissionReplyRequestBody,
    SessionReplaceRequest, SessionResource, SessionTurnRequest, SessionUserInputReplyRequestBody,
    WorkspaceListQuery, WorkspaceResolveRequest, WorkspaceResource, WorkspaceWriteRequest,
};
pub use error::ApiError;
pub use pagination::{PageInfo, PaginatedResponse};
use service::ApiService;

#[derive(Clone)]
pub struct ApiState {
    runtime: AgenaRuntime,
    service: ApiService,
    database_connected: bool,
    session_manager_override: Option<Arc<SessionManager>>,
}

impl ApiState {
    pub fn new(runtime: AgenaRuntime, db: Arc<DatabaseConnection>) -> Self {
        Self {
            runtime,
            service: ApiService::new(db),
            database_connected: true,
            session_manager_override: None,
        }
    }

    pub fn with_session_manager_override(mut self, session_manager: Arc<SessionManager>) -> Self {
        self.session_manager_override = Some(session_manager);
        self
    }

    fn runtime(&self) -> &AgenaRuntime {
        &self.runtime
    }

    fn service(&self) -> &ApiService {
        &self.service
    }

    fn session_manager(&self) -> Result<Arc<SessionManager>, ApiError> {
        self.session_manager_override
            .as_ref()
            .map(Arc::clone)
            .or_else(|| self.runtime.session_manager())
            .ok_or_else(|| {
                ApiError::service_unavailable("session runtime is not available for this API")
            })
    }
}

pub fn router(state: ApiState) -> Router {
    Router::new()
        .route("/api/v1/health", get(health))
        .route("/api/v1/runtime", get(get_runtime_status))
        .route("/api/v1/runtime/reload", post(reload_runtime))
        .route("/api/v1/auth/providers", get(list_auth_providers))
        .route(
            "/api/v1/auth/providers/openai/browser/start",
            post(start_openai_browser_auth),
        )
        .route(
            "/api/v1/auth/providers/openai/browser/finish",
            post(finish_openai_browser_auth),
        )
        .route(
            "/api/v1/auth/providers/openai/device/start",
            post(start_openai_device_auth),
        )
        .route(
            "/api/v1/auth/providers/openai/device/poll",
            post(poll_openai_device_auth),
        )
        .route(
            "/api/v1/auth/providers/gitlab/browser/start",
            post(start_gitlab_browser_auth),
        )
        .route(
            "/api/v1/auth/providers/gitlab/browser/finish",
            post(finish_gitlab_browser_auth),
        )
        .route(
            "/api/v1/auth/providers/github-copilot/device/start",
            post(start_copilot_device_auth),
        )
        .route(
            "/api/v1/auth/providers/github-copilot/device/poll",
            post(poll_copilot_device_auth),
        )
        .route(
            "/api/v1/auth/providers/{provider_id}",
            get(get_auth_provider).delete(delete_auth_provider),
        )
        .route(
            "/api/v1/auth/providers/{provider_id}/api-key",
            axum::routing::put(set_auth_provider_api_key),
        )
        .route(
            "/api/v1/auth/providers/{provider_id}/refresh",
            post(refresh_auth_provider),
        )
        .route("/api/v1/providers", get(list_providers))
        .route(
            "/api/v1/providers/{provider_id}/models",
            get(list_provider_models),
        )
        .route(
            "/api/v1/workspaces",
            get(list_workspaces).post(create_workspace),
        )
        .route("/api/v1/workspaces/resolve", post(resolve_workspace))
        .route(
            "/api/v1/workspaces/{workspace_id}",
            get(get_workspace)
                .put(replace_workspace)
                .delete(delete_workspace),
        )
        .route("/api/v1/sessions", get(list_sessions).post(create_session))
        .route(
            "/api/v1/sessions/{session_id}",
            get(get_session).put(replace_session).delete(delete_session),
        )
        .route(
            "/api/v1/sessions/{session_id}/state",
            get(get_session_state),
        )
        .route(
            "/api/v1/sessions/{session_id}/events",
            get(list_session_events),
        )
        .route(
            "/api/v1/sessions/{session_id}/events/stream",
            get(stream_session_events),
        )
        .route("/api/v1/sessions/{session_id}/messages", get(list_messages))
        .route(
            "/api/v1/sessions/{session_id}/turns",
            post(submit_session_turn),
        )
        .route(
            "/api/v1/sessions/{session_id}/continue",
            post(continue_session),
        )
        .route(
            "/api/v1/sessions/{session_id}/permission-replies",
            post(reply_session_permission),
        )
        .route(
            "/api/v1/sessions/{session_id}/user-input-replies",
            post(reply_session_user_input),
        )
        .route("/api/v1/messages/{message_id}", get(get_message))
        .route(
            "/api/v1/messages/{message_id}/parts",
            get(list_message_parts),
        )
        .route("/api/v1/message-parts/{part_id}", get(get_message_part))
        .route(
            "/api/v1/permission-rules",
            get(list_permission_rules).post(create_permission_rule),
        )
        .route(
            "/api/v1/permission-rules/{rule_id}",
            get(get_permission_rule)
                .put(replace_permission_rule)
                .delete(delete_permission_rule),
        )
        .with_state(state)
}

pub struct ApiServer {
    router: Router,
    runtime: AgenaRuntime,
}

impl ApiServer {
    pub fn new(state: ApiState) -> Self {
        let runtime = state.runtime.clone();
        Self {
            router: router(state),
            runtime,
        }
    }

    pub async fn serve(self, listener: TcpListener) -> Result<(), AppError> {
        let runtime = self.runtime.clone();
        axum::serve(listener, self.router)
            .with_graceful_shutdown(async move {
                let _ = tokio::signal::ctrl_c().await;
                runtime.shutdown();
            })
            .await
            .map_err(AppError::from)
    }

    pub fn into_router(self) -> Router {
        self.router
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MessageDetailQuery {
    #[serde(default)]
    parts: PartLoadMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct MessagePartListQuery {
    #[serde(default)]
    mode: PartLoadMode,
}

async fn health(State(state): State<ApiState>) -> Json<HealthResponse> {
    let snapshot = state.runtime().current_snapshot();
    Json(HealthResponse {
        status: "ok",
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at(),
        database_connected: state.database_connected,
    })
}

async fn get_runtime_status(
    State(state): State<ApiState>,
) -> Result<Json<RuntimeStatusResponse>, ApiError> {
    Ok(Json(runtime_status_response(&state)))
}

async fn reload_runtime(
    State(state): State<ApiState>,
) -> Result<Json<RuntimeReloadResponse>, ApiError> {
    let report = state.runtime().reload().await?;
    Ok(Json(RuntimeReloadResponse {
        cause: "manual",
        previous_generation: report.previous_generation,
        generation: report.generation,
        loaded_at: report.loaded_at,
    }))
}

async fn start_openai_browser_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthBrowserStartRequest>,
) -> Result<Json<AuthBrowserStartResource>, ApiError> {
    let start = state
        .runtime()
        .auth_manager()
        .start_openai_browser_login(request.redirect_uri)?;
    Ok(Json(auth_browser_start_resource("openai", start, None)))
}

async fn finish_openai_browser_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthOpenAiBrowserFinishRequest>,
) -> Result<Json<AuthLoginResultResource>, ApiError> {
    let code = request.code;
    let pkce_verifier = request.pkce_verifier;
    let redirect_uri = request.redirect_uri;
    let token = run_auth_future(move || async move {
        agena::provider::auth::exchange_openai_oauth_code(
            code.as_str(),
            pkce_verifier.as_str(),
            redirect_uri.as_str(),
        )
        .await
    })
    .await?;
    let auth = agena::provider::auth::AuthData::OAuth {
        refresh: token.refresh,
        access: token.access,
        expires_at_ms: token.expires_at_ms,
        account_id: token.account_id,
        enterprise_url: None,
    };
    state
        .runtime()
        .auth_manager()
        .set_auth_data("openai", auth.clone())?;
    Ok(Json(auth_login_result_resource(
        &state,
        "openai",
        Some(&auth),
    )?))
}

async fn start_openai_device_auth(
    State(_state): State<ApiState>,
) -> Result<Json<AuthDeviceStartResource>, ApiError> {
    let start = run_auth_future(|| async {
        agena::provider::auth::start_openai_headless_device_code().await
    })
    .await?;
    Ok(Json(auth_device_start_resource("openai", start, None)))
}

async fn poll_openai_device_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthOpenAiDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ApiError> {
    let device_code = request.device_code;
    let user_code = request.user_code;
    let auth = run_auth_future(move || async move {
        agena::provider::auth::poll_openai_headless_device_code(
            device_code.as_str(),
            user_code.as_str(),
        )
        .await
    })
    .await?
    .map(|token| agena::provider::auth::AuthData::OAuth {
        refresh: token.refresh,
        access: token.access,
        expires_at_ms: token.expires_at_ms,
        account_id: token.account_id,
        enterprise_url: None,
    });
    if let Some(auth) = auth.as_ref() {
        state
            .runtime()
            .auth_manager()
            .set_auth_data("openai", auth.clone())?;
    }
    Ok(Json(match auth.as_ref() {
        Some(auth) => auth_login_result_resource(&state, "openai", Some(auth))?,
        None => AuthLoginResultResource {
            completed: false,
            provider: None,
        },
    }))
}

async fn start_gitlab_browser_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthGitLabBrowserStartRequest>,
) -> Result<Json<AuthBrowserStartResource>, ApiError> {
    let instance_url = request.instance_url;
    let start = state
        .runtime()
        .auth_manager()
        .start_gitlab_login(instance_url.clone(), request.redirect_uri)?;
    Ok(Json(auth_browser_start_resource(
        "gitlab",
        start,
        Some(instance_url),
    )))
}

async fn finish_gitlab_browser_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthGitLabBrowserFinishRequest>,
) -> Result<Json<AuthLoginResultResource>, ApiError> {
    let instance_url = request.instance_url;
    let code = request.code;
    let pkce_verifier = request.pkce_verifier;
    let redirect_uri = request.redirect_uri;
    let exchange_instance_url = instance_url.clone();
    let token = run_auth_future(move || async move {
        agena::provider::auth::exchange_gitlab_oauth_code(
            exchange_instance_url.as_str(),
            code.as_str(),
            pkce_verifier.as_str(),
            redirect_uri.as_str(),
        )
        .await
    })
    .await?;
    let auth = agena::provider::auth::AuthData::OAuth {
        refresh: token.refresh,
        access: token.access,
        expires_at_ms: token.expires_at_ms,
        account_id: None,
        enterprise_url: None,
    };
    state
        .runtime()
        .auth_manager()
        .set_auth_data("gitlab", auth.clone())?;
    state.runtime().auth_manager().set_auth_data(
        "gitlab-instance",
        agena::provider::auth::AuthData::WellKnown {
            key: instance_url,
            token: String::new(),
        },
    )?;
    Ok(Json(auth_login_result_resource(
        &state,
        "gitlab",
        Some(&auth),
    )?))
}

async fn start_copilot_device_auth(
    State(_state): State<ApiState>,
    request: Option<Json<AuthCopilotDeviceStartRequest>>,
) -> Result<Json<AuthDeviceStartResource>, ApiError> {
    let request = request.map(|Json(request)| request).unwrap_or_default();
    let enterprise_domain = request
        .enterprise_domain
        .and_then(normalize_optional_string);
    let provider_id = copilot_provider_id(enterprise_domain.as_deref());
    let domain = enterprise_domain
        .clone()
        .unwrap_or_else(|| "github.com".to_string());
    let start = run_auth_future(move || async move {
        agena::provider::auth::start_copilot_device_code(domain.as_str()).await
    })
    .await?;
    Ok(Json(auth_device_start_resource(
        provider_id,
        start,
        enterprise_domain,
    )))
}

async fn poll_copilot_device_auth(
    State(state): State<ApiState>,
    Json(request): Json<AuthCopilotDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ApiError> {
    let enterprise_domain = request
        .enterprise_domain
        .and_then(normalize_optional_string);
    let provider_id = copilot_provider_id(enterprise_domain.as_deref());
    let domain = enterprise_domain
        .clone()
        .unwrap_or_else(|| "github.com".to_string());
    let device_code = request.device_code;
    let auth = run_auth_future(move || async move {
        agena::provider::auth::poll_copilot_device_code(domain.as_str(), device_code.as_str()).await
    })
    .await?
    .map(|token| agena::provider::auth::AuthData::OAuth {
        refresh: token.refresh,
        access: token.access,
        expires_at_ms: token.expires_at_ms,
        account_id: None,
        enterprise_url: enterprise_domain.clone(),
    });
    if let Some(auth) = auth.as_ref() {
        state
            .runtime()
            .auth_manager()
            .set_auth_data(provider_id, auth.clone())?;
    }
    Ok(Json(match auth.as_ref() {
        Some(auth) => auth_login_result_resource(&state, provider_id, Some(auth))?,
        None => AuthLoginResultResource {
            completed: false,
            provider: None,
        },
    }))
}

async fn list_auth_providers(
    State(state): State<ApiState>,
) -> Result<Json<Vec<AuthProviderResource>>, ApiError> {
    let configured_ids = configured_provider_ids(&state);
    let auth_map = state.runtime().auth_manager().all()?;
    let provider_ids = public_auth_provider_ids(&configured_ids, &auth_map);

    let items = provider_ids
        .into_iter()
        .map(|provider_id| {
            let auth = auth_map.get(provider_id.as_str());
            auth_provider_resource(
                configured_ids.contains(provider_id.as_str()),
                provider_id,
                auth,
            )
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(items))
}

async fn get_auth_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> Result<Json<AuthProviderResource>, ApiError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state.runtime().auth_manager().get(provider_id.as_str())?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ApiError::not_found(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

async fn set_auth_provider_api_key(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
    Json(request): Json<AuthApiKeyWriteRequest>,
) -> Result<Json<AuthProviderResource>, ApiError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state.runtime().auth_manager().get(provider_id.as_str())?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ApiError::not_found(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .set_api_key(provider_id.as_str(), request.api_key)?;
    let auth = state.runtime().auth_manager().get(provider_id.as_str())?;
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        auth.as_ref(),
    )?))
}

async fn delete_auth_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> Result<Json<AuthProviderResource>, ApiError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let auth = state.runtime().auth_manager().get(provider_id.as_str())?;
    if !configured.contains(provider_id.as_str()) && auth.is_none() {
        return Err(ApiError::not_found(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    state
        .runtime()
        .auth_manager()
        .remove(provider_id.as_str())?;
    if provider_id == "gitlab" {
        state.runtime().auth_manager().remove("gitlab-instance")?;
    }
    Ok(Json(auth_provider_resource(
        configured.contains(provider_id.as_str()),
        provider_id,
        None,
    )?))
}

async fn refresh_auth_provider(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> Result<Json<AuthProviderResource>, ApiError> {
    ensure_public_auth_provider_id(provider_id.as_str())?;
    let configured = configured_provider_ids(&state);
    let existing = state.runtime().auth_manager().get(provider_id.as_str())?;
    if !configured.contains(provider_id.as_str()) && existing.is_none() {
        return Err(ApiError::not_found(format!(
            "auth provider not found: {provider_id}"
        )));
    }

    let auth = match provider_id.as_str() {
        "openai" => {
            state
                .runtime()
                .auth_manager()
                .refresh_openai_login()
                .await?
        }
        "gitlab" => {
            state
                .runtime()
                .auth_manager()
                .refresh_gitlab_login()
                .await?
        }
        _ => {
            return Err(ApiError::bad_request(format!(
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

async fn list_providers(State(state): State<ApiState>) -> Json<Vec<ProviderSummaryResource>> {
    let snapshot = state.runtime().current_snapshot();
    let registry = snapshot.provider_registry();
    let mut providers = registry
        .provider_ids()
        .into_iter()
        .filter_map(|provider_id| {
            registry
                .get(provider_id.as_str())
                .map(|provider| ProviderSummaryResource {
                    default_model_ref: format!("{provider_id}/{}", provider.default_model()),
                    default_model: provider.default_model().to_string(),
                    provider_id,
                })
        })
        .collect::<Vec<_>>();
    providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
    Json(providers)
}

async fn list_provider_models(
    State(state): State<ApiState>,
    Path(provider_id): Path<String>,
) -> Result<Json<ProviderModelsResponse>, ApiError> {
    let snapshot = state.runtime().current_snapshot();
    if snapshot
        .provider_registry()
        .get(provider_id.as_str())
        .is_none()
    {
        return Err(ApiError::not_found(format!(
            "provider not found: {provider_id}"
        )));
    }

    let models = snapshot.list_provider_models(provider_id.as_str()).await?;
    Ok(Json(ProviderModelsResponse {
        provider_id,
        models,
    }))
}

async fn list_workspaces(
    State(state): State<ApiState>,
    Query(query): Query<WorkspaceListQuery>,
) -> Result<Json<PaginatedResponse<WorkspaceResource>>, ApiError> {
    Ok(Json(state.service().list_workspaces(query).await?))
}

async fn get_workspace(
    State(state): State<ApiState>,
    Path(workspace_id): Path<i64>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    state
        .service()
        .get_workspace(workspace_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("workspace not found: {workspace_id}")))
}

async fn create_workspace(
    State(state): State<ApiState>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    Ok(Json(state.service().create_workspace(request).await?))
}

async fn resolve_workspace(
    State(state): State<ApiState>,
    Json(request): Json<WorkspaceResolveRequest>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    Ok(Json(state.service().resolve_workspace(request).await?))
}

async fn replace_workspace(
    State(state): State<ApiState>,
    Path(workspace_id): Path<i64>,
    Json(request): Json<WorkspaceWriteRequest>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    Ok(Json(
        state
            .service()
            .replace_workspace(workspace_id, request)
            .await?,
    ))
}

async fn delete_workspace(
    State(state): State<ApiState>,
    Path(workspace_id): Path<i64>,
) -> Result<Json<WorkspaceResource>, ApiError> {
    Ok(Json(state.service().delete_workspace(workspace_id).await?))
}

async fn list_sessions(
    State(state): State<ApiState>,
    Query(query): Query<dto::SessionListQuery>,
) -> Result<Json<PaginatedResponse<SessionResource>>, ApiError> {
    Ok(Json(state.service().list_sessions(query).await?))
}

async fn get_session(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
) -> Result<Json<SessionResource>, ApiError> {
    state
        .service()
        .get_session(session_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("session not found: {session_id}")))
}

async fn get_session_state(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
) -> Result<Json<SessionExecutionResource>, ApiError> {
    let session = state.session_manager()?.get_session(session_id).await?;
    Ok(Json(
        state.service().session_execution_resource(&session).await?,
    ))
}

async fn create_session(
    State(state): State<ApiState>,
    Json(request): Json<SessionCreateRequest>,
) -> Result<Json<SessionResource>, ApiError> {
    Ok(Json(state.service().create_session(request).await?))
}

async fn replace_session(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionReplaceRequest>,
) -> Result<Json<SessionResource>, ApiError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }
    Ok(Json(
        state.service().replace_session(session_id, request).await?,
    ))
}

async fn delete_session(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
) -> Result<Json<SessionResource>, ApiError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }
    Ok(Json(state.service().delete_session(session_id).await?))
}

async fn list_session_events(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    Query(query): Query<SessionEventListQuery>,
) -> Result<Json<PaginatedResponse<agena::session::SessionEventRecord>>, ApiError> {
    Ok(Json(
        state
            .service()
            .list_session_events(session_id, query)
            .await?,
    ))
}

async fn stream_session_events(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    Query(query): Query<SessionEventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    let service = state.service().clone();
    let mut after_seq = match query.after_seq {
        Some(after_seq) => after_seq,
        None => service
            .latest_session_event_seq(session_id)
            .await?
            .unwrap_or(0),
    };
    let limit = pagination::normalize_limit(query.limit);
    let poll_interval = duration_from_ms(query.poll_interval_ms, 250, "poll_interval_ms")?;
    let idle_timeout = optional_duration_from_ms(query.idle_timeout_ms, "idle_timeout_ms")?;

    let stream = stream! {
        let mut last_activity = Instant::now();

        loop {
            match service
                .list_session_events_after(session_id, after_seq, Some(limit))
                .await
            {
                Ok(events) => {
                    if !events.is_empty() {
                        last_activity = Instant::now();
                        for record in events {
                            after_seq = record.seq;
                            match Event::default()
                                .event("session_event")
                                .id(record.seq.to_string())
                                .json_data(&record)
                            {
                                Ok(event) => yield Ok(event),
                                Err(error) => {
                                    yield Ok(sse_error_event(format!(
                                        "failed to encode session event: {error}"
                                    )));
                                    return;
                                }
                            }
                        }
                        continue;
                    }
                }
                Err(error) => {
                    yield Ok(sse_error_event(error_message(&error)));
                    return;
                }
            }

            if idle_timeout.is_some_and(|timeout| last_activity.elapsed() >= timeout) {
                return;
            }

            sleep(poll_interval).await;
        }
    };

    Ok(Sse::new(stream))
}

async fn submit_session_turn(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionTurnRequest>,
) -> Result<Json<SessionExecutionResource>, ApiError> {
    if request.parts.is_empty() {
        return Err(ApiError::bad_request(
            "session turn requires at least one part",
        ));
    }
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let session = state
        .session_manager()?
        .submit_user_turn(agena::session::SessionUserTurnRequest {
            session_id,
            options,
            parts: request.parts,
        })
        .await?;
    Ok(Json(
        state.service().session_execution_resource(&session).await?,
    ))
}

async fn continue_session(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionContinueRequestBody>,
) -> Result<Json<SessionExecutionResource>, ApiError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let session = state
        .session_manager()?
        .continue_session(agena::session::SessionContinueRequest {
            session_id,
            options,
        })
        .await?;
    Ok(Json(
        state.service().session_execution_resource(&session).await?,
    ))
}

async fn reply_session_permission(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionPermissionReplyRequestBody>,
) -> Result<Json<SessionExecutionResource>, ApiError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let session = state
        .session_manager()?
        .reply_permission(agena::session::SessionPermissionReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await?;
    Ok(Json(
        state.service().session_execution_resource(&session).await?,
    ))
}

async fn reply_session_user_input(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    headers: HeaderMap,
    Json(request): Json<SessionUserInputReplyRequestBody>,
) -> Result<Json<SessionExecutionResource>, ApiError> {
    if let Some(expected_version) = if_match_version(&headers)? {
        state
            .service()
            .assert_session_version(session_id, expected_version)
            .await?;
    }

    let options = resolve_run_options(&state, session_id, request.options).await?;
    let session = state
        .session_manager()?
        .reply_user_input(agena::session::SessionUserInputReplyRequest {
            session_id,
            options,
            reply: request.reply,
        })
        .await?;
    Ok(Json(
        state.service().session_execution_resource(&session).await?,
    ))
}

async fn list_messages(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    Query(query): Query<MessageListQuery>,
) -> Result<Json<PaginatedResponse<MessageResource>>, ApiError> {
    Ok(Json(
        state.service().list_messages(session_id, query).await?,
    ))
}

async fn get_message(
    State(state): State<ApiState>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessageDetailQuery>,
) -> Result<Json<MessageResource>, ApiError> {
    state
        .service()
        .get_message(message_id, query.parts)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("message not found: {message_id}")))
}

async fn list_message_parts(
    State(state): State<ApiState>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessagePartListQuery>,
) -> Result<Json<Vec<agena::message::MessagePart>>, ApiError> {
    Ok(Json(
        state
            .service()
            .list_message_parts(message_id, query.mode)
            .await?,
    ))
}

async fn get_message_part(
    State(state): State<ApiState>,
    Path(part_id): Path<i64>,
) -> Result<Json<agena::message::MessagePart>, ApiError> {
    state
        .service()
        .get_message_part(part_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("message part not found: {part_id}")))
}

async fn list_permission_rules(
    State(state): State<ApiState>,
    Query(query): Query<PermissionRuleListQuery>,
) -> Result<Json<PaginatedResponse<PermissionRuleResource>>, ApiError> {
    Ok(Json(state.service().list_permission_rules(query).await?))
}

async fn get_permission_rule(
    State(state): State<ApiState>,
    Path(rule_id): Path<i64>,
) -> Result<Json<PermissionRuleResource>, ApiError> {
    state
        .service()
        .get_permission_rule(rule_id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("permission rule not found: {rule_id}")))
}

async fn create_permission_rule(
    State(state): State<ApiState>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<Json<PermissionRuleResource>, ApiError> {
    Ok(Json(state.service().create_permission_rule(request).await?))
}

async fn replace_permission_rule(
    State(state): State<ApiState>,
    Path(rule_id): Path<i64>,
    Json(request): Json<PermissionRuleWriteRequest>,
) -> Result<Json<PermissionRuleResource>, ApiError> {
    Ok(Json(
        state
            .service()
            .replace_permission_rule(rule_id, request)
            .await?,
    ))
}

async fn delete_permission_rule(
    State(state): State<ApiState>,
    Path(rule_id): Path<i64>,
) -> Result<Json<PermissionRuleResource>, ApiError> {
    Ok(Json(state.service().delete_permission_rule(rule_id).await?))
}

async fn resolve_run_options(
    state: &ApiState,
    session_id: i64,
    request: dto::SessionRunOptionsRequest,
) -> Result<agena::session::SessionRunOptions, ApiError> {
    let snapshot = state.runtime().current_snapshot();
    state
        .service()
        .resolve_run_options(snapshot.provider_registry().as_ref(), session_id, request)
        .await
}

fn if_match_version(headers: &HeaderMap) -> Result<Option<i64>, ApiError> {
    let Some(value) = headers.get(IF_MATCH) else {
        return Ok(None);
    };
    let raw = value
        .to_str()
        .map_err(|error| ApiError::bad_request(format!("invalid If-Match header: {error}")))?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request("If-Match header cannot be empty"));
    }
    if trimmed == "*" {
        return Err(ApiError::bad_request(
            "If-Match '*' is not supported for session version checks",
        ));
    }
    if trimmed.contains(',') {
        return Err(ApiError::bad_request(
            "If-Match must contain exactly one session version",
        ));
    }

    let version_text = trimmed
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .unwrap_or(trimmed);
    let version = version_text.parse::<i64>().map_err(|error| {
        ApiError::bad_request(format!(
            "If-Match must be a numeric session version: {error}"
        ))
    })?;

    Ok(Some(version))
}

fn duration_from_ms(
    value: Option<u64>,
    default_ms: u64,
    field: &'static str,
) -> Result<Duration, ApiError> {
    let value = value.unwrap_or(default_ms);
    if value == 0 {
        return Err(ApiError::bad_request(format!(
            "{field} must be greater than zero"
        )));
    }
    Ok(Duration::from_millis(value.clamp(10, 30_000)))
}

fn optional_duration_from_ms(
    value: Option<u64>,
    field: &'static str,
) -> Result<Option<Duration>, ApiError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value == 0 {
        return Err(ApiError::bad_request(format!(
            "{field} must be greater than zero"
        )));
    }
    Ok(Some(Duration::from_millis(value.clamp(10, 300_000))))
}

async fn run_auth_future<T, Fut, F>(task: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    Fut: Future<Output = Result<T, AppError>> + 'static,
    F: FnOnce() -> Fut + Send + 'static,
{
    let handle = tokio::runtime::Handle::current();
    tokio::task::spawn_blocking(move || handle.block_on(task()))
        .await
        .map_err(|error| ApiError::internal(format!("auth task join failed: {error}")))?
        .map_err(ApiError::from)
}

fn sse_error_event(message: impl Into<String>) -> Event {
    Event::default().event("error").data(message.into())
}

fn error_message(error: &ApiError) -> String {
    let response = error.clone().into_response();
    format!("request failed with status {}", response.status())
}

fn runtime_status_response(state: &ApiState) -> RuntimeStatusResponse {
    let snapshot = state.runtime().current_snapshot();
    let resolution = snapshot.config_resolution();
    let mut provider_ids = snapshot.provider_registry().provider_ids();
    provider_ids.sort();

    RuntimeStatusResponse {
        generation: snapshot.generation(),
        loaded_at: snapshot.loaded_at(),
        workspace_root: state.runtime().workspace_root().display().to_string(),
        config_path: resolution.meta.config_path.display().to_string(),
        config_found: resolution.meta.config_found,
        active_mode: resolution
            .meta
            .active_mode
            .as_ref()
            .map(ToString::to_string),
        active_mode_source: resolution.meta.active_mode_source,
        auth_store_path: resolution.config.auth.store_path.display().to_string(),
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
    }
}

fn configured_provider_ids(state: &ApiState) -> BTreeSet<String> {
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

fn ensure_public_auth_provider_id(provider_id: &str) -> Result<(), ApiError> {
    if is_internal_auth_provider_id(provider_id) {
        return Err(ApiError::not_found(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(())
}

fn is_internal_auth_provider_id(provider_id: &str) -> bool {
    provider_id == "gitlab-instance"
}

fn auth_browser_start_resource(
    provider_id: &str,
    start: agena::provider::auth::OAuthAuthorizeStart,
    instance_url: Option<String>,
) -> AuthBrowserStartResource {
    AuthBrowserStartResource {
        provider_id: provider_id.to_string(),
        instance_url,
        authorize_url: start.authorize_url,
        state: start.state,
        pkce_verifier: start.pkce_verifier,
    }
}

fn auth_device_start_resource(
    provider_id: &str,
    start: agena::provider::auth::DeviceCodeStart,
    enterprise_domain: Option<String>,
) -> AuthDeviceStartResource {
    AuthDeviceStartResource {
        provider_id: provider_id.to_string(),
        enterprise_domain,
        verification_url: start.verification_url,
        user_code: start.user_code,
        device_code: start.device_code,
        interval_seconds: start.interval_seconds,
    }
}

fn auth_login_result_resource(
    state: &ApiState,
    provider_id: &str,
    auth: Option<&agena::provider::auth::AuthData>,
) -> Result<AuthLoginResultResource, ApiError> {
    Ok(AuthLoginResultResource {
        completed: auth.is_some(),
        provider: auth
            .map(|auth| {
                auth_provider_resource(
                    configured_provider_ids(state).contains(provider_id),
                    provider_id.to_string(),
                    Some(auth),
                )
            })
            .transpose()?,
    })
}

fn copilot_provider_id(enterprise_domain: Option<&str>) -> &'static str {
    if enterprise_domain.is_some() {
        "github-copilot-enterprise"
    } else {
        "github-copilot"
    }
}

fn normalize_optional_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn auth_provider_resource(
    configured: bool,
    provider_id: String,
    auth: Option<&agena::provider::auth::AuthData>,
) -> Result<AuthProviderResource, ApiError> {
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
                    DateTime::from_timestamp_millis(*expires_at_ms).ok_or_else(|| {
                        ApiError::internal(format!("invalid oauth expiry millis: {expires_at_ms}"))
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

#[cfg(test)]
mod tests {
    use std::{
        fs,
        path::Path,
        path::PathBuf,
        pin::Pin,
        sync::Arc,
        time::{SystemTime, UNIX_EPOCH},
    };

    use async_trait::async_trait;
    use axum::{
        body::{Body, to_bytes},
        http::{Method, Request, StatusCode},
    };
    use futures_util::{Stream, stream};
    use sea_orm::{Database, DatabaseConnection};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    use agena::{
        agent::Agent,
        db::init_schema,
        message::{
            AskUserToolInput, BuiltinToolOutput, Message, MessageSource, PartContent,
            ToolExecutionPart, UserInputOption, UserInputQuestion,
        },
        model::ModelId,
        permission::PermissionPolicy,
        provider::{
            CompletionFinishReason, CompletionRequest, CompletionResponse, CompletionStreamEvent,
            ModelProvider, ProviderModel, ProviderRegistry,
        },
        role::Role,
        runtime::AgenaRuntime,
        session::{ContextGovernor, ContextPolicy, SessionManager, SessionProcessor},
        tool::ToolExecutor,
    };

    use super::*;
    use crate::dto::MessageWriteRequest;

    #[tokio::test]
    async fn sessions_endpoint_uses_cursor_pagination() {
        let (app, state) = test_app().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/pagination".to_string(),
            })
            .await
            .expect("workspace should be created");
        for idx in 0..3 {
            state
                .service()
                .create_session(SessionCreateRequest {
                    workspace_id: workspace.id,
                    title: format!("session-{idx}"),
                    parent_id: None,
                })
                .await
                .expect("session should be created");
        }

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/sessions?workspace_id={}&limit=2",
                        workspace.id
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let first_status = first.status();
        let first_json = response_json(first).await;
        assert_eq!(
            first_status,
            StatusCode::OK,
            "unexpected body: {first_json}"
        );
        let first_items = first_json["items"]
            .as_array()
            .expect("items should be an array");
        assert_eq!(first_items.len(), 2);
        let cursor = first_json["page"]["next_cursor"]
            .as_str()
            .expect("next cursor should exist");

        let second = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/sessions?workspace_id={}&limit=2&cursor={cursor}",
                        workspace.id
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let second_status = second.status();
        let second_json = response_json(second).await;
        assert_eq!(
            second_status,
            StatusCode::OK,
            "unexpected body: {second_json}"
        );
        let second_items = second_json["items"]
            .as_array()
            .expect("items should be an array");
        assert_eq!(second_items.len(), 1);
    }

    #[tokio::test]
    async fn messages_endpoint_supports_summary_and_full_parts() {
        let (app, state) = test_app().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/messages".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "messages".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");
        let payload = MessageWriteRequest {
            role: agena::role::Role::Assistant,
            state: None,
            metadata: Some(agena::message::MessageMetadata {
                source: MessageSource::Assistant,
                parent_message_id: None,
                generated_by_call_id: None,
                model_provider_id: "openai".to_string(),
                model_id: "gpt-4.1-mini".to_string(),
                tags: Vec::new(),
            }),
            usage: None,
            finish: Some("stop".to_string()),
            created_at: None,
            parts: vec![dto::MessagePartWriteRequest {
                content: PartContent::text("hello world"),
                status: None,
                operation_id: None,
                created_at: None,
            }],
        };
        let created = state
            .service()
            .create_message(session.id, payload)
            .await
            .expect("message should be created");

        let summary = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/messages/{}?parts=summary", created.id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(summary.status(), StatusCode::OK);
        let summary_json = response_json(summary).await;
        assert!(summary_json["parts"][0]["content"].is_null());

        let full = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/messages/{}?parts=full", created.id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(full.status(), StatusCode::OK);
        let full_json = response_json(full).await;
        assert_eq!(full_json["parts"][0]["content"]["type"], json!("text"));
    }

    #[tokio::test]
    async fn providers_endpoint_returns_configured_provider() {
        let (app, _state) = test_app().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/providers")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(response.status(), StatusCode::OK);
        let json = response_json(response).await;
        let items = json.as_array().expect("providers should be an array");
        assert!(
            items
                .iter()
                .any(|item| item["provider_id"] == json!("openai")),
            "expected openai provider"
        );
    }

    #[tokio::test]
    async fn workspace_resolve_endpoint_returns_existing_normalized_workspace() {
        let (app, state) = test_app().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/resolve-existing".to_string(),
            })
            .await
            .expect("workspace should be created");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "path": "/tmp/./resolve-existing/"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["id"], json!(workspace.id));
        assert_eq!(payload["path"], json!("/tmp/resolve-existing"));
    }

    #[tokio::test]
    async fn workspace_resolve_endpoint_creates_missing_workspace_when_requested() {
        let (app, state) = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "path": "/tmp/resolve-create/",
                            "create_if_missing": true
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;
        let workspace_id = payload["id"]
            .as_i64()
            .expect("workspace id should be returned");

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["path"], json!("/tmp/resolve-create"));
        assert!(
            state
                .service()
                .get_workspace(workspace_id)
                .await
                .expect("workspace lookup should succeed")
                .is_some(),
            "workspace should have been created"
        );
    }

    #[tokio::test]
    async fn workspace_resolve_endpoint_returns_not_found_without_create_flag() {
        let (app, _state) = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "path": "/tmp/resolve-missing"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::NOT_FOUND, "unexpected body: {payload}");
        assert_eq!(payload["error"]["code"], json!("not_found"));
    }

    #[tokio::test]
    async fn session_state_endpoint_returns_pending_runtime_state() {
        let (app, state, _workspace) = test_app_with_scripted_manager().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/runtime-state".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "runtime state".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        state
            .session_manager()
            .expect("session manager should exist")
            .submit_user_turn(agena::session::SessionUserTurnRequest {
                session_id: session.id,
                options: state
                    .service()
                    .resolve_run_options(
                        state
                            .runtime()
                            .current_snapshot()
                            .provider_registry()
                            .as_ref(),
                        session.id,
                        dto::SessionRunOptionsRequest::default(),
                    )
                    .await
                    .expect("options should resolve"),
                parts: vec![PartContent::text("please choose model")],
            })
            .await
            .expect("turn should succeed");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/sessions/{}/state", session.id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["blocked"], json!(true));
        assert_eq!(payload["run_state"], json!("idle"));
        assert_eq!(
            payload["pending_user_input_requests"][0]["request_id"],
            json!("call_ask_user_1")
        );
    }

    #[tokio::test]
    async fn runtime_status_and_reload_endpoints_report_current_runtime() {
        let (app, _state) = test_app().await;
        let before = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/runtime")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let before_status = before.status();
        let before_json = response_json(before).await;

        assert_eq!(
            before_status,
            StatusCode::OK,
            "unexpected body: {before_json}"
        );
        let before_generation = before_json["generation"]
            .as_u64()
            .expect("generation should exist");
        assert_eq!(before_json["provider_ids"], json!(["openai"]));
        assert_eq!(before_json["session_runtime_available"], json!(true));

        let reloaded = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/runtime/reload")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let reload_status = reloaded.status();
        let reload_json = response_json(reloaded).await;

        assert_eq!(
            reload_status,
            StatusCode::OK,
            "unexpected body: {reload_json}"
        );
        assert_eq!(reload_json["cause"], json!("manual"));
        assert_eq!(reload_json["previous_generation"], json!(before_generation));
        assert_eq!(reload_json["generation"], json!(before_generation + 1));
    }

    #[tokio::test]
    async fn auth_provider_endpoints_manage_api_keys_without_exposing_secrets() {
        let (app, _state) = test_app().await;

        let initial = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/auth/providers/openai")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let initial_json = response_json(initial).await;
        assert_eq!(initial_json["configured"], json!(true));
        assert_eq!(initial_json["credential_present"], json!(false));

        let updated = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri("/api/v1/auth/providers/openai/api-key")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "api_key": "sk-test-12345678"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let updated_status = updated.status();
        let updated_json = response_json(updated).await;

        assert_eq!(
            updated_status,
            StatusCode::OK,
            "unexpected body: {updated_json}"
        );
        assert_eq!(updated_json["credential_present"], json!(true));
        assert_eq!(updated_json["credential_type"], json!("api"));
        assert_eq!(updated_json["key_preview"], json!("sk-t...5678"));

        let listed = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/auth/providers")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let listed_json = response_json(listed).await;
        assert!(
            listed_json
                .as_array()
                .is_some_and(|items| items.iter().any(|item| {
                    item["provider_id"] == json!("openai")
                        && item["credential_present"] == json!(true)
                }))
        );

        let deleted = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri("/api/v1/auth/providers/openai")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let deleted_status = deleted.status();
        let deleted_json = response_json(deleted).await;

        assert_eq!(
            deleted_status,
            StatusCode::OK,
            "unexpected body: {deleted_json}"
        );
        assert_eq!(deleted_json["credential_present"], json!(false));
        assert!(deleted_json["credential_type"].is_null());
    }

    #[tokio::test]
    async fn openai_browser_start_endpoint_returns_authorize_url_and_pkce_state() {
        let (app, _state) = test_app().await;

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/auth/providers/openai/browser/start")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "redirect_uri": "https://app.example.com/auth/callback"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["provider_id"], json!("openai"));
        assert!(payload["authorize_url"].as_str().is_some_and(|url| {
            url.contains("redirect_uri=https%3A%2F%2Fapp.example.com%2Fauth%2Fcallback")
        }));
        assert!(
            payload["state"]
                .as_str()
                .is_some_and(|state| !state.is_empty())
        );
        assert!(
            payload["pkce_verifier"]
                .as_str()
                .is_some_and(|value| !value.is_empty())
        );
    }

    #[test]
    fn public_auth_provider_ids_hide_internal_records() {
        let configured = BTreeSet::from(["openai".to_string()]);
        let auth_map = HashMap::from([
            (
                "gitlab".to_string(),
                agena::provider::auth::AuthData::OAuth {
                    refresh: "refresh".to_string(),
                    access: "access".to_string(),
                    expires_at_ms: 0,
                    account_id: None,
                    enterprise_url: None,
                },
            ),
            (
                "gitlab-instance".to_string(),
                agena::provider::auth::AuthData::WellKnown {
                    key: "https://gitlab.example.com".to_string(),
                    token: String::new(),
                },
            ),
        ]);

        let provider_ids = public_auth_provider_ids(&configured, &auth_map);

        assert!(provider_ids.contains("openai"));
        assert!(provider_ids.contains("gitlab"));
        assert!(!provider_ids.contains("gitlab-instance"));
    }

    #[tokio::test]
    async fn turn_endpoint_returns_pending_user_input_and_uses_default_model() {
        let (app, state, _workspace) = test_app_with_scripted_manager().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/runtime-turn".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "runtime turn".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/sessions/{}/turns", session.id))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "parts": [
                                {
                                    "type": "text",
                                    "text": "please choose model"
                                }
                            ]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["blocked"], json!(true));
        assert_eq!(payload["run_state"], json!("idle"));
        assert_eq!(
            payload["pending_user_input_requests"][0]["request_id"],
            json!("call_ask_user_1")
        );
        assert!(payload["latest_event_seq"].as_i64().is_some());
    }

    #[tokio::test]
    async fn user_input_reply_endpoint_resumes_session() {
        let (app, state, _workspace) = test_app_with_scripted_manager().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/runtime-reply".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "runtime reply".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        let initial = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/sessions/{}/turns", session.id))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "parts": [
                                {
                                    "type": "text",
                                    "text": "please choose model"
                                }
                            ]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let initial_json = response_json(initial).await;
        let version = initial_json["session"]["version"]
            .as_i64()
            .expect("session version should exist");

        let resumed = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!(
                        "/api/v1/sessions/{}/user-input-replies",
                        session.id
                    ))
                    .header("content-type", "application/json")
                    .header("if-match", version.to_string())
                    .body(Body::from(
                        json!({
                            "reply": {
                                "request_id": "call_ask_user_1",
                                "kind": "submit",
                                "answers": {
                                    "model_choice": "gpt-5"
                                }
                            }
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let resumed_status = resumed.status();
        let resumed_json = response_json(resumed).await;

        assert_eq!(
            resumed_status,
            StatusCode::OK,
            "unexpected body: {resumed_json}"
        );
        assert_eq!(resumed_json["blocked"], json!(false));
        assert_eq!(resumed_json["run_state"], json!("idle"));
        assert_eq!(resumed_json["pending_user_input_requests"], json!([]));

        let messages = state
            .service()
            .list_messages(
                session.id,
                MessageListQuery {
                    cursor: None,
                    limit: None,
                    parts: PartLoadMode::Full,
                },
            )
            .await
            .expect("messages should load");
        assert!(
            messages
                .items
                .last()
                .is_some_and(|message| message.parts.iter().flatten().any(|part| {
                    matches!(
                        part.content.as_ref(),
                        Some(PartContent::Text(text)) if text.text == "selected model: gpt-5"
                    )
                }))
        );
    }

    #[tokio::test]
    async fn stream_endpoint_emits_session_events_and_closes_after_idle_timeout() {
        let (app, state, _workspace) = test_app_with_scripted_manager().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/runtime-stream".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "runtime stream".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        state
            .session_manager()
            .expect("session manager should exist")
            .submit_user_turn(agena::session::SessionUserTurnRequest {
                session_id: session.id,
                options: state
                    .service()
                    .resolve_run_options(
                        state
                            .runtime()
                            .current_snapshot()
                            .provider_registry()
                            .as_ref(),
                        session.id,
                        dto::SessionRunOptionsRequest::default(),
                    )
                    .await
                    .expect("options should resolve"),
                parts: vec![PartContent::text("please choose model")],
            })
            .await
            .expect("turn should succeed");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/sessions/{}/events/stream?after_seq=0&poll_interval_ms=10&idle_timeout_ms=40",
                        session.id
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let body = response_text(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {body}");
        assert!(body.contains("event: session_event"));
        assert!(body.contains("\"session_id\":"));
    }

    #[tokio::test]
    async fn turn_endpoint_rejects_if_match_version_mismatch() {
        let (app, state) = test_app().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/runtime-if-match".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "runtime mismatch".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri(format!("/api/v1/sessions/{}/turns", session.id))
                    .header("if-match", "999")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "parts": [
                                {
                                    "type": "text",
                                    "text": "hello"
                                }
                            ]
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::CONFLICT, "unexpected body: {payload}");
        assert_eq!(payload["error"]["code"], json!("conflict"));
    }

    async fn test_app() -> (Router, ApiState) {
        let db = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("database should connect"),
        );
        init_schema(db.as_ref())
            .await
            .expect("schema should initialize");
        let runtime = test_runtime(db.clone()).await;
        let state = ApiState::new(runtime, db);
        let app = router(state.clone());
        (app, state)
    }

    async fn test_app_with_scripted_manager() -> (Router, ApiState, TempWorkspace) {
        let db = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("database should connect"),
        );
        init_schema(db.as_ref())
            .await
            .expect("schema should initialize");
        let runtime = test_runtime(db.clone()).await;
        let workspace = TempWorkspace::new();
        let manager = scripted_session_manager(db.clone(), workspace.root.as_path()).await;
        let state = ApiState::new(runtime, db).with_session_manager_override(manager);
        let app = router(state.clone());
        (app, state, workspace)
    }

    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-api-runtime-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("temp workspace should be created");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    struct ScriptedApiProvider;

    #[async_trait]
    impl ModelProvider for ScriptedApiProvider {
        fn id(&self) -> &str {
            "openai"
        }

        fn default_model(&self) -> &ModelId {
            static DEFAULT_MODEL: std::sync::LazyLock<ModelId> =
                std::sync::LazyLock::new(|| ModelId::new("gpt-4.1-mini"));
            &DEFAULT_MODEL
        }

        async fn list_models(&self) -> Result<Vec<ProviderModel>, AppError> {
            Ok(vec![ProviderModel::new("openai", "gpt-4.1-mini")])
        }

        async fn complete(
            &self,
            _request: CompletionRequest,
        ) -> Result<CompletionResponse, AppError> {
            Ok(CompletionResponse {
                provider_id: agena::model::ProviderId::new("openai"),
                model: ModelId::new("gpt-4.1-mini"),
                text: String::new(),
                finish_reason: Some(CompletionFinishReason::Stop),
                tool_calls: Vec::new(),
                usage: None,
                provider_metadata: None,
            })
        }

        async fn complete_stream(
            &self,
            request: CompletionRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let last_user_text = request
                .messages
                .iter()
                .rev()
                .find(|message| message.role == Role::User)
                .map(Message::as_text_lossy)
                .unwrap_or_default();
            let user_input_result = request.messages.iter().find_map(|message| {
                if message.role != Role::Tool {
                    return None;
                }
                message.parts.iter().find_map(|part| {
                    if part.operation_id.as_deref() != Some("call_ask_user_1") {
                        return None;
                    }
                    match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details:
                                agena::message::ToolOutput::Builtin {
                                    output: BuiltinToolOutput::AskUser { answers },
                                },
                            ..
                        })) => answers
                            .get("model_choice")
                            .and_then(|values| values.first().cloned())
                            .map(Ok),
                        Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                            error_message,
                            ..
                        })) => Some(Err(error_message.clone())),
                        _ => None,
                    }
                })
            });

            let events = if last_user_text.contains("choose model") && user_input_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_ask_user_1".to_string(),
                        id: Some("call_ask_user_1".to_string()),
                        name: Some("ask_user".to_string()),
                        arguments_delta: serde_json::to_string(&AskUserToolInput {
                            questions: vec![UserInputQuestion {
                                id: "model_choice".to_string(),
                                header: "Model".to_string(),
                                question: "Which model should we use?".to_string(),
                                options: vec![
                                    UserInputOption {
                                        label: "gpt-5".to_string(),
                                        description: "Use the flagship reasoning model."
                                            .to_string(),
                                    },
                                    UserInputOption {
                                        label: "gpt-4.1".to_string(),
                                        description: "Use the faster general-purpose model."
                                            .to_string(),
                                    },
                                ],
                                multiple: false,
                                allow_custom: false,
                            }],
                        })
                        .expect("ask_user input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if let Some(user_input_result) = user_input_result {
                let delta = match user_input_result {
                    Ok(answer) => format!("selected model: {answer}"),
                    Err(_) => "selection cancelled".to_string(),
                };
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        delta,
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else {
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        delta: format!("echo:{last_user_text}"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::Stop),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            };

            Ok(Box::pin(stream::iter(events)))
        }
    }

    async fn scripted_session_manager(
        db: Arc<DatabaseConnection>,
        workspace_root: &Path,
    ) -> Arc<SessionManager> {
        let mut registry = ProviderRegistry::new();
        registry.register(ScriptedApiProvider);
        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(ContextPolicy::default()),
        );
        let executor = ToolExecutor::new(
            workspace_root.to_path_buf(),
            Agent::new("api-test", PermissionPolicy::allow_all()),
        );
        Arc::new(SessionManager::new(
            db.as_ref().clone(),
            processor,
            executor,
        ))
    }

    async fn test_runtime(db: Arc<DatabaseConnection>) -> AgenaRuntime {
        let auth_path = temp_path("auth.json");
        let auth_store_path = format!("{:?}", auth_path.display().to_string());
        let config = write_temp_config(
            format!(
                r#"
[tracing]
filter = "info"

[auth]
store_path = {}

[providers.openai]
kind = "openai"
base_url = "https://api.openai.com/v1"
default_model = "gpt-4.1-mini"
api_key = "test"
"#,
                auth_store_path
            )
            .as_str(),
        );
        let workspace_root = config
            .parent()
            .expect("config should have parent")
            .to_path_buf();
        AgenaRuntime::builder()
            .with_load_request(agena::config::LoadConfigRequest {
                config_path: Some(config),
                ..Default::default()
            })
            .with_workspace_root(workspace_root)
            .with_database_connection(db.as_ref().clone())
            .build()
            .await
            .expect("runtime should build")
    }

    async fn response_json(response: axum::response::Response) -> Value {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        serde_json::from_slice(&body).unwrap_or_else(|error| {
            panic!(
                "response should be json: {error}; body={}",
                String::from_utf8_lossy(&body)
            )
        })
    }

    async fn response_text(response: axum::response::Response) -> String {
        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body should read");
        String::from_utf8(body.to_vec()).expect("response should be utf-8")
    }

    fn write_temp_config(contents: &str) -> PathBuf {
        let path = temp_path("toml");
        fs::write(&path, contents).expect("config should be written");
        path
    }

    fn temp_path(extension: &str) -> PathBuf {
        let mut path = std::env::temp_dir();
        path.push(format!(
            "agena-api-test-{}-{}.{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("time should advance")
                .as_nanos(),
            extension
        ));
        path
    }
}
