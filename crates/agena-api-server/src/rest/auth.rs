use super::*;

pub async fn list_auth_providers(
    State(state): State<AppState>,
) -> Result<impl IntoResponse, ServerError> {
    let configured_ids = configured_provider_auth_ids(&state);
    let items = configured_ids
        .into_iter()
        .map(|provider_id| {
            let auth = current_auth_provider_data(&state, provider_id.as_str())?;
            auth_provider_resource(true, provider_id, auth.as_ref())
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Json(items))
}

pub async fn get_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    auth_provider_config(&state, provider_id.as_str())?;
    auth_provider_json_from_state(&state, provider_id.as_str())
}

pub async fn set_auth_provider_api_key(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Json(request): Json<AuthApiKeyWriteRequest>,
) -> Result<impl IntoResponse, ServerError> {
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    if !provider_supports_api_key_write(&resolved) {
        return Err(ServerError::BadRequest(format!(
            "{provider_id} does not support api key login"
        )));
    }

    auth_manager(&state)
        .set_api_key(provider_id.as_str(), request.api_key)
        .map_err(ServerError::Core)?;
    reload_auth_provider_json_from_state(&state, provider_id.as_str()).await
}

pub async fn start_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthRedirectRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request.provider, "openai")?;
    target.require_openai_browser(provider_id.as_str())?;
    let start = auth_manager(&state)
        .start_openai_browser_login(request.redirect_uri)
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource::from_start(
        provider_id,
        None,
        start,
    )))
}

pub async fn finish_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCodeExchangeRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request.provider, "openai")?;
    target.require_openai_browser(provider_id.as_str())?;
    let manager = auth_manager(&state);
    finish_auth_login(
        &state,
        provider_id.as_str(),
        manager.finish_openai_browser_login(
            provider_id.as_str(),
            request.code,
            request.pkce_verifier,
            request.redirect_uri,
        ),
    )
    .await
}

pub async fn start_openai_device_auth(
    State(state): State<AppState>,
    payload: Option<Json<AuthProviderRequest>>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let request = payload.map(|Json(request)| request).unwrap_or_default();
    let (provider_id, target) = resolve_device_auth_request(&state, &request, "openai")?;
    target.require_openai_device(provider_id.as_str())?;
    let start = auth_manager(&state)
        .start_openai_headless_login()
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthDeviceStartResource::from_start(
        provider_id,
        None,
        start,
    )))
}

pub async fn poll_openai_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthUserCodeDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let (provider_id, target) = resolve_device_auth_request(&state, &request.provider, "openai")?;
    target.require_openai_device(provider_id.as_str())?;
    let manager = auth_manager(&state);
    poll_auth_login(
        &state,
        provider_id.as_str(),
        manager.poll_openai_headless_login(
            provider_id.as_str(),
            request.device_code,
            request.user_code,
        ),
    )
    .await
}

pub async fn start_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthRedirectRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request.provider, "gitlab")?;
    let instance_url = target.require_gitlab_browser(provider_id.as_str())?;
    let start = auth_manager(&state)
        .start_gitlab_login(instance_url.clone(), request.redirect_uri)
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource::from_start(
        provider_id,
        Some(instance_url),
        start,
    )))
}

pub async fn finish_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCodeExchangeRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request.provider, "gitlab")?;
    let instance_url = target.require_gitlab_browser(provider_id.as_str())?;
    let manager = auth_manager(&state);
    finish_auth_login(
        &state,
        provider_id.as_str(),
        manager.finish_gitlab_login(
            provider_id.as_str(),
            instance_url,
            request.code,
            request.pkce_verifier,
            request.redirect_uri,
        ),
    )
    .await
}

pub async fn start_atomgit_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthProviderRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request, "atomgit")?;
    target.require_atomgit_browser(provider_id.as_str())?;
    let start = auth_manager(&state)
        .start_atomgit_login()
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource::from_start(
        provider_id,
        None,
        start,
    )))
}

pub async fn poll_atomgit_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthStatePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let (provider_id, target) = resolve_browser_auth_request(&state, &request.provider, "atomgit")?;
    target.require_atomgit_browser(provider_id.as_str())?;
    let manager = auth_manager(&state);
    poll_auth_login(
        &state,
        provider_id.as_str(),
        manager.poll_atomgit_login(provider_id.as_str(), request.state),
    )
    .await
}

pub async fn start_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthEnterpriseDeviceRequest>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let (provider_id, target) =
        resolve_device_auth_request(&state, &request.provider, "github-copilot")?;
    target.require_copilot_device(provider_id.as_str())?;
    let deployment = copilot_deployment(request.enterprise_domain.as_deref());
    let start = auth_manager(&state)
        .start_copilot_login(deployment)
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthDeviceStartResource::from_start(
        provider_id,
        request.enterprise_domain,
        start,
    )))
}

pub async fn poll_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthEnterpriseDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let (provider_id, target) =
        resolve_device_auth_request(&state, &request.provider, "github-copilot")?;
    target.require_copilot_device(provider_id.as_str())?;
    let deployment = copilot_deployment(request.enterprise_domain.as_deref());
    let manager = auth_manager(&state);
    poll_auth_login(
        &state,
        provider_id.as_str(),
        manager.poll_copilot_login(provider_id.as_str(), request.device_code, deployment),
    )
    .await
}

pub async fn delete_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    auth_provider_config(&state, provider_id.as_str())?;

    auth_manager(&state)
        .remove(provider_id.as_str())
        .map_err(ServerError::Core)?;
    reload_auth_provider_json_from_state(&state, provider_id.as_str()).await
}

pub async fn refresh_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_oauth_auth_target(
        provider_id.as_str(),
        &resolved,
        OAuthAuthPurpose::CredentialRefresh,
    )? {
        ProviderOAuthTarget::OpenAi => {
            auth_manager(&state)
                .refresh_openai_login(provider_id.as_str())
                .await
                .map_err(ServerError::Core)?;
        }
        ProviderOAuthTarget::Gitlab { instance_url } => {
            auth_manager(&state)
                .refresh_gitlab_login(provider_id.as_str(), instance_url)
                .await
                .map_err(ServerError::Core)?;
        }
        ProviderOAuthTarget::AtomGit => {
            auth_manager(&state)
                .refresh_atomgit_login(provider_id.as_str())
                .await
                .map_err(ServerError::Core)?;
        }
    }

    reload_auth_provider_json_from_state(&state, provider_id.as_str()).await
}

fn configured_provider_auth_ids(state: &AppState) -> BTreeSet<String> {
    let snapshot = state.runtime().current_snapshot();
    snapshot
        .config_resolution()
        .config
        .providers
        .iter()
        .filter_map(|(provider_id, resolved)| configured_provider_auth_id(provider_id, resolved))
        .collect()
}

fn configured_provider_auth_id(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Option<String> {
    match &resolved.auth {
        ProviderAuthConfig::None | ProviderAuthConfig::BedrockSigv4(_) => None,
        ProviderAuthConfig::Api(_) => Some(provider_id.to_owned()),
        ProviderAuthConfig::Gitlab(_) => Some(provider_id.to_owned()),
        ProviderAuthConfig::Credential(config)
            if matches!(
                config.issuer,
                agena::provider::auth::CredentialIssuer::GoogleAdc
                    | agena::provider::auth::CredentialIssuer::SapAiCore
            ) =>
        {
            None
        }
        ProviderAuthConfig::Credential(_) => Some(provider_id.to_owned()),
    }
}

fn auth_manager(state: &AppState) -> AuthManager<ProviderConfigCredentialStore> {
    let resolution = state.runtime().config_resolution();
    AuthManager::new(ProviderConfigCredentialStore::new(
        resolution.meta.config_path.clone(),
    ))
}

fn current_auth_provider_data(
    state: &AppState,
    provider_id: &str,
) -> Result<Option<agena::provider::auth::AuthData>, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let resolved = snapshot
        .config_resolution()
        .config
        .providers
        .get(provider_id)
        .ok_or_else(|| ServerError::NotFound(format!("auth provider not found: {provider_id}")))?;
    Ok(provider_auth_data(resolved))
}

fn auth_provider_resource_from_state(
    state: &AppState,
    provider_id: &str,
) -> Result<AuthProviderResource, ServerError> {
    let auth = current_auth_provider_data(state, provider_id)?;
    auth_provider_resource(true, provider_id.to_owned(), auth.as_ref())
}

fn auth_provider_json_from_state(
    state: &AppState,
    provider_id: &str,
) -> Result<Json<AuthProviderResource>, ServerError> {
    Ok(Json(auth_provider_resource_from_state(state, provider_id)?))
}

async fn reload_auth_provider_json_from_state(
    state: &AppState,
    provider_id: &str,
) -> Result<Json<AuthProviderResource>, ServerError> {
    reload_runtime_from_config(state).await?;
    auth_provider_json_from_state(state, provider_id)
}

fn auth_provider_config(
    state: &AppState,
    provider_id: &str,
) -> Result<ResolvedProviderConfig, ServerError> {
    let snapshot = state.runtime().current_snapshot();
    let resolved = snapshot
        .config_resolution()
        .config
        .providers
        .get(provider_id)
        .cloned()
        .ok_or_else(|| ServerError::NotFound(format!("auth provider not found: {provider_id}")))?;
    if configured_provider_auth_id(provider_id, &resolved).is_none() {
        return Err(ServerError::NotFound(format!(
            "auth provider not found: {provider_id}"
        )));
    }
    Ok(resolved)
}

async fn auth_login_result_response(
    state: &AppState,
    provider_id: &str,
    completed: bool,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    if completed {
        reload_runtime_from_config(state).await?;
    }
    let provider = completed
        .then(|| auth_provider_resource_from_state(state, provider_id))
        .transpose()?;
    Ok(Json(AuthLoginResultResource {
        completed,
        provider,
    }))
}

async fn finish_auth_login<T>(
    state: &AppState,
    provider_id: &str,
    future: impl Future<Output = Result<T, agena::AppError>>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    future.await.map_err(ServerError::Core)?;
    auth_login_result_response(state, provider_id, true).await
}

async fn poll_auth_login<T>(
    state: &AppState,
    provider_id: &str,
    future: impl Future<Output = Result<Option<T>, agena::AppError>>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let auth = future.await.map_err(ServerError::Core)?;
    auth_login_result_response(state, provider_id, auth.is_some()).await
}

fn resolve_browser_auth_request(
    state: &AppState,
    request: &AuthProviderRequest,
    default_provider_id: &str,
) -> Result<(String, ProviderOAuthTarget), ServerError> {
    let provider_id = request.normalized_provider_id(default_provider_id);
    let resolved = auth_provider_config(state, provider_id.as_str())?;
    let target = resolve_oauth_auth_target(
        provider_id.as_str(),
        &resolved,
        OAuthAuthPurpose::BrowserLogin,
    )?;
    Ok((provider_id, target))
}

fn resolve_device_auth_request(
    state: &AppState,
    request: &AuthProviderRequest,
    default_provider_id: &str,
) -> Result<(String, ProviderDeviceAuthTarget), ServerError> {
    let provider_id = request.normalized_provider_id(default_provider_id);
    let resolved = auth_provider_config(state, provider_id.as_str())?;
    let target = resolve_device_auth_target(provider_id.as_str(), &resolved)?;
    Ok((provider_id, target))
}

fn normalize_optional_text(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn copilot_deployment(enterprise_domain: Option<&str>) -> CopilotDeployment {
    match normalize_optional_text(enterprise_domain) {
        Some(domain) => CopilotDeployment::Enterprise { domain },
        None => CopilotDeployment::GitHubCom,
    }
}

trait ProviderOAuthTargetExt {
    fn require_openai_browser(self, provider_id: &str) -> Result<(), ServerError>;
    fn require_gitlab_browser(self, provider_id: &str) -> Result<String, ServerError>;
    fn require_atomgit_browser(self, provider_id: &str) -> Result<(), ServerError>;
}

impl ProviderOAuthTargetExt for ProviderOAuthTarget {
    fn require_openai_browser(self, provider_id: &str) -> Result<(), ServerError> {
        match self {
            Self::OpenAi => Ok(()),
            Self::Gitlab { .. } | Self::AtomGit => Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai browser login"
            ))),
        }
    }

    fn require_gitlab_browser(self, provider_id: &str) -> Result<String, ServerError> {
        match self {
            Self::Gitlab { instance_url } => Ok(instance_url),
            Self::OpenAi | Self::AtomGit => Err(ServerError::BadRequest(format!(
                "{provider_id} does not support gitlab browser login"
            ))),
        }
    }

    fn require_atomgit_browser(self, provider_id: &str) -> Result<(), ServerError> {
        match self {
            Self::AtomGit => Ok(()),
            Self::OpenAi | Self::Gitlab { .. } => Err(ServerError::BadRequest(format!(
                "{provider_id} does not support atomgit browser login"
            ))),
        }
    }
}

#[derive(Clone, Copy)]
enum OAuthAuthPurpose {
    BrowserLogin,
    CredentialRefresh,
}

impl OAuthAuthPurpose {
    fn unsupported_error(self, provider_id: &str) -> ServerError {
        match self {
            Self::BrowserLogin => {
                ServerError::BadRequest(format!("{provider_id} does not support browser login"))
            }
            Self::CredentialRefresh => ServerError::BadRequest(format!(
                "credential refresh is not supported for provider '{provider_id}'"
            )),
        }
    }

    fn ambiguous_provider_error(self, provider_id: &str) -> ServerError {
        match self {
            Self::BrowserLogin => ServerError::BadRequest(format!(
                "{provider_id} has ambiguous browser auth providers"
            )),
            Self::CredentialRefresh => ServerError::BadRequest(format!(
                "{provider_id} has ambiguous credential refresh handlers"
            )),
        }
    }

    fn ambiguous_gitlab_error(self, provider_id: &str) -> ServerError {
        match self {
            Self::BrowserLogin => ServerError::BadRequest(format!(
                "{provider_id} has ambiguous gitlab browser auth adapters"
            )),
            Self::CredentialRefresh => ServerError::BadRequest(format!(
                "{provider_id} has ambiguous gitlab refresh adapters"
            )),
        }
    }
}

trait ProviderDeviceAuthTargetExt {
    fn require_openai_device(self, provider_id: &str) -> Result<(), ServerError>;
    fn require_copilot_device(self, provider_id: &str) -> Result<(), ServerError>;
}

impl ProviderDeviceAuthTargetExt for ProviderDeviceAuthTarget {
    fn require_openai_device(self, provider_id: &str) -> Result<(), ServerError> {
        match self {
            Self::OpenAi => Ok(()),
            Self::Copilot => Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai device login"
            ))),
        }
    }

    fn require_copilot_device(self, provider_id: &str) -> Result<(), ServerError> {
        match self {
            Self::Copilot => Ok(()),
            Self::OpenAi => Err(ServerError::BadRequest(format!(
                "{provider_id} does not support copilot device login"
            ))),
        }
    }
}

fn resolve_oauth_auth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
    purpose: OAuthAuthPurpose,
) -> Result<ProviderOAuthTarget, ServerError> {
    match resolve_provider_oauth_target(resolved) {
        Ok(Some(target)) => Ok(target),
        Ok(None) => Err(purpose.unsupported_error(provider_id)),
        Err(ProviderAuthTargetError::AmbiguousProvider) => {
            Err(purpose.ambiguous_provider_error(provider_id))
        }
        Err(ProviderAuthTargetError::AmbiguousGitlab) => {
            Err(purpose.ambiguous_gitlab_error(provider_id))
        }
    }
}

fn resolve_device_auth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<ProviderDeviceAuthTarget, ServerError> {
    match resolve_provider_device_auth_target(resolved) {
        Ok(Some(target)) => Ok(target),
        Ok(None) => Err(ServerError::BadRequest(format!(
            "{provider_id} does not support device login"
        ))),
        Err(ProviderAuthTargetError::AmbiguousProvider) => Err(ServerError::BadRequest(format!(
            "{provider_id} has ambiguous device auth providers"
        ))),
        Err(ProviderAuthTargetError::AmbiguousGitlab) => {
            unreachable!("gitlab ambiguity is not possible for device auth targets")
        }
    }
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
        username: None,
        display_name: None,
        email: None,
        avatar_url: None,
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
            user,
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
            resource.enterprise_url = enterprise_url.clone();
            if let Some(user) = user {
                resource.account_id = account_id.clone().or_else(|| Some(user.id.clone()));
                resource.username = Some(user.username.clone());
                resource.display_name = user.name.clone();
                resource.email = user.email.clone();
                resource.avatar_url = user.avatar_url.clone();
            } else {
                resource.account_id = account_id.clone();
            }
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
