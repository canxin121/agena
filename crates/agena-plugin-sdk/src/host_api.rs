//! `HostClient` — the API plugins use to call back into the host.
//!
//! At runtime the host injects a concrete impl into `Plugin::init`. For tests
//! and minimal plugins, [`NoopHostClient`] returns errors for everything.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::Arc;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::attachment::AttachmentItem;
use crate::browser::BrowserController;
use crate::error::{PluginError, Result};
use crate::hooks::{EventEnvelope, EventFilter, ToolInvokeOutput};
use crate::identity::{PluginKey, ToolKey};
use crate::manifest::{
    CommandDefinition, PathKind, PluginDisplayContribution, PluginTerminalThemeColors,
    ToolDefinition,
};
pub use agena_domain::{BackgroundActivity, BackgroundActivityKind};
mod bot;
mod companion;
mod cordis_jobs;
mod cordis_llm;
mod cordis_subagents;
mod cordis_workflows;
mod integrations;
mod mcp_apps;
mod operations;
pub use operations::HostClientExt;
mod session;
mod workspace;
pub use bot::{BotAction, BotProfileRequest, BotRequest};
pub use companion::{CompanionAction, CompanionRequest};
pub use cordis_jobs::{
    CordisJobCancelReply, CordisJobCancellation, CordisJobControl, CordisJobControlSender,
    CordisJobEvent, CordisJobInvocation, CordisJobOutcome, CordisJobProducer, CordisJobQuery, CordisJobRegistry,
    CordisJobSettlementCause, CordisJobSpec, CordisJobStart, CordisJobStarter, CordisJobStatus,
    CordisJobUpdate,
};
pub use cordis_llm::{CordisLlmMessage, CordisLlmRequest, CordisLlmResponse, CordisLlmUsage};
pub use cordis_subagents::{
    CordisSubagentControl, CordisSubagentDelivery, CordisSubagentResult, CordisSubagentRun,
    CordisSubagentStart, CordisSubagentStarted, CordisSubagentStopReason, CordisSubagentWait,
};
pub use cordis_workflows::{
    CordisWorkflowJobHost, CordisWorkflowJobRequest, CordisWorkflowResult, CordisWorkflowRunner,
    CordisWorkflowStart, CordisWorkflowStarted,
    CordisWorkflowSettlement, CordisWorkflowStopReason,
};
pub use integrations::{IntegrationAction, IntegrationRequest};
pub use mcp_apps::{
    McpAppCatalogSnapshot, McpAppCatalogWatch, McpAppCatalogWatches, McpAppHostBinding,
    McpAppHostRequest, McpAppLegacyTaskView, McpAppServerTaskPort, mcp_app_request_binding,
    mcp_app_resource_catalog_wait, mcp_app_server_task, scope_mcp_app_request,
};
pub use session::{
    HostExecutionList, HostExecutionSummary, HostPartsPage, HostQueuedMessage, HostSessionAgent,
    HostSessionModel, HostSessionState, HostSessionStreamKind, HostSessionStreamRequest, SessionOp,
    SessionOpResponse,
};
pub use workspace::{HostWorkspaceQuery, HostWorkspaceQueryRequest, HostWorkspaceQueryResponse};

/// Resources reported by a native, host-managed child process.
/// With MXC enabled, the host compiles configured permissions directly. These
/// declarations are fallback checks when MXC is off and resource_guard is on;
/// they never grant authority beyond the current host scope.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct HostProcessResources {
    pub cwd: Option<String>,
    /// Refuse native execution before requesting any fallback permissions.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub require_sandbox: bool,
    pub reads: Vec<String>,
    pub writes: Vec<String>,
    /// Optional fallback outbound targets, checked only for direct execution
    /// with resource_guard enabled. MXC uses the full configured native network
    /// policy, including every independent ceiling and persisted denial.
    pub network: Vec<String>,
}

/// A native host result. Remote plugin RPC never accepts or returns this as
/// an authorization capability; only the host constructs the policy value.
#[derive(Debug, Clone)]
pub struct HostPreparedProcessSandbox {
    pub scope: String,
    pub workspace_root: String,
    pub cwd: String,
    pub policy: serde_json::Value,
    /// Additional working directories the owning session can read.
    pub additional_working_directories: Vec<std::path::PathBuf>,
    /// Current authority for trusted native workers that validate individual
    /// resources inside the kernel boundary. Never exposed through plugin RPC.
    pub permission_snapshot: Option<serde_json::Value>,
    /// A one-call approval or operator scope must end with its originating call.
    pub call_bound: bool,
    /// Execution metadata when the originating callback belongs to a live tool.
    pub execution: Option<HostProcessExecution>,
}

#[derive(Debug, Clone)]
pub struct HostProcessExecution {
    pub id: String,
    pub model: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// MCP server specification for permission checks.
pub enum HostMcpServerSpec {
    Stdio {
        command: String,
        #[serde(default)]
        args: Vec<String>,
        #[serde(default)]
        env: BTreeMap<String, String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
    },
    Http {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        bearer: Option<String>,
        #[serde(default)]
        headers: BTreeMap<String, String>,
    },
}

/// Reserved `HostCallbackContext::call_id` for a session-scoped callback.
///
/// A session hook (`session.start`, `chat.*`, `run.pre`, …) belongs to a
/// session but not to any transcript tool call. Host APIs that normally need a
/// live tool authority accept this value and authorize under the session's
/// policy instead. It never names a real `ToolId`, which is nonnegative.
pub const SESSION_SCOPED_CALL_ID: i64 = -1;

/// Bounded text and outcomes from the owning session, in transcript order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionHistoryEvent {
    User(String),
    Assistant(String),
    ToolCompleted {
        name: String,
        output: String,
        failed: bool,
        user_declined: bool,
    },
    ContextCleared,
}

#[async_trait]
/// Client interface used by plugins to call host capabilities.
pub trait HostClient: Send + Sync + 'static {
    async fn bot_context(&self) -> Result<serde_json::Value> {
        Err(unavailable())
    }
    async fn bot_operation(&self, _request: BotRequest) -> Result<serde_json::Value> {
        Err(unavailable())
    }
    /// Inspect or edit the current Bot from a native tool callback.
    async fn bot_profile_operation(
        &self,
        _request: BotProfileRequest,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn manage_integrations(&self, _request: IntegrationRequest) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn companion_context(&self) -> Result<serde_json::Value> {
        Err(unavailable())
    }
    async fn companion_operation(&self, _request: CompanionRequest) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn log(&self, level: LogLevel, message: String, fields: serde_json::Value);

    async fn publish_event(&self, env: EventEnvelope) -> Result<()>;

    /// Publish a lifecycle observation for notification history.
    async fn publish_activity(&self, activity: BackgroundActivity) -> Result<()> {
        let _ = activity;
        Err(unavailable())
    }

    /// Bind operator controls to the browser plugin instance that owns the calling session.
    async fn register_browser_controller(
        &self,
        _controller: Arc<dyn BrowserController>,
    ) -> Result<()> {
        Err(unavailable())
    }

    async fn subscribe_events(&self, filter: EventFilter) -> Result<EventSubscription>;

    async fn unsubscribe_events(&self, subscription_id: String) -> Result<()> {
        let _ = subscription_id;
        Err(unavailable())
    }

    async fn read_config(&self, path: Option<String>) -> Result<serde_json::Value>;

    // ---------------- permission queries ----------------
    //
    // These are read-only policy lookups. They answer what the effective
    // permission configuration says about a target; they do not gate anything
    // and they do not change policy state. A plugin that performs its own I/O
    // must call `require_*_permission` before touching user paths or network targets, and
    // to honour the answer — but the host cannot enforce that for a plugin
    // that runs its own process or syscalls.

    /// Ask what the effective policy says about one path access.
    async fn check_path_permission(
        &self,
        _request: PathPermissionQuery,
    ) -> Result<PermissionQuery> {
        Err(unavailable())
    }

    /// Ask what the effective policy says about one outbound network target.
    async fn check_network_permission(&self, _target: String) -> Result<PermissionQuery> {
        Err(unavailable())
    }

    /// Root catalog for a native MCP bridge callback in a live project session.
    async fn mcp_session_roots(&self) -> Result<Option<Vec<std::path::PathBuf>>> {
        Ok(None)
    }

    /// Check one numeric DNS result against the already-approved hostname and
    /// port, including IP/CIDR rules and inherited authority ceilings.
    /// Call `require_network_permission` first and pin the permitted addresses
    /// in the actual transport; this query does not grant hostname approval.
    async fn check_resolved_network_permission(
        &self,
        _original: String,
        _resolved: String,
    ) -> Result<PermissionQuery> {
        Err(PluginError::from_kind(
            crate::PluginErrorKind::HostUnavailable,
            "the host cannot validate resolved network addresses",
        ))
    }

    /// Authorize an access before performing I/O. Runtime hosts resolve remembered
    /// rules, automatically review `auto`, and wait for approval for `ask`.
    /// The default supports explicit allow/deny only; hosts without an approval
    /// implementation fail closed with `HostUnavailable` for `Ask`.
    async fn require_path_permission(&self, request: PathPermissionQuery) -> Result<()> {
        match self.check_path_permission(request.clone()).await? {
            PermissionQuery::Allow {} => Ok(()),
            PermissionQuery::Deny { reason } => Err(policy_denied_path(&request, &reason)),
            PermissionQuery::Ask { .. } => Err(unavailable()),
        }
    }

    /// Authorize an outbound connection through the same approval flow as host
    /// tools. Unlike `check_network_permission`, this may wait for the user.
    async fn require_network_permission(&self, target: String) -> Result<()> {
        match self.check_network_permission(target.clone()).await? {
            PermissionQuery::Allow {} => Ok(()),
            PermissionQuery::Deny { reason } => Err(policy_denied_network(&target, &reason)),
            PermissionQuery::Ask { .. } => Err(unavailable()),
        }
    }

    /// Authorize an HTTP(S) URL for a transport that checks and pins its resolved addresses.
    /// A native host may share fresh DNS answers with that transport. Embedders can use
    /// the ordinary network check when the transport resolves separately.
    async fn require_pinned_http_permission(&self, target: String) -> Result<()> {
        self.require_network_permission(target).await
    }

    /// Queue a reload after the originating plugin call (including its nested
    /// calls or stream) finishes. Return acceptance, never a completed report.
    /// Do not wait for completion inside the originating call: it is a barrier.
    /// An accepted task survives cancellation of that call; runtime shutdown
    /// cancels pending work. Separate originating calls may queue separate tasks.
    async fn request_config_reload(&self) -> Result<HostConfigReloadRequestResponse> {
        Err(unavailable())
    }

    /// Query a previously accepted reload task. Task history is bounded; an
    /// unknown/expired ID is reported as an error, never inferred as success.
    async fn config_reload_status(
        &self,
        _request: HostConfigReloadStatusRequest,
    ) -> Result<HostConfigReloadStatusResponse> {
        Err(unavailable())
    }

    async fn invoke_tool(&self, tool: String, input: serde_json::Value)
    -> Result<ToolInvokeOutput>;

    /// Await a task launched by the in-process agent plugin adapter.
    async fn wait_imported_plugin_agent(
        &self,
        _child: i64,
        _timeout_ms: u64,
    ) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    async fn start_cordis_subagent(
        &self,
        _start: CordisSubagentStart,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<CordisSubagentStarted> {
        Err(unavailable())
    }

    async fn wait_for_cordis_subagent(&self, _wait: CordisSubagentWait) -> Result<()> {
        Err(unavailable())
    }

    async fn cordis_subagent_control(
        &self,
        _request: CordisSubagentControl,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn cordis_llm_complete(
        &self,
        _request: CordisLlmRequest,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<CordisLlmResponse> {
        Err(unavailable())
    }

    /// Admit a Cordis workflow under a live imported tool call. The worker gets
    /// job-owned callbacks after the launching tool returns.
    async fn start_cordis_workflow(
        &self,
        _start: CordisWorkflowStart,
        _run: CordisWorkflowRunner,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<CordisWorkflowStarted> {
        Err(unavailable())
    }

    /// Admit a JobSpec under a live imported tool. Only its producer callbacks
    /// survive the tool's host call scope.
    async fn start_cordis_job(
        &self,
        _start: CordisJobStart,
        _produce: CordisJobStarter,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<String> {
        Err(unavailable())
    }

    async fn open_cordis_job_registry(
        &self,
        _lifetime: tokio_util::sync::CancellationToken,
    ) -> Result<Arc<dyn CordisJobRegistry>> {
        Err(unavailable())
    }

    async fn cordis_workflow_job(
        &self,
        _request: CordisWorkflowJobRequest,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    /// Look up a direct child by its recorded launch name during workflow replay.
    async fn find_imported_plugin_agent(
        &self,
        _name: String,
    ) -> Result<Option<ImportedPluginAgentState>> {
        Err(unavailable())
    }

    /// Evaluate an imported prompt hook with the session's Agena model configuration.
    async fn evaluate_imported_plugin_prompt(&self, _prompt: String) -> Result<String> {
        Err(unavailable())
    }

    /// Evaluate an imported agent hook with access to authorized read-only tools.
    async fn evaluate_imported_plugin_agent(&self, _prompt: String) -> Result<String> {
        Err(unavailable())
    }

    /// Obtain a notice sink bound to the current imported plugin generation
    /// and its session. It remains usable after the originating hook returns.
    async fn bind_imported_plugin_background(&self) -> Result<ImportedPluginBackground> {
        Err(unavailable())
    }

    /// Submit text as the current imported plugin in its authorized session.
    async fn submit_imported_plugin_prompt(
        &self,
        _request: ImportedPluginPromptRequest,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    /// Inspect or invoke tools using the current imported Claude mod's session.
    async fn imported_mod_tool(
        &self,
        _request: ImportedModToolRequest,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    /// Update the current session's imported plugin file watches.
    async fn configure_imported_plugin_files(
        &self,
        _paths: Option<Vec<std::path::PathBuf>>,
        _callback_generation: Option<u64>,
    ) -> Result<()> {
        Err(unavailable())
    }

    /// The current shell environment file for a native imported hook.
    async fn imported_hook_environment(
        &self,
        _callback_generation: Option<u64>,
    ) -> Result<Option<ImportedHookEnvironment>> {
        Ok(None)
    }

    /// Files for an in-process adapter's local command hook. Scope is relative
    /// to the authorized callback session and cannot name a different session.
    async fn imported_hook_transcripts(
        &self,
        _format: HookTranscriptFormat,
        _scope: HookTranscriptScope,
        _callback_generation: Option<u64>,
    ) -> Result<ImportedHookTranscripts> {
        Ok(ImportedHookTranscripts::default())
    }

    /// Read active work for a Claude Stop hook, relative to its callback
    /// session. SubagentStop can request its verified parent's work.
    async fn imported_claude_stop_work(
        &self,
        _parent: bool,
        _callback_generation: Option<u64>,
    ) -> Result<Option<crate::ImportedClaudeStopWork>> {
        Ok(None)
    }

    /// Visit this session's durable history during an authorized native
    /// disposal hook. The caller cannot name another session or retain a reader.
    async fn visit_ending_session_history(
        &self,
        _visit: &mut (dyn FnMut(SessionHistoryEvent) + Send),
    ) -> Result<()> {
        Err(unavailable())
    }

    /// Effective role of the callback session for frontmatter hook filtering.
    async fn imported_hook_profile(&self) -> Result<Option<String>> {
        Ok(None)
    }

    /// Check a task mutation using the active imported hooks. This in-process
    /// capability is restricted to the built-in task bridge.
    async fn check_imported_task(&self, _input: crate::ImportedTaskBeforeInput) -> Result<()> {
        Err(unavailable())
    }

    /// Identity of an active imported plugin's contribution to the MCP catalog.
    async fn imported_mcp_server_identity(
        &self,
        _id: &str,
    ) -> Result<Option<ImportedMcpServerIdentity>> {
        Ok(None)
    }

    /// Invoke an MCP handler using the owning session's tool and resource policy.
    async fn call_imported_plugin_mcp(
        &self,
        _server: String,
        _tool: String,
        _arguments: serde_json::Value,
    ) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    /// Read a resource on a server contributed by the current imported package.
    async fn read_imported_plugin_mcp(
        &self,
        _server: String,
        _uri: String,
    ) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }

    /// Session-scoped management of MCP servers declared by a DeepSeek bundle.
    async fn deepseek_mcp_manager(
        &self,
        _request: DeepSeekMcpManagerRequest,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    /// Observe an imported Cordis module's project session across command calls.
    /// Native adapters consume the stream without retaining callback authority.
    async fn observe_deepseek_mcp_manager(
        &self,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<tokio::sync::watch::Receiver<serde_json::Value>> {
        Err(unavailable())
    }

    /// Let the bundled MCP tool participate in a managed HTTP session if the
    /// owning project's policy has all required grants independently of this call.
    async fn prepare_managed_mcp_http(&self, _server: String) -> Result<bool> {
        Ok(false)
    }

    /// Execute shell snippets from an imported Markdown command under its
    /// session's existing authorization. Native Claude package adapter only.
    async fn run_imported_skill_shell(
        &self,
        _request: crate::ImportedSkillShellRequest,
    ) -> Result<crate::ImportedSkillShellResult> {
        Err(unavailable())
    }

    /// Render an exact Claude skill already read by the built-in skills tool.
    /// Native callback. No expansion applies to other skill sources.
    async fn render_imported_skill(
        &self,
        _path: &std::path::Path,
        _document: &str,
        _arguments: &str,
    ) -> Result<Option<crate::ImportedSkillRender>> {
        Ok(None)
    }

    /// Read native provider skills visible to the built-in skills tool.
    async fn imported_model_skills(&self) -> Result<Vec<agena_skills::DeclaredSkill>> {
        Ok(Vec::new())
    }

    /// Invoke one explicitly imported service. The host resolves the provider
    /// from the immutable manifest graph; callers cannot select or discover a
    /// provider at invocation time.
    async fn invoke_service(
        &self,
        _req: crate::PluginServiceInvokeInput,
    ) -> Result<crate::PluginServiceInvokeOutput> {
        Err(unavailable())
    }

    // ---------------- Host workflow capabilities ----------------
    //
    // These are optional host APIs any plugin may call; hosts that do not
    // implement one return `HostUnavailable`.    /// Prompt the user for input via the active session UI (used by the
    /// `ask_user` tool).
    async fn ask_user(&self, _req: AskUserRequest) -> Result<AskUserResponse> {
        Err(unavailable())
    }

    /// Create an attached agent and return after its launch is durable.
    async fn spawn_agent(&self, _req: AgentSpawnRequest) -> Result<AgentHandle> {
        Err(unavailable())
    }

    /// Queue a message without starting an idle recipient.
    async fn message_agent(&self, _req: AgentMessageRequest) -> Result<AgentMessageReceipt> {
        Err(unavailable())
    }

    /// Wait for this callback session's mailbox, new input, or a timeout.
    async fn wait_agent(&self, _req: AgentWaitRequest) -> Result<AgentWaitResponse> {
        Err(unavailable())
    }

    /// Deliver more work to a child, starting an execution when it is idle.
    async fn followup_agent(&self, _req: AgentMessageRequest) -> Result<AgentHandle> {
        Err(unavailable())
    }

    /// Cancel a direct child's execution without deleting its session.
    async fn cancel_agent(&self, _req: AgentTargetRequest) -> Result<AgentMessageReceipt> {
        Err(unavailable())
    }

    /// List all currently registered tools (used by `tool_search`).
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>> {
        Err(unavailable())
    }

    /// Read the current session metadata.
    async fn get_session(&self, _req: HostGetSessionRequest) -> Result<HostGetSessionResponse> {
        Err(unavailable())
    }

    /// Read safe context-window and compaction status for a session.
    async fn get_context_status(
        &self,
        _req: HostContextStatusRequest,
    ) -> Result<HostContextStatusResponse> {
        Err(unavailable())
    }

    /// Rename the current session.
    async fn rename_session(
        &self,
        _req: HostRenameSessionRequest,
    ) -> Result<HostRenameSessionResponse> {
        Err(unavailable())
    }

    /// Run one session operation.
    ///
    /// `SessionOp` wraps the same `Query`/`Command` enums every other transport
    /// uses, so a plugin can reach exactly what the Web and terminal clients
    /// reach. A new operation is added to those enums once; the host's
    /// exhaustive match then fails to compile until it is routed.
    async fn session_op(&self, _op: SessionOp) -> Result<SessionOpResponse> {
        Err(unavailable())
    }

    /// Execute any shared application query and retain its result envelope.
    async fn query(&self, query: agena_session_api::queries::Query) -> Result<serde_json::Value> {
        operations::response_value(self.session_op(SessionOp::Query(query)).await?, false)
    }

    /// Execute any shared application command and retain its result envelope.
    async fn command(
        &self,
        command: agena_session_api::commands::Command,
    ) -> Result<serde_json::Value> {
        operations::response_value(self.session_op(SessionOp::Command(command)).await?, true)
    }

    /// Subscribe to a live session stream. Returns a subscription id; events are
    /// delivered through the existing `subscribe_events` channel as
    /// `EventEnvelope` values whose payload decodes to `SessionChangeResource`.
    async fn session_stream(&self, _req: HostSessionStreamRequest) -> Result<EventSubscription> {
        Err(unavailable())
    }

    /// Read projects and their files, subject to the host's path permission
    /// fence. `Read` never returns more than the host's bounded read budget.
    async fn workspace_query(
        &self,
        _req: HostWorkspaceQueryRequest,
    ) -> Result<HostWorkspaceQueryResponse> {
        Err(unavailable())
    }

    /// Resolve an ephemeral connection for the bundled ChatGPT, Claude and
    /// Gemini tools against the active session's provider. This native-only
    /// callback is deliberately not exposed over plugin RPC: its result can
    /// contain credentials and must never become tool output or persisted settings.
    async fn builtin_provider_connection(&self) -> Result<BuiltinProviderConnection> {
        Err(unavailable())
    }

    /// Discover all enabled adapters of the active provider, retaining model
    /// provenance and partial failures. Native bundled provider plugin only;
    /// the returned catalog never contains credentials.
    async fn builtin_provider_models(&self) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    /// Execute a terminal image generate/edit request against the current
    /// session's selected provider/model route. The host validates inputs and
    /// persists provider output before returning any attachment.
    async fn image_execute(
        &self,
        _req: HostImageExecuteRequest,
    ) -> Result<HostImageExecuteResponse> {
        Err(unavailable())
    }

    /// Dynamic command registry — register a new user-facing command owned by
    /// this plugin. Commands are the only user-facing surface of their kind:
    /// both clients read them from the server catalog.
    async fn register_command(
        &self,
        _req: HostCommandRegisterRequest,
    ) -> Result<HostCommandMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic command registry — replace the definition of an existing command
    /// owned by this plugin (matched by `command.id`).
    async fn update_command(
        &self,
        _req: HostCommandUpdateRequest,
    ) -> Result<HostCommandMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic command registry — remove a command owned by this plugin.
    async fn remove_command(
        &self,
        _req: HostCommandRemoveRequest,
    ) -> Result<HostCommandMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic command registry — list the commands visible to the caller.
    async fn list_registered_commands(&self) -> Result<HostRegisteredCommandListResponse> {
        Err(unavailable())
    }

    /// LSP observability — list configured language servers.
    ///
    /// Kept as a host API because `agena.lsp.servers` is a plugin-implemented
    /// tool: its handler reads this projection directly.
    async fn lsp_list_servers(&self) -> Result<HostLspListServersResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — register a new tool owned by this plugin.
    async fn register_tool(
        &self,
        _req: HostToolRegisterRequest,
    ) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — replace the definition of an existing tool owned by
    /// this plugin (matched by `tool.name`).
    async fn update_tool(&self, _req: HostToolUpdateRequest) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — remove a tool owned by this plugin.
    async fn remove_tool(&self, _req: HostToolRemoveRequest) -> Result<HostToolMutationResponse> {
        Err(unavailable())
    }

    /// Dynamic tool registry — list all registered tools known to the plugin host.
    async fn list_registered_tools(&self) -> Result<HostRegisteredToolListResponse> {
        Err(unavailable())
    }

    /// List every plugin loaded by the host, with manifest metadata (id,
    /// version, summary, tags) so plugins can discover each other without
    /// static host-capability gating.
    async fn list_plugins(&self) -> Result<HostPluginListResponse> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — read.
    async fn storage_get(&self, _req: HostStorageGetRequest) -> Result<HostStorageGetResponse> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — write.
    async fn storage_set(&self, _req: HostStorageSetRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Atomically replace an in-process plugin record if its prior value matches.
    async fn storage_compare_exchange(
        &self,
        _req: HostStorageSetRequest,
        _expected: Option<String>,
    ) -> Result<bool> {
        Err(unavailable())
    }

    /// Private, instance-specific data directory for the native agent plugin
    /// importer. This callback is only available to the in-process adapter.
    async fn imported_plugin_storage_directory(&self) -> Result<std::path::PathBuf> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — delete.
    async fn storage_delete(&self, _req: HostStorageDeleteRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-namespaced KV storage — enumerate keys.
    async fn storage_list(&self, _req: HostStorageListRequest) -> Result<HostStorageListResponse> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — read a secret value.
    async fn secret_get(&self, _req: HostSecretGetRequest) -> Result<HostSecretGetResponse> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — write a secret value.
    async fn secret_set(&self, _req: HostSecretSetRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — delete a secret value.
    async fn secret_delete(&self, _req: HostSecretDeleteRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Plugin-scoped secret storage — list secret names (never values).
    async fn secret_list(&self) -> Result<HostSecretListResponse> {
        Err(unavailable())
    }

    /// Daemon lifecycle — list every plugin status known to the host.
    async fn plugin_status_list(&self) -> Result<HostPluginStatusListResponse> {
        Err(unavailable())
    }

    /// Daemon lifecycle — fetch a single plugin status by id.
    async fn plugin_status_get(
        &self,
        _req: HostPluginStatusGetRequest,
    ) -> Result<HostPluginStatusGetResponse> {
        Err(unavailable())
    }

    /// Hook registry — list every hook currently subscribed across plugins.
    async fn hook_list(&self) -> Result<HostHookListResponse> {
        Err(unavailable())
    }

    /// Authorize a configured MCP process or endpoint without starting it,
    /// connecting, saving configuration, or requiring the bridge to be enabled.
    /// Native host callback only; remote plugin transports do not expose this
    /// capability. Call before persisting an enabled connection or testing it.
    async fn require_mcp_server_permission(&self, _spec: HostMcpServerSpec) -> Result<()> {
        Err(unavailable())
    }

    /// Resolve a server's input request under the built-in MCP bridge's live
    /// tool callback. This capability is only available in-process.
    async fn mcp_elicitation(
        &self,
        _server: &str,
        _request: agena_domain::mcp_elicitation::McpElicitation,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<agena_domain::mcp_elicitation::McpElicitationReply> {
        Err(unavailable())
    }

    /// Request a model completion from the built-in MCP bridge for a live tool call.
    async fn mcp_sampling(
        &self,
        _server: &str,
        _request: agena_domain::mcp_sampling::McpSamplingRequest,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Option<agena_domain::mcp_sampling::McpSamplingReply>> {
        Err(unavailable())
    }

    /// Invoke a browser view's tool under the bundled MCP bridge's live call.
    async fn call_mcp_app_view_tool(
        &self,
        _target: uuid::Uuid,
        _arguments: serde_json::Map<String, serde_json::Value>,
        _cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<agena_domain::mcp_app_tools::McpAppToolResult> {
        Err(unavailable())
    }

    /// Resolve and authorize a child process's effects under the current
    /// validated host callback. Native host integrations only: this method is
    /// deliberately absent from external plugin RPC transports.
    async fn prepare_process_sandbox(
        &self,
        _resources: HostProcessResources,
    ) -> Result<HostPreparedProcessSandbox> {
        Err(unavailable())
    }

    /// Prepare an application-owned I/O worker with a private working directory.
    /// Only explicit resources participate in fallback declaration checks; a
    /// network request does not require read permission for the project cwd.
    /// Native-only capability, intentionally absent from external plugin RPC.
    async fn prepare_worker_sandbox(
        &self,
        _resources: HostProcessResources,
    ) -> Result<HostPreparedProcessSandbox> {
        Err(unavailable())
    }

    /// Declarative display contribution — contribute or update one contribution
    /// by id. The host decides placement and priority; plugins never target a
    /// location (Phase 6 unified display model).
    async fn display_contribute(&self, _req: HostDisplayContributeRequest) -> Result<()> {
        Err(unavailable())
    }

    /// Declarative display contribution — remove a previously contributed id.
    async fn display_remove(
        &self,
        _req: HostDisplayRemoveRequest,
    ) -> Result<HostDisplayRemoveResponse> {
        Err(unavailable())
    }

    /// Unified notification — tell the user something now. The host decides
    /// surface, priority, and dedupe; plugins never target a location (Phase 6).
    async fn notify(&self, _req: PluginNotifyRequest) -> Result<()> {
        Err(unavailable())
    }

    /// UI theme — register or update a palette.
    async fn ui_theme_register(&self, _req: HostThemeRegisterRequest) -> Result<()> {
        Err(unavailable())
    }

    /// UI theme — list every registered palette.
    async fn ui_theme_list(&self) -> Result<HostThemeListResponse> {
        Err(unavailable())
    }

    /// UI theme — remove a palette by id.
    async fn ui_theme_remove(
        &self,
        _req: HostThemeRemoveRequest,
    ) -> Result<HostThemeRemoveResponse> {
        Err(unavailable())
    }
}

pub struct ImportedPluginBackground {
    pub cancellation: tokio_util::sync::CancellationToken,
    pub notices: std::sync::Arc<dyn ImportedPluginNotices>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ImportedPluginPromptRequest {
    pub text: String,
    pub wait: bool,
    pub as_user: bool,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub enum ImportedModToolRequest {
    List,
    Check {
        tool: String,
        input: serde_json::Value,
    },
    Call {
        tool: String,
        input: serde_json::Value,
    },
}

#[async_trait::async_trait]
pub trait ImportedPluginNotices: Send + Sync {
    async fn deliver(&self, text: String, wake: bool) -> Result<()>;
}

pub struct ImportedPluginWatchPath {
    pub requested: std::path::PathBuf,
    pub resolved: std::path::PathBuf,
}

pub struct ImportedMcpServerIdentity {
    pub package: String,
    pub server: String,
}

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub enum DeepSeekMcpManagerRequest {
    List,
    Watch {
        previous: Vec<serde_json::Value>,
    },
    Restart {
        name: String,
    },
    Probe {
        name: String,
        options: DeepSeekMcpProbeOptions,
    },
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DeepSeekMcpProbeOptions {
    pub timeout_ms: Option<f64>,
    pub max_pages: Option<u64>,
    pub max_tool_names: Option<u64>,
    pub aborted: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HookTranscriptFormat {
    Claude,
    Codex,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookTranscriptScope {
    Current,
    Parent,
    ParentAndAgent,
}

#[derive(Clone, Default)]
pub struct ImportedHookTranscripts {
    pub transcript: Option<ImportedHookFile>,
    pub agent: Option<ImportedHookFile>,
}

impl ImportedHookTranscripts {
    pub fn files(&self) -> impl Iterator<Item = &ImportedHookFile> {
        self.transcript.iter().chain(&self.agent)
    }
}

/// Keeps a session's private file alive until the hook or shell exits.
#[derive(Clone)]
pub struct ImportedHookFile {
    path: std::path::PathBuf,
    _owner: std::sync::Arc<dyn Send + Sync>,
}

impl ImportedHookFile {
    pub fn new(path: std::path::PathBuf, owner: std::sync::Arc<dyn Send + Sync>) -> Self {
        Self {
            path,
            _owner: owner,
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }
}

pub type ImportedHookEnvironment = ImportedHookFile;

#[async_trait::async_trait]
pub trait ImportedPluginFileEvents: Send + Sync {
    async fn resolve_paths(
        &self,
        absolute: Vec<std::path::PathBuf>,
    ) -> Result<Vec<ImportedPluginWatchPath>>;

    async fn changed(
        &self,
        path: ImportedPluginWatchPath,
        change: crate::SessionFileChange,
    ) -> Result<()>;
}

/// Unified imperative notification entry for plugins (Phase 6). Replaces
/// ad-hoc imperative UI hooks as the way a plugin tells the user something
/// now: the host decides surface, priority, and dedupe. Persistent display
/// state uses [`PluginDisplayContribution`] via `host.display_contribute`.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
/// Request to notify the user from a plugin.
pub struct PluginNotifyRequest {
    /// Optional structured event for shared terminal/browser attention delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attention: Option<crate::AttentionNotification>,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// One of `info` | `success` | `warning` | `error`.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub severity: String,
    /// Optional session scope; `None` means global.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PluginNotifyAction>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
/// Action offered in a plugin notification.
pub struct PluginNotifyAction {
    pub label: String,
    pub target: PluginNotifyActionTarget,
}

/// A notification action button. Maps to the domain `ActionTarget` subset
/// that plugins may produce (recovery is app-owned).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Target of a plugin notification action.
pub enum PluginNotifyActionTarget {
    Command {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
    Navigate {
        route: String,
    },
    Copy {
        text: String,
    },
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
/// Log level of a plugin log record.
pub enum LogLevel {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
/// Subscription to runtime events.
pub struct EventSubscription {
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginCommandInvocation {
    pub id: agena_domain::ExecutionId,
    pub command_id: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
/// Context of a host callback executed on behalf of a plugin.
pub struct HostCallbackContext {
    /// Local app authority never travels over plugin RPC.
    #[serde(skip)]
    pub mcp_app: Option<McpAppHostBinding>,
    /// In-process authority for a dispatched plugin command.
    #[serde(skip)]
    pub plugin_command: Option<PluginCommandInvocation>,
    /// Sender metadata frozen by the host for this execution. Plugins may only
    /// echo it with the matching authority token, never substitute another user.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caller: Option<crate::CallerIdentity>,
    /// Trusted host execution mode. This travels with callback authority and
    /// is independent of the model's tool arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox_enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub call_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    /// When the active host call originated from a `tool_invoke`, this holds
    /// the plugin-original tool name. Used for per-tool capability scoping
    /// so that capabilities declared by tool A do not implicitly authorize
    /// host calls coming back through tool B.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    /// Opaque host-issued authority proving that the session/call/workspace/
    /// tool context above originated from the active Host→Plugin call. Plugin
    /// code must treat this as an echo-only capability; inventing or replaying
    /// a token outside the originating call is rejected by the host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority_token: Option<String>,
    /// Opaque HTTP callback generation selected by the transport. This is only
    /// a routing selector for a registered binding; it grants no authority and
    /// must never contain a callback URL or credential. HTTP drivers overwrite
    /// it from the validated request header at every independent boundary.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http_binding_id: Option<String>,
}

tokio::task_local! {
    static HOST_CALLBACK_CONTEXT: HostCallbackContext;
}

/// Patch the enclosing callback context, inheriting any unspecified fields.
/// Independent requests and newly validated authorities should use
/// [`run_in_isolated_host_callback_context`] instead.
pub async fn run_in_host_callback_context<F>(patch: HostCallbackContext, fut: F) -> F::Output
where
    F: Future,
{
    let mut current = current_host_callback_context().unwrap_or_default();
    if let Some(binding) = patch.mcp_app {
        current.mcp_app = Some(binding);
    }
    if let Some(command) = patch.plugin_command {
        current.plugin_command = Some(command);
    }
    if let Some(caller) = patch.caller {
        current.caller = Some(caller);
    }
    if let Some(enabled) = patch.sandbox_enabled {
        current.sandbox_enabled = Some(enabled);
    }
    if let Some(plugin_id) = patch.plugin_id {
        current.plugin_id = Some(plugin_id);
    }
    if let Some(session_id) = patch.session_id {
        current.session_id = Some(session_id);
    }
    if let Some(call_id) = patch.call_id {
        current.call_id = Some(call_id);
    }
    if let Some(workspace_root) = patch.workspace_root {
        current.workspace_root = Some(workspace_root);
    }
    if let Some(tool_name) = patch.tool_name {
        current.tool_name = Some(tool_name);
    }
    if let Some(authority_token) = patch.authority_token {
        current.authority_token = Some(authority_token);
    }
    if let Some(http_binding_id) = patch.http_binding_id {
        current.http_binding_id = Some(http_binding_id);
    }
    run_in_isolated_host_callback_context(current, fut).await
}

/// Run with exactly the supplied callback context. Unspecified fields remain
/// absent, so an independent request cannot inherit privileges or an authority
/// token from the enclosing call. The enclosing context is restored afterwards.
pub async fn run_in_isolated_host_callback_context<F>(
    context: HostCallbackContext,
    fut: F,
) -> F::Output
where
    F: Future,
{
    // MCP App invocations have unique scoped call IDs even before a task is
    // handed off to a transcript tool.
    let key = context.session_id.zip(context.call_id).filter(|(_, call)| {
        *call >= 0 || context.mcp_app.as_ref().is_some_and(|app| app.view.is_some())
    });
    agena_async::permission_wait::scope(key, HOST_CALLBACK_CONTEXT.scope(context, fut)).await
}

pub fn current_host_callback_context() -> Option<HostCallbackContext> {
    HOST_CALLBACK_CONTEXT.try_with(Clone::clone).ok()
}

// ---------------- permission queries ----------------

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// One path-access question to the host's effective policy.
pub struct PathPermissionQuery {
    /// Read or write.
    pub kind: PathKind,
    /// The path as the plugin will use it. A relative path resolves against
    /// the session workspace root, exactly as a host tool would resolve it.
    pub path: String,
}

impl PathPermissionQuery {
    pub fn read(path: impl Into<String>) -> Self {
        Self {
            kind: PathKind::Read,
            path: path.into(),
        }
    }

    pub fn write(path: impl Into<String>) -> Self {
        Self {
            kind: PathKind::Write,
            path: path.into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "verdict", rename_all = "snake_case")]
/// The effective policy's answer to one permission query.
///
/// A query never waits for a human, so `Ask` cannot mean "a prompt is
/// pending". It means the policy did not approve on its own: the user would
/// have to be asked, and a query cannot ask. Treat it as not approved, or
/// call `HostClient::require_path_permission` / `require_network_permission`,
/// whose runtime implementations run the full approval flow.
pub enum PermissionQuery {
    /// The effective policy approves this outright.
    Allow {},
    /// The effective policy refuses this, and will refuse a host tool call to
    /// the same target too. `reason` is the policy's own explanation.
    Deny { reason: String },
    /// The effective policy would ask the user. Not an approval.
    Ask { reason: String },
}

impl PermissionQuery {
    pub fn is_allowed(&self) -> bool {
        matches!(self, Self::Allow {})
    }

    /// The policy's explanation, for either non-approval verdict.
    pub fn reason(&self) -> Option<&str> {
        match self {
            Self::Allow {} => None,
            Self::Deny { reason } | Self::Ask { reason } => Some(reason.as_str()),
        }
    }
}

fn policy_denied_path(request: &PathPermissionQuery, reason: &str) -> PluginError {
    let kind = match request.kind {
        PathKind::Read => "read",
        PathKind::Write => "write",
    };
    let mut error = PluginError::from_kind(
        crate::error::PluginErrorKind::PolicyDenied,
        format!(
            "path {kind} is not permitted for `{}`: {reason}",
            request.path
        ),
    );
    error.diagnostic.data = Some(serde_json::json!({
        "kind": "path",
        "access": kind,
        "path": request.path,
    }));
    error
}

fn policy_denied_network(target: &str, reason: &str) -> PluginError {
    let mut error = PluginError::from_kind(
        crate::error::PluginErrorKind::PolicyDenied,
        format!("network access is not permitted for `{target}`: {reason}"),
    );
    error.diagnostic.data = Some(serde_json::json!({
        "kind": "network",
        "target": target,
    }));
    error
}

// ---------------- ask_user ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// An option offered by an ask-user request.
pub struct AskUserOption {
    pub label: String,
    #[serde(default)]
    pub description: String,
    /// Select this option to enter a custom answer; no other options are required.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub custom: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A question asked to the user by a plugin.
pub struct AskUserQuestion {
    #[serde(default)]
    pub header: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<AskUserOption>,
    #[serde(default)]
    pub multiple: bool,
    #[serde(default)]
    pub allow_custom: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Request to ask the user something.
pub struct AskUserRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// Optional Markdown body shown in the review dialog. The plan approval
    /// review (`kind == "review"`) sets it to the full plan document so the
    /// user can see the plan being approved; other ask_user requests leave it
    /// empty.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_markdown: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_resolution_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub questions: Vec<AskUserQuestion>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default)]
    pub allow_free_text: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Response to an ask-user request.
pub struct AskUserResponse {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub reply: String,
    #[serde(default)]
    pub cancelled: bool,
    #[serde(default)]
    pub timed_out: bool,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub answers: BTreeMap<String, Vec<String>>,
}

// ---------------- agents ----------------

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput)]
#[input(
    trim("name", "prompt"),
    non_empty("name", "prompt"),
    minimum("timeout_ms", 1),
    minimum("max_tokens", 1),
    minimum("max_cost_microusd", 1)
)]
#[serde(deny_unknown_fields)]
pub struct AgentSpawnRequest {
    /// Unique nonnumeric name: 1–64 lowercase letters, digits or underscores; `parent` and `self` are reserved.
    pub name: String,
    /// Scope, constraints, required context and output contracts, and expected final result.
    pub prompt: String,
    /// Role profile from the available subagent catalog. Choose the role that
    /// fits the task; omission uses the configured subagent default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    /// Command instructions attached to the initial message.
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selection: Option<AgentModelSelection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Per-execution token budget, retained for followup executions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_cost_microusd: Option<u64>,
    /// Seed the subagent with the parent's completed turns as of creation.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub inherit_parent_context: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
pub struct AgentModelSelection {
    pub provider: Option<String>,
    pub adapter: Option<String>,
    pub model: Option<String>,
    pub thinking_mode: Option<String>,
    pub speed_mode: Option<String>,
    pub verbosity: Option<String>,
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHandle {
    pub agent_id: i64,
    pub name: String,
    /// Effective role selected for this child.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile: Option<String>,
    pub status: agena_domain::AgentStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput)]
#[input(trim("target", "message"), non_empty("target", "message"))]
#[serde(deny_unknown_fields)]
pub struct AgentMessageRequest {
    /// An agent_id, a visible agent name, or `parent`.
    pub target: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput)]
#[input(trim("target"), non_empty("target"))]
#[serde(deny_unknown_fields)]
pub struct AgentTargetRequest {
    pub target: String,
}

#[derive(
    Debug, Clone, Default, Serialize, Deserialize, schemars::JsonSchema, agena_macros::ToolInput,
)]
#[input(minimum("timeout_ms", 1), maximum("timeout_ms", 300000))]
#[serde(deny_unknown_fields)]
pub struct AgentWaitRequest {
    /// Defaults to 60 seconds. Wait for messages instead of polling statuses.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentMessageReceipt {
    pub agent_id: i64,
    /// Acceptance into the durable mailbox, not proof the recipient has read it.
    pub accepted: bool,
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWakeReason {
    Message,
    UserInput,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentWaitResponse {
    /// Message content is delivered once at the next model boundary.
    pub reason: AgentWakeReason,
}

// ---------------- list_tools ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Descriptor of a tool visible to a plugin.
pub struct ToolDescriptor {
    pub name: String,
    /// Fully qualified plugin id that publishes this tool, e.g. `agena.fs`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plugin_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
}

/// Session information visible to a plugin.
///
/// This *is* the HTTP `SessionResource`, not a parallel declaration: a plugin
/// and a browser client read one session from the same shape, so neither
/// surface can drift from the other.
pub type HostSession = agena_session_api::resource::SessionResource;

#[cfg(test)]
mod host_session_scope_tests {
    use super::HostSession;

    #[test]
    fn session_workspace_preserves_global_and_project_scope() {
        let mut session = HostSession {
            id: 1,
            parent_id: None,
            depth: 0,
            root_id: 1,
            workspace_id: None,
            title: "Bot".into(),
            owner_bot_id: None,
            owner_surface: None,
            favorite: false,
            pinned: false,
            version: 1,
            relation_kind: agena_domain::SessionRelationKind::Root,
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            state: Default::default(),
            source_cutoff_seq_global: None,
            source_message_id: None,
            is_subagent: false,
            agent_name: None,
            agent_status: None,
            created_at: chrono::DateTime::<chrono::Utc>::from_timestamp_millis(0).unwrap(),
            updated_at: chrono::DateTime::<chrono::Utc>::from_timestamp_millis(0).unwrap(),
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        };
        let global = serde_json::to_value(&session).unwrap();
        assert!(global["workspace_id"].is_null());
        assert_eq!(
            serde_json::from_value::<HostSession>(global).unwrap(),
            session
        );
        session.workspace_id = Some(42);
        let project = serde_json::to_value(&session).unwrap();
        assert_eq!(project["workspace_id"], 42);
        assert_eq!(
            serde_json::from_value::<HostSession>(project).unwrap(),
            session
        );
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Request for host context status.
pub struct HostContextStatusRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Host context status response.
pub struct HostContextStatusResponse {
    pub session_id: i64,
    pub current_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub measured_prompt_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub projected_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remaining_tokens: Option<u64>,
    pub reserved_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_context_window_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_max_input_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_max_output_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    pub prompt_window_generation: u64,
    pub compacted: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_before_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_compaction_after_tokens: Option<u64>,
    pub auto_compaction_disabled: bool,
    pub consecutive_compaction_failures: u8,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to fetch session information from the host.
pub struct HostGetSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response with session information from the host.
pub struct HostGetSessionResponse {
    pub session: HostSession,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to rename a session.
pub struct HostRenameSessionRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a session rename.
pub struct HostRenameSessionResponse {
    pub session: HostSession,
}

/// Sensitive, native-only connection for the three bundled provider plugins.
/// It is intentionally neither serializable nor printable with credentials.
#[derive(Clone)]
pub struct BuiltinProviderConnection {
    pub provider_id: String,
    pub base_url: String,
    pub messages_url: Option<String>,
    pub realtime_ws_url: Option<String>,
    pub headers: std::collections::BTreeMap<String, String>,
    pub model: Option<String>,
    pub timeout_secs: u64,
    pub connect_timeout_secs: u64,
    /// Used only to authenticate local provider-file receipts.
    pub receipt_key: String,
}

impl std::fmt::Debug for BuiltinProviderConnection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BuiltinProviderConnection")
            .field("provider_id", &self.provider_id)
            .field("model", &self.model)
            .finish_non_exhaustive()
    }
}

// ---------------- direct provider image execution ----------------

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Operation performed by an image execution.
pub enum HostImageOperation {
    Generate,
    Edit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case", deny_unknown_fields)]
/// Input of an image execution.
pub enum HostImageInput {
    /// Workspace-relative or absolute local path. The host resolves it using
    /// the callback-scoped tool executor and requires an existing read grant.
    Path { path: String },
    /// Already-materialized attachment. Only base64/data-url/local-path image
    /// sources are accepted; URL and provider file-id sources are rejected.
    Attachment { attachment: AttachmentItem },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Request to execute an image operation.
pub struct HostImageExecuteRequest {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub operation: HostImageOperation,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<HostImageInput>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub moderation: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Response of an image execution.
pub struct HostImageExecuteResponse {
    pub operation: HostImageOperation,
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revised_prompt: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<AttachmentItem>,
}

// ---------------- monitor ----------------

// ---------------- command registry ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to register a command in the host command registry.
pub struct HostCommandRegisterRequest {
    pub command: CommandDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to update a registered command.
pub struct HostCommandUpdateRequest {
    pub command: CommandDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to remove a registered command.
pub struct HostCommandRemoveRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a command registry mutation. Commands carry no model-facing
/// alias, so the response reports the command itself rather than a name.
pub struct HostCommandMutationResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<CommandDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response listing the commands visible to the caller's scope.
pub struct HostRegisteredCommandListResponse {
    pub generation: u64,
    pub commands: Vec<CommandDefinition>,
}

// ---------------- tool registry ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to register a tool in the host tool registry.
pub struct HostToolRegisterRequest {
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to update a registered tool.
pub struct HostToolUpdateRequest {
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to remove a registered tool.
pub struct HostToolRemoveRequest {
    pub name: String,
    #[serde(default)]
    pub by_model_name: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a tool registry mutation.
pub struct HostToolMutationResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolDefinition>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a tool registry change.
pub enum ToolRegistryChangeKind {
    Registered,
    Updated,
    Removed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// Event emitted when the tool registry changes.
pub struct ToolRegistryChangedEvent {
    pub kind: ToolRegistryChangeKind,
    pub generation: u64,
    pub timestamp_ms: i64,
    pub plugin: PluginKey,
    pub tool_key: ToolKey,
    /// Opaque capability visibility scope. Absent means the mutation is
    /// plugin-global; session-scoped dynamic tools carry `session:<id>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<ToolDefinition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Descriptor of a tool registered in the host registry.
pub struct HostRegisteredToolDescriptor {
    pub plugin: PluginKey,
    pub tool_key: ToolKey,
    pub tool: ToolDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response listing registered tools.
pub struct HostRegisteredToolListResponse {
    pub generation: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<HostRegisteredToolDescriptor>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event: Option<ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImportedPluginAgentState {
    Active,
    Completed(i64),
    Failed,
}

// ---------------- plugin storage ----------------

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Scope of a plugin storage record.
pub enum HostStorageScope {
    Session,
    Workspace,
    #[default]
    Global,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Visibility of a plugin storage record.
pub enum HostStorageVisibility {
    #[default]
    Private,
    Shared,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to get a storage record.
pub struct HostStorageGetRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response with a storage record.
pub struct HostStorageGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to set a storage record.
pub struct HostStorageSetRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to delete a storage record.
pub struct HostStorageDeleteRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    pub namespace: String,
    pub key: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to list storage records.
pub struct HostStorageListRequest {
    #[serde(default)]
    pub scope: HostStorageScope,
    #[serde(default)]
    pub visibility: HostStorageVisibility,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing storage records.
pub struct HostStorageListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub records: Vec<HostStorageRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A plugin storage record.
pub struct HostStorageRecord {
    pub namespace: String,
    pub key: String,
}

// ---------------- plugin secrets ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to get a plugin secret.
pub struct HostSecretGetRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response with a plugin secret.
pub struct HostSecretGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to set a plugin secret.
pub struct HostSecretSetRequest {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to delete a plugin secret.
pub struct HostSecretDeleteRequest {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing plugin secret names.
pub struct HostSecretListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub names: Vec<String>,
}

// ---------------- plugin status ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Status of a plugin as seen by the host.
pub struct HostPluginStatus {
    pub plugin_id: PluginKey,
    pub kind: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    #[serde(default)]
    pub restart_count: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_exit_code: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_restart_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<agena_failure::UserProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing plugin statuses.
pub struct HostPluginStatusListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<HostPluginStatus>,
}

/// One loaded plugin's manifest metadata plus its tool names, used for
/// plugin discovery.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostPluginDescriptor {
    pub plugin_id: PluginKey,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default)]
    pub version: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// Canonical names of every execution tool this plugin publishes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing plugins.
pub struct HostPluginListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plugins: Vec<HostPluginDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request for the status of one plugin.
pub struct HostPluginStatusGetRequest {
    pub plugin_id: PluginKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response with the status of one plugin.
pub struct HostPluginStatusGetResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<HostPluginStatus>,
}

// ---------------- lsp ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing LSP servers.
pub struct HostLspListServersResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub servers: Vec<HostLspServer>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Descriptor of an LSP server.
pub struct HostLspServer {
    pub name: String,
    pub command: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub file_extensions: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub executable: Option<String>,
    /// File-system lookup only; does not certify startup or language support.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command_available: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub root_markers: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub running_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// An LSP diagnostic.
pub struct HostLspDiagnostic {
    pub uri: String,
    pub severity: String,
    pub message: String,
    pub start_line: u32,
    pub start_character: u32,
    pub end_line: u32,
    pub end_character: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<String>,
}

// ---------------- scheduler ----------------

// ---------------- hooks ----------------

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing host hook registrations.
pub struct HostHookListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookRegistration>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A host hook registration of a plugin.
pub struct HostHookRegistration {
    pub plugin_id: PluginKey,
    pub trust_level: String,
    pub trust_status: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provenance: Vec<String>,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookDescriptor>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Descriptor of a registered hook.
pub struct HostHookDescriptor {
    pub name: String,
    /// Every registered handler must be self-documenting.
    pub handler_id: String,
    pub summary: String,
    pub help: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub translations: BTreeMap<String, crate::HookHandlerTranslation>,
    pub priority: i32,
    pub filters: crate::HookHandlerFilters,
    pub is_async: bool,
    #[serde(default)]
    pub enabled: bool,
    pub trust_level: String,
    pub trust_status: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_hash: Option<String>,
}

// ---------------- mcp ----------------

// ---------------- UI: declarative display contribution / theme ----------------

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to contribute a display block.
pub struct HostDisplayContributeRequest {
    pub contribution: PluginDisplayContribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to remove a display block.
pub struct HostDisplayRemoveRequest {
    pub contribution_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response of removing a display block.
pub struct HostDisplayRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfigReloadRequestResponse {
    pub task_id: String,
    /// False means another request from this originating call already queued it.
    pub started: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfigReloadStatusRequest {
    pub task_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HostConfigReloadStatusResponse {
    pub task_id: String,
    pub state: HostConfigReloadState,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum HostConfigReloadState {
    /// Includes waiting for the originating call to finish.
    Running {},
    Succeeded {},
    Failed {
        problem: agena_failure::UserProblem,
    },
    Cancelled {},
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A theme palette contributed by a plugin.
pub struct HostThemePalette {
    pub id: String,
    pub plugin_id: PluginKey,
    pub display_name: String,
    pub colors: PluginTerminalThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to register a theme palette.
pub struct HostThemeRegisterRequest {
    pub id: String,
    pub display_name: String,
    pub colors: PluginTerminalThemeColors,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Request to remove a theme palette.
pub struct HostThemeRemoveRequest {
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response of removing a theme palette.
pub struct HostThemeRemoveResponse {
    pub removed: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Response listing theme palettes.
pub struct HostThemeListResponse {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<HostThemePalette>,
}

/// Default impl returning `HostUnavailable` for every callback. Use in tests.
pub struct NoopHostClient;

#[async_trait]
impl HostClient for NoopHostClient {
    async fn log(&self, _l: LogLevel, _m: String, _f: serde_json::Value) {}

    async fn publish_event(&self, _: EventEnvelope) -> Result<()> {
        Err(unavailable())
    }

    async fn subscribe_events(&self, _: EventFilter) -> Result<EventSubscription> {
        Err(unavailable())
    }

    async fn read_config(&self, _: Option<String>) -> Result<serde_json::Value> {
        Err(unavailable())
    }

    async fn invoke_tool(&self, _: String, _: serde_json::Value) -> Result<ToolInvokeOutput> {
        Err(unavailable())
    }
}

fn unavailable() -> PluginError {
    PluginError::from_kind(
        crate::error::PluginErrorKind::HostUnavailable,
        "host is unavailable for this plugin",
    )
}

#[cfg(test)]
mod agent_profile_contract_tests {
    use super::{AgentHandle, AgentSpawnRequest};

    #[test]
    fn agent_spawn_profile_is_optional_for_existing_callers_and_exposed_in_schema() {
        let old: AgentSpawnRequest = serde_json::from_value(serde_json::json!({
            "name":"research", "prompt":"Find the call sites"
        }))
        .unwrap();
        assert!(old.profile.is_none());
        let selected: AgentSpawnRequest = serde_json::from_value(serde_json::json!({
            "name":"research", "prompt":"Find the call sites", "profile":"explore"
        }))
        .unwrap();
        assert_eq!(selected.profile.as_deref(), Some("explore"));
        let schema = serde_json::to_value(schemars::schema_for!(AgentSpawnRequest)).unwrap();
        assert!(schema["properties"].get("profile").is_some());
        assert!(
            !schema["required"]
                .as_array()
                .unwrap()
                .iter()
                .any(|value| value == "profile")
        );
    }

    #[test]
    fn agent_handles_accept_old_payloads_and_report_the_selected_profile() {
        let mut handle: AgentHandle = serde_json::from_value(serde_json::json!({
            "agent_id":1,"name":"research","status":"running"
        }))
        .unwrap();
        assert!(handle.profile.is_none());
        handle.profile = Some("explore".into());
        assert_eq!(serde_json::to_value(handle).unwrap()["profile"], "explore");
    }
}
