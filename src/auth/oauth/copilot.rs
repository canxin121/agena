use serde::Deserialize;

use crate::{
    auth::{DeviceCodeStart, OAuthTokenResponse},
    error::{AppError, ProviderErrorKind},
};

use super::shared::{COPILOT_CLIENT_ID, normalize_domain};

pub async fn start_copilot_device_code(domain: &str) -> Result<DeviceCodeStart, AppError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/device/code");

    #[derive(Debug, Deserialize)]
    struct DeviceCodeResponse {
        verification_uri: String,
        user_code: String,
        device_code: String,
        interval: Option<u64>,
    }

    let response = reqwest::Client::new()
        .post(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "scope": "read:user",
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<empty>".to_owned());
        return Err(AppError::HttpStatus {
            provider: "github-copilot".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let data: DeviceCodeResponse = response.json().await?;
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
) -> Result<Option<OAuthTokenResponse>, AppError> {
    let domain = normalize_domain(domain);
    let url = format!("https://{domain}/login/oauth/access_token");

    #[derive(Debug, Deserialize)]
    struct PollResult {
        access_token: Option<String>,
        error: Option<String>,
    }

    let response = reqwest::Client::new()
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

    if !response.status().is_success() {
        let status = response.status();
        let body = response
            .text()
            .await
            .unwrap_or_else(|_| "<empty>".to_owned());
        return Err(AppError::HttpStatus {
            provider: "github-copilot".to_owned(),
            status,
            body,
            kind: ProviderErrorKind::ApiError,
            retryable: false,
        });
    }

    let data: PollResult = response.json().await?;
    if let Some(token) = data.access_token {
        return Ok(Some(OAuthTokenResponse {
            refresh: token.clone(),
            access: token,
            expires_at_ms: 0,
            account_id: None,
        }));
    }

    match data.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") => Ok(None),
        Some(error) => Err(AppError::Provider(format!(
            "github copilot device oauth failed: {error}"
        ))),
        None => Ok(None),
    }
}
