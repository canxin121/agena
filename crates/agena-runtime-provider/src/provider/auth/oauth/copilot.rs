use serde::Deserialize;

use crate::ProviderError;
use agena_provider::DeviceCodeStart;

use super::shared::{
    COPILOT_CLIENT_ID, ensure_http_success, normalize_domain, provider_http_client,
    response_json_bounded,
};
use agena_provider::OAuthTokenResponse;

pub async fn start_copilot_device_code(domain: &str) -> Result<DeviceCodeStart, ProviderError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/device/code");

    #[derive(Debug, Deserialize)]
    struct DeviceCodeResponse {
        verification_uri: String,
        user_code: String,
        device_code: String,
        interval: Option<u64>,
    }

    let response = provider_http_client()?
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "scope": "read:user",
        }))
        .send()
        .await?;

    let response = ensure_http_success("github-copilot", None, response).await?;
    let data: DeviceCodeResponse = response_json_bounded(response).await?;
    Ok(DeviceCodeStart {
        verification_url: data.verification_uri,
        user_code: data.user_code,
        device_code: data.device_code,
        interval_seconds: data.interval.unwrap_or(5),
    })
}

pub async fn poll_copilot_device_code(
    domain: &str,
    device_code: &str,
) -> Result<Option<OAuthTokenResponse>, ProviderError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/oauth/access_token");

    let response = provider_http_client()?
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        }))
        .send()
        .await?;

    let response = ensure_http_success("github-copilot", None, response).await?;
    let data: PollResult = response_json_bounded(response).await?;
    data.into_token()
}

#[derive(Debug, Deserialize)]
struct PollResult {
    access_token: Option<String>,
    error: Option<String>,
}

impl PollResult {
    fn into_token(self) -> Result<Option<OAuthTokenResponse>, ProviderError> {
        if let Some(token) = self.access_token {
            return Ok(Some(OAuthTokenResponse {
                refresh: token.clone(),
                access: token,
                id_token: None,
                expires_at_ms: 0,
                account_id: None,
                chatgpt_account_is_fedramp: false,
                user: None,
            }));
        }

        match self.error.as_deref() {
            Some("authorization_pending") | Some("slow_down") => Ok(None),
            Some(error) => Err(ProviderError::Provider(format!(
                "github copilot device oauth failed: {error}"
            ))),
            None => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn device_poll_fixtures_preserve_pending_slowdown_success_and_terminal_errors() {
        for value in [
            json!({}),
            json!({"error": "authorization_pending"}),
            json!({"error": "slow_down"}),
        ] {
            assert!(
                serde_json::from_value::<PollResult>(value)
                    .unwrap()
                    .into_token()
                    .unwrap()
                    .is_none()
            );
        }
        let token = serde_json::from_value::<PollResult>(json!({"access_token": "github-token"}))
            .unwrap()
            .into_token()
            .unwrap()
            .unwrap();
        assert_eq!(token.access, "github-token");
        assert_eq!(token.refresh, "github-token");
        assert_eq!(token.expires_at_ms, 0);
        for error in ["expired_token", "access_denied"] {
            assert!(
                serde_json::from_value::<PollResult>(json!({"error": error}))
                    .unwrap()
                    .into_token()
                    .is_err()
            );
        }
    }
}
