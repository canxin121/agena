//! CLI argument schema, command definitions, and dispatch.

use agena_domain::{
    Model, ModelCapabilities, ModelRef, PermissionReply, Role, SessionCostSummary,
    SessionListRequest, SessionSummary, UsageStatsQuery, WorkflowState,
};
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

use agena_domain::{
    PermissionAction, PermissionMode, PermissionReplyKind, PermissionScope, StructuredObject,
};
use agena_mcp_client::protocol::{CallToolParams, CallToolResult, ToolDescriptor};
use agena_mcp_server::{McpServerBackend, McpServerError};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::Serialize;

use agena_application::AuthLoginKind;
use agena_domain::ToolInvocation;
use agena_domain::{ModelMetadata, UsagePeriod};
use agena_runtime::{
    OutputFormat, SessionCreateRequest, SessionExecutionRequest, SessionForkRequest,
    SessionPermissionReplyRequest, SessionRunOptions, SessionUserMessageRequest,
};
use agena_tool::ApplyPatchExecution;

mod cli_auth_helpers;
mod cli_permissions;
mod cli_render;
mod cli_run;
mod cli_runtime;
mod cli_runtime_helpers;
mod cli_session_helpers;
mod cli_validation;

/// CLI-owned presentation/process error boundary.
///
/// This deliberately does not carry Runtime configuration or session
/// implementation types. Runtime bootstrap failures are mapped to text at
/// this CLI presentation boundary.
#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("configuration error: {0}")]
    Config(String),
    #[error("provider error: {0}")]
    Provider(String),
    #[error("serde json error: {0}")]
    SerdeJson(#[from] serde_json::Error),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("internal error: {0}")]
    Internal(String),
}

pub type AppError = CliError;

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
                  `mcp-server`, or `rpc-server` for non-TUI workflows.\n\n\
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
                  Run as a tools-only MCP server over stdio:\n    \
                  agena mcp-server --workspace ."
)]
pub struct AgenaCli {
    #[arg(short = 'c', long = "set", global = true)]
    /// Raw `--set` expressions. Their schema-specific parsing belongs to the
    /// Runtime bootstrap composition boundary, not the CLI's public launch intent.
    pub overrides: Vec<String>,
    #[arg(long, env = "AGENA_DATABASE_URL", global = true)]
    pub database_url: Option<String>,
    #[arg(long, env = "AGENA_DATABASE_PATH", global = true)]
    pub database_path: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<AgenaCommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum AgenaCommand {
    RpcServer(RpcServerArgs),
    Server(ServerArgs),
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
    /// Print the deterministic compiled capability manifest for documentation and CI.
    Inspect(InspectArgs),
    Login(LoginArgs),
    Memory(MemoryCommand),
    Logout(LogoutArgs),
    Mcp(McpCommand),
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

/// Fully resolved process launch intent.
///
/// Parsing happens once in [`AgenaCli::into_launch_mode`]. The application
/// binary owns process-wide setup and dispatches one of these typed requests;
/// presentation command handlers never signal dispatch by manufacturing an
/// error.
#[derive(Debug, Clone)]
pub enum LaunchMode {
    Tui(TuiLaunchRequest),
    Command(AgenaCli),
    RpcServer(RpcServerRequest),
    Server(ServerLaunchRequest),
}

#[derive(Debug, Clone)]
pub struct TuiLaunchRequest {
    pub config_override_expressions: Vec<String>,
    pub args: TuiArgs,
}

#[derive(Debug, Clone)]
pub struct RpcServerRequest {
    pub config_override_expressions: Vec<String>,
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
    pub args: RpcServerArgs,
}

#[derive(Debug, Clone)]
pub struct ServerLaunchRequest {
    pub config_override_expressions: Vec<String>,
    pub database_url: Option<String>,
    pub database_path: Option<PathBuf>,
    pub args: ServerArgs,
}

impl AgenaCli {
    /// Converts parsed arguments into one unambiguous top-level launch mode.
    pub fn into_launch_mode(self) -> LaunchMode {
        let config_override_expressions = self.overrides.clone();
        match self.command.clone() {
            None => LaunchMode::Tui(TuiLaunchRequest {
                config_override_expressions,
                args: TuiArgs {
                    database_url: self.database_url,
                    database_path: self.database_path,
                    ..TuiArgs::default()
                },
            }),
            Some(AgenaCommand::Tui(mut args)) => {
                args.database_url = args.database_url.or(self.database_url);
                args.database_path = args.database_path.or(self.database_path);
                LaunchMode::Tui(TuiLaunchRequest {
                    config_override_expressions,
                    args,
                })
            }
            Some(AgenaCommand::RpcServer(args)) => LaunchMode::RpcServer(RpcServerRequest {
                config_override_expressions,
                database_url: self.database_url.clone(),
                database_path: self.database_path.clone(),
                args,
            }),
            Some(AgenaCommand::Server(args)) => LaunchMode::Server(ServerLaunchRequest {
                config_override_expressions,
                database_url: self.database_url.clone(),
                database_path: self.database_path.clone(),
                args: ServerArgs {
                    overrides: self.overrides,
                    database_url: self.database_url,
                    database_path: self.database_path,
                    ..args
                },
            }),
            Some(_) => LaunchMode::Command(self),
        }
    }
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CostArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value_t = UsagePeriodArg::Week, value_enum)]
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CommitArgs {
    pub message: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    Auto,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SnapshotArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct GitArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct InspectArgs {
    /// Machine-readable JSON. The inspect manifest is JSON-only; this flag is
    /// accepted explicitly so CI can use `agena inspect --json`.
    #[arg(long)]
    pub json: bool,
    /// Emit the reviewable identity-only snapshot used by the CI drift check.
    #[arg(long, requires = "json")]
    pub identity_snapshot: bool,
    /// Emit the generated Markdown tool reference committed at
    /// `crates/agena-bundled-plugins/generated/tools-reference.md` and embedded into `cargo doc`.
    #[arg(long)]
    pub tools_reference: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCommand {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginInspectArgs {
    pub plugin_id: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginLogsArgs {
    pub plugin_id: String,
    #[arg(long, default_value_t = 50)]
    pub limit: usize,
    #[arg(long)]
    pub after_seq: Option<u64>,
    #[arg(long, default_value_t = PluginLogOutputFormat::Text, value_enum)]
    pub format: PluginLogOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginValidateArgs {
    /// Manifest file, plugin directory, configured plugin JSON, or agena config.
    pub path: PathBuf,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
pub struct RpcServerArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value_t = RpcServerTransport::Stdio, value_enum)]
    pub transport: RpcServerTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
pub enum RpcServerTransport {
    Stdio,
}

#[derive(Debug, Clone, Args)]
pub struct ServerArgs {
    #[arg(skip)]
    pub overrides: Vec<String>,
    #[arg(skip)]
    pub database_url: Option<String>,
    #[arg(skip)]
    pub database_path: Option<PathBuf>,
    #[arg(long, env = "AGENA_SERVER_HOST", default_value = "127.0.0.1")]
    pub host: String,
    #[arg(short, long, env = "AGENA_SERVER_PORT", default_value_t = 3210)]
    pub port: u16,
    #[arg(long, env = "AGENA_SERVER_UI_PASSWORD")]
    pub ui_password: Option<String>,
    #[arg(long = "workspace", env = "AGENA_WORKSPACE_ROOT", value_name = "PATH")]
    pub workspace_root: Option<PathBuf>,
    #[arg(long, env = "AGENA_SERVER_UI_DIR", value_name = "PATH")]
    pub ui_dir: Option<String>,
    #[arg(
        long,
        env = "AGENA_SERVER_CORS_ORIGINS",
        value_delimiter = ',',
        value_name = "ORIGIN"
    )]
    pub cors_origin: Vec<String>,
    #[arg(long, env = "AGENA_SERVER_CORS_ALLOW_ALL", default_value_t = false)]
    pub cors_allow_all: bool,
    #[arg(
        long,
        env = "AGENA_SERVER_UI_COOKIE_SAMESITE",
        value_enum,
        default_value = "auto"
    )]
    pub ui_cookie_samesite: UiCookieSameSite,
}

#[derive(Clone, Debug, ValueEnum)]
#[value(rename_all = "kebab_case")]
pub enum UiCookieSameSite {
    Auto,
    Strict,
    Lax,
    None,
}

#[derive(Debug, Clone, Args)]
#[command(
    about = "Serve Agena runtime tools as a stdio MCP server",
    long_about = "Starts a stdio MCP server that exposes the current Agena runtime Tool API. \
                  This endpoint deliberately advertises tools only: MCP resources and prompts are not exposed. \
                  Configure an MCP client with command `agena`, arguments `mcp-server --workspace <path>`.",
    after_help = "EXAMPLE:\n  agena mcp-server --workspace ."
)]
pub struct McpServerArgs {
    #[arg(long = "workspace", env = "AGENA_WORKSPACE_ROOT", value_name = "PATH")]
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct McpCommand {
    #[command(subcommand)]
    pub command: Option<McpSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
pub enum McpSubcommand {
    /// Show live, redacted MCP connection health for every configured server.
    Status(McpStatusArgs),
    /// List every configured MCP server with its current connection state.
    List(McpStatusArgs),
    /// Show one configured MCP server with its current connection state.
    Get(McpGetArgs),
    /// Add one stdio or streamable-HTTP MCP server to Agena configuration.
    Add(McpAddArgs),
    /// Remove a configured MCP server from Agena configuration.
    Remove(McpRemoveArgs),
    /// Enable the static MCP bridge plugin in Agena configuration.
    Enable(McpPluginToggleArgs),
    /// Disable the static MCP bridge plugin without deleting server records.
    Disable(McpPluginToggleArgs),
    /// Reconnect one configured MCP server and refresh its tool cache.
    Reconnect(McpReconnectArgs),
    /// Store a bearer credential or complete browser OAuth without adding a
    /// secret to agena.json.
    Login(McpLoginArgs),
    /// Delete a stored bearer credential. This is idempotent.
    Logout(McpLogoutArgs),
}

#[derive(Debug, Clone, Args)]
pub struct McpStatusArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct McpGetArgs {
    pub server: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum McpConfigLayerArg {
    #[default]
    Global,
    Workspace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum McpHttpAuthArg {
    #[default]
    None,
    BearerFromStore,
    BearerFromEnv,
    #[value(name = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, Args)]
pub struct McpAddArgs {
    pub server: String,
    /// HTTP endpoint. Supply exactly one of --url and --command.
    #[arg(long)]
    pub url: Option<String>,
    /// Stdio executable. Supply exactly one of --url and --command.
    #[arg(long)]
    pub command: Option<String>,
    /// Repeatable argument passed to a stdio server process.
    #[arg(long = "arg")]
    pub args: Vec<String>,
    /// Repeatable KEY=VALUE environment pair for a stdio server process.
    #[arg(long = "env")]
    pub env: Vec<String>,
    #[arg(long)]
    pub cwd: Option<PathBuf>,
    /// Repeatable HTTP header KEY=VALUE. Authorization headers are rejected;
    /// use bearer-from-store or bearer-from-env instead.
    #[arg(long = "header")]
    pub headers: Vec<String>,
    #[arg(long, value_enum, default_value_t = McpHttpAuthArg::None)]
    pub auth: McpHttpAuthArg,
    /// Repeatable OAuth scope requested when --auth oauth is selected.
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    /// Repeatable tool-name glob to allow from this MCP server. If specified,
    /// only matching tools can be discovered or called.
    #[arg(long = "include-tool")]
    pub include_tools: Vec<String>,
    /// Repeatable tool-name glob to block from this MCP server. Exclusion wins
    /// over inclusion and is enforced at invocation time.
    #[arg(long = "exclude-tool")]
    pub exclude_tools: Vec<String>,
    /// Environment variable holding the bearer token when --auth
    /// bearer-from-env is selected.
    #[arg(long = "auth-env")]
    pub auth_env: Option<String>,
    #[arg(long, value_enum, default_value_t = McpConfigLayerArg::Global)]
    pub layer: McpConfigLayerArg,
    /// Replace an existing record with the same server name.
    #[arg(long, default_value_t = false)]
    pub force: bool,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    /// Write configuration but leave reload to a later explicit action.
    #[arg(long, default_value_t = false)]
    pub no_reload: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct McpRemoveArgs {
    pub server: String,
    #[arg(long, value_enum, default_value_t = McpConfigLayerArg::Global)]
    pub layer: McpConfigLayerArg,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    #[arg(long, default_value_t = false)]
    pub no_reload: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct McpPluginToggleArgs {
    #[arg(long, value_enum, default_value_t = McpConfigLayerArg::Global)]
    pub layer: McpConfigLayerArg,
    #[arg(long, default_value_t = false)]
    pub dry_run: bool,
    #[arg(long, default_value_t = false)]
    pub no_reload: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct McpReconnectArgs {
    pub server: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum McpCredentialStoreArg {
    #[default]
    Keyring,
    File,
}

#[derive(Debug, Clone, Args)]
pub struct McpLoginArgs {
    pub server: String,
    /// Bearer token for non-interactive automation. Prefer --token-stdin so
    /// shell history and process listings do not retain a secret.
    #[arg(long)]
    pub token: Option<String>,
    /// Read the bearer token from standard input until EOF.
    #[arg(long, default_value_t = false)]
    pub token_stdin: bool,
    /// Run browser authorization-code login using S256 PKCE. This is mutually
    /// exclusive with --token and --token-stdin.
    #[arg(long, default_value_t = false)]
    pub browser: bool,
    /// Streamable HTTP MCP endpoint for browser OAuth login. It is deliberately
    /// explicit so login never guesses a server target from an unrelated layer.
    #[arg(long)]
    pub url: Option<String>,
    /// Repeatable OAuth scope. Omit to let the protected resource select scopes.
    #[arg(long = "scope")]
    pub scopes: Vec<String>,
    /// Loopback callback port used by browser OAuth login.
    #[arg(long, default_value_t = 1455)]
    pub port: u16,
    /// Credential backend. Defaults to the system keyring; file is an
    /// explicit compatibility option for configurations that select it.
    #[arg(long, value_enum, default_value_t = McpCredentialStoreArg::Keyring)]
    pub store: McpCredentialStoreArg,
}

#[derive(Debug, Clone, Args)]
pub struct McpLogoutArgs {
    pub server: String,
    /// Credential backend from which to remove the token.
    #[arg(long, value_enum, default_value_t = McpCredentialStoreArg::Keyring)]
    pub store: McpCredentialStoreArg,
    /// Remove the OAuth client/token record, leaving a manual bearer record
    /// untouched. OAuth credentials are always keyring backed.
    #[arg(long, default_value_t = false)]
    pub oauth: bool,
    /// Revoke the OAuth credential at the authorization server before
    /// deleting its local keyring record.  Requires --oauth and --url.  The
    /// operation is available only when discovered metadata advertises the
    /// optional RFC 7009 revocation endpoint.
    #[arg(long, default_value_t = false)]
    pub revoke: bool,
    /// Streamable HTTP MCP resource endpoint used to discover OAuth metadata
    /// for --revoke.  It is intentionally explicit so credential deletion
    /// never guesses a remote authorization authority.
    #[arg(long)]
    pub url: Option<String>,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct AuthListArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value_t = SessionListView::All, value_enum)]
    pub view: SessionListView,
    #[arg(long)]
    pub anchor_session_id: Option<i64>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryListArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    pub model: Option<String>,
    #[arg(long)]
    pub temperature: Option<f32>,
    #[arg(long)]
    pub max_output_tokens: Option<u32>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    #[arg(long, env = "AGENA_LOG_FILE", conflicts_with = "log_stderr")]
    pub log_file: Option<PathBuf>,
    #[arg(long, env = "AGENA_LOG_STDERR")]
    pub log_stderr: bool,
}

#[derive(Debug, Clone, Args)]
pub struct ReviewArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value = "main")]
    pub base: String,
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
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigResolveArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderListArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderModelsArgs {
    pub provider_id: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCapabilitiesArgs {
    pub target: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
    state: agena_domain::ExecutionStatus,
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
    summary: SessionCostSummary,
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
    staged_files: u64,
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
    status: agena_domain::WorkflowState,
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
    models: Vec<Model>,
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
struct PluginStatusOutput {
    statuses: Vec<agena_plugin_host::status::PluginStatus>,
}

#[derive(Debug, Serialize)]
struct PluginInspectOutput {
    plugin: agena_plugin_host::PluginInspect,
}

#[derive(Debug, Serialize)]
struct PluginLogsOutput {
    plugin_id: String,
    logs: Vec<agena_plugin_host::PluginLogRecord>,
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
    // Retain Runtime lifecycle ownership for the complete stdio-server
    // lifetime; service trait objects alone are not the bootstrap boundary.
    runtime: agena_runtime::RuntimeBootstrapResult,
    tools: Arc<dyn agena_runtime::RuntimeToolExecutionService>,
    event_publisher: Option<Arc<dyn agena_runtime::RuntimeEventPublishService>>,
    next_call_id: Arc<AtomicI64>,
}

#[async_trait]
impl McpServerBackend for AgenaMcpBackend {
    async fn list_tools(&self) -> Result<Vec<ToolDescriptor>, McpServerError> {
        Ok(self
            .tools
            .available_runtime_tools()
            .into_iter()
            .map(|tool| ToolDescriptor {
                name: tool.name,
                title: None,
                aliases: Vec::new(),
                description: tool.summary,
                before_help: tool.before_help,
                after_help: tool.after_help,
                input_schema: Some(tool.input_schema),
                output_schema: None,
                annotations: None,
                execution: None,
                icons: Vec::new(),
                meta: None,
            })
            .collect())
    }

    async fn call_tool(&self, params: CallToolParams) -> Result<CallToolResult, McpServerError> {
        let name = params.name;
        let input = structured_tool_input(params.arguments)?;
        let invocation = mcp_tool_invocation(name.as_str(), input)?;
        let call_id = self.next_call_id.fetch_add(1, Ordering::SeqCst);
        let result = self.tools.execute_runtime_tool(&invocation, call_id).await;
        match result {
            Ok(outcome) => {
                let summary = outcome.into_summary();
                self.audit_tool_call(name.as_str(), call_id, false, None)
                    .await;
                let text = if summary.output_text.is_empty() {
                    serde_json::to_string_pretty(&summary.payload)
                        .unwrap_or_else(|_| "<empty output>".to_owned())
                } else {
                    summary.output_text
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
}

impl AgenaMcpBackend {
    fn shutdown(&self) {
        self.runtime.shutdown();
    }

    async fn audit_tool_call(
        &self,
        tool_name: &str,
        call_id: i64,
        is_error: bool,
        error: Option<&str>,
    ) {
        let Some(publisher) = self.event_publisher.as_ref() else {
            return;
        };
        let payload = serde_json::json!({
            "tool_name": tool_name,
            "call_id": call_id,
            "is_error": is_error,
            "error": error,
        });
        let _ = publisher
            .publish_event(agena_runtime::RuntimeEventPublishRequest::PluginEvent {
                plugin_id: "agena.mcp_server"
                    .parse::<agena_plugin_host::PluginKey>()
                    .expect("static plugin key"),
                kind_label: "mcp_tool_call".to_owned(),
                payload,
            })
            .await;
    }
}

#[cfg(test)]
mod parser_contract_tests {
    use super::{AgenaCli, AgenaCommand, LaunchMode, McpCredentialStoreArg, McpSubcommand};
    use clap::Parser;

    #[test]
    fn parser_routes_bare_invocation_to_tui_mode() {
        let cli = AgenaCli::try_parse_from(["agena"]).expect("parse bare CLI invocation");
        assert!(matches!(cli.into_launch_mode(), LaunchMode::Tui(_)));
    }

    #[test]
    fn parser_keeps_subcommands_in_command_mode() {
        let cli = AgenaCli::try_parse_from(["agena", "sessions", "list"])
            .expect("parse sessions command");
        assert!(matches!(cli.into_launch_mode(), LaunchMode::Command(_)));
    }

    #[test]
    fn inspect_json_is_a_runtime_free_machine_readable_command() {
        let cli = AgenaCli::try_parse_from(["agena", "inspect", "--json"])
            .expect("parse inspect command");
        assert!(
            matches!(&cli.command, Some(AgenaCommand::Inspect(args)) if args.json && !args.identity_snapshot)
        );
        assert!(matches!(cli.into_launch_mode(), LaunchMode::Command(_)));

        let snapshot =
            AgenaCli::try_parse_from(["agena", "inspect", "--json", "--identity-snapshot"])
                .expect("parse inspect identity snapshot command");
        assert!(matches!(
            &snapshot.command,
            Some(AgenaCommand::Inspect(args)) if args.json && args.identity_snapshot
        ));

        let reference = AgenaCli::try_parse_from(["agena", "inspect", "--tools-reference"])
            .expect("parse inspect tools reference command");
        assert!(matches!(
            &reference.command,
            Some(AgenaCommand::Inspect(args)) if args.tools_reference
        ));
    }

    #[test]
    fn mcp_server_starts_directly_from_the_cli_and_mcp_credentials_stay_available() {
        let server =
            AgenaCli::try_parse_from(["agena", "mcp-server", "--workspace", "/workspace/project"])
                .expect("parse tools-only MCP server command");
        assert!(matches!(
            &server.command,
            Some(AgenaCommand::McpServer(args)) if args.workspace.as_deref() == Some(std::path::Path::new("/workspace/project"))
        ));
        assert!(matches!(server.into_launch_mode(), LaunchMode::Command(_)));

        let login = AgenaCli::try_parse_from(["agena", "mcp", "login", "example", "--token-stdin"])
            .expect("parse MCP credential command");
        let Some(AgenaCommand::Mcp(command)) = login.command else {
            panic!("expected mcp command");
        };
        let Some(McpSubcommand::Login(args)) = command.command else {
            panic!("expected mcp login subcommand");
        };
        assert_eq!(args.server, "example");
        assert!(args.token_stdin);
        assert_eq!(args.store, McpCredentialStoreArg::Keyring);
    }

    #[test]
    fn mcp_oauth_and_reconnect_subcommands_keep_their_parser_contracts() {
        let oauth = AgenaCli::try_parse_from([
            "agena",
            "mcp",
            "login",
            "example",
            "--browser",
            "--url",
            "https://mcp.example.test",
            "--scope",
            "mcp:read",
        ])
        .expect("parse MCP browser OAuth login");
        let Some(AgenaCommand::Mcp(command)) = oauth.command else {
            panic!("expected mcp command");
        };
        let Some(McpSubcommand::Login(args)) = command.command else {
            panic!("expected mcp login subcommand");
        };
        assert!(args.browser);
        assert_eq!(args.scopes, ["mcp:read"]);

        let add = AgenaCli::try_parse_from([
            "agena",
            "mcp",
            "add",
            "example",
            "--url",
            "https://mcp.example.test",
            "--auth",
            "oauth",
            "--scope",
            "mcp:read",
        ])
        .expect("parse MCP OAuth configuration");
        let Some(AgenaCommand::Mcp(command)) = add.command else {
            panic!("expected mcp command");
        };
        let Some(McpSubcommand::Add(args)) = command.command else {
            panic!("expected mcp add subcommand");
        };
        assert_eq!(args.auth, super::McpHttpAuthArg::OAuth);
        assert_eq!(args.scopes, ["mcp:read"]);

        let reconnect = AgenaCli::try_parse_from(["agena", "mcp", "reconnect", "example"])
            .expect("parse MCP reconnect");
        let Some(AgenaCommand::Mcp(command)) = reconnect.command else {
            panic!("expected mcp command");
        };
        let Some(McpSubcommand::Reconnect(args)) = command.command else {
            panic!("expected mcp reconnect subcommand");
        };
        assert_eq!(args.server, "example");

        let logout = AgenaCli::try_parse_from([
            "agena",
            "mcp",
            "logout",
            "example",
            "--oauth",
            "--revoke",
            "--url",
            "https://mcp.example.test",
        ])
        .expect("parse MCP OAuth logout");
        let Some(AgenaCommand::Mcp(command)) = logout.command else {
            panic!("expected mcp command");
        };
        let Some(McpSubcommand::Logout(args)) = command.command else {
            panic!("expected mcp logout subcommand");
        };
        assert!(args.oauth);
        assert!(args.revoke);
        assert_eq!(args.url.as_deref(), Some("https://mcp.example.test"));
    }
}
