use agena_application::dto::{
    AuthCredentialIssuerResource, AuthCredentialType, AuthDeviceStartResource, AuthProviderResource,
};
use std::{io::Write, time::Duration};

use super::{AppError, AuthSummary, SessionListRequest, SessionSummary, io, poll_until};

pub(super) fn review_prompt(base: &str) -> String {
    format!(
        "Review the current workspace changes against `{base}`. Focus on correctness, regressions, security issues, and missing tests. Report findings first, then concise remediation guidance."
    )
}

pub(super) fn auth_summary(auth: AuthProviderResource) -> AuthSummary {
    AuthSummary {
        provider_id: auth.provider_id,
        kind: match auth.credential_type {
            Some(AuthCredentialType::Api) => "api_key",
            Some(AuthCredentialType::Oauth) => "oauth",
            Some(AuthCredentialType::WellKnown) => "well_known",
            None => "none",
        }
        .to_owned(),
        account_id: auth.account_id,
        enterprise_url: auth.enterprise_url,
        username: auth.username,
        display_name: auth.display_name,
        email: auth.email,
        issuer: auth.credential_issuer.map(|issuer| match issuer {
            AuthCredentialIssuerResource::OpenaiChatgpt => "openai_chatgpt".to_owned(),
            AuthCredentialIssuerResource::GithubCopilot => "github_copilot".to_owned(),
            AuthCredentialIssuerResource::Gitlab => "gitlab".to_owned(),
            AuthCredentialIssuerResource::GoogleAdc => "google_adc".to_owned(),
            AuthCredentialIssuerResource::SapAiCore => "sap_ai_core".to_owned(),
        }),
        expires_at_ms: auth.expires_at.map(|value| value.timestamp_millis()),
    }
}

pub(super) fn normalize_login_provider(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

pub(super) fn browser_login_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

pub(super) fn prompt_browser_login(authorize_url: &str) -> Result<(), AppError> {
    println!("open this URL to continue: {authorize_url}");
    io::stdout().flush()?;
    Ok(())
}

pub(super) fn prompt_device_login(start: &AuthDeviceStartResource) -> Result<(), AppError> {
    println!("open this URL: {}", start.verification_url);
    println!("enter code: {}", start.user_code);
    io::stdout().flush()?;
    Ok(())
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
    queries: &dyn agena_runtime::SessionQueryService,
) -> Result<Vec<SessionSummary>, AppError> {
    let mut offset = 0_u64;
    let page_size = 200_u64;
    let mut sessions = Vec::new();
    loop {
        let page = queries
            .list_session_summaries(SessionListRequest {
                offset,
                limit: Some(page_size),
                include_subagents: false,
            })
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let count = page.len() as u64;
        sessions.extend(page);
        if count < page_size {
            break;
        }
        offset = offset.saturating_add(count);
    }
    Ok(sessions)
}
