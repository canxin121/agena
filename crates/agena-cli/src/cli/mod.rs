//! CLI argument schema, command definitions, and dispatch.

use agena_domain::{Role, SessionCostSummary, SessionSummary, UsageStatsQuery};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    path::{Path, PathBuf},
};

use chrono::{DateTime, Utc};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Deserialize, Serialize};

use agena_domain::UsagePeriod;
use agena_mcp_client::protocol::{CallToolParams, CallToolResult, ToolDescriptor};
use agena_mcp_server::{McpServerBackend, McpServerError};
use agena_runtime::OutputFormat;
use agena_tool::ApplyPatchExecution;
use async_trait::async_trait;

mod cli_auth_helpers;
mod cli_permissions;
mod cli_run;
mod cli_runtime_helpers;
mod cli_server;
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

impl CliError {
    /// Convert a typed configuration failure without discarding any nested
    /// `source()` diagnostics at the CLI process boundary.
    pub fn config_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Config(agena_failure::diagnostic::format_error_chain(error))
    }

    /// Convert a typed internal failure without truncating its source chain.
    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::Internal(agena_failure::diagnostic::format_error_chain(error))
    }

    pub fn internal_error_with_context(
        context: impl AsRef<str>,
        error: &(dyn std::error::Error + 'static),
    ) -> Self {
        Self::Internal(agena_failure::diagnostic::format_error_chain_with_context(
            context, error,
        ))
    }
}

#[derive(Debug, Clone, Args)]
/// Arguments for the resolved plugin architecture catalog.
pub struct PluginArchitectureArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
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
                  subcommands like `server`, `exec`, `sessions`, `plugin`, \
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
                  Expose server-owned tools over MCP stdio:\n    \
                  agena mcp-server --workspace .\n\n  \
                  Inspect the server:\n    \
                  agena diagnostics"
)]
/// Top-level CLI entry point.
pub struct AgenaCli {
    #[arg(short = 'c', long = "set", global = true)]
    /// Raw `--set` expressions. Their schema-specific parsing belongs to the
    /// Runtime bootstrap composition boundary, not the CLI's public launch intent.
    pub overrides: Vec<String>,
    /// Base URL of the long-lived server used by thin clients and
    /// session-critical one-shot commands.
    #[arg(long, env = "AGENA_SERVER_URL", global = true)]
    pub server: Option<String>,
    /// Ephemeral bearer token for a password-protected server.
    /// Prefer AGENA_SERVER_TOKEN so the secret is not exposed in process args.
    #[arg(
        long,
        env = "AGENA_SERVER_TOKEN",
        global = true,
        hide_env_values = true,
        conflicts_with = "server_password"
    )]
    pub server_token: Option<String>,
    /// Server password exchanged for an in-memory server bearer token.
    /// Prefer AGENA_SERVER_PASSWORD so the secret is not exposed in process args.
    #[arg(
        long,
        env = "AGENA_SERVER_PASSWORD",
        global = true,
        hide_env_values = true,
        conflicts_with = "server_token"
    )]
    pub server_password: Option<String>,
    /// Root directory for all local Agena instance data.
    #[arg(long, env = "AGENA_DATA_DIR", global = true, value_name = "DIR")]
    pub data_dir: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Option<AgenaCommand>,
}

#[derive(Debug, Clone, Subcommand)]
/// Top-level CLI commands.
pub enum AgenaCommand {
    RpcServer(RpcServerArgs),
    /// Run the long-lived Agena server.
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
/// Launch parameters for the TUI.
pub struct TuiLaunchRequest {
    pub config_override_expressions: Vec<String>,
    pub args: TuiArgs,
}

#[derive(Debug, Clone)]
/// Launch parameters for the JSON-RPC server.
pub struct RpcServerRequest {
    pub config_override_expressions: Vec<String>,
    pub data_dir: Option<PathBuf>,
    pub args: RpcServerArgs,
}

#[derive(Debug, Clone)]
/// Common server launch parameters.
pub struct ServerLaunchRequest {
    pub config_override_expressions: Vec<String>,
    pub data_dir: Option<PathBuf>,
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
                    server: self.server,
                    server_token: self.server_token,
                    server_password: self.server_password,
                    ..TuiArgs::default()
                },
            }),
            Some(AgenaCommand::Tui(mut args)) => {
                args.server = self.server;
                args.server_token = self.server_token;
                args.server_password = self.server_password;
                LaunchMode::Tui(TuiLaunchRequest {
                    config_override_expressions,
                    args,
                })
            }
            Some(AgenaCommand::RpcServer(mut args)) => {
                args.server = self.server.clone();
                args.server_token = self.server_token.clone();
                args.server_password = self.server_password.clone();
                LaunchMode::RpcServer(RpcServerRequest {
                    config_override_expressions,
                    data_dir: self.data_dir.clone(),
                    args,
                })
            }
            Some(AgenaCommand::Server(args)) => LaunchMode::Server(ServerLaunchRequest {
                config_override_expressions,
                data_dir: self.data_dir.clone(),
                args: ServerArgs {
                    overrides: self.overrides,
                    data_dir: self.data_dir,
                    ..args
                },
            }),
            Some(_) => LaunchMode::Command(self),
        }
    }
}

#[derive(Debug, Clone, Args)]
/// Provider authentication command.
pub struct AuthCommand {
    #[command(subcommand)]
    pub command: Option<AuthSubcommand>,
}

#[derive(Debug, Clone, Args)]
/// Configuration command.
pub struct ConfigCommand {
    #[command(subcommand)]
    pub command: Option<ConfigSubcommand>,
}

#[derive(Debug, Clone, Args)]
/// Debugging command.
pub struct DebugCommand {
    #[command(subcommand)]
    pub command: DebugSubcommand,
}

#[derive(Debug, Clone, Args)]
/// Arguments for diagnostics.
pub struct DiagnosticsArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for cost reporting.
pub struct CostArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Usage reporting period.
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
/// Arguments for usage reporting.
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
    #[arg(
        long,
        default_value_t = 0,
        value_parser = clap::value_parser!(i32).range(-1439..=1439)
    )]
    pub timezone_offset_minutes: i32,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the commit helper.
pub struct CommitArgs {
    pub message: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the pull-request helper.
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
/// Permissions management command.
pub struct PermissionsArgs {
    #[command(subcommand)]
    pub command: Option<PermissionsSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
/// Permissions subcommands.
pub enum PermissionsSubcommand {
    List(PermissionsListArgs),
    Create(PermissionsWriteArgs),
    Replace(PermissionsReplaceArgs),
    Revoke(PermissionsRevokeArgs),
    Reply(PermissionsReplyArgs),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Permission scope argument.
pub enum PermissionScopeArg {
    Session,
    Workspace,
    Global,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Permission mode argument.
pub enum PermissionModeArg {
    Allow,
    Auto,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Permission reply kind argument.
pub enum PermissionReplyKindArg {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
}

#[derive(Debug, Clone, Args)]
/// Arguments for listing permissions.
pub struct PermissionsListArgs {
    #[arg(long)]
    pub search: Option<String>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for writing a permission rule.
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
/// Arguments for replacing a permission rule.
pub struct PermissionsReplaceArgs {
    pub rule_id: i64,
    #[command(flatten)]
    pub rule: PermissionsWriteArgs,
}

#[derive(Debug, Clone, Args)]
/// Arguments for revoking a permission rule.
pub struct PermissionsRevokeArgs {
    pub rule_id: i64,
    #[arg(long)]
    pub reason: Option<String>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for replying to a permission request.
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
/// Git helper command.
pub struct GitArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the inspect command.
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
/// Provider management command.
pub struct ProviderCommand {
    #[command(subcommand)]
    pub command: Option<ProviderSubcommand>,
}

#[derive(Debug, Clone, Args)]
/// Memory management command.
pub struct MemoryCommand {
    #[command(subcommand)]
    pub command: Option<MemorySubcommand>,
}

#[derive(Debug, Clone, Args)]
/// Plugin management command.
pub struct PluginCommand {
    #[command(subcommand)]
    pub command: PluginSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
/// Plugin subcommands.
pub enum PluginSubcommand {
    /// Print runtime status for configured plugins, including failed loads.
    Status(PluginStatusArgs),
    /// Show detailed runtime and manifest information for one plugin.
    Inspect(PluginInspectArgs),
    /// Show the resolved plugin architecture graph and lifecycle inventory.
    Architecture(PluginArchitectureArgs),
    /// Show recent retained logs for one plugin.
    Logs(PluginLogsArgs),
    /// Validate a plugin manifest, configured plugin, or agena config plugin list.
    Validate(PluginValidateArgs),
    /// Scaffold a standalone plugin repository with CI and GitHub Release automation.
    Init(PluginInitArgs),
    /// Package one compiled plugin artifact and emit a release-manifest fragment.
    Package(PluginPackageArgs),
    /// Assemble platform release fragments into a publishable manifest.
    Release(PluginReleaseOperation),
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Template kind generated by `agena plugin init`.
pub enum PluginTemplateArg {
    Stdio,
    Cdylib,
}

#[derive(Debug, Clone, Args)]
/// Arguments for scaffolding a plugin repository.
pub struct PluginInitArgs {
    /// Destination directory for the new repository.
    pub path: PathBuf,
    /// Stable plugin id (`namespace.name`).
    #[arg(long)]
    pub id: String,
    /// Cargo package/binary name. Defaults to `agena-plugin-<name>`.
    #[arg(long)]
    pub crate_name: Option<String>,
    /// Human-readable plugin name.
    #[arg(long)]
    pub name: Option<String>,
    /// Marketplace and README description.
    #[arg(long, default_value = "An Agena plugin.")]
    pub description: String,
    /// Copyright holder used by the generated Apache-2.0 license.
    #[arg(long, default_value = "Agena Plugin Authors")]
    pub author: String,
    /// Canonical GitHub repository URL written into Cargo and release metadata.
    #[arg(long)]
    pub repository: Option<String>,
    /// Transport/template shape.
    #[arg(long, default_value_t = PluginTemplateArg::Stdio, value_enum)]
    pub kind: PluginTemplateArg,
    /// Overwrite template-owned files in a non-empty directory.
    #[arg(long, default_value_t = false)]
    pub force: bool,
}

#[derive(Debug, Clone, Args)]
/// Arguments for packaging a compiled plugin artifact.
pub struct PluginPackageArgs {
    /// Developer manifest generated by `agena plugin init`.
    #[arg(long, default_value = "agena-plugin.toml")]
    pub manifest: PathBuf,
    /// Compiled binary, cdylib, or wasm artifact.
    #[arg(long)]
    pub artifact: PathBuf,
    /// Rust target triple. Defaults to the current host target.
    #[arg(long)]
    pub target: Option<String>,
    /// Output directory for the archive and release fragment.
    #[arg(long, default_value = "dist")]
    pub output: PathBuf,
}

#[derive(Debug, Clone, Args)]
/// Plugin release-manifest operations.
pub struct PluginReleaseOperation {
    #[command(subcommand)]
    pub command: PluginReleaseSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum PluginReleaseSubcommand {
    /// Merge platform fragments into `agena-plugin-release.json`.
    Assemble(PluginReleaseAssembleArgs),
}

#[derive(Debug, Clone, Args)]
pub struct PluginReleaseAssembleArgs {
    /// Directory containing release fragments and packaged archives.
    pub fragments: PathBuf,
    /// Output directory for the final manifest.
    #[arg(long, default_value = "dist/release")]
    pub output: PathBuf,
    /// Immutable GitHub Release asset base URL.
    #[arg(long)]
    pub base_url: String,
    /// Expected plugin version, usually the Git tag without or with a leading `v`.
    #[arg(long)]
    pub expected_version: Option<String>,
    /// Canonical GitHub repository URL recorded as immutable release provenance.
    #[arg(long, requires = "github_tag", requires = "github_commit")]
    pub github_repository: Option<String>,
    /// GitHub tag used for this release.
    #[arg(long, requires = "github_repository", requires = "github_commit")]
    pub github_tag: Option<String>,
    /// Exact 40-character Git commit SHA used to build the release.
    #[arg(long, requires = "github_repository", requires = "github_tag")]
    pub github_commit: Option<String>,
    /// Optional GitHub Actions run URL for auditability.
    #[arg(long, requires = "github_repository")]
    pub github_workflow_run_url: Option<String>,
}

#[derive(Debug, Clone, Args)]
/// Arguments for plugin status.
pub struct PluginStatusArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for plugin inspection.
pub struct PluginInspectArgs {
    pub plugin_id: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for plugin logs.
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
/// Arguments for plugin validation.
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
/// Output format for plugin logs.
pub enum PluginLogOutputFormat {
    Text,
    Json,
}

#[derive(Debug, Clone, Args)]
/// Arguments for installing a plugin.
pub struct PluginInstallArgs {
    /// Plugin id, optionally with `@<version>`, or `owner/repo[@tag]` for GitHub Releases.
    pub spec: String,
    /// Marketplace URL, local path, or GitHub `owner/repo[@ref]` shorthand.
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
/// Arguments for uninstalling a plugin.
pub struct PluginUninstallArgs {
    pub plugin_id: String,
    /// Also uninstall any plugin that depends on this one.
    #[arg(long, default_value_t = false)]
    pub cascade: bool,
}

#[derive(Debug, Clone, Args)]
/// Arguments for syncing plugins.
pub struct PluginSyncArgs {
    /// Marketplace URL, local path, or GitHub `owner/repo[@ref]` shorthand.
    #[arg(long, default_value = "canxin121/agena-plugin-marketplace")]
    pub registry: String,
    #[arg(long, default_value = "default")]
    pub registry_id: String,
}

#[derive(Debug, Clone, Args)]
/// Arguments for searching plugins.
pub struct PluginSearchArgs {
    pub query: String,
    /// Marketplace URL, local path, or GitHub `owner/repo[@ref]` shorthand.
    #[arg(long, default_value = "canxin121/agena-plugin-marketplace")]
    pub registry: String,
    #[arg(long, default_value = "default")]
    pub registry_id: String,
}

#[derive(Debug, Clone, Args)]
/// Arguments for upgrading plugins.
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
/// Session management command.
pub struct SessionsCommand {
    #[command(subcommand)]
    pub command: Option<SessionsSubcommand>,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the RPC server.
pub struct RpcServerArgs {
    /// Base URL of the remote server. The IDE bridge is a remote client and does
    /// not create an in-process Runtime.
    #[arg(skip)]
    pub server: Option<String>,
    #[arg(skip)]
    pub server_token: Option<String>,
    #[arg(skip)]
    pub server_password: Option<String>,
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value_t = RpcServerTransport::Stdio, value_enum)]
    pub transport: RpcServerTransport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// Transport of the RPC server.
pub enum RpcServerTransport {
    Stdio,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the HTTP server.
pub struct ServerArgs {
    /// Optional lifecycle operation. Omit to run the server in the foreground.
    #[arg(value_enum)]
    pub action: Option<ServerLifecycleAction>,
    #[arg(skip)]
    pub overrides: Vec<String>,
    #[arg(skip)]
    pub data_dir: Option<PathBuf>,
    #[arg(long, env = "AGENA_SERVER_HOST", default_value = "127.0.0.1")]
    pub host: String,
    #[arg(short, long, env = "AGENA_SERVER_PORT", default_value_t = 3210)]
    pub port: u16,
    /// Server password shared by Web/TUI login and MCP OAuth authorization.
    /// Set AGENA_SERVER_PASSWORD to avoid exposing it in process arguments.
    #[arg(
        long,
        env = "AGENA_SERVER_PASSWORD",
        alias = "ui-password",
        hide_env_values = true,
        value_name = "PASSWORD"
    )]
    pub password: Option<String>,
    /// Whether the HTTP MCP surface is enabled. An explicit CLI/environment
    /// value overrides a persisted Web/TUI selection on every process start.
    #[arg(long, env = "AGENA_MCP_ENABLED", value_name = "BOOL")]
    pub mcp_enabled: Option<bool>,
    /// Public MCP resource URL. A bare origin is normalized to `/mcp`. When
    /// omitted, the listener-local URL is used; request forwarding headers are
    /// never trusted to define OAuth identity.
    #[arg(long, env = "AGENA_MCP_PUBLIC_URL", value_name = "URL")]
    pub mcp_public_url: Option<String>,
    /// Agena access address. Derives /mcp and all embedded OAuth endpoints;
    /// remote addresses require HTTPS, while loopback addresses allow HTTP.
    #[arg(long, env = "AGENA_MCP_SERVER_URL", value_name = "URL", conflicts_with_all = ["mcp_public_url", "mcp_oauth_issuer_url"])]
    pub mcp_server_url: Option<String>,
    /// Public browser-facing issuer for Agena's built-in OAuth server. It must
    /// be an HTTPS origin without a path. Omit it when OAuth and MCP use the
    /// same domain; Agena then derives the issuer from the MCP public URL.
    #[arg(long, env = "AGENA_MCP_OAUTH_ISSUER_URL", value_name = "URL")]
    pub mcp_oauth_issuer_url: Option<String>,
    #[arg(long = "workspace", env = "AGENA_WORKSPACE_ROOT", value_name = "PATH")]
    pub workspace_root: Option<PathBuf>,
    /// Directory containing the built Web frontend. When omitted, repository
    /// and packaged `web-dist` layouts are auto-detected.
    #[arg(long = "ui-dir", env = "AGENA_SERVER_UI_DIR", value_name = "PATH")]
    pub ui_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
#[value(rename_all = "kebab_case")]
pub enum ServerLifecycleAction {
    Start,
    Status,
    Stop,
    Install,
    Uninstall,
    /// Open the installed macOS service's one-time Full Disk Access setup.
    Permissions,
}

#[derive(Debug, Clone, Args)]
#[command(
    about = "Expose server-owned runtime tools over MCP stdio",
    long_about = "Run a tools-only MCP stdio bridge backed by the long-lived server. \
                  Tool discovery and invocation cross the public server API; this client never \
                  starts or shuts down a Runtime, and stdio EOF does not cancel server-owned work."
)]
/// Arguments for the thin-client MCP stdio bridge.
pub struct McpServerArgs {
    #[arg(long = "workspace", env = "AGENA_WORKSPACE_ROOT", value_name = "PATH")]
    pub workspace: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
/// MCP management command.
pub struct McpCommand {
    #[command(subcommand)]
    pub command: Option<McpSubcommand>,
}

#[derive(Debug, Clone, Subcommand)]
/// MCP subcommands.
pub enum McpSubcommand {
    /// Show live, redacted MCP connection health for every configured server.
    Status(McpStatusArgs),
    /// List every configured MCP server with its current connection state.
    List(McpStatusArgs),
    /// Show one configured MCP server with its current connection state.
    Get(McpGetArgs),
    /// Add one stdio or streamable-HTTP MCP server to Agena configuration.
    Add(Box<McpAddArgs>),
    /// Remove a configured MCP server from Agena configuration.
    Remove(McpRemoveArgs),
    /// Enable the static MCP bridge plugin in Agena configuration.
    Enable(McpPluginToggleArgs),
    /// Disable the static MCP bridge plugin without deleting server records.
    Disable(McpPluginToggleArgs),
    /// Reconnect one configured MCP server and refresh its tool cache.
    Reconnect(McpReconnectArgs),
}

#[derive(Debug, Clone, Args)]
/// Arguments for MCP status.
pub struct McpStatusArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for reading MCP config.
pub struct McpGetArgs {
    pub server: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
/// Config layer of an MCP edit.
pub enum McpConfigLayerArg {
    #[default]
    Global,
    Workspace,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
/// HTTP auth of an MCP server.
pub enum McpHttpAuthArg {
    #[default]
    None,
    BearerFromStore,
    BearerFromEnv,
    #[value(name = "oauth")]
    OAuth,
}

#[derive(Debug, Clone, Args)]
/// Arguments for adding an MCP server.
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
/// Arguments for removing an MCP server.
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
/// Arguments for toggling an MCP plugin.
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
/// Arguments for reconnecting MCP servers.
pub struct McpReconnectArgs {
    pub server: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Subcommand)]
/// Auth subcommands.
pub enum AuthSubcommand {
    List(AuthListArgs),
}

#[derive(Debug, Clone, Subcommand)]
/// Config subcommands.
pub enum ConfigSubcommand {
    Resolve(ConfigResolveArgs),
    Validate,
}

#[derive(Debug, Clone, Subcommand)]
/// Debug subcommands.
pub enum DebugSubcommand {
    Session(DebugSessionArgs),
}

#[derive(Debug, Clone, Subcommand)]
/// Provider subcommands.
pub enum ProviderSubcommand {
    List(ProviderListArgs),
    Models(ProviderModelsArgs),
    Capabilities(ProviderCapabilitiesArgs),
}

#[derive(Debug, Clone, Subcommand)]
/// Memory subcommands.
pub enum MemorySubcommand {
    List(MemoryListArgs),
    Forget(MemoryForgetArgs),
    Edit(MemoryEditArgs),
}

#[derive(Debug, Clone, Subcommand)]
/// Sessions subcommands.
pub enum SessionsSubcommand {
    List(SessionListArgs),
    /// Read or change a session's additional system prompt.
    SystemPrompt(SessionSystemPromptArgs),
    /// Export a session to a JSONL bundle (stdout). Pipe to a file to keep.
    Export(SessionExportArgs),
    /// Replay a JSONL bundle (read from stdin) as a fresh session in the
    /// current workspace.
    Import(SessionImportArgs),
    /// Print every session sharing the given tree root, in (depth, id) order.
    Tree(SessionTreeArgs),
}

#[derive(Debug, Clone, Args)]
/// Arguments for a session's additional instructions.
pub struct SessionSystemPromptArgs {
    pub session_id: i64,
    /// Set an explicit override. Pass an empty string to disable inherited instructions.
    #[arg(long, conflicts_with_all = ["file", "inherit"])]
    pub text: Option<String>,
    /// Read the exact prompt text from a UTF-8 file.
    #[arg(long, conflicts_with_all = ["text", "inherit"])]
    pub file: Option<PathBuf>,
    /// Remove the session override and resume model/project/global inheritance.
    #[arg(long, conflicts_with_all = ["text", "file"])]
    pub inherit: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for session export.
pub struct SessionExportArgs {
    pub session_id: i64,
}

#[derive(Debug, Clone, Args)]
/// Arguments for session import.
pub struct SessionImportArgs {
    /// Optional path. Reads from stdin if omitted.
    #[arg(long)]
    pub path: Option<std::path::PathBuf>,
    /// Reuse this UUID when retrying the same archive after a lost response.
    #[arg(long)]
    pub idempotency_key: Option<agena_domain::CommandId>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for the session tree.
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
/// Arguments for listing auth.
pub struct AuthListArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for provider login.
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
/// Arguments for provider logout.
pub struct LogoutArgs {
    pub provider_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, ValueEnum)]
/// View of a session listing.
pub enum SessionListView {
    All,
    Roots,
    Subtree,
}

#[derive(Debug, Clone, Args)]
/// Arguments for listing sessions.
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
/// Arguments for listing memory.
pub struct MemoryListArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for forgetting memory.
pub struct MemoryForgetArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    pub name: String,
}

#[derive(Debug, Clone, Args)]
/// Arguments for editing memory.
pub struct MemoryEditArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Args)]
/// Arguments for resuming a session.
pub struct ResumeArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for shell completion.
pub struct CompletionArgs {
    #[arg(value_enum)]
    pub shell: clap_complete::Shell,
}

#[derive(Debug, Clone, Args)]
/// Arguments for continuing a run.
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
/// Apply a patch through the server's workspace-bound tool bridge.
pub struct ApplyArgs {
    #[arg(long = "workspace")]
    pub workspace: Option<PathBuf>,
    #[arg(long)]
    pub json: bool,
    pub patch_file: PathBuf,
}

#[derive(Debug, Clone, Args)]
/// Arguments for debug session.
pub struct DebugSessionArgs {
    pub session_id: i64,
    #[arg(long)]
    pub json: bool,
}

#[derive(Debug, Clone, Args)]
/// Arguments for exec.
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
/// Arguments for the TUI.
pub struct TuiArgs {
    /// Base URL of the remote server. The TUI is a remote client by default.
    #[arg(skip)]
    pub server: Option<String>,
    #[arg(skip)]
    pub server_token: Option<String>,
    #[arg(skip)]
    pub server_password: Option<String>,
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
/// Arguments for review.
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
/// Arguments for forking a session.
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
/// Arguments for config resolution.
pub struct ConfigResolveArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for listing providers.
pub struct ProviderListArgs {
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for provider models.
pub struct ProviderModelsArgs {
    pub provider_id: String,
    #[arg(long, default_value = "json")]
    pub format: OutputFormat,
}

#[derive(Debug, Clone, Args)]
/// Arguments for provider capabilities.
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
    runs: Vec<DebugRunOutput>,
}

#[derive(Debug, Serialize)]
struct DebugRunOutput {
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
    agena_data_dir_set: bool,
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
}

#[derive(Debug, Serialize)]
struct ProviderModelsOutput {
    provider_id: String,
    models: Vec<agena_api::resource::ProviderModelResource>,
}

#[derive(Debug, Serialize)]
struct ProviderCapabilitiesOutput {
    provider_id: String,
    model: String,
    model_ref: String,
    capabilities: agena_api::resource::ProviderModelCapabilitiesResource,
    metadata: agena_api::resource::ProviderModelMetadataResource,
}

#[derive(Debug, Serialize)]
struct PluginStatusOutput {
    statuses: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
struct PluginInspectOutput {
    plugin: serde_json::Value,
}

#[derive(Debug, Serialize, Deserialize)]
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
