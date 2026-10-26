//! REST client. Wraps `reqwest` and serializes [`agena_api`] commands/queries
//! into the current endpoints.

use portable_atomic::{AtomicBool, AtomicU64};
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    fmt,
    sync::{Arc, RwLock, atomic::Ordering},
    time::{Duration, Instant},
};

use agena_api::{
    commands::{
        CancelRunParams, Command, CommandResult, ContinueRunParams, CreateSessionParams,
        CreateWorkspaceParams, DeletePermissionRuleParams, DeleteSessionParams,
        DeleteWorkspaceParams, DismissActivityParams, ForkSessionParams, ListSessionTreeParams,
        ReplacePermissionRuleParams, ReplyPermissionParams, ReplyUserInputParams,
        ResolveWorkspaceParams, RevokePermissionRuleParams, RewindSessionParams,
        StopActivityParams, SubmitRunParams, UpdateSessionParams, UpdateWorkspaceParams,
        UpsertPermissionRuleParams,
    },
    notifications::Notification,
    queries::{
        ActivityLogsParams, GetActivityParams, GetPermissionRuleParams, GetSessionParams,
        GetWorkspaceParams, ListPermissionRulesParams, ListProviderAdapterModelsParams,
        ListProviderModelsParams, ListSavedProviderAdapterModelsParams, ListSessionsParams,
        ListWorkspacesParams, Query, QueryResult,
    },
    resource::{
        BackgroundActivityResource, HealthResponse, PermissionRuleResource,
        ProviderAdapterModelsRequest, ProviderAdapterModelsResource, ProviderAdapterModelsResponse,
        RunOptions, SavedProviderAdapterModelsRequest, SessionExecutionResource,
        SessionOverviewResource, SessionResource, WorkspaceResource,
    },
};
use futures_util::{StreamExt, TryStreamExt as _};
use secrecy::{ExposeSecret as _, SecretString};
use tokio::{sync::mpsc, task::JoinHandle};
use tokio_sse_codec::{Frame as SseFrame, SseDecoder};
use tokio_util::{codec::FramedRead, io::StreamReader};

use crate::error::ClientError;
use crate::ws::SubscriptionEvent;

mod archive;
mod content;
#[cfg(test)]
mod tests;
pub use content::ContentSubscription;

const MAX_JSON_RESPONSE_BYTES: usize = 64 * 1024 * 1024;
const MAX_ERROR_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_BINARY_RESPONSE_BYTES: usize = 100 * 1024 * 1024;
const MAX_SSE_EVENT_BYTES: usize = 4 * 1024 * 1024;

fn is_local_server(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Ipv4(ip)) => ip.is_loopback() || ip.is_unspecified(),
        Some(url::Host::Ipv6(ip)) => {
            ip.is_loopback()
                || ip.is_unspecified()
                || ip.to_ipv4_mapped().is_some_and(|ip| ip.is_loopback())
        }
        Some(url::Host::Domain(host)) => {
            let host = host.trim_end_matches('.');
            host == "localhost" || host.ends_with(".localhost")
        }
        None => false,
    }
}

fn response_json(
    bytes: &[u8],
    status: reqwest::StatusCode,
    context: &str,
) -> Result<serde_json::Value, ClientError> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Err(ClientError::Protocol(format!(
            "{context} returned an empty response (HTTP {status}); expected JSON; check the server URL and proxy settings"
        )));
    }
    serde_json::from_slice(bytes).map_err(|error| {
        ClientError::Protocol(format!(
            "{context} returned invalid JSON (HTTP {status}): {error}"
        ))
    })
}

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
                "server password must not be empty".to_owned(),
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
            .expect("server bearer lock poisoned")
            .clone()
    }

    fn replace_bearer(&self, token: &str) -> Result<(), ClientError> {
        let token = bearer_header(token)?;
        *self.bearer.write().expect("server bearer lock poisoned") = Some(token);
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
            "server bearer token must not be empty".to_owned(),
        ));
    }
    let mut authorization = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
        .map_err(|_| {
            ClientError::Protocol(
                "server bearer token contains invalid header characters".to_owned(),
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
    revision_lease: Arc<GlobalRevisionLease>,
}

pub struct BtwSubscription {
    rx: mpsc::Receiver<Result<agena_api::resource::BtwAnswer, ClientError>>,
    task: JoinHandle<()>,
}

impl BtwSubscription {
    pub async fn recv(&mut self) -> Option<Result<agena_api::resource::BtwAnswer, ClientError>> {
        self.rx.recv().await
    }
}

impl Drop for BtwSubscription {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// Snapshot-plus-live attachment to one server-owned session.
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
        self.revision_lease.release();
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

/// REST client with bounded shared read caches. Holds a `reqwest::Client` and the base URL like
/// `http://localhost:7878`.
#[derive(Clone)]
/// HTTP client for the Agena runtime API.
pub struct AgenaClient {
    base_url: url::Url,
    http: reqwest::Client,
    authentication: Arc<ClientAuthentication>,
    reads: Arc<tokio::sync::Mutex<ResourceReads>>,
    revision_check: Arc<tokio::sync::Mutex<()>>,
    read_limit: Arc<tokio::sync::Semaphore>,
}

#[derive(Clone)]
struct CachedRead {
    resource: String,
    token: String,
    etag: String,
    value: serde_json::Value,
    bytes: usize,
    used: u64,
    generation: u64,
}

#[derive(Default)]
struct ResourceReads {
    versions: HashMap<String, String>,
    bodies: HashMap<String, CachedRead>,
    locks: HashMap<String, std::sync::Weak<ResourceReadGate>>,
    checked_at: Option<Instant>,
    epoch: Option<String>,
    retired_epochs: VecDeque<String>,
    bytes: usize,
    sequence: u64,
    read_sequence: u64,
    global_live_users: Arc<AtomicU64>,
}

struct ResourceReadGate {
    lock: tokio::sync::Mutex<()>,
    requested: AtomicU64,
}

struct GlobalRevisionLease {
    users: Option<Arc<AtomicU64>>,
    active: AtomicBool,
}

impl GlobalRevisionLease {
    fn release(&self) {
        if self.active.swap(false, Ordering::AcqRel)
            && let Some(users) = &self.users
        {
            users.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for GlobalRevisionLease {
    fn drop(&mut self) {
        self.release();
    }
}

struct RevisionStreamGuard(Arc<GlobalRevisionLease>);

impl Drop for RevisionStreamGuard {
    fn drop(&mut self) {
        self.0.release();
    }
}

impl ResourceReads {
    fn observe(&mut self, key: &str, token: &str, expected: Option<&str>) -> bool {
        let Some((epoch, time)) = token.rsplit_once(':') else {
            return false;
        };
        let Ok(time) = time.parse::<i64>() else {
            return false;
        };
        if self.retired_epochs.iter().any(|retired| retired == epoch) {
            return false;
        }
        if self
            .epoch
            .as_deref()
            .is_some_and(|current| current != epoch)
        {
            if expected.is_some_and(|expected| {
                self.versions
                    .get(key)
                    .is_some_and(|current| expected != current)
            }) {
                return false;
            }
            if let Some(previous) = self.epoch.take() {
                self.retired_epochs.push_back(previous);
                if self.retired_epochs.len() > 16 {
                    self.retired_epochs.pop_front();
                }
            }
            self.versions.clear();
            self.checked_at = None;
        }
        self.epoch = Some(epoch.to_owned());
        if let Some(previous) = self.versions.get(key)
            && let Some((old_epoch, old_time)) = previous.rsplit_once(':')
            && old_epoch == epoch
            && old_time.parse::<i64>().unwrap_or(0) > time
        {
            return false;
        }
        if self.versions.len() >= 512 && !self.versions.contains_key(key) {
            self.versions.clear();
        }
        self.versions.insert(key.to_owned(), token.to_owned());
        true
    }

    fn touch(&mut self, path: &str) {
        self.sequence = self.sequence.saturating_add(1);
        if let Some(body) = self.bodies.get_mut(path) {
            body.used = self.sequence;
        }
    }

    fn forget(&mut self, path: &str) {
        if let Some(body) = self.bodies.remove(path) {
            self.bytes -= body.bytes;
        }
    }

    fn remember(&mut self, path: String, mut body: CachedRead) {
        self.forget(&path);
        self.sequence = self.sequence.saturating_add(1);
        body.used = self.sequence;
        if body.bytes > 2 * 1024 * 1024 {
            return;
        }
        self.bytes += body.bytes;
        self.bodies.insert(path, body);
        while self.bodies.len() > 64 || self.bytes > 4 * 1024 * 1024 {
            let Some(key) = self
                .bodies
                .iter()
                .min_by_key(|(_, body)| body.used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some(body) = self.bodies.remove(&key) {
                self.bytes -= body.bytes;
            }
        }
    }
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
        let mut http = reqwest::Client::builder().connect_timeout(Duration::from_secs(10));
        // Local control-plane traffic must reach this machine even when a
        // shell has HTTP(S)/ALL_PROXY set without NO_PROXY. Keep the normal
        // proxy behavior for explicitly configured remote Agena servers.
        if is_local_server(&base_url) {
            http = http.no_proxy();
        }
        Ok(Self {
            base_url,
            http: http.build()?,
            authentication: Arc::new(ClientAuthentication::anonymous()),
            reads: Arc::new(tokio::sync::Mutex::new(ResourceReads::default())),
            revision_check: Arc::new(tokio::sync::Mutex::new(())),
            read_limit: Arc::new(tokio::sync::Semaphore::new(4)),
        })
    }

    pub fn with_bearer_token(mut self, token: impl AsRef<str>) -> Result<Self, ClientError> {
        self.authentication = Arc::new(ClientAuthentication::static_bearer(token.as_ref())?);
        self.reads = Arc::new(tokio::sync::Mutex::new(ResourceReads::default()));
        Ok(self)
    }

    fn with_password_session(mut self, password: &str, token: &str) -> Result<Self, ClientError> {
        self.authentication = Arc::new(ClientAuthentication::password_session(password, token)?);
        self.reads = Arc::new(tokio::sync::Mutex::new(ResourceReads::default()));
        Ok(self)
    }

    /// Connect to a server, validate its public identity, and
    /// optionally authenticate using an ephemeral bearer token or server password.
    /// Password login exchanges the password for an in-memory session token;
    /// neither secret is written to endpoint discovery metadata.
    pub async fn connect_server(
        base_url: impl AsRef<str>,
        bearer_token: Option<&str>,
        password: Option<&str>,
    ) -> Result<Self, ClientError> {
        if bearer_token.is_some() && password.is_some() {
            return Err(ClientError::Protocol(
                "pass either a server token or password, not both".to_owned(),
            ));
        }
        let client = Self::new(base_url)?;
        client.server_identity().await?;
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
                "server password must not be empty".to_owned(),
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
            "server authentication response",
        )
        .await?;
        let value = response_json(body.as_bytes(), status, "server authentication")?;
        if !status.is_success() {
            if let Ok(api) = serde_json::from_value::<agena_api::error::ApiError>(value.clone()) {
                return Err(ClientError::Api(api));
            }
            return Err(ClientError::Protocol(format!(
                "HTTP {status} authentication error did not use the shared API envelope"
            )));
        }
        value
            .get("token")
            .and_then(serde_json::Value::as_str)
            .filter(|token| !token.trim().is_empty())
            .map(str::to_owned)
            .ok_or_else(|| {
                ClientError::Protocol("server authentication returned no bearer token".to_owned())
            })
    }

    fn request_builder(
        &self,
        method: reqwest::Method,
        url: url::Url,
        body: Option<&serde_json::Value>,
        accept: Option<&str>,
    ) -> reqwest::RequestBuilder {
        let bounded_read = method == reqwest::Method::GET
            && !matches!(accept, Some("text/event-stream" | "application/x-ndjson"));
        let mut request = self.http.request(method, url);
        // Ordinary snapshots must release in-flight UI guards even if a peer
        // stops sending. Streaming/long-running mutation requests have their
        // own lifecycle instead of this total timeout.
        if bounded_read {
            request = request.timeout(std::time::Duration::from_secs(30));
        }
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
    /// A server restart invalidates every process-local session token. Many
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
    /// from a server password, reauthenticate and replay it once after HTTP 401.
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
        self.send_request_with_headers(method, url, body, accept, reqwest::header::HeaderMap::new())
            .await
    }

    async fn send_request_with_headers(
        &self,
        method: reqwest::Method,
        url: url::Url,
        body: Option<&serde_json::Value>,
        accept: Option<&str>,
        headers: reqwest::header::HeaderMap,
    ) -> Result<reqwest::Response, ClientError> {
        let observed_generation = self.authentication.generation.load(Ordering::Acquire);
        let response = self
            .request_builder(method.clone(), url.clone(), body, accept)
            .headers(headers.clone())
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
            .headers(headers)
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
        let value = response_json(&bytes, status, "server API")?;
        if !status.is_success() {
            if let Ok(api) = serde_json::from_value::<agena_api::error::ApiError>(value.clone()) {
                return Err(ClientError::Api(api));
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

    /// Bodies are shared across panes, with exact URL ownership and bounded
    /// retention. SSE updates the clocks; a shared metadata check closes gaps.
    async fn conditional_json<T: serde::de::DeserializeOwned>(
        &self,
        resource: &str,
        url: url::Url,
        force: bool,
    ) -> Result<T, ClientError> {
        let path = format!("{resource}\n{url}");
        let (gate, required_generation) = {
            let mut reads = self.reads.lock().await;
            reads.read_sequence = reads.read_sequence.saturating_add(1);
            let required_generation = reads.read_sequence;
            reads.locks.retain(|_, lock| lock.strong_count() > 0);
            let gate = if let Some(gate) = reads.locks.get(&path).and_then(std::sync::Weak::upgrade)
            {
                gate
            } else {
                let gate = Arc::new(ResourceReadGate {
                    lock: tokio::sync::Mutex::new(()),
                    requested: AtomicU64::new(0),
                });
                reads.locks.insert(path.clone(), Arc::downgrade(&gate));
                gate
            };
            gate.requested
                .fetch_max(required_generation, Ordering::AcqRel);
            (gate, required_generation)
        };
        let _gate = gate.lock.lock().await;
        let (cached, due, live) = {
            let reads = self.reads.lock().await;
            (
                reads.bodies.get(&path).cloned(),
                reads
                    .checked_at
                    .is_none_or(|time| time.elapsed() >= Duration::from_secs(30)),
                reads.global_live_users.load(Ordering::Acquire) > 0,
            )
        };
        // A client without a global stream cannot know that a cached token
        // stayed current. Validate its explicit read with If-None-Match;
        // otherwise ordinary API callers can miss completion for 30 seconds.
        let reuse = !force && live;
        if cached.is_some() && due && reuse {
            let _check = self.revision_check.lock().await;
            let (keys, observations, due) = {
                let reads = self.reads.lock().await;
                let mut keys = reads
                    .bodies
                    .values()
                    .map(|body| body.resource.clone())
                    .collect::<Vec<_>>();
                keys.sort_unstable();
                keys.dedup();
                (
                    keys,
                    reads.versions.clone(),
                    reads
                        .checked_at
                        .is_none_or(|time| time.elapsed() >= Duration::from_secs(30)),
                )
            };
            if due && !keys.is_empty() {
                // A failed probe has the same shared cooldown as a successful
                // one; callers cannot turn an outage into a request loop.
                self.reads.lock().await.checked_at = Some(Instant::now());
                let mut check_url = self.endpoint("/api/v1/changes/revisions");
                check_url
                    .query_pairs_mut()
                    .append_pair("resources", &serde_json::to_string(&keys)?);
                let response = self
                    .send_request(reqwest::Method::GET, check_url, None, None)
                    .await?;
                let versions: BTreeMap<String, String> = self.parse_json(response).await?;
                let mut reads = self.reads.lock().await;
                for (key, token) in versions {
                    reads.observe(&key, &token, observations.get(&key).map(String::as_str));
                }
                reads.checked_at = Some(Instant::now());
            }
        }
        let observation = self.reads.lock().await.versions.get(resource).cloned();
        if (reuse
            || cached
                .as_ref()
                .is_some_and(|body| body.generation >= required_generation))
            && let Some(cached) = &cached
            && observation.as_deref() == Some(cached.token.as_str())
        {
            self.reads.lock().await.touch(&path);
            return Ok(serde_json::from_value(cached.value.clone())?);
        }
        let _permit = self
            .read_limit
            .acquire()
            .await
            .map_err(|_| ClientError::Protocol("display read limiter closed".into()))?;
        // Calls queued before dispatch share this validated representation,
        // including forced reads. Calls arriving after dispatch revalidate.
        let generation = gate.requested.load(Ordering::Acquire);
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(cached) = &cached {
            headers.insert(
                reqwest::header::IF_NONE_MATCH,
                reqwest::header::HeaderValue::from_str(&cached.etag).map_err(|error| {
                    ClientError::Protocol(format!("invalid resource ETag: {error}"))
                })?,
            );
        }
        let response = self
            .send_request_with_headers(reqwest::Method::GET, url, None, None, headers)
            .await?;
        if response.status() == reqwest::StatusCode::NOT_MODIFIED {
            let mut cached = cached.ok_or_else(|| {
                ClientError::Protocol("304 without a cached representation".into())
            })?;
            if let Some(token) = response
                .headers()
                .get("x-resource-revision")
                .and_then(|value| value.to_str().ok())
            {
                cached.token = token.to_owned();
            }
            if let Some(etag) = response
                .headers()
                .get(reqwest::header::ETAG)
                .and_then(|value| value.to_str().ok())
            {
                cached.etag = etag.to_owned();
            }
            let mut reads = self.reads.lock().await;
            if reads.observe(resource, &cached.token, observation.as_deref())
                && let Some(body) = reads.bodies.get_mut(&path)
            {
                body.generation = generation;
                body.token = cached.token.clone();
                body.etag = cached.etag.clone();
            }
            reads.touch(&path);
            return Ok(serde_json::from_value(cached.value)?);
        }
        let etag = response
            .headers()
            .get(reqwest::header::ETAG)
            .and_then(|value| value.to_str().ok())
            .unwrap_or("")
            .to_owned();
        let token = response
            .headers()
            .get("x-resource-revision")
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
            .unwrap_or_else(|| etag.trim_start_matches("W/").trim_matches('"').to_owned());
        let value: serde_json::Value = self.parse_json(response).await?;
        if !token.is_empty() {
            let mut reads = self.reads.lock().await;
            if reads.observe(resource, &token, observation.as_deref()) {
                let bytes = serde_json::to_vec(&value)?.len();
                reads.remember(
                    path,
                    CachedRead {
                        resource: resource.to_owned(),
                        token,
                        etag,
                        value: value.clone(),
                        bytes,
                        used: 0,
                        generation,
                    },
                );
                reads.checked_at.get_or_insert_with(Instant::now);
            }
        } else {
            self.reads.lock().await.forget(&path);
        }
        Ok(serde_json::from_value(value)?)
    }

    pub async fn session_plan(
        &self,
        session_id: i64,
        force: bool,
    ) -> Result<serde_json::Value, ClientError> {
        self.conditional_json(
            &format!("session:{session_id}:plan"),
            self.endpoint(&format!("/api/v1/sessions/{session_id}/plan")),
            force,
        )
        .await
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

    async fn post_json_if_match<T: serde::de::DeserializeOwned>(
        &self,
        path: &str,
        body: serde_json::Value,
        expected_version: Option<i64>,
    ) -> Result<T, ClientError> {
        let mut headers = reqwest::header::HeaderMap::new();
        if let Some(version) = expected_version {
            headers.insert(
                reqwest::header::IF_MATCH,
                reqwest::header::HeaderValue::from_str(&format!("\"{version}\""))
                    .expect("an integer is a valid quoted header value"),
            );
        }
        let response = self
            .send_request_with_headers(
                reqwest::Method::POST,
                self.endpoint(path),
                Some(&body),
                None,
                headers,
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

    async fn send_notification_frame(
        tx: &mpsc::Sender<Result<SubscriptionEvent, ClientError>>,
        reads: &tokio::sync::Mutex<ResourceReads>,
        notification: Notification,
    ) -> bool {
        let item = match notification {
            Notification::SessionChanged {
                change, revisions, ..
            } => {
                let mut reads = reads.lock().await;
                for (key, token) in revisions {
                    reads.observe(&key, &token, None);
                }
                drop(reads);
                Ok(SubscriptionEvent::SessionChanged(*change))
            }
            Notification::RuntimeSignal {
                signal, revisions, ..
            } => {
                let mut reads = reads.lock().await;
                for (key, token) in revisions {
                    reads.observe(&key, &token, None);
                }
                drop(reads);
                Ok(SubscriptionEvent::RuntimeSignal(*signal))
            }
            Notification::Lagged { skipped, .. } => {
                reads.lock().await.checked_at = None;
                Ok(SubscriptionEvent::Lagged(skipped))
            }
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

    /// Read the live control-plane projection for Agena's own HTTP MCP
    /// server. The response is deliberately kept as JSON until the public API
    /// crate grows a shared MCP-control resource; this endpoint is server
    /// specific and never contains password hashes or OAuth tokens.
    pub async fn mcp_server_control(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/server/mcp").await
    }

    /// Update the live MCP surface. `public_url == None` leaves the current
    /// URL unchanged; `Some(None)` clears it; `Some(Some(url))` sets it.
    pub async fn update_mcp_server_control(
        &self,
        enabled: bool,
        public_url: Option<Option<String>>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut body = serde_json::Map::new();
        body.insert("enabled".to_owned(), serde_json::Value::Bool(enabled));
        if let Some(public_url) = public_url {
            body.insert("publicUrl".to_owned(), serde_json::to_value(public_url)?);
        }
        self.put_json("/api/v1/server/mcp", serde_json::Value::Object(body))
            .await
    }

    /// Configure one Agena address for both the MCP resource and OAuth issuer.
    /// `None` clears the configured address and uses the listener-local URL.
    pub async fn set_mcp_server_url(
        &self,
        server_url: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        self.put_json(
            "/api/v1/server/mcp",
            serde_json::json!({"serverUrl": server_url}),
        )
        .await
    }

    /// Configure the stable OAuth authorization-server issuer. `None` clears
    /// the explicit issuer so the server derives it from the configured public
    /// MCP resource, never from request Host/forwarded headers.
    pub async fn set_mcp_server_oauth_issuer_url(
        &self,
        oauth_issuer_url: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        self.put_json(
            "/api/v1/server/mcp",
            serde_json::json!({"oauthIssuerUrl": oauth_issuer_url}),
        )
        .await
    }

    /// Validate the endpoint protocol and return its process-lifetime identity.
    pub async fn server_identity(
        &self,
    ) -> Result<agena_api::resource::ServerIdentityResource, ClientError> {
        let health = self.health().await?;
        let server = health.server;
        if server.protocol_version != agena_api::PROTOCOL_VERSION {
            return Err(ClientError::Protocol(format!(
                "server protocol {} is incompatible with client protocol {}",
                server.protocol_version,
                agena_api::PROTOCOL_VERSION
            )));
        }
        Ok(server)
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

    /// Return the server's saved provider routing projection. Unlike the
    /// provider model listing, this excludes discovered but unconfigured
    /// models and preserves each adapter's enabled state.
    pub async fn configured_provider_adapter_models(
        &self,
        provider_id: &str,
    ) -> Result<Vec<ProviderAdapterModelsResource>, ClientError> {
        self.get_json(&format!(
            "/api/v1/providers/{provider_id}/configured-models"
        ))
        .await
    }

    /// Return only models implied by the server's saved provider
    /// configuration, without triggering live provider discovery.
    pub async fn configured_provider_models(
        &self,
        provider_id: &str,
    ) -> Result<agena_api::resource::ProviderModelsResponse, ClientError> {
        self.get_json(&format!(
            "/api/v1/providers/{provider_id}/configured-local-models"
        ))
        .await
    }

    /// Fetch, persist, and activate the server's current provider client
    /// versions.
    pub async fn refresh_provider_client_versions(&self) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/providers/client-versions/refresh",
            serde_json::json!({}),
        )
        .await
    }

    pub async fn aws_profile_names(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/providers/aws-profiles").await
    }

    /// Read a server-owned workspace file tree.
    pub async fn workspace_file_tree(
        &self,
        workspace_id: i64,
        path: Option<&str>,
        depth: usize,
        limit: usize,
        respect_ignores: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/workspaces/{workspace_id}/files"));
        {
            let mut query = url.query_pairs_mut();
            if let Some(path) = path.filter(|path| !path.trim().is_empty()) {
                query.append_pair("path", path);
            }
            query.append_pair("depth", &depth.to_string());
            query.append_pair("limit", &limit.to_string());
            if respect_ignores {
                query.append_pair("respect_ignores", "true");
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Upload one client-local file into the server-owned workspace's managed
    /// upload directory. The returned JSON contains the workspace-relative path
    /// clients should persist as a `WorkspacePath` reference.
    pub async fn upload_workspace_file(
        &self,
        workspace_id: i64,
        filename: &str,
        data_base64: &str,
        mime: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            &format!("/api/v1/workspaces/{workspace_id}/files"),
            serde_json::json!({
                "filename": filename,
                "data_base64": data_base64,
                "mime": mime,
            }),
        )
        .await
    }

    /// Download bytes from the server-owned workspace. The returned filename
    /// is reduced to a client-safe final path component even if the remote
    /// server sends an unsafe Content-Disposition header.
    pub async fn download_workspace_file(
        &self,
        workspace_id: i64,
        path: &str,
    ) -> Result<(String, Vec<u8>), ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/workspaces/{workspace_id}/download"));
        url.query_pairs_mut().append_pair("path", path);
        self.download_binary_file(url).await
    }

    /// Download a runtime-managed image belonging to a server session. The
    /// server confines this path to that session's generated-media directory.
    pub async fn download_session_media_file(
        &self,
        session_id: i64,
        path: &str,
    ) -> Result<(String, Vec<u8>), ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/media"));
        url.query_pairs_mut().append_pair("path", path);
        self.download_binary_file(url).await
    }

    async fn download_binary_file(&self, url: url::Url) -> Result<(String, Vec<u8>), ClientError> {
        let mut response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        if !response.status().is_success() {
            let _: serde_json::Value = self.parse_json(response).await?;
            unreachable!("a non-success response cannot decode as success JSON");
        }
        if response
            .content_length()
            .is_some_and(|length| length > MAX_BINARY_RESPONSE_BYTES as u64)
        {
            return Err(ClientError::Protocol(
                "workspace download exceeds the 100 MiB client limit".to_owned(),
            ));
        }
        let filename = response
            .headers()
            .get(reqwest::header::CONTENT_DISPOSITION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.split("filename=\"").nth(1))
            .and_then(|value| value.split('"').next())
            .map(sanitize_download_filename)
            .unwrap_or_else(|| "workspace-file".to_owned());
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            if bytes.len().saturating_add(chunk.len()) > MAX_BINARY_RESPONSE_BYTES {
                return Err(ClientError::Protocol(
                    "workspace download exceeds the 100 MiB client limit".to_owned(),
                ));
            }
            bytes.extend_from_slice(&chunk);
        }
        Ok((filename, bytes))
    }

    pub async fn provider_studio_draft(
        &self,
        provider_id: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/provider-studio/draft");
        if let Some(provider_id) = provider_id {
            url.query_pairs_mut()
                .append_pair("provider_id", provider_id);
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn provider_studio_operation(
        &self,
        operation: &str,
        body: serde_json::Value,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            &format!("/api/v1/provider-studio/{}", operation.trim_matches('/')),
            body,
        )
        .await
    }

    pub async fn create_session(
        &self,
        workspace_id: i64,
        title: impl Into<String>,
        parent_id: Option<i64>,
    ) -> Result<SessionResource, ClientError> {
        self.create_session_with_auto_title(workspace_id, title, parent_id, false)
            .await
    }

    /// Create a session whose placeholder may be replaced by the title worker.
    pub async fn create_session_with_auto_title(
        &self,
        workspace_id: i64,
        title: impl Into<String>,
        parent_id: Option<i64>,
        auto_title: bool,
    ) -> Result<SessionResource, ClientError> {
        let body = serde_json::json!({
            "workspace_id": workspace_id,
            "title": title.into(),
            "parent_id": parent_id,
            "auto_title": auto_title,
        });
        self.post_json("/api/v1/sessions", body).await
    }

    /// Update the durable favorite marker without changing the session title
    /// or pin state.
    pub async fn set_session_favorite(
        &self,
        session_id: i64,
        favorite: bool,
    ) -> Result<SessionResource, ClientError> {
        self.put_json(
            &format!("/api/v1/sessions/{session_id}"),
            serde_json::json!({ "favorite": favorite }),
        )
        .await
    }

    /// Update the independent durable navigation pin.
    pub async fn set_session_pinned(
        &self,
        session_id: i64,
        pinned: bool,
    ) -> Result<SessionResource, ClientError> {
        self.put_json(
            &format!("/api/v1/sessions/{session_id}"),
            serde_json::json!({ "pinned": pinned }),
        )
        .await
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

    /// Read the server-owned plugin runtime status. The plugin runtime's
    /// detailed host DTOs have not yet moved into `agena-api`, so this SDK
    /// method preserves the public REST JSON without depending on the host
    /// implementation crate.
    pub async fn plugin_statuses(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/plugins").await
    }

    /// Fetch the neutral plugin operation catalog plus terminal decorations.
    pub async fn plugin_surface_catalog(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/plugins/surface").await
    }

    /// Fetch the unified live plugin architecture catalog: resolved profiles,
    /// dependency/service graph, reload decisions, effect ownership, typed
    /// pipeline handlers, and scoped registrations.
    pub async fn plugin_architecture_catalog(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/plugins/architecture").await
    }

    pub async fn plugin_inspect(&self, plugin_id: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/plugins");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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

    /// Invoke a plugin Tool API endpoint through `POST /api/v1/plugins/tools/invoke`,
    /// returning the raw plugin tool invoke response JSON.
    pub async fn invoke_plugin_tool(
        &self,
        plugin_id: &str,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
    ) -> Result<serde_json::Value, ClientError> {
        let response: serde_json::Value = self
            .post_json(
                "/api/v1/plugins/tools/invoke",
                serde_json::json!({
                    "plugin_id": plugin_id,
                    "tool": tool_name,
                    "input": input,
                    "session_id": session_id,
                }),
            )
            .await?;
        if plugin_id == "agena.plan"
            && tool_name != "get"
            && let Some(id) = session_id
        {
            let resource = format!("session:{id}:plan");
            let mut reads = self.reads.lock().await;
            if let Some(next) = response.pointer("/payload/plan") {
                let keys = reads
                    .bodies
                    .iter()
                    .filter(|(_, body)| {
                        body.resource == resource
                            && body.value.pointer("/payload/plan/revision") != next.get("revision")
                    })
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in keys {
                    reads.forget(&key);
                }
            } else if response.pointer("/payload/cleared") == Some(&serde_json::Value::Bool(true)) {
                let keys = reads
                    .bodies
                    .iter()
                    .filter(|(_, body)| body.resource == resource)
                    .map(|(key, _)| key.clone())
                    .collect::<Vec<_>>();
                for key in keys {
                    reads.forget(&key);
                }
            }
        }
        Ok(response)
    }

    /// Invoke one server-owned plugin command.
    pub async fn invoke_plugin_command(
        &self,
        plugin_id: &str,
        command_id: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        slash: Option<&str>,
        raw: &str,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/plugins");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
            .push(plugin_id)
            .push("commands")
            .push(command_id)
            .push("invoke");
        let body = serde_json::json!({
            "input": input,
            "session_id": session_id,
            "slash": slash,
            "raw": raw,
        });
        let response = self
            .send_request(reqwest::Method::POST, url, Some(&body), None)
            .await?;
        self.parse_json(response).await
    }

    /// Control a background activity (`pause`, `resume`, or `delete`) through
    /// `POST /api/v1/activities/{activity_id}/{action}`.
    pub async fn control_activity(
        &self,
        activity_id: &str,
        action: &str,
    ) -> Result<agena_api::resource::BackgroundActivityResource, ClientError> {
        self.post_json(
            &format!("/api/v1/activities/{activity_id}/{action}"),
            serde_json::json!({}),
        )
        .await
    }

    pub async fn auth_providers(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/auth/providers").await
    }

    pub async fn auth_provider(&self, provider_id: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/auth/providers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
            .push(provider_id);
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn git_status(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/git/status").await
    }

    /// Durable edits owned by this session, independent of current Git status.
    pub async fn session_file_changes(
        &self,
        session_id: i64,
        offset: usize,
        summary: bool,
        path: Option<&str>,
        max_bytes: usize,
    ) -> Result<agena_api::resource::SessionFileChangesResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/file-changes"));
        url.query_pairs_mut()
            .append_pair("offset", &offset.to_string())
            .append_pair("limit", "40")
            .append_pair("summary", if summary { "true" } else { "false" })
            .append_pair("max_bytes", &max_bytes.to_string());
        if let Some(path) = path {
            url.query_pairs_mut().append_pair("path", path);
        }
        self.conditional_json(&format!("session:{session_id}:files"), url, false)
            .await
    }

    /// Bounded workbench status shared by terminal and browser clients.
    pub async fn workspace_git_status(
        &self,
        directory: &str,
        offset: usize,
        summary: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/workbench/git/status");
        url.query_pairs_mut()
            .append_pair("directory", directory)
            .append_pair("offset", &offset.to_string())
            .append_pair("limit", "40")
            .append_pair("summary", if summary { "true" } else { "false" })
            .append_pair("includeDiffStats", "false");
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// One file only; untracked files and binary patches use server semantics.
    pub async fn workspace_git_diff(
        &self,
        directory: &str,
        path: &str,
        staged: bool,
        max_bytes: usize,
        old_path: Option<&str>,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/workbench/git/diff");
        url.query_pairs_mut()
            .append_pair("directory", directory)
            .append_pair("path", path)
            .append_pair("staged", if staged { "true" } else { "false" })
            .append_pair("contextLines", "3")
            .append_pair("maxBytes", &max_bytes.to_string());
        if let Some(old_path) = old_path {
            url.query_pairs_mut().append_pair("oldPath", old_path);
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
            .push(layer);
        url.query_pairs_mut().append_pair("path", path);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Patch values in the global config file through `PATCH /api/v1/settings`.
    /// The server owns the write: it validates against the full composed
    /// configuration, persists to the global file layer, and reloads the
    /// runtime when the edit requires it. `changes` is a JSON object merged
    /// at the given `path` (for example, the `providers` object with explicit
    /// provider configuration changes).
    pub async fn patch_settings(
        &self,
        path: &str,
        changes: serde_json::Value,
        dry_run: bool,
        reload: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let url = self.endpoint("/api/v1/settings");
        let body = serde_json::json!({
            "path": path,
            "changes": changes,
            "dry_run": dry_run,
            "validate": true,
            "reload": reload,
        });
        let response = self
            .send_request(reqwest::Method::PATCH, url, Some(&body), None)
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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

    /// Delete a value from a settings layer (`global` or `workspace`) and
    /// return the settings edit response.
    pub async fn delete_settings_layer_value(
        &self,
        layer: &str,
        path: &str,
        dry_run: bool,
        reload: bool,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/settings/layers");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
            .push(layer);
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("path", path);
            if dry_run {
                query.append_pair("dry_run", "true");
            }
            query.append_pair("validate", "true");
            if reload {
                query.append_pair("reload", "true");
            }
        }
        let response = self
            .send_request(reqwest::Method::DELETE, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// List the resolved model catalog (optionally filtered by query and
    /// paginated) through `GET /api/v1/model-catalog`.
    pub async fn model_catalog(
        &self,
        query: &str,
        origin: Option<&str>,
        offset: usize,
        limit: usize,
    ) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/model-catalog");
        {
            let mut params = url.query_pairs_mut();
            if !query.is_empty() {
                params.append_pair("q", query);
            }
            if let Some(origin) = origin.filter(|origin| !origin.is_empty()) {
                params.append_pair("origin", origin);
            }
            params.append_pair("offset", &offset.to_string());
            params.append_pair("limit", &limit.to_string());
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Resolve display-only catalog suggestions for raw provider model IDs.
    pub async fn model_catalog_matches(
        &self,
        model_ids: &[String],
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/model-catalog/match",
            serde_json::json!({ "model_ids": model_ids }),
        )
        .await
    }

    /// Request the server to refresh its model catalog through
    /// `POST /api/v1/model-catalog/refresh`.
    pub async fn refresh_model_catalog(&self) -> Result<serde_json::Value, ClientError> {
        self.post_json("/api/v1/model-catalog/refresh", serde_json::json!({}))
            .await
    }

    /// Set the session's selected permission policy through
    /// `PUT /api/v1/sessions/{session_id}/permission`.
    pub async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_api::resource::PermissionConfigResource,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.put_json(
            &format!("/api/v1/sessions/{session_id}/permission"),
            serde_json::json!({ "permission": permission }),
        )
        .await
    }

    pub async fn memory_overview(&self) -> Result<serde_json::Value, ClientError> {
        self.get_json("/api/v1/memories/overview").await
    }

    pub async fn get_memory(&self, name: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/memories");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
            .push(name);
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    pub async fn delete_memory(&self, name: &str) -> Result<serde_json::Value, ClientError> {
        let mut url = self.endpoint("/api/v1/memories");
        url.path_segments_mut()
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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
        self.create_git_commit_in_workspace(None, message).await
    }

    pub async fn create_git_commit_in_workspace(
        &self,
        workspace_id: Option<i64>,
        message: String,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/git/commits",
            serde_json::json!({ "workspace_id": workspace_id, "message": message }),
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
        self.create_git_pull_request_in_workspace(None, title, body, base, head)
            .await
    }

    pub async fn create_git_pull_request_in_workspace(
        &self,
        workspace_id: Option<i64>,
        title: String,
        body: Option<String>,
        base: Option<String>,
        head: Option<String>,
    ) -> Result<serde_json::Value, ClientError> {
        self.post_json(
            "/api/v1/git/pull-requests",
            serde_json::json!({
                "workspace_id": workspace_id,
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
        self.get_session_state_validated(session_id, false).await
    }

    /// Validate a snapshot after a known notification gap or page mismatch.
    /// A forced read still uses ETag and shares reads queued before dispatch.
    pub async fn get_session_state_validated(
        &self,
        session_id: i64,
        force: bool,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.conditional_json(
            &format!("session:{session_id}:state"),
            self.endpoint(&format!("/api/v1/sessions/{session_id}/state")),
            force,
        )
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

    /// Fetch one tool-call detail section. Transcript snapshots intentionally
    /// omit the collapsed raw sections, so callers should request this only
    /// when the corresponding disclosure row is opened.
    pub async fn session_tool_detail(
        &self,
        session_id: i64,
        part_id: i64,
        section: agena_api::live::ToolDetailSection,
    ) -> Result<agena_api::live::ToolDetailResource, ClientError> {
        let resource = if section == agena_api::live::ToolDetailSection::Presentation {
            format!("part:{part_id}")
        } else {
            format!("part:{part_id}:{}", section.as_str())
        };
        self.conditional_json(
            &resource,
            self.endpoint(&format!(
                "/api/v1/sessions/{session_id}/parts/{part_id}/tool-sections/{}",
                section.as_str()
            )),
            false,
        )
        .await
    }

    /// Fetch one bounded newest-first cursor page. The response rows are
    /// chronological, ready for transcript renderers. Tool-call input,
    /// metadata, and raw output remain omitted until `session_tool_detail`
    /// loads the corresponding disclosure section.
    pub async fn session_parts_page(
        &self,
        session_id: i64,
        limit: u64,
        cursor: Option<&str>,
    ) -> Result<agena_api::live::SessionPartsResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/parts"));
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
                query.append_pair("cursor", cursor);
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Reconcile a bounded set of already-loaded memberships. A requested id
    /// absent from the response has been removed or is no longer user-visible.
    pub async fn session_parts_by_ids(
        &self,
        session_id: i64,
        ids: &[i64],
    ) -> Result<agena_api::live::SessionPartsResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/parts"));
        url.query_pairs_mut().append_pair(
            "ids",
            &ids.iter().map(i64::to_string).collect::<Vec<_>>().join(","),
        );
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Fetch the complete visible part history through the current bounded
    /// cursor-paged parts API. Pages are returned newest-first by cursor but
    /// each page is chronological, so page order is reversed before flattening.
    pub async fn session_all_parts(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_api::part::PartResource>, ClientError> {
        let mut cursor: Option<String> = None;
        let mut pages = Vec::new();
        loop {
            let page = self
                .session_parts_page(session_id, 200, cursor.as_deref())
                .await?;
            let next_cursor = page.page.next_cursor.clone();
            pages.push(page.parts);
            if !page.page.has_more {
                break;
            }
            let next_cursor = next_cursor.ok_or_else(|| {
                ClientError::Protocol(
                    "parts page reported has_more without a next cursor".to_owned(),
                )
            })?;
            if cursor.as_deref() == Some(next_cursor.as_str()) {
                return Err(ClientError::Protocol(
                    "parts pagination cursor did not advance".to_owned(),
                ));
            }
            cursor = Some(next_cursor);
        }
        pages.reverse();
        Ok(pages.into_iter().flatten().collect())
    }

    /// Read bounded run membership windows and factual summaries.
    pub async fn session_run_window(
        &self,
        session_id: i64,
        limit: u64,
        part_limit: u64,
        cursor: Option<&str>,
    ) -> Result<agena_api::live::SessionPartsResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/runs"));
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            query.append_pair("part_limit", &part_limit.to_string());
            if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
                query.append_pair("cursor", cursor);
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
    }

    /// Read a bounded child range in one or more runs. Visual grouping is
    /// entirely the caller's responsibility.
    pub async fn session_run_parts_window(
        &self,
        session_id: i64,
        run_ids: &[i64],
        limit: u64,
        cursor: Option<&str>,
    ) -> Result<agena_api::live::SessionPartsResource, ClientError> {
        let mut url = self.endpoint(&format!("/api/v1/sessions/{session_id}/parts"));
        {
            let mut query = url.query_pairs_mut();
            query.append_pair("limit", &limit.to_string());
            query.append_pair(
                "run_ids",
                &run_ids
                    .iter()
                    .map(i64::to_string)
                    .collect::<Vec<_>>()
                    .join(","),
            );
            if let Some(cursor) = cursor.filter(|cursor| !cursor.is_empty()) {
                query.append_pair("cursor", cursor);
            }
        }
        let response = self
            .send_request(reqwest::Method::GET, url, None, None)
            .await?;
        self.parse_json(response).await
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

    pub async fn take_queued_message(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<agena_domain::ComposerDocument, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/queue/{execution_id}/take"),
            serde_json::json!({"request_id": agena_domain::CommandId::new()}),
        )
        .await
    }

    pub async fn queued_message_for_edit(
        &self,
        session_id: i64,
        execution: agena_domain::ExecutionId,
    ) -> Result<agena_api::resource::QueuedMessageEditResource, ClientError> {
        self.get_json(&format!("/api/v1/sessions/{session_id}/queue/{execution}"))
            .await
    }

    pub async fn edit_queued_message(
        &self,
        session_id: i64,
        execution: agena_domain::ExecutionId,
        expected_revision: i64,
        document: agena_domain::ComposerDocument,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.put_json(&format!("/api/v1/sessions/{session_id}/queue/{execution}"),
            serde_json::json!({"request_id": agena_domain::CommandId::new(), "expected_revision":expected_revision,"document":document}),
        ).await
    }

    pub async fn delete_queued_message(
        &self,
        session_id: i64,
        execution: agena_domain::ExecutionId,
    ) -> Result<SessionExecutionResource, ClientError> {
        let body = serde_json::json!({"request_id": agena_domain::CommandId::new()});
        let response = self
            .send_request(
                reqwest::Method::DELETE,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/queue/{execution}")),
                Some(&body),
                None,
            )
            .await?;
        self.parse_json(response).await
    }

    pub async fn set_message_queue_paused(
        &self,
        session_id: i64,
        paused: bool,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/queue/pause"),
            serde_json::json!({"paused": paused}),
        )
        .await
    }

    pub async fn clear_message_queue(
        &self,
        session_id: i64,
    ) -> Result<SessionExecutionResource, ClientError> {
        self.post_json(
            &format!("/api/v1/sessions/{session_id}/queue/clear"),
            serde_json::json!({"request_id": agena_domain::CommandId::new()}),
        )
        .await
    }

    pub async fn cancel_run(
        &self,
        session_id: i64,
        execution_id: Option<agena_domain::ExecutionId>,
    ) -> Result<agena_domain::CancellationOutcome, ClientError> {
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
            .map_err(|()| ClientError::Protocol("server URL cannot carry path segments".into()))?
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
        let response = tokio::time::timeout(
            std::time::Duration::from_secs(15),
            self.send_request(reqwest::Method::GET, url, None, Some("text/event-stream")),
        )
        .await
        .map_err(|_| ClientError::Transport("live subscription handshake timed out".into()))??;
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
        let reads = self.reads.clone();
        reads.lock().await.checked_at = None;
        let lease = if matches!(scope, agena_api::Scope::Global) {
            let users = reads.lock().await.global_live_users.clone();
            users.fetch_add(1, Ordering::AcqRel);
            GlobalRevisionLease {
                users: Some(users),
                active: AtomicBool::new(true),
            }
        } else {
            GlobalRevisionLease {
                users: None,
                active: AtomicBool::new(false),
            }
        };
        let lease = Arc::new(lease);
        let stream_lease = RevisionStreamGuard(lease.clone());
        let task = tokio::spawn(async move {
            let _lease = stream_lease;
            while let Some(frame) = tokio::select! {
                _ = tx.closed() => None,
                frame = tokio::time::timeout(std::time::Duration::from_secs(60), frames.next()) => {
                    match frame {
                        Ok(frame) => frame,
                        Err(_) => {
                            let _ = tx.send(Err(ClientError::Transport("notification stream stalled".into()))).await;
                            None
                        }
                    }
                },
            } {
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
                        if let Err(send_error) = tx.send(Err(ClientError::Decode(error))).await {
                            tracing::debug!(
                                diagnostic = %send_error,
                                "notification decode failure receiver was dropped"
                            );
                        }
                        return;
                    }
                };
                if !Self::send_notification_frame(&tx, &reads, notification).await {
                    return;
                }
            }
        });

        Ok(NotificationSubscription {
            rx,
            task: Some(task),
            revision_lease: lease,
        })
    }

    pub async fn ask_btw(
        &self,
        session_id: i64,
        request: agena_api::resource::BtwRequest,
    ) -> Result<BtwSubscription, ClientError> {
        let body = serde_json::to_value(request)?;
        let response = self
            .send_request(
                reqwest::Method::POST,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/btw")),
                Some(&body),
                Some("text/event-stream"),
            )
            .await?;
        if !response.status().is_success() {
            return self
                .parse_json::<agena_api::resource::BtwAnswer>(response)
                .await
                .and_then(|_| Err(ClientError::Protocol("Expected a BTW stream".into())));
        }
        let (tx, rx) = mpsc::channel(2);
        let task = tokio::spawn(async move {
            let reader = StreamReader::new(response.bytes_stream().map_err(std::io::Error::other));
            let mut frames = FramedRead::new(
                reader,
                SseDecoder::<String>::with_max_size(MAX_SSE_EVENT_BYTES),
            );
            loop {
                let frame = tokio::select! {
                    _ = tx.closed() => return,
                    frame = tokio::time::timeout(std::time::Duration::from_secs(60), frames.next()) => frame,
                };
                let result = match frame {
                    Ok(Some(Ok(SseFrame::Event(event)))) if event.name == "btw" => {
                        serde_json::from_str::<agena_api::resource::BtwAnswer>(&event.data)
                            .map_err(ClientError::Decode)
                    }
                    Ok(Some(Ok(SseFrame::Comment(_) | SseFrame::Retry(_)))) => continue,
                    Ok(Some(Ok(SseFrame::Event(_)))) => {
                        Err(ClientError::Protocol("Unexpected BTW event".into()))
                    }
                    Ok(Some(Err(error))) => Err(ClientError::Protocol(format!(
                        "Invalid BTW stream: {error}"
                    ))),
                    Ok(None) => Err(ClientError::Protocol(
                        "BTW stream ended before completion".into(),
                    )),
                    Err(_) => Err(ClientError::Transport("BTW stream stalled".into())),
                };
                let done = result.as_ref().map_or(true, |answer| answer.done);
                if tx.send(result).await.is_err() || done {
                    return;
                }
            }
        });
        Ok(BtwSubscription { rx, task })
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
                auto_title,
                parent_id,
            }) => Ok(CommandResult::Session(
                self.create_session_with_auto_title(workspace_id, title, parent_id, auto_title)
                    .await?,
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
                at_message_id,
                expected_version,
            }) => Ok(CommandResult::Execution(
                self.post_json_if_match(
                    &format!("/api/v1/sessions/{session_id}/rewind"),
                    serde_json::json!({ "at_message_id": at_message_id }),
                    expected_version,
                )
                .await?,
            )),
            Command::ForkSession(ForkSessionParams {
                conversation_mode,
                session_id,
                at_message_id,
                title,
            }) => Ok(CommandResult::Execution(
                self.post_json(
                    &format!("/api/v1/sessions/{session_id}/fork"),
                    serde_json::json!({
                        "at_message_id": at_message_id,
                        "title": title,
                        "conversation_mode": conversation_mode,
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
        self.query_with_validation(q, false).await
    }

    /// Revalidate cached list queries even while a global live stream exists.
    /// Explicit refreshes and reads after mutations cannot assume that every
    /// revision notification has already reached the client. ETag validation
    /// and coalescing of concurrent requests still apply.
    pub async fn query_revalidated(&self, q: Query) -> Result<QueryResult, ClientError> {
        self.query_with_validation(q, true).await
    }

    async fn query_with_validation(
        &self,
        q: Query,
        force: bool,
    ) -> Result<QueryResult, ClientError> {
        match q {
            Query::ReadContentText(params) => Ok(QueryResult::ContentText(
                self.read_content_text(params).await?,
            )),
            Query::ReadParts(params) => {
                let mut url =
                    self.endpoint(&format!("/api/v1/sessions/{}/parts", params.session_id));
                {
                    let mut query = url.query_pairs_mut();
                    if !params.ids.is_empty() {
                        query.append_pair(
                            "ids",
                            &params
                                .ids
                                .iter()
                                .map(i64::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                    if !params.run_ids.is_empty() {
                        query.append_pair(
                            "run_ids",
                            &params
                                .run_ids
                                .iter()
                                .map(i64::to_string)
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                    if let Some(cursor) = params.cursor {
                        query.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = params.limit {
                        query.append_pair("limit", &limit.to_string());
                    }
                    if !params.sections.is_empty() {
                        query.append_pair(
                            "sections",
                            &params
                                .sections
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::Parts(self.parse_json(response).await?))
            }
            Query::ReadRuns(params) => {
                let mut url =
                    self.endpoint(&format!("/api/v1/sessions/{}/runs", params.session_id));
                {
                    let mut query = url.query_pairs_mut();
                    if let Some(cursor) = params.cursor {
                        query.append_pair("cursor", &cursor);
                    }
                    if let Some(limit) = params.limit {
                        query.append_pair("limit", &limit.to_string());
                    }
                    if let Some(limit) = params.part_limit {
                        query.append_pair("part_limit", &limit.to_string());
                    }
                    if !params.sections.is_empty() {
                        query.append_pair(
                            "sections",
                            &params
                                .sections
                                .iter()
                                .map(|s| s.as_str())
                                .collect::<Vec<_>>()
                                .join(","),
                        );
                    }
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::Parts(self.parse_json(response).await?))
            }
            Query::ReadContent(params) => Ok(QueryResult::Content(
                self.read_content(
                    params.session_id,
                    params.resource_id,
                    params.after,
                    params.max_bytes,
                )
                .await?,
            )),
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
                offset,
            }) => {
                let mut url = self.endpoint("/api/v1/workspaces");
                {
                    let mut q = url.query_pairs_mut();
                    if offset > 0 {
                        q.append_pair("offset", &offset.to_string());
                    }
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
                Ok(QueryResult::Workspaces(
                    self.conditional_json("workspaces", url, force).await?,
                ))
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
                offset,
                bucket,
                include_total,
            }) => {
                let mut url = self.endpoint("/api/v1/sessions");
                {
                    let mut q = url.query_pairs_mut();
                    if offset > 0 {
                        q.append_pair("offset", &offset.to_string());
                    }
                    if include_total {
                        q.append_pair("include_total", "true");
                    }
                    if let Some(bucket) = bucket {
                        let value = serde_json::to_value(bucket)?;
                        if let Some(value) = value.as_str() {
                            q.append_pair("bucket", value);
                        }
                    }
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
                let resource = workspace_id
                    .map(|id| format!("workspace:{id}:sessions"))
                    .unwrap_or_else(|| "sessions".to_owned());
                Ok(QueryResult::Sessions(
                    self.conditional_json(&resource, url, force).await?,
                ))
            }
            Query::GetSession(GetSessionParams { session_id }) => Ok(QueryResult::Session(
                self.get_json(&format!("/api/v1/sessions/{session_id}"))
                    .await?,
            )),
            Query::GetSessionState(GetSessionParams { session_id }) => Ok(
                QueryResult::SessionState(self.get_session_state(session_id).await?),
            ),
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
                Ok(QueryResult::Activities(
                    self.conditional_json("activities", url, force).await?,
                ))
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
                if wait_ms == 0 {
                    return Ok(QueryResult::ActivityLogs(
                        self.conditional_json(&format!("activity:{activity_id}"), url, false)
                            .await?,
                    ));
                }
                let response = self
                    .send_request(reqwest::Method::GET, url, None, None)
                    .await?;
                Ok(QueryResult::ActivityLogs(self.parse_json(response).await?))
            }
        }
    }
}

fn sanitize_download_filename(value: &str) -> String {
    let filename = value
        .rsplit(['/', '\\'])
        .next()
        .map(str::trim)
        .filter(|value| !value.is_empty() && *value != "." && *value != "..")
        .unwrap_or("workspace-file");
    let sanitized = filename
        .chars()
        .map(|character| {
            if character.is_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .take(240)
        .collect::<String>();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "workspace-file".to_owned()
    } else {
        sanitized
    }
}
