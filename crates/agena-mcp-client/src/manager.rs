//! Connection manager for MCP servers.

#![expect(
    deprecated,
    reason = "MCP roots remain required for compatible servers"
)]

use portable_atomic::AtomicU64;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use http::{HeaderName, HeaderValue};
use rmcp::model::{
    ClientCapabilities, ClientInfo, ClientRequest, ContentBlock as RmcpContentBlock, CustomRequest,
    CustomResult, ElicitRequestParams, ElicitResult, ElicitationCapability,
    FormElicitationCapability, GetPromptRequest, GetPromptRequestParams, Implementation,
    ListRootsResult, PaginatedRequestParams, ProtocolVersion, ReadResourceRequest,
    ReadResourceRequestParams, ResourceContents as RmcpResourceContents, Role, Root,
    RootsCapabilities, ServerNotification, ServerResult, SubscriptionFilter, Tool,
    UrlElicitationCapability,
};
use rmcp::service::{
    NotificationContext, Peer, RequestContext, RoleClient, RunningService, ServiceError,
};
use rmcp::transport::streamable_http_client::StreamableHttpClientTransportConfig;
use rmcp::transport::{
    AuthClient, AuthorizationManager, StreamableHttpClientTransport, TokioChildProcess,
};
use rmcp::{ClientHandler, ClientLifecycleMode, ClientServiceExt};
use serde_json::Value;
use tokio::process::Command;
use tokio::sync::{Mutex, RwLock, watch};
use tokio_util::sync::CancellationToken;
use tracing::warn;
use url::Url;
use uuid::Uuid;

use crate::error::{McpError, McpResult};
use crate::protocol::{
    CallToolResult, ContentBlock, GetPromptResult, ListPromptsResult, ListResourceTemplatesResult,
    ListResourcesResult, ReadResourceResult, ResourceContents, ResourceDescriptor,
    ResourceTemplateDescriptor, ToolDescriptor,
};
use crate::{
    FileOAuthCredentialStore, McpOAuthRegistration, McpRedactions, OAuthCredentialHealth,
    credential_store_name, oauth_credential_binding,
};

mod app_connections;
mod borrowed_http;
mod claude_channel;
mod elicitation;
mod probe;
mod scoped;
mod skills;
mod tool_loading;
pub use tool_loading::{
    MCP_TOOL_LOADER, McpOnDemandConfig, McpOnDemandScope, McpOnDemandSelection, McpOnDemandServer,
};
mod tasks;
pub use tasks::{McpTaskObserver, McpTaskViewInputs};
pub use claude_channel::{ClaudeChannelEvent, ClaudeChannelPeer, ClaudeChannelStream};
pub use elicitation::{McpElicitationHandler, respond_to_app_sampling, respond_to_app_tool_inputs};
pub use probe::{McpProbeOptions, McpProbeResult};
pub use scoped::{McpConnectionObservation, McpSandboxContext, StdioResourceDeclarations};
pub use skills::{McpSkill, McpSkillPage, McpSkillResources};
mod managed;
pub use managed::McpManagedConnection;
use scoped::{PluginHttpCallKey, ScopedServerKey};

static TOOL_CATALOGS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static TOOL_SNAPSHOTS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static MCP_PAYLOADS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);
static SPEC_RESOLVERS: agena_async::BlockingPool = agena_async::BlockingPool::new(2);

fn projection_error(error: tokio::task::JoinError) -> McpError {
    McpError::Transport(format!("MCP projection worker failed: {error}"))
}

async fn prepare_tools(
    tools: Vec<Tool>,
    policy: McpToolPolicy,
    redactions: McpRedactions,
) -> McpResult<Arc<Vec<ToolDescriptor>>> {
    TOOL_CATALOGS
        .run(move || {
            redactions
                .payload(filter_tools(tools, &policy))
                .map(Arc::new)
        })
        .await
        .map_err(projection_error)?
}

async fn replace_tools(events: &ServerEventState, tools: Arc<Vec<ToolDescriptor>>) {
    let retired = {
        let mut cache = events.tools.write().await;
        let count_changed = cache.len() != tools.len();
        let retired = std::mem::replace(&mut *cache, tools);
        // Publish the generation before the next await. Cancellation while
        // releasing the old catalog must not leave a changed cache unmarked.
        events.tool_generation.fetch_add(1, Ordering::Relaxed);
        if count_changed && let Some(observations) = &events.observations {
            observations.send_modify(|revision| *revision += 1);
        }
        retired
    };
    release_tools(retired).await;
}

async fn release_tools(retired: Arc<Vec<ToolDescriptor>>) {
    if let Err(error) = TOOL_CATALOGS.run(move || drop(retired)).await {
        tracing::error!(%error, "MCP tool catalog release worker failed");
    }
}

async fn project_payload<T: Send + 'static>(
    project: impl FnOnce() -> T + Send + 'static,
) -> McpResult<T> {
    MCP_PAYLOADS.run(project).await.map_err(projection_error)
}

async fn project_reply<T: serde::Serialize + serde::de::DeserializeOwned + Send + 'static>(
    server: &ConnectedServer,
    project: impl FnOnce() -> T + Send + 'static,
) -> McpResult<T> {
    let redactions = server.redactions.clone();
    project_payload(move || redactions.payload(project())).await?
}

async fn request_with_elicitation(
    server: &ConnectedServer,
    request: ClientRequest,
    handler: Option<&dyn McpElicitationHandler>,
    operation: &str,
) -> McpResult<ServerResult> {
    agena_async::permission_wait::timeout(
        server.request_timeout,
        elicitation::request(server, request, handler, &mut false),
    )
    .await
    .map_err(|error| {
        McpError::timeout_error(
            format!(
                "MCP {operation} timed out after {}ms",
                server.request_timeout.as_millis()
            ),
            &error,
        )
    })?
    .map_err(|error| server.redactions.error(error.into()))
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Specification of an MCP server.
pub enum ServerSpec {
    Stdio {
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        cwd: Option<PathBuf>,
        tool_policy: McpToolPolicy,
        resources: StdioResourceDeclarations,
        /// Host-compiled policy. Configured definitions remain deferred until
        /// an authenticated execution scope supplies this value.
        sandbox: Option<agena_process::sandbox::SandboxPolicy>,
    },
    Http {
        url: Url,
        transport: McpHttpTransport,
        headers: HashMap<String, String>,
        header_helper: Option<crate::McpHeaderHelper>,
        auth: Option<HttpAuth>,
        tool_policy: McpToolPolicy,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpHttpTransport {
    #[default]
    Streamable,
    Sse,
    WebSocket,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Policy filtering MCP tools.
pub enum McpToolPolicy {
    Patterns {
        include: Vec<String>,
        exclude: Vec<String>,
    },
    ExactNames {
        enabled: Option<BTreeSet<String>>,
        disabled: BTreeSet<String>,
    },
}

impl Default for McpToolPolicy {
    fn default() -> Self {
        Self::Patterns {
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
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
        match self {
            Self::Patterns { include, exclude } => {
                (include.is_empty()
                    || include
                        .iter()
                        .any(|pattern| tool_pattern_matches(pattern, tool)))
                    && !exclude
                        .iter()
                        .any(|pattern| tool_pattern_matches(pattern, tool))
            }
            Self::ExactNames { enabled, disabled } => {
                enabled.as_ref().is_none_or(|names| names.contains(tool))
                    && !disabled.contains(tool)
            }
        }
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

    fn credential_name(&self, server: &str) -> String {
        match self {
            Self::Http { url, .. } => credential_store_name(server, url),
            Self::Stdio { .. } => server.to_owned(),
        }
    }

    fn oauth_health(&self, server: &str) -> Option<OAuthCredentialHealth> {
        let server = self.credential_name(server);
        match self {
            Self::Http {
                auth:
                    Some(HttpAuth::OAuth {
                        scopes,
                        registration,
                    }),
                ..
            } => Some(
                FileOAuthCredentialStore::new(&server)
                    .map(|store| store.with_binding(oauth_credential_binding(scopes, registration)).health())
                    // Server names are validated when specifications are
                    // installed. If that invariant is ever violated, report
                    // a deliberately opaque unreadable projection instead of
                    // bubbling an identifier/storage error into status.
                    .unwrap_or(OAuthCredentialHealth {
                        credential_state: crate::OAuthCredentialState::Unreadable,
                        expiry_state: None,
                        refresh_available: None,
                    }),
            ),
            _ => None,
        }
    }

    async fn credential_projection(
        &self,
        server: &str,
        token_store: Option<Arc<dyn TokenStore>>,
    ) -> (Option<OAuthCredentialHealth>, Option<McpCredentialConflict>) {
        let mode = self.auth_mode();
        if !matches!(
            mode,
            McpServerAuthMode::OAuth | McpServerAuthMode::BearerFromStore
        ) {
            return (None, None);
        }
        let spec = self.clone();
        let name = server.to_owned();
        match crate::token_store::SECRET_STORE_WORKERS
            .run(move || {
                (
                    spec.oauth_health(&name),
                    spec.credential_conflict(&name, token_store.as_deref()),
                )
            })
            .await
        {
            Ok(projection) => projection,
            Err(error) => {
                tracing::warn!(server, %error, "MCP credential status worker failed");
                (
                    matches!(mode, McpServerAuthMode::OAuth).then_some(OAuthCredentialHealth {
                        credential_state: crate::OAuthCredentialState::Unreadable,
                        expiry_state: None,
                        refresh_available: None,
                    }),
                    None,
                )
            }
        }
    }

    fn credential_conflict(
        &self,
        server: &str,
        token_store: Option<&dyn TokenStore>,
    ) -> Option<McpCredentialConflict> {
        let server = self.credential_name(server);
        match self {
            // OAuth transport never reads the bearer store (see
            // `connect_http`).  A remaining manual bearer record is therefore
            // safe but often signals a previous auth-mode switch that should be
            // cleaned up explicitly rather than silently combined.
            Self::Http {
                auth: Some(HttpAuth::OAuth { .. }),
                ..
            } => match token_store
                .map(|store| store.credential_state(&server))
                .unwrap_or(McpCredentialState::Missing)
            {
                McpCredentialState::Configured => {
                    Some(McpCredentialConflict::OAuthWithManualBearer)
                }
                McpCredentialState::Unreadable => {
                    Some(McpCredentialConflict::OAuthWithUnreadableManualBearer)
                }
                McpCredentialState::Missing => None,
            },
            // Do not combine the two credential stores in the reverse direction either:
            // the user must first change config to `auth: oauth`, verify the
            // new connection, then remove the bearer credential.
            Self::Http {
                auth: Some(HttpAuth::BearerFromStore),
                ..
            } => match FileOAuthCredentialStore::new(&server)
                .map(|store| store.health().credential_state)
                .unwrap_or(crate::OAuthCredentialState::Unreadable)
            {
                crate::OAuthCredentialState::Configured => {
                    Some(McpCredentialConflict::BearerWithOAuth)
                }
                crate::OAuthCredentialState::Unreadable => {
                    Some(McpCredentialConflict::BearerWithUnreadableOAuth)
                }
                crate::OAuthCredentialState::Missing => None,
            },
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// HTTP authentication of an MCP server.
pub enum HttpAuth {
    Bearer(String),
    BearerFromEnv(String),
    BearerFromStore,
    OAuth {
        scopes: Vec<String>,
        registration: McpOAuthRegistration,
    },
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
/// deliberate auth-mode switch without receiving a secret or storage
/// diagnostic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCredentialState {
    Missing,
    Configured,
    Unreadable,
}

/// Explicit, non-mutating credential conflict. This status proves neither
/// credential is used by the other auth route; it tells the operator that two
/// separately stored records coexist and gives a safe cleanup path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpCredentialConflict {
    OAuthWithManualBearer,
    OAuthWithUnreadableManualBearer,
    BearerWithOAuth,
    BearerWithUnreadableOAuth,
}

impl McpCredentialConflict {
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
            Self::BearerWithUnreadableOAuth => {
                "inspect_or_clear_oauth_record_before_switching_auth"
            }
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

struct ConnectionAttempt {
    active: Option<Arc<()>>,
    observations: watch::Sender<u64>,
}

impl ConnectionAttempt {
    fn new(observations: watch::Sender<u64>) -> Self {
        Self {
            active: Some(Arc::new(())),
            observations,
        }
    }
}

impl Drop for ConnectionAttempt {
    fn drop(&mut self) {
        self.active.take();
        self.observations.send_modify(|revision| *revision += 1);
    }
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
    http_session_open: Arc<std::sync::atomic::AtomicBool>,
    app_http_events: crate::http_transport::AppHttpEvents,
    app_resource_listener: Mutex<()>,
    tools: RwLock<Arc<Vec<ToolDescriptor>>>,
    tool_generation: AtomicU64,
    tool_list_changes: watch::Sender<u64>,
    resource_generation: AtomicU64,
    resource_list_changes: watch::Sender<u64>,
    prompt_generation: AtomicU64,
    last_refresh_failure: RwLock<Option<agena_failure::Failure>>,
    tool_refresh_state: AtomicU8,
    shutdown: CancellationToken,
    tasks: tasks::TaskRegistry,
    observations: Option<watch::Sender<u64>>,
    claude_channel: claude_channel::ChannelEvents,
}

impl ServerEventState {
    fn resource_list_changed(&self) {
        self.resource_generation.fetch_add(1, Ordering::Relaxed);
        self.resource_list_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
}

#[derive(Clone)]
struct AgenaMcpClientHandler {
    info: ClientInfo,
    roots: Arc<RwLock<Vec<Root>>>,
    events: Arc<ServerEventState>,
    refresh_catalog_in_background: bool,
    request_timeout: Duration,
    server_name: String,
    tool_policy: McpToolPolicy,
    redactions: McpRedactions,
}

impl ClientHandler for AgenaMcpClientHandler {
    fn get_info(&self) -> ClientInfo {
        self.info.clone()
    }

    fn on_ordered_custom_notification(
        &self,
        notification: &rmcp::model::CustomNotification,
        context: &NotificationContext<RoleClient>,
    ) -> bool {
        self.events
            .claude_channel
            .handle(notification, &context.peer, &self.redactions)
    }

    async fn list_roots(
        &self,
        context: RequestContext<RoleClient>,
    ) -> Result<ListRootsResult, rmcp::ErrorData> {
        let route = self.events.tasks.legacy_route(&context)?;
        let mut roots = ListRootsResult::new(self.roots.read().await.clone());
        if let Some(route) = route {
            roots.meta = Some(route.metadata());
        } else {
            let _ = elicitation::associated_incoming(&context);
        }
        Ok(roots)
    }

    async fn create_elicitation(
        &self,
        request: ElicitRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<ElicitResult, rmcp::ErrorData> {
        elicitation::standard_request(request, context, &self.events.tasks).await
    }

    #[allow(deprecated)]
    async fn create_message(
        &self,
        request: rmcp::model::CreateMessageRequestParams,
        context: RequestContext<RoleClient>,
    ) -> Result<rmcp::model::CreateMessageResult, rmcp::ErrorData> {
        elicitation::sampling_request(request, context, &self.events.tasks).await
    }

    async fn on_custom_request(
        &self,
        request: CustomRequest,
        context: RequestContext<RoleClient>,
    ) -> Result<CustomResult, rmcp::ErrorData> {
        elicitation::custom_request(request, context, &self.events.tasks).await
    }

    async fn on_tool_list_changed(&self, context: NotificationContext<RoleClient>) {
        self.tool_list_changed(context.peer);
    }

    async fn on_resource_list_changed(&self, _context: NotificationContext<RoleClient>) {
        self.resource_list_changed();
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

impl AgenaMcpClientHandler {
    fn resource_list_changed(&self) {
        self.events.resource_list_changed();
    }

    fn tool_list_changed(&self, peer: Peer<RoleClient>) {
        self.events
            .tool_list_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        if self.refresh_catalog_in_background {
            self.queue_tool_refresh(peer);
        }
    }

    fn queue_tool_refresh(&self, peer: Peer<RoleClient>) {
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
        let redactions = self.redactions.clone();
        tokio::spawn(async move {
            loop {
                let refresh = tokio::time::timeout(timeout, async {
                    peer.clear_response_cache().await;
                    peer.list_all_tools().await
                });
                tokio::select! {
                    biased;
                    _ = events.shutdown.cancelled() => return,
                    result = refresh => match result {
                        Ok(Ok(tools)) => {
                            match prepare_tools(tools, tool_policy.clone(), redactions.clone()).await {
                                Ok(tools) => {
                                    replace_tools(&events, tools).await;
                                    *events.last_refresh_failure.write().await = None;
                                }
                                Err(error) => {
                                    warn!(category = %error.user_message(), server = %server_name, "MCP tool catalog projection failed");
                                    *events.last_refresh_failure.write().await = Some(mcp_failure(&error));
                                }
                            }
                        }
                        Ok(Err(error)) => {
                            let error = McpError::from(error);
                            let failure = mcp_failure(&error);
                            warn!(target: "agena_mcp_client::manager", failure_id = %failure.id, server = %server_name, category = %error.user_message(), "MCP tool list refresh failed");
                            *events.last_refresh_failure.write().await = Some(failure);
                        }
                        Err(timeout_error) => {
                            let error = McpError::timeout_error(
                                format!(
                                    "MCP tool-list refresh timed out after {}ms",
                                    timeout.as_millis()
                                ),
                                &timeout_error,
                            );
                            let failure = mcp_failure(&error);
                            warn!(target: "agena_mcp_client::manager", failure_id = %failure.id, server = %server_name, category = %error.user_message(), "MCP tool list refresh timed out");
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
}

/// A connected MCP server.
pub struct ConnectedServer {
    peer_id: Uuid,
    name: String,
    peer: Peer<RoleClient>,
    running: Mutex<Option<RunningClient>>,
    events: Arc<ServerEventState>,
    network_target: Option<String>,
    instructions: Option<String>,
    tool_policy: McpToolPolicy,
    request_timeout: Duration,
    tool_timeout: Duration,
    close_timeout: Duration,
    connected_at_ms: i64,
    redactions: McpRedactions,
}

impl ConnectedServer {
    fn borrowed(&self, peer: Peer<RoleClient>) -> Arc<Self> {
        Arc::new(Self {
            peer_id: self.peer_id,
            name: self.name.clone(),
            peer,
            running: Mutex::new(None),
            events: self.events.clone(),
            network_target: self.network_target.clone(),
            instructions: self.instructions.clone(),
            tool_policy: self.tool_policy.clone(),
            request_timeout: self.request_timeout,
            tool_timeout: self.tool_timeout,
            close_timeout: self.close_timeout,
            connected_at_ms: self.connected_at_ms,
            redactions: self.redactions.clone(),
        })
    }
}

impl Drop for ConnectedServer {
    fn drop(&mut self) {
        if self.running.get_mut().is_some() {
            self.events.shutdown.cancel();
        }
    }
}

/// Manages connections to MCP servers.
pub struct McpConnectionManager {
    inner: Arc<RwLock<Inner>>,
    client_name: String,
    client_version: String,
    apps_enabled: bool,
    token_store: Option<Arc<dyn TokenStore>>,
    http_network_authorizer: Option<Arc<dyn crate::McpHttpNetworkAuthorizer>>,
    http_background_enabled: bool,
    borrowed_http: bool,
    _legacy_app_operation: Option<tokio::sync::OwnedMutexGuard<()>>,
    connect_timeout: Duration,
    request_timeout: Duration,
    roots: Arc<RwLock<Vec<Root>>>,
    server_operations: Mutex<BTreeMap<String, Weak<Mutex<()>>>>,
    lifecycle: RwLock<()>,
    reconnect_supervisor: std::sync::Mutex<Option<ReconnectSupervisor>>,
    oauth_sessions: crate::oauth::McpOAuthSessionManager,
    scoped_servers: Mutex<HashMap<ScopedServerKey, Arc<McpConnectionManager>>>,
    plugin_http_calls: Mutex<BTreeMap<PluginHttpCallKey, Arc<McpConnectionManager>>>,
    app_http_connections: Mutex<BTreeMap<(i64, i64), app_connections::AppHttpConnection>>,
    managed_connections: Mutex<BTreeMap<(i64, String), Arc<McpManagedConnection>>>,
    observations: watch::Sender<u64>,
    next_spec_revision: AtomicU64,
    next_tool_observation: AtomicU64,
}

#[derive(Default)]
struct Inner {
    servers: BTreeMap<String, Arc<ConnectedServer>>,
    specs: BTreeMap<String, StoredServerSpec>,
    last_failures: BTreeMap<String, agena_failure::Failure>,
    connecting: BTreeMap<String, Weak<()>>,
    on_demand: tool_loading::SessionCatalogs,
}

#[derive(Clone)]
struct StoredServerSpec {
    spec: ServerSpec,
    connect_timeout: Duration,
    request_timeout: Duration,
    plugin_timeouts: ScopedPluginTimeouts,
    revision: u64,
    plugin_scoped: bool,
    resolve_workspace: Option<ScopedPluginServerResolver>,
    redactions: McpRedactions,
    on_demand: Option<McpOnDemandConfig>,
}

#[derive(Clone, Copy, Default)]
pub struct ScopedPluginTimeouts {
    /// Complete connection setup, including initial tool discovery.
    pub startup: Option<Duration>,
    /// Active time for a tool invocation and its continuations.
    pub tool_call: Option<Duration>,
    /// Budget for shutting down the scoped instance.
    pub close: Option<Duration>,
}

#[derive(Clone)]
pub struct ScopedPluginServer {
    pub spec: ServerSpec,
    pub redactions: McpRedactions,
}

impl From<ServerSpec> for ScopedPluginServer {
    fn from(spec: ServerSpec) -> Self {
        Self {
            spec,
            redactions: McpRedactions::default(),
        }
    }
}

/// Reinterpret a plugin definition against the authenticated caller's workspace.
pub type ScopedPluginServerResolver =
    Arc<dyn Fn(&Path) -> Result<ScopedPluginServer, String> + Send + Sync>;

impl StoredServerSpec {
    fn requires_scope(&self, http_authorized: bool) -> bool {
        match &self.spec {
            ServerSpec::Stdio { sandbox, .. } => sandbox.is_none(),
            ServerSpec::Http { .. } => self.plugin_scoped && !http_authorized,
        }
    }

    async fn in_workspace(&self, workspace: &Path) -> McpResult<ScopedPluginServer> {
        let Some(resolve) = self.resolve_workspace.clone() else {
            return Ok(ScopedPluginServer {
                spec: self.spec.clone(),
                redactions: self.redactions.clone(),
            });
        };
        let workspace = workspace.to_owned();
        SPEC_RESOLVERS
            .run(move || resolve(&workspace))
            .await
            .map_err(projection_error)?
            .map_err(McpError::Malformed)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Status of an MCP server.
pub struct McpServerStatus {
    pub name: String,
    pub connected: bool,
    /// The definition is waiting for an authorized caller scope.
    pub scope_required: bool,
    pub protocol_version: Option<String>,
    pub server_info: Option<Value>,
    pub capabilities: Option<Value>,
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
    /// local secret-store inspection and never triggers a refresh or HTTP request.
    pub oauth_health: Option<OAuthCredentialHealth>,
    /// Optional, redacted advisory that a distinct bearer and OAuth record
    /// coexist across an explicit auth-mode boundary.  It never changes the
    /// selected credential or changes the selected credential.
    pub credential_conflict: Option<McpCredentialConflict>,
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
            apps_enabled: false,
            token_store: None,
            http_network_authorizer: None,
            http_background_enabled: true,
            borrowed_http: false,
            _legacy_app_operation: None,
            connect_timeout: Duration::from_secs(20),
            request_timeout: Duration::from_secs(60),
            roots: Arc::new(RwLock::new(Vec::new())),
            server_operations: Mutex::new(BTreeMap::new()),
            lifecycle: RwLock::new(()),
            reconnect_supervisor: std::sync::Mutex::new(None),
            oauth_sessions: Default::default(),
            scoped_servers: Mutex::new(HashMap::new()),
            plugin_http_calls: Mutex::default(),
            app_http_connections: Mutex::default(),
            managed_connections: Mutex::default(),
            observations: watch::Sender::new(0),
            next_spec_revision: AtomicU64::new(1),
            next_tool_observation: AtomicU64::new(1),
        }
    }

    pub fn with_timeouts(mut self, connect_timeout: Duration, request_timeout: Duration) -> Self {
        self.connect_timeout = connect_timeout.max(Duration::from_millis(1));
        self.request_timeout = request_timeout.max(Duration::from_millis(1));
        self
    }

    pub fn with_apps(mut self, enabled: bool) -> Self {
        self.apps_enabled = enabled;
        self
    }

    /// Bind HTTP I/O to one live call. Do not install this on a shared catalog.
    pub fn with_http_network_authorizer(
        mut self,
        authorizer: Arc<dyn crate::McpHttpNetworkAuthorizer>,
    ) -> Self {
        self.http_network_authorizer = Some(authorizer);
        self
    }

    pub fn oauth_sessions(&self) -> &crate::oauth::McpOAuthSessionManager {
        &self.oauth_sessions
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
        let roots = mcp_roots(roots);
        {
            let mut current = self.roots.write().await;
            if *current == roots {
                return Ok(());
            }
            *current = roots;
        }
        let peers = self
            .inner
            .read()
            .await
            .servers
            .values()
            .filter(|server| !server.peer.is_transport_closed())
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
        self.add_server_with_timeouts(name, spec, self.connect_timeout, self.request_timeout)
            .await
    }

    pub async fn add_server_with_timeouts(
        &self,
        name: &str,
        spec: ServerSpec,
        connect_timeout: Duration,
        request_timeout: Duration,
    ) -> McpResult<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(McpError::Malformed(
                "MCP server name must not be empty".to_string(),
            ));
        }
        let _lifecycle_guard = self.lifecycle.read().await;
        let operation_lock = self.server_operation_lock(name).await;
        let _operation_guard = operation_lock.lock().await;
        self.connect_server_locked(
            name,
            StoredServerSpec {
                spec,
                connect_timeout: connect_timeout.max(Duration::from_millis(1)),
                request_timeout: request_timeout.max(Duration::from_millis(1)),
                plugin_timeouts: ScopedPluginTimeouts::default(),
                revision: self.next_spec_revision.fetch_add(1, Ordering::Relaxed),
                plugin_scoped: false,
                resolve_workspace: None,
                redactions: McpRedactions::default(),
                on_demand: None,
            },
            claude_channel::ChannelEvents::default(),
        )
        .await
    }

    /// Register a plugin contribution without connecting or replacing an
    /// existing definition. Only a caller-owned scope can start a connection.
    pub async fn register_scoped_plugin_server(
        &self,
        name: &str,
        server: impl Into<ScopedPluginServer>,
        timeouts: ScopedPluginTimeouts,
        resolve_workspace: Option<ScopedPluginServerResolver>,
    ) -> McpResult<()> {
        self.register_scoped_plugin_server_with_loading(
            name,
            server,
            timeouts,
            resolve_workspace,
            None,
        )
        .await
    }

    pub async fn register_scoped_plugin_server_with_loading(
        &self,
        name: &str,
        server: impl Into<ScopedPluginServer>,
        timeouts: ScopedPluginTimeouts,
        resolve_workspace: Option<ScopedPluginServerResolver>,
        on_demand: Option<McpOnDemandConfig>,
    ) -> McpResult<()> {
        let ScopedPluginServer { spec, redactions } = server.into();
        if name.is_empty()
            || name.trim() != name
            || matches!(
                &spec,
                ServerSpec::Stdio {
                    sandbox: Some(_),
                    ..
                }
            )
        {
            return Err(McpError::Malformed(
                "invalid scoped MCP server definition".into(),
            ));
        }
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        let mut inner = self.inner.write().await;
        if inner.specs.contains_key(name) {
            return Err(McpError::Malformed(
                "MCP server name is already configured".into(),
            ));
        }
        inner.specs.insert(
            name.to_owned(),
            StoredServerSpec {
                spec,
                connect_timeout: self.connect_timeout,
                request_timeout: self.request_timeout,
                plugin_timeouts: timeouts,
                revision: self.next_spec_revision.fetch_add(1, Ordering::Relaxed),
                plugin_scoped: true,
                resolve_workspace,
                redactions,
                on_demand,
            },
        );
        Ok(())
    }

    async fn server_operation_lock(&self, name: &str) -> Arc<Mutex<()>> {
        let mut operations = self.server_operations.lock().await;
        operations.retain(|_, lock| lock.strong_count() > 0);
        if let Some(lock) = operations.get(name).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(Mutex::new(()));
            operations.insert(name.to_string(), Arc::downgrade(&lock));
            lock
        }
    }

    /// The lifecycle read guard and per-server operation guard must already be
    /// held. Reconnect resolves its stored definition under the same guard, so
    /// a concurrent removal cannot resurrect a deleted connection.
    async fn connect_server_locked(
        &self,
        name: &str,
        stored: StoredServerSpec,
        claude_channel: claude_channel::ChannelEvents,
    ) -> McpResult<()> {
        self.retire_scoped_server(name).await;
        if self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .is_some_and(|previous| previous.revision != stored.revision)
        {
            self.retire_app_http_connections(|_, server| server == name)
                .await;
        }
        let startup_timeout = stored.plugin_timeouts.startup;
        let connect_timeout = startup_timeout.unwrap_or(stored.connect_timeout);
        let request_timeout = stored.request_timeout;
        let tool_timeout = stored.plugin_timeouts.tool_call.unwrap_or(request_timeout);
        let close_timeout = stored
            .plugin_timeouts
            .close
            .unwrap_or(SERVER_SHUTDOWN_TIMEOUT);
        let requires_scope = stored.requires_scope(self.http_network_authorizer.is_some());
        let plugin_scoped = stored.plugin_scoped;
        let spec = stored.spec.clone();
        let redactions = stored.redactions.clone();
        let attempt = (!requires_scope).then(|| ConnectionAttempt::new(self.observations.clone()));
        if let Some(existing) = {
            let mut inner = self.inner.write().await;
            inner.specs.insert(name.to_string(), stored);
            inner.last_failures.remove(name);
            if let Some(active) = attempt.as_ref().and_then(|attempt| attempt.active.as_ref()) {
                inner
                    .connecting
                    .insert(name.to_owned(), Arc::downgrade(active));
                self.notify_observation();
            }
            inner.servers.remove(name)
        } {
            shutdown_server(existing).await;
        }

        // Config loading stores definitions without launching a global stdio
        // process. A scoped manager supplies the immutable sandbox policy.
        if requires_scope {
            return Ok(());
        }

        let events = Arc::new(ServerEventState {
            observations: Some(self.observations.clone()),
            claude_channel,
            ..Default::default()
        });
        let tool_policy = spec.tool_policy().clone();
        let client_handler = self.client_handler(
            name,
            Arc::clone(&events),
            tool_policy.clone(),
            request_timeout,
            redactions.clone(),
        );
        let connection = self.open_transport(name, spec, client_handler, plugin_scoped);
        let started_at = tokio::time::Instant::now();
        let (running, network_target) =
            match tokio::time::timeout(connect_timeout, connection).await {
                Ok(Ok(connected)) => connected,
                Ok(Err(error)) => {
                    let error = if events.http_session_open.load(Ordering::Acquire) {
                        McpError::UnconfirmedClosure
                    } else {
                        redactions.error(error)
                    };
                    self.record_error(name, &error).await;
                    return Err(error);
                }
                Err(error) => {
                    let error = if events.http_session_open.load(Ordering::Acquire) {
                        McpError::UnconfirmedClosure
                    } else {
                        McpError::timeout_error(
                            format!(
                                "MCP connection timed out after {}ms",
                                connect_timeout.as_millis()
                            ),
                            &error,
                        )
                    };
                    self.record_error(name, &error).await;
                    return Err(error);
                }
            };

        let peer = running.peer().clone();
        let instructions = peer
            .peer_info()
            .and_then(|info| info.instructions.clone())
            .filter(|value| !value.trim().is_empty())
            .map(|value| redactions.text(value));
        let tools_result = if peer
            .peer_info()
            .is_some_and(|info| info.capabilities.tools.is_some())
        {
            let listing_timeout = startup_timeout.map_or(request_timeout, |budget| {
                budget.saturating_sub(started_at.elapsed())
            });
            match tokio::time::timeout(listing_timeout, peer.list_all_tools()).await {
                Ok(Ok(tools)) => {
                    prepare_tools(tools, tool_policy.clone(), redactions.clone()).await
                }
                Ok(Err(error)) => Err(redactions.error(McpError::from(error))),
                Err(timeout_error) => Err(McpError::timeout_error(
                    format!(
                        "initial MCP tool listing timed out after {}ms",
                        listing_timeout.as_millis()
                    ),
                    &timeout_error,
                )),
            }
        } else {
            Ok(Arc::new(Vec::new()))
        };
        let tools = match tools_result {
            Ok(tools) => tools,
            Err(error) => {
                events.shutdown.cancel();
                let confirmed = shutdown_client(name, running, close_timeout).await
                    && !events.http_session_open.load(Ordering::Acquire);
                let error = if confirmed {
                    error
                } else {
                    McpError::UnconfirmedClosure
                };
                self.record_error(name, &error).await;
                return Err(error);
            }
        };
        let retired = {
            let mut cache = events.tools.write().await;
            if events.tool_generation.load(Ordering::Relaxed) == 0 {
                std::mem::replace(&mut *cache, tools)
            } else {
                tools
            }
        };
        release_tools(retired).await;
        if self.http_background_enabled {
            start_catalog_subscription(running.service().clone(), peer.clone());
        }
        observe_transport_close(&events, peer.clone());
        let connected = Arc::new(ConnectedServer {
            peer_id: Uuid::new_v4(),
            name: name.to_owned(),
            peer,
            running: Mutex::new(Some(running)),
            events,
            network_target,
            instructions,
            tool_policy,
            request_timeout,
            tool_timeout,
            close_timeout,
            connected_at_ms: chrono::Utc::now().timestamp_millis(),
            redactions,
        });

        let mut inner = self.inner.write().await;
        inner.servers.insert(name.to_string(), connected);
        inner.last_failures.remove(name);
        Ok(())
    }

    async fn open_transport(
        &self,
        name: &str,
        spec: ServerSpec,
        client_handler: AgenaMcpClientHandler,
        plugin_scoped: bool,
    ) -> McpResult<(RunningClient, Option<String>)> {
        match spec {
            ServerSpec::Stdio {
                command,
                args,
                env,
                cwd,
                sandbox,
                ..
            } => Ok((
                connect_stdio(
                    client_handler,
                    command,
                    args,
                    env,
                    cwd,
                    sandbox,
                    plugin_scoped,
                )
                .await?,
                None,
            )),
            ServerSpec::Http {
                url,
                transport,
                headers,
                header_helper,
                auth,
                ..
            } => {
                let target = Some(url.to_string());
                let running = connect_http(
                    client_handler,
                    name,
                    url,
                    transport,
                    headers,
                    header_helper,
                    auth,
                    self.token_store.clone(),
                    self.http_network_authorizer.clone(),
                    self.http_background_enabled,
                )
                .await?;
                Ok((running, target))
            }
        }
    }

    pub async fn reconnect(&self, name: &str) -> McpResult<()> {
        self.reconnect_with_channel_events(name, claude_channel::ChannelEvents::default())
            .await
    }

    async fn reconnect_with_channel_events(
        &self,
        name: &str,
        claude_channel: claude_channel::ChannelEvents,
    ) -> McpResult<()> {
        if self.borrowed_http {
            return Err(McpError::TransportClosed);
        }
        let _lifecycle_guard = self.lifecycle.read().await;
        let operation_lock = self.server_operation_lock(name).await;
        let _operation_guard = operation_lock.lock().await;
        let spec = self
            .inner
            .read()
            .await
            .specs
            .get(name)
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))?;
        if spec.requires_scope(self.http_network_authorizer.is_some()) {
            return Err(scoped::missing_scope(name));
        }
        self.connect_server_locked(name, spec, claude_channel).await
    }

    pub async fn remove_server(&self, name: &str) -> McpResult<()> {
        let _lifecycle_guard = self.lifecycle.read().await;
        let operation_lock = self.server_operation_lock(name).await;
        let _operation_guard = operation_lock.lock().await;
        self.retire_scoped_server(name).await;
        self.retire_app_http_connections(|_, server| server == name)
            .await;
        self.retire_managed_connections(|_, server| server == name)
            .await;
        let removed = {
            let mut inner = self.inner.write().await;
            inner.specs.remove(name);
            inner.last_failures.remove(name);
            inner.connecting.remove(name);
            inner.on_demand.retain(|(_, server), _| server != name);
            inner.servers.remove(name)
        };
        self.notify_observation();
        if let Some(server) = removed {
            shutdown_server(server).await;
        }
        Ok(())
    }

    pub async fn server_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner.specs.keys().cloned().collect()
    }

    pub async fn is_plugin_server(&self, name: &str) -> bool {
        self.inner
            .read()
            .await
            .specs
            .get(name)
            .is_some_and(|server| server.plugin_scoped)
    }

    pub async fn statuses(&self) -> Vec<McpServerStatus> {
        let inner = self.inner.read().await;
        let specs = inner.specs.clone();
        let servers = inner.servers.clone();
        let failures = inner.last_failures.clone();
        drop(inner);
        let mut statuses = Vec::with_capacity(specs.len());
        let reconnect_supervisor_running = self.reconnect_supervisor_running();
        for (name, stored) in specs {
            let scope_required = stored.requires_scope(self.http_network_authorizer.is_some());
            let spec = stored.spec;
            let server = servers.get(name.as_str()).cloned();
            let connected = server
                .as_ref()
                .is_some_and(|server| !server.peer.is_transport_closed());
            let peer_info = server.as_ref().and_then(|server| server.peer.peer_info());
            let tool_count = if connected {
                server
                    .as_ref()
                    .expect("connected server exists")
                    .events
                    .tools
                    .read()
                    .await
                    .len()
            } else {
                0
            };
            let last_refresh_failure = match server.as_ref() {
                Some(server) => server.events.last_refresh_failure.read().await.clone(),
                None => None,
            };
            let (oauth_health, credential_conflict) = spec
                .credential_projection(&name, self.token_store.clone())
                .await;
            statuses.push(McpServerStatus {
                name: name.clone(),
                connected,
                scope_required,
                protocol_version: peer_info
                    .as_ref()
                    .map(|info| info.protocol_version.to_string()),
                server_info: peer_info
                    .as_ref()
                    .and_then(|info| serialize_optional(info.server_info.clone()))
                    .and_then(|info| server.as_ref()?.redactions.payload(info).ok()),
                capabilities: peer_info
                    .as_ref()
                    .and_then(|info| serde_json::to_value(&info.capabilities).ok())
                    .and_then(|capabilities| {
                        server.as_ref()?.redactions.payload(capabilities).ok()
                    }),
                tool_count,
                network_target: server.as_ref().and_then(|server| {
                    server.network_target.as_deref().and_then(redacted_endpoint)
                }),
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
                oauth_health,
                credential_conflict,
            });
        }
        statuses
    }

    pub async fn all_tools(&self) -> Vec<(String, ToolDescriptor)> {
        let servers = {
            let inner = self.inner.read().await;
            inner
                .servers
                .values()
                .filter(|server| !server.peer.is_transport_closed())
                .cloned()
                .collect::<Vec<_>>()
        };
        let mut snapshots = Vec::with_capacity(servers.len());
        for server in servers {
            let tools = server.events.tools.read().await.clone();
            snapshots.push((server.name.clone(), tools));
        }
        TOOL_SNAPSHOTS
            .run(move || {
                let mut out = Vec::new();
                for (name, tools) in snapshots {
                    out.extend(tools.iter().map(|tool| (name.clone(), tool.clone())));
                }
                out
            })
            .await
            .unwrap_or_else(|error| {
                tracing::error!(%error, "MCP tool catalog snapshot worker failed");
                Vec::new()
            })
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
        if server.peer.is_transport_closed() {
            return McpToolRisk::Medium;
        }
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
            .filter_map(|(name, stored)| match &stored.spec {
                ServerSpec::Http { url, .. } => Some((name.clone(), url.to_string())),
                ServerSpec::Stdio { .. } => None,
            })
            .collect()
    }

    pub async fn refresh_tools(&self, name: &str) -> McpResult<Vec<ToolDescriptor>> {
        let server = self.get(name).await?;
        if !server
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.tools.is_some())
        {
            return Ok(Vec::new());
        }
        server.peer.clear_response_cache().await;
        let tools = prepare_tools(
            tokio::time::timeout(server.request_timeout, server.peer.list_all_tools())
                .await
                .map_err(|error| {
                    McpError::timeout_error(
                        format!(
                            "MCP tool-list refresh timed out after {}ms",
                            server.request_timeout.as_millis()
                        ),
                        &error,
                    )
                })?
                .map_err(|error| server.redactions.error(error.into()))?,
            server.tool_policy.clone(),
            server.redactions.clone(),
        )
        .await?;
        replace_tools(&server.events, Arc::clone(&tools)).await;
        *server.events.last_refresh_failure.write().await = None;
        TOOL_SNAPSHOTS
            .run(move || tools.as_ref().clone())
            .await
            .map_err(projection_error)
    }

    pub async fn call_tool(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
    ) -> McpResult<CallToolResult> {
        self.call_tool_with_elicitation(server, tool, arguments, None)
            .await
    }

    pub async fn call_tool_with_elicitation(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
        handler: Option<&dyn McpElicitationHandler>,
    ) -> McpResult<CallToolResult> {
        self.call_tool_with_peer(server, tool, arguments, handler)
            .await
            .map(|(result, _)| result)
    }

    /// The peer ID comes from the connection that delivered the result, even
    /// when HTTP authentication rotated the peer during the call.
    pub async fn call_tool_with_peer(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
        handler: Option<&dyn McpElicitationHandler>,
    ) -> McpResult<(CallToolResult, Uuid)> {
        self.call_tool_on_peer(server, tool, arguments, handler, None).await
    }

    pub async fn call_tool_observed(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
        handler: &dyn McpElicitationHandler,
        observer: &McpTaskObserver<'_>,
    ) -> McpResult<CallToolResult> {
        self.call_tool_on_peer(server, tool, arguments, Some(handler), Some(observer))
            .await.map(|(result, _)| result)
    }

    async fn call_tool_on_peer(
        &self,
        server: &str,
        tool: &str,
        arguments: Option<Value>,
        handler: Option<&dyn McpElicitationHandler>,
        observer: Option<&McpTaskObserver<'_>>,
    ) -> McpResult<(CallToolResult, Uuid)> {
        let server = self.get(server).await?;
        require_capability(&server, "tools")?;
        if !server.tool_policy.permits(tool) {
            return Err(McpError::ToolDisallowed {
                server: server.name.clone(),
                tool: tool.to_owned(),
            });
        }
        let mut params = rmcp::model::CallToolRequestParams::new(tool.to_string());
        if let Some(arguments) = value_to_json_object(arguments).await? {
            params.arguments = Some(arguments);
        }
        let mut call = tasks::ToolCall::default();
        let work = async {
            let error = match call.run(&server, params.clone(), handler, observer).await {
                Ok(result) => return Ok((Arc::clone(&server), result)),
                Err(error) => error,
            };
            if call.continuation_started || !error.http_auth_rejected() {
                return Err(McpError::from(error));
            }
            let Some(fresh) = self.refresh_header_connection(&server).await? else {
                return Err(McpError::from(error));
            };
            require_capability(&fresh, "tools")?;
            if !fresh.tool_policy.permits(tool) {
                return Err(McpError::ToolDisallowed {
                    server: fresh.name.clone(),
                    tool: tool.to_owned(),
                });
            }
            let result = call.run(&fresh, params, handler, observer).await.map_err(|error| {
                if !call.continuation_started && error.http_auth_rejected() {
                    McpError::Auth("MCP server rejected the refreshed credentials".into())
                } else {
                    fresh.redactions.error(error.into())
                }
            })?;
            Ok((fresh, result))
        };
        let outcome = if let Some(observer) = observer {
            tokio::select! {
                biased;
                _ = observer.cancellation.cancelled() => Ok(Err(ServiceError::Cancelled { reason: None }.into())),
                result = agena_async::permission_wait::timeout(server.tool_timeout, work) => result,
            }
        } else {
            agena_async::permission_wait::timeout(server.tool_timeout, work).await
        };
        call.settle().await;
        let (connected, result) = outcome
            .map_err(|error| {
                McpError::timeout_error(
                    format!(
                        "MCP tool call timed out after {}ms",
                        server.tool_timeout.as_millis()
                    ),
                    &error,
                )
            })?
            .map_err(|error| server.redactions.error(error))?;
        let result = project_reply(&connected, move || convert_call_tool_result(result)).await?;
        Ok((result, connected.peer_id))
    }

    async fn refresh_header_connection(
        &self,
        stale: &Arc<ConnectedServer>,
    ) -> McpResult<Option<Arc<ConnectedServer>>> {
        if self.borrowed_http {
            return Ok(None);
        }
        let name = &stale.name;
        let _lifecycle = self.lifecycle.read().await;
        let operation = self.server_operation_lock(name).await;
        let _operation = operation.lock().await;
        let (stored, reconnected) = {
            let inner = self.inner.read().await;
            let Some(stored) = inner.specs.get(name) else {
                return Ok(None);
            };
            if !matches!(
                stored.spec,
                ServerSpec::Http {
                    header_helper: Some(_),
                    ..
                }
            ) || stored.requires_scope(self.http_network_authorizer.is_some())
            {
                return Ok(None);
            }
            let reconnected = inner.servers.get(name).is_some_and(|server| {
                !Arc::ptr_eq(server, stale) && !server.peer.is_transport_closed()
            });
            (stored.clone(), reconnected)
        };
        if !reconnected {
            self.connect_server_locked(name, stored, claude_channel::ChannelEvents::default())
                .await?;
        }
        self.get(name).await.map(Some)
    }

    pub async fn list_resources(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListResourcesResult> {
        let server = self.get(server).await?;
        if !server
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
        {
            return Ok(ListResourcesResult::default());
        }
        let result = tokio::time::timeout(
            server.request_timeout,
            server.peer.list_resources(pagination_params(cursor)),
        )
        .await
        .map_err(|error| {
            McpError::timeout_error(
                format!(
                    "MCP resource listing timed out after {}ms",
                    server.request_timeout.as_millis()
                ),
                &error,
            )
        })?
        .map_err(|error| server.redactions.error(McpError::from(error)))?;
        project_reply(&server, move || ListResourcesResult {
            resources: result
                .resources
                .into_iter()
                .map(convert_resource_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
        .await
    }

    pub async fn list_resource_templates(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListResourceTemplatesResult> {
        let server = self.get(server).await?;
        if !server
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.resources.is_some())
        {
            return Ok(ListResourceTemplatesResult::default());
        }
        let result = tokio::time::timeout(
            server.request_timeout,
            server
                .peer
                .list_resource_templates(pagination_params(cursor)),
        )
        .await
        .map_err(|error| {
            McpError::timeout_error(
                format!(
                    "MCP resource-template listing timed out after {}ms",
                    server.request_timeout.as_millis()
                ),
                &error,
            )
        })?
        .map_err(|error| server.redactions.error(McpError::from(error)))?;
        project_reply(&server, move || ListResourceTemplatesResult {
            resource_templates: result
                .resource_templates
                .into_iter()
                .map(convert_resource_template)
                .collect(),
            next_cursor: result.next_cursor,
        })
        .await
    }

    pub async fn read_resource(&self, server: &str, uri: &str) -> McpResult<ReadResourceResult> {
        self.read_resource_with_elicitation(server, uri, None).await
    }

    pub async fn read_resource_with_elicitation(
        &self,
        server: &str,
        uri: &str,
        handler: Option<&dyn McpElicitationHandler>,
    ) -> McpResult<ReadResourceResult> {
        let server = self.get(server).await?;
        require_capability(&server, "resources")?;
        let request = ReadResourceRequest::new(ReadResourceRequestParams::new(uri.to_string()));
        let result = request_with_elicitation(
            &server,
            ClientRequest::ReadResourceRequest(request),
            handler,
            "resource read",
        )
        .await?;
        let ServerResult::ReadResourceResult(result) = result else {
            return Err(ServiceError::UnexpectedResponse.into());
        };
        project_reply(&server, move || ReadResourceResult {
            contents: result
                .contents
                .into_iter()
                .filter_map(convert_resource_contents)
                .collect(),
        })
        .await
    }

    pub async fn list_prompts(
        &self,
        server: &str,
        cursor: Option<String>,
    ) -> McpResult<ListPromptsResult> {
        let server = self.get(server).await?;
        if !server
            .peer
            .peer_info()
            .is_some_and(|info| info.capabilities.prompts.is_some())
        {
            return Ok(ListPromptsResult::default());
        }
        let result = tokio::time::timeout(
            server.request_timeout,
            server.peer.list_prompts(pagination_params(cursor)),
        )
        .await
        .map_err(|error| {
            McpError::timeout_error(
                format!(
                    "MCP prompt listing timed out after {}ms",
                    server.request_timeout.as_millis()
                ),
                &error,
            )
        })?
        .map_err(|error| server.redactions.error(McpError::from(error)))?;
        project_reply(&server, move || ListPromptsResult {
            prompts: result
                .prompts
                .into_iter()
                .map(convert_prompt_descriptor)
                .collect(),
            next_cursor: result.next_cursor,
        })
        .await
    }

    pub async fn get_prompt(
        &self,
        server: &str,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
    ) -> McpResult<GetPromptResult> {
        self.get_prompt_with_elicitation(server, name, arguments, None)
            .await
    }

    pub async fn get_prompt_with_elicitation(
        &self,
        server: &str,
        name: &str,
        arguments: Option<BTreeMap<String, String>>,
        handler: Option<&dyn McpElicitationHandler>,
    ) -> McpResult<GetPromptResult> {
        let server = self.get(server).await?;
        require_capability(&server, "prompts")?;
        let mut params = GetPromptRequestParams::new(name.to_string());
        if let Some(arguments) = arguments {
            params.arguments = Some(
                arguments
                    .into_iter()
                    .map(|(key, value)| (key, Value::String(value)))
                    .collect(),
            );
        }
        let result = request_with_elicitation(
            &server,
            ClientRequest::GetPromptRequest(GetPromptRequest::new(params)),
            handler,
            "prompt read",
        )
        .await?;
        let ServerResult::GetPromptResult(result) = result else {
            return Err(ServiceError::UnexpectedResponse.into());
        };
        project_reply(&server, move || GetPromptResult {
            description: result.description,
            messages: result
                .messages
                .into_iter()
                .map(convert_prompt_message)
                .collect(),
        })
        .await
    }

    pub async fn shutdown_all(&self) {
        self.stop_reconnect_supervisor();
        self.oauth_sessions.shutdown().await;
        let _lifecycle_guard = self.lifecycle.write().await;
        self.retire_app_http_connections(|_, _| true).await;
        self.retire_managed_connections(|_, _| true).await;
        let scoped = std::mem::take(&mut *self.scoped_servers.lock().await);
        let calls = std::mem::take(&mut *self.plugin_http_calls.lock().await);
        for manager in scoped.into_values().chain(calls.into_values()) {
            manager.shutdown_local().await;
        }
        self.shutdown_connections().await;
    }

    async fn shutdown_local(&self) {
        self.stop_reconnect_supervisor();
        self.oauth_sessions.shutdown().await;
        // Wait for in-flight add/remove transactions before draining. Without
        // this gate an add could finish after the drain and leave a live MCP
        // process in a manager that had already reported shutdown complete.
        let _lifecycle_guard = self.lifecycle.write().await;
        self.shutdown_connections().await;
    }

    async fn shutdown_connections(&self) {
        let servers = {
            let mut inner = self.inner.write().await;
            inner.specs.clear();
            inner.last_failures.clear();
            inner.connecting.clear();
            inner.on_demand.clear();
            std::mem::take(&mut inner.servers)
                .into_values()
                .collect::<Vec<_>>()
        };
        self.notify_observation();
        let mut shutdowns = tokio::task::JoinSet::new();
        for server in servers {
            shutdowns.spawn(shutdown_server(server));
        }
        while shutdowns.join_next().await.is_some() {}
    }

    async fn get(&self, name: &str) -> McpResult<Arc<ConnectedServer>> {
        let inner = self.inner.read().await;
        if inner
            .specs
            .get(name)
            .is_some_and(|stored| stored.requires_scope(self.http_network_authorizer.is_some()))
        {
            return Err(scoped::missing_scope(name));
        }
        inner
            .servers
            .get(name)
            .filter(|server| {
                !server.peer.is_transport_closed() && !server.events.shutdown.is_cancelled()
            })
            .cloned()
            .ok_or_else(|| McpError::ServerNotConnected(name.to_string()))
    }

    async fn disconnected_server_names(&self) -> Vec<String> {
        let inner = self.inner.read().await;
        inner
            .specs
            .iter()
            .filter(|(_, stored)| !stored.requires_scope(self.http_network_authorizer.is_some()))
            .map(|(name, _)| name)
            .filter(|name| {
                inner
                    .servers
                    .get(*name)
                    .is_none_or(|server| server.peer.is_transport_closed())
            })
            .cloned()
            .collect()
    }

    #[allow(deprecated)]
    fn client_handler(
        &self,
        server_name: &str,
        events: Arc<ServerEventState>,
        tool_policy: McpToolPolicy,
        request_timeout: Duration,
        redactions: McpRedactions,
    ) -> AgenaMcpClientHandler {
        let mut roots = RootsCapabilities::default();
        roots.list_changed = Some(true);
        let mut capabilities = ClientCapabilities::default();
        capabilities.roots = Some(roots);
        let mut sampling = rmcp::model::SamplingCapability::default();
        sampling.tools = Some(Default::default());
        sampling.context = Some(Default::default());
        capabilities.sampling = Some(sampling);
        let mut extensions = elicitation::openai_capabilities();
        extensions.insert(rmcp::model::TASKS_EXTENSION_ID.into(), Default::default());
        extensions.insert(skills::EXTENSION_ID.into(), Default::default());
        if self.apps_enabled {
            extensions.insert(
                crate::apps::APP_EXTENSION_ID.into(),
                serde_json::Map::from_iter([(
                    "mimeTypes".into(),
                    serde_json::json!([crate::apps::APP_MIME_TYPE]),
                )]),
            );
        }
        capabilities.extensions = Some(extensions);
        // The legacy capability declares the requestor role. Agena never
        // advertises accepting server-created sampling or elicitation tasks.
        capabilities.legacy_tasks = Some(Default::default());
        capabilities.elicitation = Some(
            ElicitationCapability::new()
                .with_form(FormElicitationCapability::new().with_schema_validation(true))
                .with_url(UrlElicitationCapability::new()),
        );
        AgenaMcpClientHandler {
            info: ClientInfo::new(
                capabilities,
                Implementation::new(self.client_name.clone(), self.client_version.clone()),
            ),
            roots: Arc::clone(&self.roots),
            events,
            refresh_catalog_in_background: self.http_background_enabled,
            request_timeout,
            server_name: server_name.to_string(),
            tool_policy,
            redactions,
        }
    }

    async fn record_error(&self, name: &str, error: &McpError) {
        let failure = mcp_failure(error);
        warn!(
            target: "agena_mcp_client::manager",
            failure_id = %failure.id,
            server = %name,
            category = %error.user_message(),
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
        McpError::Timeout(_) => (
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
        McpError::ToolDisallowed { .. } | McpError::PermissionDenied => (
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
        McpError::UnconfirmedClosure => (
            "mcp.close_unconfirmed",
            FailureCategory::DependencyUnavailable,
            FailureResponsibility::Dependency,
            RetryDirective::AfterUserAction,
            RecoveryDirective::RestartPlugin,
            "The previous MCP connection did not confirm closure.",
        ),
        McpError::SamplingDeclined => (
            "mcp.sampling_declined",
            FailureCategory::PermissionDenied,
            FailureResponsibility::Policy,
            RetryDirective::AfterUserAction,
            RecoveryDirective::RequestPermission,
            "The MCP sampling request was declined or cancelled.",
        ),
        McpError::SamplingUnsupported | McpError::CapabilityUnavailable { .. } => (
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

fn require_capability(server: &ConnectedServer, capability: &str) -> McpResult<()> {
    let supported = server
        .peer
        .peer_info()
        .is_some_and(|info| match capability {
            "tools" => info.capabilities.tools.is_some(),
            "resources" => info.capabilities.resources.is_some(),
            "prompts" => info.capabilities.prompts.is_some(),
            _ => false,
        });
    if supported {
        Ok(())
    } else {
        Err(McpError::CapabilityUnavailable {
            server: server.name.clone(),
            capability: capability.to_owned(),
        })
    }
}

fn client_lifecycle() -> ClientLifecycleMode {
    ClientLifecycleMode::Auto {
        preferred_versions: vec![ProtocolVersion::V_2026_07_28],
        legacy_version: Some(ProtocolVersion::V_2025_11_25),
    }
}

/// Modern peers deliver catalog changes through subscriptions/listen; legacy
/// peers continue using the ordinary ClientHandler notification callbacks.
fn start_catalog_subscription(handler: AgenaMcpClientHandler, peer: Peer<RoleClient>) {
    let Some(info) = peer.peer_info() else {
        return;
    };
    if info.protocol_version < ProtocolVersion::V_2026_07_28 {
        return;
    }
    let mut filter = SubscriptionFilter::default();
    filter.tools_list_changed = info
        .capabilities
        .tools
        .as_ref()
        .and_then(|value| value.list_changed)
        .filter(|value| *value);
    filter.resources_list_changed = info
        .capabilities
        .resources
        .as_ref()
        .and_then(|value| value.list_changed)
        .filter(|value| *value);
    filter.prompts_list_changed = info
        .capabilities
        .prompts
        .as_ref()
        .and_then(|value| value.list_changed)
        .filter(|value| *value);
    if filter.tools_list_changed.is_none()
        && filter.resources_list_changed.is_none()
        && filter.prompts_list_changed.is_none()
    {
        return;
    }
    drop(info);
    tokio::spawn(async move {
        let mut retry_delay = Duration::from_secs(1);
        loop {
            if peer.is_transport_closed() {
                return;
            }
            let opened = tokio::select! {
                biased;
                _ = handler.events.shutdown.cancelled() => return,
                result = tokio::time::timeout(handler.request_timeout, peer.listen(filter.clone())) => result,
            };
            let failure = match opened {
                Ok(Ok(mut subscription)) => {
                    retry_delay = Duration::from_secs(1);
                    // Reconcile any changes between initial discovery and the
                    // subscription acknowledgment, or during a dropped stream.
                    if filter.tools_list_changed == Some(true) {
                        handler.tool_list_changed(peer.clone());
                    }
                    if filter.resources_list_changed == Some(true) {
                        handler.resource_list_changed();
                    }
                    if filter.prompts_list_changed == Some(true) {
                        handler
                            .events
                            .prompt_generation
                            .fetch_add(1, Ordering::Relaxed);
                    }
                    loop {
                        let next = tokio::select! {
                            biased;
                            _ = handler.events.shutdown.cancelled() => return,
                            next = subscription.next() => next,
                        };
                        match next {
                            Ok(Some(ServerNotification::ToolListChangedNotification(_))) => {
                                handler.tool_list_changed(peer.clone())
                            }
                            Ok(Some(ServerNotification::ResourceListChangedNotification(_))) => {
                                handler.resource_list_changed();
                            }
                            Ok(Some(ServerNotification::ResourceUpdatedNotification(_))) => {
                                handler
                                    .events
                                    .resource_generation
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            Ok(Some(ServerNotification::PromptListChangedNotification(_))) => {
                                handler
                                    .events
                                    .prompt_generation
                                    .fetch_add(1, Ordering::Relaxed);
                            }
                            Ok(Some(_)) => {}
                            Ok(None) => break None,
                            Err(error) => break Some(McpError::from(error)),
                        }
                    }
                }
                Ok(Err(error)) => Some(McpError::from(error)),
                Err(error) => Some(McpError::timeout_error(
                    "MCP catalog subscription timed out",
                    &error,
                )),
            };
            if let Some(error) = failure {
                *handler.events.last_refresh_failure.write().await = Some(mcp_failure(&error));
            }
            tokio::select! {
                biased;
                _ = handler.events.shutdown.cancelled() => return,
                _ = tokio::time::sleep(retry_delay) => {}
            }
            retry_delay = (retry_delay * 2).min(Duration::from_secs(30));
        }
    });
}

fn observe_transport_close(events: &ServerEventState, peer: Peer<RoleClient>) {
    let observations = events.observations.clone();
    let shutdown = events.shutdown.clone();
    tokio::spawn(async move {
        tokio::select! {
            _ = shutdown.cancelled() => {}
            _ = peer.transport_closed() => {
                shutdown.cancel();
                if let Some(observations) = observations {
                    observations.send_modify(|revision| *revision += 1);
                }
            }
        }
    });
}

fn redacted_endpoint(value: &str) -> Option<String> {
    let mut url = Url::parse(value).ok()?;
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
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
                        category = %error.user_message(),
                        "MCP reconnect supervisor attempt failed"
                    );
                }
            }
        }
        drop(manager);
        tokio::time::sleep(policy.poll_interval).await;
    }
}

fn mcp_roots(paths: impl IntoIterator<Item = PathBuf>) -> Vec<Root> {
    let mut uris = std::collections::BTreeSet::new();
    paths
        .into_iter()
        .filter_map(|path| {
            let uri = match Url::from_directory_path(path.as_path()) {
                Ok(uri) => uri,
                Err(()) => {
                    tracing::warn!(
                        root_path = %path.display(),
                        "MCP root was omitted because the directory path cannot be represented as a file URL"
                    );
                    return None;
                }
            };
            let name = path
                .file_name()
                .and_then(|value| value.to_str())
                .map(str::to_owned);
            let uri = uri.to_string();
            if !uris.insert(uri.clone()) {
                return None;
            }
            let mut root = Root::new(uri);
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
    sandbox: Option<agena_process::sandbox::SandboxPolicy>,
    plugin_scoped: bool,
) -> McpResult<RunningClient> {
    let mut cmd = Command::new(command);
    if plugin_scoped {
        cmd.env_clear();
        if let Some(path) = std::env::var_os("PATH") {
            cmd.env("PATH", path);
        }
    }
    cmd.args(args);
    cmd.envs(env);
    if let Some(cwd) = cwd {
        cmd.current_dir(cwd);
    }
    let policy = sandbox.ok_or_else(|| {
        McpError::Transport("MCP stdio requires a host-authorized sandbox scope".into())
    })?;
    agena_process::sandbox::wrap_tokio_command(&mut cmd, &policy)
        .map_err(|error| McpError::Transport(format!("MCP sandbox launch failed: {error}")))?;
    let transport = TokioChildProcess::new(agena_process::wrap_command(cmd))?;
    Ok(client_handler
        .serve_with_lifecycle(transport, client_lifecycle())
        .await?)
}

async fn connect_http(
    client_handler: AgenaMcpClientHandler,
    server_name: &str,
    url: Url,
    transport: McpHttpTransport,
    mut headers: HashMap<String, String>,
    header_helper: Option<crate::McpHeaderHelper>,
    auth: Option<HttpAuth>,
    token_store: Option<Arc<dyn TokenStore>>,
    network_authorizer: Option<Arc<dyn crate::McpHttpNetworkAuthorizer>>,
    background_enabled: bool,
) -> McpResult<RunningClient> {
    let credential_name = credential_store_name(server_name, &url);
    let mut has_authorization = headers
        .keys()
        .any(|key| key.eq_ignore_ascii_case("authorization"));
    let oauth_binding = match auth.as_ref() {
        Some(HttpAuth::OAuth {
            scopes,
            registration,
        }) => Some(oauth_credential_binding(scopes, registration)),
        _ => None,
    };
    if oauth_binding.is_none()
        && let Some(auth) = auth
    {
        // TokenStore is a synchronous port and may access the private secret store.
        // Its wait must be outside Tokio workers for connect_timeout to work.
        if matches!(auth, HttpAuth::BearerFromStore) {
            let credential_name = credential_name.clone();
            headers = crate::token_store::SECRET_STORE_WORKERS
                .run(move || {
                    apply_http_auth(&credential_name, auth, &mut headers, token_store.as_deref())?;
                    Ok::<_, McpError>(headers)
                })
                .await
                .map_err(|error| McpError::auth_error(&error))??;
        } else {
            apply_http_auth(&credential_name, auth, &mut headers, token_store.as_deref())?;
        }
    }

    let dynamic_headers = header_helper.is_some();
    if let Some(helper) = header_helper {
        let authorizer = network_authorizer
            .as_deref()
            .ok_or(McpError::PermissionDenied)?;
        helper
            .merge_headers(authorizer, url.as_str(), &mut headers)
            .await?;
        has_authorization = headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("authorization"));
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

    if transport != McpHttpTransport::Streamable {
        if oauth_binding.is_some() {
            return Err(McpError::Auth(
                "this MCP transport requires static request headers".into(),
            ));
        }
        let authority = network_authorizer.ok_or(McpError::PermissionDenied)?;
        let mut headers: http::HeaderMap = custom_headers.into_iter().collect();
        if let Some(token) = bearer {
            headers.insert(
                http::header::AUTHORIZATION,
                parse_header_value(&format!("Bearer {token}"))?,
            );
        }
        return match transport {
            McpHttpTransport::Sse => Ok(client_handler
                .serve_with_lifecycle(
                    crate::legacy_sse::connect(url, headers, authority, !background_enabled)
                        .await?,
                    ClientLifecycleMode::Initialize,
                )
                .await?),
            McpHttpTransport::WebSocket => Ok(client_handler
                .serve_with_lifecycle(
                    crate::websocket::connect(url, headers, authority, !background_enabled).await?,
                    ClientLifecycleMode::Initialize,
                )
                .await?),
            McpHttpTransport::Streamable => unreachable!("handled below"),
        };
    }

    let mut config = StreamableHttpClientTransportConfig::with_uri(url.to_string())
        .custom_headers(custom_headers)
        .reinit_on_expired_session(background_enabled);
    config.listen_for_server_messages = background_enabled;
    let app_events = (!background_enabled).then(|| client_handler.events.app_http_events.clone());
    if let Some(binding) = oauth_binding {
        if bearer.is_some() || has_authorization {
            return Err(McpError::Auth(
                "OAuth MCP configuration must not also supply an Authorization header".to_owned(),
            ));
        }
        let store = FileOAuthCredentialStore::new(&credential_name)
            .map_err(|error| McpError::auth_error(&error))?
            .with_binding(binding);
        let oauth_client = Arc::new(crate::oauth_http::AuthorizedOAuthHttpClient::for_transport(
            network_authorizer.clone(),
        ));
        let mut manager =
            AuthorizationManager::new_with_oauth_http_client(url.clone(), oauth_client)
                .await
                .map_err(|error| McpError::auth_error(&error))?;
        manager.set_credential_store(store);
        if !manager
            .initialize_from_store()
            .await
            .map_err(|error| McpError::auth_error(&error))?
        {
            return Err(McpError::Auth(format!(
                "OAuth authorization is required for MCP server '{server_name}'; sign in from MCP connections settings"
            )));
        }
        if let Some(authorizer) = network_authorizer {
            let transport = StreamableHttpClientTransport::with_client(
                AuthClient::new(
                    crate::http_transport::authorized_mcp_client(
                        authorizer,
                        false,
                        client_handler.events.http_session_open.clone(),
                        app_events,
                    )?,
                    manager,
                ),
                config,
            );
            return Ok(client_handler
                .serve_with_lifecycle(transport, client_lifecycle())
                .await?);
        }
        let http_client = crate::http_transport::trusted_client()?;
        let transport = StreamableHttpClientTransport::with_client(
            AuthClient::new(http_client, manager),
            config,
        );
        return Ok(client_handler
            .serve_with_lifecycle(transport, client_lifecycle())
            .await?);
    }
    let config = match bearer {
        Some(token) => config.auth_header(token),
        None => config,
    };
    if let Some(authorizer) = network_authorizer {
        let transport = StreamableHttpClientTransport::with_client(
            crate::http_transport::authorized_mcp_client(
                authorizer,
                dynamic_headers,
                client_handler.events.http_session_open.clone(),
                app_events,
            )?,
            config,
        );
        return Ok(client_handler
            .serve_with_lifecycle(transport, client_lifecycle())
            .await?);
    }
    let http_client = crate::http_transport::trusted_client()?;
    let transport = StreamableHttpClientTransport::with_client(http_client, config);
    Ok(client_handler
        .serve_with_lifecycle(transport, client_lifecycle())
        .await?)
}

async fn shutdown_server(server: Arc<ConnectedServer>) -> bool {
    // Take ownership of the cancellable connection before awaiting its
    // shutdown. A cancellation may notify tasks that need to inspect the
    // same `running` slot; holding the mutex across that await creates an
    // avoidable lock cycle.
    let running = server.running.lock().await.take();
    if let Some(running) = running {
        server.events.shutdown.cancel();
        shutdown_client(&server.name, running, server.close_timeout).await
            && !server.events.http_session_open.load(Ordering::Acquire)
    } else {
        true
    }
}

async fn shutdown_client(server: &str, running: RunningClient, timeout: Duration) -> bool {
    match tokio::time::timeout(timeout, running.cancel()).await {
        Ok(Ok(_reason)) => return true,
        Ok(Err(_error)) => tracing::warn!(server, "failed to shut down an MCP connection"),
        Err(error) => tracing::warn!(
            server,
            timeout_ms = timeout.as_millis(),
            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                format!("timed out after {}ms while shutting down an MCP connection", timeout.as_millis()),
                &error,
            ),
            "timed out while shutting down an MCP connection"
        ),
    }
    false
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
) -> McpResult<()> {
    let token = match auth {
        HttpAuth::Bearer(token) => Some(token),
        HttpAuth::BearerFromEnv(env_name) => Some(std::env::var(&env_name)
            .ok().filter(|value| !value.trim().is_empty())
            .ok_or_else(|| McpError::Auth("The MCP bearer environment variable is missing or empty.".into()))?),
        HttpAuth::BearerFromStore => Some(token_store.and_then(|store| store.bearer(server))
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| McpError::Auth("No bearer credential is configured for this MCP endpoint. Enter a token in connection settings.".into()))?),
        HttpAuth::OAuth { .. } => unreachable!("OAuth is handled before bearer header setup"),
        HttpAuth::Custom(custom_headers) => {
            for (key, value) in custom_headers {
                headers.entry(key).or_insert(value);
            }
            None
        }
    };
    if let Some(token) = token {
        if token.trim().is_empty() {
            return Err(McpError::Auth("The MCP bearer token is empty.".into()));
        }
        if !headers
            .keys()
            .any(|key| key.eq_ignore_ascii_case("authorization"))
        {
            headers.insert("Authorization".into(), format!("Bearer {token}"));
        }
    }
    Ok(())
}

fn parse_header_name(name: &str) -> McpResult<HeaderName> {
    HeaderName::try_from(name)
        .map_err(|err| McpError::Http(format!("invalid HTTP header name '{name}': {err}")))
}

fn parse_header_value(value: &str) -> McpResult<HeaderValue> {
    HeaderValue::from_str(value)
        .map_err(|err| McpError::Http(format!("invalid HTTP header value: {err}")))
}

async fn value_to_json_object(value: Option<Value>) -> McpResult<Option<rmcp::model::JsonObject>> {
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Object(map)) if map.is_empty() => Ok(None),
        Some(Value::Object(map)) => Ok(Some(map)),
        Some(other) => Err(project_payload(move || {
            McpError::Malformed(format!("tool arguments must be a JSON object, got {other}"))
        })
        .await?),
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
        execution: tool.execution.map(Value::Object),
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
            raw: match serde_json::to_value(other) {
                Ok(raw) => raw,
                Err(error) => {
                    tracing::error!(
                        diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                            "serialize an unknown MCP content block",
                            &error,
                        ),
                        "unknown MCP content block could not be preserved verbatim"
                    );
                    serde_json::json!({
                        "projection_error": "The MCP content block could not be encoded."
                    })
                }
            },
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
    value.and_then(|value| match serde_json::to_value(value) {
        Ok(value) => Some(value),
        Err(error) => {
            tracing::error!(
                serialized_type = std::any::type_name::<T>(),
                diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize optional MCP descriptor metadata",
                    &error,
                ),
                "optional MCP descriptor metadata was omitted after serialization failure"
            );
            None
        }
    })
}

fn serialize_values<T: serde::Serialize>(values: Vec<T>) -> Vec<Value> {
    values
        .into_iter()
        .filter_map(|value| match serde_json::to_value(value) {
            Ok(value) => Some(value),
            Err(error) => {
                tracing::error!(
                    serialized_type = std::any::type_name::<T>(),
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "serialize MCP descriptor metadata item",
                        &error,
                    ),
                    "MCP descriptor metadata item was omitted after serialization failure"
                );
                None
            }
        })
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
    /// secret-store failure from an absent token should override this rather
    /// than collapsing it to `Missing`.
    fn credential_state(&self, server: &str) -> McpCredentialState {
        if self.bearer(server).is_some() {
            McpCredentialState::Configured
        } else {
            McpCredentialState::Missing
        }
    }
}
