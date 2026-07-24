use super::*;

#[derive(Clone, Copy)]
pub(super) enum AuthOAuthPurpose {
    BrowserLogin,
    CredentialRefresh,
    DeviceLogin,
}

pub(super) fn auth_bad_request(
    message: impl Into<String>,
) -> agena_runtime::RuntimeAuthenticationError {
    agena_runtime::RuntimeAuthenticationError::bad_request(message)
}

pub(super) fn auth_internal(
    error: impl std::fmt::Display,
) -> agena_runtime::RuntimeAuthenticationError {
    agena_runtime::RuntimeAuthenticationError::internal(error.to_string())
}

pub(super) fn auth_manager_for_runtime(
    runtime: &AgenaRuntime,
) -> crate::provider::auth::AuthManager<crate::config::ProviderConfigCredentialStore> {
    crate::provider::auth::AuthManager::new(crate::config::ProviderConfigCredentialStore::new(
        runtime.current_snapshot().config_path().to_path_buf(),
    ))
}

pub(super) fn auth_provider_is_configured(
    resolved: &agena_runtime::ResolvedProviderConfig,
) -> bool {
    match &resolved.auth {
        ProviderAuthConfig::None => false,
        ProviderAuthConfig::Api(api) => api.bedrock_sigv4().is_none(),
        ProviderAuthConfig::Credential(config) => !matches!(
            config.issuer(),
            agena_provider::CredentialIssuer::GoogleAdc
                | agena_provider::CredentialIssuer::SapAiCore
        ),
    }
}

pub(super) fn auth_resolved_provider(
    runtime: &AgenaRuntime,
    provider_id: &str,
) -> Result<agena_runtime::ResolvedProviderConfig, agena_runtime::RuntimeAuthenticationError> {
    runtime
        .current_snapshot()
        .provider_configs()
        .get(provider_id)
        .cloned()
        .filter(auth_provider_is_configured)
        .ok_or_else(|| {
            agena_runtime::RuntimeAuthenticationError::not_found(format!(
                "auth provider not found: {provider_id}"
            ))
        })
}

pub(super) fn auth_provider_projection(
    provider_id: &str,
    resolved: &agena_runtime::ResolvedProviderConfig,
) -> Result<agena_runtime::RuntimeAuthProvider, agena_runtime::RuntimeAuthenticationError> {
    let auth = crate::config::provider_auth_data(resolved);
    let credential_present = auth.is_some();
    let (
        credential_type,
        credential_issuer,
        key_preview,
        expires_at,
        account_id,
        enterprise_url,
        username,
        display_name,
        email,
        avatar_url,
    ) = match auth {
        Some(agena_provider::AuthData::Api { key }) => (
            Some(agena_runtime::RuntimeAuthCredentialType::Api),
            None,
            auth_secret_preview(&key),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        Some(agena_provider::AuthData::OAuth {
            issuer,
            expires_at_ms,
            account_id,
            enterprise_url,
            user,
            ..
        }) => {
            let expires_at = if expires_at_ms > 0 {
                Some(
                    chrono::DateTime::from_timestamp_millis(expires_at_ms).ok_or_else(|| {
                        auth_internal(format!("invalid oauth expiry millis: {expires_at_ms}"))
                    })?,
                )
            } else {
                None
            };
            let (account_id, username, display_name, email, avatar_url) = match user {
                Some(user) => (
                    account_id.or(Some(user.id)),
                    Some(user.username),
                    user.name,
                    user.email,
                    user.avatar_url,
                ),
                None => (account_id, None, None, None, None),
            };
            (
                Some(agena_runtime::RuntimeAuthCredentialType::Oauth),
                issuer.map(auth_issuer),
                None,
                expires_at,
                account_id,
                enterprise_url,
                username,
                display_name,
                email,
                avatar_url,
            )
        }
        Some(agena_provider::AuthData::WellKnown { key, .. }) => (
            Some(agena_runtime::RuntimeAuthCredentialType::WellKnown),
            None,
            Some(key),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        ),
        None => (None, None, None, None, None, None, None, None, None, None),
    };
    let (browser_login_kind, browser_login_instance_url) =
        match crate::config::resolve_provider_oauth_target(resolved) {
            Ok(Some(crate::config::ProviderOAuthTarget::OpenAi)) => (
                Some(agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt),
                None,
            ),
            Ok(Some(crate::config::ProviderOAuthTarget::Gitlab { instance_url })) => (
                Some(agena_runtime::RuntimeAuthLoginKind::Gitlab),
                Some(instance_url),
            ),
            Err(crate::config::ProviderAuthTargetError::AmbiguousGitlab) => {
                (None, crate::config::provider_gitlab_instance_url(resolved))
            }
            Ok(None) | Err(crate::config::ProviderAuthTargetError::AmbiguousProvider) => {
                (None, None)
            }
        };
    let device_login_kind = match crate::config::resolve_provider_device_auth_target(resolved) {
        Ok(Some(crate::config::ProviderDeviceAuthTarget::OpenAi)) => {
            Some(agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt)
        }
        Ok(Some(crate::config::ProviderDeviceAuthTarget::Copilot)) => {
            Some(agena_runtime::RuntimeAuthLoginKind::GithubCopilot)
        }
        Ok(None) | Err(_) => None,
    };
    Ok(agena_runtime::RuntimeAuthProvider {
        provider_id: provider_id.to_owned(),
        credential_present,
        credential_type,
        credential_issuer,
        key_preview,
        expires_at,
        account_id,
        enterprise_url,
        username,
        display_name,
        email,
        avatar_url,
        api_key_write_supported: crate::config::provider_supports_api_key_write(resolved),
        browser_login_kind,
        browser_login_instance_url,
        device_login_kind,
    })
}

pub(super) fn auth_issuer(
    issuer: agena_provider::CredentialIssuer,
) -> agena_runtime::RuntimeAuthCredentialIssuer {
    match issuer {
        agena_provider::CredentialIssuer::OpenaiChatgpt => {
            agena_runtime::RuntimeAuthCredentialIssuer::OpenaiChatgpt
        }
        agena_provider::CredentialIssuer::GithubCopilot => {
            agena_runtime::RuntimeAuthCredentialIssuer::GithubCopilot
        }
        agena_provider::CredentialIssuer::Gitlab => {
            agena_runtime::RuntimeAuthCredentialIssuer::Gitlab
        }
        agena_provider::CredentialIssuer::GoogleAdc => {
            agena_runtime::RuntimeAuthCredentialIssuer::GoogleAdc
        }
        agena_provider::CredentialIssuer::SapAiCore => {
            agena_runtime::RuntimeAuthCredentialIssuer::SapAiCore
        }
    }
}

pub(super) fn auth_secret_preview(secret: &str) -> Option<String> {
    let value = secret.trim();
    (!value.is_empty()).then(|| {
        if value.len() <= 8 {
            "*".repeat(value.len())
        } else {
            format!("{}...{}", &value[..4], &value[value.len() - 4..])
        }
    })
}

pub(super) fn auth_oauth_target(
    runtime: &AgenaRuntime,
    provider_id: &str,
    purpose: AuthOAuthPurpose,
) -> Result<crate::config::ProviderOAuthTarget, agena_runtime::RuntimeAuthenticationError> {
    let resolved = auth_resolved_provider(runtime, provider_id)?;
    crate::config::resolve_provider_oauth_target(&resolved)
        .map_err(|error| auth_target_error(provider_id, purpose, error))?
        .ok_or_else(|| match purpose {
            AuthOAuthPurpose::BrowserLogin => {
                auth_bad_request(format!("{provider_id} does not support browser login"))
            }
            AuthOAuthPurpose::CredentialRefresh => auth_bad_request(format!(
                "credential refresh is not supported for provider '{provider_id}'"
            )),
            AuthOAuthPurpose::DeviceLogin => {
                auth_bad_request(format!("{provider_id} does not support device login"))
            }
        })
}

pub(super) fn auth_device_target(
    runtime: &AgenaRuntime,
    provider_id: &str,
) -> Result<crate::config::ProviderDeviceAuthTarget, agena_runtime::RuntimeAuthenticationError> {
    let resolved = auth_resolved_provider(runtime, provider_id)?;
    crate::config::resolve_provider_device_auth_target(&resolved)
        .map_err(|error| auth_target_error(provider_id, AuthOAuthPurpose::DeviceLogin, error))?
        .ok_or_else(|| auth_bad_request(format!("{provider_id} does not support device login")))
}

pub(super) fn auth_target_error(
    provider_id: &str,
    purpose: AuthOAuthPurpose,
    error: crate::config::ProviderAuthTargetError,
) -> agena_runtime::RuntimeAuthenticationError {
    let text = match (purpose, error) {
        (
            AuthOAuthPurpose::BrowserLogin,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous browser auth providers"),
        (
            AuthOAuthPurpose::BrowserLogin,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous gitlab browser auth adapters"),
        (
            AuthOAuthPurpose::CredentialRefresh,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous credential refresh handlers"),
        (
            AuthOAuthPurpose::CredentialRefresh,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous gitlab refresh adapters"),
        (
            AuthOAuthPurpose::DeviceLogin,
            crate::config::ProviderAuthTargetError::AmbiguousProvider,
        ) => format!("{provider_id} has ambiguous device auth providers"),
        (
            AuthOAuthPurpose::DeviceLogin,
            crate::config::ProviderAuthTargetError::AmbiguousGitlab,
        ) => format!("{provider_id} has ambiguous device auth providers"),
    };
    auth_bad_request(text)
}

pub(super) fn auth_kind_name(kind: agena_runtime::RuntimeAuthLoginKind) -> &'static str {
    match kind {
        agena_runtime::RuntimeAuthLoginKind::OpenaiChatgpt => "openai",
        agena_runtime::RuntimeAuthLoginKind::GithubCopilot => "copilot",
        agena_runtime::RuntimeAuthLoginKind::Gitlab => "gitlab",
    }
}

pub(super) fn auth_copilot_deployment(domain: Option<String>) -> agena_provider::CopilotDeployment {
    match domain
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
    {
        Some(domain) => agena_provider::CopilotDeployment::Enterprise { domain },
        None => agena_provider::CopilotDeployment::GitHubCom,
    }
}
