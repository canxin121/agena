//! Provider-neutral OAuth transport for providers that have not been saved yet.
//!
//! Unlike [`RuntimeAuthenticationService`], this port never names a persisted
//! provider id or credential store. The terminal owns its draft and writes the
//! returned token into that draft only after a successful interactive flow.

use async_trait::async_trait;
use oauth2::{
    AuthUrl, AuthorizationCode, ClientId, ClientSecret, CsrfToken, EndpointNotSet, EndpointSet,
    PkceCodeChallenge, PkceCodeVerifier, RedirectUrl, Scope, TokenResponse, TokenUrl,
    basic::BasicClient,
};

use crate::RuntimeAuthenticationError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Kind of a draft authentication flow.
pub enum RuntimeDraftAuthKind {
    OpenaiChatgpt,
    GithubCopilot,
    Gitlab,
}

#[derive(Debug, Clone)]
/// Browser start details of a draft auth flow.
pub struct RuntimeDraftAuthBrowserStart {
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone)]
/// Device start details of a draft auth flow.
pub struct RuntimeDraftAuthDeviceStart {
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone)]
/// Token of a draft authentication flow.
pub struct RuntimeDraftAuthToken {
    pub refresh_token: String,
    pub access_token: String,
    pub expires_at_ms: i64,
    pub account_id: Option<String>,
}

#[async_trait]
/// Service for draft provider authentication.
pub trait RuntimeDraftAuthenticationService: Send + Sync {
    fn start_draft_auth_browser(
        &self,
        kind: RuntimeDraftAuthKind,
        instance_url: Option<String>,
        redirect_uri: String,
    ) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError>;

    async fn finish_draft_auth_browser(
        &self,
        kind: RuntimeDraftAuthKind,
        instance_url: Option<String>,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError>;

    async fn start_draft_auth_device(
        &self,
        kind: RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
    ) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError>;

    async fn poll_draft_auth_device(
        &self,
        kind: RuntimeDraftAuthKind,
        enterprise_domain: Option<String>,
        device_code: String,
        user_code: Option<String>,
    ) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError>;
}

const COPILOT_CLIENT_ID: &str = "Ov23li8tweQw6odWQebz";
const OPENAI_CLIENT_ID: &str = "app_EMoamEEZ73f0CkXaXp7hrann";
const OPENAI_ISSUER: &str = "https://auth.openai.com";

type DraftOAuthClient =
    BasicClient<EndpointSet, EndpointNotSet, EndpointNotSet, EndpointNotSet, EndpointSet>;

fn normalized_copilot_domain(url_or_domain: &str) -> String {
    url_or_domain
        .trim()
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_owned()
}

/// Runtime-owned GitHub Copilot device-auth transport used by the draft-auth
/// adapter. It deliberately returns only draft-safe stable values.
pub async fn start_copilot_draft_auth_device(
    domain: &str,
) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError> {
    #[derive(serde::Deserialize)]
    struct DeviceCodeResponse {
        verification_uri: String,
        user_code: String,
        device_code: String,
        interval: Option<u64>,
    }

    let domain = normalized_copilot_domain(domain);
    let response = reqwest::Client::new()
        .post(format!("https://{domain}/login/device/code"))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "scope": "read:user",
        }))
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(RuntimeAuthenticationError::internal(format!(
            "github copilot device oauth start failed with status {status}: {body}"
        )));
    }
    let data: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    Ok(RuntimeDraftAuthDeviceStart {
        verification_url: data.verification_uri,
        user_code: data.user_code,
        device_code: data.device_code,
        interval_seconds: data.interval.unwrap_or(5),
    })
}

/// Poll the Runtime-owned Copilot device flow. `Ok(None)` is the normal
/// pending/slow-down state, not an error.
pub async fn poll_copilot_draft_auth_device(
    domain: &str,
    device_code: &str,
) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError> {
    #[derive(serde::Deserialize)]
    struct PollResult {
        access_token: Option<String>,
        error: Option<String>,
    }

    let domain = normalized_copilot_domain(domain);
    let response = reqwest::Client::new()
        .post(format!("https://{domain}/login/oauth/access_token"))
        .header(reqwest::header::ACCEPT, "application/json")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .json(&serde_json::json!({
            "client_id": COPILOT_CLIENT_ID,
            "device_code": device_code,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
        }))
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(RuntimeAuthenticationError::internal(format!(
            "github copilot device oauth poll failed with status {status}: {body}"
        )));
    }
    let data: PollResult = response
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    if let Some(token) = data.access_token {
        return Ok(Some(RuntimeDraftAuthToken {
            refresh_token: token.clone(),
            access_token: token,
            expires_at_ms: 0,
            account_id: None,
        }));
    }
    match data.error.as_deref() {
        Some("authorization_pending") | Some("slow_down") | None => Ok(None),
        Some(error) => Err(RuntimeAuthenticationError::internal(format!(
            "github copilot device oauth failed: {error}"
        ))),
    }
}

/// Runtime-owned OpenAI device-auth start. The caller supplies the process
/// request identity while Runtime owns HTTP, token decoding, and projection.
pub async fn start_openai_draft_auth_device(
    user_agent: &str,
) -> Result<RuntimeDraftAuthDeviceStart, RuntimeAuthenticationError> {
    #[derive(serde::Deserialize)]
    struct DeviceCodeResponse {
        device_auth_id: String,
        user_code: String,
        #[serde(default)]
        interval: Option<serde_json::Value>,
    }

    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/usercode"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, user_agent)
        .json(&serde_json::json!({ "client_id": OPENAI_CLIENT_ID }))
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let response = openai_success("device oauth start", response).await?;
    let data: DeviceCodeResponse = response
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    Ok(RuntimeDraftAuthDeviceStart {
        verification_url: format!("{OPENAI_ISSUER}/codex/device"),
        user_code: data.user_code,
        device_code: data.device_auth_id,
        interval_seconds: parse_openai_device_interval(data.interval, 5),
    })
}

/// Poll the Runtime-owned OpenAI device flow while retaining its account-id
/// projection from JWT token bodies.
pub async fn poll_openai_draft_auth_device(
    user_agent: &str,
    device_auth_id: &str,
    user_code: &str,
) -> Result<Option<RuntimeDraftAuthToken>, RuntimeAuthenticationError> {
    #[derive(serde::Deserialize)]
    struct DevicePollResponse {
        authorization_code: String,
        code_verifier: String,
    }
    #[derive(serde::Deserialize)]
    struct TokenResponseBody {
        #[serde(default)]
        id_token: Option<String>,
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/api/accounts/deviceauth/token"))
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .header(reqwest::header::USER_AGENT, user_agent)
        .json(&serde_json::json!({
            "device_auth_id": device_auth_id,
            "user_code": user_code,
        }))
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    if matches!(
        response.status(),
        reqwest::StatusCode::FORBIDDEN | reqwest::StatusCode::NOT_FOUND
    ) {
        return Ok(None);
    }
    let poll: DevicePollResponse = openai_success("device oauth poll", response)
        .await?
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let encoded_form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", poll.authorization_code.as_str());
        form.append_pair(
            "redirect_uri",
            "https://auth.openai.com/deviceauth/callback",
        );
        form.append_pair("client_id", OPENAI_CLIENT_ID);
        form.append_pair("code_verifier", poll.code_verifier.as_str());
        form.finish()
    };
    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::USER_AGENT, user_agent)
        .body(encoded_form)
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let token: TokenResponseBody = openai_success("device oauth token exchange", response)
        .await?
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        RuntimeAuthenticationError::internal("openai device oauth response missing refresh_token")
    })?;
    Ok(Some(RuntimeDraftAuthToken {
        refresh_token,
        access_token: token.access_token.clone(),
        expires_at_ms: expires_at_ms_from_seconds(token.expires_in),
        account_id: openai_account_id(token.id_token.as_deref(), token.access_token.as_str()),
    }))
}

pub fn start_openai_draft_auth_browser(
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError> {
    let client = draft_oauth_client(
        "openai",
        OPENAI_CLIENT_ID,
        None,
        format!("{OPENAI_ISSUER}/oauth/authorize"),
        format!("{OPENAI_ISSUER}/oauth/token"),
        Some(redirect_uri),
    )?;
    Ok(draft_oauth_authorize_start(
        &client,
        &[
            "openid",
            "profile",
            "email",
            "offline_access",
            "api.connectors.read",
            "api.connectors.invoke",
        ],
        &[
            ("id_token_add_organizations", "true"),
            ("codex_cli_simplified_flow", "true"),
            ("originator", crate::RUNTIME_CODEX_ORIGINATOR),
        ],
    ))
}

pub async fn finish_openai_draft_auth_browser(
    user_agent: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError> {
    #[derive(serde::Deserialize)]
    struct TokenResponseBody {
        #[serde(default)]
        id_token: Option<String>,
        access_token: String,
        #[serde(default)]
        refresh_token: Option<String>,
        #[serde(default)]
        expires_in: Option<u64>,
    }

    let encoded_form = {
        let mut form = url::form_urlencoded::Serializer::new(String::new());
        form.append_pair("grant_type", "authorization_code");
        form.append_pair("code", code);
        form.append_pair("redirect_uri", redirect_uri);
        form.append_pair("client_id", OPENAI_CLIENT_ID);
        form.append_pair("code_verifier", pkce_verifier);
        form.finish()
    };
    let response = reqwest::Client::new()
        .post(format!("{OPENAI_ISSUER}/oauth/token"))
        .header(
            reqwest::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(reqwest::header::USER_AGENT, user_agent)
        .body(encoded_form)
        .send()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let token: TokenResponseBody = openai_success("oauth token exchange", response)
        .await?
        .json()
        .await
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let refresh_token = token.refresh_token.ok_or_else(|| {
        RuntimeAuthenticationError::internal(
            "openai oauth token exchange failed: missing refresh_token",
        )
    })?;
    Ok(RuntimeDraftAuthToken {
        refresh_token,
        access_token: token.access_token.clone(),
        expires_at_ms: expires_at_ms_from_seconds(token.expires_in),
        account_id: openai_account_id(token.id_token.as_deref(), token.access_token.as_str()),
    })
}

pub fn start_gitlab_draft_auth_browser(
    instance_url: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthBrowserStart, RuntimeAuthenticationError> {
    let client = gitlab_draft_oauth_client(instance_url, redirect_uri, false)?;
    Ok(draft_oauth_authorize_start(&client, &["read_user"], &[]))
}

pub async fn finish_gitlab_draft_auth_browser(
    instance_url: &str,
    code: &str,
    pkce_verifier: &str,
    redirect_uri: &str,
) -> Result<RuntimeDraftAuthToken, RuntimeAuthenticationError> {
    let client = gitlab_draft_oauth_client(instance_url, redirect_uri, true)?;
    let http = oauth2::reqwest::ClientBuilder::new()
        .redirect(oauth2::reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| RuntimeAuthenticationError::internal(error.to_string()))?;
    let token = client
        .exchange_code(AuthorizationCode::new(code.to_owned()))
        .set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier.to_owned()))
        .request_async(&http)
        .await
        .map_err(|error| {
            RuntimeAuthenticationError::internal(format!(
                "gitlab oauth token exchange failed: {error}"
            ))
        })?;
    let refresh_token = token
        .refresh_token()
        .map(|value| value.secret().to_owned())
        .unwrap_or_default();
    Ok(RuntimeDraftAuthToken {
        refresh_token,
        access_token: token.access_token().secret().to_owned(),
        expires_at_ms: token
            .expires_in()
            .and_then(|duration| i64::try_from(duration.as_millis()).ok())
            .filter(|millis| *millis > 0)
            .map(|millis| chrono::Utc::now().timestamp_millis() + millis)
            .unwrap_or(0),
        account_id: None,
    })
}

fn draft_oauth_client(
    provider: &str,
    client_id: impl Into<String>,
    client_secret: Option<String>,
    auth_url: String,
    token_url: String,
    redirect_uri: Option<&str>,
) -> Result<DraftOAuthClient, RuntimeAuthenticationError> {
    let client = BasicClient::new(ClientId::new(client_id.into()))
        .set_auth_uri(AuthUrl::new(auth_url).map_err(|error| {
            RuntimeAuthenticationError::bad_request(format!("invalid {provider} auth url: {error}"))
        })?)
        .set_token_uri(TokenUrl::new(token_url).map_err(|error| {
            RuntimeAuthenticationError::bad_request(format!(
                "invalid {provider} token url: {error}"
            ))
        })?);
    let client = if let Some(client_secret) = client_secret {
        client.set_client_secret(ClientSecret::new(client_secret))
    } else {
        client
    };
    match redirect_uri {
        Some(redirect_uri) => {
            let redirect_uri = RedirectUrl::new(redirect_uri.to_owned()).map_err(|error| {
                RuntimeAuthenticationError::bad_request(format!("invalid redirect uri: {error}"))
            })?;
            Ok(client.set_redirect_uri(redirect_uri))
        }
        None => Ok(client),
    }
}

fn draft_oauth_authorize_start(
    client: &DraftOAuthClient,
    scopes: &[&str],
    extra_params: &[(&str, &str)],
) -> RuntimeDraftAuthBrowserStart {
    let (challenge, verifier) = PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(CsrfToken::new_random)
        .set_pkce_challenge(challenge);
    for scope in scopes {
        request = request.add_scope(Scope::new((*scope).to_owned()));
    }
    for (key, value) in extra_params {
        request = request.add_extra_param(*key, *value);
    }
    let (authorize_url, state) = request.url();
    RuntimeDraftAuthBrowserStart {
        authorize_url: authorize_url.to_string(),
        state: state.secret().to_owned(),
        pkce_verifier: verifier.secret().to_owned(),
    }
}

fn gitlab_draft_oauth_client(
    instance_url: &str,
    redirect_uri: &str,
    include_secret: bool,
) -> Result<DraftOAuthClient, RuntimeAuthenticationError> {
    let instance = instance_url.trim_end_matches('/');
    let client_id = std::env::var("GITLAB_CLIENT_ID")
        .map_err(|_| RuntimeAuthenticationError::bad_request("GITLAB_CLIENT_ID is not set"))?;
    let client_secret = if include_secret {
        let value = std::env::var("GITLAB_CLIENT_SECRET").map_err(|_| {
            RuntimeAuthenticationError::bad_request("GITLAB_CLIENT_SECRET is not set")
        })?;
        let value = value.trim();
        if value.is_empty() {
            return Err(RuntimeAuthenticationError::bad_request(
                "GITLAB_CLIENT_SECRET cannot be empty",
            ));
        }
        Some(value.to_owned())
    } else {
        None
    };
    draft_oauth_client(
        "gitlab",
        client_id,
        client_secret,
        format!("{instance}/oauth/authorize"),
        format!("{instance}/oauth/token"),
        Some(redirect_uri),
    )
}

async fn openai_success(
    context: &str,
    response: reqwest::Response,
) -> Result<reqwest::Response, RuntimeAuthenticationError> {
    if response.status().is_success() {
        return Ok(response);
    }
    let status = response.status();
    let body = response.text().await.unwrap_or_default();
    Err(RuntimeAuthenticationError::internal(format!(
        "openai {context} failed with status {status}: {body}"
    )))
}

fn parse_openai_device_interval(value: Option<serde_json::Value>, default_seconds: u64) -> u64 {
    value
        .as_ref()
        .and_then(|value| {
            value
                .as_u64()
                .or_else(|| value.as_str()?.trim().parse().ok())
        })
        .map(|value| value.max(1))
        .unwrap_or(default_seconds)
}

fn expires_at_ms_from_seconds(expires_in: Option<u64>) -> i64 {
    expires_in
        .filter(|seconds| *seconds > 0)
        .and_then(|seconds| i64::try_from(seconds).ok())
        .map(|seconds| chrono::Utc::now().timestamp_millis() + seconds.saturating_mul(1_000))
        .unwrap_or(0)
}

fn openai_account_id(id_token: Option<&str>, access_token: &str) -> Option<String> {
    id_token
        .and_then(openai_jwt_account_id)
        .or_else(|| openai_jwt_account_id(access_token))
}

fn openai_jwt_account_id(jwt: &str) -> Option<String> {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};

    let payload = jwt.split('.').nth(1)?;
    let decoded = URL_SAFE_NO_PAD.decode(payload).ok()?;
    let json: serde_json::Value = serde_json::from_slice(&decoded).ok()?;
    json.get("chatgpt_account_id")
        .and_then(|value| value.as_str())
        .map(ToOwned::to_owned)
        .or_else(|| {
            json.get("https://api.openai.com/auth")
                .and_then(|value| value.get("chatgpt_account_id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
        .or_else(|| {
            json.get("organizations")
                .and_then(|value| value.as_array())
                .and_then(|items| items.first())
                .and_then(|value| value.get("id"))
                .and_then(|value| value.as_str())
                .map(ToOwned::to_owned)
        })
}
