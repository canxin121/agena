//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the v2 endpoints.

use std::{
    fmt,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
};

use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, DismissActivityParams, ExportSessionParams, ForkSessionParams,
        ImportSessionParams, ListSessionTreeParams, ReplacePermissionRuleParams,
        ReplyPermissionParams, ReplyUserInputParams, ResolveWorkspaceParams,
        RevokePermissionRuleParams, RewindSessionParams, StopActivityParams, SubmitRunParams,
        UpdateSessionParams, UpdateWorkspaceParams, UpsertPermissionRuleParams,
    },
    notifications::Notification,
    queries::{
        ActivityLogsParams, GetActivityParams, GetOperationDetailParams, GetPermissionRuleParams,
        GetSessionParams, GetWorkspaceParams, ListPermissionRulesParams,
        ListProviderAdapterModelsParams, ListProviderModelsParams,
        ListSavedProviderAdapterModelsParams, ListSessionsParams, ListWorkspacesParams, Query,
        QueryResult,
    },
    resource::{
        BackgroundActivityResource, HealthResponse, PermissionRuleResource,
        ProviderAdapterModelsRequest, ProviderAdapterModelsResponse, RunOptions,
        SavedProviderAdapterModelsRequest, SessionExecutionResource, SessionOverviewResource,
        SessionResource, WorkspaceResource,
    },
};
use futures_util::{StreamExt, TryStreamExt as _};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_sse_codec::{Frame as SseFrame, SseDecoder};
use tokio_util::{codec::FramedRead, io::StreamReader};

use crate::error::ClientError;
use crate::ws::SubscriptionEvent;

const MAX_JSON_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_TEXT_RESPONSE_BYTES: usize = 16 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;

#[derive(Default)]
struct ClientAuthentication {
    /// Retained only for password-authenticated long-lived clients. It never
    /// enters endpoint discovery, DTOs, request diagnostics, or Debug output.
    password: Option<SecretString>,
    bearer: RwLock<Option<reqwest::header::HeaderValue>>,
    generation: AtomicU64,
    refresh: tokio::sync::Mutex<()>,
}

impl ClientAuthentication {
    fn anonymous() -> Self {
        Self::default()
    }

    fn static_bearer(token: &str) -> Result<Self, ClientError> {
        Ok(Self {
            password: None,
            bearer: RwLock::new(Some(bearer_header(token)?)),
            generation: AtomicU64::new(1),
            refresh: tokio::sync::Mutex::new(()),
        })
    }

    fn password_session(password: &str, token: &str) -> Result<Self, ClientError> {
        let password = password.trim();
        if password.is_empty() {
            return Err(ClientError::Protocol(
                "processing-center password must not be empty".to_owned(),
            ));
        }
        Ok(Self {
            password: Some(SecretString::from(password.to_owned())),
            bearer: RwLock::new(Some(bearer_header(token)?)),
            generation: AtomicU64::new(1),
            refresh: tokio::sync::Mutex::new(()),
        })
    }

    fn bearer(&self) -> Option<reqwest::header::HeaderValue> {
        self.bearer
            .read()
            .expect("processing-center bearer lock poisoned")
            .clone()
    }

    fn replace_bearer(&self, token: &str) -> Result<(), ClientError> {
        let token = bearer_header(token)?;
        *self
            .bearer
            .write()
            .expect("processing-center bearer lock poisoned") = Some(token);
        self.generation.fetch_add(1, Ordering::Release);
        Ok(())
    }

    fn kind(&self) -> &'static str {
        if self.password.is_some() {
            "password-refreshable"
        } else if self.bearer().is_some() {
            "static-bearer"
        } else {
            "anonymous"
        }
    }
}

fn bearer_header(token: &str) -> Result<reqwest::header::HeaderValue, ClientError> {
    let token = token.trim();
    if token.is_empty() {
        return Err(ClientError::Protocol(
            "processing-center bearer token must not be empty".to_owned(),
        ));
    }
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| {
            ClientError::Protocol(
                "processing-center bearer token contains invalid header characters".to_owned(),
            )
        })?;
    authorization.set_sensitive(true);
    Ok(authorization)
}

async fn read_response_text_bounded(
    mut response: reqwest::Response,
    max_bytes: usize,
    context: &str,
) -> Result<String, ClientError> {
    if response
        .content_length()
        .is_some_and(|length| length > max_bytes as u64)
    {
        return Err(ClientError::Protocol(format!(
            "{context} exceeds the {max_bytes}-byte limit"
        )));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len().saturating_add(chunk.len()) > max_bytes {
            return Err(ClientError::Protocol(format!(
                "{context} exceeds the {max_bytes}-byte limit"
            )));
        }
        bytes.extend_from_slice(&chunk);
    }
    String::from_utf8(bytes)
        .map_err(|_| ClientError::Protocol(format!("{context} is not UTF-8 text")))
}

/// Handle to an active HTTP notification subscription.
pub struct NotificationSubscription {
    rx: mpsc::Receiver<Result<SubscriptionEvent, ClientError>>,
    task: Option<JoinHandle<()>>,
}

/// Snapshot-plus-live attachment to one center-owned session.
///
/// The live subscription is established before the snapshot is read. Changes
/// committed during that read are therefore queued and the caller can safely
/// converge by applying them (or re-reading after a lag notification).
pub struct SessionConnection {
    pub snapshot: SessionExecutionResource,
    pub subscription: NotificationSubscription,
}

impl NotificationSubscription {
    pub async fn recv(&mut self) -> Option<Result<SubscriptionEvent, ClientError>> {
        self.rx.recv().await
    }

    pub fn close(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl Drop for NotificationSubscription {
    fn drop(&mut self) {
        self.close();
    }
}

/// Stateless REST client. Holds a `reqwest::Client` and the base URL like
/// `http://localhost:7878`.
#[derive(Clone)]
/// HTTP client for the Agena runtime API.
pub struct AgenaClient {
    base_url: url::Url,
    http: reqwest::Client,
    authentication: Arc<ClientAuthentication>,
}

impl fmt::Debug for AgenaClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgenaClient")
            .field("base_url", &self.base_url)
            .field("authentication", &self.authentication.kind())
            .finish_non_exhaustive()
    }
}

impl AgenaClient {
    pub fn new(base_url: impl AsRef<str>) -> Result<Self, ClientError> {
        let base_url = url::Url::parse(base_url.as_ref())
            .map_err(|e| ClientError::Transport(format!("invalid base url: {e}")))?;
        Ok(Self {
            base_url,
            http: reqwest::Client::builder()
                .connect_timeout(std::time::Duration::from_secs(10))
                .build()?,
            authentication: Arc::new(ClientAuthentication::anonymous()),
        })
    }

    pub fn with_bearer_token(mut self, token: impl AsRef<str>) -> Result<Self, ClientError> {
        self.authentication = Arc::new(ClientAuthentication::static_bearer(token.as_ref())?);
        Ok(self)
    }

    fn with_password_session(mut self, password: &str, token: &str) -> Result<Self, ClientError> {
        self.authentication = Arc::new(ClientAuthentication::password_session(password, token)?);
        Ok(self)
    }

    /// Connect to a processing center, validate its public identity, and
    /// optionally authenticate using an ephemeral bearer token or UI password.
    /// Password login exchanges the password for an in-memory session token;
    /// neither secret is written to endpoint discovery metadata.
    pub async fn connect_center(
        base_url: impl AsRef<str>,
        bearer_token: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, ClientError> {
        if bearer_token.is_some() && password.is_some() {
            return Err(ClientError::Protocol(
                "pass either a processing-center token or password, not both".to_owned(),
            ));
        }
        let client = Self::new(base_url)?;
        client.center_identity().await?;
        if let Some(token) = bearer_token {
            return client.with_bearer_token(token);
        }
        if let Some(password) = password {
            let token = client.create_ui_session(password).await?;
            return client.with_password_session(password, token.as_str());
        }
        Ok(client)
    }

    async fn create_ui_session(&self, password: &str) -> Result<String, ClientError> {
        if password.trim().is_empty() {
            return Err(ClientError::Protocol(
                "processing-center password must not be empty".to_owned(),
            ));
        }
        let response = self
            .http
            .post(self.endpoint("/auth/session"))
            .json(&serde_json::json!({ "password": password }))
            .send()
            .await?;
        let status = response.status();
        let body = read_response_text_bounded(
            response,
            MAX_ERROR_RESPONSE_BYTES,
            "processing-center authentication response",
        )
        .await?;
        let value: serde_json::Value = serde_json::from_str(body.as_str())?;
        if !status.is_success() {
            return Err(ClientError::Api(agena_api::error::ApiError::bad_request(
                "Processing-center authentication failed. Check the configured password or token.",
            )));
        }
        value
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                ClientError::Protocol(
                    "processing-center authentication returned no bearer token".to_owned(),
                )
            })
    }

    fn request_builder(
        &self,
        method: reqwest::Method,
        url: url::Url,
        body: Option<&serde_json::Value>,
        accept: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let mut request = self.http.request(method, url);
        if let Some(body) = body {
            request = request.json(body);
        }
        if let Some(accept) = accept {
            request = request.header(reqwest::header::ACCEPT, accept);
        }
        if let Some(bearer) = self.authentication.bearer() {
            request = request.header(reqwest::header::AUTHORIZATION, bearer);
        }
        request
    }

    /// Refresh a password-derived bearer token exactly once for a generation.
    ///
    /// A center restart invalidates every process-local session token. Many
    /// concurrent TUI refreshes may observe the resulting 401 together, so a
    /// shared mutex elects one password exchange while the generation check
    /// lets the remaining requests reuse its new token without repeating the
    /// login. Static bearer credentials are never reinterpreted as passwords.
    async fn refresh_password_session(
        &self,
        observed_generation: u64,
    ) -> Result<bool, ClientError> {
        let Some(password) = self.authentication.password.as_ref() else {
            return Ok(false);
        };
        let _refresh = self.authentication.refresh.lock().await;
        if self.authentication.generation.load(Ordering::Acquire) != observed_generation {
            return Ok(true);
        }
        let token = self.create_ui_session(password.expose_secret()).await?;
        self.authentication.replace_bearer(token.as_str())?;
        Ok(true)
    }

    /// Send one replayable REST/SSE handshake request and, for clients created
    /// from a UI password, reauthenticate and replay it once after HTTP 401.
    /// Authentication middleware rejects a request before its handler runs, so
    /// replaying a mutation after this specific response cannot duplicate an
    /// accepted write.
    async fn send_request(
        &self,
        method: reqwest::Method,
        url: url::Url,
        body: Option<&serde_json::Value>,
        accept: Option<&str>,
    ) -> Result<reqwest::Response, ClientError> {
        let observed_generation = self.authentication.generation.load(Ordering::Acquire);
        let response = self
            .request_builder(method.clone(), url.clone(), body, accept)
            .send()
            .await?;
        if response.status() != reqwest::StatusCode::UNAUTHORIZED {
            return Ok(response);
        }
        if !self.refresh_password_session(observed_generation).await? {
            return Ok(response);
        }
        drop(response);
        Ok(self
            .request_builder(method, url, body, accept)
            .send()
            .await?)
    }

    pub fn base_url(&self) -> &url::Url {
        &self.base_url
    }

    fn endpoint(&self, path: &str) -> url::Url {
        self.base_url
            .join(path.trim_start_matches('/'))
            .expect("valid endpoint")
    }

    fn append_live_scope(url: &mut url::Url, scope: &agena_api::Scope) {
        let mut q = url.query_pairs_mut();
        match scope {
            agena_api::Scope::Global => {}
            agena_api::Scope::Workspace { workspace_id } => {
                q.append_pair("scope_kind", "workspace");
                q.append_pair("workspace_id", &workspace_id.to_string());
            }
            agena_api::Scope::Session { session_id } => {
                q.append_pair("scope_kind", "session");
                q.append_pair("session_id", &session_id.to_string());
            }
        }
    }

    fn changes_stream_url(&self, scope: &agena_api::Scope) -> url::Url {
        let mut url = self.endpoint("/api/v1/changes/stream");
        Self::append_live_scope(&mut url, scope);
        url
    }

    async fn parse_json<T: serde::de::DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, ClientError> {
        let status = response.status();
        if response
            .content_length()
            .is_some_and(|length| length > MAX_JSON_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::Protocol(format!(
                "JSON response exceeds the {} MiB limit",
                MAX_JSON_RESPONSE_BYTES / 1024 / 1024
            )));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_JSON_RESPONSE_BYTES {
                return Err(ClientError::Protocol(format!(
                    "JSON response exceeds the {} MiB limit",
                    MAX_JSON_RESPONSE_BYTES / 1024 / 1024
                )));
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: serde_json::Value = serde_json::from_slice(&bytes)?;
        if !status.is_success() {
            if let Ok(api) = serde_json::from_value::<agena_api::error::ApiError>(value.clone()) {
                return Err(ClientError::Api(api));
            }
            if status == reqwest::StatusCode::UNAUTHORIZED
                && value
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|code| code.starts_with("auth_"))
            {
                return Err(ClientError::Api(agena_api::error::ApiError::bad_request(
                    "Processing-center authentication is required. Set AGENA_CENTER_PASSWORD or AGENA_CENTER_TOKEN.",
                )));
            }
            return Err(ClientError::Protocol(format!(
                "HTTP {status} error response did not use the shared API envelope"
            )));
        }
        Ok(serde_json::from_value(value)?)
    }

    async fn get_json<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<T, ClientError> {
        let response = self
            .send_request(reqwest::Method::GET, self.endpoint(path), None, None)
            .await?;
        self.parse_json(response).await
    }

    async fn post_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(path),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    async fn put_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
    ) -> Result<T, ClientError> {
        let response = self
            .send_request(reqwest::Method::PUT, self.endpoint(path), Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    async fn delete_json<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
    ) -> Result<T, ClientError> {
        let response = self
            .send_request(reqwest::Method::DELETE, self.endpoint(path), None, None)
            .await?;
        self.parse_json(response).await
    }

    async fn get_text(&self, path: &str) -> Result<String, ClientError> {
        let response = self
            .send_request(reqwest::Method::GET, self.endpoint(path), None, None)
            .await?;
        let status = response.status();
        let text =
            read_response_text_bounded(response, MAX_TEXT_RESPONSE_BYTES, "text response").await?;
        if !status.is_success() {
            let api: agena_api::error::ApiError = serde_json::from_str(&text)?;
            return Err(ClientError::Api(api));
        }
        Ok(text)
    }

    async fn send_notification_frame(
        tx: &mpsc::Sender<Result<SubscriptionEvent, ClientError>>,
        notification: Notification,
    ) -> bool {
        let item = match notification {
            Notification::SessionChanged { change, .. } => {
                Ok(SubscriptionEvent::SessionChanged(*change))
            }
            Notification::RuntimeSignal { signal, .. } => {
                Ok(SubscriptionEvent::RuntimeSignal(*signal))
            }
            Notification::Lagged { skipped, .. } => Ok(SubscriptionEvent::Lagged(skipped)),
            Notification::SubscriptionClosed { reason, .. } => Err(ClientError::Protocol(format!(
                "sse subscription closed: {reason}"
            ))),
        };
        tx.send(item).await.is_ok()
    }

    // ─── high-level conveniences ───

    pub async fn health(&self) -> Result<HealthResponse, ClientError> {
        self.get_json("/api/v1/health").await
    }

    /// Validate that the endpoint is a current Agena processing center and
    /// return its process-lifetime identity. Legacy servers without identity
    /// metadata are reachable but cannot participate in safe discovery.
    pub async fn center_identity(
        &self,
    ) -> Result<agena_api::resource::CenterIdentityResource, ClientError> {
        let health = self.health().await?;
        let center = health.center.ok_or_else(|| {
            ClientError::Protocol(
                "the endpoint does not expose a processing-center identity".to_owned(),
            )
        })?;
        if center.protocol_version != agena_api::PROTOCOL_VERSION {
            return Err(ClientError::Protocol(format!(
                "processing-center protocol {} is incompatible with client protocol {}",
                center.protocol_version,
                agena_api::PROTOCOL_VERSION
            )));
        }
        Ok(center)
    }

    pub async fn list_provider_adapter_models(
        &self,
        request: ProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsResponse, ClientError> {
        self.post_json("/api/v1/providers/models", serde_json::to_value(request)?)
            .await
    }

    pub async fn list_saved_provider_adapter_models(
        &self,
        provider_id: &str,
        request: SavedProviderAdapterModelsRequest,
    ) -> Result<ProviderAdapterModelsResponse, ClientError> {
        self.post_json(
            &format!("/api/v1/providers/{provider_id}/models"),
            serde_json::to_value(request)?,
        )
        .await
    }

    pub async fn create_session(
        &self,
        workspace_id: i64,
        title: impl Into<String>,
        parent_id: Option<i64>,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "workspace_id": workspace_id,
            "title": title.into(),
            "parent_id": parent_id,
        });
        self.post_json("/api/v1/sessions", body).await
    }

    pub async fn submit_message(
        &self,
        params: SubmitRunParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert(
                "document".to_string(),
                serde_json::to_value(params.document)?,
            );
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/messages", params.session_id),
            body,
        )
        .await
    }

    pub async fn continue_run(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ClientError> {
        let body = serde_json::to_value(options.clone())?;
        let _ = ContinueRunParams {
            session_id,
            options,
        };
        self.post_json(&format!("/api/v1/sessions/{session_id}/continue"), body)
            .await
    }

    /// Read the center-owned plugin runtime status. The plugin runtime's
    /// detailed host DTOs have not yet moved into `agena-api`, so this SDK
    /// method preserves the public REST JSON without depending on the host
    /// implementation crate.
    pub async fn plugin_statuses(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/plugins").await
    }

    pub async fn plugin_inspect(&self, plugin_id: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/plugins");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(plugin_id);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn plugin_logs(
        &self,
        plugin_id: &str,
        after_seq: Option<u64>,
        limit: usize,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/plugins");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(plugin_id)
            .push("logs");
        {
            let mut query = url.query_pairs_mut();
            if let Some(after_seq) = after_seq {
                query.append_pair("after_seq", &after_seq.to_string());
            }
            query.append_pair("limit", &limit.to_string());
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn auth_providers(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/auth/providers").await
    }

    pub async fn auth_provider(&self, provider_id: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/auth/providers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(provider_id);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn start_openai_browser_auth(
        &self,
        provider_id: &str,
        redirect_uri: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/openai/browser/start",
            serde_json::json!({
                "provider_id": provider_id,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn finish_openai_browser_auth(
        &self,
        provider_id: &str,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/openai/browser/finish",
            serde_json::json!({
                "provider_id": provider_id,
                "code": code,
                "pkce_verifier": pkce_verifier,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn start_gitlab_browser_auth(
        &self,
        provider_id: &str,
        redirect_uri: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/gitlab/browser/start",
            serde_json::json!({
                "provider_id": provider_id,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn finish_gitlab_browser_auth(
        &self,
        provider_id: &str,
        code: String,
        pkce_verifier: String,
        redirect_uri: String,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/gitlab/browser/finish",
            serde_json::json!({
                "provider_id": provider_id,
                "code": code,
                "pkce_verifier": pkce_verifier,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn start_openai_device_auth(
        &self,
        provider_id: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/openai/device/start",
            serde_json::json!({ "provider_id": provider_id }),
        )
        .await
    }

    pub async fn poll_openai_device_auth(
        &self,
        provider_id: &str,
        device_code: String,
        user_code: String,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/openai/device/poll",
            serde_json::json!({
                "provider_id": provider_id,
                "device_code": device_code,
                "user_code": user_code,
            }),
        )
        .await
    }

    pub async fn start_copilot_device_auth(
        &self,
        provider_id: &str,
        enterprise_domain: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/github-copilot/device/start",
            serde_json::json!({
                "provider_id": provider_id,
                "enterprise_domain": enterprise_domain,
            }),
        )
        .await
    }

    pub async fn poll_copilot_device_auth(
        &self,
        provider_id: &str,
        device_code: String,
        enterprise_domain: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/auth/providers/github-copilot/device/poll",
            serde_json::json!({
                "provider_id": provider_id,
                "device_code": device_code,
                "enterprise_domain": enterprise_domain,
            }),
        )
        .await
    }

    pub async fn set_auth_api_key(
        &self,
        provider_id: &str,
        api_key: String,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/auth/providers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(provider_id)
            .push("api-key");
        let body = serde_json::json!({ "api_key": api_key });
        let response = self
            .send_request(reqwest::Method::PUT, url, Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn delete_auth_provider(
        &self,
        provider_id: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/auth/providers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(provider_id);
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn set_mcp_bearer_credential(
        &self,
        server: &str,
        token: String,
        store: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/mcp/credentials");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(server)
            .push("bearer");
        let body = serde_json::json!({"token": token, "store": store});
        let response = self
            .send_request(reqwest::Method::PUT, url, Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn delete_mcp_bearer_credential(
        &self,
        server: &str,
        store: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/mcp/credentials");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(server)
            .push("bearer");
        url.query_pairs_mut().append_pair("store", store);
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn start_mcp_oauth(
        &self,
        server: &str,
        url: &str,
        scopes: &[String],
        redirect_uri: &str,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/mcp/oauth/start",
            serde_json::json!({
                "server": server,
                "url": url,
                "scopes": scopes,
                "redirect_uri": redirect_uri,
            }),
        )
        .await
    }

    pub async fn finish_mcp_oauth(
        &self,
        flow_id: uuid::Uuid,
        code: String,
        state: String,
        issuer: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/mcp/oauth/finish",
            serde_json::json!({
                "flow_id": flow_id,
                "code": code,
                "state": state,
                "issuer": issuer,
            }),
        )
        .await
    }

    pub async fn delete_mcp_oauth_credential(
        &self,
        server: &str,
        revoke: bool,
        endpoint: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/mcp/oauth");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(server);
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("revoke", if revoke { "true" } else { "false" });
            if let Some(endpoint) = endpoint {
                query.append_pair("url", endpoint);
            }
        }
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn git_status(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/git/status").await
    }

    pub async fn snapshot_status(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/snapshots").await
    }

    pub async fn runtime_status(
        &self,
    ) -> Result<agena_api::resource::RuntimeStatusResponse, ClientError> {
        self.get_json("/api/v1/runtime").await
    }

    pub async fn resolved_config(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/config/resolved").await
    }

    pub async fn validate_config(&self) -> Result<serde_json::Value, ClientError> {
        self.post_json("/api/v1/settings/validate", serde_json::json!({}))
            .await
    }

    pub async fn settings_layer_value(
        &self,
        layer: &str,
        path: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/settings/layers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(layer);
        url.query_pairs_mut().append_pair("path", path);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn set_settings_layer_value(
        &self,
        layer: &str,
        path: &str,
        value: serde_json::Value,
        dry_run: bool,
        reload: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/settings/layers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(layer);
        let body = serde_json::json!({
            "path": path,
            "value": value,
            "dry_run": dry_run,
            "validate": true,
            "reload": reload,
        });
        let response = self
            .send_request(reqwest::Method::PUT, url, Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn memory_overview(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/memories/overview").await
    }

    pub async fn get_memory(&self, name: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/memories");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(name);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn delete_memory(&self, name: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/memories");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(name);
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn ensure_memory_index(&self) -> Result<serde_json::Value, ClientError> {
        self.post_json("/api/v1/memories/index", serde_json::json!({}))
            .await
    }

    pub async fn operator_tools(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/operator/tools").await
    }

    pub async fn invoke_operator_tool(
        &self,
        workspace_id: i64,
        tool: &str,
        input: Option<serde_json::Value>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/operator/tools/invoke",
            serde_json::json!({
                "workspace_id": workspace_id,
                "tool": tool,
                "input": input
            }),
        )
        .await
    }

    pub async fn create_git_commit(
        &self,
        message: String,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/git/commits",
            serde_json::json!({ "message": message }),
        )
        .await
    }

    pub async fn create_git_pull_request(
        &self,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/git/pull-requests",
            serde_json::json!({
                "title": title,
                "body": body,
                "base": base,
                "head": head,
            }),
        )
        .await
    }

    pub async fn compact_session(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/compact"),
            serde_json::to_value(options)?,
        )
        .await
    }

    pub async fn update_session_selection(
        &self,
        session_id: i64,
        options: RunOptions,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.put_json(
            &format!("/api/v1/sessions/{session_id}/selection"),
            serde_json::to_value(options)?,
        )
        .await
    }

    pub async fn get_session_state(
        &self,
        session_id: i64,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.get_json(&format!("/api/v1/sessions/{session_id}/state"))
            .await
    }

    pub async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionCostSummary, ClientError> {
        self.get_json(&format!("/api/v1/sessions/{session_id}/cost"))
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn usage_stats(
        &self,
        period: agena_domain::UsagePeriod,
        from: Option<chrono::DateTime<chrono::Utc>>,
        to: Option<chrono::DateTime<chrono::Utc>>,
        provider_ids: &[String],
        model_ids: &[String],
        session_ids: &[i64],
        include_subagents: bool,
        timezone_offset_minutes: i32,
    ) -> Result<agena_domain::UsageStats, ClientError> {
        let mut url = self.endpoint("/api/v1/usage");
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("period", period.label());
            if let Some(from) = from {
                query.append_pair("from", from.to_rfc3339().as_str());
            }
            if let Some(to) = to {
                query.append_pair("to", to.to_rfc3339().as_str());
            }
            if !provider_ids.is_empty() {
                query.append_pair("provider", provider_ids.join(",").as_str());
            }
            if !model_ids.is_empty() {
                query.append_pair("model", model_ids.join(",").as_str());
            }
            if !session_ids.is_empty() {
                query.append_pair(
                    "session",
                    session_ids
                        .iter()
                        .map(i64::to_string)
                        .collect::<Vec<_>>()
                        .join(",")
                        .as_str(),
                );
            }
            query.append_pair("include_subagents", &include_subagents.to_string());
            query.append_pair(
                "timezone_offset_minutes",
                &timezone_offset_minutes.to_string(),
            );
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn session_overview(
        &self,
        workspace_id: Option<i64>,
        recent_limit: u64,
    ) -> Result<SessionOverviewResource, ClientError> {
        let mut url = self.endpoint("/api/v1/sessions/overview");
        {
            let mut query = url.query_pairs_mut();
            if let Some(workspace_id) = workspace_id {
                query.append_pair("workspace_id", &workspace_id.to_string());
            }
            query.append_pair("recent_limit", &recent_limit.to_string());
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn session_parts(
        &self,
        session_id: i64,
    ) -> Result<agena_api::live::SessionPartsResource, ClientError> {
        self.get_json(&format!("/api/v1/sessions/{session_id}/parts"))
            .await
    }

    pub async fn connect_session(&self, session_id: i64) -> Result<SessionConnection, ClientError> {
        let subscription = self
            .stream_changes(agena_api::Scope::Session { session_id })
            .await?;
        match self.get_session_state(session_id).await {
            Ok(snapshot) => Ok(SessionConnection {
                snapshot,
                subscription,
            }),
            Err(error) => {
                drop(subscription);
                Err(error)
            }
        }
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::CancellationResult, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/cancel"),
            serde_json::json!({ "execution_id": execution_id }),
        )
        .await
    }

    pub async fn reply_permission(
        &self,
        params: ReplyPermissionParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert("reply".to_string(), serde_json::to_value(params.reply)?);
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/permission-replies", params.session_id),
            body,
        )
        .await
    }

    pub async fn reply_user_input(
        &self,
        params: ReplyUserInputParams,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut body = serde_json::to_value(params.options)?;
        if let serde_json::Value::Object(ref mut object) = body {
            object.insert("reply".to_string(), serde_json::to_value(params.reply)?);
        }
        self.post_json(
            &format!("/api/v1/sessions/{}/user-input-replies", params.session_id),
            body,
        )
        .await
    }

    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: &str,
    ) -> Result<SessionExecutionResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/interactive"));
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("center URL cannot carry path segments".into()))?
            .push(request_id)
            .push("present");
        let body = serde_json::json!({});
        let response = self
            .send_request(reqwest::Method::POST, url, Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn stream_changes(
        &self,
        scope: agena_api::Scope,
    ) -> Result<NotificationSubscription, ClientError> {
        let url = self.changes_stream_url(&scope);
        let response = self
            .send_request(reqwest::Method::GET, url, None, Some("text/event-stream"))
            .await?;
        let status = response.status();
        if !status.is_success() {
            let body = read_response_text_bounded(
                response,
                MAX_ERROR_RESPONSE_BYTES,
                "notification stream error response",
            )
            .await?;
            if let Ok(api) = serde_json::from_str::<agena_api::error::ApiError>(&body) {
                return Err(ClientError::Api(api));
            }
            return Err(ClientError::Transport(format!(
                "notification stream request failed ({status}): {}",
                body.trim()
            )));
        }

        let (tx, rx) = mpsc::channel(256);
        let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
        let mut frames = FramedRead::new(
            reader,
            SseDecoder::<String>::with_max_size(MAX_SSE_EVENT_BYTES),
        );
        let task = tokio::spawn(async move {
            while let Some(frame) = frames.next().await {
                let event = match frame {
                    Ok(SseFrame::Event(event)) => event,
                    Ok(SseFrame::Comment(_) | SseFrame::Retry(_)) => continue,
                    Err(error) => {
                        let _ = tx
                            .send(Err(ClientError::Protocol(format!(
                                "invalid or oversized notification event: {error}"
                            ))))
                            .await;
                        return;
                    }
                };
                if event.name != "notification" || event.data.trim().is_empty() {
                    continue;
                }
                let notification = match serde_json::from_str::<Notification>(&event.data) {
                    Ok(notification) => notification,
                    Err(error) => {
                        let _ = tx.send(Err(ClientError::Decode(error))).await;
                        return;
                    }
                };
                if !Self::send_notification_frame(&tx, notification).await {
                    return;
                }
            }
        });

        Ok(NotificationSubscription {
            rx,
            task: Some(task),
        })
    }

    /// Escape hatch: run any [`Command`] over REST where a dedicated route
    /// already exists.
    pub async fn command(&self, cmd: Command) -> Result<CommandResult, ClientError> {
        match cmd {
            Command::CreateWorkspace(CreateWorkspaceParams { path }) => {
                Ok(CommandResult::Workspace(
                    self.post_json("/api/v1/workspaces", serde_json::json!({ "path": path }))
                        .await?,
                ))
            }
            Command::UpdateWorkspace(UpdateWorkspaceParams {
                workspace_id, path, ..
            }) => Ok(CommandResult::Workspace(
                self.put_json(
                    &format!("/api/v1/workspaces/{workspace_id}"),
                    serde_json::json!({ "path": path }),
                )
                .await?,
            )),
            Command::DeleteWorkspace(DeleteWorkspaceParams { workspace_id }) => {
                let _: WorkspaceResource = self
                    .delete_json(&format!("/api/v1/workspaces/{workspace_id}"))
                    .await?;
                Ok(CommandResult::WorkspaceDeleted { id: workspace_id })
            }
            Command::ResolveWorkspace(ResolveWorkspaceParams {
                path,
                create_if_missing,
            }) => Ok(CommandResult::Workspace(
                self.post_json(
                    "/api/v1/workspaces/resolve",
                    serde_json::json!({
                        "path": path,
                        "create_if_missing": create_if_missing,
                    }),
                )
                .await?,
            )),
            Command::CreateSession(CreateSessionParams {
                workspace_id,
                title,
                parent_id,
            }) => Ok(CommandResult::Session(
                self.create_session(workspace_id, title, parent_id).await?,
            )),
            Command::UpdateSession(UpdateSessionParams {
                session_id, title, ..
            }) => Ok(CommandResult::Session(
                self.put_json(
                    &format!("/api/v1/sessions/{session_id}"),
                    serde_json::json!({
                        "title": title,
                    }),
                )
                .await?,
            )),
            Command::DeleteSession(DeleteSessionParams { session_id, .. }) => {
                let _: SessionResource = self
                    .delete_json(&format!("/api/v1/sessions/{session_id}"))
                    .await?;
                Ok(CommandResult::SessionDeleted { id: session_id })
            }
            Command::SubmitMessage(params) => {
                Ok(CommandResult::Execution(self.submit_message(params).await?))
            }
            Command::ContinueRun(ContinueRunParams {
                session_id,
                options,
            }) => Ok(CommandResult::Execution(
                self.continue_run(session_id, options).await?,
            )),
            Command::CancelRun(CancelRunParams {
                session_id,
                execution_id,
            }) => {
                let result = self.cancel_run(session_id, execution_id).await?;
                Ok(CommandResult::Cancellation(result))
            }
            Command::RewindSession(RewindSessionParams {
                session_id,
                turn_id,
                ..
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/rewind"),
                    serde_json::json!({ "turn_id": turn_id }),
                )
                .await?,
            )),
            Command::ForkSession(ForkSessionParams {
                session_id,
                at_message_id,
                title,
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/fork"),
                    serde_json::json!({
                        "at_message_id": at_message_id,
                        "title": title,
                    }),
                )
                .await?,
            )),
            Command::ListSessionTree(ListSessionTreeParams { root_id }) => {
                Ok(CommandResult::SessionTree(
                    self.get_json(&format!("/api/v1/sessions/tree/{root_id}"))
                        .await?,
                ))
            }
            Command::ExportSession(ExportSessionParams { session_id }) => {
                Ok(CommandResult::SessionExport {
                    jsonl: self
                        .get_text(&format!("/api/v1/sessions/{session_id}/export"))
                        .await?,
                })
            }
            Command::ImportSession(ImportSessionParams { jsonl }) => Ok(CommandResult::Execution(
                self.post_json(
                    "/api/v1/sessions/import",
                    serde_json::json!({ "jsonl": jsonl }),
                )
                .await?,
            )),
            Command::ReplyPermission(params) => Ok(CommandResult::Execution(
                self.reply_permission(params).await?,
            )),
            Command::ReplyUserInput(params) => Ok(CommandResult::Execution(
                self.reply_user_input(params).await?,
            )),
            Command::UpsertPermissionRule(UpsertPermissionRuleParams {
                action_key,
                subject_kind,
                tool_name,
                qualifier,
                path_access_kind,
                workspace_root,
                target_path,
                network_target,
                network_host,
                network_port,
                scope,
                session_id,
                mode,
            }) => {
                let mut body = serde_json::Map::new();
                if let Some(action_key) = action_key {
                    body.insert("action_key".to_string(), serde_json::json!(action_key));
                }
                if let Some(subject_kind) = subject_kind {
                    body.insert("subject_kind".to_string(), serde_json::json!(subject_kind));
                }
                if let Some(tool_name) = tool_name {
                    body.insert("tool_name".to_string(), serde_json::json!(tool_name));
                }
                if let Some(qualifier) = qualifier {
                    body.insert("qualifier".to_string(), serde_json::json!(qualifier));
                }
                if let Some(path_access_kind) = path_access_kind {
                    body.insert(
                        "path_access_kind".to_string(),
                        serde_json::json!(path_access_kind),
                    );
                }
                if let Some(workspace_root) = workspace_root {
                    body.insert(
                        "workspace_root".to_string(),
                        serde_json::json!(workspace_root),
                    );
                }
                if let Some(target_path) = target_path {
                    body.insert("target_path".to_string(), serde_json::json!(target_path));
                }
                if let Some(network_target) = network_target {
                    body.insert(
                        "network_target".to_string(),
                        serde_json::json!(network_target),
                    );
                }
                if let Some(network_host) = network_host {
                    body.insert("network_host".to_string(), serde_json::json!(network_host));
                }
                if let Some(network_port) = network_port {
                    body.insert("network_port".to_string(), serde_json::json!(network_port));
                }
                if let Some(scope) = scope {
                    body.insert("scope".to_string(), serde_json::json!(scope));
                }
                if let Some(session_id) = session_id {
                    body.insert("session_id".to_string(), serde_json::json!(session_id));
                }
                body.insert("mode".to_string(), serde_json::json!(mode));
                Ok(CommandResult::PermissionRule(
                    self.post_json("/api/v1/permission-rules", serde_json::Value::Object(body))
                        .await?,
                ))
            }
            Command::ReplacePermissionRule(ReplacePermissionRuleParams { rule_id, rule }) => {
                let UpsertPermissionRuleParams {
                    action_key,
                    subject_kind,
                    tool_name,
                    qualifier,
                    path_access_kind,
                    workspace_root,
                    target_path,
                    network_target,
                    network_host,
                    network_port,
                    scope,
                    session_id,
                    mode,
                } = rule;
                let mut body = serde_json::Map::new();
                if let Some(action_key) = action_key {
                    body.insert("action_key".to_string(), serde_json::json!(action_key));
                }
                if let Some(subject_kind) = subject_kind {
                    body.insert("subject_kind".to_string(), serde_json::json!(subject_kind));
                }
                if let Some(tool_name) = tool_name {
                    body.insert("tool_name".to_string(), serde_json::json!(tool_name));
                }
                if let Some(qualifier) = qualifier {
                    body.insert("qualifier".to_string(), serde_json::json!(qualifier));
                }
                if let Some(path_access_kind) = path_access_kind {
                    body.insert(
                        "path_access_kind".to_string(),
                        serde_json::json!(path_access_kind),
                    );
                }
                if let Some(workspace_root) = workspace_root {
                    body.insert(
                        "workspace_root".to_string(),
                        serde_json::json!(workspace_root),
                    );
                }
                if let Some(target_path) = target_path {
                    body.insert("target_path".to_string(), serde_json::json!(target_path));
                }
                if let Some(network_target) = network_target {
                    body.insert(
                        "network_target".to_string(),
                        serde_json::json!(network_target),
                    );
                }
                if let Some(network_host) = network_host {
                    body.insert("network_host".to_string(), serde_json::json!(network_host));
                }
                if let Some(network_port) = network_port {
                    body.insert("network_port".to_string(), serde_json::json!(network_port));
                }
                if let Some(scope) = scope {
                    body.insert("scope".to_string(), serde_json::json!(scope));
                }
                if let Some(session_id) = session_id {
                    body.insert("session_id".to_string(), serde_json::json!(session_id));
                }
                body.insert("mode".to_string(), serde_json::json!(mode));
                Ok(CommandResult::PermissionRule(
                    self.put_json(
                        &format!("/api/v1/permission-rules/{rule_id}"),
                        serde_json::Value::Object(body),
                    )
                    .await?,
                ))
            }
            Command::RevokePermissionRule(RevokePermissionRuleParams { rule_id, reason }) => {
                Ok(CommandResult::PermissionRule(
                    self.post_json(
                        &format!("/api/v1/permission-rules/{rule_id}/revoke"),
                        serde_json::json!({ "reason": reason }),
                    )
                    .await?,
                ))
            }
            Command::DeletePermissionRule(DeletePermissionRuleParams { rule_id }) => {
                let _: PermissionRuleResource = self
                    .delete_json(&format!("/api/v1/permission-rules/{rule_id}"))
                    .await?;
                Ok(CommandResult::PermissionRuleDeleted { id: rule_id })
            }
            Command::StopActivity(StopActivityParams { activity_id }) => {
                Ok(CommandResult::Activity(
                    self.post_json(
                        &format!("/api/v1/activities/{activity_id}/stop"),
                        serde_json::json!({}),
                    )
                    .await?,
                ))
            }
            Command::DismissActivity(DismissActivityParams { activity_id }) => {
                let _: BackgroundActivityResource = self
                    .post_json(
                        &format!("/api/v1/activities/{activity_id}/dismiss"),
                        serde_json::json!({}),
                    )
                    .await?;
                Ok(CommandResult::ActivityDeleted { id: activity_id })
            }
            Command::ClearFinishedActivities => {
                let count: usize = self
                    .post_json("/api/v1/activities/clear-finished", serde_json::json!({}))
                    .await?;
                Ok(CommandResult::ActivitiesCleared { count })
            }
            _ => Err(ClientError::Protocol(
                "unsupported command in generic HTTP client".to_string(),
            )),
        }
    }

    /// Generic query escape hatch over the existing REST surface.
    pub async fn query(&self, q: Query) -> Result<QueryResult, ClientError> {
        match q {
            Query::Health => Ok(QueryResult::Health(self.health().await?)),
            Query::Runtime => Ok(QueryResult::Runtime(
                self.get_json("/api/v1/runtime").await?,
            )),
            Query::ListProviders => Ok(QueryResult::Providers(
                self.get_json("/api/v1/providers").await?,
            )),
            Query::ListProviderModels(ListProviderModelsParams { provider_id }) => {
                Ok(QueryResult::ProviderModels(
                    self.get_json(&format!("/api/v1/providers/{provider_id}/models"))
                        .await?,
                ))
            }
            Query::ListProviderAdapterModels(ListProviderAdapterModelsParams {
                provider_id,
                base_url,
                protocol_paths,
                api_key,
                adapter_ids,
            }) => Ok(QueryResult::ProviderAdapterModels(
                self.list_provider_adapter_models(ProviderAdapterModelsRequest {
                    provider_id,
                    base_url,
                    protocol_paths,
                    api_key,
                    adapter_ids,
                })
                .await?,
            )),
            Query::ListSavedProviderAdapterModels(ListSavedProviderAdapterModelsParams {
                provider_id,
                adapter_ids,
            }) => Ok(QueryResult::ProviderAdapterModels(
                self.list_saved_provider_adapter_models(
                    provider_id.as_str(),
                    SavedProviderAdapterModelsRequest { adapter_ids },
                )
                .await?,
            )),
            Query::ListWorkspaces(ListWorkspacesParams {
                cursor,
                limit,
                search,
                include_session_count,
            }) => {
                let mut url = self.endpoint("/api/v1/workspaces");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                    if include_session_count {
                        q.append_pair("include_session_count", "true");
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::Workspaces(self.parse_json(response).await?))
            }
            Query::GetWorkspace(GetWorkspaceParams { workspace_id }) => Ok(QueryResult::Workspace(
                self.get_json(&format!("/api/v1/workspaces/{workspace_id}"))
                    .await?,
            )),
            Query::ListSessions(ListSessionsParams {
                cursor,
                limit,
                workspace_id,
                parent_id,
                roots,
                exclude_subagents,
                search,
            }) => {
                let mut url = self.endpoint("/api/v1/sessions");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(workspace_id) = workspace_id {
                        q.append_pair("workspace_id", &workspace_id.to_string());
                    }
                    if let Some(parent_id) = parent_id {
                        q.append_pair("parent_id", &parent_id.to_string());
                    }
                    if roots {
                        q.append_pair("roots", "true");
                    }
                    if exclude_subagents {
                        q.append_pair("exclude_subagents", "true");
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::Sessions(self.parse_json(response).await?))
            }
            Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
                self.get_json(&format!("/api/v1/sessions/{session_id}"))
                    .await?,
            )),
            Query::GetSessionState(GetSessionParams { session_id }) => {
                Ok(QueryResult::SessionState(
                    self.get_json(&format!("/api/v1/sessions/{session_id}/state"))
                        .await?,
                ))
            }
            Query::GetOperationDetail(GetOperationDetailParams {
                session_id,
                activity_id,
            }) => Ok(QueryResult::OperationDetail(
                self.get_json(&format!(
                    "/api/v1/sessions/{session_id}/operations/{activity_id}/detail"
                ))
                .await?,
            )),
            Query::ListPermissionRules(ListPermissionRulesParams {
                cursor,
                limit,
                search,
            }) => {
                let mut url = self.endpoint("/api/v1/permission-rules");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(cursor) = cursor {
                        q.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if let Some(search) = search.filter(|search| !search.is_empty()) {
                        q.append_pair("search", &search);
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::PermissionRules(
                    self.parse_json(response).await?,
                ))
            }
            Query::GetPermissionRule(GetPermissionRuleParams { rule_id }) => {
                Ok(QueryResult::PermissionRule(
                    self.get_json(&format!("/api/v1/permission-rules/{rule_id}"))
                        .await?,
                ))
            }
            Query::ListActivities(params) => {
                let mut url = self.endpoint("/api/v1/activities");
                {
                    let mut q = url.query_pairs_mut();
                    if let Some(kinds) = params.kinds.filter(|kinds| !kinds.is_empty()) {
                        q.append_pair("kinds", &kinds);
                    }
                    if let Some(statuses) = params.statuses.filter(|statuses| !statuses.is_empty())
                    {
                        q.append_pair("statuses", &statuses);
                    }
                    if let Some(session_id) = params.session_id {
                        q.append_pair("session_id", &session_id.to_string());
                    }
                    if params.active_only {
                        q.append_pair("active_only", "true");
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::Activities(self.parse_json(response).await?))
            }
            Query::GetActivity(GetActivityParams { activity_id }) => Ok(QueryResult::Activity(
                self.get_json(&format!("/api/v1/activities/{activity_id}"))
                    .await?,
            )),
            Query::ActivityLogs(ActivityLogsParams {
                activity_id,
                since_seq,
                limit,
                wait_ms,
            }) => {
                let mut url = self.endpoint(&format!("/api/v1/activities/{activity_id}/logs"));
                {
                    let mut q = url.query_pairs_mut();
                    if since_seq > 0 {
                        q.append_pair("since_seq", &since_seq.to_string());
                    }
                    if let Some(limit) = limit {
                        q.append_pair("limit", &limit.to_string());
                    }
                    if wait_ms > 0 {
                        q.append_pair("wait_ms", &wait_ms.to_string());
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::ActivityLogs(self.parse_json(response).await?))
            }
        }
    }
}

#[cfg(test)]
mod sse_contract_tests {
    use super::AgenaClient;
    use axum::{
        Json, Router,
        extract::State,
        http::{HeaderMap, StatusCode, header},
        response::{IntoResponse, Response},
        routing::{get, post},
    };
    use futures_util::StreamExt as _;
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;
    use tokio_sse_codec::{Frame as SseFrame, SseDecoder};
    use tokio_util::codec::FramedRead;

    fn fixture(name: &str) -> String {
        std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures")
                .join(name),
        )
        .expect("checked-in client fixture must be readable")
    }

    #[derive(Clone, Default)]
    struct RefreshAuthFixture {
        login_count: Arc<AtomicUsize>,
        minimum_valid_generation: Arc<AtomicUsize>,
        stale_unauthorized_count: Arc<AtomicUsize>,
        login_authorization_seen: Arc<AtomicBool>,
        accepted_mutations: Arc<AtomicUsize>,
    }

    fn bearer_generation(headers: &HeaderMap) -> Option<usize> {
        headers
            .get(header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer token-"))
            .and_then(|value| value.parse().ok())
    }

    async fn refresh_health() -> Json<serde_json::Value> {
        Json(serde_json::json!({
            "status": "ok",
            "generation": 1,
            "loaded_at": "2026-08-16T00:00:00Z",
            "database_connected": true,
            "center": {
                "id": "b6cb9914-e388-4e90-8b40-9be12e65ecdb",
                "pid": 4242,
                "started_at": "2026-08-16T00:00:00Z",
                "protocol_version": agena_api::PROTOCOL_VERSION,
            }
        }))
    }

    async fn refresh_login(
        State(state): State<RefreshAuthFixture>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        if headers.contains_key(header::AUTHORIZATION) {
            state
                .login_authorization_seen
                .store(true, Ordering::Release);
        }
        if body.get("password").and_then(serde_json::Value::as_str)
            != Some("refresh-password-secret")
        {
            return (
                StatusCode::UNAUTHORIZED,
                Json(serde_json::json!({
                    "error": "Invalid password",
                    "locked": true,
                    "code": "auth_invalid_password"
                })),
            )
                .into_response();
        }
        let generation = state.login_count.fetch_add(1, Ordering::AcqRel) + 1;
        Json(serde_json::json!({
            "authenticated": true,
            "disabled": false,
            "token": format!("token-{generation}")
        }))
        .into_response()
    }

    async fn refresh_authorized(
        state: &RefreshAuthFixture,
        headers: &HeaderMap,
    ) -> Result<(), Response> {
        let generation = bearer_generation(headers).unwrap_or_default();
        if generation >= state.minimum_valid_generation.load(Ordering::Acquire) {
            return Ok(());
        }
        // Keep stale requests in flight together so the test exercises the
        // refresh mutex rather than merely making sequential requests.
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
        state
            .stale_unauthorized_count
            .fetch_add(1, Ordering::AcqRel);
        Err((
            StatusCode::UNAUTHORIZED,
            Json(serde_json::json!({
                "error": "UI authentication required",
                "locked": true,
                "code": "auth_required"
            })),
        )
            .into_response())
    }

    async fn refresh_protected(
        State(state): State<RefreshAuthFixture>,
        headers: HeaderMap,
    ) -> Response {
        if let Err(response) = refresh_authorized(&state, &headers).await {
            return response;
        }
        Json(serde_json::json!({"ok": true})).into_response()
    }

    async fn refresh_sse(State(state): State<RefreshAuthFixture>, headers: HeaderMap) -> Response {
        if let Err(response) = refresh_authorized(&state, &headers).await {
            return response;
        }
        (
            [(header::CONTENT_TYPE, "text/event-stream")],
            "event: keepalive\ndata: ready\n\n",
        )
            .into_response()
    }

    async fn refresh_mutation(
        State(state): State<RefreshAuthFixture>,
        headers: HeaderMap,
        Json(body): Json<serde_json::Value>,
    ) -> Response {
        if let Err(response) = refresh_authorized(&state, &headers).await {
            return response;
        }
        state.accepted_mutations.fetch_add(1, Ordering::AcqRel);
        Json(serde_json::json!({"accepted": body})).into_response()
    }

    async fn spawn_refresh_auth_fixture()
    -> (String, RefreshAuthFixture, tokio::task::JoinHandle<()>) {
        let state = RefreshAuthFixture::default();
        state.minimum_valid_generation.store(2, Ordering::Release);
        let router = Router::new()
            .route("/api/v1/health", get(refresh_health))
            .route("/auth/session", post(refresh_login))
            .route("/protected", get(refresh_protected))
            .route("/api/v1/changes/stream", get(refresh_sse))
            .route("/mutation", post(refresh_mutation))
            .with_state(state.clone());
        let listener = TcpListener::bind(("127.0.0.1", 0))
            .await
            .expect("bind refresh-auth fixture");
        let address = listener.local_addr().expect("refresh-auth fixture address");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve refresh-auth fixture");
        });
        (format!("http://{address}"), state, server)
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn password_client_refreshes_one_token_for_concurrent_requests_and_sse_reconnect() {
        let (url, state, server) = spawn_refresh_auth_fixture().await;
        let client =
            AgenaClient::connect_center(url.as_str(), None, Some("refresh-password-secret"))
                .await
                .expect("connect password-authenticated center client");
        assert_eq!(state.login_count.load(Ordering::Acquire), 1);

        let debug = format!("{client:?}");
        assert!(debug.contains("password-refreshable"));
        assert!(!debug.contains("refresh-password-secret"));
        assert!(!debug.contains("token-1"));

        let mut requests = Vec::new();
        for _ in 0..12 {
            let client = client.clone();
            requests.push(tokio::spawn(async move {
                client
                    .get_json::<serde_json::Value>("/protected")
                    .await
                    .expect("protected request refreshes and succeeds")
            }));
        }
        for request in requests {
            assert_eq!(request.await.expect("join protected request")["ok"], true);
        }
        assert!(
            state.stale_unauthorized_count.load(Ordering::Acquire) > 1,
            "multiple clones must have observed the same expired token"
        );
        assert_eq!(
            state.login_count.load(Ordering::Acquire),
            2,
            "one shared refresh must satisfy every concurrent clone"
        );

        // Model a later center restart/session expiry. SSE reconnect uses the
        // same authenticated request path and must obtain token-3 once.
        state.minimum_valid_generation.store(3, Ordering::Release);
        let subscription = client
            .stream_changes(agena_api::Scope::Global)
            .await
            .expect("SSE handshake refreshes after token invalidation");
        assert_eq!(state.login_count.load(Ordering::Acquire), 3);
        drop(subscription);

        state.minimum_valid_generation.store(4, Ordering::Release);
        let mutation = client
            .post_json::<serde_json::Value>("/mutation", serde_json::json!({"value": 7}))
            .await
            .expect("401-rejected mutation refreshes and replays once");
        assert_eq!(mutation["accepted"]["value"], 7);
        assert_eq!(state.login_count.load(Ordering::Acquire), 4);
        assert_eq!(
            state.accepted_mutations.load(Ordering::Acquire),
            1,
            "authentication retry must not duplicate an accepted mutation"
        );
        assert!(
            !state.login_authorization_seen.load(Ordering::Acquire),
            "password exchange must not carry the stale bearer token"
        );

        server.abort();
    }

    #[tokio::test]
    async fn static_bearer_is_never_reinterpreted_as_a_refreshable_password() {
        let (url, state, server) = spawn_refresh_auth_fixture().await;
        let client = AgenaClient::connect_center(url.as_str(), Some("token-1"), None)
            .await
            .expect("connect static-bearer client");
        let debug = format!("{client:?}");
        assert!(debug.contains("static-bearer"));
        assert!(!debug.contains("token-1"));

        client
            .get_json::<serde_json::Value>("/protected")
            .await
            .expect_err("expired static bearer must remain an authentication error");
        assert_eq!(
            state.login_count.load(Ordering::Acquire),
            0,
            "a bearer token is not a password and must never trigger login"
        );
        server.abort();
    }

    #[tokio::test]
    async fn bounded_sse_codec_normalizes_crlf_and_multiline_data() {
        let source =
            &b"event: notification\r\ndata: {\"seq\":1}\r\ndata: {\"kind\":\"x\"}\r\n\r\n"[..];
        let mut frames = FramedRead::new(source, SseDecoder::<String>::with_max_size(1024));
        let frame = frames
            .next()
            .await
            .expect("one SSE frame")
            .expect("valid SSE frame");
        let SseFrame::Event(event) = frame else {
            panic!("expected event frame");
        };
        assert_eq!(event.name, "notification");
        assert_eq!(event.data, "{\"seq\":1}\n{\"kind\":\"x\"}");
    }

    #[tokio::test]
    async fn bounded_sse_codec_rejects_an_oversized_event() {
        let source = format!("event: notification\ndata: {}\n\n", "x".repeat(2_048));
        let mut frames = FramedRead::new(
            source.as_bytes(),
            SseDecoder::<String>::with_max_size(1_024),
        );

        assert!(
            frames
                .next()
                .await
                .expect("oversized event must produce a decoder result")
                .is_err()
        );
    }

    #[tokio::test]
    async fn health_round_trips_through_a_real_http_transport() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = fixture("health-response.json");
        let fixture: agena_api::resource::HealthResponse =
            serde_json::from_str(&response_body).expect("health fixture matches API resource");
        assert_eq!(fixture.status, "ok");
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request = String::from_utf8(request).unwrap();
            assert!(request.starts_with("GET /api/v1/health HTTP/1.1"));
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let health = client.health().await.unwrap();
        assert_eq!(health.status, "ok");
        assert_eq!(health.generation, 7);
        assert_eq!(health.loaded_at.to_rfc3339(), "2026-01-02T03:04:05+00:00");
        assert!(health.database_connected);
        assert!(
            health.center.is_none(),
            "legacy health fixtures remain compatible"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn center_identity_validates_the_protocol_handshake() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let center_id = uuid::Uuid::parse_str("9fe2e22b-e2b8-4e2a-92c7-40e71da2015c").unwrap();
        let response_body = serde_json::json!({
            "status": "ok",
            "generation": 3,
            "loaded_at": "2026-08-15T01:02:03Z",
            "database_connected": true,
            "center": {
                "id": center_id,
                "pid": 4242,
                "started_at": "2026-08-15T00:00:00Z",
                "protocol_version": agena_api::PROTOCOL_VERSION,
            }
        })
        .to_string();
        let response = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
            response_body.len(),
            response_body
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0_u8; 1024];
            loop {
                let read = stream.read(&mut buffer).await.unwrap();
                if read == 0 {
                    break;
                }
                request.extend_from_slice(&buffer[..read]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let center = client.center_identity().await.unwrap();
        assert_eq!(center.id, center_id);
        assert_eq!(center.pid, 4242);
        assert_eq!(center.protocol_version, agena_api::PROTOCOL_VERSION);
        server.await.unwrap();
    }

    #[tokio::test]
    async fn non_success_json_maps_to_the_shared_api_error() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = fixture("api-error-not-found.json");
        let fixture: agena_api::ApiError =
            serde_json::from_str(&response_body).expect("error fixture matches API error");
        assert_eq!(
            fixture.problem.category,
            agena_failure::FailureCategory::NotFound
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let response = format!(
                "HTTP/1.1 404 Not Found\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let error = client.health().await.expect_err("404 must be an API error");
        match error {
            crate::ClientError::Api(api) => {
                assert_eq!(
                    api.problem.category,
                    agena_failure::FailureCategory::NotFound
                );
                assert_eq!(api.problem.user.fallback, "workspace missing");
            }
            other => panic!("expected shared API error, got {other:?}"),
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn legacy_ui_auth_errors_map_to_a_safe_actionable_problem() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        let response_body = serde_json::json!({
            "error": "UI authentication required",
            "locked": true,
            "code": "auth_required"
        })
        .to_string();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let response = format!(
                "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(),
                response_body
            );
            stream.write_all(response.as_bytes()).await.unwrap();
        });

        let client = AgenaClient::new(format!("http://{address}")).unwrap();
        let error = client
            .health()
            .await
            .expect_err("protected endpoint must require auth");
        match error {
            crate::ClientError::Api(api) => {
                assert!(api.problem.user.fallback.contains("AGENA_CENTER_PASSWORD"))
            }
            other => panic!("expected safe auth API error, got {other:?}"),
        }
        server.await.unwrap();
    }

    #[tokio::test]
    async fn center_connection_rejects_two_authentication_mechanisms() {
        let error =
            AgenaClient::connect_center("http://127.0.0.1:9", Some("token"), Some("password"))
                .await
                .expect_err("ambiguous auth must fail before connecting");
        assert!(matches!(error, crate::ClientError::Protocol(_)));
    }
}
