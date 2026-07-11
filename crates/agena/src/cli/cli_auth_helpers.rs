use std::io::Write;

use super::{
    AppError, AuthData, AuthSummary, CopilotDeployment, DeviceCodeStart, Duration,
    OAuthAuthorizeStart, OAuthCallback, ProviderAuthTargetError, ProviderDeviceAuthTarget,
    ProviderOAuthTarget, ResolvedProviderConfig, SessionListRequest, SessionSummary, io,
    poll_until, resolve_provider_device_auth_target, resolve_provider_oauth_target,
    wait_for_oauth_callback,
};

pub(super) fn review_prompt(base: &str) -> String {
    format!(
        "Review the current workspace changes against `{base}`. Focus on correctness, regressions, security issues, and missing tests. Report findings first, then concise remediation guidance."
    )
}

pub(super) fn auth_summary(provider_id: String, auth: AuthData) -> AuthSummary {
    match auth {
        AuthData::Api { .. } => AuthSummary {
            provider_id,
            kind: "api_key".to_owned(),
            account_id: None,
            enterprise_url: None,
            username: None,
            display_name: None,
            email: None,
            issuer: None,
            expires_at_ms: None,
        },
        AuthData::OAuth {
            issuer,
            expires_at_ms,
            account_id,
            enterprise_url,
            user,
            ..
        } => {
            let account_id = account_id.or_else(|| user.as_ref().map(|user| user.id.clone()));
            AuthSummary {
                provider_id,
                kind: "oauth".to_owned(),
                account_id,
                enterprise_url,
                username: user.as_ref().map(|user| user.username.clone()),
                display_name: user.as_ref().and_then(|user| user.name.clone()),
                email: user.as_ref().and_then(|user| user.email.clone()),
                issuer: issuer.map(|issuer| match issuer {
                    crate::provider::auth::CredentialIssuer::OpenaiChatgpt => {
                        "openai_chatgpt".to_owned()
                    }
                    crate::provider::auth::CredentialIssuer::GithubCopilot => {
                        "github_copilot".to_owned()
                    }
                    crate::provider::auth::CredentialIssuer::Gitlab => "gitlab".to_owned(),
                    crate::provider::auth::CredentialIssuer::GoogleAdc => "google_adc".to_owned(),
                    crate::provider::auth::CredentialIssuer::SapAiCore => "sap_ai_core".to_owned(),
                }),
                expires_at_ms: Some(expires_at_ms),
            }
        }
        AuthData::WellKnown { .. } => AuthSummary {
            provider_id,
            kind: "well_known".to_owned(),
            account_id: None,
            enterprise_url: None,
            username: None,
            display_name: None,
            email: None,
            issuer: None,
            expires_at_ms: None,
        },
    }
}

pub(super) fn normalize_login_provider(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

pub(super) fn browser_login_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

pub(super) fn resolve_login_oauth_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<ProviderOAuthTarget, AppError> {
    match resolve_provider_oauth_target(resolved) {
        Ok(Some(target)) => Ok(target),
        Ok(None) => Err(AppError::Config(format!(
            "{provider_id} does not support browser login"
        ))),
        Err(ProviderAuthTargetError::AmbiguousProvider) => Err(AppError::Config(format!(
            "{provider_id} has ambiguous browser auth providers"
        ))),
        Err(ProviderAuthTargetError::AmbiguousGitlab) => Err(AppError::Config(format!(
            "{provider_id} has ambiguous gitlab browser auth adapters"
        ))),
    }
}

pub(super) fn resolve_login_device_target(
    provider_id: &str,
    resolved: &ResolvedProviderConfig,
) -> Result<ProviderDeviceAuthTarget, AppError> {
    match resolve_provider_device_auth_target(resolved) {
        Ok(Some(target)) => Ok(target),
        Ok(None) => Err(AppError::Config(format!(
            "{provider_id} does not support device login"
        ))),
        Err(ProviderAuthTargetError::AmbiguousProvider) => Err(AppError::Config(format!(
            "{provider_id} has ambiguous device auth providers"
        ))),
        Err(ProviderAuthTargetError::AmbiguousGitlab) => {
            unreachable!("gitlab ambiguity is not possible for device auth targets")
        }
    }
}

pub(super) fn prompt_browser_login(authorize_url: &str) -> Result<(), AppError> {
    println!("open this URL to continue: {authorize_url}");
    io::stdout().flush()?;
    Ok(())
}

pub(super) fn prompt_device_login(start: &DeviceCodeStart) -> Result<(), AppError> {
    println!("open this URL: {}", start.verification_url);
    println!("enter code: {}", start.user_code);
    io::stdout().flush()?;
    Ok(())
}

pub(super) fn copilot_deployment_from_domain(enterprise_domain: Option<&str>) -> CopilotDeployment {
    match enterprise_domain
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        Some(domain) => CopilotDeployment::Enterprise {
            domain: domain.to_owned(),
        },
        None => CopilotDeployment::GitHubCom,
    }
}

pub(super) async fn complete_browser_callback_login<F, Fut>(
    port: u16,
    timeout: Duration,
    start: &OAuthAuthorizeStart,
    finish: F,
) -> Result<(), AppError>
where
    F: FnOnce(OAuthCallback) -> Fut,
    Fut: std::future::Future<Output = Result<(), AppError>>,
{
    prompt_browser_login(start.authorize_url.as_str())?;
    let callback = wait_for_oauth_callback(port, start.state.as_str(), timeout)?;
    finish(callback).await
}

pub(super) async fn complete_polled_login<T, F, Fut, P>(
    timeout: Duration,
    interval: Duration,
    timeout_message: &str,
    prompt: P,
    poll: F,
) -> Result<(), AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, AppError>>,
    P: FnOnce() -> Result<(), AppError>,
{
    prompt()?;
    if poll_until(timeout, interval, poll).await?.is_some() {
        Ok(())
    } else {
        Err(AppError::Config(timeout_message.to_owned()))
    }
}

pub(super) async fn list_all_session_summaries(
    manager: &crate::session::SessionManager,
) -> Result<Vec<SessionSummary>, AppError> {
    let mut offset = 0_u64;
    let page_size = 200_u64;
    let mut sessions = Vec::new();
    loop {
        let page = manager
            .list_session_summaries(SessionListRequest {
                offset,
                limit: Some(page_size),
                include_subagents: false,
            })
            .await?;
        let count = page.len() as u64;
        sessions.extend(page);
        if count < page_size {
            break;
        }
        offset = offset.saturating_add(count);
    }
    Ok(sessions)
}
