//! Connection manager for MCP servers.

#![expect(
    deprecated,
    reason = "MCP roots remain required for compatible servers"
)]

use portable_atomic::AtomicU64;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::{
    ClientCapabilities, ClientInfo, ContentBlock as RmcpContentBlock, GetPromptRequestParams,
    Implementation, ListRootsResult, PaginatedRequestParams, ReadResourceRequestParams,
    ResourceContents as RmcpResourceContents, Role, Root, RootsCapabilities, Tool,
};
use rmcp::service::{NotificationContext, Peer, RoleClient, RunningService};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{
    AuthClient, AuthorizationManager, StreamableHttpClientTransport, TokioChildProcess,
};
use rmcp::{ClientHandler, ServiceExt};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;

use crate::error::{McpError, McpResult};
use crate::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ResourceTemplateDescriptor, ToolDescriptor,
};
use crate::{KeyringOAuthCredentialStore, OAuthCredentialHealth};

#[derive(Debug, Clone)]
/// Specification of an MCP server.
pub enum ServerSpec {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
        tool_policy: McpToolPolicy,
    },
    Http {
        url: Url,
        headers: HashMap<String, String>,
        auth: Option<HttpAuth>,
        tool_policy: McpToolPolicy,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
/// Policy filtering MCP tools.
pub struct McpToolPolicy {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

/// Normalized advisory risk for a discovered MCP tool. This is derived from
/// the server's cached `tools/list` descriptor, never from model-supplied
/// invocation fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpToolRisk {
    Low,
    Medium,
    High,
}

impl McpToolPolicy {
    pub fn permits(&self, tool: &str) -> bool {
        let included = self.include.is_empty()
            || self
                .include
                .iter()
                .any(|pattern| tool_pattern_matches(pattern, tool));
        included
            && !self
                .exclude
                .iter()
                .any(|pattern| tool_pattern_matches(pattern, tool))
    }
}

impl ServerSpec {
    fn tool_policy(&self) -> &McpToolPolicy {
        match self {
            Self::Stdio { tool_policy, .. } | Self::Http { tool_policy, .. } => tool_policy,
        }
    }

    fn auth_mode(&self) -> McpServerAuthMode {
        match self {
            Self::Stdio { .. } => McpServerAuthMode::NotApplicable,
            Self::Http { headers, auth, .. } => match auth {
                Some(HttpAuth::Bearer(_)) => McpServerAuthMode::Bearer,
                Some(HttpAuth::BearerFromEnv(_)) => McpServerAuthMode::BearerFromEnv,
                Some(HttpAuth::BearerFromStore) => McpServerAuthMode::BearerFromStore,
                Some(HttpAuth::OAuth { .. }) => McpServerAuthMode::OAuth,
                Some(HttpAuth::Custom(_)) => McpServerAuthMode::Custom,
                None if headers
                    .keys()
                    .any(|header| header.eq_ignore_ascii_case("authorization")) =>
                {
                    McpServerAuthMode::AuthorizationHeader
                }
                None => McpServerAuthMode::None,
            },
        }
    }

    fn oauth_health(&self, server: &str) -> Option<OAuthCredentialHealth> {
        match self {
            Self::Http {
                auth: Some(HttpAuth::OAuth { .. }),
                ..
            } => Some(
                KeyringOAuthCredentialStore::new(server)
                    .map(|store| store.health())
                    // Server names are validated when specifications are
                    // installed. If that invariant is ever violated, report
                    // a deliberately opaque unreadable projection instead of
                    // bubbling an identifier/keyring error into status.
                    .unwrap_or(OAuthCredentialHealth {
                        credential_state: crate::OAuthCredentialState::Unreadable,
                        expiry_state: None,
                        refresh_available: None,
                    }),
            ),
            _ => None,
        }
    }

    fn credential_migration(
        &self,
        server: &str,
        token_store: Option<&dyn TokenStore>,
    ) -> Option<McpCredentialMigration> {
        match self {
            // OAuth transport never reads the bearer store (see
            // `connect_http`).  A remaining manual bearer record is therefore
            // safe but often signals a completed migration that should be
            // cleaned up explicitly rather than silently combined.
            Self::Http {
                auth: Some(HttpAuth::OAuth { .. }),
                ..
            } => match token_store
                .map(|store| store.credential_state(server))
                .unwrap_or(McpCredentialState::Missing)
            {
                McpCredentialState::Configured => {
                    Some(McpCredentialMigration::OAuthWithManualBearer)
                }
                McpCredentialState::Unreadable => {
                    Some(McpCredentialMigration::OAuthWithUnreadableManualBearer)
                }
                McpCredentialState::Missing => None,
            },
            // Do not migrate automatically in the reverse direction either:
            // the user must first change config to `auth: oauth`, verify the
            // new connection, then remove the bearer credential.
            Self::Http {
                auth: Some(HttpAuth::BearerFromStore),
                ..
            } => match KeyringOAuthCredentialStore::new(server)
                .map(|store| store.health().credential_state)
                .unwrap_or(crate::OAuthCredentialState::Unreadable)
            {
                crate::OAuthCredentialState::Configured => {
                    Some(McpCredentialMigration::BearerWithOAuth)
                }
                crate::OAuthCredentialState::Unreadable => {
                    Some(McpCredentialMigration::BearerWithUnreadableOAuth)
                }
                crate::OAuthCredentialState::Missing => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
/// HTTP authentication of an MCP server.
pub enum HttpAuth {
    Bearer(String),
    BearerFromEnv(String),
    BearerFromStore,
    OAuth { scopes: Vec<String> },
    Custom(HashMap<String, String>),
}

/// Authentication mode of a configured MCP server. This is intentionally a
/// descriptor of configuration shape only; it never contains headers,
/// environment-variable names, scopes, client registrations, or tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpServerAuthMode {
    NotApplicable,
    None,
    Bearer,
    BearerFromEnv,
    BearerFromStore,
    OAuth,
    Custom,
    AuthorizationHeader,
}

impl McpServerAuthMode {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::NotApplicable => "not_applicable",
            Self::None => "none",
            Self::Bearer => "bearer",
            Self::BearerFromEnv => "bearer_from_env",
            Self::BearerFromStore => "bearer_from_store",
            Self::OAuth => "oauth",
            Self::Custom => "custom",
            Self::AuthorizationHeader => "authorization_header",
        }
    }
}

/// Redacted presence state for a manual bearer credential.  It is separate
/// from OAuth health so a status caller can detect a stale credential after a
/// deliberate auth-mode migration without receiving a secret or keyring
/// diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCredentialState {
    Missing,
    Configured,
    Unreadable,
}

/// Explicit, non-mutating migration advisory.  This status proves neither
/// credential is used by the other auth route: it only tells the operator
/// that two separately stored records coexist and gives a safe cleanup path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCredentialMigration {
    OAuthWithManualBearer,
    OAuthWithUnreadableManualBearer,
    BearerWithOAuth,
    BearerWithUnreadableOAuth,
}

impl McpCredentialMigration {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::OAuthWithManualBearer => "oauth_with_manual_bearer",
            Self::OAuthWithUnreadableManualBearer => "oauth_with_unreadable_manual_bearer",
            Self::BearerWithOAuth => "bearer_with_oauth",
            Self::BearerWithUnreadableOAuth => "bearer_with_unreadable_oauth",
        }
    }

    pub const fn recommendation(self) -> &'static str {
        match self {
            Self::OAuthWithManualBearer => "verify_oauth_then_remove_manual_bearer",
            Self::OAuthWithUnreadableManualBearer => {
                "inspect_or_clear_manual_bearer_before_cleanup"
            }
            Self::BearerWithOAuth => "switch_config_to_oauth_verify_then_remove_manual_bearer",
            Self::BearerWithUnreadableOAuth => "inspect_or_clear_oauth_record_before_migration",
        }
    }
}

/// Retry policy for configured MCP servers that are temporarily disconnected.
/// The supervisor only retries entries still present in the manager's
/// configuration; it never invents a connection target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReconnectPolicy {
    pub initial_delay: Duration,
    pub max_delay: Duration,
    pub poll_interval: Duration,
}

impl Default for ReconnectPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(1),
            max_delay: Duration::from_secs(30),
            poll_interval: Duration::from_millis(500),
        }
    }
}

impl ReconnectPolicy {
    pub fn new(initial_delay: Duration, max_delay: Duration, poll_interval: Duration) -> Self {
        let initial_delay = initial_delay.max(Duration::from_millis(1));
        Self {
            initial_delay,
            max_delay: max_delay.max(initial_delay),
            poll_interval: poll_interval.max(Duration::from_millis(1)),
        }
    }

    fn delay_after_failure(self, failures: u32) -> Duration {
        let exponent = failures.saturating_sub(1).min(20);
        let multiplier = 1_u32 << exponent;
        self.initial_delay
            .checked_mul(multiplier)
            .unwrap_or(self.max_delay)
            .min(self.max_delay)
    }
}

struct ReconnectSupervisor {
    handle: tokio::task::JoinHandle<()>,
}

#[derive(Debug, Clone, Copy)]
struct ReconnectAttempt {
    failures: u32,
    retry_at: tokio::time::Instant,
}

type RunningClient = RunningService<RoleClient, AgenaMcpClientHandler>;

const TOOL_REFRESH_RUNNING: u8 = 1;
const TOOL_REFRESH_PENDING: u8 = 2;
const SERVER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(2);

#[derive(Default)]
struct ServerEventState {
    tools: RwLock<Vec<ToolDescriptor>>,
    tool_generation: AtomicU64,
    resource_generation: AtomicU64,
    prompt_generation: AtomicU64,
    last_refresh_failure: RwLock<Option<agena_failure::Failure>>,
    tool_refresh_state: AtomicU8,
    shutdown: CancellationToken,
}

#[derive(Clone)]
struct AgenaMcpClientHandler {
    info: ClientInfo,
    roots: Arc<RwLock<Vec<Root>>>,
    events: Arc<ServerEventState>,
    request_timeout: Duration,
    server_name: String,
    tool_policy: McpToolPolicy,
}

impl ClientHandler for AgenaMcpClientHandler {
    fn get_info(&self) -> ClientInfo {
        self.info.clone()
    }

    async fn list_roots(
        &self,
        _context: rmcp::service::RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, rmcp::ErrorData> {
        Ok(ListRootsResult::new(self.roots.read().await.clone()))
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        loop {
            let previous = self
                .events
                .tool_refresh_state
                .fetch_or(TOOL_REFRESH_PENDING, Ordering::AcqRel);
            if previous & TOOL_REFRESH_RUNNING != 0 {
                return;
            }
            if self
                .events
                .tool_refresh_state
                .compare_exchange(
                    TOOL_REFRESH_PENDING,
                    TOOL_REFRESH_RUNNING,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                )
                .is_ok()
            {
                break;
            }
        }

        let events = Arc::clone(&self.events);
        let timeout = self.request_timeout;
        let server_name = self.server_name.clone();
        let tool_policy = self.tool_policy.clone();
        tokio::spawn(async move {
            loop {
                let refresh = tokio::time::timeout(timeout, context.peer.list_all_tools());
                tokio::select! {
                    biased;
                    _ = events.shutdown.cancelled() => return,
                    result = refresh => match result {
                        Ok(Ok(tools)) => {
                            *events.tools.write().await = filter_tools(tools, &tool_policy);
                            events.tool_generation.fetch_add(1, Ordering::Relaxed);
                            *events.last_refresh_failure.write().await = None;
                        }
                        Ok(Err(error)) => {
                            let error = McpError::from(error);
                            let failure = mcp_failure(&error);
                            warn!(target: "agena_mcp_client::manager", failure_id = %failure.id, server = %server_name, diagnostic = %error, "MCP tool list refresh failed");
                            *events.last_refresh_failure.write().await = Some(failure);
                        }
                        Err(_) => {
                            let error = McpError::Timeout;
                            let failure = mcp_failure(&error);
                            warn!(target: "agena_mcp_client::manager", failure_id = %failure.id, server = %server_name, diagnostic = %error, "MCP tool list refresh timed out");
                            *events.last_refresh_failure.write().await = Some(failure);
                        }
                    }
                }

                loop {
                    let state = events.tool_refresh_state.load(Ordering::Acquire);
                    if state & TOOL_REFRESH_PENDING != 0 {
                        if events
                            .tool_refresh_state
                            .compare_exchange(
                                TOOL_REFRESH_RUNNING | TOOL_REFRESH_PENDING,
                                TOOL_REFRESH_RUNNING,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            )
                            .is_ok()
                        {
                            break;
                        }
                    } else if events
                        .tool_refresh_state
                        .compare_exchange(
                            TOOL_REFRESH_RUNNING,
                            0,
                            Ordering::AcqRel,
                            Ordering::Acquire,
                        )
                        .is_ok()
                    {
                        return;
                    }
                }
            }
        });
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.events
            .resource_generation
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn on_prompt_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.events
            .prompt_generation
            .fetch_add(1, Ordering::Relaxed);
    }

    async fn on_resource_updated(
        &self,
        _params: rmcp::model::ResourceUpdatedNotificationParam,
        _context: NotificationContext<RoleClient>,
    ) {
        self.events
            .resource_generation
            .fetch_add(1, Ordering::Relaxed);
    }
}

/// A connected MCP server.
pub struct ConnectedServer {
    name: String,
    peer: Peer<RoleClient>,
    running: Mutex<Option<RunningClient>>,
    events: Arc<ServerEventState>,
    network_target: Option<String>,
    instructions: Option<String>,
    tool_policy: McpToolPolicy,
}

impl ConnectedServer {
    fn new(
        name: String,
        peer: Peer<RoleClient>,
        running: RunningClient,
        events: Arc<ServerEventState>,
        network_target: Option<String>,
        instructions: Option<String>,
        tool_policy: McpToolPolicy,
    ) -> Self {
        Self {
            name,
            peer,
            running: Mutex::new(Some(running)),
            events,
            network_target,
            instructions,
            tool_policy,
        }
    }
}

impl Drop for ConnectedServer {
    fn drop(&mut self) {
        self.events.shutdown.cancel();
    }
}

/// Manages connections to MCP servers.
pub struct McpConnectionManager {
    inner: Arc<RwLock<Inner>>,
    client_name: String,
    client_version: String,
    token_store: Option<Arc<dyn TokenStore>>,
    connect_timeout: Duration,
    request_timeout: Duration,
    roots: Arc<RwLock<Vec<Root>>>,
    server_operations: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    lifecycle: RwLock<()>,
    reconnect_supervisor: std::sync::Mutex<Option<ReconnectSupervisor>>,
}

#[derive(Default)]
struct Inner {
    servers: BTreeMap<String, Arc<ConnectedServer>>,
    specs: BTreeMap<String, ServerSpec>,
    last_failures: BTreeMap<String, agena_failure::Failure>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Status of an MCP server.
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub network_target: Option<String>,
    pub last_failure: Option<agena_failure::Failure>,
    pub instructions: Option<String>,
    pub tool_generation: u64,
    pub resource_generation: u64,
    pub prompt_generation: u64,
    pub last_refresh_failure: Option<agena_failure::Failure>,
    pub reconnect_supervisor_running: bool,
    pub auth_mode: McpServerAuthMode,
    /// Present only for servers configured with standard MCP OAuth. It is a
    /// local keyring inspection and never triggers a refresh or HTTP request.
    pub oauth_health: Option<OAuthCredentialHealth>,
    /// Optional, redacted advisory that a distinct bearer and OAuth record
    /// coexist across an explicit auth-mode boundary.  It never changes the
    /// selected credential or triggers migration.
    pub credential_migration: Option<McpCredentialMigration>,
}

impl Default for McpConnectionManager {
    fn default() -> Self {
        Self::new("agena", env!("CARGO_PKG_VERSION"))
    }
}

impl McpConnectionManager {
    pub fn new(client_name: impl Into<String>, client_version: impl Into<String>) -> Self {
        Self {
            inner: Default::default(),
            client_name: client_name.into(),
            client_version: client_version.into(),
            token_store: None,
            connect_timeout: Duration::from_secs(20),
            request_timeout: Duration::from_secs(60),
            roots: Arc::new(RwLock::new(Vec::new())),
            server_operations: Mutex::new(BTreeMap::new()),
            lifecycle: RwLock::new(()),
            reconnect_supervisor: std::sync::Mutex::new(None),
        }
    }

    pub fn with_timeouts(mut self, connect_timeout: Duration, request_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout.max(Duration::from_millis(1));
        self.request_timeout = request_timeout.max(Duration::from_millis(1));
        self
    }

    pub fn set_token_store(&mut self, store: Arc<dyn TokenStore>) {
        self.token_store = Some(store);
    }

    pub fn with_roots(mut self, roots: impl IntoIterator<Item = PathBuf>) -> Self {
        self.roots = Arc::new(RwLock::new(mcp_roots(roots)));
        self
    }

    /// Replace the roots returned to MCP servers and notify every active
    /// connection that the advertised root catalog changed.
    pub async fn replace_roots(&self, roots: impl IntoIterator<Item = PathBuf>) -> McpResult<()> {
        *self.roots.write().await = mcp_roots(roots);
        let peers = self
            .inner
            .read()
            .await
            .servers
            .values()
            .map(|server| server.peer.clone())
            .collect::<Vec<_>>();
        for peer in peers {
            peer.notify_roots_list_changed()
                .await
                .map_err(McpError::from)?;
        }
        Ok(())
    }

    /// Start a best-effort reconnect supervisor. The task retains only a
    /// `Weak` reference to the manager, so it cannot keep a runtime snapshot
    /// alive after shutdown. Calling this method again updates neither an
    /// active task nor its policy; stop it first when changing policy.
    pub fn start_reconnect_supervisor(self: &Arc<Self>, policy: ReconnectPolicy) {
        let mut supervisor = self
            .reconnect_supervisor
            .lock()
            .expect("MCP reconnect supervisor lock poisoned");
        if supervisor
            .as_ref()
            .is_some_and(|running| !running.handle.is_finished())
        {
            return;
        }
        if let Some(previous) = supervisor.take() {
            previous.handle.abort();
        }
        let manager = Arc::downgrade(self);
        let handle = tokio::spawn(async move { run_reconnect_supervisor(manager, policy).await });
        *supervisor = Some(ReconnectSupervisor { handle });
    }

    /// Stop the reconnect supervisor if one is active. Configured server
    /// specs remain intact and may still be reconnected explicitly.
    pub fn stop_reconnect_supervisor(&self) {
        let mut supervisor = self
            .reconnect_supervisor
            .lock()
            .expect("MCP reconnect supervisor lock poisoned");
        if let Some(supervisor) = supervisor.take() {
            supervisor.handle.abort();
        }
    }

    pub fn reconnect_supervisor_running(&self) -> bool {
        self.reconnect_supervisor
            .lock()
            .expect("MCP reconnect supervisor lock poisoned")
            .as_ref()
            .is_some_and(|supervisor| !supervisor.handle.is_finished())
    }

    pub async fn add_server(&self, name: &str, spec: ServerSpec) -> McpResult<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(McpError::Malformed(
                "MCP server name must not be empty".to_string(),
            ));
        }
        let _lifecycle_guard = self.lifecycle.read().await;
        self.add_server_in_lifecycle(name, spec).await
    }

    /// Add or replace one server while the caller holds a lifecycle read
    /// permit. This split lets reconnect read its stored spec and connect it
    /// in the same transaction without recursively acquiring the fair Tokio
    /// `RwLock` (which can deadlock behind a queued shutdown writer).
    async fn add_server_in_lifecycle(&self, name: &str, spec: ServerSpec) -> McpResult<()> {
        let operation_lock = {
            let mut operations = self.server_operations.lock().await;
            operations.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = operations.get(name).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                operations.insert(name.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        let _operation_guard = operation_lock.lock().await;
        if let Some(existing) = {
            let mut inner = self.inner.write().await;
            inner.specs.insert(name.to_string(), spec.clone());
            inner.last_failures.remove(name);
            inner.servers.remove(name)
        } {
            shutdown_server(existing).await;
        }

        let events = Arc::new(ServerEventState::default());
        let tool_policy = spec.tool_policy().clone();
        let client_handler = self.client_handler(name, Arc::clone(&events), tool_policy.clone());
        let connection = async {
            match spec {
                ServerSpec::Stdio {
                    command,
                    args,
                    env,
                    cwd,
                    ..
                } => Ok::<_, McpError>((
                    connect_stdio(client_handler, command, args, env, cwd).await?,
                    None,
                )),
                ServerSpec::Http {
                    url, headers, auth, ..
                } => {
                    let target = Some(url.to_string());
                    let running = connect_http(
                        client_handler,
                        name,
                        url,
                        headers,
                        auth,
                        self.token_store.as_deref(),
                    )
                    .await?;
                    Ok::<_, McpError>((running, target))
                }
            }
        };
        let (running, network_target) =
            match tokio::time::timeout(self.connect_timeout, connection).await {
                Ok(Ok(connected)) => connected,
                Ok(Err(error)) => {
                    self.record_error(name, &error).await;
                    return Err(error);
                }
                Err(_) => {
                    self.record_error(name, &McpError::Timeout).await;
                    return Err(McpError::Timeout);
                }
            };

        let peer = running.peer().clone();
        let instructions = peer
            .peer_info()
            .and_then(|info| info.instructions.clone())
            .filter(|value| !value.trim().is_empty());
        let tools = match tokio::time::timeout(self.request_timeout, peer.list_all_tools()).await {
            Ok(Ok(tools)) => filter_tools(tools, &tool_policy),
            Ok(Err(error)) => {
                let error = McpError::from(error);
                self.record_error(name, &error).await;
                let _ = running.cancel().await;
                return Err(error);
            }
            Err(_) => {
                self.record_error(name, &McpError::Timeout).await;
                let _ = running.cancel().await;
                return Err(McpError::Timeout);
            }
        };
        *events.tools.write().await = tools;
        let connected = Arc::new(ConnectedServer::new(
            name.to_string(),
            peer,
            running,
            events,
            network_target,
            instructions,
            tool_policy,
        ));

        let mut inner = self.inner.write().await;
        inner.servers.insert(name.to_string(), connected);
        inner.last_failures.remove(name);
        Ok(())
    }

    pub async fn reconnect(&self, name: &str) -> McpResult<()> {
        let _lifecycle_guard = self.lifecycle.read().await;
        let spec = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))?;
        self.add_server_in_lifecycle(name, spec).await
    }

    pub async fn remove_server(&self, name: &str) -> McpResult<()> {
        let _lifecycle_guard = self.lifecycle.read().await;
        let operation_lock = {
            let mut operations = self.server_operations.lock().await;
            operations.retain(|_, lock| lock.strong_count() > 0);
            if let Some(lock) = operations.get(name).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(Mutex::new(()));
                operations.insert(name.to_string(), Arc::downgrade(&lock));
                lock
            }
        };
        let _operation_guard = operation_lock.lock().await;
        let removed = {
            let mut inner = self.inner.write().await;
            inner.specs.remove(name);
            inner.last_failures.remove(name);
            inner.servers.remove(name)
        };
        if let Some(server) = removed {
            shutdown_server(server).await;
        }
        Ok(())
    }

    pub async fn server_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.specs.keys().cloned().collect()
    }

    pub async fn statuses(&self) -> Vec<McpServerStatus> {
        let inner = self.inner.read().await;
        let specs = inner.specs.clone();
        let servers = inner.servers.clone();
        let failures = inner.last_failures.clone();
        drop(inner);
        let mut statuses = Vec::with_capacity(specs.len());
        let reconnect_supervisor_running = self.reconnect_supervisor_running();
        for (name, spec) in specs {
            let server = servers.get(name.as_str()).cloned();
            let tool_count = match server.as_ref() {
                Some(server) => server.events.tools.read().await.len(),
                None => 0,
            };
            let last_refresh_failure = match server.as_ref() {
                Some(server) => server.events.last_refresh_failure.read().await.clone(),
                None => None,
            };
            statuses.push(McpServerStatus {
                name: name.clone(),
                connected: server.is_some(),
                tool_count,
                network_target: server
                    .as_ref()
                    .and_then(|server| server.network_target.clone()),
                last_failure: failures.get(name.as_str()).cloned(),
                instructions: server
                    .as_ref()
                    .and_then(|server| server.instructions.clone()),
                tool_generation: server.as_ref().map_or(0, |server| {
                    server.events.tool_generation.load(Ordering::Relaxed)
                }),
                resource_generation: server.as_ref().map_or(0, |server| {
                    server.events.resource_generation.load(Ordering::Relaxed)
                }),
                prompt_generation: server.as_ref().map_or(0, |server| {
                    server.events.prompt_generation.load(Ordering::Relaxed)
                }),
                last_refresh_failure,
                reconnect_supervisor_running,
                auth_mode: spec.auth_mode(),
                oauth_health: spec.oauth_health(name.as_str()),
                credential_migration: spec
                    .credential_migration(name.as_str(), self.token_store.as_deref()),
            });
        }
        statuses
    }

    pub async fn all_tools(&self) -> Vec<(String, ToolDescriptor)> {
        let servers = {
            let inner = self.inner.read().await;
            inner.servers.values().cloned().collect::<Vec<_>>()
        };
        let mut out = Vec::new();
        for server in servers {
            let tools = server.events.tools.read().await.clone();
            out.extend(tools.into_iter().map(|tool| (server.name.clone(), tool)));
        }
        out
    }

    /// Non-blocking risk lookup for the execution permission path. A cache
    /// miss is deliberately conservative (`Medium`); callers may convert
    /// high-risk values into an additional approval without trusting input.
    pub fn cached_tool_risk(&self, server: &str, tool: &str) -> McpToolRisk {
        let Ok(inner) = self.inner.try_read() else {
            return McpToolRisk::Medium;
        };
        let Some(server) = inner.servers.get(server).cloned() else {
            return McpToolRisk::Medium;
        };
        drop(inner);
        let Ok(tools) = server.events.tools.try_read() else {
            return McpToolRisk::Medium;
        };
        tools
            .iter()
            .find(|candidate| candidate.name == tool)
            .map(tool_descriptor_risk)
            .unwrap_or(McpToolRisk::Medium)
    }

    pub async fn server_network_targets(&self) -> BTreeMap<String, String> {
        let inner = self.inner.read().await;
        inner
            .specs
            .iter()
            .filter_map(|(name, spec)| match spec {
                ServerSpec::Http { url, .. } => Some((name.clone(), url.to_string())),
                ServerSpec::Stdio { .. } => None,
            })
            .collect()
    }

    pub async fn refresh_tools(&self, name: &str) -> McpResult<Vec<ToolDescriptor>> {
        let server = self.get(name).await?;
        let tools = filter_tools(
            tokio::time::timeout(self.request_timeout, server.peer.list_all_tools())
                .await
                .map_err(|_| McpError::Timeout)??,
            &server.tool_policy,
        );
        *server.events.tools.write().await = tools.clone();
        server
            .events
            .tool_generation
            .fetch_add(1, Ordering::Relaxed);
        *server.events.last_refresh_failure.write().await = None;
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
    ) -> McpResult<CallToolResult> {
        let server = self.get(server).await?;
        if !server.tool_policy.permits(tool) {
            return Err(McpError::ToolDisallowed {
                server: server.name.clone(),
                tool: tool.to_owned(),
            });
        }
        let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
        if let Some(arguments) = value_to_json_object(arguments)? {
            params.arguments = Some(arguments);
        }
        let result = tokio::time::timeout(self.request_timeout, server.peer.call_tool(params))
            .await
            .map_err(|_| McpError::Timeout)??;
        Ok(convert_call_tool_result(result))
    }

    pub async fn list_resources(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListResourcesResult> {
        let server = self.get(server).await?;
        let result = tokio::time::timeout(
            self.request_timeout,
            server.peer.list_resources(pagination_params(cursor)),
        )
        .await
        .map_err(|_| McpError::Timeout)??;
        Ok(ListResourcesResult {
            resources: result
                .resources
                .into_iter()
                .map(convert_resource_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
    }

    pub async fn list_resource_templates(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListResourceTemplatesResult> {
        let server = self.get(server).await?;
        let result = tokio::time::timeout(
            self.request_timeout,
            server
                .peer
                .list_resource_templates(pagination_params(cursor)),
        )
        .await
        .map_err(|_| McpError::Timeout)??;
        Ok(ListResourceTemplatesResult {
            resource_templates: result
                .resource_templates
                .into_iter()
                .map(convert_resource_template)
                .collect(),
            next_cursor: result.next_cursor,
        })
    }

    pub async fn read_resource(&self, server: &str, uri: &str) -> McpResult<ReadResourceResult> {
        let server = self.get(server).await?;
        let result = tokio::time::timeout(
            self.request_timeout,
            server
                .peer
                .read_resource(ReadResourceRequestParams::new(uri.to_string())),
        )
        .await
        .map_err(|_| McpError::Timeout)??;
        Ok(ReadResourceResult {
            contents: result
                .contents
                .into_iter()
                .filter_map(convert_resource_contents)
                .collect(),
        })
    }

    pub async fn list_prompts(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListPromptsResult> {
        let server = self.get(server).await?;
        let result = tokio::time::timeout(
            self.request_timeout,
            server.peer.list_prompts(pagination_params(cursor)),
        )
        .await
        .map_err(|_| McpError::Timeout)??;
        Ok(ListPromptsResult {
            prompts: result
                .prompts
                .into_iter()
                .map(convert_prompt_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
    }

    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> McpResult<GetPromptResult> {
        let server = self.get(server).await?;
        let mut params = GetPromptRequestParams::new(name.to_string());
        if let Some(arguments) = arguments {
            params.arguments = Some(
                arguments
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            );
        }
        let result = tokio::time::timeout(self.request_timeout, server.peer.get_prompt(params))
            .await
            .map_err(|_| McpError::Timeout)??;
        Ok(GetPromptResult {
            description: result.description,
            messages: result
                .messages
                .into_iter()
                .map(convert_prompt_message)
                .collect(),
        })
    }

    pub async fn shutdown_all(&self) {
        self.stop_reconnect_supervisor();
        // Wait for in-flight add/remove transactions before draining. Without
        // this gate an add could finish after the drain and leave a live MCP
        // process in a manager that had already reported shutdown complete.
        let _lifecycle_guard = self.lifecycle.write().await;
        let servers = {
            let mut inner = self.inner.write().await;
            inner.specs.clear();
            inner.last_failures.clear();
            std::mem::take(&mut inner.servers)
                .into_values()
                .collect::<Vec<_>>()
        };
        let mut shutdowns = tokio::task::JoinSet::new();
        for server in servers {
            shutdowns.spawn(shutdown_server(server));
        }
        while shutdowns.join_next().await.is_some() {}
    }

    async fn get(&self, name: &str) -> McpResult<Arc<ConnectedServer>> {
        let inner = self.inner.read().await;
        inner
            .servers
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))
    }

    async fn disconnected_server_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner
            .specs
            .keys()
            .filter(|name| !inner.servers.contains_key(*name))
            .cloned()
            .collect()
    }

    fn client_handler(
        &self,
        server_name: &str,
        events: Arc<ServerEventState>,
        tool_policy: McpToolPolicy,
    ) -> AgenaMcpClientHandler {
        let mut roots = RootsCapabilities::default();
        roots.list_changed = Some(true);
        let mut capabilities = ClientCapabilities::default();
        capabilities.roots = Some(roots);
        AgenaMcpClientHandler {
            info: ClientInfo::new(
                capabilities,
                Implementation::new(self.client_name.clone(), self.client_version.clone()),
            ),
            roots: Arc::clone(&self.roots),
            events,
            request_timeout: self.request_timeout,
            server_name: server_name.to_string(),
            tool_policy,
        }
    }

    async fn record_error(&self, name: &str, error: &McpError) {
        let failure = mcp_failure(error);
        warn!(
            target: "agena_mcp_client::manager",
            failure_id = %failure.id,
            server = %name,
            diagnostic = %error,
            "MCP connection failed"
        );
        self.inner
            .write()
            .await
            .last_failures
            .insert(name.to_string(), failure);
    }
}

fn mcp_failure(error: &McpError) -> agena_failure::Failure {
    use agena_failure::{
        Failure, FailureCategory, FailureCode, FailureImpact, FailureResponsibility,
        RecoveryDirective, RetryDirective, UserPresentation,
    };

    let (code, category, responsibility, retry, recovery, fallback) = match error {
        McpError::Auth(_) => (
            "mcp.authentication_required",
            FailureCategory::AuthenticationRequired,
            FailureResponsibility::Caller,
            RetryDirective::AfterUserAction,
            RecoveryDirective::Reauthenticate,
            "The MCP server requires authentication. Sign in and try again.",
        ),
        McpError::Timeout => (
            "mcp.timeout",
            FailureCategory::Timeout,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::Retry,
            "The MCP server did not respond in time. Try again shortly.",
        ),
        McpError::Malformed(_) | McpError::Serde(_) | McpError::Rpc { .. } => (
            "mcp.protocol_failure",
            FailureCategory::ProtocolFailure,
            FailureResponsibility::Dependency,
            RetryDirective::UseAlternative,
            RecoveryDirective::RestartPlugin,
            "The MCP server returned an invalid response.",
        ),
        McpError::ToolDisallowed { .. } => (
            "mcp.permission_denied",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Policy,
            RetryDirective::AfterUserAction,
            RecoveryDirective::RequestPermission,
            "The MCP tool is disabled by the current permission policy.",
        ),
        McpError::ServerNotConnected(_)
        | McpError::TransportClosed
        | McpError::Transport(_)
        | McpError::Io(_)
        | McpError::Http(_)
        | McpError::Shutdown => (
            "mcp.unavailable",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::Backoff,
            RecoveryDirective::RestartPlugin,
            "The MCP server is unavailable. Reconnect it and try again.",
        ),
        McpError::SamplingUnsupported => (
            "mcp.unsupported",
            FailureCategory::NotFound,
            FailureResponsibility::Dependency,
            RetryDirective::UseAlternative,
            RecoveryDirective::ChooseAlternative,
            "The MCP server does not support this operation.",
        ),
    };
    Failure::new(
        FailureCode::new(code),
        category,
        responsibility,
        retry,
        recovery,
        FailureImpact::RuntimeDegraded,
        UserPresentation::new(code, fallback),
    )
}

async fn run_reconnect_supervisor(
    manager: std::sync::Weak<McpConnectionManager>,
    policy: ReconnectPolicy,
) {
    let mut attempts = BTreeMap::<String, ReconnectAttempt>::new();
    loop {
        let Some(manager) = manager.upgrade() else {
            break;
        };
        let disconnected = manager.disconnected_server_names().await;
        let disconnected_set = disconnected
            .iter()
            .cloned()
            .collect::<std::collections::BTreeSet<_>>();
        attempts.retain(|name, _| disconnected_set.contains(name));
        let now = tokio::time::Instant::now();
        for name in disconnected {
            let attempt = attempts.entry(name.clone()).or_insert(ReconnectAttempt {
                failures: 0,
                retry_at: now,
            });
            if attempt.retry_at > now {
                continue;
            }
            match manager.reconnect(name.as_str()).await {
                Ok(()) => {
                    attempts.remove(name.as_str());
                    tracing::info!(target: "agena_mcp_client::manager", server = %name, "MCP reconnect supervisor restored connection");
                }
                Err(error) => {
                    attempt.failures = attempt.failures.saturating_add(1);
                    let delay = policy.delay_after_failure(attempt.failures);
                    attempt.retry_at = tokio::time::Instant::now() + delay;
                    tracing::debug!(
                        target: "agena_mcp_client::manager",
                        server = %name,
                        failures = attempt.failures,
                        retry_after_ms = delay.as_millis(),
                        "MCP reconnect supervisor attempt failed: {error}"
                    );
                }
            }
        }
        drop(manager);
        tokio::time::sleep(policy.poll_interval).await;
    }
}

fn mcp_roots(paths: impl IntoIterator<Item = PathBuf>) -> Vec<Root> {
    paths
        .into_iter()
        .filter_map(|path| {
            let uri = Url::from_directory_path(path.as_path()).ok()?;
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned);
            let mut root = Root::new(uri.to_string());
            root.name = name;
            Some(root)
        })
        .collect()
}

fn pagination_params(cursor: Option<String>) -> Option<PaginatedRequestParams> {
    cursor.map(|cursor| PaginatedRequestParams::default().with_cursor(Some(cursor)))
}

async fn connect_stdio(
    client_handler: AgenaMcpClientHandler,
    command: String,
    args: Vec<String>,
    env: HashMap<String, String>,
    cwd: Option<PathBuf>,
) -> McpResult<RunningClient> {
    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.envs(env);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let transport = TokioChildProcess::new(agena_process::wrap_command(cmd))?;
    Ok(client_handler.serve(transport).await?)
}

async fn connect_http(
    client_handler: AgenaMcpClientHandler,
    server_name: &str,
    url: Url,
    mut headers: HashMap<String, String>,
    auth: Option<HttpAuth>,
    token_store: Option<&dyn TokenStore>,
) -> McpResult<RunningClient> {
    let has_authorization = headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization"));
    let oauth_scopes = match auth.as_ref() {
        Some(HttpAuth::OAuth { scopes }) => Some(scopes.clone()),
        _ => None,
    };
    if oauth_scopes.is_none()
        && let Some(auth) = auth
    {
        apply_http_auth(server_name, auth, &mut headers, token_store);
    }

    let mut custom_headers = HashMap::new();
    let mut bearer = None;
    for (key, value) in headers {
        if key.eq_ignore_ascii_case("authorization")
            && !has_authorization
            && let Some(stripped) = value.strip_prefix("Bearer ")
        {
            bearer = Some(stripped.to_string());
            continue;
        }
        custom_headers.insert(parse_header_name(&key)?, parse_header_value(&value)?);
    }

    let config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .custom_headers(custom_headers)
        .reinit_on_expired_session(true);
    if let Some(_scopes) = oauth_scopes {
        if bearer.is_some() || has_authorization {
            return Err(McpError::Auth(
                "OAuth MCP configuration must not also supply an Authorization header".to_owned(),
            ));
        }
        let store = KeyringOAuthCredentialStore::new(server_name)
            .map_err(|error| McpError::Auth(error.to_string()))?;
        let mut manager = AuthorizationManager::new(url.clone())
            .await
            .map_err(|error| McpError::Auth(error.to_string()))?;
        manager.set_credential_store(store);
        if !manager
            .initialize_from_store()
            .await
            .map_err(|error| McpError::Auth(error.to_string()))?
        {
            return Err(McpError::Auth(format!(
                "OAuth authorization is required for MCP server '{server_name}'; configure its OAuth credential in the configured MCP client credential store before starting Agena"
            )));
        }
        let http_client = reqwest::Client::builder()
            .build()
            .map_err(|error| McpError::Http(error.to_string()))?;
        manager
            .with_client(http_client.clone())
            .map_err(|error| McpError::Auth(error.to_string()))?;
        let transport = StreamableHttpClientTransport::with_client(
            AuthClient::new(http_client, manager),
            config,
        );
        return Ok(client_handler.serve(transport).await?);
    }
    let config = match bearer {
        Some(token) => config.auth_header(token),
        None => config,
    };
    let transport = StreamableHttpClientTransport::from_config(config);
    Ok(client_handler.serve(transport).await?)
}

async fn shutdown_server(server: Arc<ConnectedServer>) {
    server.events.shutdown.cancel();
    // Take ownership of the cancellable connection before awaiting its
    // shutdown. A cancellation may notify tasks that need to inspect the
    // same `running` slot; holding the mutex across that await creates an
    // avoidable lock cycle.
    let running = server.running.lock().await.take();
    if let Some(running) = running {
        let _ = tokio::time::timeout(SERVER_SHUTDOWN_TIMEOUT, running.cancel()).await;
    }
}

impl Drop for McpConnectionManager {
    fn drop(&mut self) {
        if let Ok(supervisor) = self.reconnect_supervisor.get_mut()
            && let Some(supervisor) = supervisor.take()
        {
            supervisor.handle.abort();
        }
    }
}

fn apply_http_auth(
    server: &str,
    auth: HttpAuth,
    headers: &mut HashMap<String, String>,
    token_store: Option<&dyn TokenStore>,
) {
    match auth {
        HttpAuth::Bearer(token) => {
            headers
                .entry("Authorization".to_string())
                .or_insert_with(|| format!("Bearer {token}"));
        }
        HttpAuth::BearerFromEnv(env_name) => match std::env::var(&env_name) {
            Ok(token) if !token.trim().is_empty() => {
                headers
                    .entry("Authorization".to_string())
                    .or_insert_with(|| format!("Bearer {token}"));
            }
            _ => warn!(
                target: "agena_mcp_client::manager",
                server,
                env = %env_name,
                "missing bearer token env for MCP server"
            ),
        },
        HttpAuth::BearerFromStore => {
            let Some(token_store) = token_store else {
                warn!(
                    target: "agena_mcp_client::manager",
                    server,
                    "no token store configured for MCP bearer lookup"
                );
                return;
            };
            match token_store.bearer(server) {
                Some(token) => {
                    headers
                        .entry("Authorization".to_string())
                        .or_insert_with(|| format!("Bearer {token}"));
                }
                None => warn!(
                    target: "agena_mcp_client::manager",
                    server,
                    "no bearer token found in token store for MCP server"
                ),
            }
        }
        HttpAuth::OAuth { .. } => unreachable!("OAuth is handled before bearer header setup"),
        HttpAuth::Custom(custom_headers) => {
            for (key, value) in custom_headers {
                headers.entry(key).or_insert(value);
            }
        }
    }
}

fn parse_header_name(name: &str) -> McpResult<HeaderName> {
    HeaderName::try_from(name)
        .map_err(|err| McpError::Http(format!("invalid HTTP header name '{name}': {err}")))
}

fn parse_header_value(value: &str) -> McpResult<HeaderValue> {
    HeaderValue::from_str(value)
        .map_err(|err| McpError::Http(format!("invalid HTTP header value: {err}")))
}

fn value_to_json_object(value: Option<Value>) -> McpResult<Option<rmcp::model::JsonObject>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) if map.is_empty() => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(other) => Err(McpError::Malformed(format!(
            "tool arguments must be a JSON object, got {other}"
        ))),
    }
}

fn convert_tools(tools: Vec<Tool>) -> Vec<ToolDescriptor> {
    tools.into_iter().map(convert_tool_descriptor).collect()
}

fn filter_tools(tools: Vec<Tool>, policy: &McpToolPolicy) -> Vec<ToolDescriptor> {
    convert_tools(tools)
        .into_iter()
        .filter(|tool| policy.permits(tool.name.as_str()))
        .collect()
}

fn tool_descriptor_risk(tool: &ToolDescriptor) -> McpToolRisk {
    let annotations = tool.annotations.as_ref();
    let destructive = annotations
        .and_then(|value| value.get("destructiveHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let open_world = annotations
        .and_then(|value| value.get("openWorldHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let read_only = annotations
        .and_then(|value| value.get("readOnlyHint"))
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if destructive || open_world {
        McpToolRisk::High
    } else if read_only {
        McpToolRisk::Low
    } else {
        McpToolRisk::Medium
    }
}

fn tool_pattern_matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return false;
    }
    if !pattern.contains('*') {
        return pattern == name;
    }
    let mut remainder = name;
    let mut first = true;
    for part in pattern.split('*').filter(|part| !part.is_empty()) {
        if first && !pattern.starts_with('*') {
            let Some(after) = remainder.strip_prefix(part) else {
                return false;
            };
            remainder = after;
        } else if let Some(index) = remainder.find(part) {
            remainder = &remainder[index + part.len()..];
        } else {
            return false;
        }
        first = false;
    }
    pattern.ends_with('*') || remainder.is_empty()
}

fn convert_tool_descriptor(tool: Tool) -> ToolDescriptor {
    ToolDescriptor {
        name: tool.name.to_string(),
        title: tool.title,
        aliases: Vec::new(),
        description: tool.description.map(|value| value.into_owned()),
        before_help: None,
        after_help: None,
        input_schema: Some(Value::Object(tool.input_schema.as_ref().clone())),
        output_schema: tool
            .output_schema
            .map(|schema| Value::Object(schema.as_ref().clone())),
        annotations: serialize_optional(tool.annotations),
        execution: None,
        icons: serialize_values(tool.icons.unwrap_or_default()),
        meta: serialize_optional(tool.meta),
    }
}

fn convert_call_tool_result(result: rmcp::model::CallToolResult) -> CallToolResult {
    CallToolResult {
        content: result
            .content
            .into_iter()
            .map(convert_content_block)
            .collect(),
        is_error: result.is_error.unwrap_or(false),
        structured_content: result.structured_content,
        meta: serialize_optional(result.meta),
    }
}

fn convert_content_block(content: RmcpContentBlock) -> ContentBlock {
    match content {
        RmcpContentBlock::Text(text) => ContentBlock::Text {
            text: text.text,
            annotations: serialize_optional(text.annotations),
            meta: serialize_optional(text.meta),
        },
        RmcpContentBlock::Image(image) => ContentBlock::Image {
            data: image.data,
            mime_type: image.mime_type,
            annotations: serialize_optional(image.annotations),
            meta: serialize_optional(image.meta),
        },
        RmcpContentBlock::Audio(audio) => ContentBlock::Audio {
            data: audio.data,
            mime_type: audio.mime_type,
            annotations: serialize_optional(audio.annotations),
            meta: serialize_optional(audio.meta),
        },
        RmcpContentBlock::Resource(resource) => {
            let annotations = serialize_optional(resource.annotations);
            let meta = serialize_optional(resource.meta);
            convert_resource_contents(resource.resource)
                .map(|resource| ContentBlock::Resource {
                    resource,
                    annotations,
                    meta,
                })
                .unwrap_or_else(|| ContentBlock::Unknown { raw: Value::Null })
        }
        RmcpContentBlock::ResourceLink(resource) => ContentBlock::ResourceLink {
            resource: convert_resource_descriptor(resource),
        },
        other => ContentBlock::Unknown {
            raw: serde_json::to_value(other).unwrap_or(Value::Null),
        },
    }
}

fn convert_resource_descriptor(resource: rmcp::model::Resource) -> ResourceDescriptor {
    ResourceDescriptor {
        uri: resource.uri.to_string(),
        name: Some(resource.name.clone()),
        title: resource.title.clone(),
        description: resource.description.clone(),
        mime_type: resource.mime_type.clone(),
        size: resource.size,
        icons: serialize_values(resource.icons.unwrap_or_default()),
        annotations: serialize_optional(resource.annotations),
        meta: serialize_optional(resource.meta),
    }
}

fn convert_resource_template(
    resource: rmcp::model::ResourceTemplate,
) -> ResourceTemplateDescriptor {
    ResourceTemplateDescriptor {
        uri_template: resource.uri_template,
        name: resource.name,
        title: resource.title,
        description: resource.description,
        mime_type: resource.mime_type,
        icons: serialize_values(resource.icons.unwrap_or_default()),
        annotations: serialize_optional(resource.annotations),
        meta: serialize_optional(resource.meta),
    }
}

fn convert_resource_contents(resource: RmcpResourceContents) -> Option<ResourceContents> {
    match resource {
        RmcpResourceContents::TextResourceContents {
            uri,
            mime_type,
            text,
            meta,
        } => Some(ResourceContents {
            uri,
            mime_type,
            text: Some(text),
            blob: None,
            meta: serialize_optional(meta),
        }),
        RmcpResourceContents::BlobResourceContents {
            uri,
            mime_type,
            blob,
            meta,
        } => Some(ResourceContents {
            uri,
            mime_type,
            text: None,
            blob: Some(blob),
            meta: serialize_optional(meta),
        }),
        _ => None,
    }
}

fn serialize_optional<T: serde::Serialize>(value: Option<T>) -> Option<Value> {
    value.and_then(|value| serde_json::to_value(value).ok())
}

fn serialize_values<T: serde::Serialize>(values: Vec<T>) -> Vec<Value> {
    values
        .into_iter()
        .filter_map(|value| serde_json::to_value(value).ok())
        .collect()
}

fn convert_prompt_descriptor(prompt: rmcp::model::Prompt) -> crate::protocol::PromptDescriptor {
    crate::protocol::PromptDescriptor {
        name: prompt.name,
        description: prompt.description,
        arguments: prompt
            .arguments
            .unwrap_or_default()
            .into_iter()
            .map(|argument| crate::protocol::PromptArgument {
                name: argument.name,
                description: argument.description,
                required: argument.required.unwrap_or(false),
            })
            .collect(),
    }
}

fn convert_prompt_message(message: rmcp::model::PromptMessage) -> crate::protocol::PromptMessage {
    crate::protocol::PromptMessage {
        role: match message.role {
            Role::User => "user".to_string(),
            Role::Assistant => "assistant".to_string(),
        },
        content: convert_content_block(message.content),
    }
}

/// Storage of MCP OAuth bearer tokens.
pub trait TokenStore: Send + Sync {
    fn bearer(&self, server: &str) -> Option<String>;

    /// Redacted local-presence check.  Implementations that can distinguish a
    /// keyring/file failure from an absent token should override this rather
    /// than collapsing it to `Missing`.
    fn credential_state(&self, server: &str) -> McpCredentialState {
        if self.bearer(server).is_some() {
            McpCredentialState::Configured
        } else {
            McpCredentialState::Missing
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pagination_params_preserve_non_empty_cursor() {
        assert!(pagination_params(None).is_none());
        assert_eq!(
            pagination_params(Some("next-page".to_string()))
                .expect("pagination params")
                .cursor
                .as_deref(),
            Some("next-page")
        );
    }

    #[test]
    fn audio_and_resource_links_are_not_dropped() {
        let audio = convert_content_block(RmcpContentBlock::audio("ZGF0YQ==", "audio/wav"));
        assert!(matches!(
            audio,
            ContentBlock::Audio {
                data,
                mime_type,
                ..
            } if data == "ZGF0YQ==" && mime_type == "audio/wav"
        ));

        let resource = rmcp::model::Resource::new("docs://guide", "guide")
            .with_title("Guide")
            .with_mime_type("text/markdown");
        let link = convert_content_block(RmcpContentBlock::resource_link(resource));
        assert!(matches!(
            link,
            ContentBlock::ResourceLink { resource }
                if resource.uri == "docs://guide"
                    && resource.title.as_deref() == Some("Guide")
                    && resource.mime_type.as_deref() == Some("text/markdown")
        ));
    }

    #[test]
    fn call_tool_result_preserves_structured_content() {
        let mut input = rmcp::model::CallToolResult::success(vec![RmcpContentBlock::text("ok")]);
        input.structured_content = Some(serde_json::json!({ "answer": 42 }));
        let output = convert_call_tool_result(input);
        assert_eq!(
            output.structured_content,
            Some(serde_json::json!({ "answer": 42 }))
        );
        assert!(matches!(
            output.content.as_slice(),
            [ContentBlock::Text { text, .. }] if text == "ok"
        ));
    }

    #[test]
    fn reconnect_policy_grows_exponentially_and_caps() {
        let policy = ReconnectPolicy::new(
            Duration::from_millis(100),
            Duration::from_millis(350),
            Duration::from_millis(10),
        );
        assert_eq!(policy.delay_after_failure(1), Duration::from_millis(100));
        assert_eq!(policy.delay_after_failure(2), Duration::from_millis(200));
        assert_eq!(policy.delay_after_failure(3), Duration::from_millis(350));
        assert_eq!(policy.delay_after_failure(99), Duration::from_millis(350));
    }

    #[tokio::test]
    async fn reconnect_supervisor_is_explicitly_stoppable_and_uses_weak_manager() {
        let manager = Arc::new(McpConnectionManager::new("test", "1"));
        manager.start_reconnect_supervisor(ReconnectPolicy::new(
            Duration::from_millis(1),
            Duration::from_millis(10),
            Duration::from_millis(1),
        ));
        assert!(manager.reconnect_supervisor_running());
        manager.stop_reconnect_supervisor();
        assert!(!manager.reconnect_supervisor_running());

        manager.start_reconnect_supervisor(ReconnectPolicy::default());
        let weak = Arc::downgrade(&manager);
        drop(manager);
        tokio::task::yield_now().await;
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn client_advertises_and_returns_workspace_roots() {
        let root = PathBuf::from("/tmp/agena-mcp-workspace");
        let manager = McpConnectionManager::new("test", "1").with_roots([root]);
        let handler = manager.client_handler(
            "example",
            Arc::new(ServerEventState::default()),
            McpToolPolicy::default(),
        );

        assert_eq!(
            handler
                .info
                .capabilities
                .roots
                .as_ref()
                .and_then(|roots| roots.list_changed),
            Some(true)
        );
        let roots = handler.roots.read().await;
        assert_eq!(roots.len(), 1);
        assert_eq!(roots[0].name.as_deref(), Some("agena-mcp-workspace"));
        assert_eq!(roots[0].uri, "file:///tmp/agena-mcp-workspace/");
    }

    #[test]
    fn tool_policy_filters_with_exclude_precedence_and_wildcards() {
        let policy = McpToolPolicy {
            include: vec!["repo_*".to_owned(), "read".to_owned()],
            exclude: vec!["repo_delete".to_owned(), "*_secret".to_owned()],
        };
        assert!(policy.permits("repo_list"));
        assert!(policy.permits("read"));
        assert!(!policy.permits("write"));
        assert!(!policy.permits("repo_delete"));
        assert!(!policy.permits("repo_secret"));
    }

    #[test]
    fn oauth_and_bearer_migration_advisories_are_explicit_and_redacted() {
        struct PresentBearer;
        impl TokenStore for PresentBearer {
            fn bearer(&self, _server: &str) -> Option<String> {
                Some("not-exposed".to_owned())
            }

            fn credential_state(&self, _server: &str) -> McpCredentialState {
                McpCredentialState::Configured
            }
        }

        let oauth = ServerSpec::Http {
            url: Url::parse("https://mcp.example.test").expect("URL"),
            headers: HashMap::new(),
            auth: Some(HttpAuth::OAuth { scopes: Vec::new() }),
            tool_policy: McpToolPolicy::default(),
        };
        let advisory = oauth
            .credential_migration("example", Some(&PresentBearer))
            .expect("manual bearer advisory");
        assert_eq!(advisory.as_str(), "oauth_with_manual_bearer");
        assert_eq!(
            advisory.recommendation(),
            "verify_oauth_then_remove_manual_bearer"
        );
        assert!(!advisory.as_str().contains("not-exposed"));
    }

    #[tokio::test]
    async fn failed_connection_remains_configured_and_reports_health() {
        let manager = McpConnectionManager::new("test", "1")
            .with_timeouts(Duration::from_secs(1), Duration::from_secs(1));
        let result = manager
            .add_server(
                "missing",
                ServerSpec::Stdio {
                    command: "/definitely/not/an/agena-mcp-server".to_string(),
                    args: Vec::new(),
                    env: HashMap::new(),
                    cwd: None,
                    tool_policy: McpToolPolicy::default(),
                },
            )
            .await;
        assert!(result.is_err());
        assert_eq!(manager.server_names().await, ["missing"]);
        let statuses = manager.statuses().await;
        assert_eq!(statuses.len(), 1);
        assert!(!statuses[0].connected);
        let failure = statuses[0]
            .last_failure
            .as_ref()
            .expect("structured MCP failure");
        assert!(
            !failure
                .user
                .fallback
                .contains("/definitely/not/an/agena-mcp-server")
        );
        assert_eq!(statuses[0].tool_generation, 0);
        assert_eq!(statuses[0].resource_generation, 0);
        assert_eq!(statuses[0].prompt_generation, 0);
        assert!(manager.reconnect("missing").await.is_err());
        manager.remove_server("missing").await.expect("remove spec");
        assert!(manager.server_names().await.is_empty());
    }
}
