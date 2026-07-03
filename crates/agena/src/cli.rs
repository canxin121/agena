use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use agena_mcp_client::protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult,
    PromptDescriptor, PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents,
    ResourceDescriptor, ToolDescriptor,
};
use agena_mcp_server::{McpServerBackend, McpServerError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use clap::{Args, CommandFactory, Parser, Subcommand, ValueEnum};
use sea_orm::{
    ActiveModelTrait, ActiveValue::Set, ColumnTrait, EntityTrait, QueryFilter, QueryOrder,
};
use serde::Serialize;

use crate::{
    agent::Agent,
    config::{
        ConfigEnvironment, ConfigLoader, ConfigOutputFormat, ConfigOverride, LoadConfigRequest,
        ProcessEnvironment, ProviderAuthConfig, ProviderAuthTargetError,
        ProviderConfigCredentialStore, ProviderDeviceAuthTarget, ProviderOAuthTarget,
        ResolvedProviderConfig, TracingConfig, resolve_provider_device_auth_target,
        resolve_provider_oauth_target,
    },
    db::{
        crud::{permission_rule as permission_rule_crud, workspace as workspace_crud},
        entities, init_schema,
    },
    error::AppError,
    memory::{MemoryStore, MemoryType},
    message::{ApplyPatchToolInput, PartContent, StructuredObject, ToolInvocation},
    model::ModelRef,
    permission::{
        PermissionAction, PermissionMode, PermissionPolicy, PermissionReply, PermissionReplyKind,
        PermissionScope, PersistedPermissionRule, ToolPermissionPolicy,
    },
    provider::{
        ModelCapabilities, ModelMetadata, ProviderModel,
        auth::{
            AuthData, AuthManager, CopilotDeployment, DeviceCodeStart, OAuthAuthorizeStart,
            OAuthCallback, wait_for_oauth_callback,
        },
    },
    role::Role,
    runtime::{AgenaRuntime, TracingFilterReloadHandle},
    session::{
        RunStatus, Session, SessionCreateRequest, SessionExecutionRequest, SessionForkRequest,
        SessionListRequest, SessionManager, SessionRunOptions, SessionSummary,
        SessionUserMessageRequest, UsagePeriod, UsageStatsQuery,
    },
    storage::StorageConfig,
    tool::{ApplyPatchExecution, ToolExecutor, ToolPayloadInput},
    tracing as tracing_config,
};

#[derive(Debug, Clone, Parser)]
#[command(
    name = "agena",
    version,
    about = "Agena unified CLI and terminal UI",
    long_about = "Agena is an LLM-agent runtime with a unified CLI/TUI. \
                  Running `agena` starts the terminal UI directly; use \
                  subcommands like `exec`, `sessions`, `plugin`, \
                  `mcp-server`, or `app-server` for non-TUI workflows.\n\n\
                  Quick start:\n  \
                  agena\n  \
                  agena exec \"summarise the README\"\n  \
                  agena sessions list\n\n\
                  Configuration is loaded from the single home config \
                  `~/agena/agena.json`. \
                  Run `agena config resolve` to inspect the resolved settings.",
    after_help = "EXAMPLES:\n  \
                  Start the terminal UI:\n    \
                  agena\n\n  \
                  Start the terminal UI at a specific session:\n    \
                  agena tui --session 42\n\n  \
                  Start a one-shot run:\n    \
                  agena exec \"explain crates/agena-api-server\"\n\n  \
                  Resume the most recent session:\n    \
                  agena resume\n\n  \
                  Show effective config:\n    \
                  agena config resolve --format json\n\n  \
                  Run as an MCP server over stdio:\n    \
                  agena mcp-server --transport stdio"
)]
pub struct AgenaCli {
    #[arg(short = 'c', long = "set", global = true)]
    pub overrides: Vec<ConfigOverride>,
    #[arg(long, env = "AGENA_DATABASE_URL", global = true)]
    pub database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH", global = true)]
    pub database_path: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<AgenaCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgenaCommand {
    AppServer(AppServerArgs),
    Agents(AgentsCommand),
    Apply(ApplyArgs),
    Auth(AuthCommand),
    Completion(CompletionArgs),
    Config(ConfigCommand),
    Continue(ContinueArgs),
    Commit(CommitArgs),
    Pr(PrArgs),
    Debug(DebugCommand),
    Diagnostics(DiagnosticsArgs),
    Exec(ExecArgs),
    Fork(ForkArgs),
    Cost(CostArgs),
    Usage(UsageArgs),
    Git(GitArgs),
    Login(LoginArgs),
    Memory(MemoryCommand),
    Logout(LogoutArgs),
    McpServer(McpServerArgs),
    Permissions(PermissionsArgs),
    Provider(ProviderCommand),
    Plugin(PluginCommand),
    Resume(ResumeArgs),
    Review(ReviewArgs),
    Sessions(SessionsCommand),
    Tui(TuiArgs),
    Snapshot(SnapshotArgs),
}

#[derive(Debug, Clone, Args)]
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: Option<AuthSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: Option<ConfigSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct DebugCommand {
    #[command(subcommand)]
    pub command: DebugSubcommand,
}

#[derive(Debug, Clone, Args)]
pub struct DiagnosticsArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CostArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum UsagePeriodArg {
    Today,
    Week,
    ThirtyDays,
    Month,
    All,
}

impl UsagePeriodArg {
    fn into_usage_period(self) -> UsagePeriod {
        match self {
            Self::Today => UsagePeriod::Today,
            Self::Week => UsagePeriod::Last7Days,
            Self::ThirtyDays => UsagePeriod::Last30Days,
            Self::Month => UsagePeriod::MonthToDate,
            Self::All => UsagePeriod::AllTime,
        }
    }
}

#[derive(Debug, Clone, Args)]
pub struct UsageArgs {
    /// Preset reporting window. Use --from/--to for an exact custom range.
    #[arg(long, value_enum, default_value_t = UsagePeriodArg::Week)]
    pub period: UsagePeriodArg,
    /// Start of a custom range. Accepts YYYY-MM-DD or RFC3339.
    #[arg(long)]
    pub from: Option<String>,
    /// End of a custom range. Accepts YYYY-MM-DD or RFC3339.
    #[arg(long)]
    pub to: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CommitArgs {
    pub message: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PrArgs {
    pub title: String,
    #[arg(long)]
    pub body: Option<String>,
    #[arg(long)]
    pub base: Option<String>,
    #[arg(long)]
    pub head: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub command: Option<PermissionsSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum PermissionsSubcommand {
    List(PermissionsListArgs),
    Create(PermissionsWriteArgs),
    Replace(PermissionsReplaceArgs),
    Revoke(PermissionsRevokeArgs),
    Reply(PermissionsReplyArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionScopeArg {
    Session,
    Workspace,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionModeArg {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PermissionReplyKindArg {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsListArgs {
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsWriteArgs {
    #[arg(long)]
    pub action_key: Option<String>,
    #[arg(long)]
    pub tool_name: Option<String>,
    #[arg(long)]
    pub qualifier: Option<String>,
    #[arg(long)]
    pub path_access_kind: Option<String>,
    #[arg(long)]
    pub workspace_root: Option<String>,
    #[arg(long)]
    pub target_path: Option<String>,
    #[arg(long)]
    pub network_target: Option<String>,
    #[arg(long)]
    pub network_host: Option<String>,
    #[arg(long)]
    pub network_port: Option<u16>,
    #[arg(long, value_enum)]
    pub scope: PermissionScopeArg,
    #[arg(long)]
    pub session_id: Option<i64>,
    #[arg(long = "rule-mode", value_enum)]
    pub rule_mode: PermissionModeArg,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsReplaceArgs {
    pub rule_id: i64,
    #[command(flatten)]
    pub rule: PermissionsWriteArgs,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsRevokeArgs {
    pub rule_id: i64,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PermissionsReplyArgs {
    pub request_id: String,
    #[arg(long)]
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, value_enum)]
    pub kind: PermissionReplyKindArg,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long, value_enum)]
    pub scope: Option<PermissionScopeArg>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SnapshotArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct GitArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCommand {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct AgentsCommand {
    #[command(subcommand)]
    pub command: Option<AgentsSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryCommand {
    #[command(subcommand)]
    pub command: Option<MemorySubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct PluginCommand {
    #[command(subcommand)]
    pub command: PluginSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum PluginSubcommand {
    /// Print runtime status for configured plugins, including failed loads.
    Status(PluginStatusArgs),
    /// Show detailed runtime and manifest information for one plugin.
    Inspect(PluginInspectArgs),
    /// Show recent retained logs for one plugin.
    Logs(PluginLogsArgs),
    /// Validate a plugin manifest, configured plugin, or agena config plugin list.
    Validate(PluginValidateArgs),
    /// Install a plugin from a marketplace registry into the active config.
    Install(PluginInstallArgs),
    /// Remove a plugin previously installed via `agena plugin install`.
    Uninstall(PluginUninstallArgs),
    /// List plugins installed via the marketplace.
    ListInstalled,
    /// Refresh the cached registry index.
    Sync(PluginSyncArgs),
    /// Search registry plugins by id, name, or description substring.
    Search(PluginSearchArgs),
    /// Re-resolve installed plugins against their registry and reinstall newer versions.
    Upgrade(PluginUpgradeArgs),
    /// Print plugins for which a newer version is available on their registry.
    Outdated,
}

#[derive(Debug, Clone, Args)]
pub struct PluginStatusArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginInspectArgs {
    pub plugin_id: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginLogsArgs {
    pub plugin_id: String,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long)]
    pub after_seq: Option<u64>,
    #[arg(long, value_enum, default_value_t = PluginLogOutputFormat::Text)]
    pub format: PluginLogOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginValidateArgs {
    /// Manifest file, plugin directory, configured plugin JSON, or agena config.
    pub path: PathBuf,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
    /// Treat warnings as validation failures.
    #[arg(long, default_value_t = false)]
    pub strict: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum PluginLogOutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Args)]
pub struct PluginInstallArgs {
    /// Plugin id, optionally with `@<version>` suffix.
    pub spec: String,
    /// Registry index URL (overrides the default configured registry).
    #[arg(long)]
    pub registry: Option<String>,
    /// Registry id, used as a cache key (defaults to "default").
    #[arg(long, default_value = "default")]
    pub registry_id: String,
    /// Overwrite an existing plugin config with the same plugin id.
    #[arg(long, default_value_t = false)]
    pub force: bool,
    /// Compute side-effects but skip writing the config or cache.
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Allow installing artifacts that lack a sha256 in the registry record.
    #[arg(long, default_value_t = false)]
    pub allow_unverified: bool,
    /// Refresh the cached registry index before resolving versions.
    #[arg(long, default_value_t = false)]
    pub refresh: bool,
    /// Treat the registry as requiring an ed25519 signature on every record.
    #[arg(long, default_value_t = false)]
    pub require_signature: bool,
}

#[derive(Debug, Clone, Args)]
pub struct PluginUninstallArgs {
    pub plugin_id: String,
    /// Also uninstall any plugin that depends on this one.
    #[arg(long, default_value_t = false)]
    pub cascade: bool,
}

#[derive(Debug, Clone, Args)]
pub struct PluginSyncArgs {
    /// Registry index URL.
    pub registry: String,
    #[arg(long, default_value = "default")]
    pub registry_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct PluginSearchArgs {
    pub query: String,
    /// Registry index URL.
    pub registry: String,
    #[arg(long, default_value = "default")]
    pub registry_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct PluginUpgradeArgs {
    /// Plugin id to upgrade. Pass `--all` to upgrade every installed plugin.
    pub plugin_id: Option<String>,
    /// Upgrade every installed plugin one by one.
    #[arg(long, default_value_t = false)]
    pub all: bool,
    /// Override the registry URL recorded at install time.
    #[arg(long)]
    pub registry: Option<String>,
    #[arg(long, default_value = "default")]
    pub registry_id: String,
}

#[derive(Debug, Clone, Args)]
pub struct SessionsCommand {
    #[command(subcommand)]
    pub command: Option<SessionsSubcommand>,
}

#[derive(Debug, Clone, Args)]
pub struct AppServerArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = AppServerTransport::Stdio)]
    pub transport: AppServerTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum AppServerTransport {
    Stdio,
}

#[derive(Debug, Clone, Args)]
pub struct McpServerArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AuthSubcommand {
    List(AuthListArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ConfigSubcommand {
    Resolve(ConfigResolveArgs),
    Validate,
}

#[derive(Debug, Clone, Subcommand)]
pub enum DebugSubcommand {
    Session(DebugSessionArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum ProviderSubcommand {
    List(ProviderListArgs),
    Models(ProviderModelsArgs),
    Capabilities(ProviderCapabilitiesArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgentsSubcommand {
    List(AgentsListArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum MemorySubcommand {
    List(MemoryListArgs),
    Forget(MemoryForgetArgs),
    Edit(MemoryEditArgs),
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionsSubcommand {
    List(SessionListArgs),
    /// Export a session to a JSONL bundle (stdout). Pipe to a file to keep.
    Export(SessionExportArgs),
    /// Replay a JSONL bundle (read from stdin) as a fresh session in the
    /// current workspace.
    Import(SessionImportArgs),
    /// Print every session sharing the given tree root, in (depth, id) order.
    Tree(SessionTreeArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SessionExportArgs {
    pub session_id: i64,
}

#[derive(Debug, Clone, Args)]
pub struct SessionImportArgs {
    /// Optional path. Reads from stdin if omitted.
    #[arg(long)]
    pub path: Option<std::path::PathBuf>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionTreeArgs {
    pub root_id: i64,
    /// Cap the rendered subtree at this depth relative to the root (root = 0).
    #[arg(long)]
    pub max_depth: Option<i64>,
    /// Cap the number of sessions rendered. Useful for very wide trees.
    #[arg(long)]
    pub limit: Option<usize>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct AuthListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct LoginArgs {
    pub provider_id: String,
    #[arg(long)]
    pub api_key: Option<String>,
    #[arg(long)]
    pub browser: bool,
    #[arg(long)]
    pub device: bool,
    #[arg(long, default_value_t = 1455)]
    pub port: u16,
    #[arg(long, default_value_t = 600)]
    pub timeout_secs: u64,
    #[arg(long, default_value = "https://gitlab.com")]
    pub instance_url: String,
    #[arg(long)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct LogoutArgs {
    pub provider_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum SessionListView {
    All,
    Roots,
    Subtree,
}

#[derive(Debug, Clone, Args)]
pub struct SessionListArgs {
    #[arg(long, default_value_t = 20)]
    pub limit: u64,
    #[arg(long, default_value_t = 0)]
    pub offset: u64,
    #[arg(long, value_enum, default_value_t = SessionListView::All)]
    pub view: SessionListView,
    #[arg(long)]
    pub anchor_session_id: Option<i64>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryListArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryForgetArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    pub name: String,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryEditArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Args)]
pub struct ResumeArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[derive(Debug, Clone, Args)]
pub struct ContinueArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ApplyArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
    pub patch_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
pub struct DebugSessionArgs {
    pub session_id: i64,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ExecArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long)]
    pub json: bool,
    pub prompt: String,
}

#[derive(Debug, Clone, Args, Default)]
pub struct TuiArgs {
    #[arg(long, env = "AGENA_DATABASE_URL")]
    pub database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH")]
    pub database_path: Option<PathBuf>,
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub session: Option<i64>,
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long)]
    pub locale: Option<String>,
    #[arg(long, env = "AGENA_TUI_LOG_FILE", conflicts_with = "log_stderr")]
    pub log_file: Option<PathBuf>,
    #[arg(long, env = "AGENA_TUI_LOG_STDERR")]
    pub log_stderr: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ReviewArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value = "main")]
    pub base: String,
    #[arg(long)]
    pub agent: Option<String>,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ForkArgs {
    pub session_id: i64,
    /// Fork point: clones every event up to and including the last one tied
    /// to this message id. Omit to clone the entire history.
    #[arg(long)]
    pub at_message: Option<i64>,
    #[arg(long)]
    pub title: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigResolveArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderModelsArgs {
    pub provider_id: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCapabilitiesArgs {
    pub target: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct AgentsListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Json)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Serialize)]
struct AuthListOutput {
    credentials: Vec<AuthSummary>,
}

#[derive(Debug, Serialize)]
struct AuthSummary {
    provider_id: String,
    kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    enterprise_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    issuer: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
struct SessionListOutput {
    sessions: Vec<SessionSummary>,
}

#[derive(Debug, Serialize)]
struct SessionOutput {
    session: SessionDetail,
}

#[derive(Debug, Serialize)]
struct SessionForkOutput {
    source_session_id: i64,
    forked: SessionDetail,
}

#[derive(Debug, Serialize)]
struct SessionImportOutput {
    session: SessionDetail,
}

#[derive(Debug, Serialize)]
struct MemoryListOutput {
    dir: String,
    count: usize,
    memories: Vec<MemorySummaryOutput>,
}

#[derive(Debug, Serialize)]
struct MemorySummaryOutput {
    file_name: String,
    name: String,
    description: String,
    #[serde(rename = "type", skip_serializing_if = "Option::is_none")]
    memory_type: Option<String>,
    path: String,
}

#[derive(Debug, Serialize)]
struct ApplyOutput {
    title: String,
    output_text: String,
    patch: ApplyPatchExecution,
}

#[derive(Debug, Serialize)]
struct ExecOutput {
    session: SessionDetail,
    text: String,
}

#[derive(Debug, Serialize)]
struct DebugSessionOutput {
    session: SessionDetail,
    messages: Vec<DebugMessageOutput>,
}

#[derive(Debug, Serialize)]
struct DebugMessageOutput {
    id: i64,
    role: Role,
    state: crate::message::MessageStatus,
    text: String,
}

#[derive(Debug, Serialize)]
struct DiagnosticsOutput {
    version: &'static str,
    os: String,
    arch: String,
    current_dir: String,
    config: DiagnosticsConfigOutput,
    environment: DiagnosticsEnvironmentOutput,
}

#[derive(Debug, Serialize)]
struct CostOutput {
    session: SessionDetail,
    summary: crate::session::SessionCostSummary,
}

#[derive(Debug, Serialize)]
struct PermissionRuleOutput {
    id: i64,
    action_key: String,
    mode: String,
    scope: String,
    session_id: Option<i64>,
    workspace_id: Option<i64>,
    source: String,
    reason: Option<String>,
    operator: Option<String>,
    revoked_at: Option<DateTime<Utc>>,
    revoked_reason: Option<String>,
    revoked_by: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct PermissionsOutput {
    count: usize,
    rules: Vec<PermissionRuleOutput>,
}

#[derive(Debug, Serialize)]
struct ActiveSnapshotOutput {
    session_id: i64,
    path: String,
    branch: String,
    backend: String,
    created_here: bool,
}

#[derive(Debug, Serialize)]
struct ManagedSnapshotOutput {
    path: String,
    session_id: Option<i64>,
    branch: Option<String>,
    backend: Option<String>,
    registered_with_git: bool,
    registered_with_rift: bool,
    stale: bool,
}

#[derive(Debug, Serialize)]
struct SnapshotBackendSupportOutput {
    available: bool,
    detail: String,
}

#[derive(Debug, Serialize)]
struct SnapshotCapabilitiesOutput {
    preferred_backend: Option<String>,
    git: SnapshotBackendSupportOutput,
    rift: SnapshotBackendSupportOutput,
}

#[derive(Debug, Serialize)]
struct SnapshotOutput {
    workspace_root: String,
    capabilities: SnapshotCapabilitiesOutput,
    active: Vec<ActiveSnapshotOutput>,
    managed: Vec<ManagedSnapshotOutput>,
}

#[derive(Debug, Clone)]
struct GitPreflight {
    git_available: bool,
    repo: bool,
    gh_available: bool,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    staged_files: u64,
    unstaged_files: u64,
    untracked_files: u64,
    changed_files: u64,
    clean: bool,
}

#[derive(Debug, Serialize)]
struct GitOutput {
    workspace_root: String,
    git_available: bool,
    repo: bool,
    gh_available: bool,
    branch: Option<String>,
    upstream: Option<String>,
    ahead: Option<u64>,
    behind: Option<u64>,
    staged_files: u64,
    unstaged_files: u64,
    untracked_files: u64,
    changed_files: u64,
    clean: bool,
    snapshot_active_sessions: u64,
    snapshot_managed_dirs: u64,
}

#[derive(Debug, Serialize)]
struct CommitOutput {
    workspace_root: String,
    commit: String,
    summary: String,
}

#[derive(Debug, Serialize)]
struct PrOutput {
    workspace_root: String,
    branch: String,
    url: String,
}

#[derive(Debug, Serialize)]
struct DiagnosticsConfigOutput {
    path: String,
    found: bool,
    project_path: String,
    project_found: bool,
    applied_layers: Vec<String>,
    provider_count: usize,
    plugin_count: usize,
}

#[derive(Debug, Serialize)]
struct DiagnosticsEnvironmentOutput {
    agena_database_url_set: bool,
    agena_database_path_set: bool,
    agena_adapter_log_set: bool,
}

#[derive(Debug, Serialize)]
struct SessionDetail {
    id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    parent_id: Option<i64>,
    workspace_id: i64,
    title: String,
    version: i64,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
    message_count: usize,
    status: RunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    latest_event_seq: Option<i64>,
}

#[derive(Debug, Serialize)]
struct ProviderListOutput {
    providers: Vec<ProviderSummary>,
}

#[derive(Debug, Serialize)]
struct ProviderSummary {
    provider_id: String,
    defaults: ProviderDefaultsSummary,
}

#[derive(Debug, Serialize)]
struct ProviderDefaultsSummary {
    #[serde(skip_serializing_if = "Option::is_none")]
    adapter: Option<String>,
    model: String,
}

#[derive(Debug, Serialize)]
struct ProviderModelsOutput {
    provider_id: String,
    models: Vec<ProviderModel>,
}

#[derive(Debug, Serialize)]
struct ProviderCapabilitiesOutput {
    provider_id: String,
    model: String,
    model_ref: String,
    capabilities: ModelCapabilities,
    metadata: ModelMetadata,
}

#[derive(Debug, Serialize)]
struct AgentsListOutput {
    default_agent: String,
    total_count: usize,
    agents: Vec<crate::agents::AgentDescriptor>,
}

#[derive(Debug, Serialize)]
struct PluginStatusOutput {
    statuses: Vec<crate::plugin::status::PluginStatus>,
}

#[derive(Debug, Serialize)]
struct PluginInspectOutput {
    plugin: crate::plugin::PluginInspect,
}

#[derive(Debug, Serialize)]
struct PluginLogsOutput {
    plugin_id: String,
    logs: Vec<crate::plugin::PluginLogRecord>,
}

#[derive(Debug, Serialize)]
struct PluginValidateOutput {
    path: String,
    target_kind: String,
    ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    errors: Vec<PluginValidationMessage>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    warnings: Vec<PluginValidationMessage>,
}

#[derive(Debug, Clone, Serialize)]
struct PluginValidationMessage {
    code: String,
    message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<String>,
}

impl AgenaCli {
    pub fn resolved_tracing_config(&self) -> TracingConfig {
        ConfigLoader::default()
            .load(&self.load_request())
            .map(|resolution| resolution.config.tracing)
            .unwrap_or_default()
    }

    pub async fn run(
        self,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let loader = ConfigLoader::new(ProcessEnvironment);

        match self.command.clone() {
            Some(AgenaCommand::AppServer(_)) => Err(AppError::Config(
                "app-server command must be handled by the agena-cli binary".to_owned(),
            )),
            Some(AgenaCommand::Agents(command)) => self.run_agents(command).await,
            Some(AgenaCommand::Apply(args)) => self.run_apply(args),
            Some(AgenaCommand::Auth(command)) => self.run_auth(loader, command).await,
            Some(AgenaCommand::Completion(args)) => self.run_completion(args),
            Some(AgenaCommand::Config(command)) => self.run_config(loader, command),
            Some(AgenaCommand::Continue(args)) => self.run_continue(args).await,
            Some(AgenaCommand::Commit(args)) => self.run_commit(args).await,
            Some(AgenaCommand::Pr(args)) => self.run_pr(args).await,
            Some(AgenaCommand::Debug(command)) => self.run_debug(command).await,
            Some(AgenaCommand::Diagnostics(args)) => self.run_diagnostics(loader, args),
            Some(AgenaCommand::Exec(args)) => self.run_exec(args).await,
            Some(AgenaCommand::Fork(args)) => self.run_fork(args).await,
            Some(AgenaCommand::Cost(args)) => self.run_cost(args).await,
            Some(AgenaCommand::Usage(args)) => self.run_usage(args).await,
            Some(AgenaCommand::Git(args)) => self.run_git(args).await,
            Some(AgenaCommand::Login(args)) => self.run_login(loader, args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(loader, args).await,
            Some(AgenaCommand::Memory(command)) => self.run_memory(command),
            Some(AgenaCommand::McpServer(args)) => self.run_mcp_server(args).await,
            Some(AgenaCommand::Permissions(args)) => self.run_permissions(args).await,
            Some(AgenaCommand::Provider(command)) => self.run_provider(loader, command).await,
            Some(AgenaCommand::Plugin(command)) => self.run_plugin(command).await,
            Some(AgenaCommand::Resume(args)) => self.run_resume(args).await,
            Some(AgenaCommand::Review(args)) => self.run_review(args).await,
            Some(AgenaCommand::Sessions(command)) => self.run_sessions(command).await,
            Some(AgenaCommand::Tui(_)) => Err(AppError::Config(
                "tui command must be handled by the agena-cli binary".to_owned(),
            )),
            Some(AgenaCommand::Snapshot(args)) => self.run_snapshot(args).await,
            None => self.run_default(loader, tracing_reload_handle).await,
        }
    }

    async fn run_default(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        loader.load(&self.load_request())?;
        let mut builder = AgenaRuntime::builder().with_load_request(self.load_request());
        if let Some(handle) = tracing_reload_handle {
            builder = builder.with_tracing_reload_handle(handle);
        }
        let runtime = builder.build().await?;
        let snapshot = runtime.current_snapshot();
        tracing::info!(
            generation = snapshot.generation(),
            providers = snapshot.provider_registry().provider_ids().len(),
            plugins = snapshot.plugin_manager().plugins().len(),
            sessions = snapshot.session_manager().is_some(),
            "Agena started with resolved configuration"
        );
        Ok(())
    }

    fn run_apply(self, args: ApplyArgs) -> Result<(), AppError> {
        let output = self.render_apply_command(args)?;
        println!("{output}");
        Ok(())
    }

    async fn run_plugin(self, command: PluginCommand) -> Result<(), AppError> {
        use agena_plugin_marketplace::{
            InstallRequest, MarketplaceCache, MarketplaceClient, RegistrySpec, default_cache_root,
        };

        let cache = MarketplaceCache::new(default_cache_root());
        let client =
            MarketplaceClient::with_default_fetcher(cache, std::collections::BTreeMap::new());

        match command.command {
            PluginSubcommand::Status(args) => {
                let runtime = self.session_runtime().await?;
                let output = PluginStatusOutput {
                    statuses: runtime
                        .current_snapshot()
                        .plugin_manager()
                        .plugin_statuses(),
                };
                println!("{}", render_serialized(args.format, &output)?);
                Ok(())
            }
            PluginSubcommand::Inspect(args) => {
                let runtime = self.session_runtime().await?;
                let plugin = runtime
                    .current_snapshot()
                    .plugin_manager()
                    .plugin_inspect(args.plugin_id.as_str())
                    .ok_or_else(|| {
                        AppError::Config(format!("plugin not found: {}", args.plugin_id))
                    })?;
                println!(
                    "{}",
                    render_serialized(args.format, &PluginInspectOutput { plugin })?
                );
                Ok(())
            }
            PluginSubcommand::Logs(args) => {
                let runtime = self.session_runtime().await?;
                let plugin_manager = runtime.current_snapshot().plugin_manager();
                if plugin_manager
                    .plugin_status(args.plugin_id.as_str())
                    .is_none()
                {
                    return Err(AppError::Config(format!(
                        "plugin not found: {}",
                        args.plugin_id
                    )));
                }
                let output = PluginLogsOutput {
                    plugin_id: args.plugin_id.clone(),
                    logs: plugin_manager.plugin_logs(
                        args.plugin_id.as_str(),
                        args.after_seq,
                        args.limit,
                    ),
                };
                match args.format {
                    PluginLogOutputFormat::Text => {
                        println!("{}", format_plugin_logs_output(&output))
                    }
                    PluginLogOutputFormat::Json => println!(
                        "{}",
                        serde_json::to_string_pretty(&output).map_err(|err| AppError::Config(
                            format!("failed to render json output: {err}")
                        ))?
                    ),
                }
                Ok(())
            }
            PluginSubcommand::Validate(args) => {
                let output = validate_plugin_target(args.path.as_path(), args.strict)?;
                let has_errors = !output.errors.is_empty();
                println!("{}", render_plugin_validate_output(args.format, &output)?);
                if has_errors {
                    return Err(AppError::Config(format!(
                        "plugin validation failed with {} error(s)",
                        output.errors.len()
                    )));
                }
                Ok(())
            }
            PluginSubcommand::Install(args) => {
                let registry_url = args.registry.ok_or_else(|| {
                    AppError::Config("agena plugin install requires --registry <url>".to_string())
                })?;
                let (plugin_id, version) = match args.spec.split_once('@') {
                    Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
                    None => (args.spec.clone(), None),
                };
                let config_path = ConfigLoader::default().default_config_path();
                let outcome = client
                    .install(InstallRequest {
                        registry: RegistrySpec {
                            id: args.registry_id.clone(),
                            url: registry_url,
                            require_signature: args.require_signature,
                        },
                        plugin_id,
                        version,
                        config_path,
                        force: args.force,
                        dry_run: args.dry_run,
                        allow_unverified: args.allow_unverified,
                        refresh_index: args.refresh,
                    })
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if outcome.dry_run {
                    println!(
                        "DRY-RUN: would install {} v{} ({}) into {}",
                        outcome.plugin_id,
                        outcome.version,
                        outcome.kind.as_str(),
                        outcome.config_path.display()
                    );
                } else {
                    println!(
                        "Installed {} v{} ({}); restart agena to load.",
                        outcome.plugin_id,
                        outcome.version,
                        outcome.kind.as_str()
                    );
                }
                Ok(())
            }
            PluginSubcommand::Uninstall(args) => {
                let outcomes = client
                    .uninstall_with(&args.plugin_id, args.cascade)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                for outcome in outcomes {
                    println!(
                        "Uninstalled {} v{} from {}",
                        outcome.plugin_id,
                        outcome.version,
                        outcome.config_path.display()
                    );
                }
                Ok(())
            }
            PluginSubcommand::ListInstalled => {
                let records = client
                    .list_installed()
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if records.is_empty() {
                    println!("(no plugins installed via agena marketplace)");
                } else {
                    for record in records {
                        println!(
                            "{} v{} ({}) -> {}",
                            record.plugin_id,
                            record.version,
                            record.kind.as_str(),
                            record.binary_path.display()
                        );
                    }
                }
                Ok(())
            }
            PluginSubcommand::Sync(args) => {
                let registry = client.registry(RegistrySpec {
                    id: args.registry_id,
                    url: args.registry,
                    require_signature: false,
                });
                let index = registry
                    .fetch_index(true)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                println!(
                    "registry index refreshed: {} plugin(s)",
                    index.plugins.len()
                );
                Ok(())
            }
            PluginSubcommand::Search(args) => {
                let registry = client.registry(RegistrySpec {
                    id: args.registry_id,
                    url: args.registry,
                    require_signature: false,
                });
                let index = registry
                    .fetch_index(false)
                    .map_err(|err| AppError::Config(err.to_string()))?;
                let needle = args.query.to_ascii_lowercase();
                let mut hits = 0usize;
                for plugin in index.plugins {
                    let blob = format!("{} {} {}", plugin.id, plugin.name, plugin.description)
                        .to_ascii_lowercase();
                    if blob.contains(&needle) {
                        hits += 1;
                        println!(
                            "{} — {} ({} version{})",
                            plugin.id,
                            if plugin.description.is_empty() {
                                plugin.name.as_str()
                            } else {
                                plugin.description.as_str()
                            },
                            plugin.versions.len(),
                            if plugin.versions.len() == 1 { "" } else { "s" }
                        );
                    }
                }
                if hits == 0 {
                    println!("(no matches)");
                }
                Ok(())
            }
            PluginSubcommand::Upgrade(args) => {
                let override_spec = args.registry.as_ref().map(|url| RegistrySpec {
                    id: args.registry_id.clone(),
                    url: url.clone(),
                    require_signature: false,
                });
                let targets: Vec<String> = if args.all {
                    client
                        .list_installed()
                        .map_err(|err| AppError::Config(err.to_string()))?
                        .into_iter()
                        .map(|r| r.plugin_id)
                        .collect()
                } else {
                    let id = args.plugin_id.clone().ok_or_else(|| {
                        AppError::Config(
                            "agena plugin upgrade requires <plugin_id> or --all".to_string(),
                        )
                    })?;
                    vec![id]
                };
                let mut errors = Vec::new();
                for id in targets {
                    match client.upgrade(&id, override_spec.clone()) {
                        Ok(out) if out.upgraded => println!(
                            "Upgraded {} {} -> {}",
                            out.plugin_id, out.previous_version, out.installed_version
                        ),
                        Ok(out) => {
                            println!(
                                "{} is up to date (v{})",
                                out.plugin_id, out.previous_version
                            )
                        }
                        Err(err) => errors.push(format!("{id}: {err}")),
                    }
                }
                if !errors.is_empty() {
                    return Err(AppError::Config(errors.join("; ")));
                }
                Ok(())
            }
            PluginSubcommand::Outdated => {
                let outdated = client
                    .list_outdated()
                    .map_err(|err| AppError::Config(err.to_string()))?;
                if outdated.is_empty() {
                    println!("(all installed plugins are up to date)");
                } else {
                    println!("{:<32} {:<14} LATEST", "PLUGIN", "INSTALLED");
                    for record in outdated {
                        println!(
                            "{:<32} {:<14} {}",
                            record.plugin_id, record.installed_version, record.latest_version
                        );
                    }
                }
                Ok(())
            }
        }
    }

    async fn run_auth(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: AuthCommand,
    ) -> Result<(), AppError> {
        let output = self.render_auth_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_login(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LoginArgs,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let manager = AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path.clone(),
        ));
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let resolved = resolution
            .config
            .providers
            .get(provider_id.as_str())
            .ok_or_else(|| AppError::Config(format!("provider not found: {provider_id}")))?;
        let method_count = usize::from(args.api_key.is_some())
            + usize::from(args.browser)
            + usize::from(args.device);
        if method_count != 1 {
            return Err(AppError::Config(
                "login requires exactly one of --api-key, --browser, or --device".to_owned(),
            ));
        }

        if let Some(api_key) = args.api_key {
            if !matches!(
                resolved.auth,
                ProviderAuthConfig::Api(_) | ProviderAuthConfig::Gitlab(_)
            ) {
                return Err(AppError::Config(format!(
                    "{provider_id} does not support api key login"
                )));
            }
            manager.set_api_key(provider_id.as_str(), api_key)?;
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.browser {
            let timeout = Duration::from_secs(args.timeout_secs);
            match resolve_login_oauth_target(provider_id.as_str(), resolved)? {
                ProviderOAuthTarget::OpenAi => {
                    let redirect_uri = browser_login_redirect_uri(args.port);
                    let start = manager.start_openai_browser_login(redirect_uri.clone())?;
                    let pkce_verifier = start.pkce_verifier.clone();
                    let callback_provider_id = provider_id.clone();
                    complete_browser_callback_login(
                        args.port,
                        timeout,
                        &start,
                        |callback| async move {
                            manager
                                .finish_openai_browser_login(
                                    callback_provider_id.as_str(),
                                    callback.code,
                                    pkce_verifier,
                                    redirect_uri,
                                )
                                .await?;
                            Ok(())
                        },
                    )
                    .await?;
                }
                ProviderOAuthTarget::Gitlab { instance_url } => {
                    let redirect_uri = browser_login_redirect_uri(args.port);
                    let start =
                        manager.start_gitlab_login(instance_url.clone(), redirect_uri.clone())?;
                    let pkce_verifier = start.pkce_verifier.clone();
                    let callback_provider_id = provider_id.clone();
                    complete_browser_callback_login(
                        args.port,
                        timeout,
                        &start,
                        |callback| async move {
                            manager
                                .finish_gitlab_login(
                                    callback_provider_id.as_str(),
                                    instance_url,
                                    callback.code,
                                    pkce_verifier,
                                    redirect_uri,
                                )
                                .await?;
                            Ok(())
                        },
                    )
                    .await?;
                }
            }
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.device {
            let timeout = Duration::from_secs(args.timeout_secs);
            match resolve_login_device_target(provider_id.as_str(), resolved)? {
                ProviderDeviceAuthTarget::OpenAi => {
                    let start = manager.start_openai_headless_login().await?;
                    let device_code = start.device_code.clone();
                    let user_code = start.user_code.clone();
                    complete_polled_login(
                        timeout,
                        Duration::from_secs(start.interval_seconds.max(1)),
                        "openai device login timed out",
                        || prompt_device_login(&start),
                        || {
                            manager.poll_openai_headless_login(
                                provider_id.as_str(),
                                device_code.clone(),
                                user_code.clone(),
                            )
                        },
                    )
                    .await?;
                }
                ProviderDeviceAuthTarget::Copilot => {
                    let deployment =
                        copilot_deployment_from_domain(args.enterprise_domain.as_deref());
                    let start = manager.start_copilot_login(deployment.clone()).await?;
                    let device_code = start.device_code.clone();
                    complete_polled_login(
                        timeout,
                        Duration::from_secs(start.interval_seconds.max(1)),
                        "copilot device login timed out",
                        || prompt_device_login(&start),
                        || {
                            manager.poll_copilot_login(
                                provider_id.as_str(),
                                device_code.clone(),
                                deployment.clone(),
                            )
                        },
                    )
                    .await?;
                }
            }
            println!("logged in: {provider_id}");
        }

        Ok(())
    }

    async fn run_logout(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LogoutArgs,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let manager = AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path,
        ));
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        let revoke_warning = manager.logout(provider_id.as_str()).await?;
        if let Some(warning) = revoke_warning {
            eprintln!("warning: {warning}");
        }
        println!("logged out: {provider_id}");
        Ok(())
    }

    async fn run_provider(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ProviderCommand,
    ) -> Result<(), AppError> {
        let output = self.render_provider_command(&loader, command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_agents(self, command: AgentsCommand) -> Result<(), AppError> {
        let output = self.render_agents_command(command).await?;
        println!("{output}");
        Ok(())
    }

    fn run_memory(self, command: MemoryCommand) -> Result<(), AppError> {
        let output = self.render_memory_command(command)?;
        println!("{output}");
        Ok(())
    }

    async fn run_sessions(self, command: SessionsCommand) -> Result<(), AppError> {
        let output = self.render_sessions_command(command).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_resume(self, args: ResumeArgs) -> Result<(), AppError> {
        let output = self.render_resume_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_continue(self, args: ContinueArgs) -> Result<(), AppError> {
        let output = self.render_continue_command(args).await?;
        println!("{output}");
        Ok(())
    }

    fn run_completion(self, args: CompletionArgs) -> Result<(), AppError> {
        print!("{}", render_completion_command(args)?);
        Ok(())
    }

    async fn run_cost(self, args: CostArgs) -> Result<(), AppError> {
        let output = self.render_cost_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_usage(self, args: UsageArgs) -> Result<(), AppError> {
        let output = self.render_usage_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_permissions(self, args: PermissionsArgs) -> Result<(), AppError> {
        let output = self.render_permissions_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_snapshot(self, args: SnapshotArgs) -> Result<(), AppError> {
        let output = self.render_snapshot_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_git(self, args: GitArgs) -> Result<(), AppError> {
        let output = self.render_git_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_commit(self, args: CommitArgs) -> Result<(), AppError> {
        let output = self.render_commit_command(args)?;
        println!("{output}");
        Ok(())
    }

    async fn run_pr(self, args: PrArgs) -> Result<(), AppError> {
        let output = self.render_pr_command(args)?;
        println!("{output}");
        Ok(())
    }

    async fn run_debug(self, command: DebugCommand) -> Result<(), AppError> {
        let output = self.render_debug_command(command).await?;
        println!("{output}");
        Ok(())
    }

    fn run_diagnostics(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: DiagnosticsArgs,
    ) -> Result<(), AppError> {
        let output = self.render_diagnostics_command(&loader, args)?;
        println!("{output}");
        Ok(())
    }

    async fn run_exec(self, args: ExecArgs) -> Result<(), AppError> {
        let output = self.render_exec_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_fork(self, args: ForkArgs) -> Result<(), AppError> {
        let output = self.render_fork_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_mcp_server(self, args: McpServerArgs) -> Result<(), AppError> {
        let backend = self.mcp_server_backend(args).await?;
        agena_mcp_server::serve_stdio(backend)
            .await
            .map_err(|err| AppError::Config(err.to_string()))
    }

    async fn run_review(self, args: ReviewArgs) -> Result<(), AppError> {
        let output = self.render_review_command(args).await?;
        println!("{output}");
        Ok(())
    }

    fn run_config(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        command: ConfigCommand,
    ) -> Result<(), AppError> {
        match command
            .command
            .unwrap_or(ConfigSubcommand::Resolve(ConfigResolveArgs {
                format: ConfigOutputFormat::Json,
            })) {
            ConfigSubcommand::Resolve(args) => {
                let resolution = loader.load(&self.load_request())?;
                println!("{}", resolution.render(args.format)?);
            }
            ConfigSubcommand::Validate => {
                let resolution = loader.load(&self.load_request())?;
                println!(
                    "config valid: path={}",
                    resolution.meta.config_path.display()
                );
            }
        }

        Ok(())
    }

    fn render_apply_command(&self, args: ApplyArgs) -> Result<String, AppError> {
        let patch = fs::read_to_string(&args.patch_file)?;
        let workspace = args
            .workspace
            .clone()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)?;
        let plugins =
            crate::tool::default_tool_host(workspace.clone()).map_err(AppError::Config)?;
        let executor = ToolExecutor::new(
            workspace,
            Agent::new("cli", PermissionPolicy::allow_all())
                .with_tool_policy(ToolPermissionPolicy::allow_all()),
        )
        .with_plugin_manager(plugins);
        let input = ToolPayloadInput::ApplyPatch(ApplyPatchToolInput { patch }).into_invocation();
        let execution = executor
            .execute_invocation_detailed_bypassing_permissions(&input, -1, -1)
            .map_err(|err| AppError::Config(err.to_string()))?;
        let patch = execution.apply_patch.ok_or_else(|| {
            AppError::Internal("apply_patch tool did not return patch metadata".to_owned())
        })?;
        if args.json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ApplyOutput {
                    title: execution.view.title,
                    output_text: execution.view.output_text,
                    patch,
                },
            )
        } else {
            Ok(format_apply_output(&patch))
        }
    }

    async fn render_auth_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: AuthCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let manager = self.auth_manager(loader)?;
        match command
            .command
            .unwrap_or(AuthSubcommand::List(AuthListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            AuthSubcommand::List(args) => {
                let mut credentials = manager
                    .all()?
                    .into_iter()
                    .map(|(provider_id, auth)| auth_summary(provider_id, auth))
                    .collect::<Vec<_>>();
                credentials.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &AuthListOutput { credentials })
            }
        }
    }

    fn render_memory_command(&self, command: MemoryCommand) -> Result<String, AppError> {
        match command
            .command
            .unwrap_or(MemorySubcommand::List(MemoryListArgs {
                workspace: None,
                format: ConfigOutputFormat::Json,
            })) {
            MemorySubcommand::List(args) => {
                let store = self.memory_store_for_workspace(args.workspace.as_ref())?;
                let entries = store
                    .list()
                    .map_err(|error| AppError::Config(error.to_string()))?;
                let memories = entries
                    .into_iter()
                    .map(|memory| MemorySummaryOutput {
                        file_name: memory.file_name.clone(),
                        name: memory_record_name(&memory),
                        description: memory.frontmatter.description.clone(),
                        memory_type: memory_type_label(memory.frontmatter.r#type),
                        path: memory.path.display().to_string(),
                    })
                    .collect::<Vec<_>>();
                render_serialized(
                    args.format,
                    &MemoryListOutput {
                        dir: store.dir().display().to_string(),
                        count: memories.len(),
                        memories,
                    },
                )
            }
            MemorySubcommand::Forget(args) => {
                self.memory_store_for_workspace(args.workspace.as_ref())?
                    .forget(args.name.as_str())
                    .map_err(|error| AppError::Config(error.to_string()))?;
                Ok(format!("forgot memory: {}", args.name))
            }
            MemorySubcommand::Edit(args) => {
                let store = self.memory_store_for_workspace(args.workspace.as_ref())?;
                let path = match args.name.as_deref() {
                    Some(name) => {
                        store
                            .get(name)
                            .map_err(|error| AppError::Config(error.to_string()))?
                            .path
                    }
                    None => ensure_memory_index_path(&store)?,
                };
                Ok(path.display().to_string())
            }
        }
    }

    async fn render_sessions_command(&self, command: SessionsCommand) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        match command
            .command
            .unwrap_or(SessionsSubcommand::List(SessionListArgs {
                limit: 20,
                offset: 0,
                view: SessionListView::All,
                anchor_session_id: None,
                format: ConfigOutputFormat::Json,
            })) {
            SessionsSubcommand::List(args) => {
                let sessions = list_all_session_summaries(manager.as_ref()).await?;
                let sessions =
                    filter_session_summaries_by_view(sessions, args.view, args.anchor_session_id)?;
                let sessions = paginate_session_summaries(sessions, args.offset, args.limit);
                render_serialized(args.format, &SessionListOutput { sessions })
            }
            SessionsSubcommand::Export(args) => {
                let bundle = manager.export_session_jsonl(args.session_id).await?;
                Ok(bundle)
            }
            SessionsSubcommand::Import(args) => {
                let bundle = match args.path {
                    Some(path) => std::fs::read_to_string(&path).map_err(|err| {
                        AppError::Internal(format!("read import bundle {}: {err}", path.display()))
                    })?,
                    None => {
                        use std::io::Read;
                        let mut buf = String::new();
                        std::io::stdin()
                            .read_to_string(&mut buf)
                            .map_err(|err| AppError::Internal(format!("read stdin: {err}")))?;
                        buf
                    }
                };
                let session = manager.import_session_jsonl(&bundle).await?;
                let latest_event_seq = latest_event_seq(&manager, session.id).await?;
                render_serialized(
                    args.format,
                    &SessionImportOutput {
                        session: session_detail(&session, latest_event_seq),
                    },
                )
            }
            SessionsSubcommand::Tree(args) => {
                let mut sessions = manager.list_session_tree(args.root_id).await?;
                if let Some(max_depth) = args.max_depth {
                    let root_depth = sessions.first().map(|first| first.depth).unwrap_or(0);
                    sessions.retain(|s| s.depth - root_depth <= max_depth);
                }
                if let Some(limit) = args.limit {
                    sessions.truncate(limit);
                }
                render_serialized(args.format, &SessionListOutput { sessions })
            }
        }
    }

    async fn render_resume_command(&self, args: ResumeArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = if args.agent.is_some() {
            let mut options = SessionRunOptions::new(default_model(&runtime)?);
            options.agent_profile = args
                .agent
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned);
            manager
                .continue_session(SessionExecutionRequest::new(session_id, options))
                .await?
        } else {
            manager.get_session(session_id).await?
        };
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_cost_command(&self, args: CostArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let output = CostOutput {
            session: session_detail(&session, latest_event_seq),
            summary: crate::session::cost::summarize(&session.messages),
        };
        render_serialized(args.format, &output)
    }

    async fn render_usage_command(&self, args: UsageArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let query = usage_stats_query_from_args(&args)?;
        let output = manager.usage_stats(query).await?;
        render_serialized(args.format, &output)
    }

    async fn render_permissions_command(&self, args: PermissionsArgs) -> Result<String, AppError> {
        match args
            .command
            .unwrap_or(PermissionsSubcommand::List(PermissionsListArgs {
                search: None,
                format: ConfigOutputFormat::Json,
            })) {
            PermissionsSubcommand::List(args) => self.render_permissions_list_command(args).await,
            PermissionsSubcommand::Create(args) => {
                self.render_permissions_create_command(args).await
            }
            PermissionsSubcommand::Replace(args) => {
                self.render_permissions_replace_command(args).await
            }
            PermissionsSubcommand::Revoke(args) => {
                self.render_permissions_revoke_command(args).await
            }
            PermissionsSubcommand::Reply(args) => self.render_permissions_reply_command(args).await,
        }
    }

    async fn render_permissions_list_command(
        &self,
        args: PermissionsListArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;

        let mut query = entities::permission_rule::Entity::find()
            .order_by_desc(entities::permission_rule::Column::UpdatedAtMs)
            .order_by_desc(entities::permission_rule::Column::Id);
        if let Some(search) = args
            .search
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            query = query
                .filter(entities::permission_rule::Column::ActionKey.like(format!("%{search}%")));
        }
        let rows = query.all(&db).await?;
        let rules = rows
            .into_iter()
            .map(permission_rule_output)
            .collect::<Result<Vec<_>, AppError>>()?;
        render_serialized(
            args.format,
            &PermissionsOutput {
                count: rules.len(),
                rules,
            },
        )
    }

    async fn render_permissions_create_command(
        &self,
        args: PermissionsWriteArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let workspace_root = self.resolve_workspace_root(None)?;
        let created =
            upsert_permission_rule_from_args(&db, workspace_root.as_path(), &args).await?;
        render_serialized(args.format, &created)
    }

    async fn render_permissions_replace_command(
        &self,
        args: PermissionsReplaceArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let workspace_root = self.resolve_workspace_root(None)?;
        let updated = replace_permission_rule_from_args(
            &db,
            workspace_root.as_path(),
            args.rule_id,
            &args.rule,
        )
        .await?;
        render_serialized(args.rule.format, &updated)
    }

    async fn render_permissions_revoke_command(
        &self,
        args: PermissionsRevokeArgs,
    ) -> Result<String, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let db = tracing_config::connect_database(
            database_url.as_str(),
            &self.resolved_tracing_config(),
        )
        .await?;
        init_schema(&db).await?;
        let updated = permission_rule_crud::revoke_rule(
            &db,
            args.rule_id,
            args.reason,
            Some("cli".to_string()),
        )
        .await?;
        let Some(updated) = updated else {
            return Err(AppError::Config(format!(
                "permission rule not found: {}",
                args.rule_id
            )));
        };
        render_serialized(args.format, &permission_rule_output(updated)?)
    }

    async fn render_permissions_reply_command(
        &self,
        args: PermissionsReplyArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager
            .reply_permission(crate::session::SessionPermissionReplyRequest::new(
                session_id,
                resolve_run_options(&runtime, None, None, None, None)?,
                PermissionReply {
                    request_id: args.request_id,
                    kind: permission_reply_kind_from_arg(args.kind),
                    reason: args.reason,
                    scope: args.scope.map(permission_scope_from_arg),
                },
                Some("cli".to_string()),
            ))
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_snapshot_command(&self, args: SnapshotArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let executor = manager.tool_executor();
        let registry = executor.snapshot_registry().ok_or_else(|| {
            AppError::Config("snapshot registry is not enabled in this runtime".to_owned())
        })?;
        let capabilities = crate::tool::snapshot_backend_capabilities(runtime.workspace_root());
        let active = crate::tool::snapshot_list_active(registry)
            .into_iter()
            .map(|entry| ActiveSnapshotOutput {
                session_id: entry.session_id,
                path: entry.path.display().to_string(),
                branch: entry.branch,
                backend: entry.backend.as_str().to_string(),
                created_here: entry.created_here,
            })
            .collect::<Vec<_>>();
        let managed = crate::tool::snapshot_list_managed(runtime.workspace_root(), registry)
            .into_iter()
            .map(|entry: crate::tool::ManagedSnapshot| {
                let stale = entry.is_stale();
                ManagedSnapshotOutput {
                    path: entry.path.display().to_string(),
                    session_id: entry.session_id,
                    branch: entry.branch,
                    backend: entry
                        .backend
                        .map(|backend: crate::tool::SnapshotBackend| backend.as_str().to_string()),
                    registered_with_git: entry.registered_with_git,
                    registered_with_rift: entry.registered_with_rift,
                    stale,
                }
            })
            .collect::<Vec<_>>();
        render_serialized(
            args.format,
            &SnapshotOutput {
                workspace_root: runtime.workspace_root().display().to_string(),
                capabilities: SnapshotCapabilitiesOutput {
                    preferred_backend: capabilities
                        .preferred_backend
                        .map(|backend: crate::tool::SnapshotBackend| backend.as_str().to_string()),
                    git: SnapshotBackendSupportOutput {
                        available: capabilities.git.available,
                        detail: capabilities.git.detail,
                    },
                    rift: SnapshotBackendSupportOutput {
                        available: capabilities.rift.available,
                        detail: capabilities.rift.detail,
                    },
                },
                active,
                managed,
            },
        )
    }

    async fn render_git_command(&self, args: GitArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let workspace_root = runtime.workspace_root().to_path_buf();
        let preflight = collect_git_preflight(&workspace_root)?;

        let (snapshot_active_sessions, snapshot_managed_dirs) = match runtime.session_manager() {
            Some(manager) => {
                let executor = manager.tool_executor();
                match executor.snapshot_registry() {
                    Some(registry) => (
                        crate::tool::snapshot_list_active(registry).len() as u64,
                        crate::tool::snapshot_list_managed(runtime.workspace_root(), registry).len()
                            as u64,
                    ),
                    None => (0, 0),
                }
            }
            None => (0, 0),
        };

        render_serialized(
            args.format,
            &GitOutput {
                workspace_root: workspace_root.display().to_string(),
                git_available: preflight.git_available,
                repo: preflight.repo,
                gh_available: preflight.gh_available,
                branch: preflight.branch,
                upstream: preflight.upstream,
                ahead: preflight.ahead,
                behind: preflight.behind,
                staged_files: preflight.staged_files,
                unstaged_files: preflight.unstaged_files,
                untracked_files: preflight.untracked_files,
                changed_files: preflight.changed_files,
                clean: preflight.clean,
                snapshot_active_sessions,
                snapshot_managed_dirs,
            },
        )
    }

    fn render_commit_command(&self, args: CommitArgs) -> Result<String, AppError> {
        let workspace_root = self.resolve_workspace_root(None)?;
        let preflight = collect_git_preflight(&workspace_root)?;
        if !preflight.git_available {
            return Err(AppError::Config("git is not available in PATH".to_owned()));
        }
        if !preflight.repo {
            return Err(AppError::Config(format!(
                "not a git repository: {}",
                workspace_root.display()
            )));
        }
        if preflight.staged_files == 0 {
            return Err(AppError::Config("no staged changes to commit".to_owned()));
        }
        let output = Command::new("git")
            .args(["commit", "-m", args.message.as_str()])
            .current_dir(&workspace_root)
            .output()?;
        if !output.status.success() {
            return Err(AppError::Config(format!(
                "git commit failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let commit = git_output(&workspace_root, ["rev-parse", "HEAD"])?;
        let summary = git_output(&workspace_root, ["log", "-1", "--pretty=%s"])?;
        render_serialized(
            args.format,
            &CommitOutput {
                workspace_root: workspace_root.display().to_string(),
                commit,
                summary,
            },
        )
    }

    fn render_pr_command(&self, args: PrArgs) -> Result<String, AppError> {
        let workspace_root = self.resolve_workspace_root(None)?;
        let preflight = collect_git_preflight(&workspace_root)?;
        if !preflight.git_available {
            return Err(AppError::Config("git is not available in PATH".to_owned()));
        }
        if !preflight.gh_available {
            return Err(AppError::Config("gh is not available in PATH".to_owned()));
        }
        if !preflight.repo {
            return Err(AppError::Config(format!(
                "not a git repository: {}",
                workspace_root.display()
            )));
        }
        let branch = args
            .head
            .clone()
            .or(preflight.branch.clone())
            .ok_or_else(|| AppError::Config("could not determine current branch".to_owned()))?;

        let mut command = Command::new("gh");
        command
            .arg("pr")
            .arg("create")
            .arg("--title")
            .arg(args.title);
        command.arg("--body").arg(args.body.unwrap_or_default());
        if let Some(base) = args.base {
            command.arg("--base").arg(base);
        }
        if let Some(head) = args.head {
            command.arg("--head").arg(head);
        }
        command.current_dir(&workspace_root);

        let output = command.output()?;
        if !output.status.success() {
            return Err(AppError::Config(format!(
                "gh pr create failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        let url = String::from_utf8_lossy(&output.stdout).trim().to_string();
        render_serialized(
            args.format,
            &PrOutput {
                workspace_root: workspace_root.display().to_string(),
                branch,
                url,
            },
        )
    }

    async fn render_continue_command(&self, args: ContinueArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session_id = selected_session_id(&manager, args.session_id, args.last).await?;
        let session = manager.get_session(session_id).await?;
        let options = resolve_continue_options(&runtime, &session, &args)?;
        let session = manager
            .continue_session(SessionExecutionRequest::new(session_id, options))
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_debug_command(&self, command: DebugCommand) -> Result<String, AppError> {
        match command.command {
            DebugSubcommand::Session(args) => self.render_debug_session_command(args).await,
        }
    }

    async fn render_debug_session_command(
        &self,
        args: DebugSessionArgs,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let session = manager.get_session(args.session_id).await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let output = DebugSessionOutput {
            session: session_detail(&session, latest_event_seq),
            messages: session
                .messages
                .iter()
                .map(|message| DebugMessageOutput {
                    id: message.id,
                    role: message.role,
                    state: message.state,
                    text: message.as_text_lossy(),
                })
                .collect(),
        };
        if args.json {
            render_serialized(ConfigOutputFormat::Json, &output)
        } else {
            Ok(format_debug_session_output(&output))
        }
    }

    fn render_diagnostics_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        args: DiagnosticsArgs,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        let config = &resolution.config;
        render_serialized(
            args.format,
            &DiagnosticsOutput {
                version: env!("CARGO_PKG_VERSION"),
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
                current_dir: std::env::current_dir()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| "<unavailable>".to_owned()),
                config: DiagnosticsConfigOutput {
                    path: resolution.meta.config_path.display().to_string(),
                    found: resolution.meta.config_found,
                    project_path: resolution.meta.project_config_path.display().to_string(),
                    project_found: resolution.meta.project_config_found,
                    applied_layers: resolution
                        .meta
                        .applied_layers
                        .iter()
                        .map(|layer| layer.description.clone())
                        .collect(),
                    provider_count: config.providers.len(),
                    plugin_count: config.plugins.list.len(),
                },
                environment: DiagnosticsEnvironmentOutput {
                    agena_database_url_set: std::env::var_os("AGENA_DATABASE_URL").is_some(),
                    agena_database_path_set: std::env::var_os("AGENA_DATABASE_PATH").is_some(),
                    agena_adapter_log_set: std::env::var_os("AGENA_ADAPTER_LOG").is_some(),
                },
            },
        )
    }

    async fn render_exec_command(&self, args: ExecArgs) -> Result<String, AppError> {
        self.render_prompt_command(
            args.workspace.as_ref(),
            args.prompt.as_str(),
            title_from_prompt(args.prompt.as_str()),
            args.model.as_deref(),
            args.agent.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    async fn render_review_command(&self, args: ReviewArgs) -> Result<String, AppError> {
        let prompt = review_prompt(args.base.as_str());
        self.render_prompt_command(
            args.workspace.as_ref(),
            prompt.as_str(),
            format!("Review changes against {}", args.base),
            args.model.as_deref(),
            args.agent.as_deref(),
            args.temperature,
            args.max_output_tokens,
            args.json,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    async fn render_prompt_command(
        &self,
        workspace: Option<&PathBuf>,
        prompt: &str,
        title: String,
        model: Option<&str>,
        agent_profile: Option<&str>,
        temperature: Option<f32>,
        max_output_tokens: Option<u32>,
        json: bool,
    ) -> Result<String, AppError> {
        let runtime = self.session_runtime_with_workspace(workspace).await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let options = resolve_run_options(
            &runtime,
            model,
            agent_profile,
            temperature,
            max_output_tokens,
        )?;
        let created = manager
            .create_session(SessionCreateRequest {
                title,
                parent_session_id: None,
            })
            .await?;
        let session = manager
            .submit_user_message(SessionUserMessageRequest::new(
                created.id,
                options,
                vec![PartContent::text(prompt)],
            ))
            .await?;
        if session.runtime.run.status == RunStatus::Blocked {
            return Err(AppError::Config(
                "command is blocked awaiting permission or user input".to_owned(),
            ));
        }
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        let text = last_assistant_text(&session).unwrap_or_default();
        if json {
            render_serialized(
                ConfigOutputFormat::Json,
                &ExecOutput {
                    session: session_detail(&session, latest_event_seq),
                    text,
                },
            )
        } else {
            Ok(text)
        }
    }

    async fn render_fork_command(&self, args: ForkArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let forked = manager
            .fork_session(SessionForkRequest {
                session_id: args.session_id,
                at_message_id: args.at_message,
                title: args.title,
                expected_version: None,
            })
            .await?;
        let latest_event_seq = latest_event_seq(&manager, forked.id).await?;
        render_serialized(
            args.format,
            &SessionForkOutput {
                source_session_id: args.session_id,
                forked: session_detail(&forked, latest_event_seq),
            },
        )
    }

    async fn session_runtime(&self) -> Result<AgenaRuntime, AppError> {
        self.session_runtime_with_workspace(None).await
    }

    async fn session_runtime_with_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<AgenaRuntime, AppError> {
        let storage = StorageConfig {
            database_url: self.database_url.clone(),
            database_path: self.database_path.clone(),
        };
        let database_url = storage.resolve_url()?;
        StorageConfig::ensure_parent(database_url.as_str())?;
        let mut load_request = self.load_request();
        if let Some(workspace) = workspace {
            load_request.workspace_root = Some(workspace.clone());
        }
        let mut builder = AgenaRuntime::builder()
            .with_load_request(load_request)
            .with_database_url(database_url);
        if let Some(workspace) = workspace {
            builder = builder.with_workspace_root(workspace.clone());
        }
        builder.build().await
    }

    fn memory_store_for_workspace(
        &self,
        workspace: Option<&PathBuf>,
    ) -> Result<MemoryStore, AppError> {
        Ok(MemoryStore::for_workspace(
            self.resolve_workspace_root(workspace)?.as_path(),
        ))
    }

    fn resolve_workspace_root(&self, workspace: Option<&PathBuf>) -> Result<PathBuf, AppError> {
        workspace
            .cloned()
            .map(Ok)
            .unwrap_or_else(std::env::current_dir)
            .map_err(AppError::from)
    }

    fn auth_manager<E>(
        &self,
        loader: &ConfigLoader<E>,
    ) -> Result<AuthManager<ProviderConfigCredentialStore>, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        Ok(AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path,
        )))
    }

    async fn render_provider_command<E>(
        &self,
        loader: &ConfigLoader<E>,
        command: ProviderCommand,
    ) -> Result<String, AppError>
    where
        E: ConfigEnvironment,
    {
        let resolution = loader.load(&self.load_request())?;
        let registry = resolution
            .config
            .build_provider_registry_with_env(loader.environment())?;

        match command
            .command
            .unwrap_or(ProviderSubcommand::List(ProviderListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            ProviderSubcommand::List(args) => {
                let mut providers = registry
                    .provider_ids()
                    .into_iter()
                    .filter_map(|provider_id| {
                        registry
                            .get(provider_id.as_str())
                            .map(|provider| ProviderSummary {
                                defaults: ProviderDefaultsSummary {
                                    adapter: provider.default_adapter().map(ToString::to_string),
                                    model: provider.default_model().to_string(),
                                },
                                provider_id,
                            })
                    })
                    .collect::<Vec<_>>();
                providers.sort_by(|left, right| left.provider_id.cmp(&right.provider_id));
                render_serialized(args.format, &ProviderListOutput { providers })
            }
            ProviderSubcommand::Models(args) => {
                let models = registry.list_models(args.provider_id.as_str()).await?;
                render_serialized(
                    args.format,
                    &ProviderModelsOutput {
                        provider_id: args.provider_id,
                        models,
                    },
                )
            }
            ProviderSubcommand::Capabilities(args) => {
                let model_ref =
                    registry.resolve_model_target(args.target.as_str(), args.model.as_deref())?;
                let capabilities = registry.model_capabilities(&model_ref)?;
                let metadata = registry.model_metadata(&model_ref)?;
                render_serialized(
                    args.format,
                    &ProviderCapabilitiesOutput {
                        provider_id: model_ref.provider_id.to_string(),
                        model: model_ref.model_id.to_string(),
                        model_ref: model_ref.to_string(),
                        capabilities,
                        metadata,
                    },
                )
            }
        }
    }

    async fn render_agents_command(&self, command: AgentsCommand) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let snapshot = runtime.current_snapshot();
        let resolution = snapshot.config_resolution();
        let mut agents = snapshot.agents().list_descriptors();
        agents.sort_by(|left, right| left.name.cmp(&right.name));
        let default_agent = resolution
            .config
            .default_agent
            .clone()
            .filter(|name| agents.iter().any(|entry| entry.name == *name))
            .or_else(|| agents.iter().map(|entry| entry.name.clone()).next())
            .unwrap_or_else(|| "none".to_string());
        let total_count = agents.len();

        match command
            .command
            .unwrap_or(AgentsSubcommand::List(AgentsListArgs {
                format: ConfigOutputFormat::Json,
            })) {
            AgentsSubcommand::List(args) => render_serialized(
                args.format,
                &AgentsListOutput {
                    default_agent,
                    total_count,
                    agents,
                },
            ),
        }
    }

    async fn mcp_server_backend(&self, args: McpServerArgs) -> Result<AgenaMcpBackend, AppError> {
        let runtime = self
            .session_runtime_with_workspace(args.workspace.as_ref())
            .await?;
        let snapshot = runtime.current_snapshot();
        let plugins = snapshot.plugin_manager();
        let session_manager = runtime.session_manager();
        let executor = session_manager.as_ref().map_or_else(
            || {
                let agent = Agent::new("mcp-server", PermissionPolicy::allow_all())
                    .with_tool_policy(ToolPermissionPolicy::allow_all());
                ToolExecutor::new(runtime.workspace_root().to_path_buf(), agent)
                    .with_plugin_manager(Arc::clone(&plugins))
            },
            |manager| manager.tool_executor(),
        );
        Ok(AgenaMcpBackend {
            executor,
            plugins,
            session_manager,
            workspace_root: runtime.workspace_root().to_path_buf(),
            next_call_id: Arc::new(AtomicI64::new(1)),
        })
    }

    pub fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            overrides: self.overrides.clone(),
            workspace_root: None,
        }
    }
}

#[derive(Clone)]
struct AgenaMcpBackend {
    executor: ToolExecutor,
    plugins: Arc<crate::plugin::PluginHost>,
    session_manager: Option<Arc<SessionManager>>,
    workspace_root: PathBuf,
    next_call_id: Arc<AtomicI64>,
}

#[async_trait]
impl McpServerBackend for AgenaMcpBackend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        Ok(self
            .executor
            .available_tools()
            .into_iter()
            .map(|tool| {
                let description = tool.description_text().to_string();
                let before_help = tool.before_help_text().map(ToString::to_string);
                let after_help = tool.after_help_text().map(ToString::to_string);
                let input_schema = tool.sanitized_input_schema();
                let aliases = tool.alias_exposed_names();
                ToolDescriptor {
                    name: tool.exposed_name,
                    aliases,
                    description: Some(description),
                    before_help,
                    after_help,
                    input_schema: Some(input_schema),
                }
            })
            .collect())
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        let name = params.name;
        let input = structured_tool_input(params.arguments)?;
        let invocation = mcp_tool_invocation(name.as_str(), input)?;
        let call_id = self.next_call_id.fetch_add(1, Ordering::SeqCst);
        let result = self
            .executor
            .execute_invocation_detailed(&invocation, -1, call_id);
        match result {
            Ok(execution) => {
                self.audit_tool_call(name.as_str(), call_id, false, None)
                    .await;
                let text = if execution.view.output_text.is_empty() {
                    serde_json::to_string_pretty(&execution.output)
                        .unwrap_or_else(|_| "<empty output>".to_owned())
                } else {
                    execution.view.output_text
                };
                Ok(agena_mcp_server::text_result(text))
            }
            Err(err) => {
                let message = err.to_string();
                self.audit_tool_call(name.as_str(), call_id, true, Some(message.as_str()))
                    .await;
                Ok(agena_mcp_server::text_error(message))
            }
        }
    }

    async fn list_resources(&self) -> Result<Vec<ResourceDescriptor>, McpServerError> {
        let mut resources = vec![ResourceDescriptor {
            uri: "agena://workspace".to_owned(),
            name: Some("Workspace".to_owned()),
            description: Some("Current Agena workspace root".to_owned()),
            mime_type: Some("text/plain".to_owned()),
        }];
        if self.session_manager.is_some() {
            resources.push(ResourceDescriptor {
                uri: "agena://sessions".to_owned(),
                name: Some("Sessions".to_owned()),
                description: Some("Recent Agena session metadata".to_owned()),
                mime_type: Some("application/json".to_owned()),
            });
        }
        Ok(resources)
    }

    async fn read_resource(
        &self,
        params: ReadResourceParams,
    ) -> Result<ReadResourceResult, McpServerError> {
        let (text, mime_type) = match params.uri.as_str() {
            "agena://workspace" => (
                self.workspace_root.display().to_string(),
                Some("text/plain".to_owned()),
            ),
            "agena://sessions" => {
                let manager = self
                    .session_manager
                    .as_ref()
                    .ok_or_else(|| McpServerError::NotFound(params.uri.clone()))?;
                let sessions = manager
                    .list_session_summaries(SessionListRequest {
                        offset: 0,
                        limit: Some(50),
                        include_subagents: false,
                    })
                    .await
                    .map_err(|err| McpServerError::Backend(err.to_string()))?;
                (
                    serde_json::to_string_pretty(&sessions)
                        .map_err(|err| McpServerError::Backend(err.to_string()))?,
                    Some("application/json".to_owned()),
                )
            }
            other => return Err(McpServerError::NotFound(other.to_owned())),
        };
        Ok(ReadResourceResult {
            contents: vec![ResourceContents {
                uri: params.uri,
                mime_type,
                text: Some(text),
                blob: None,
            }],
        })
    }

    async fn list_prompts(&self) -> Result<Vec<PromptDescriptor>, McpServerError> {
        let mut prompts = self
            .plugins
            .registered_tools()
            .into_iter()
            .filter(|entry| matches!(entry.plugin_name.as_str(), "agena.skills"))
            .map(|entry| PromptDescriptor {
                name: entry.exposed_name,
                description: entry.decl.description,
                arguments: Vec::new(),
            })
            .collect::<Vec<_>>();
        prompts.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(prompts)
    }

    async fn get_prompt(&self, params: GetPromptParams) -> Result<GetPromptResult, McpServerError> {
        let entry = self
            .plugins
            .lookup_tool(params.name.as_str())
            .ok_or_else(|| McpServerError::NotFound(params.name.clone()))?;
        if !matches!(entry.plugin_name.as_str(), "agena.skills") {
            return Err(McpServerError::NotFound(params.name));
        }

        let args = params.arguments.and_then(|arguments| {
            if arguments.is_empty() {
                None
            } else {
                Some(
                    arguments
                        .into_iter()
                        .map(|(key, value)| format!("- {key}: {value}"))
                        .collect::<Vec<_>>()
                        .join("\n"),
                )
            }
        });
        let invocation = ToolInvocation::new(
            entry.exposed_name.clone(),
            StructuredObject::try_from(serde_json::json!({ "args": args }))
                .map_err(McpServerError::InvalidParams)?,
        );
        let execution = self
            .executor
            .execute_invocation_detailed(&invocation, -1, -1)
            .map_err(|err| McpServerError::Backend(err.to_string()))?;
        Ok(GetPromptResult {
            description: entry.decl.description,
            messages: vec![PromptMessage {
                role: "user".to_owned(),
                content: ContentBlock::Text {
                    text: execution.view.output_text,
                },
            }],
        })
    }
}

impl AgenaMcpBackend {
    async fn audit_tool_call(
        &self,
        tool_name: &str,
        call_id: i64,
        is_error: bool,
        error: Option<&str>,
    ) {
        let Some(manager) = self.session_manager.as_ref() else {
            return;
        };
        let payload = serde_json::json!({
            "tool_name": tool_name,
            "call_id": call_id,
            "is_error": is_error,
            "error": error,
        });
        let _ = manager
            .event_publisher()
            .publish(
                Default::default(),
                crate::event::EventKind::PluginEvent(crate::event::PluginEventPayload {
                    plugin_id: "agena.mcp_server".to_owned(),
                    kind_label: "mcp_tool_call".to_owned(),
                    payload,
                }),
            )
            .await;
    }
}

fn structured_tool_input(
    arguments: Option<serde_json::Value>,
) -> Result<StructuredObject, McpServerError> {
    StructuredObject::try_from(arguments.unwrap_or_else(|| serde_json::json!({})))
        .map_err(McpServerError::InvalidParams)
}

fn mcp_tool_invocation(
    name: &str,
    input: StructuredObject,
) -> Result<ToolInvocation, McpServerError> {
    Ok(ToolInvocation::new(name.to_owned(), input))
}

fn ensure_memory_index_path(store: &MemoryStore) -> Result<PathBuf, AppError> {
    store.ensure_exists()?;
    let path = store.dir().join("MEMORY.md");
    if !path.exists() {
        fs::write(&path, "")?;
    }
    Ok(path)
}

fn memory_record_name(entry: &crate::memory::MemoryRecord) -> String {
    if entry.frontmatter.name.trim().is_empty() {
        entry.file_name.trim_end_matches(".md").to_string()
    } else {
        entry.frontmatter.name.clone()
    }
}

fn memory_type_label(memory_type: Option<MemoryType>) -> Option<String> {
    memory_type.map(|value| value.label().to_string())
}

fn permission_mode_from_arg(mode: PermissionModeArg) -> PermissionMode {
    match mode {
        PermissionModeArg::Allow => PermissionMode::Allow,
        PermissionModeArg::Ask => PermissionMode::Ask,
        PermissionModeArg::Deny => PermissionMode::Deny,
    }
}

fn permission_scope_from_arg(scope: PermissionScopeArg) -> PermissionScope {
    match scope {
        PermissionScopeArg::Session => PermissionScope::Session,
        PermissionScopeArg::Workspace => PermissionScope::Workspace,
        PermissionScopeArg::Global => PermissionScope::Global,
    }
}

fn permission_reply_kind_from_arg(kind: PermissionReplyKindArg) -> PermissionReplyKind {
    match kind {
        PermissionReplyKindArg::AllowOnce => PermissionReplyKind::AllowOnce,
        PermissionReplyKindArg::AllowAlways => PermissionReplyKind::AllowAlways,
        PermissionReplyKindArg::DenyOnce => PermissionReplyKind::DenyOnce,
        PermissionReplyKindArg::DenyAlways => PermissionReplyKind::DenyAlways,
    }
}

fn permission_rule_output(
    row: entities::permission_rule::Model,
) -> Result<PermissionRuleOutput, AppError> {
    Ok(PermissionRuleOutput {
        id: row.id,
        action_key: row.action_key,
        mode: row.mode,
        scope: row.scope,
        session_id: row.session_id,
        workspace_id: row.workspace_id,
        source: row.source,
        reason: row.reason,
        operator: row.operator,
        revoked_at: timestamp_ms_to_datetime(row.revoked_at_ms)?,
        revoked_reason: row.revoked_reason,
        revoked_by: row.revoked_by,
        created_at: required_timestamp_ms_to_datetime(
            "permission rule created_at_ms",
            row.created_at_ms,
        )?,
        updated_at: required_timestamp_ms_to_datetime(
            "permission rule updated_at_ms",
            row.updated_at_ms,
        )?,
    })
}

fn required_timestamp_ms_to_datetime(label: &str, value: i64) -> Result<DateTime<Utc>, AppError> {
    DateTime::<Utc>::from_timestamp_millis(value)
        .ok_or_else(|| AppError::Internal(format!("invalid {label}: {value}")))
}

fn timestamp_ms_to_datetime(value: Option<i64>) -> Result<Option<DateTime<Utc>>, AppError> {
    value
        .map(|value| required_timestamp_ms_to_datetime("timestamp_ms", value))
        .transpose()
}

fn permission_action_from_args(
    workspace_root: &Path,
    args: &PermissionsWriteArgs,
) -> Result<PermissionAction, AppError> {
    if let Some(action_key) = args
        .action_key
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return serde_json::from_str(action_key)
            .map_err(|err| AppError::Config(format!("invalid action_key json: {err}")));
    }
    if let Some(tool_name) = args
        .tool_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        return Ok(PermissionAction::Tool {
            tool_name: tool_name.to_string(),
            qualifier: args
                .qualifier
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToOwned::to_owned),
        });
    }
    if let Some(target) = args
        .network_target
        .as_deref()
        .or(args.network_host.as_deref())
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        let parse_target = args
            .network_port
            .map(|port| format!("{target}:{port}"))
            .unwrap_or_else(|| target.to_string());
        let parsed = crate::permission::NetworkTarget::parse(&parse_target)
            .map_err(|err| AppError::Config(format!("invalid network target: {err}")))?;
        return Ok(PermissionAction::NetworkAccess {
            target: target.to_string(),
            host: parsed.host().to_string(),
            port: parsed.port(),
        });
    }
    let path_access_kind = args
        .path_access_kind
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Config(
                "permission rule requires either --action-key, or tool/path fields".to_string(),
            )
        })?;
    let target_path = args
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| AppError::Config("path_access rules require --target-path".to_string()))?;
    let workspace_root_value = args
        .workspace_root
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| workspace_root.to_string_lossy().to_string());
    Ok(PermissionAction::PathAccess {
        access_kind: path_access_kind.to_string(),
        workspace_root: workspace_root_value,
        target_path: target_path.to_string(),
    })
}

async fn upsert_permission_rule_from_args(
    db: &sea_orm::DatabaseConnection,
    workspace_root: &Path,
    args: &PermissionsWriteArgs,
) -> Result<PermissionRuleOutput, AppError> {
    let scope = permission_scope_from_arg(args.scope);
    let action = permission_action_from_args(workspace_root, args)?;
    let action_key = serde_json::to_string(&action).map_err(AppError::from)?;
    let workspace_id = match scope {
        PermissionScope::Workspace => Some(
            workspace_crud::ensure_workspace_id(db, workspace_root.to_string_lossy().as_ref())
                .await?,
        ),
        PermissionScope::Session | PermissionScope::Global => None,
    };
    let session_id = match scope {
        PermissionScope::Session => args.session_id,
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    if matches!(scope, PermissionScope::Session) && session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_string(),
        ));
    }
    let (row, _) = permission_rule_crud::upsert_rule(
        db,
        &PersistedPermissionRule {
            action_key,
            mode: permission_mode_from_arg(args.rule_mode),
            scope,
            session_id,
            workspace_id,
            source: "cli".to_string(),
            reason: None,
            operator: Some("cli".to_string()),
            revoked_at_ms: None,
            revoked_reason: None,
            revoked_by: None,
        },
    )
    .await?;
    permission_rule_output(row)
}

async fn replace_permission_rule_from_args(
    db: &sea_orm::DatabaseConnection,
    workspace_root: &Path,
    rule_id: i64,
    args: &PermissionsWriteArgs,
) -> Result<PermissionRuleOutput, AppError> {
    let existing = entities::permission_rule::Entity::find_by_id(rule_id)
        .one(db)
        .await?
        .ok_or_else(|| AppError::Config(format!("permission rule not found: {rule_id}")))?;
    let scope = permission_scope_from_arg(args.scope);
    let action = permission_action_from_args(workspace_root, args)?;
    let action_key = serde_json::to_string(&action).map_err(AppError::from)?;
    let workspace_id = match scope {
        PermissionScope::Workspace => Some(
            workspace_crud::ensure_workspace_id(db, workspace_root.to_string_lossy().as_ref())
                .await?,
        ),
        PermissionScope::Session | PermissionScope::Global => None,
    };
    let session_id = match scope {
        PermissionScope::Session => args.session_id,
        PermissionScope::Workspace | PermissionScope::Global => None,
    };
    if matches!(scope, PermissionScope::Session) && session_id.is_none() {
        return Err(AppError::Config(
            "session scope requires --session-id".to_string(),
        ));
    }
    let mut active: entities::permission_rule::ActiveModel = existing.into();
    active.action_key = Set(action_key);
    active.mode = Set(permission_rule_crud::mode_to_string(
        permission_mode_from_arg(args.rule_mode),
    ));
    active.scope = Set(permission_rule_crud::scope_to_string(scope));
    active.session_id = Set(session_id);
    active.workspace_id = Set(workspace_id);
    active.source = Set("cli".to_string());
    active.operator = Set(Some("cli".to_string()));
    active.updated_at_ms = Set(Utc::now().timestamp_millis());
    let row = active.update(db).await?;
    permission_rule_output(row)
}

fn render_completion_command(args: CompletionArgs) -> Result<String, AppError> {
    let mut command = AgenaCli::command();
    let mut buffer = Vec::new();
    clap_complete::generate(args.shell, &mut command, "agena", &mut buffer);
    String::from_utf8(buffer)
        .map_err(|err| AppError::Internal(format!("completion output was not utf-8: {err}")))
}

fn render_serialized<T>(format: ConfigOutputFormat, value: &T) -> Result<String, AppError>
where
    T: Serialize,
{
    match format {
        ConfigOutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
    }
}

fn format_apply_output(execution: &ApplyPatchExecution) -> String {
    let mut output = format!("applied patch: {}", execution.operation_id);
    for file in &execution.files {
        output.push_str(&format!("\n- {:?} {}", file.kind, file.path));
    }
    output
}

fn format_debug_session_output(output: &DebugSessionOutput) -> String {
    let mut rendered = format!(
        "session {}: {}\nstatus: {:?}\nmessages: {}",
        output.session.id,
        output.session.title,
        output.session.status,
        output.messages.len()
    );
    for message in &output.messages {
        rendered.push_str(&format!(
            "\n\n[{} #{} {}]\n{}",
            message.role, message.id, message.state, message.text
        ));
    }
    rendered
}

fn format_plugin_logs_output(output: &PluginLogsOutput) -> String {
    if output.logs.is_empty() {
        return format!("plugin {} has no retained logs", output.plugin_id);
    }
    output
        .logs
        .iter()
        .map(|log| {
            let timestamp = DateTime::<Utc>::from_timestamp_millis(log.timestamp_ms)
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| log.timestamp_ms.to_string());
            let mut line = format!(
                "[{}] #{} {} {} {}",
                timestamp, log.seq, log.level, log.source, log.message
            );
            if !log.fields.is_null() {
                line.push(' ');
                line.push_str(&log.fields.to_string());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_plugin_validate_output(
    format: ConfigOutputFormat,
    output: &PluginValidateOutput,
) -> Result<String, AppError> {
    render_serialized(format, output)
}

fn validate_plugin_target(path: &Path, strict: bool) -> Result<PluginValidateOutput, AppError> {
    let path = resolve_plugin_validate_path(path)?;
    let raw = fs::read_to_string(&path)?;
    let value: serde_json::Value = serde_json::from_str(&raw)?;
    let mut output = PluginValidateOutput {
        path: path.display().to_string(),
        target_kind: "unknown".to_string(),
        ok: false,
        manifest_hash: None,
        errors: Vec::new(),
        warnings: Vec::new(),
    };
    let base_dir = path.parent().unwrap_or_else(|| Path::new("."));

    if looks_like_plugin_manifest(&value) {
        output.target_kind = "manifest".to_string();
        validate_plugin_manifest_value("$", &value, &mut output);
    } else if value.get("package").is_some() {
        output.target_kind = "configured_plugin".to_string();
        validate_configured_plugin_value("$", &value, base_dir, &BTreeMap::new(), &mut output);
    } else if let Some(plugin_list) = value.pointer("/plugins/list").and_then(|v| v.as_object()) {
        output.target_kind = "agena_config".to_string();
        let trusted_keys = value
            .pointer("/plugins/host/trusted_keys")
            .cloned()
            .and_then(|v| serde_json::from_value::<BTreeMap<String, String>>(v).ok())
            .unwrap_or_default();
        if plugin_list.is_empty() {
            push_warning(
                &mut output,
                "config.plugins.empty",
                "plugins.list is empty",
                Some("$.plugins.list"),
            );
        }
        for (plugin_id, plugin_value) in plugin_list {
            validate_configured_plugin_value(
                &format!("$.plugins.list.{plugin_id}"),
                plugin_value,
                base_dir,
                &trusted_keys,
                &mut output,
            );
        }
    } else {
        push_error(
            &mut output,
            "target.unsupported",
            "expected a plugin manifest, configured plugin object, or agena config with plugins.list",
            Some("$"),
        );
    }

    if strict && !output.warnings.is_empty() {
        for warning in output.warnings.clone() {
            output.errors.push(PluginValidationMessage {
                code: format!("strict.{}", warning.code),
                message: format!("warning treated as error: {}", warning.message),
                path: warning.path,
            });
        }
    }
    output.ok = output.errors.is_empty();
    Ok(output)
}

fn resolve_plugin_validate_path(path: &Path) -> Result<PathBuf, AppError> {
    if path.is_file() {
        return Ok(path.to_path_buf());
    }
    if path.is_dir() {
        for candidate in [
            path.join(".agena-plugin").join("plugin.json"),
            path.join("plugin.json"),
            path.join("manifest.json"),
        ] {
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err(AppError::Config(format!(
        "plugin validate target not found: {}",
        path.display()
    )))
}

fn looks_like_plugin_manifest(value: &serde_json::Value) -> bool {
    value.get("schema_version").is_some()
        || value.get("tools").is_some()
        || value.get("transports").is_some()
        || (value.get("name").is_some() && value.get("version").is_some())
}

fn validate_plugin_manifest_value(
    path: &str,
    value: &serde_json::Value,
    output: &mut PluginValidateOutput,
) {
    check_object_keys(
        value,
        path,
        &[
            "schema_version",
            "name",
            "version",
            "description",
            "summary",
            "help",
            "tool_description_mode",
            "ui_display_mode",
            "authors",
            "transports",
            "hooks",
            "tools",
            "commands",
            "plugin_capabilities",
            "ui",
            "config_schema",
            "config_schema_i18n",
        ],
        "manifest.unknown_field",
        output,
    );
    warn_marketplace_fields(value, path, output);
    validate_raw_hook_array(value.get("hooks"), &format!("{path}.hooks"), output);

    let manifest: crate::plugin::PluginManifest = match serde_json::from_value(value.clone()) {
        Ok(manifest) => manifest,
        Err(err) => {
            push_error(
                output,
                "manifest.schema",
                format!("manifest does not match plugin manifest schema: {err}"),
                Some(path),
            );
            return;
        }
    };

    output.manifest_hash = serde_json::to_vec(&manifest)
        .ok()
        .map(|bytes| blake3::hash(&bytes).to_hex().to_string());

    if manifest.schema_version != 1 {
        push_warning(
            output,
            "manifest.schema_version",
            format!(
                "schema_version {} is not the current schema_version 1",
                manifest.schema_version
            ),
            Some(format!("{path}.schema_version")),
        );
    }
    if manifest.name.trim().is_empty() {
        push_error(
            output,
            "manifest.name.empty",
            "manifest name must not be empty",
            Some(format!("{path}.name")),
        );
    }
    if manifest.name.contains('/') {
        push_error(
            output,
            "manifest.name.invalid",
            "manifest name must not contain `/`; exposed tool names use plugin__tool",
            Some(format!("{path}.name")),
        );
    }
    if manifest.transports.is_empty() {
        push_warning(
            output,
            "manifest.transports.empty",
            "manifest declares no transport kind",
            Some(format!("{path}.transports")),
        );
    }

    if let Some(tools) = value.get("tools").and_then(|v| v.as_array()) {
        for (idx, tool_value) in tools.iter().enumerate() {
            validate_tool_manifest_value(
                &manifest.name,
                &manifest.tools.get(idx),
                tool_value,
                &format!("{path}.tools[{idx}]"),
                output,
            );
        }
    }
    validate_tool_name_collisions(&manifest, path, output);
    validate_manifest_ui_actions(&manifest, path, output);

    if let Some(schema) = manifest.config_schema.as_ref() {
        validate_schema_defaults(&format!("{path}.config_schema"), schema, output);
    }
    for (locale, schema) in &manifest.config_schema_i18n {
        validate_schema_defaults(
            &format!("{path}.config_schema_i18n.{locale}"),
            schema,
            output,
        );
    }

    if !manifest.tools.is_empty()
        && !manifest.hooks.intersects(
            crate::plugin::HookSubscription::TOOL_INVOKE
                | crate::plugin::HookSubscription::TOOL_INVOKE_STREAM,
        )
    {
        push_warning(
            output,
            "manifest.hooks.tool_invoke_missing",
            "manifest declares tools but does not subscribe to tool.invoke or tool.invoke.stream",
            Some(format!("{path}.hooks")),
        );
    }
}

fn validate_tool_manifest_value(
    plugin_name: &str,
    parsed_tool: &Option<&crate::plugin::PluginToolDecl>,
    value: &serde_json::Value,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    check_object_keys(
        value,
        path,
        &[
            "name",
            "aliases",
            "description",
            "before_help",
            "after_help",
            "summary",
            "help",
            "examples",
            "description_mode",
            "ui_display_mode",
            "input_schema",
            "input_paths",
            "input_networks",
            "path_access",
            "network_access",
            "tags",
            "concurrency_safe",
            "strict",
            "streaming",
            "result_policy",
            "host_capabilities",
        ],
        "tool.unknown_field",
        output,
    );
    if let Some(policy) = value.get("result_policy") {
        check_object_keys(
            policy,
            &format!("{path}.result_policy"),
            &[
                "max_model_chars",
                "preview_lines",
                "persist_large_output",
                "ui_render_kind",
            ],
            "tool.result_policy.unknown_field",
            output,
        );
    }
    if let Some(tool) = parsed_tool.as_ref() {
        validate_tool_segment(
            plugin_name,
            tool.name.as_str(),
            &format!("{path}.name"),
            output,
        );
        for (idx, alias) in tool.aliases.iter().enumerate() {
            validate_tool_segment(
                plugin_name,
                alias.as_str(),
                &format!("{path}.aliases[{idx}]"),
                output,
            );
        }
        for (idx, spec) in tool.path_access.iter().enumerate() {
            validate_no_parent_path(
                spec.path.as_str(),
                &format!("{path}.path_access[{idx}].path"),
                output,
            );
        }
    }
}

fn validate_tool_segment(
    plugin_name: &str,
    tool_name: &str,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    if tool_name.trim().is_empty() {
        push_error(
            output,
            "tool.name.empty",
            "tool name must not be empty",
            Some(path),
        );
    }
    if tool_name.contains('/') {
        push_error(
            output,
            "tool.name.invalid",
            "tool name must not contain `/`; exposed tool names use plugin__tool",
            Some(path),
        );
    }
    let exposed = safe_exposed_tool_name(plugin_name, tool_name);
    if exposed == "tool__tool" {
        push_warning(
            output,
            "tool.name.normalized_empty",
            "tool name normalizes to a generic exposed name",
            Some(path),
        );
    }
}

fn validate_tool_name_collisions(
    manifest: &crate::plugin::PluginManifest,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    let mut seen: BTreeMap<String, String> = BTreeMap::new();
    for (idx, tool) in manifest.tools.iter().enumerate() {
        for (label, raw_name) in std::iter::once(("name".to_string(), tool.name.as_str())).chain(
            tool.aliases
                .iter()
                .enumerate()
                .map(|(alias_idx, alias)| (format!("aliases[{alias_idx}]"), alias.as_str())),
        ) {
            let exposed = safe_exposed_tool_name(manifest.name.as_str(), raw_name);
            let location = format!("{path}.tools[{idx}].{label}");
            if let Some(existing) = seen.insert(exposed.clone(), location.clone()) {
                push_error(
                    output,
                    "tool.name.collision",
                    format!(
                        "`{raw_name}` normalizes to exposed name `{exposed}`, colliding with {existing}"
                    ),
                    Some(location),
                );
            }
        }
    }
}

fn validate_manifest_ui_actions(
    manifest: &crate::plugin::PluginManifest,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    let known_tools = manifest
        .tools
        .iter()
        .flat_map(|tool| {
            std::iter::once(tool.name.as_str()).chain(tool.aliases.iter().map(String::as_str))
        })
        .collect::<HashSet<_>>();
    for (idx, command) in manifest.commands.iter().enumerate() {
        validate_ui_action_tool(
            &command.action,
            &known_tools,
            &format!("{path}.commands[{idx}].action"),
            output,
        );
    }
    for (idx, control) in manifest.ui.studio.controls.iter().enumerate() {
        validate_ui_action_tool(
            &control.action,
            &known_tools,
            &format!("{path}.ui.studio.controls[{idx}].action"),
            output,
        );
    }
}

fn validate_ui_action_tool(
    action: &crate::plugin::PluginUiAction,
    known_tools: &HashSet<&str>,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    if let crate::plugin::PluginUiAction::InvokeTool { tool, .. } = action {
        if tool.contains('/') {
            push_error(
                output,
                "ui.action.tool.invalid",
                "UI action tool must use the local tool name or exposed plugin__tool name, not plugin/tool",
                Some(format!("{path}.tool")),
            );
        }
        if !known_tools.contains(tool.as_str()) && !tool.contains("__") {
            push_warning(
                output,
                "ui.action.tool.unknown",
                format!("UI action references unknown local tool `{tool}`"),
                Some(format!("{path}.tool")),
            );
        }
    }
}

fn validate_configured_plugin_value(
    path: &str,
    value: &serde_json::Value,
    base_dir: &Path,
    trusted_keys: &BTreeMap<String, String>,
    output: &mut PluginValidateOutput,
) {
    let configured: crate::plugin::ConfiguredPlugin = match serde_json::from_value(value.clone()) {
        Ok(configured) => configured,
        Err(err) => {
            push_error(
                output,
                "config_plugin.schema",
                format!("configured plugin does not match schema: {err}"),
                Some(path),
            );
            return;
        }
    };

    match &configured.package {
        crate::plugin::PluginPackage::Static {} => {}
        crate::plugin::PluginPackage::Cdylib {
            path: package_path,
            sha256,
            signature,
        } => {
            let resolved = resolve_config_path(base_dir, package_path);
            validate_no_parent_path(
                package_path.to_string_lossy().as_ref(),
                &format!("{path}.package.path"),
                output,
            );
            validate_existing_file(&resolved, &format!("{path}.package.path"), output);
            validate_sha256_if_present(
                &resolved,
                sha256.as_deref(),
                &format!("{path}.package.sha256"),
                output,
            );
            validate_signature_if_present(
                &resolved,
                signature.as_ref(),
                trusted_keys,
                &format!("{path}.package.signature"),
                output,
            );
        }
        crate::plugin::PluginPackage::Stdio {
            command,
            cwd,
            sha256,
            ..
        } => {
            if let Some(cwd) = cwd {
                validate_no_parent_path(
                    cwd.to_string_lossy().as_ref(),
                    &format!("{path}.package.cwd"),
                    output,
                );
                let resolved_cwd = resolve_config_path(base_dir, cwd);
                if !resolved_cwd.is_dir() {
                    push_error(
                        output,
                        "transport.cwd.missing",
                        format!(
                            "stdio cwd does not exist or is not a directory: {}",
                            resolved_cwd.display()
                        ),
                        Some(format!("{path}.package.cwd")),
                    );
                }
            }
            let resolved_command = resolve_command_path(command, cwd.as_deref(), base_dir);
            match resolved_command {
                Some(command_path) => {
                    validate_sha256_if_present(
                        &command_path,
                        sha256.as_deref(),
                        &format!("{path}.package.sha256"),
                        output,
                    );
                }
                None => push_error(
                    output,
                    "transport.command.not_found",
                    format!("stdio command is not executable or not found on PATH: {command}"),
                    Some(format!("{path}.package.command")),
                ),
            }
        }
        crate::plugin::PluginPackage::Http { url, .. } => {
            if !matches!(url.scheme(), "http" | "https") {
                push_error(
                    output,
                    "transport.http.scheme",
                    format!("unsupported http plugin URL scheme `{}`", url.scheme()),
                    Some(format!("{path}.package.url")),
                );
            }
        }
        crate::plugin::PluginPackage::Wasm {
            path: wasm_path,
            sha256,
        } => {
            let resolved = resolve_config_path(base_dir, wasm_path);
            validate_no_parent_path(
                wasm_path.to_string_lossy().as_ref(),
                &format!("{path}.package.path"),
                output,
            );
            validate_existing_file(&resolved, &format!("{path}.package.path"), output);
            validate_sha256_if_present(
                &resolved,
                sha256.as_deref(),
                &format!("{path}.package.sha256"),
                output,
            );
        }
    }
}

fn validate_raw_hook_array(
    hooks: Option<&serde_json::Value>,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    let Some(hooks) = hooks else {
        return;
    };
    let Some(items) = hooks.as_array() else {
        push_error(
            output,
            "hooks.schema",
            "hooks must be an array of hook names",
            Some(path),
        );
        return;
    };
    for (idx, item) in items.iter().enumerate() {
        let item_path = format!("{path}[{idx}]");
        let Some(name) = item.as_str() else {
            push_error(
                output,
                "hooks.schema",
                "hook subscription must be a string",
                Some(item_path),
            );
            continue;
        };
        if crate::plugin::HookSubscription::for_name(name).is_none() {
            push_error(
                output,
                "hooks.unknown",
                format!("unknown hook subscription `{name}`"),
                Some(item_path),
            );
        }
    }
}

fn validate_schema_defaults(
    path: &str,
    schema: &serde_json::Value,
    output: &mut PluginValidateOutput,
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    if let Some(default_value) = object.get("default") {
        validate_default_matches_schema(path, schema, default_value, output);
    }
    for key in ["properties", "$defs", "definitions"] {
        if let Some(children) = object.get(key).and_then(|v| v.as_object()) {
            for (name, child) in children {
                validate_schema_defaults(&format!("{path}.{key}.{name}"), child, output);
            }
        }
    }
    if let Some(items) = object.get("items") {
        validate_schema_defaults(&format!("{path}.items"), items, output);
    }
    for key in ["oneOf", "anyOf", "allOf"] {
        if let Some(items) = object.get(key).and_then(|v| v.as_array()) {
            for (idx, child) in items.iter().enumerate() {
                validate_schema_defaults(&format!("{path}.{key}[{idx}]"), child, output);
            }
        }
    }
}

fn validate_default_matches_schema(
    path: &str,
    schema: &serde_json::Value,
    default_value: &serde_json::Value,
    output: &mut PluginValidateOutput,
) {
    if let Some(enum_values) = schema.get("enum").and_then(|v| v.as_array())
        && !enum_values.iter().any(|value| value == default_value)
    {
        push_error(
            output,
            "config_schema.default.enum",
            "default value is not present in enum",
            Some(format!("{path}.default")),
        );
    }
    if let Some(type_names) = schema_type_names(schema)
        && !type_names
            .iter()
            .any(|type_name| json_value_matches_type(default_value, type_name))
    {
        push_error(
            output,
            "config_schema.default.type",
            format!(
                "default value does not match schema type {}",
                type_names.join("|")
            ),
            Some(format!("{path}.default")),
        );
    }
    if let Some(required) = schema.get("required").and_then(|v| v.as_array())
        && let Some(default_object) = default_value.as_object()
    {
        for required_name in required.iter().filter_map(|v| v.as_str()) {
            if !default_object.contains_key(required_name) {
                push_error(
                    output,
                    "config_schema.default.required",
                    format!("default object is missing required field `{required_name}`"),
                    Some(format!("{path}.default")),
                );
            }
        }
    }
    if let (Some(properties), Some(default_object)) = (
        schema.get("properties").and_then(|v| v.as_object()),
        default_value.as_object(),
    ) {
        for (name, property_schema) in properties {
            if let Some(child_default) = default_object.get(name) {
                validate_default_matches_schema(
                    &format!("{path}.properties.{name}"),
                    property_schema,
                    child_default,
                    output,
                );
            }
        }
    }
}

fn schema_type_names(schema: &serde_json::Value) -> Option<Vec<String>> {
    let value = schema.get("type")?;
    if let Some(name) = value.as_str() {
        return Some(vec![name.to_string()]);
    }
    Some(
        value
            .as_array()?
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
    )
}

fn json_value_matches_type(value: &serde_json::Value, type_name: &str) -> bool {
    match type_name {
        "null" => value.is_null(),
        "boolean" => value.is_boolean(),
        "string" => value.is_string(),
        "number" => value.is_number(),
        "integer" => value.as_i64().is_some() || value.as_u64().is_some(),
        "object" => value.is_object(),
        "array" => value.is_array(),
        _ => true,
    }
}

fn validate_no_parent_path(path_value: &str, path: &str, output: &mut PluginValidateOutput) {
    if Path::new(path_value)
        .components()
        .any(|component| matches!(component, std::path::Component::ParentDir))
    {
        push_error(
            output,
            "path.traversal",
            format!("path must not contain `..`: {path_value}"),
            Some(path),
        );
    }
}

fn validate_existing_file(path: &Path, json_path: &str, output: &mut PluginValidateOutput) {
    if !path.is_file() {
        push_error(
            output,
            "transport.file.not_found",
            format!("transport file does not exist: {}", path.display()),
            Some(json_path),
        );
    }
}

fn validate_sha256_if_present(
    path: &Path,
    expected: Option<&str>,
    json_path: &str,
    output: &mut PluginValidateOutput,
) {
    let Some(expected) = expected else {
        push_warning(
            output,
            "transport.sha256.missing",
            "transport artifact has no sha256 pin",
            Some(json_path),
        );
        return;
    };
    if !path.is_file() {
        return;
    }
    match sha256_hex_file(path) {
        Ok(actual) if actual.eq_ignore_ascii_case(expected) => {}
        Ok(actual) => push_error(
            output,
            "transport.sha256.mismatch",
            format!("sha256 mismatch: expected {expected}, got {actual}"),
            Some(json_path),
        ),
        Err(err) => push_error(
            output,
            "transport.sha256.read_failed",
            format!("failed to compute sha256: {err}"),
            Some(json_path),
        ),
    }
}

fn validate_signature_if_present(
    path: &Path,
    signature: Option<&crate::plugin::PluginSignature>,
    trusted_keys: &BTreeMap<String, String>,
    json_path: &str,
    output: &mut PluginValidateOutput,
) {
    let Some(signature) = signature else {
        push_warning(
            output,
            "transport.signature.missing",
            "transport artifact has no signature",
            Some(json_path),
        );
        return;
    };
    if !trusted_keys.contains_key(&signature.key_id) {
        push_error(
            output,
            "transport.signature.untrusted_key",
            format!(
                "signature key `{}` is not configured as trusted",
                signature.key_id
            ),
            Some(format!("{json_path}.key_id")),
        );
        return;
    }
    #[cfg(feature = "plugin-signing")]
    {
        if path.is_file()
            && let Err(err) = crate::plugin::verify_signature(path, signature, trusted_keys)
        {
            push_error(output, "transport.signature.invalid", err, Some(json_path));
        }
    }
    #[cfg(not(feature = "plugin-signing"))]
    {
        let _ = path;
        push_warning(
            output,
            "transport.signature.not_verified",
            "signature is present but this binary was built without plugin-signing",
            Some(json_path),
        );
    }
}

fn sha256_hex_file(path: &Path) -> Result<String, std::io::Error> {
    use sha2::{Digest, Sha256};
    let bytes = fs::read(path)?;
    let digest = Sha256::digest(&bytes);
    Ok(hex::encode(digest))
}

fn resolve_config_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

fn resolve_command_path(command: &str, cwd: Option<&Path>, base_dir: &Path) -> Option<PathBuf> {
    let command_path = Path::new(command);
    if command_path.components().count() > 1 || command.contains(std::path::MAIN_SEPARATOR) {
        let base = cwd
            .map(|cwd| resolve_config_path(base_dir, cwd))
            .unwrap_or_else(|| base_dir.to_path_buf());
        let resolved = if command_path.is_absolute() {
            command_path.to_path_buf()
        } else {
            base.join(command_path)
        };
        return is_executable_file(&resolved).then_some(resolved);
    }
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|dir| dir.join(command))
            .find(|candidate| is_executable_file(candidate))
    })
}

fn is_executable_file(path: &Path) -> bool {
    if !path.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

fn check_object_keys(
    value: &serde_json::Value,
    path: &str,
    allowed: &[&str],
    code: &str,
    output: &mut PluginValidateOutput,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            push_error(
                output,
                code,
                format!("unknown field `{key}`"),
                Some(format!("{path}.{key}")),
            );
        }
    }
}

fn warn_marketplace_fields(
    value: &serde_json::Value,
    path: &str,
    output: &mut PluginValidateOutput,
) {
    const MARKETPLACE_FIELDS: &[&str] = &[
        "id",
        "homepage",
        "repository",
        "license",
        "category",
        "versions",
        "artifact",
        "archive",
        "sha256",
        "signature",
        "registry",
        "install",
        "marketplace",
    ];
    let Some(object) = value.as_object() else {
        return;
    };
    for field in MARKETPLACE_FIELDS {
        if object.contains_key(*field) {
            push_warning(
                output,
                "manifest.marketplace_field",
                format!("marketplace field `{field}` does not belong in plugin manifest"),
                Some(format!("{path}.{field}")),
            );
        }
    }
}

fn safe_exposed_tool_name(plugin_name: &str, tool_name: &str) -> String {
    format!(
        "{}__{}",
        crate::plugin::registry::exposed_tool_name_segment(plugin_name),
        crate::plugin::registry::exposed_tool_name_segment(tool_name),
    )
}

fn push_error(
    output: &mut PluginValidateOutput,
    code: impl Into<String>,
    message: impl Into<String>,
    path: Option<impl Into<String>>,
) {
    output.errors.push(PluginValidationMessage {
        code: code.into(),
        message: message.into(),
        path: path.map(Into::into),
    });
}

fn push_warning(
    output: &mut PluginValidateOutput,
    code: impl Into<String>,
    message: impl Into<String>,
    path: Option<impl Into<String>>,
) {
    output.warnings.push(PluginValidationMessage {
        code: code.into(),
        message: message.into(),
        path: path.map(Into::into),
    });
}

fn review_prompt(base: &str) -> String {
    format!(
        "Review the current workspace changes against `{base}`. Focus on correctness, regressions, security issues, and missing tests. Report findings first, then concise remediation guidance."
    )
}

fn auth_summary(provider_id: String, auth: AuthData) -> AuthSummary {
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

fn normalize_login_provider(provider_id: &str) -> String {
    provider_id.trim_end_matches('/').to_owned()
}

fn browser_login_redirect_uri(port: u16) -> String {
    format!("http://localhost:{port}/auth/callback")
}

fn resolve_login_oauth_target(
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

fn resolve_login_device_target(
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

fn prompt_browser_login(authorize_url: &str) -> Result<(), AppError> {
    println!("open this URL to continue: {authorize_url}");
    io::stdout().flush()?;
    Ok(())
}

fn prompt_device_login(start: &DeviceCodeStart) -> Result<(), AppError> {
    println!("open this URL: {}", start.verification_url);
    println!("enter code: {}", start.user_code);
    io::stdout().flush()?;
    Ok(())
}

fn copilot_deployment_from_domain(enterprise_domain: Option<&str>) -> CopilotDeployment {
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

async fn complete_browser_callback_login<F, Fut>(
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

async fn complete_polled_login<T, F, Fut, P>(
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

async fn list_all_session_summaries(
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

fn filter_session_summaries_by_view(
    sessions: Vec<SessionSummary>,
    view: SessionListView,
    anchor_session_id: Option<i64>,
) -> Result<Vec<SessionSummary>, AppError> {
    match view {
        SessionListView::Roots => {
            let mut roots = sessions
                .into_iter()
                .filter(|session| session.parent_id.is_none())
                .collect::<Vec<_>>();
            roots.sort_by(session_summary_sort_recent);
            Ok(roots)
        }
        SessionListView::All => render_session_summary_tree(sessions, None),
        SessionListView::Subtree => {
            let anchor_session_id = anchor_session_id.ok_or_else(|| {
                AppError::Config("subtree view requires --anchor-session-id <id>".to_owned())
            })?;
            render_session_summary_tree(sessions, Some(anchor_session_id))
        }
    }
}

fn render_session_summary_tree(
    sessions: Vec<SessionSummary>,
    anchor_session_id: Option<i64>,
) -> Result<Vec<SessionSummary>, AppError> {
    let by_id = sessions
        .into_iter()
        .map(|session| (session.id, session))
        .collect::<BTreeMap<_, _>>();
    let mut children = BTreeMap::<Option<i64>, Vec<i64>>::new();
    for session in by_id.values() {
        let parent_id = session
            .parent_id
            .filter(|parent_id| by_id.contains_key(parent_id));
        children.entry(parent_id).or_default().push(session.id);
    }
    for child_ids in children.values_mut() {
        child_ids.sort_by(|left, right| session_summary_sort_recent(&by_id[left], &by_id[right]));
    }

    let root_ids = match anchor_session_id {
        Some(anchor_id) => vec![resolve_session_summary_root(anchor_id, &by_id)?],
        None => children.get(&None).cloned().unwrap_or_default(),
    };
    let kept_ids = match anchor_session_id {
        Some(root_id) => collect_session_summary_subtree_ids(
            resolve_session_summary_root(root_id, &by_id)?,
            &children,
        ),
        None => by_id.keys().copied().collect::<HashSet<_>>(),
    };
    let mut out = Vec::new();
    for root_id in root_ids {
        append_session_summary_subtree(root_id, &children, &by_id, &kept_ids, &mut out);
    }
    Ok(out)
}

fn resolve_session_summary_root(
    session_id: i64,
    by_id: &BTreeMap<i64, SessionSummary>,
) -> Result<i64, AppError> {
    let mut current = session_id;
    let mut seen = HashSet::new();
    loop {
        let session = by_id.get(&current).ok_or_else(|| {
            AppError::Config(format!("session not found for subtree view: {session_id}"))
        })?;
        let Some(parent_id) = session.parent_id else {
            return Ok(current);
        };
        if !seen.insert(current) {
            return Err(AppError::Internal(format!(
                "cycle detected while resolving session subtree root for {session_id}"
            )));
        }
        if !by_id.contains_key(&parent_id) {
            return Ok(current);
        }
        current = parent_id;
    }
}

fn collect_session_summary_subtree_ids(
    root_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
) -> HashSet<i64> {
    let mut kept = HashSet::new();
    let mut stack = vec![root_id];
    while let Some(session_id) = stack.pop() {
        if !kept.insert(session_id) {
            continue;
        }
        if let Some(child_ids) = children.get(&Some(session_id)) {
            stack.extend(child_ids.iter().copied());
        }
    }
    kept
}

fn append_session_summary_subtree(
    session_id: i64,
    children: &BTreeMap<Option<i64>, Vec<i64>>,
    by_id: &BTreeMap<i64, SessionSummary>,
    kept_ids: &HashSet<i64>,
    out: &mut Vec<SessionSummary>,
) {
    if !kept_ids.contains(&session_id) {
        return;
    }
    if let Some(session) = by_id.get(&session_id) {
        out.push(session.clone());
    }
    if let Some(child_ids) = children.get(&Some(session_id)) {
        for child_id in child_ids {
            append_session_summary_subtree(*child_id, children, by_id, kept_ids, out);
        }
    }
}

fn session_summary_sort_recent(
    left: &SessionSummary,
    right: &SessionSummary,
) -> std::cmp::Ordering {
    right
        .updated_at
        .cmp(&left.updated_at)
        .then_with(|| right.id.cmp(&left.id))
}

fn paginate_session_summaries(
    sessions: Vec<SessionSummary>,
    offset: u64,
    limit: u64,
) -> Vec<SessionSummary> {
    sessions
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect()
}

async fn selected_session_id(
    manager: &crate::session::SessionManager,
    session_id: Option<i64>,
    last: bool,
) -> Result<i64, AppError> {
    if session_id.is_some() && last {
        return Err(AppError::Config(
            "pass either a session id or --last, not both".to_owned(),
        ));
    }
    if let Some(session_id) = session_id {
        return Ok(session_id);
    }
    let sessions = manager
        .list_session_summaries(SessionListRequest {
            offset: 0,
            limit: Some(1),
            include_subagents: false,
        })
        .await?;
    sessions
        .first()
        .map(|session| session.id)
        .ok_or_else(|| AppError::Config("no sessions found".to_owned()))
}

fn usage_stats_query_from_args(args: &UsageArgs) -> Result<UsageStatsQuery, AppError> {
    let has_custom_range = args.from.is_some() || args.to.is_some();
    let mut query = UsageStatsQuery::for_period(args.period.into_usage_period(), Utc::now());
    if has_custom_range {
        query = UsageStatsQuery::custom(
            args.from
                .as_deref()
                .map(|value| parse_usage_datetime(value, false))
                .transpose()?,
            args.to
                .as_deref()
                .map(|value| parse_usage_datetime(value, true))
                .transpose()?
                .or_else(|| Some(Utc::now())),
        );
    }

    if let (Some(from), Some(to)) = (query.from.as_ref(), query.to.as_ref())
        && from > to
    {
        return Err(AppError::Config(
            "--from must be earlier than or equal to --to".to_string(),
        ));
    }

    Ok(query)
}

fn parse_usage_datetime(raw: &str, end_of_day: bool) -> Result<DateTime<Utc>, AppError> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(AppError::Config("usage date cannot be empty".to_string()));
    }

    if let Ok(parsed) = DateTime::parse_from_rfc3339(trimmed) {
        return Ok(parsed.with_timezone(&Utc));
    }

    if let Ok(date) = chrono::NaiveDate::parse_from_str(trimmed, "%Y-%m-%d") {
        let datetime = if end_of_day {
            date.and_hms_milli_opt(23, 59, 59, 999)
        } else {
            date.and_hms_milli_opt(0, 0, 0, 0)
        }
        .expect("valid date boundary");
        return Ok(datetime.and_utc());
    }

    Err(AppError::Config(format!(
        "invalid usage date `{raw}`; expected YYYY-MM-DD or RFC3339"
    )))
}

async fn latest_event_seq(
    manager: &crate::session::SessionManager,
    session_id: i64,
) -> Result<Option<i64>, AppError> {
    Ok(manager
        .list_session_events(session_id)
        .await?
        .last()
        .map(|event| event.meta.seq_global))
}

fn session_detail(session: &Session, latest_event_seq: Option<i64>) -> SessionDetail {
    SessionDetail {
        id: session.id,
        parent_id: session.parent_id,
        workspace_id: session.workspace_id,
        title: session.title.clone(),
        version: session.version,
        created_at: session.created_at,
        updated_at: session.updated_at,
        message_count: session.messages.len(),
        status: session.runtime.run.status,
        latest_event_seq,
    }
}

fn resolve_continue_options(
    runtime: &AgenaRuntime,
    session: &Session,
    args: &ContinueArgs,
) -> Result<SessionRunOptions, AppError> {
    let snapshot = runtime.current_snapshot();
    let model = if let Some(model) = args.model.as_deref() {
        snapshot.resolve_model_target(model, None)?
    } else if let Some(model) = session
        .runtime
        .effective_model_ref()
        .map_err(|err| AppError::Config(format!("invalid persisted model reference: {err}")))?
    {
        model
    } else {
        default_model(runtime)?
    };

    let mut options = SessionRunOptions::new(model);
    options.temperature = args.temperature;
    options.max_output_tokens = args.max_output_tokens;
    options.agent_profile = args
        .agent
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(options)
}

fn resolve_run_options(
    runtime: &AgenaRuntime,
    model: Option<&str>,
    agent_profile: Option<&str>,
    temperature: Option<f32>,
    max_output_tokens: Option<u32>,
) -> Result<SessionRunOptions, AppError> {
    let model = if let Some(model) = model {
        runtime
            .current_snapshot()
            .resolve_model_target(model, None)?
    } else {
        default_model(runtime)?
    };

    let mut options = SessionRunOptions::new(model);
    options.temperature = temperature;
    options.max_output_tokens = max_output_tokens;
    options.agent_profile = agent_profile
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned);
    Ok(options)
}

fn default_model(runtime: &AgenaRuntime) -> Result<ModelRef, AppError> {
    runtime
        .current_snapshot()
        .resolve_default_model()?
        .ok_or_else(|| AppError::Config("no providers configured".to_owned()))
}

fn last_assistant_text(session: &Session) -> Option<String> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| message.role == Role::Assistant)
        .map(|message| message.as_text_lossy())
}

fn title_from_prompt(prompt: &str) -> String {
    let title = prompt.trim().replace('\n', " ");
    let mut chars = title.chars();
    let truncated = chars.by_ref().take(80).collect::<String>();
    if truncated.is_empty() {
        "exec".to_owned()
    } else if chars.next().is_some() {
        format!("{truncated}…")
    } else {
        truncated
    }
}

fn command_available(command: &str) -> bool {
    Command::new(command)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_success<const N: usize>(workspace_root: &Path, args: [&str; N]) -> bool {
    Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()
        .is_ok_and(|output| output.status.success())
}

fn git_output<const N: usize>(workspace_root: &Path, args: [&str; N]) -> Result<String, AppError> {
    let output = Command::new("git")
        .args(args)
        .current_dir(workspace_root)
        .output()?;
    if !output.status.success() {
        return Err(AppError::Config(format!(
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn collect_git_preflight(workspace_root: &Path) -> Result<GitPreflight, AppError> {
    let git_available = command_available("git");
    let gh_available = command_available("gh");
    if !git_available {
        return Ok(GitPreflight {
            git_available,
            repo: false,
            gh_available,
            branch: None,
            upstream: None,
            ahead: None,
            behind: None,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            changed_files: 0,
            clean: true,
        });
    }

    let repo = git_success(workspace_root, ["rev-parse", "--is-inside-work-tree"]);
    if !repo {
        return Ok(GitPreflight {
            git_available,
            repo,
            gh_available,
            branch: None,
            upstream: None,
            ahead: None,
            behind: None,
            staged_files: 0,
            unstaged_files: 0,
            untracked_files: 0,
            changed_files: 0,
            clean: true,
        });
    }

    let branch = non_empty_string(git_output(workspace_root, ["branch", "--show-current"])?);
    let upstream = git_output(
        workspace_root,
        [
            "rev-parse",
            "--abbrev-ref",
            "--symbolic-full-name",
            "@{upstream}",
        ],
    )
    .ok()
    .and_then(non_empty_string);
    let (ahead, behind) = parse_ahead_behind(
        upstream
            .as_ref()
            .and_then(|_| {
                git_output(
                    workspace_root,
                    ["rev-list", "--left-right", "--count", "HEAD...@{upstream}"],
                )
                .ok()
            })
            .as_deref(),
    );
    let status = git_output(workspace_root, ["status", "--porcelain"])?;
    let (staged_files, unstaged_files, untracked_files, changed_files) =
        summarize_git_status(status.as_str());

    Ok(GitPreflight {
        git_available,
        repo,
        gh_available,
        branch,
        upstream,
        ahead,
        behind,
        staged_files,
        unstaged_files,
        untracked_files,
        changed_files,
        clean: changed_files == 0,
    })
}

fn non_empty_string(value: String) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn parse_ahead_behind(value: Option<&str>) -> (Option<u64>, Option<u64>) {
    let Some(value) = value else {
        return (None, None);
    };
    let mut parts = value.split_whitespace();
    let behind = parts.next().and_then(|part| part.parse::<u64>().ok());
    let ahead = parts.next().and_then(|part| part.parse::<u64>().ok());
    (ahead, behind)
}

fn summarize_git_status(status: &str) -> (u64, u64, u64, u64) {
    let mut staged = 0_u64;
    let mut unstaged = 0_u64;
    let mut untracked = 0_u64;
    let mut changed = 0_u64;

    for line in status.lines().filter(|line| !line.is_empty()) {
        changed += 1;
        let bytes = line.as_bytes();
        let x = bytes.first().copied().unwrap_or(b' ');
        let y = bytes.get(1).copied().unwrap_or(b' ');
        if x == b'?' && y == b'?' {
            untracked += 1;
            continue;
        }
        if x != b' ' {
            staged += 1;
        }
        if y != b' ' {
            unstaged += 1;
        }
    }

    (staged, unstaged, untracked, changed)
}

fn session_storage_error() -> AppError {
    AppError::Config("session storage is unavailable; configure a database URL or path".to_owned())
}

async fn poll_until<T, F, Fut>(
    timeout: Duration,
    interval: Duration,
    mut poll: F,
) -> Result<Option<T>, AppError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, AppError>>,
{
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(value) = poll().await? {
            return Ok(Some(value));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn plugin_validate_reports_tool_name_normalization_collisions() {
        let tempdir = tempfile::tempdir().expect("tempdir should build");
        let manifest_path = tempdir.path().join("plugin.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": 1,
                "name": "fixture.plugin",
                "version": "0.1.0",
                "transports": ["static"],
                "hooks": ["tool.invoke"],
                "tools": [
                    {
                        "name": "plan.get",
                        "input_schema": { "type": "object" },
                        "concurrency_safe": false
                    },
                    {
                        "name": "plan_get",
                        "input_schema": { "type": "object" },
                        "concurrency_safe": false
                    }
                ]
            }))
            .expect("manifest json should serialize"),
        )
        .expect("manifest should write");

        let output = validate_plugin_target(&manifest_path, false).expect("validation should run");
        assert!(!output.ok);
        assert!(
            output
                .errors
                .iter()
                .any(|message| message.code == "tool.name.collision")
        );
    }

    #[test]
    fn plugin_validate_strict_promotes_warnings_to_errors() {
        let tempdir = tempfile::tempdir().expect("tempdir should build");
        let manifest_path = tempdir.path().join("plugin.json");
        fs::write(
            &manifest_path,
            serde_json::to_vec_pretty(&json!({
                "schema_version": 1,
                "name": "fixture.plugin",
                "version": "0.1.0",
                "tools": [
                    {
                        "name": "echo",
                        "input_schema": { "type": "object" },
                        "concurrency_safe": false
                    }
                ]
            }))
            .expect("manifest json should serialize"),
        )
        .expect("manifest should write");

        let output = validate_plugin_target(&manifest_path, true).expect("validation should run");
        assert!(!output.ok);
        assert!(
            output
                .errors
                .iter()
                .any(|message| message.code.starts_with("strict."))
        );
    }
}
