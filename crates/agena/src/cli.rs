use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{self},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc,
        atomic::{AtomicI64, Ordering},
    },
    time::{Duration, Instant},
};

use agena_mcp_client::protocol::{
    CallToolParams, CallToolResult, ContentBlock, GetPromptParams, GetPromptResult, PromptArgument,
    PromptDescriptor, PromptMessage, ReadResourceParams, ReadResourceResult, ResourceContents,
    ResourceDescriptor, ToolDescriptor,
};
use agena_mcp_server::{McpServerBackend, McpServerError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use sea_orm::ActiveValue::Set;
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
        Session, SessionCreateRequest, SessionExecutionRequest, SessionForkRequest,
        SessionListRequest, SessionManager, SessionRunOptions, SessionSummary,
        SessionUserMessageRequest, UsagePeriod, UsageStatsQuery, WorkflowState,
    },
    storage::StorageConfig,
    tool::{ApplyPatchExecution, ToolExecutor, ToolPayloadInput},
    tracing as tracing_config,
};

mod cli_auth_helpers;
mod cli_permissions;
mod cli_render;
mod cli_run;
mod cli_runtime;
mod cli_runtime_helpers;
mod cli_session_helpers;
mod cli_validation;

use self::cli_auth_helpers::*;
use self::cli_permissions::*;
use self::cli_runtime_helpers::*;
use self::cli_session_helpers::*;
use self::cli_validation::*;

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
    Yesterday,
    Week,
    TwoWeeks,
    ThirtyDays,
    NinetyDays,
    Month,
    Year,
    All,
}

impl UsagePeriodArg {
    fn into_usage_period(self) -> UsagePeriod {
        match self {
            Self::Today => UsagePeriod::Today,
            Self::Yesterday => UsagePeriod::Yesterday,
            Self::Week => UsagePeriod::Last7Days,
            Self::TwoWeeks => UsagePeriod::Last14Days,
            Self::ThirtyDays => UsagePeriod::Last30Days,
            Self::NinetyDays => UsagePeriod::Last90Days,
            Self::Month => UsagePeriod::MonthToDate,
            Self::Year => UsagePeriod::YearToDate,
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
    /// Only include these provider ids (repeat or use comma-separated values).
    #[arg(long, value_delimiter = ',')]
    pub provider: Vec<String>,
    /// Only include these model ids (repeat or use comma-separated values).
    #[arg(long, value_delimiter = ',')]
    pub model: Vec<String>,
    /// Only include these session ids (repeat or use comma-separated values).
    #[arg(long, value_delimiter = ',')]
    pub session: Vec<i64>,
    /// Include subagent sessions in the report.
    #[arg(long, default_value_t = true, action = clap::ArgAction::Set)]
    pub include_subagents: bool,
    /// Fixed UTC offset in minutes for calendar windows and daily buckets.
    #[arg(long, default_value_t = 0, value_parser = -1439..=1439)]
    pub timezone_offset_minutes: i32,
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
    status: crate::session::WorkflowState,
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

type PluginValidationMessages = (Vec<PluginValidationMessage>, Vec<PluginValidationMessage>);

#[derive(Clone)]
struct AgenaMcpBackend {
    executor: ToolExecutor,
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
                let summary = tool.summary_text().map(ToString::to_string);
                let before_help = tool.before_help_text().map(ToString::to_string);
                let after_help = tool.after_help_text().map(ToString::to_string);
                let input_schema = tool.input_schema();
                ToolDescriptor {
                    name: crate::tool::tool_value_name(tool.model_name().as_str()),
                    aliases: Vec::new(),
                    description: summary,
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
        let invocation = ToolInvocation::new("agena.skills.list", StructuredObject::default());
        let execution = self
            .executor
            .execute_invocation_detailed(&invocation, -1, -1)
            .map_err(|err| McpServerError::Backend(err.to_string()))?;
        let payload = execution.output.to_json_payload().ok_or_else(|| {
            McpServerError::Backend("skills.list did not return a JSON payload".to_string())
        })?;
        skill_prompt_descriptors(payload)
    }

    async fn get_prompt(&self, params: GetPromptParams) -> Result<GetPromptResult, McpServerError> {
        let prompt_name = params.name;
        let invocation = ToolInvocation::new(
            "agena.skills.run",
            skill_prompt_invocation_input(prompt_name.as_str(), params.arguments)?,
        );
        let execution = self
            .executor
            .execute_invocation_detailed(&invocation, -1, -1)
            .map_err(|err| McpServerError::Backend(err.to_string()))?;
        Ok(GetPromptResult {
            description: Some(format!("Render skill or command prompt `{prompt_name}`.")),
            messages: vec![PromptMessage {
                role: "user".to_owned(),
                content: ContentBlock::Text {
                    text: execution.view.output_text,
                },
            }],
        })
    }
}

fn skill_prompt_descriptors(
    payload: serde_json::Value,
) -> Result<Vec<PromptDescriptor>, McpServerError> {
    let entries = payload
        .get("tools")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| {
            McpServerError::Backend("skills.list payload is missing `tools`".to_string())
        })?;
    let mut prompts = entries
        .iter()
        .map(|entry| {
            let name = entry
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .ok_or_else(|| {
                    McpServerError::Backend(
                        "skills.list payload contains an item without a name".to_string(),
                    )
                })?;
            let description = entry
                .get("summary")
                .and_then(serde_json::Value::as_str)
                .map(ToOwned::to_owned);
            Ok(PromptDescriptor {
                name: name.to_string(),
                description,
                arguments: vec![PromptArgument {
                    name: "args".to_string(),
                    description: Some(
                        "Optional arguments inserted into the skill prompt.".to_string(),
                    ),
                    required: false,
                }],
            })
        })
        .collect::<Result<Vec<_>, McpServerError>>()?;
    prompts.sort_by(|left, right| left.name.cmp(&right.name));
    Ok(prompts)
}

fn skill_prompt_invocation_input(
    name: &str,
    arguments: Option<BTreeMap<String, String>>,
) -> Result<StructuredObject, McpServerError> {
    let args = match arguments {
        None => None,
        Some(arguments) if arguments.is_empty() => None,
        Some(mut arguments) => {
            let args = arguments.remove("args").ok_or_else(|| {
                McpServerError::InvalidParams(
                    "skill prompts accept only the optional `args` argument".to_string(),
                )
            })?;
            if !arguments.is_empty() {
                return Err(McpServerError::InvalidParams(
                    "skill prompts accept only the optional `args` argument".to_string(),
                ));
            }
            Some(args)
        }
    };
    StructuredObject::try_from(serde_json::json!({ "name": name, "args": args }))
        .map_err(McpServerError::InvalidParams)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod mcp_skill_prompt_tests {
    use super::{skill_prompt_descriptors, skill_prompt_invocation_input};
    use std::collections::BTreeMap;

    #[test]
    fn skills_are_exposed_as_mcp_prompts_with_an_optional_args_argument() {
        let prompts = skill_prompt_descriptors(serde_json::json!({
            "tools": [
                {"name": "review", "summary": "Review a change", "kind": "skill"},
                {"name": "commit", "summary": "Prepare a commit", "kind": "command"}
            ]
        }))
        .expect("valid skills list payload");

        assert_eq!(
            prompts
                .iter()
                .map(|prompt| prompt.name.as_str())
                .collect::<Vec<_>>(),
            vec!["commit", "review"]
        );
        assert_eq!(prompts[0].arguments.len(), 1);
        assert_eq!(prompts[0].arguments[0].name, "args");
        assert!(!prompts[0].arguments[0].required);
    }

    #[test]
    fn mcp_prompt_arguments_map_to_skills_run_input() {
        let input = skill_prompt_invocation_input(
            "review",
            Some(BTreeMap::from([(
                String::from("args"),
                String::from("focus on tests"),
            )])),
        )
        .expect("args is supported");

        assert_eq!(
            serde_json::Value::from(input),
            serde_json::json!({"name": "review", "args": "focus on tests"})
        );
    }

    #[test]
    fn mcp_prompt_rejects_unknown_arguments() {
        let error = skill_prompt_invocation_input(
            "review",
            Some(BTreeMap::from([(
                String::from("unexpected"),
                String::from("value"),
            )])),
        )
        .expect_err("unknown arguments must be rejected");

        assert!(error.to_string().contains("only the optional `args`"));
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
                    plugin_id: "agena.mcp_server"
                        .parse::<crate::plugin::PluginKey>()
                        .expect("static plugin key"),
                    kind_label: "mcp_tool_call".to_owned(),
                    payload,
                }),
            )
            .await;
    }
}
