mod dto;
mod error;
mod pagination;
mod service;

use std::{
    collections::{BTreeSet, HashMap},
    convert::Infallible,
    future::Future,
    sync::Arc,
};

use async_stream::stream;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, header::IF_MATCH},
    response::sse::{Event, Sse},
    routing::{get, post},
};
use chrono::DateTime;
use sea_orm::DatabaseConnection;
use serde::Deserialize;
use tokio::net::TcpListener;

use agena::{AppError, runtime::AgenaRuntime, session::SessionManager};

pub use dto::{
    AuthApiKeyWriteRequest, AuthBrowserStartRequest, AuthBrowserStartResource,
    AuthCopilotDevicePollRequest, AuthCopilotDeviceStartRequest, AuthCredentialType,
    AuthDeviceStartResource, AuthGitLabBrowserFinishRequest, AuthGitLabBrowserStartRequest,
    AuthLoginResultResource, AuthOpenAiBrowserFinishRequest, AuthOpenAiDevicePollRequest,
    AuthProviderResource, HealthResponse, MessageListQuery, MessageResource, PartLoadMode,
    PermissionRuleListQuery, PermissionRuleResource, PermissionRuleWriteRequest,
    ProviderModelsResponse, ProviderSummaryResource, RuntimeReloadResponse,
    RuntimeSessionCacheResource, RuntimeStatusResponse, RuntimeTaskResource,
    SessionContinueRequestBody, SessionCreateRequest, SessionEventListQuery,
    SessionEventStreamQuery, SessionExecutionResource, SessionListQuery,
    SessionPermissionReplyRequestBody, SessionReplaceRequest, SessionResource,
    SessionRunOptionsRequest, SessionTurnRequest, SessionUserInputReplyRequestBody,
    WorkspaceFileKind, WorkspaceFileNode, WorkspaceFileTreeQuery, WorkspaceFileTreeResource,
    WorkspaceListQuery, WorkspaceResolveRequest, WorkspaceResource, WorkspaceWriteRequest,
};
pub use error::ApiError;
pub use pagination::{PageInfo, PaginatedResponse};
pub use service::ApiService;

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
        .route(
            "/api/v1/workspaces/{workspace_id}/files",
            get(list_workspace_files),
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
        .route("/plugin-rpc/{plugin_id}", post(plugin_rpc))
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

async fn list_workspace_files(
    State(state): State<ApiState>,
    Path(workspace_id): Path<i64>,
    Query(query): Query<WorkspaceFileTreeQuery>,
) -> Result<Json<WorkspaceFileTreeResource>, ApiError> {
    Ok(Json(
        state
            .service()
            .list_workspace_files(workspace_id, query)
            .await?,
    ))
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
        state
            .service()
            .session_execution_resource(state.session_manager()?.as_ref(), &session)
            .await?,
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
) -> Result<Json<PaginatedResponse<agena::event::DomainEvent>>, ApiError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_session_events(manager.as_ref(), session_id, query)
            .await?,
    ))
}

async fn stream_session_events(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    Query(query): Query<SessionEventStreamQuery>,
) -> Result<Sse<impl futures_core::Stream<Item = Result<Event, Infallible>>>, ApiError> {
    use agena::event::{EventFilter, Scope, bus::SubscriptionItem};

    let manager = state.session_manager()?;
    let service = state.service().clone();

    // Backfill anything older than the live broadcast window before
    // attaching the subscription.
    let backfill_after = query.after_seq.unwrap_or(0);
    let backfill_limit = pagination::normalize_limit(query.limit);
    let initial = service
        .list_session_events_after(
            manager.as_ref(),
            session_id,
            backfill_after,
            Some(backfill_limit),
        )
        .await?;

    let bus = manager.event_bus();
    let mut subscription = bus.subscribe(EventFilter::new(Scope::Session { session_id }));

    let stream = stream! {
        // Replay backfilled events first so subscribers see the full
        // history starting from `after_seq`.
        for event in &initial {
            match Event::default()
                .event("session_event")
                .id(event.meta.seq_global.to_string())
                .json_data(event)
            {
                Ok(ev) => yield Ok(ev),
                Err(error) => {
                    yield Ok(sse_error_event(format!(
                        "failed to encode session event: {error}"
                    )));
                    return;
                }
            }
        }
        let mut last_seen = initial
            .last()
            .map(|e| e.meta.seq_global)
            .unwrap_or(backfill_after);

        // Live subscription. The bus drops slow consumers via
        // `SubscriptionItem::Lagged` — we forward them as comments so the
        // client can choose to reconnect with `after_seq = last_seen`.
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
                        Ok(ev) => yield Ok(ev),
                        Err(error) => {
                            yield Ok(sse_error_event(format!(
                                "failed to encode session event: {error}"
                            )));
                            return;
                        }
                    }
                }
                Some(SubscriptionItem::Lagged(skipped)) => {
                    yield Ok(Event::default()
                        .event("lagged")
                        .data(skipped.to_string()));
                }
                None => return,
            }
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
        state
            .service()
            .session_execution_resource(state.session_manager()?.as_ref(), &session)
            .await?,
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
        state
            .service()
            .session_execution_resource(state.session_manager()?.as_ref(), &session)
            .await?,
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
        state
            .service()
            .session_execution_resource(state.session_manager()?.as_ref(), &session)
            .await?,
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
        state
            .service()
            .session_execution_resource(state.session_manager()?.as_ref(), &session)
            .await?,
    ))
}

async fn list_messages(
    State(state): State<ApiState>,
    Path(session_id): Path<i64>,
    Query(query): Query<MessageListQuery>,
) -> Result<Json<PaginatedResponse<MessageResource>>, ApiError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_messages(manager.as_ref(), session_id, query)
            .await?,
    ))
}

async fn get_message(
    State(state): State<ApiState>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessageDetailQuery>,
) -> Result<Json<MessageResource>, ApiError> {
    let manager = state.session_manager()?;
    state
        .service()
        .get_message(manager.as_ref(), message_id, query.parts)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError::not_found(format!("message not found: {message_id}")))
}

async fn list_message_parts(
    State(state): State<ApiState>,
    Path(message_id): Path<i64>,
    Query(query): Query<MessagePartListQuery>,
) -> Result<Json<Vec<agena::message::MessagePart>>, ApiError> {
    let manager = state.session_manager()?;
    Ok(Json(
        state
            .service()
            .list_message_parts(manager.as_ref(), message_id, query.mode)
            .await?,
    ))
}

async fn get_message_part(
    State(state): State<ApiState>,
    Path(part_id): Path<i64>,
) -> Result<Json<agena::message::MessagePart>, ApiError> {
    let manager = state.session_manager()?;
    state
        .service()
        .get_message_part(manager.as_ref(), part_id)
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

/// Bidirectional callback endpoint for HTTP plugins. Plugins POST a
/// JSON-RPC request here; we route it through the loaded plugin's
/// `HostHandle` so they can call back into agena (log, publish events,
/// ask permissions, invoke other tools).
async fn plugin_rpc(
    State(state): State<ApiState>,
    Path(plugin_id): Path<String>,
    Json(req): Json<agena::plugin::sdk::rpc::Request>,
) -> Json<agena::plugin::sdk::rpc::Response> {
    use agena::plugin::sdk::rpc::{ErrorObject, JsonRpcVersion, Response, ResponsePayload, codes};

    let host = state.runtime().current_snapshot().plugin_manager();
    let id = req.id.clone();

    if host.plugins().iter().all(|p| p.id != plugin_id) {
        return Json(Response {
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

    let handle = host.host_handle();
    let params = req.params.unwrap_or(serde_json::Value::Null);
    match handle.handle_call(&req.method, params).await {
        Ok(result) => Json(Response {
            jsonrpc: JsonRpcVersion,
            id,
            payload: ResponsePayload::Ok { result },
        }),
        Err(err) => Json(Response {
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

async fn resolve_run_options(
    state: &ApiState,
    session_id: i64,
    request: dto::SessionRunOptionsRequest,
) -> Result<agena::session::SessionRunOptions, ApiError> {
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

fn runtime_status_response(state: &ApiState) -> RuntimeStatusResponse {
    let snapshot = state.runtime().current_snapshot();
    let resolution = snapshot.config_resolution();
    let mut provider_ids = snapshot.provider_registry().provider_ids();
    provider_ids.sort();
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
        session_cache,
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
#[allow(dead_code)]
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
    use tokio::sync::Notify;
    use tower::ServiceExt;
    use uuid::Uuid;

    use agena::{
        agent::Agent,
        db::init_schema,
        event::{EventKind, PublishContext},
        message::{
            ApplyPatchToolInput, AskUserToolInput, BashToolInput, BuiltinToolOutput, GlobToolInput,
            GrepToolInput, Message, PartContent, ReadToolInput, TodoItem, TodoPriority, TodoStatus,
            TodoWriteToolInput, ToolExecutionPart, ToolSearchToolInput, UserInputOption,
            UserInputQuestion,
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
    async fn session_events_endpoint_pages_latest_window_in_ascending_order() {
        let (app, state, db) = test_app_with_db().await;
        let _ = db;
        let manager = state.session_manager().expect("session manager available");
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: "/tmp/events-pagination".to_string(),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "events pagination".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

        for seq in 1..=3 {
            manager
                .event_publisher()
                .publish(
                    PublishContext::for_session(session.id),
                    EventKind::PluginEvent(agena::event::PluginEventPayload {
                        plugin_id: "test".into(),
                        kind_label: format!("event_{seq}"),
                        payload: serde_json::json!({}),
                    }),
                )
                .await
                .expect("session event should be appended");
            let _ = seq;
        }

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/sessions/{}/events?limit=2", session.id))
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
        assert_eq!(first_json["page"]["order"], json!("asc"));
        assert_eq!(first_json["page"]["has_more"], json!(true));
        assert_eq!(
            first_json["items"]
                .as_array()
                .expect("items should be an array")
                .iter()
                .map(|item| item["seq_global"].as_i64().expect("event seq should exist"))
                .collect::<Vec<_>>(),
            vec![2, 3]
        );
        let cursor = first_json["page"]["next_cursor"]
            .as_str()
            .expect("next cursor should exist");

        let second = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/sessions/{}/events?limit=2&cursor={cursor}",
                        session.id
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
        assert_eq!(second_json["page"]["order"], json!("asc"));
        assert_eq!(second_json["page"]["has_more"], json!(false));
        assert_eq!(
            second_json["items"]
                .as_array()
                .expect("items should be an array")
                .iter()
                .map(|item| item["seq_global"].as_i64().expect("event seq should exist"))
                .collect::<Vec<_>>(),
            vec![1]
        );
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
    async fn workspace_files_endpoint_lists_workspace_tree() {
        let (app, state) = test_app().await;
        let workspace = TempWorkspace::new();
        let src_dir = workspace.root.join("src");
        fs::create_dir_all(&src_dir).expect("src directory should be created");
        fs::write(src_dir.join("main.rs"), "fn main() {}\n").expect("file should be written");
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: workspace.root.display().to_string(),
            })
            .await
            .expect("workspace should be created");

        let response = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/workspaces/{}/files?depth=2", workspace.id))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let status = response.status();
        let payload = response_json(response).await;

        assert_eq!(status, StatusCode::OK, "unexpected body: {payload}");
        assert_eq!(payload["workspace_id"], json!(workspace.id));
        let entries = payload["entries"]
            .as_array()
            .expect("entries should be returned");
        let src = entries
            .iter()
            .find(|entry| entry["path"] == json!("src"))
            .expect("src directory should be listed");
        assert_eq!(src["kind"], json!("directory"));
        assert!(
            src["children"]
                .as_array()
                .expect("src children should be expanded")
                .iter()
                .any(
                    |entry| entry["path"] == json!("src/main.rs") && entry["kind"] == json!("file")
                )
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
        assert_eq!(before_json["session_cache"]["hits"], json!(0));
        assert_eq!(before_json["session_cache"]["misses"], json!(0));
        assert_eq!(before_json["session_cache"]["inserts"], json!(0));
        assert_eq!(before_json["session_cache"]["entry_count"], json!(0));

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
    async fn session_state_endpoint_records_session_cache_hits_and_misses() {
        let (app, state) = test_app().await;
        let workspace = state
            .service()
            .create_workspace(WorkspaceWriteRequest {
                path: format!("/tmp/cache-state-{}", Uuid::new_v4()),
            })
            .await
            .expect("workspace should be created");
        let session = state
            .service()
            .create_session(SessionCreateRequest {
                workspace_id: workspace.id,
                title: "cache stats".to_string(),
                parent_id: None,
            })
            .await
            .expect("session should be created");

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
        assert_eq!(before.status(), StatusCode::OK);
        let before_json = response_json(before).await;

        for _ in 0..2 {
            let response = app
                .clone()
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
        }

        let after = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/runtime")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        assert_eq!(after.status(), StatusCode::OK);
        let after_json = response_json(after).await;

        let before_cache = &before_json["session_cache"];
        let after_cache = &after_json["session_cache"];
        assert!(
            after_cache["misses"].as_u64().unwrap_or_default()
                >= before_cache["misses"].as_u64().unwrap_or_default() + 1,
            "unexpected before={before_json} after={after_json}"
        );
        assert!(
            after_cache["hits"].as_u64().unwrap_or_default()
                >= before_cache["hits"].as_u64().unwrap_or_default() + 1,
            "unexpected before={before_json} after={after_json}"
        );
        assert!(
            after_cache["inserts"].as_u64().unwrap_or_default()
                >= before_cache["inserts"].as_u64().unwrap_or_default() + 1,
            "unexpected before={before_json} after={after_json}"
        );
        assert!(
            after_cache["entry_count"].as_u64().unwrap_or_default() >= 1,
            "unexpected body: {after_json}"
        );
    }

    #[tokio::test]
    async fn crud_endpoints_roundtrip_for_workspaces_sessions_and_permission_rules() {
        let (app, _state) = test_app().await;
        let workspace_path = format!("/tmp/api-crud-{}", Uuid::new_v4());
        let renamed_workspace_path = format!("{workspace_path}-renamed");

        let created_workspace = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "path": workspace_path.clone() }).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let workspace_status = created_workspace.status();
        let workspace_json = response_json(created_workspace).await;
        assert_eq!(
            workspace_status,
            StatusCode::OK,
            "unexpected body: {workspace_json}"
        );
        let workspace_id = workspace_json["id"]
            .as_i64()
            .expect("workspace id should exist");

        let resolved_workspace = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/workspaces/resolve")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "path": workspace_path.clone(),
                            "create_if_missing": true
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let resolved_status = resolved_workspace.status();
        let resolved_json = response_json(resolved_workspace).await;
        assert_eq!(
            resolved_status,
            StatusCode::OK,
            "unexpected body: {resolved_json}"
        );
        assert_eq!(resolved_json["id"], json!(workspace_id));

        let replaced_workspace = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/v1/workspaces/{workspace_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({ "path": renamed_workspace_path.clone() }).to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let replaced_workspace_status = replaced_workspace.status();
        let replaced_workspace_json = response_json(replaced_workspace).await;
        assert_eq!(
            replaced_workspace_status,
            StatusCode::OK,
            "unexpected body: {replaced_workspace_json}"
        );
        assert_eq!(
            replaced_workspace_json["path"],
            json!(renamed_workspace_path)
        );

        let root_session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "workspace_id": workspace_id,
                            "title": "Root Session"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let root_status = root_session.status();
        let root_json = response_json(root_session).await;
        assert_eq!(root_status, StatusCode::OK, "unexpected body: {root_json}");
        let root_session_id = root_json["id"]
            .as_i64()
            .expect("root session id should exist");

        let child_session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/sessions")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "workspace_id": workspace_id,
                            "title": "Child Session",
                            "parent_id": root_session_id
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let child_status = child_session.status();
        let child_json = response_json(child_session).await;
        assert_eq!(
            child_status,
            StatusCode::OK,
            "unexpected body: {child_json}"
        );
        let child_session_id = child_json["id"]
            .as_i64()
            .expect("child session id should exist");

        let listed_workspaces = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/workspaces?include_session_count=true")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let listed_workspaces_status = listed_workspaces.status();
        let listed_workspaces_json = response_json(listed_workspaces).await;
        assert_eq!(
            listed_workspaces_status,
            StatusCode::OK,
            "unexpected body: {listed_workspaces_json}"
        );
        let workspace_items = listed_workspaces_json["items"]
            .as_array()
            .expect("workspace items should be an array");
        let listed_workspace = workspace_items
            .iter()
            .find(|item| item["id"] == json!(workspace_id))
            .expect("created workspace should be listed");
        assert_eq!(listed_workspace["session_count"], json!(2));

        let listed_sessions = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!(
                        "/api/v1/sessions?workspace_id={workspace_id}&limit=10"
                    ))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let listed_sessions_status = listed_sessions.status();
        let listed_sessions_json = response_json(listed_sessions).await;
        assert_eq!(
            listed_sessions_status,
            StatusCode::OK,
            "unexpected body: {listed_sessions_json}"
        );
        let session_items = listed_sessions_json["items"]
            .as_array()
            .expect("session items should be an array");
        assert_eq!(session_items.len(), 2);

        let replaced_session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/v1/sessions/{child_session_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "title": "Child Session Updated",
                            "parent_id": root_session_id
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let replaced_session_status = replaced_session.status();
        let replaced_session_json = response_json(replaced_session).await;
        assert_eq!(
            replaced_session_status,
            StatusCode::OK,
            "unexpected body: {replaced_session_json}"
        );
        assert_eq!(
            replaced_session_json["title"],
            json!("Child Session Updated")
        );

        let fetched_session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/sessions/{child_session_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let fetched_session_status = fetched_session.status();
        let fetched_session_json = response_json(fetched_session).await;
        assert_eq!(
            fetched_session_status,
            StatusCode::OK,
            "unexpected body: {fetched_session_json}"
        );
        assert_eq!(
            fetched_session_json["title"],
            json!("Child Session Updated")
        );

        let created_rule = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/v1/permission-rules")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "action_key": "tool:bash",
                            "mode": "ask"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let created_rule_status = created_rule.status();
        let created_rule_json = response_json(created_rule).await;
        assert_eq!(
            created_rule_status,
            StatusCode::OK,
            "unexpected body: {created_rule_json}"
        );
        let rule_id = created_rule_json["id"]
            .as_i64()
            .expect("rule id should exist");

        let listed_rules = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/api/v1/permission-rules?search=tool%3Abash")
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let listed_rules_status = listed_rules.status();
        let listed_rules_json = response_json(listed_rules).await;
        assert_eq!(
            listed_rules_status,
            StatusCode::OK,
            "unexpected body: {listed_rules_json}"
        );
        let rule_items = listed_rules_json["items"]
            .as_array()
            .expect("rule items should be an array");
        assert!(
            rule_items
                .iter()
                .any(|item| item["id"] == json!(rule_id) && item["mode"] == json!("ask")),
            "unexpected body: {listed_rules_json}"
        );

        let replaced_rule = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::PUT)
                    .uri(format!("/api/v1/permission-rules/{rule_id}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({
                            "action_key": "tool:bash",
                            "mode": "allow"
                        })
                        .to_string(),
                    ))
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let replaced_rule_status = replaced_rule.status();
        let replaced_rule_json = response_json(replaced_rule).await;
        assert_eq!(
            replaced_rule_status,
            StatusCode::OK,
            "unexpected body: {replaced_rule_json}"
        );
        assert_eq!(replaced_rule_json["mode"], json!("allow"));

        let fetched_rule = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(format!("/api/v1/permission-rules/{rule_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let fetched_rule_status = fetched_rule.status();
        let fetched_rule_json = response_json(fetched_rule).await;
        assert_eq!(
            fetched_rule_status,
            StatusCode::OK,
            "unexpected body: {fetched_rule_json}"
        );
        assert_eq!(fetched_rule_json["mode"], json!("allow"));

        let deleted_rule = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v1/permission-rules/{rule_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let deleted_rule_status = deleted_rule.status();
        let deleted_rule_json = response_json(deleted_rule).await;
        assert_eq!(
            deleted_rule_status,
            StatusCode::OK,
            "unexpected body: {deleted_rule_json}"
        );
        assert_eq!(deleted_rule_json["id"], json!(rule_id));

        let deleted_child = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v1/sessions/{child_session_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let deleted_child_status = deleted_child.status();
        let deleted_child_json = response_json(deleted_child).await;
        assert_eq!(
            deleted_child_status,
            StatusCode::OK,
            "unexpected body: {deleted_child_json}"
        );
        assert_eq!(deleted_child_json["id"], json!(child_session_id));

        let deleted_root = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v1/sessions/{root_session_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let deleted_root_status = deleted_root.status();
        let deleted_root_json = response_json(deleted_root).await;
        assert_eq!(
            deleted_root_status,
            StatusCode::OK,
            "unexpected body: {deleted_root_json}"
        );
        assert_eq!(deleted_root_json["id"], json!(root_session_id));

        let deleted_workspace = app
            .oneshot(
                Request::builder()
                    .method(Method::DELETE)
                    .uri(format!("/api/v1/workspaces/{workspace_id}"))
                    .body(Body::empty())
                    .expect("request should build"),
            )
            .await
            .expect("request should succeed");
        let deleted_workspace_status = deleted_workspace.status();
        let deleted_workspace_json = response_json(deleted_workspace).await;
        assert_eq!(
            deleted_workspace_status,
            StatusCode::OK,
            "unexpected body: {deleted_workspace_json}"
        );
        assert_eq!(deleted_workspace_json["id"], json!(workspace_id));
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

    async fn test_app_with_db() -> (Router, ApiState, Arc<DatabaseConnection>) {
        let db = Arc::new(
            Database::connect("sqlite::memory:")
                .await
                .expect("database should connect"),
        );
        init_schema(db.as_ref())
            .await
            .expect("schema should initialize");
        let runtime = test_runtime(db.clone()).await;
        let state = ApiState::new(runtime, db.clone());
        let app = router(state.clone());
        (app, state, db)
    }

    async fn test_app_with_scripted_manager() -> (Router, ApiState, TempWorkspace) {
        test_app_with_provider(ScriptedApiProvider).await
    }

    async fn test_app_with_provider<P>(provider: P) -> (Router, ApiState, TempWorkspace)
    where
        P: ModelProvider + 'static,
    {
        test_app_with_provider_and_agent(
            provider,
            Agent::new("api-test", PermissionPolicy::allow_all()),
        )
        .await
    }

    async fn test_app_with_provider_and_agent<P>(
        provider: P,
        agent: Agent,
    ) -> (Router, ApiState, TempWorkspace)
    where
        P: ModelProvider + 'static,
    {
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
        let manager = session_manager_with_provider_and_agent(
            db.clone(),
            workspace.root.as_path(),
            provider,
            agent,
        )
        .await;
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

    #[derive(Clone)]
    struct BlockingApiProvider {
        first_delta_sent: Arc<Notify>,
        release_completion: Arc<Notify>,
    }

    struct ToolSuiteApiProvider;

    struct PermissionPatchApiProvider;

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
                reasoning_text: None,
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
                            details,
                            ..
                        })) => match details.as_builtin() {
                            Some(BuiltinToolOutput::AskUser { answers }) => answers
                                .get("model_choice")
                                .and_then(|values| values.first().cloned())
                                .map(Ok),
                            _ => None,
                        },
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

    #[async_trait]
    impl ModelProvider for BlockingApiProvider {
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
                reasoning_text: None,
            })
        }

        async fn complete_stream(
            &self,
            _request: CompletionRequest,
        ) -> Result<
            Pin<Box<dyn Stream<Item = Result<CompletionStreamEvent, AppError>> + Send>>,
            AppError,
        > {
            let first_delta_sent = Arc::clone(&self.first_delta_sent);
            let release_completion = Arc::clone(&self.release_completion);

            Ok(Box::pin(async_stream::stream! {
                yield Ok(CompletionStreamEvent::TextDelta {
                    provider_id: agena::model::ProviderId::new("openai"),
                    model: ModelId::new("gpt-4.1-mini"),
                    delta: "Hel".to_string(),
                });
                first_delta_sent.notify_one();
                release_completion.notified().await;
                yield Ok(CompletionStreamEvent::TextDelta {
                    provider_id: agena::model::ProviderId::new("openai"),
                    model: ModelId::new("gpt-4.1-mini"),
                    delta: "lo".to_string(),
                });
                yield Ok(CompletionStreamEvent::Completed {
                    provider_id: agena::model::ProviderId::new("openai"),
                    model: ModelId::new("gpt-4.1-mini"),
                    finish_reason: Some(CompletionFinishReason::Stop),
                    usage: None,
                    provider_metadata: None,
                });
            }))
        }
    }

    #[async_trait]
    impl ModelProvider for ToolSuiteApiProvider {
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
                reasoning_text: None,
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
            let is_tool_suite = last_user_text.contains("tool suite");
            let apply_patch_loaded =
                request_loaded_deferred_tool(request.messages.as_slice(), "apply_patch");
            let bash_loaded = request_loaded_deferred_tool(request.messages.as_slice(), "bash");

            let events = if is_tool_suite && (!apply_patch_loaded || !bash_loaded) {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_tool_search_1".to_string(),
                        id: Some("call_tool_search_1".to_string()),
                        name: Some("tool_search".to_string()),
                        arguments_delta: serde_json::to_string(&ToolSearchToolInput {
                            query: "load deferred write tools".to_string(),
                            load: vec!["bash".to_string(), "apply_patch".to_string()],
                            limit: None,
                        })
                        .expect("tool_search input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_glob_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_glob_1".to_string(),
                        id: Some("call_glob_1".to_string()),
                        name: Some("glob".to_string()),
                        arguments_delta: serde_json::to_string(&GlobToolInput {
                            pattern: "**/*.md".to_string(),
                            path: None,
                        })
                        .expect("glob input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_grep_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_grep_1".to_string(),
                        id: Some("call_grep_1".to_string()),
                        name: Some("grep".to_string()),
                        arguments_delta: serde_json::to_string(&GrepToolInput {
                            pattern: "cache marker".to_string(),
                            path: None,
                            include: Some("**/*.md".to_string()),
                        })
                        .expect("grep input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_read_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_read_1".to_string(),
                        id: Some("call_read_1".to_string()),
                        name: Some("read".to_string()),
                        arguments_delta: serde_json::to_string(&ReadToolInput {
                            file_path: "README.md".to_string(),
                            offset: None,
                            limit: None,
                        })
                        .expect("read input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_todo_write_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_todo_write_1".to_string(),
                        id: Some("call_todo_write_1".to_string()),
                        name: Some("todo_write".to_string()),
                        arguments_delta: serde_json::to_string(&TodoWriteToolInput {
                            items: vec![
                                TodoItem {
                                    content: "Inspect workspace".to_string(),
                                    status: TodoStatus::Completed,
                                    priority: TodoPriority::High,
                                },
                                TodoItem {
                                    content: "Patch output files".to_string(),
                                    status: TodoStatus::InProgress,
                                    priority: TodoPriority::Medium,
                                },
                            ],
                        })
                        .expect("todo_write input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_bash_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_bash_1".to_string(),
                        id: Some("call_bash_1".to_string()),
                        name: Some("bash".to_string()),
                        arguments_delta: serde_json::to_string(&BashToolInput {
                            command: "printf 'bash-created\\n' > bash_output.txt".to_string(),
                            description: "Create a file via bash.".to_string(),
                            timeout_ms: None,
                            workdir: None,
                        })
                        .expect("bash input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite
                && request_tool_result(request.messages.as_slice(), "call_apply_patch_1").is_none()
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_apply_patch_1".to_string(),
                        id: Some("call_apply_patch_1".to_string()),
                        name: Some("apply_patch".to_string()),
                        arguments_delta: serde_json::to_string(&ApplyPatchToolInput {
                            patch: "*** Begin Patch\n*** Add File: patched.txt\n+patched\n*** End Patch"
                                .to_string(),
                        })
                        .expect("apply_patch input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if is_tool_suite {
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        delta: "tool suite completed".to_string(),
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

    #[async_trait]
    impl ModelProvider for PermissionPatchApiProvider {
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
                reasoning_text: None,
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
            let apply_patch_loaded =
                request_loaded_deferred_tool(request.messages.as_slice(), "apply_patch");
            let apply_patch_result =
                request_tool_result(request.messages.as_slice(), "call_apply_patch_1");

            let events = if last_user_text.contains("patch")
                && apply_patch_result.is_none()
                && !apply_patch_loaded
            {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_tool_search_1".to_string(),
                        id: Some("call_tool_search_1".to_string()),
                        name: Some("tool_search".to_string()),
                        arguments_delta: serde_json::to_string(&ToolSearchToolInput {
                            query: "patch file".to_string(),
                            load: vec!["apply_patch".to_string()],
                            limit: None,
                        })
                        .expect("tool_search input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if last_user_text.contains("patch") && apply_patch_result.is_none() {
                vec![
                    Ok(CompletionStreamEvent::ToolCallDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        stream_key: "call_apply_patch_1".to_string(),
                        id: Some("call_apply_patch_1".to_string()),
                        name: Some("apply_patch".to_string()),
                        arguments_delta: serde_json::to_string(&ApplyPatchToolInput {
                            patch: "*** Begin Patch\n*** Add File: result.txt\n+approved\n*** End Patch"
                                .to_string(),
                        })
                        .expect("apply_patch input should serialize"),
                    }),
                    Ok(CompletionStreamEvent::Completed {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        finish_reason: Some(CompletionFinishReason::ToolCalls),
                        usage: None,
                        provider_metadata: None,
                    }),
                ]
            } else if let Some(result) = apply_patch_result {
                let delta = if result.is_ok() {
                    "patch done"
                } else {
                    "patch denied"
                };
                vec![
                    Ok(CompletionStreamEvent::TextDelta {
                        provider_id: agena::model::ProviderId::new("openai"),
                        model: ModelId::new("gpt-4.1-mini"),
                        delta: delta.to_string(),
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

    async fn session_manager_with_provider_and_agent<P>(
        db: Arc<DatabaseConnection>,
        workspace_root: &Path,
        provider: P,
        agent: Agent,
    ) -> Arc<SessionManager>
    where
        P: ModelProvider + 'static,
    {
        let mut registry = ProviderRegistry::new();
        registry.register(provider);
        let processor = SessionProcessor::new(
            Arc::new(registry),
            ContextGovernor::new(ContextPolicy::default()),
        );
        let executor = ToolExecutor::new(workspace_root.to_path_buf(), agent);
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
store_backend = "file"

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

    fn request_loaded_deferred_tool(messages: &[Message], tool_name: &str) -> bool {
        messages.iter().any(|message| {
            message.parts.iter().any(|part| {
                let details = match part.content.as_ref() {
                    Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                        details,
                        ..
                    })) => details,
                    _ => return false,
                };
                matches!(
                    details.as_builtin(),
                    Some(BuiltinToolOutput::ToolSearch { ref loaded_tools, .. })
                        if loaded_tools.iter().any(|loaded| loaded == tool_name)
                )
            })
        })
    }

    fn request_tool_result(
        messages: &[Message],
        operation_id: &str,
    ) -> Option<Result<BuiltinToolOutput, String>> {
        messages.iter().find_map(|message| {
            if message.role != Role::Tool {
                return None;
            }
            message.parts.iter().find_map(|part| {
                if part.operation_id.as_deref() != Some(operation_id) {
                    return None;
                }
                match part.content.as_ref() {
                    Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                        details,
                        ..
                    })) => details.as_builtin().map(Ok),
                    Some(PartContent::ToolExecution(ToolExecutionPart::Failed {
                        error_message,
                        ..
                    })) => Some(Err(error_message.clone())),
                    _ => None,
                }
            })
        })
    }

    fn completed_builtin_output(
        messages: &[MessageResource],
        operation_id: &str,
    ) -> Option<BuiltinToolOutput> {
        messages.iter().find_map(|message| {
            message.parts.as_ref().and_then(|parts| {
                parts.iter().find_map(|part| {
                    if part.operation_id.as_deref() != Some(operation_id) {
                        return None;
                    }
                    match part.content.as_ref() {
                        Some(PartContent::ToolExecution(ToolExecutionPart::Completed {
                            details,
                            ..
                        })) => details.as_builtin(),
                        _ => None,
                    }
                })
            })
        })
    }

    fn assistant_text(messages: &[MessageResource]) -> Option<String> {
        messages.iter().rev().find_map(|message| {
            if message.role != Role::Assistant {
                return None;
            }
            message.parts.as_ref().and_then(|parts| {
                parts.iter().find_map(|part| match part.content.as_ref() {
                    Some(PartContent::Text(text)) => Some(text.text.clone()),
                    _ => None,
                })
            })
        })
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
