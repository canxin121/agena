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
    Ok(Json(auth_provider_resource_from_state(
        &state,
        provider_id.as_str(),
    )?))
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
    reload_runtime_from_config(&state).await?;
    Ok(Json(auth_provider_resource_from_state(
        &state,
        provider_id.as_str(),
    )?))
}

pub async fn start_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthBrowserStartRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "openai");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::OpenAi => {}
        BrowserAuthTarget::Gitlab { .. } => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai browser login"
            )));
        }
        BrowserAuthTarget::AtomGit => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai browser login"
            )));
        }
    }

    let start = auth_manager(&state)
        .start_openai_browser_login(request.redirect_uri)
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource {
        provider_id,
        instance_url: None,
        authorize_url: start.authorize_url,
        state: start.state,
        pkce_verifier: start.pkce_verifier,
    }))
}

pub async fn finish_openai_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthOpenAiBrowserFinishRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "openai");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::OpenAi => {}
        BrowserAuthTarget::Gitlab { .. } => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai browser login"
            )));
        }
        BrowserAuthTarget::AtomGit => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai browser login"
            )));
        }
    }

    auth_manager(&state)
        .finish_openai_browser_login(
            provider_id.as_str(),
            request.code,
            request.pkce_verifier,
            request.redirect_uri,
        )
        .await
        .map_err(ServerError::Core)?;
    reload_runtime_from_config(&state).await?;
    Ok(Json(AuthLoginResultResource {
        completed: true,
        provider: Some(auth_provider_resource_from_state(
            &state,
            provider_id.as_str(),
        )?),
    }))
}

pub async fn start_openai_device_auth(
    State(state): State<AppState>,
    payload: Option<Json<AuthOpenAiDeviceStartRequest>>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let request = payload.map(|Json(request)| request).unwrap_or_default();
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "openai");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_device_auth_target(provider_id.as_str(), &resolved)? {
        DeviceAuthTarget::OpenAi => {}
        DeviceAuthTarget::Copilot => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai device login"
            )));
        }
    }

    let start = auth_manager(&state)
        .start_openai_headless_login()
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthDeviceStartResource {
        provider_id,
        enterprise_domain: None,
        verification_url: start.verification_url,
        user_code: start.user_code,
        device_code: start.device_code,
        interval_seconds: start.interval_seconds,
    }))
}

pub async fn poll_openai_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthOpenAiDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "openai");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_device_auth_target(provider_id.as_str(), &resolved)? {
        DeviceAuthTarget::OpenAi => {}
        DeviceAuthTarget::Copilot => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support openai device login"
            )));
        }
    }

    let auth = auth_manager(&state)
        .poll_openai_headless_login(provider_id.as_str(), request.device_code, request.user_code)
        .await
        .map_err(ServerError::Core)?;
    if auth.is_some() {
        reload_runtime_from_config(&state).await?;
    }
    let provider = if auth.is_some() {
        Some(auth_provider_resource_from_state(
            &state,
            provider_id.as_str(),
        )?)
    } else {
        None
    };
    Ok(Json(AuthLoginResultResource {
        completed: auth.is_some(),
        provider,
    }))
}

pub async fn start_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthGitLabBrowserStartRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "gitlab");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    let instance_url = match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::Gitlab { instance_url } => instance_url,
        BrowserAuthTarget::OpenAi => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support gitlab browser login"
            )));
        }
        BrowserAuthTarget::AtomGit => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support gitlab browser login"
            )));
        }
    };

    let start = auth_manager(&state)
        .start_gitlab_login(instance_url.clone(), request.redirect_uri)
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource {
        provider_id,
        instance_url: Some(instance_url),
        authorize_url: start.authorize_url,
        state: start.state,
        pkce_verifier: start.pkce_verifier,
    }))
}

pub async fn finish_gitlab_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthGitLabBrowserFinishRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "gitlab");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    let instance_url = match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::Gitlab { instance_url } => instance_url,
        BrowserAuthTarget::OpenAi => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support gitlab browser login"
            )));
        }
        BrowserAuthTarget::AtomGit => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support gitlab browser login"
            )));
        }
    };

    auth_manager(&state)
        .finish_gitlab_login(
            provider_id.as_str(),
            instance_url,
            request.code,
            request.pkce_verifier,
            request.redirect_uri,
        )
        .await
        .map_err(ServerError::Core)?;
    reload_runtime_from_config(&state).await?;
    Ok(Json(AuthLoginResultResource {
        completed: true,
        provider: Some(auth_provider_resource_from_state(
            &state,
            provider_id.as_str(),
        )?),
    }))
}

pub async fn start_atomgit_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthAtomGitBrowserStartRequest>,
) -> Result<Json<AuthBrowserStartResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "atomgit");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::AtomGit => {}
        BrowserAuthTarget::OpenAi | BrowserAuthTarget::Gitlab { .. } => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support atomgit browser login"
            )));
        }
    }

    let start = auth_manager(&state)
        .start_atomgit_login()
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthBrowserStartResource {
        provider_id,
        instance_url: None,
        authorize_url: start.authorize_url,
        state: start.state,
        pkce_verifier: start.pkce_verifier,
    }))
}

pub async fn poll_atomgit_browser_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthAtomGitBrowserPollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id = normalize_requested_provider_id(request.provider_id.as_deref(), "atomgit");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_browser_auth_target(provider_id.as_str(), &resolved)? {
        BrowserAuthTarget::AtomGit => {}
        BrowserAuthTarget::OpenAi | BrowserAuthTarget::Gitlab { .. } => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support atomgit browser login"
            )));
        }
    }

    let auth = auth_manager(&state)
        .poll_atomgit_login(provider_id.as_str(), request.state)
        .await
        .map_err(ServerError::Core)?;
    if auth.is_some() {
        reload_runtime_from_config(&state).await?;
    }
    let provider = if auth.is_some() {
        Some(auth_provider_resource_from_state(
            &state,
            provider_id.as_str(),
        )?)
    } else {
        None
    };
    Ok(Json(AuthLoginResultResource {
        completed: auth.is_some(),
        provider,
    }))
}

pub async fn start_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCopilotDeviceStartRequest>,
) -> Result<Json<AuthDeviceStartResource>, ServerError> {
    let provider_id =
        normalize_requested_provider_id(request.provider_id.as_deref(), "github-copilot");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_device_auth_target(provider_id.as_str(), &resolved)? {
        DeviceAuthTarget::Copilot => {}
        DeviceAuthTarget::OpenAi => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support copilot device login"
            )));
        }
    }

    let deployment = copilot_deployment(request.enterprise_domain.as_deref());
    let start = auth_manager(&state)
        .start_copilot_login(deployment)
        .await
        .map_err(ServerError::Core)?;
    Ok(Json(AuthDeviceStartResource {
        provider_id,
        enterprise_domain: request.enterprise_domain,
        verification_url: start.verification_url,
        user_code: start.user_code,
        device_code: start.device_code,
        interval_seconds: start.interval_seconds,
    }))
}

pub async fn poll_copilot_device_auth(
    State(state): State<AppState>,
    Json(request): Json<AuthCopilotDevicePollRequest>,
) -> Result<Json<AuthLoginResultResource>, ServerError> {
    let provider_id =
        normalize_requested_provider_id(request.provider_id.as_deref(), "github-copilot");
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_device_auth_target(provider_id.as_str(), &resolved)? {
        DeviceAuthTarget::Copilot => {}
        DeviceAuthTarget::OpenAi => {
            return Err(ServerError::BadRequest(format!(
                "{provider_id} does not support copilot device login"
            )));
        }
    }

    let deployment = copilot_deployment(request.enterprise_domain.as_deref());
    let auth = auth_manager(&state)
        .poll_copilot_login(provider_id.as_str(), request.device_code, deployment)
        .await
        .map_err(ServerError::Core)?;
    if auth.is_some() {
        reload_runtime_from_config(&state).await?;
    }
    let provider = if auth.is_some() {
        Some(auth_provider_resource_from_state(
            &state,
            provider_id.as_str(),
        )?)
    } else {
        None
    };
    Ok(Json(AuthLoginResultResource {
        completed: auth.is_some(),
        provider,
    }))
}

pub async fn delete_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    auth_provider_config(&state, provider_id.as_str())?;

    auth_manager(&state)
        .remove(provider_id.as_str())
        .map_err(ServerError::Core)?;
    reload_runtime_from_config(&state).await?;
    Ok(Json(auth_provider_resource_from_state(
        &state,
        provider_id.as_str(),
    )?))
}

pub async fn refresh_auth_provider(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
) -> Result<impl IntoResponse, ServerError> {
    let resolved = auth_provider_config(&state, provider_id.as_str())?;
    match resolve_refresh_auth_target(provider_id.as_str(), &resolved)? {
        RefreshAuthTarget::OpenAi => {
            auth_manager(&state)
                .refresh_openai_login(provider_id.as_str())
                .await
                .map_err(ServerError::Core)?;
        }
        RefreshAuthTarget::Gitlab { instance_url } => {
            auth_manager(&state)
                .refresh_gitlab_login(provider_id.as_str(), instance_url)
                .await
                .map_err(ServerError::Core)?;
        }
        RefreshAuthTarget::AtomGit => {
            auth_manager(&state)
                .refresh_atomgit_login(provider_id.as_str())
                .await
                .map_err(ServerError::Core)?;
        }
    }

    reload_runtime_from_config(&state).await?;
    Ok(Json(auth_provider_resource_from_state(
        &state,
        provider_id.as_str(),
    )?))
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

fn normalize_requested_provider_id(requested: Option<&str>, default: &str) -> String {
    requested
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(default)
        .to_owned()
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

enum BrowserAuthTarget {
    OpenAi,
    Gitlab { instance_url: String },
    AtomGit,
}

enum DeviceAuthTarget {
    OpenAi,
    Copilot,
}

enum RefreshAuthTarget {
    OpenAi,
    Gitlab { instance_url: String },
    AtomGit,
}

fn resolve_browser_auth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<BrowserAuthTarget, ServerError> {
    let openai = provider_supports_openai_oauth(resolved);
    let gitlab = provider_has_gitlab_adapter(resolved);
    let atomgit = provider_supports_atomgit_oauth(resolved);
    match (openai, gitlab, atomgit) {
        (true, false, false) => Ok(BrowserAuthTarget::OpenAi),
        (false, true, false) => provider_gitlab_instance_url(resolved)
            .map(|instance_url| BrowserAuthTarget::Gitlab { instance_url })
            .ok_or_else(|| {
                ServerError::BadRequest(format!(
                    "{provider_id} has ambiguous gitlab browser auth adapters"
                ))
            }),
        (false, false, true) => Ok(BrowserAuthTarget::AtomGit),
        (false, false, false) => Err(ServerError::BadRequest(format!(
            "{provider_id} does not support browser login"
        ))),
        _ => Err(ServerError::BadRequest(format!(
            "{provider_id} has ambiguous browser auth providers"
        ))),
    }
}

fn resolve_device_auth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<DeviceAuthTarget, ServerError> {
    let openai = provider_supports_openai_oauth(resolved);
    let copilot = provider_supports_copilot_device(resolved);
    match (openai, copilot) {
        (true, false) => Ok(DeviceAuthTarget::OpenAi),
        (false, true) => Ok(DeviceAuthTarget::Copilot),
        (true, true) => Err(ServerError::BadRequest(format!(
            "{provider_id} has ambiguous device auth providers"
        ))),
        (false, false) => Err(ServerError::BadRequest(format!(
            "{provider_id} does not support device login"
        ))),
    }
}

fn resolve_refresh_auth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<RefreshAuthTarget, ServerError> {
    let openai = provider_supports_openai_oauth(resolved);
    let gitlab = provider_has_gitlab_adapter(resolved);
    let atomgit = provider_supports_atomgit_oauth(resolved);
    match (openai, gitlab, atomgit) {
        (true, false, false) => Ok(RefreshAuthTarget::OpenAi),
        (false, true, false) => provider_gitlab_instance_url(resolved)
            .map(|instance_url| RefreshAuthTarget::Gitlab { instance_url })
            .ok_or_else(|| {
                ServerError::BadRequest(format!(
                    "{provider_id} has ambiguous gitlab refresh adapters"
                ))
            }),
        (false, false, true) => Ok(RefreshAuthTarget::AtomGit),
        (false, false, false) => Err(ServerError::BadRequest(format!(
            "credential refresh is not supported for provider '{provider_id}'"
        ))),
        _ => Err(ServerError::BadRequest(format!(
            "{provider_id} has ambiguous credential refresh handlers"
        ))),
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

#[cfg(test)]
mod tests {
    use super::*;
    use agena::config::{
        BedrockSigv4AuthConfig, HttpProviderAdapterConfig, OpenAiApiModeConfig,
        OpenAiBackendConfig, OpenAiProviderOptions, ProviderAdapterDefinition,
        ProviderApiAuthConfig, ProviderAuthConfig, ProviderCredentialAuthConfig,
        ProviderProtocolPathsConfig, ResolvedProviderAdapterConfig, ResolvedProviderConfig,
        StreamTransportMode,
    };

    fn openai_adapter(backend: OpenAiBackendConfig) -> (String, ResolvedProviderAdapterConfig) {
        (
            "openai".to_owned(),
            ResolvedProviderAdapterConfig {
                enabled: true,
                model_discovery: Default::default(),
                definition: ProviderAdapterDefinition::OpenAi(HttpProviderAdapterConfig {
                    extra_headers: Default::default(),
                    options: OpenAiProviderOptions {
                        backend,
                        api_mode: OpenAiApiModeConfig::Responses,
                        api_mode_explicit: false,
                        stream_mode: StreamTransportMode::Sse,
                        realtime_ws_url: None,
                        models_url: None,
                        auth_header: "authorization".to_owned(),
                        auth_scheme: Some("Bearer".to_owned()),
                        capability_family: None,
                    },
                }),
            },
        )
    }

    fn resolved_provider_with_auth(
        auth: ProviderAuthConfig,
        adapters: Vec<(String, ResolvedProviderAdapterConfig)>,
    ) -> ResolvedProviderConfig {
        ResolvedProviderConfig {
            enabled: true,
            default_adapter: "openai".to_owned(),
            default_model: "openai/default".to_owned(),
            auth,
            adapters: adapters.into_iter().collect(),
            models: Default::default(),
        }
    }

    fn oauth_credential_auth(
        issuer: agena::provider::auth::CredentialIssuer,
        credential: agena::provider::auth::AuthData,
    ) -> ProviderCredentialAuthConfig {
        ProviderCredentialAuthConfig {
            issuer,
            credential: Some(credential),
            base_url: None,
            protocol_paths: ProviderProtocolPathsConfig::default(),
            service_key_env: None,
        }
    }

    #[test]
    fn configured_provider_auth_id_uses_openai_for_chatgpt_codex_backend() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(oauth_credential_auth(
                agena::provider::auth::CredentialIssuer::OpenaiChatgpt,
                agena::provider::auth::AuthData::OAuth {
                    issuer: Some(agena::provider::auth::CredentialIssuer::OpenaiChatgpt),
                    refresh: "refresh-token".to_owned(),
                    access: "access-token".to_owned(),
                    expires_at_ms: 4_102_444_800_000,
                    account_id: Some("acct-openai".to_owned()),
                    enterprise_url: None,
                    user: None,
                },
            )),
            vec![openai_adapter(OpenAiBackendConfig::ChatgptCodex)],
        );

        assert_eq!(
            configured_provider_auth_id("openai_chatgpt", &provider).as_deref(),
            Some("openai_chatgpt")
        );
    }

    #[test]
    fn configured_provider_auth_id_prefers_gitlab_auth_provider_when_no_api_key_is_set() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(oauth_credential_auth(
                agena::provider::auth::CredentialIssuer::Gitlab,
                agena::provider::auth::AuthData::OAuth {
                    issuer: Some(agena::provider::auth::CredentialIssuer::Gitlab),
                    refresh: "refresh-token".to_owned(),
                    access: "access-token".to_owned(),
                    expires_at_ms: 4_102_444_800_000,
                    account_id: None,
                    enterprise_url: None,
                    user: None,
                },
            )),
            vec![],
        );

        assert_eq!(
            configured_provider_auth_id("gitlab-duo", &provider).as_deref(),
            Some("gitlab-duo")
        );
    }

    #[test]
    fn configured_provider_auth_id_uses_provider_id_for_direct_gitlab_api_key() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: Some("https://gitlab.com/api/v4".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                api_key: Some("glpat-test".to_owned()),
                api_key_env: None,
            }),
            vec![],
        );

        assert_eq!(
            configured_provider_auth_id("gitlab-self", &provider).as_deref(),
            Some("gitlab-self")
        );
    }

    #[test]
    fn configured_provider_auth_id_ignores_empty_gitlab_api_key_overrides() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(oauth_credential_auth(
                agena::provider::auth::CredentialIssuer::Gitlab,
                agena::provider::auth::AuthData::OAuth {
                    issuer: Some(agena::provider::auth::CredentialIssuer::Gitlab),
                    refresh: "refresh-token".to_owned(),
                    access: "access-token".to_owned(),
                    expires_at_ms: 4_102_444_800_000,
                    account_id: None,
                    enterprise_url: None,
                    user: None,
                },
            )),
            vec![],
        );

        assert_eq!(
            configured_provider_auth_id("gitlab", &provider).as_deref(),
            Some("gitlab")
        );
    }

    #[test]
    fn configured_provider_auth_id_keeps_direct_http_provider_ids() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: Some("https://api.openai.com".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                api_key: Some("sk-test".to_owned()),
                api_key_env: None,
            }),
            vec![],
        );

        assert_eq!(
            configured_provider_auth_id("openai", &provider).as_deref(),
            Some("openai")
        );
    }

    #[test]
    fn configured_provider_auth_id_uses_configured_copilot_auth_provider() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(oauth_credential_auth(
                agena::provider::auth::CredentialIssuer::GithubCopilot,
                agena::provider::auth::AuthData::OAuth {
                    issuer: Some(agena::provider::auth::CredentialIssuer::GithubCopilot),
                    refresh: "refresh-token".to_owned(),
                    access: "access-token".to_owned(),
                    expires_at_ms: 4_102_444_800_000,
                    account_id: None,
                    enterprise_url: Some("github.example.com".to_owned()),
                    user: None,
                },
            )),
            vec![openai_adapter(OpenAiBackendConfig::Api)],
        );

        assert_eq!(
            configured_provider_auth_id("copilot-enterprise", &provider).as_deref(),
            Some("copilot-enterprise")
        );
    }

    #[test]
    fn configured_provider_auth_id_keeps_cloudflare_gateway_provider_id() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Api(ProviderApiAuthConfig {
                base_url: Some("https://gateway.cloudflare.example".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                api_key: Some("cf-test".to_owned()),
                api_key_env: None,
            }),
            vec![],
        );

        assert_eq!(
            configured_provider_auth_id("cloudflare-ai-gateway", &provider).as_deref(),
            Some("cloudflare-ai-gateway")
        );
    }

    #[test]
    fn configured_provider_auth_id_skips_google_adc_and_sigv4_auth() {
        let google = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(ProviderCredentialAuthConfig {
                issuer: agena::provider::auth::CredentialIssuer::GoogleAdc,
                credential: None,
                base_url: Some("https://vertex.example.com".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                service_key_env: None,
            }),
            vec![],
        );
        let bedrock = resolved_provider_with_auth(
            ProviderAuthConfig::BedrockSigv4(BedrockSigv4AuthConfig {
                base_url: "https://bedrock-runtime.us-east-1.amazonaws.com".to_owned(),
                region: "us-east-1".to_owned(),
                profile: None,
                access_key_id: None,
                secret_access_key: None,
                session_token: None,
            }),
            vec![],
        );

        assert_eq!(configured_provider_auth_id("vertex", &google), None);
        assert_eq!(configured_provider_auth_id("bedrock", &bedrock), None);
    }

    #[test]
    fn configured_provider_auth_id_skips_sap_ai_core_service_key_auth() {
        let provider = resolved_provider_with_auth(
            ProviderAuthConfig::Credential(ProviderCredentialAuthConfig {
                issuer: agena::provider::auth::CredentialIssuer::SapAiCore,
                credential: None,
                base_url: Some("https://api.example.com/v2".to_owned()),
                protocol_paths: ProviderProtocolPathsConfig::default(),
                service_key_env: Some("AICORE_SERVICE_KEY".to_owned()),
            }),
            vec![],
        );

        assert_eq!(configured_provider_auth_id("sap-ai-core", &provider), None);
    }
}
