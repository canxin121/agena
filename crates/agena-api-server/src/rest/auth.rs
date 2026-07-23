pub async fn list_auth_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let items = state.auth_providers().map_err(ServerError::from)?;
    Ok(Json(items))
}

pub async fn get_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    auth_provider_json_from_state(&state, provider_id.as_str())
}

pub async fn set_auth_provider_api_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<AuthApiKeyWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let provider = state
        .set_auth_api_key(provider_id.as_str(), request.api_key)
        .await
        .map_err(ServerError::from)?;
    Ok(Json(provider))
}

pub async fn start_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthRedirectRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    browser_start_response(
        &state,
        request.provider.normalized_provider_id("openai"),
        AuthLoginKind::OpenaiChatgpt,
        request.redirect_uri,
    )
    .await
}

pub async fn finish_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCodeExchangeRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    browser_finish_response(
        &state,
        request.provider.normalized_provider_id("openai"),
        AuthLoginKind::OpenaiChatgpt,
        request.code,
        request.pkce_verifier,
        request.redirect_uri,
    )
    .await
}

pub async fn start_openai_device_auth(
    State(state): State<AppState>,
    payload: Option<Json<AuthProviderRequest>>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let request = payload.map(|Json(request)| request).unwrap_or_default();
    device_start_response(
        &state,
        request.normalized_provider_id("openai"),
        AuthLoginKind::OpenaiChatgpt,
        None,
    )
    .await
}

pub async fn poll_openai_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthUserCodeDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    device_poll_response(
        &state,
        request.provider.normalized_provider_id("openai"),
        AuthLoginKind::OpenaiChatgpt,
        request.device_code,
        Some(request.user_code),
        None,
    )
    .await
}

pub async fn start_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthRedirectRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    browser_start_response(
        &state,
        request.provider.normalized_provider_id("gitlab"),
        AuthLoginKind::Gitlab,
        request.redirect_uri,
    )
    .await
}

pub async fn finish_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCodeExchangeRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    browser_finish_response(
        &state,
        request.provider.normalized_provider_id("gitlab"),
        AuthLoginKind::Gitlab,
        request.code,
        request.pkce_verifier,
        request.redirect_uri,
    )
    .await
}

pub async fn start_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthEnterpriseDeviceRequest>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let provider_id = request.provider.normalized_provider_id("github-copilot");
    device_start_response(
        &state,
        provider_id,
        AuthLoginKind::GithubCopilot,
        request.enterprise_domain,
    )
    .await
}

pub async fn poll_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthEnterpriseDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id = request.provider.normalized_provider_id("github-copilot");
    device_poll_response(
        &state,
        provider_id,
        AuthLoginKind::GithubCopilot,
        request.device_code,
        None,
        request.enterprise_domain,
    )
    .await
}

pub async fn delete_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let provider = state
        .remove_auth_provider(provider_id.as_str())
        .await
        .map_err(ServerError::from)?;
    Ok(Json(provider))
}

pub async fn refresh_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let provider = state
        .refresh_auth_provider(provider_id.as_str())
        .await
        .map_err(ServerError::from)?;
    Ok(Json(provider))
}

async fn browser_start_response(
    state: &AppState,
    provider_id: String,
    kind: AuthLoginKind,
    redirect_uri: String,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    Ok(Json(
        state
            .start_auth_browser(provider_id, kind, redirect_uri)
            .await
            .map_err(ServerError::from)?,
    ))
}

async fn browser_finish_response(
    state: &AppState,
    provider_id: String,
    kind: AuthLoginKind,
    code: String,
    pkce_verifier: String,
    redirect_uri: String,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    Ok(Json(
        state
            .finish_auth_browser(
                provider_id.as_str(),
                kind,
                code,
                pkce_verifier,
                redirect_uri,
            )
            .await
            .map_err(ServerError::from)?,
    ))
}

async fn device_start_response(
    state: &AppState,
    provider_id: String,
    kind: AuthLoginKind,
    enterprise_domain: Option<String>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    Ok(Json(
        state
            .start_auth_device(provider_id, kind, enterprise_domain)
            .await
            .map_err(ServerError::from)?,
    ))
}

async fn device_poll_response(
    state: &AppState,
    provider_id: String,
    kind: AuthLoginKind,
    device_code: String,
    user_code: Option<String>,
    enterprise_domain: Option<String>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    Ok(Json(
        state
            .poll_auth_device(
                provider_id.as_str(),
                kind,
                device_code,
                user_code,
                enterprise_domain,
            )
            .await
            .map_err(ServerError::from)?,
    ))
}

fn auth_provider_json_from_state(
    state: &AppState,
    provider_id: &str,
) -> Result<Json<AuthProviderResource>, ServerError> {
    state
        .auth_provider(provider_id)
        .map(Json)
        .map_err(ServerError::from)
}

use super::{
    AppState, AuthApiKeyWriteRequest, AuthBrowserStartResource, AuthCodeExchangeRequest,
    AuthDeviceStartResource, AuthEnterpriseDevicePollRequest, AuthEnterpriseDeviceRequest,
    AuthLoginResultResource, AuthProviderRequest, AuthProviderResource, AuthRedirectRequest,
    AuthUserCodeDevicePollRequest, IntoResponse, Json, Path, ServerError, State,
};
use agena_application::AuthLoginKind;
