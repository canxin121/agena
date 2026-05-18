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
        ProcessEnvironment, ProviderAuthConfig, ProviderConfigCredentialStore, TracingConfig,
        provider_gitlab_instance_url,
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
        auth::{AuthData, AuthManager, CopilotDeployment, wait_for_oauth_callback},
    },
    role::Role,
    runtime::{AgenaRuntime, TracingFilterReloadHandle},
    session::{
        Session, SessionContinueRequest, SessionCreateRequest, SessionForkRequest,
        SessionListRequest, SessionManager, SessionRunOptions, SessionRuntimeStatus,
        SessionSummary, SessionUserTurnRequest,
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
                  Configuration is loaded from $AGENA_CONFIG, the path passed \
                  with --config, or `agena.toml` in the workspace. \
                  Run `agena config show` to inspect the resolved settings.",
    after_help = "EXAMPLES:\n  \
                  Start the terminal UI:\n    \
                  agena\n\n  \
                  Start the terminal UI at a specific session:\n    \
                  agena tui --session 42\n\n  \
                  Start a one-shot turn:\n    \
                  agena exec \"explain crates/agena-api-server\"\n\n  \
                  Resume the most recent session:\n    \
                  agena resume\n\n  \
                  Show effective config:\n    \
                  agena config show --format toml\n\n  \
                  Run as an MCP server over stdio:\n    \
                  agena mcp-server --transport stdio"
)]
pub struct AgenaCli {
    #[arg(long, env = "AGENA_CONFIG", global = true)]
    pub config: Option<PathBuf>,
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
    Worktree(WorktreeArgs),
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CostArgs {
    pub session_id: Option<i64>,
    #[arg(long)]
    pub last: bool,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct CommitArgs {
    pub message: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct WorktreeArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct GitArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct PluginInspectArgs {
    pub plugin_id: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    /// Path to the agena config.toml that should receive the plugin tool.
    #[arg(long)]
    pub config: Option<PathBuf>,
    /// Overwrite an existing entry with the same plugin id.
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
    /// Upgrade every installed plugin in turn.
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
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long = "workspace", alias = "cwd")]
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
    Goal(SessionGoalCommand),
    /// Legacy compatibility command. Rewind now creates a fork, so unrewind
    /// returns the current source session without mutating it.
    Unrewind(SessionUnrewindArgs),
    /// Export a session to a JSONL bundle (stdout). Pipe to a file to keep.
    Export(SessionExportArgs),
    /// Replay a JSONL bundle (read from stdin) as a fresh session in the
    /// current workspace.
    Import(SessionImportArgs),
    /// Print every session sharing the given tree root, in (depth, id) order.
    Tree(SessionTreeArgs),
    /// List rewind audit checkpoints for a session — what was dropped and when.
    Checkpoints(SessionCheckpointsArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SessionGoalCommand {
    #[command(subcommand)]
    pub command: SessionGoalSubcommand,
}

#[derive(Debug, Clone, Subcommand)]
pub enum SessionGoalSubcommand {
    Get(SessionGoalGetArgs),
    Create(SessionGoalCreateArgs),
    Complete(SessionGoalCompleteArgs),
    Clear(SessionGoalClearArgs),
}

#[derive(Debug, Clone, Args)]
pub struct SessionGoalGetArgs {
    pub session_id: i64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionGoalCreateArgs {
    pub session_id: i64,
    pub objective: String,
    #[arg(long)]
    pub token_budget: Option<u64>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionGoalCompleteArgs {
    pub session_id: i64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionGoalClearArgs {
    pub session_id: i64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionCheckpointsArgs {
    pub session_id: i64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct SessionUnrewindArgs {
    pub session_id: i64,
    #[arg(long = "message")]
    pub message_id: i64,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct AuthListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryListArgs {
    #[arg(long = "workspace", alias = "cwd")]
    pub workspace: Option<PathBuf>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryForgetArgs {
    #[arg(long = "workspace", alias = "cwd")]
    pub workspace: Option<PathBuf>,
    pub name: String,
}

#[derive(Debug, Clone, Args)]
pub struct MemoryEditArgs {
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ApplyArgs {
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long, env = "AGENA_TUI_CONFIG")]
    pub tui_config: Option<PathBuf>,
}

#[derive(Debug, Clone, Args)]
pub struct ReviewArgs {
    #[arg(long = "workspace", alias = "cwd")]
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
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ConfigResolveArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderModelsArgs {
    pub provider_id: String,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct ProviderCapabilitiesArgs {
    pub target: String,
    #[arg(long)]
    pub model: Option<String>,
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
    pub format: ConfigOutputFormat,
}

#[derive(Debug, Clone, Args)]
pub struct AgentsListArgs {
    #[arg(long, value_enum, default_value_t = ConfigOutputFormat::Toml)]
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
struct SessionUnrewindOutput {
    session: SessionDetail,
}

#[derive(Debug, Serialize)]
struct SessionImportOutput {
    session: SessionDetail,
}

#[derive(Debug, Serialize)]
struct SessionCheckpointsOutput {
    session_id: i64,
    checkpoints: Vec<crate::session::RewindCheckpoint>,
}

#[derive(Debug, Serialize)]
struct SessionGoalOutput {
    session_id: i64,
    goal: Option<crate::session::SessionGoal>,
}

#[derive(Debug, Serialize)]
struct SessionGoalClearedOutput {
    session_id: i64,
    cleared: bool,
}

#[derive(Debug, Serialize)]
struct MemoryListOutput {
    dir: String,
    count: usize,
    entries: Vec<MemorySummaryOutput>,
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
    telemetry: DiagnosticsTelemetryOutput,
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
struct ActiveWorktreeOutput {
    session_id: i64,
    path: String,
    branch: String,
    created_here: bool,
}

#[derive(Debug, Serialize)]
struct ManagedWorktreeOutput {
    path: String,
    session_id: Option<i64>,
    branch: Option<String>,
    registered_with_git: bool,
    stale: bool,
}

#[derive(Debug, Serialize)]
struct WorktreeOutput {
    workspace_root: String,
    active: Vec<ActiveWorktreeOutput>,
    managed: Vec<ManagedWorktreeOutput>,
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
    worktree_active_sessions: u64,
    worktree_managed_dirs: u64,
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
    applied_layers: Vec<String>,
    provider_count: usize,
    plugin_count: usize,
}

#[derive(Debug, Serialize)]
struct DiagnosticsTelemetryOutput {
    enabled: bool,
    service_name: String,
    otlp_endpoint_set: bool,
    header_count: usize,
}

#[derive(Debug, Serialize)]
struct DiagnosticsEnvironmentOutput {
    agena_config_set: bool,
    agena_database_url_set: bool,
    agena_database_path_set: bool,
    agena_telemetry_enabled_set: bool,
    agena_otel_endpoint_set: bool,
    otel_exporter_otlp_traces_endpoint_set: bool,
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
    status: SessionRuntimeStatus,
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
    default_adapter: Option<String>,
    default_model: String,
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
    primary_count: usize,
    subagent_count: usize,
    hidden_count: usize,
    agents: Vec<crate::agents::AgentDescriptor>,
}

#[derive(Debug, Serialize)]
struct PluginStatusOutput {
    entries: Vec<crate::plugin::status::PluginStatus>,
}

#[derive(Debug, Serialize)]
struct PluginInspectOutput {
    plugin: crate::plugin::PluginInspect,
}

#[derive(Debug, Serialize)]
struct PluginLogsOutput {
    plugin_id: String,
    entries: Vec<crate::plugin::PluginLogEntry>,
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
            Some(AgenaCommand::Git(args)) => self.run_git(args).await,
            Some(AgenaCommand::Login(args)) => self.run_login(loader, args).await,
            Some(AgenaCommand::Logout(args)) => self.run_logout(loader, args),
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
            Some(AgenaCommand::Worktree(args)) => self.run_worktree(args).await,
            None => self.run_default(loader, tracing_reload_handle).await,
        }
    }

    async fn run_default(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        tracing_reload_handle: Option<TracingFilterReloadHandle>,
    ) -> Result<(), AppError> {
        let _resolution = loader.load(&self.load_request())?;
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
                    entries: runtime
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
                    entries: plugin_manager.plugin_logs(
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
            PluginSubcommand::Install(args) => {
                let registry_url = args.registry.ok_or_else(|| {
                    AppError::Config("agena plugin install requires --registry <url>".to_string())
                })?;
                let (plugin_id, version) = match args.spec.split_once('@') {
                    Some((id, ver)) => (id.to_string(), Some(ver.to_string())),
                    None => (args.spec.clone(), None),
                };
                let config_path = args
                    .config
                    .clone()
                    .or_else(default_user_config_path)
                    .ok_or_else(|| {
                        AppError::Config(
                            "could not determine config path; pass --config".to_string(),
                        )
                    })?;
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
                ProviderAuthConfig::Api(_) | ProviderAuthConfig::SapAiCore(_)
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
            match &resolved.auth {
                ProviderAuthConfig::Credential(config)
                    if matches!(
                        config.issuer,
                        crate::provider::auth::CredentialIssuer::OpenaiChatgpt
                    ) =>
                {
                    let redirect_uri = format!("http://localhost:{}/auth/callback", args.port);
                    let start = manager.start_openai_browser_login(redirect_uri.clone())?;
                    println!("open this URL to continue: {}", start.authorize_url);
                    io::stdout().flush()?;
                    let callback = wait_for_oauth_callback(
                        args.port,
                        start.state.as_str(),
                        Duration::from_secs(args.timeout_secs),
                    )?;
                    manager
                        .finish_openai_browser_login(
                            provider_id.as_str(),
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await?;
                }
                ProviderAuthConfig::Credential(config)
                    if matches!(
                        config.issuer,
                        crate::provider::auth::CredentialIssuer::Gitlab
                    ) =>
                {
                    let instance_url = provider_gitlab_instance_url(resolved).ok_or_else(|| {
                        AppError::Config(format!(
                            "{provider_id} has ambiguous gitlab browser auth adapters"
                        ))
                    })?;
                    let redirect_uri = format!("http://localhost:{}/auth/callback", args.port);
                    let start =
                        manager.start_gitlab_login(instance_url.clone(), redirect_uri.clone())?;
                    println!("open this URL to continue: {}", start.authorize_url);
                    io::stdout().flush()?;
                    let callback = wait_for_oauth_callback(
                        args.port,
                        start.state.as_str(),
                        Duration::from_secs(args.timeout_secs),
                    )?;
                    manager
                        .finish_gitlab_login(
                            provider_id.as_str(),
                            instance_url,
                            callback.code,
                            start.pkce_verifier,
                            redirect_uri,
                        )
                        .await?;
                }
                ProviderAuthConfig::Credential(config)
                    if matches!(
                        config.issuer,
                        crate::provider::auth::CredentialIssuer::AtomGit
                    ) =>
                {
                    let start = manager.start_atomgit_login().await?;
                    println!("open this URL to continue: {}", start.authorize_url);
                    io::stdout().flush()?;
                    let auth = poll_until(
                        Duration::from_secs(args.timeout_secs),
                        Duration::from_secs(2),
                        || manager.poll_atomgit_login(provider_id.as_str(), start.state.clone()),
                    )
                    .await?;
                    if auth.is_none() {
                        return Err(AppError::Config(
                            "atomgit browser login timed out".to_owned(),
                        ));
                    }
                }
                _ => {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support browser login"
                    )));
                }
            }
            println!("logged in: {provider_id}");
            return Ok(());
        }

        if args.device {
            match &resolved.auth {
                ProviderAuthConfig::Credential(config)
                    if matches!(
                        config.issuer,
                        crate::provider::auth::CredentialIssuer::OpenaiChatgpt
                    ) =>
                {
                    let start = manager.start_openai_headless_login().await?;
                    println!("open this URL: {}", start.verification_url);
                    println!("enter code: {}", start.user_code);
                    io::stdout().flush()?;
                    let auth = poll_until(
                        Duration::from_secs(args.timeout_secs),
                        Duration::from_secs(start.interval_seconds.max(1)),
                        || {
                            manager.poll_openai_headless_login(
                                provider_id.as_str(),
                                start.device_code.clone(),
                                start.user_code.clone(),
                            )
                        },
                    )
                    .await?;
                    if auth.is_none() {
                        return Err(AppError::Config("openai device login timed out".to_owned()));
                    }
                }
                ProviderAuthConfig::Credential(config)
                    if matches!(
                        config.issuer,
                        crate::provider::auth::CredentialIssuer::GithubCopilot
                    ) =>
                {
                    let deployment = match args
                        .enterprise_domain
                        .as_deref()
                        .map(str::trim)
                        .filter(|value| !value.is_empty())
                    {
                        Some(domain) => CopilotDeployment::Enterprise {
                            domain: domain.to_owned(),
                        },
                        None => CopilotDeployment::GitHubCom,
                    };
                    let start = manager.start_copilot_login(deployment.clone()).await?;
                    println!("open this URL: {}", start.verification_url);
                    println!("enter code: {}", start.user_code);
                    io::stdout().flush()?;
                    let auth = poll_until(
                        Duration::from_secs(args.timeout_secs),
                        Duration::from_secs(start.interval_seconds.max(1)),
                        || {
                            manager.poll_copilot_login(
                                provider_id.as_str(),
                                start.device_code.clone(),
                                deployment.clone(),
                            )
                        },
                    )
                    .await?;
                    if auth.is_none() {
                        return Err(AppError::Config(
                            "copilot device login timed out".to_owned(),
                        ));
                    }
                }
                _ => {
                    return Err(AppError::Config(format!(
                        "{provider_id} does not support device login"
                    )));
                }
            }
            println!("logged in: {provider_id}");
        }

        Ok(())
    }

    fn run_logout(
        self,
        loader: ConfigLoader<ProcessEnvironment>,
        args: LogoutArgs,
    ) -> Result<(), AppError> {
        let resolution = loader.load(&self.load_request())?;
        let manager = AuthManager::new(ProviderConfigCredentialStore::new(
            resolution.meta.config_path,
        ));
        let provider_id = normalize_login_provider(args.provider_id.as_str());
        manager.remove(provider_id.as_str())?;
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

    async fn run_permissions(self, args: PermissionsArgs) -> Result<(), AppError> {
        let output = self.render_permissions_command(args).await?;
        println!("{output}");
        Ok(())
    }

    async fn run_worktree(self, args: WorktreeArgs) -> Result<(), AppError> {
        let output = self.render_worktree_command(args).await?;
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
                format: ConfigOutputFormat::Toml,
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
                format: ConfigOutputFormat::Toml,
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
                format: ConfigOutputFormat::Toml,
            })) {
            MemorySubcommand::List(args) => {
                let store = self.memory_store_for_workspace(args.workspace.as_ref())?;
                let entries = store
                    .list()
                    .map_err(|error| AppError::Config(error.to_string()))?;
                let entries = entries
                    .into_iter()
                    .map(|entry| MemorySummaryOutput {
                        file_name: entry.file_name.clone(),
                        name: memory_entry_name(&entry),
                        description: entry.frontmatter.description.clone(),
                        memory_type: memory_type_label(entry.frontmatter.r#type),
                        path: entry.path.display().to_string(),
                    })
                    .collect::<Vec<_>>();
                render_serialized(
                    args.format,
                    &MemoryListOutput {
                        dir: store.dir().display().to_string(),
                        count: entries.len(),
                        entries,
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
                format: ConfigOutputFormat::Toml,
            })) {
            SessionsSubcommand::List(args) => {
                let sessions = list_all_session_summaries(manager.as_ref()).await?;
                let sessions =
                    filter_session_summaries_by_view(sessions, args.view, args.anchor_session_id)?;
                let sessions = paginate_session_summaries(sessions, args.offset, args.limit);
                render_serialized(args.format, &SessionListOutput { sessions })
            }
            SessionsSubcommand::Goal(args) => match args.command {
                SessionGoalSubcommand::Get(args) => {
                    let goal = manager.get_goal(args.session_id).await?;
                    render_serialized(
                        args.format,
                        &SessionGoalOutput {
                            session_id: args.session_id,
                            goal,
                        },
                    )
                }
                SessionGoalSubcommand::Create(args) => {
                    let goal = manager
                        .create_goal(crate::session::SessionGoalCreateRequest {
                            session_id: args.session_id,
                            objective: args.objective,
                            token_budget: args.token_budget,
                        })
                        .await?;
                    render_serialized(
                        args.format,
                        &SessionGoalOutput {
                            session_id: args.session_id,
                            goal: Some(goal),
                        },
                    )
                }
                SessionGoalSubcommand::Complete(args) => {
                    let goal = manager.complete_goal(args.session_id).await?;
                    render_serialized(
                        args.format,
                        &SessionGoalOutput {
                            session_id: args.session_id,
                            goal: Some(goal),
                        },
                    )
                }
                SessionGoalSubcommand::Clear(args) => {
                    let cleared = manager.clear_goal(args.session_id).await?;
                    render_serialized(
                        args.format,
                        &SessionGoalClearedOutput {
                            session_id: args.session_id,
                            cleared,
                        },
                    )
                }
            },
            SessionsSubcommand::Unrewind(args) => {
                let session = manager
                    .unrewind_session(crate::session::SessionUnrewindRequest {
                        expected_version: None,
                        session_id: args.session_id,
                        message_id: args.message_id,
                    })
                    .await?;
                let latest_event_seq = latest_event_seq(&manager, session.id).await?;
                render_serialized(
                    args.format,
                    &SessionUnrewindOutput {
                        session: session_detail(&session, latest_event_seq),
                    },
                )
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
            SessionsSubcommand::Checkpoints(args) => {
                let checkpoints = manager.list_rewind_checkpoints(args.session_id).await?;
                render_serialized(
                    args.format,
                    &SessionCheckpointsOutput {
                        session_id: args.session_id,
                        checkpoints,
                    },
                )
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
            manager
                .continue_session(SessionContinueRequest {
                    session_id,
                    options: SessionRunOptions {
                        model: default_model(&runtime)?,
                        thinking_mode: None,
                        speed_mode: None,
                        verbosity: None,
                        thinking: None,
                        request_override: Default::default(),
                        system: None,
                        temperature: None,
                        max_output_tokens: None,
                        agent_profile: args
                            .agent
                            .as_deref()
                            .map(str::trim)
                            .filter(|value| !value.is_empty())
                            .map(ToOwned::to_owned),
                        max_turn_loops: None,
                    },
                })
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

    async fn render_permissions_command(&self, args: PermissionsArgs) -> Result<String, AppError> {
        match args
            .command
            .unwrap_or(PermissionsSubcommand::List(PermissionsListArgs {
                search: None,
                format: ConfigOutputFormat::Toml,
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
            .reply_permission(crate::session::SessionPermissionReplyRequest {
                session_id,
                options: resolve_run_options(&runtime, None, None, None, None)?,
                reply: PermissionReply {
                    request_id: args.request_id,
                    kind: permission_reply_kind_from_arg(args.kind),
                    reason: args.reason,
                    scope: args.scope.map(permission_scope_from_arg),
                },
                operator: Some("cli".to_string()),
            })
            .await?;
        let latest_event_seq = latest_event_seq(&manager, session.id).await?;
        render_serialized(
            args.format,
            &SessionOutput {
                session: session_detail(&session, latest_event_seq),
            },
        )
    }

    async fn render_worktree_command(&self, args: WorktreeArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let manager = runtime
            .session_manager()
            .ok_or_else(session_storage_error)?;
        let executor = manager.tool_executor();
        let registry = executor.worktree_registry().ok_or_else(|| {
            AppError::Config("worktree registry is not enabled in this runtime".to_owned())
        })?;
        let active = crate::tool::worktree_list_active(registry)
            .into_iter()
            .map(|entry| ActiveWorktreeOutput {
                session_id: entry.session_id,
                path: entry.path.display().to_string(),
                branch: entry.branch,
                created_here: entry.created_here,
            })
            .collect::<Vec<_>>();
        let managed = crate::tool::worktree_list_managed(runtime.workspace_root(), registry)
            .into_iter()
            .map(|entry| {
                let stale = entry.is_stale();
                ManagedWorktreeOutput {
                    path: entry.path.display().to_string(),
                    session_id: entry.session_id,
                    branch: entry.branch,
                    registered_with_git: entry.registered_with_git,
                    stale,
                }
            })
            .collect::<Vec<_>>();
        render_serialized(
            args.format,
            &WorktreeOutput {
                workspace_root: runtime.workspace_root().display().to_string(),
                active,
                managed,
            },
        )
    }

    async fn render_git_command(&self, args: GitArgs) -> Result<String, AppError> {
        let runtime = self.session_runtime().await?;
        let workspace_root = runtime.workspace_root().to_path_buf();
        let preflight = collect_git_preflight(&workspace_root)?;

        let (worktree_active_sessions, worktree_managed_dirs) = match runtime.session_manager() {
            Some(manager) => {
                let executor = manager.tool_executor();
                match executor.worktree_registry() {
                    Some(registry) => (
                        crate::tool::worktree_list_active(registry).len() as u64,
                        crate::tool::worktree_list_managed(runtime.workspace_root(), registry).len()
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
                worktree_active_sessions,
                worktree_managed_dirs,
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
            .continue_session(SessionContinueRequest {
                session_id,
                options,
            })
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
        let telemetry = &config.telemetry;
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
                    applied_layers: resolution
                        .meta
                        .applied_layers
                        .iter()
                        .map(|layer| layer.description.clone())
                        .collect(),
                    provider_count: config.providers.len(),
                    plugin_count: config.plugins.list.len(),
                },
                telemetry: DiagnosticsTelemetryOutput {
                    enabled: telemetry.enabled,
                    service_name: telemetry.service_name.clone(),
                    otlp_endpoint_set: telemetry.otlp_endpoint.is_some(),
                    header_count: telemetry.headers.len(),
                },
                environment: DiagnosticsEnvironmentOutput {
                    agena_config_set: std::env::var_os("AGENA_CONFIG").is_some(),
                    agena_database_url_set: std::env::var_os("AGENA_DATABASE_URL").is_some(),
                    agena_database_path_set: std::env::var_os("AGENA_DATABASE_PATH").is_some(),
                    agena_telemetry_enabled_set: std::env::var_os("AGENA_TELEMETRY_ENABLED")
                        .is_some(),
                    agena_otel_endpoint_set: std::env::var_os("AGENA_OTEL_ENDPOINT").is_some(),
                    otel_exporter_otlp_traces_endpoint_set: std::env::var_os(
                        "OTEL_EXPORTER_OTLP_TRACES_ENDPOINT",
                    )
                    .is_some(),
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
            .submit_user_turn(SessionUserTurnRequest {
                session_id: created.id,
                options,
                parts: vec![PartContent::text(prompt)],
            })
            .await?;
        if session.runtime.turn.status == SessionRuntimeStatus::Blocked {
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
        let mut builder = AgenaRuntime::builder()
            .with_load_request(self.load_request())
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
                format: ConfigOutputFormat::Toml,
            })) {
            ProviderSubcommand::List(args) => {
                let mut providers = registry
                    .provider_ids()
                    .into_iter()
                    .filter_map(|provider_id| {
                        registry
                            .get(provider_id.as_str())
                            .map(|provider| ProviderSummary {
                                default_adapter: provider
                                    .default_adapter()
                                    .map(ToString::to_string),
                                default_model: provider.default_model().to_string(),
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
        let default_agent = Some(resolution.config.default.agent.clone())
            .filter(|name| agents.iter().any(|entry| entry.name == *name))
            .or_else(|| {
                agents
                    .iter()
                    .find(|entry| entry.mode.allows_root() && !entry.hidden)
                    .map(|entry| entry.name.clone())
            })
            .unwrap_or_else(|| "none".to_string());
        let total_count = agents.len();
        let primary_count = agents
            .iter()
            .filter(|entry| entry.mode.allows_root())
            .count();
        let subagent_count = agents
            .iter()
            .filter(|entry| entry.mode.allows_subagent())
            .count();
        let hidden_count = agents.iter().filter(|entry| entry.hidden).count();

        match command
            .command
            .unwrap_or(AgentsSubcommand::List(AgentsListArgs {
                format: ConfigOutputFormat::Toml,
            })) {
            AgentsSubcommand::List(args) => render_serialized(
                args.format,
                &AgentsListOutput {
                    default_agent,
                    total_count,
                    primary_count,
                    subagent_count,
                    hidden_count,
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
        let agent = Agent::new("mcp-server", PermissionPolicy::allow_all())
            .with_tool_policy(ToolPermissionPolicy::allow_all());
        let executor = ToolExecutor::new(runtime.workspace_root().to_path_buf(), agent)
            .with_plugin_manager(Arc::clone(&plugins));
        Ok(AgenaMcpBackend {
            executor,
            plugins,
            session_manager: runtime.session_manager(),
            workspace_root: runtime.workspace_root().to_path_buf(),
            next_call_id: Arc::new(AtomicI64::new(1)),
        })
    }

    pub fn load_request(&self) -> LoadConfigRequest {
        LoadConfigRequest {
            config_path: self.config.clone(),
            overrides: self.overrides.clone(),
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
                let input_schema = tool.sanitized_input_schema();
                ToolDescriptor {
                    name: tool.exposed_name,
                    description: Some(description),
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
            .entry_entries()
            .into_iter()
            .filter(|entry| {
                matches!(
                    entry.plugin_name.as_str(),
                    "agena.workflow" | "agena.skills"
                ) && entry.decl.input_schema
                    == crate::entry::definition::json_schema_for::<
                        crate::message::WorkflowPromptToolInput,
                    >()
            })
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
            .lookup_entry(params.name.as_str())
            .ok_or_else(|| McpServerError::NotFound(params.name.clone()))?;
        if !matches!(
            entry.plugin_name.as_str(),
            "agena.workflow" | "agena.skills"
        ) || entry.decl.input_schema
            != crate::entry::definition::json_schema_for::<crate::message::WorkflowPromptToolInput>(
            )
        {
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

fn memory_entry_name(entry: &crate::memory::MemoryEntry) -> String {
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

fn default_user_config_path() -> Option<PathBuf> {
    let base = std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(PathBuf::from)?;
    Some(base.join(".agena").join("config.toml"))
}

fn render_serialized<T>(format: ConfigOutputFormat, value: &T) -> Result<String, AppError>
where
    T: Serialize,
{
    match format {
        ConfigOutputFormat::Json => Ok(serde_json::to_string_pretty(value)?),
        ConfigOutputFormat::Toml => toml::to_string_pretty(value)
            .map_err(|err| AppError::Config(format!("failed to render toml output: {err}"))),
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
    if output.entries.is_empty() {
        return format!("plugin {} has no retained logs", output.plugin_id);
    }
    output
        .entries
        .iter()
        .map(|entry| {
            let timestamp = DateTime::<Utc>::from_timestamp_millis(entry.timestamp_ms)
                .map(|ts| ts.to_rfc3339())
                .unwrap_or_else(|| entry.timestamp_ms.to_string());
            let mut line = format!(
                "[{}] #{} {} {} {}",
                timestamp, entry.seq, entry.level, entry.source, entry.message
            );
            if !entry.fields.is_null() {
                line.push(' ');
                line.push_str(&entry.fields.to_string());
            }
            line
        })
        .collect::<Vec<_>>()
        .join("\n")
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
                    crate::provider::auth::CredentialIssuer::AtomGit => "atomgit".to_owned(),
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
        status: session.runtime.turn.status,
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
    } else if let (Some(provider_id), Some(model_id)) = (
        session.runtime.turn.model_provider_id.as_deref(),
        session.runtime.turn.model_id.as_deref(),
    ) {
        match session.runtime.turn.model_adapter_id.as_deref() {
            Some(adapter_id) => ModelRef::try_new_with_adapter(provider_id, adapter_id, model_id),
            None => ModelRef::try_new(provider_id, model_id),
        }
        .map_err(|err| AppError::Config(format!("invalid persisted model reference: {err}")))?
    } else {
        default_model(runtime)?
    };

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature: args.temperature,
        max_output_tokens: args.max_output_tokens,
        agent_profile: args
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        max_turn_loops: None,
    })
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

    Ok(SessionRunOptions {
        model,
        thinking_mode: None,
        speed_mode: None,
        verbosity: None,
        thinking: None,
        request_override: Default::default(),
        system: None,
        temperature,
        max_output_tokens,
        agent_profile: agent_profile
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        max_turn_loops: None,
    })
}

fn default_model(runtime: &AgenaRuntime) -> Result<ModelRef, AppError> {
    let snapshot = runtime.current_snapshot();
    let registry = snapshot.provider_registry();
    let default_config = &snapshot.config_resolution().config.default;
    if let Some(provider_id) = default_config.provider.as_deref() {
        return registry.resolve_model_selection(
            provider_id,
            default_config.adapter.as_deref(),
            default_config.model.as_deref(),
        );
    }

    let mut providers = registry.provider_ids();
    providers.sort();
    let provider_id = providers
        .first()
        .ok_or_else(|| AppError::Config("no providers configured".to_owned()))?;
    registry.resolve_model_target(provider_id, None)
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
    use std::{
        collections::BTreeMap,
        fs,
        path::PathBuf,
        time::{SystemTime, UNIX_EPOCH},
    };

    use serde_json::{Value, json};

    use super::*;
    use crate::{config::ConfigEnvironment, provider::CapabilitySupport};

    #[derive(Debug, Clone, Default)]
    struct TestEnvironment {
        vars: BTreeMap<String, String>,
    }

    impl ConfigEnvironment for TestEnvironment {
        fn var(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }

        fn vars(&self) -> Vec<(String, String)> {
            self.vars
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        }
    }

    #[tokio::test]
    async fn login_api_key_then_auth_list_redacts_secret() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let cli = AgenaCli {
            config: Some(path.clone()),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        cli.clone()
            .run_login(
                ConfigLoader::new(ProcessEnvironment),
                LoginArgs {
                    provider_id: "openai".to_owned(),
                    api_key: Some("sk-test".to_owned()),
                    browser: false,
                    device: false,
                    port: 1455,
                    timeout_secs: 1,
                    instance_url: "https://gitlab.com".to_owned(),
                    enterprise_domain: None,
                },
            )
            .await
            .expect("login should write credential");

        let output = cli
            .render_auth_command(
                &ConfigLoader::new(TestEnvironment::default()),
                AuthCommand {
                    command: Some(AuthSubcommand::List(AuthListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("auth list should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["credentials"][0]["provider_id"], "openai");
        assert_eq!(value["credentials"][0]["kind"], "api_key");
        assert!(!output.contains("sk-test"));
    }

    #[tokio::test]
    async fn logout_removes_cli_credential() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_model = "openai/gpt-4.1-mini"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let cli = AgenaCli {
            config: Some(path),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        cli.clone()
            .run_login(
                ConfigLoader::new(ProcessEnvironment),
                LoginArgs {
                    provider_id: "openai".to_owned(),
                    api_key: Some("sk-test".to_owned()),
                    browser: false,
                    device: false,
                    port: 1455,
                    timeout_secs: 1,
                    instance_url: "https://gitlab.com".to_owned(),
                    enterprise_domain: None,
                },
            )
            .await
            .expect("login should write credential");
        cli.clone()
            .run_logout(
                ConfigLoader::new(ProcessEnvironment),
                LogoutArgs {
                    provider_id: "openai".to_owned(),
                },
            )
            .expect("logout should remove credential");

        let output = cli
            .render_auth_command(
                &ConfigLoader::new(TestEnvironment::default()),
                AuthCommand {
                    command: Some(AuthSubcommand::List(AuthListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("auth list should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        assert!(value["credentials"].as_array().unwrap().is_empty());
    }

    #[tokio::test]
    async fn sessions_list_and_resume_last_render_session() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-session-{suffix}.db"));
        let config_path = write_temp_config("");
        let cli = AgenaCli {
            config: Some(config_path),
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let runtime = cli.session_runtime().await.expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let created = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "cli session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");

        let list_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::List(SessionListArgs {
                    limit: 10,
                    offset: 0,
                    view: SessionListView::All,
                    anchor_session_id: None,
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("sessions list should render");
        let list: Value = serde_json::from_str(list_output.as_str()).expect("list should be json");
        assert_eq!(list["sessions"][0]["id"], created.id);

        let resume_output = cli
            .render_resume_command(ResumeArgs {
                session_id: None,
                last: true,
                agent: None,
                format: ConfigOutputFormat::Json,
            })
            .await
            .expect("resume should render");
        let resumed: Value =
            serde_json::from_str(resume_output.as_str()).expect("resume should be json");
        assert_eq!(resumed["session"]["id"], created.id);
        assert_eq!(resumed["session"]["title"], "cli session");
    }

    #[tokio::test]
    async fn sessions_list_supports_roots_and_subtree_views() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-session-view-{suffix}.db"));
        let config_path = write_temp_config("");
        let cli = AgenaCli {
            config: Some(config_path),
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let runtime = cli.session_runtime().await.expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let root = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "root".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("root should be created");
        let child = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "child".to_owned(),
                parent_session_id: Some(root.id),
            })
            .await
            .expect("child should be created");
        let other = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "other".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("other root should be created");

        let roots_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::List(SessionListArgs {
                    limit: 20,
                    offset: 0,
                    view: SessionListView::Roots,
                    anchor_session_id: None,
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("roots view should render");
        let roots: Value =
            serde_json::from_str(roots_output.as_str()).expect("roots should be json");
        let root_ids = roots["sessions"]
            .as_array()
            .expect("sessions should be array")
            .iter()
            .map(|item| item["id"].as_i64().expect("id should be i64"))
            .collect::<Vec<_>>();
        assert!(root_ids.contains(&root.id));
        assert!(root_ids.contains(&other.id));
        assert!(!root_ids.contains(&child.id));

        let subtree_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::List(SessionListArgs {
                    limit: 20,
                    offset: 0,
                    view: SessionListView::Subtree,
                    anchor_session_id: Some(child.id),
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("subtree view should render");
        let subtree: Value =
            serde_json::from_str(subtree_output.as_str()).expect("subtree should be json");
        let subtree_ids = subtree["sessions"]
            .as_array()
            .expect("sessions should be array")
            .iter()
            .map(|item| item["id"].as_i64().expect("id should be i64"))
            .collect::<Vec<_>>();
        assert!(subtree_ids.contains(&root.id));
        assert!(subtree_ids.contains(&child.id));
        assert!(!subtree_ids.contains(&other.id));
        assert_eq!(subtree_ids.first().copied(), Some(root.id));
    }

    #[tokio::test]
    async fn sessions_goal_commands_render_goal_lifecycle() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-session-goal-{suffix}.db"));
        let config_path = write_temp_config("");
        let cli = AgenaCli {
            config: Some(config_path),
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let runtime = cli.session_runtime().await.expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let created = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "goal session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");

        let created_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::Goal(SessionGoalCommand {
                    command: SessionGoalSubcommand::Create(SessionGoalCreateArgs {
                        session_id: created.id,
                        objective: "cli goal".to_owned(),
                        token_budget: Some(321),
                        format: ConfigOutputFormat::Json,
                    }),
                })),
            })
            .await
            .expect("goal create should render");
        let created_value: Value =
            serde_json::from_str(created_output.as_str()).expect("output should be json");
        assert_eq!(created_value["session_id"], created.id);
        assert_eq!(created_value["goal"]["objective"], "cli goal");
        assert_eq!(created_value["goal"]["status"], "active");
        assert_eq!(created_value["goal"]["token_budget"], 321);

        let completed_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::Goal(SessionGoalCommand {
                    command: SessionGoalSubcommand::Complete(SessionGoalCompleteArgs {
                        session_id: created.id,
                        format: ConfigOutputFormat::Json,
                    }),
                })),
            })
            .await
            .expect("goal complete should render");
        let completed_value: Value =
            serde_json::from_str(completed_output.as_str()).expect("output should be json");
        assert_eq!(completed_value["goal"]["status"], "completed");
        assert!(completed_value["goal"]["completed_at"].is_string());

        let cleared_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::Goal(SessionGoalCommand {
                    command: SessionGoalSubcommand::Clear(SessionGoalClearArgs {
                        session_id: created.id,
                        format: ConfigOutputFormat::Json,
                    }),
                })),
            })
            .await
            .expect("goal clear should render");
        let cleared_value: Value =
            serde_json::from_str(cleared_output.as_str()).expect("output should be json");
        assert_eq!(cleared_value["session_id"], created.id);
        assert_eq!(cleared_value["cleared"], true);

        let fetched_output = cli
            .render_sessions_command(SessionsCommand {
                command: Some(SessionsSubcommand::Goal(SessionGoalCommand {
                    command: SessionGoalSubcommand::Get(SessionGoalGetArgs {
                        session_id: created.id,
                        format: ConfigOutputFormat::Json,
                    }),
                })),
            })
            .await
            .expect("goal get should render");
        let fetched_value: Value =
            serde_json::from_str(fetched_output.as_str()).expect("output should be json");
        assert_eq!(fetched_value["session_id"], created.id);
        assert!(fetched_value["goal"].is_null());
    }

    #[test]
    fn memory_command_parses_workspace_alias() {
        let cli =
            AgenaCli::parse_from(["agena", "memory", "list", "--cwd", ".", "--format", "json"]);
        let Some(AgenaCommand::Memory(MemoryCommand {
            command: Some(MemorySubcommand::List(args)),
        })) = cli.command
        else {
            unreachable!("expected memory list command after successful parse");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.format, ConfigOutputFormat::Json);
    }

    #[test]
    fn memory_commands_render_list_edit_and_forget() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!("agena-cli-memory-{suffix}"));
        fs::create_dir_all(&workspace).expect("workspace should be created");
        let store = MemoryStore::for_workspace(&workspace);
        store
            .save(crate::memory::NewMemory {
                name: "user_role".to_string(),
                description: "who they are".to_string(),
                memory_type: Some(MemoryType::User),
                body: "user is a data scientist".to_string(),
                index_line: Some("- [User role](user_role.md) — who they are".to_string()),
            })
            .expect("memory should be saved");

        let cli = AgenaCli {
            config: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let list_output = cli
            .render_memory_command(MemoryCommand {
                command: Some(MemorySubcommand::List(MemoryListArgs {
                    workspace: Some(workspace.clone()),
                    format: ConfigOutputFormat::Json,
                })),
            })
            .expect("memory list should render");
        let list: Value = serde_json::from_str(list_output.as_str()).expect("list should be json");
        assert_eq!(list["count"], 1);
        assert_eq!(list["entries"][0]["name"], "user_role");
        assert_eq!(list["entries"][0]["type"], "user");

        let edit_output = cli
            .render_memory_command(MemoryCommand {
                command: Some(MemorySubcommand::Edit(MemoryEditArgs {
                    workspace: Some(workspace.clone()),
                    name: Some("user_role".to_string()),
                })),
            })
            .expect("memory edit should resolve entry path");
        assert!(edit_output.ends_with("user_role.md"));

        let index_output = cli
            .render_memory_command(MemoryCommand {
                command: Some(MemorySubcommand::Edit(MemoryEditArgs {
                    workspace: Some(workspace.clone()),
                    name: None,
                })),
            })
            .expect("memory edit should resolve index path");
        assert!(index_output.ends_with("MEMORY.md"));
        assert!(PathBuf::from(index_output.clone()).exists());

        let forget_output = cli
            .render_memory_command(MemoryCommand {
                command: Some(MemorySubcommand::Forget(MemoryForgetArgs {
                    workspace: Some(workspace.clone()),
                    name: "user_role".to_string(),
                })),
            })
            .expect("memory forget should succeed");
        assert_eq!(forget_output, "forgot memory: user_role");
        assert!(!store.dir().join("user_role.md").exists());
    }

    #[test]
    fn exec_command_parses_json_and_workspace_alias() {
        let cli = AgenaCli::parse_from([
            "agena",
            "exec",
            "--cwd",
            ".",
            "--model",
            "openai/gpt-5",
            "--json",
            "summarize",
        ]);

        let Some(AgenaCommand::Exec(args)) = cli.command else {
            unreachable!("expected exec command after successful parse");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.model.as_deref(), Some("openai/gpt-5"));
        assert!(args.json);
        assert_eq!(args.prompt, "summarize");
    }

    #[test]
    fn exec_helpers_render_last_assistant_text_and_title() {
        let mut session = Session::new(1, 1, "test", Utc::now());
        session
            .messages
            .push(crate::message::Message::prompt_text(Role::User, "hello"));
        session.messages.push(crate::message::Message::prompt_text(
            Role::Assistant,
            "first",
        ));
        session.messages.push(crate::message::Message::prompt_text(
            Role::Assistant,
            "second",
        ));

        assert_eq!(last_assistant_text(&session).as_deref(), Some("second"));
        assert_eq!(title_from_prompt("  hello\nworld  "), "hello world");
    }

    #[test]
    fn completion_command_outputs_fish_completion() {
        let cli = AgenaCli::parse_from(["agena", "completion", "fish"]);
        let Some(AgenaCommand::Completion(args)) = cli.command else {
            unreachable!("expected completion command after successful parse");
        };
        assert_eq!(args.shell, clap_complete::Shell::Fish);

        let output = render_completion_command(args).expect("completion should render");
        assert!(output.contains("complete"));
        assert!(output.contains("agena"));
        assert!(output.contains("exec"));
    }

    #[test]
    fn developer_workflow_commands_parse() {
        let apply = AgenaCli::parse_from([
            "agena",
            "apply",
            "--workspace",
            ".",
            "--json",
            "change.patch",
        ]);
        let Some(AgenaCommand::Apply(args)) = apply.command else {
            unreachable!("expected apply command after successful parse");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.patch_file, PathBuf::from("change.patch"));
        assert!(args.json);

        let review = AgenaCli::parse_from(["agena", "review", "--base", "develop"]);
        let Some(AgenaCommand::Review(args)) = review.command else {
            unreachable!("expected review command after successful parse");
        };
        assert_eq!(args.base, "develop");

        let debug = AgenaCli::parse_from(["agena", "debug", "session", "42", "--json"]);
        let Some(AgenaCommand::Debug(command)) = debug.command else {
            unreachable!("expected debug command after successful parse");
        };
        let DebugSubcommand::Session(args) = command.command;
        assert_eq!(args.session_id, 42);
        assert!(args.json);

        let app_server =
            AgenaCli::parse_from(["agena", "app-server", "--cwd", ".", "--transport", "stdio"]);
        let Some(AgenaCommand::AppServer(args)) = app_server.command else {
            unreachable!("expected app-server command after successful parse");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
        assert_eq!(args.transport, AppServerTransport::Stdio);

        let mcp_server = AgenaCli::parse_from(["agena", "mcp-server", "--cwd", "."]);
        let Some(AgenaCommand::McpServer(args)) = mcp_server.command else {
            unreachable!("expected mcp-server command after successful parse");
        };
        assert_eq!(args.workspace, Some(PathBuf::from(".")));
    }

    #[test]
    fn workflow_commands_parse() {
        let cost = AgenaCli::parse_from(["agena", "cost", "--last", "--format", "json"]);
        let Some(AgenaCommand::Cost(args)) = cost.command else {
            unreachable!("expected cost command after successful parse");
        };
        assert!(args.last);
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let permissions = AgenaCli::parse_from([
            "agena",
            "permissions",
            "list",
            "--search",
            "bash",
            "--format",
            "json",
        ]);
        let Some(AgenaCommand::Permissions(PermissionsArgs {
            command: Some(PermissionsSubcommand::List(args)),
        })) = permissions.command
        else {
            unreachable!("expected permissions list command after successful parse");
        };
        assert_eq!(args.search.as_deref(), Some("bash"));
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let permissions_create = AgenaCli::parse_from([
            "agena",
            "permissions",
            "create",
            "--tool-name",
            "bash",
            "--scope",
            "workspace",
            "--rule-mode",
            "allow",
            "--format",
            "json",
        ]);
        let Some(AgenaCommand::Permissions(PermissionsArgs {
            command: Some(PermissionsSubcommand::Create(args)),
        })) = permissions_create.command
        else {
            unreachable!("expected permissions create command after successful parse");
        };
        assert_eq!(args.tool_name.as_deref(), Some("bash"));
        assert_eq!(args.scope, PermissionScopeArg::Workspace);
        assert_eq!(args.rule_mode, PermissionModeArg::Allow);
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let worktree = AgenaCli::parse_from(["agena", "worktree", "--format", "json"]);
        let Some(AgenaCommand::Worktree(args)) = worktree.command else {
            unreachable!("expected worktree command after successful parse");
        };
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let git = AgenaCli::parse_from(["agena", "git", "--format", "json"]);
        let Some(AgenaCommand::Git(args)) = git.command else {
            unreachable!("expected git command after successful parse");
        };
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let commit = AgenaCli::parse_from(["agena", "commit", "ship it", "--format", "json"]);
        let Some(AgenaCommand::Commit(args)) = commit.command else {
            unreachable!("expected commit command after successful parse");
        };
        assert_eq!(args.message, "ship it");
        assert_eq!(args.format, ConfigOutputFormat::Json);

        let pr = AgenaCli::parse_from([
            "agena",
            "pr",
            "ship feature",
            "--body",
            "details",
            "--base",
            "main",
            "--head",
            "feature",
            "--format",
            "json",
        ]);
        let Some(AgenaCommand::Pr(args)) = pr.command else {
            unreachable!("expected pr command after successful parse");
        };
        assert_eq!(args.title, "ship feature");
        assert_eq!(args.body.as_deref(), Some("details"));
        assert_eq!(args.base.as_deref(), Some("main"));
        assert_eq!(args.head.as_deref(), Some("feature"));
        assert_eq!(args.format, ConfigOutputFormat::Json);
    }

    #[tokio::test]
    async fn cost_command_renders_session_summary() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-cost-{suffix}.db"));
        let config_path = write_temp_config("");
        let cli = AgenaCli {
            config: Some(config_path),
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let runtime = cli.session_runtime().await.expect("runtime should build");
        let manager = runtime
            .session_manager()
            .expect("session manager should be available");
        let created = manager
            .create_session(crate::session::SessionCreateRequest {
                title: "cost session".to_owned(),
                parent_session_id: None,
            })
            .await
            .expect("session should be created");

        let output = cli
            .render_cost_command(CostArgs {
                session_id: Some(created.id),
                last: false,
                format: ConfigOutputFormat::Json,
            })
            .await
            .expect("cost command should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        assert_eq!(value["session"]["id"], created.id);
        assert_eq!(value["summary"]["turns"], 0);
        assert_eq!(value["summary"]["input_tokens"], 0);
        assert_eq!(value["summary"]["by_model"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn permissions_command_renders_rules() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-perm-{suffix}.db"));
        let cli = AgenaCli {
            config: None,
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };
        let storage = StorageConfig {
            database_url: cli.database_url.clone(),
            database_path: cli.database_path.clone(),
        };
        let database_url = storage.resolve_url().expect("db url should resolve");
        StorageConfig::ensure_parent(database_url.as_str()).expect("parent should exist");
        let db =
            tracing_config::connect_database(database_url.as_str(), &cli.resolved_tracing_config())
                .await
                .expect("db should connect");
        init_schema(&db).await.expect("schema should init");
        crate::db::crud::permission_rule::upsert_rule(
            &db,
            &crate::permission::PersistedPermissionRule {
                action_key: "bash".to_string(),
                mode: crate::permission::PermissionMode::Allow,
                scope: crate::permission::PermissionScope::Workspace,
                session_id: None,
                workspace_id: Some(1),
                source: "test".to_string(),
                reason: None,
                operator: None,
                revoked_at_ms: None,
                revoked_reason: None,
                revoked_by: None,
            },
        )
        .await
        .expect("permission rule should upsert");

        let output = cli
            .render_permissions_command(PermissionsArgs {
                command: Some(PermissionsSubcommand::List(PermissionsListArgs {
                    search: Some("bash".to_owned()),
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("permissions command should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        assert_eq!(value["count"], 1);
        assert_eq!(value["rules"][0]["action_key"], "bash");
        assert_eq!(value["rules"][0]["mode"], "allow");
    }

    #[tokio::test]
    async fn permissions_create_and_revoke_commands_round_trip() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let db_path = std::env::temp_dir().join(format!("agena-cli-perm-mutate-{suffix}.db"));
        let cli = AgenaCli {
            config: None,
            overrides: Vec::new(),
            database_url: Some(format!("sqlite://{}?mode=rwc", db_path.display())),
            database_path: None,
            command: None,
        };

        let created = cli
            .render_permissions_command(PermissionsArgs {
                command: Some(PermissionsSubcommand::Create(PermissionsWriteArgs {
                    action_key: None,
                    tool_name: Some("bash".to_string()),
                    qualifier: Some("git status".to_string()),
                    path_access_kind: None,
                    workspace_root: None,
                    target_path: None,
                    network_target: None,
                    network_host: None,
                    network_port: None,
                    scope: PermissionScopeArg::Workspace,
                    session_id: None,
                    rule_mode: PermissionModeArg::Allow,
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("permissions create should render");
        let created: Value =
            serde_json::from_str(created.as_str()).expect("create output should be json");
        let rule_id = created["id"].as_i64().expect("created rule should have id");
        assert_eq!(created["scope"], "workspace");
        assert_eq!(created["mode"], "allow");

        let revoked = cli
            .render_permissions_command(PermissionsArgs {
                command: Some(PermissionsSubcommand::Revoke(PermissionsRevokeArgs {
                    rule_id,
                    reason: Some("test revoke".to_string()),
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("permissions revoke should render");
        let revoked: Value =
            serde_json::from_str(revoked.as_str()).expect("revoke output should be json");
        assert_eq!(revoked["id"], rule_id);
        assert_eq!(revoked["revoked_reason"], "test revoke");
        assert!(revoked["revoked_at"].is_string());
    }

    #[test]
    fn worktree_output_serializes_registry_shape() {
        let output = WorktreeOutput {
            workspace_root: "/tmp/workspace".to_owned(),
            active: vec![ActiveWorktreeOutput {
                session_id: 7,
                path: "/tmp/workspace/.agena/worktrees/managed".to_owned(),
                branch: "agena/managed".to_owned(),
                created_here: true,
            }],
            managed: vec![ManagedWorktreeOutput {
                path: "/tmp/workspace/.agena/worktrees/managed".to_owned(),
                session_id: Some(7),
                branch: Some("agena/managed".to_owned()),
                registered_with_git: false,
                stale: false,
            }],
        };

        let rendered = render_serialized(ConfigOutputFormat::Json, &output)
            .expect("worktree output should serialize");
        let value: Value = serde_json::from_str(rendered.as_str()).expect("output should be json");
        assert_eq!(value["active"][0]["session_id"], 7);
        assert_eq!(value["active"][0]["branch"], "agena/managed");
        assert_eq!(value["managed"][0]["branch"], "agena/managed");
    }

    #[test]
    fn apply_command_invokes_apply_patch_tool() {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let workspace = std::env::temp_dir().join(format!("agena-cli-apply-{suffix}"));
        fs::create_dir_all(&workspace).expect("workspace should be created");
        let patch_file = workspace.join("change.patch");
        fs::write(
            &patch_file,
            "*** Begin Patch\n*** Add File: added.txt\n+created\n*** End Patch",
        )
        .expect("patch file should be written");
        let cli = AgenaCli {
            config: None,
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_apply_command(ApplyArgs {
                workspace: Some(workspace.clone()),
                json: true,
                patch_file,
            })
            .expect("apply should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["patch"]["files"][0]["path"], "added.txt");
        assert_eq!(
            fs::read_to_string(workspace.join("added.txt")).expect("file should be created"),
            "created\n"
        );
    }

    #[test]
    fn debug_session_plain_output_includes_messages() {
        let output = DebugSessionOutput {
            session: SessionDetail {
                id: 7,
                parent_id: None,
                workspace_id: 1,
                title: "debug me".to_owned(),
                version: 1,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                message_count: 1,
                status: SessionRuntimeStatus::Idle,
                latest_event_seq: Some(9),
            },
            messages: vec![DebugMessageOutput {
                id: 3,
                role: Role::Assistant,
                state: crate::message::MessageStatus::Completed,
                text: "done".to_owned(),
            }],
        };

        let rendered = format_debug_session_output(&output);
        assert!(rendered.contains("session 7: debug me"));
        assert!(rendered.contains("[assistant #3 completed]"));
        assert!(rendered.contains("done"));
    }

    #[test]
    fn format_plugin_logs_output_renders_entries_and_fields() {
        let output = PluginLogsOutput {
            plugin_id: "ops.plugin".to_string(),
            entries: vec![crate::plugin::PluginLogEntry {
                seq: 9,
                timestamp_ms: 1_700_000_000_123,
                plugin_id: "ops.plugin".to_string(),
                level: "warn".to_string(),
                source: "stderr".to_string(),
                message: "request failed".to_string(),
                fields: json!({"attempt": 2}),
            }],
        };

        let rendered = format_plugin_logs_output(&output);

        assert!(rendered.contains("#9 warn stderr request failed"));
        assert!(rendered.contains("{\"attempt\":2}"));
        assert!(rendered.starts_with("[2023-"));
    }

    #[test]
    fn format_plugin_logs_output_handles_empty_logs() {
        let output = PluginLogsOutput {
            plugin_id: "ops.plugin".to_string(),
            entries: Vec::new(),
        };

        assert_eq!(
            format_plugin_logs_output(&output),
            "plugin ops.plugin has no retained logs"
        );
    }

    #[test]
    fn plugin_status_output_serializes_failed_state() {
        let output = PluginStatusOutput {
            entries: vec![crate::plugin::status::PluginStatus {
                plugin_id: "broken.plugin".to_string(),
                kind: "stdio",
                state: crate::plugin::status::PluginRunState::Failed,
                pid: None,
                restart_count: 2,
                last_exit_code: Some(23),
                last_restart_at_ms: Some(1_700_000_000_000),
                last_error: Some("spawn failed".to_string()),
            }],
        };

        let rendered = render_serialized(ConfigOutputFormat::Json, &output)
            .expect("plugin status should serialize");
        let value: Value = serde_json::from_str(rendered.as_str()).expect("output should be json");

        assert_eq!(value["entries"][0]["plugin_id"], "broken.plugin");
        assert_eq!(value["entries"][0]["state"], "failed");
        assert_eq!(value["entries"][0]["kind"], "stdio");
    }

    #[tokio::test]
    async fn provider_capabilities_command_renders_resolved_provider_capabilities() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_adapter = "openai"
default_model = "gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true

[providers.openai.adapters.openai.models."gpt-5"]
input = { unsupported = ["image"] }
"#,
        );
        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env);
        let cli = AgenaCli {
            config: Some(path),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: Some(AgenaCommand::Provider(ProviderCommand {
                command: Some(ProviderSubcommand::Capabilities(ProviderCapabilitiesArgs {
                    target: "openai".to_owned(),
                    model: None,
                    format: ConfigOutputFormat::Json,
                })),
            })),
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::Capabilities(ProviderCapabilitiesArgs {
                        target: "openai/gpt-5".to_owned(),
                        model: None,
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider capabilities command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["provider_id"], "openai");
        assert_eq!(value["model"], "gpt-5");
        assert_eq!(
            value["model_ref"],
            "provider=openai adapter=openai model=gpt-5"
        );
        assert_eq!(value["capabilities"]["image_input"], "unsupported");
        assert_eq!(value["capabilities"]["document_input"], "supported");
    }

    #[tokio::test]
    async fn provider_models_command_renders_static_gitlab_models() {
        let path = write_temp_config(
            r#"
[providers.gitlab]
default_adapter = "gitlab"
default_model = "claude-sonnet-4-5"

[providers.gitlab.auth]
mode = "api"
base_url = "https://gitlab.com/api/v4"
api_key = "glpat-test"

[providers.gitlab.adapters.gitlab]
enabled = true

[providers.gitlab.adapters.gitlab.models."claude-sonnet-4-5"]
"#,
        );
        let loader = ConfigLoader::new(TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        });
        let cli = AgenaCli {
            config: Some(path),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::Models(ProviderModelsArgs {
                        provider_id: "gitlab".to_owned(),
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider models command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["provider_id"], "gitlab");
        let model = value["models"]
            .as_array()
            .expect("models should be an array")
            .iter()
            .find(|model| model["id"] == "claude-sonnet-4-5")
            .expect("gitlab claude model should be listed");
        assert_eq!(model["adapter_id"], "gitlab");
        assert_eq!(model["capabilities"]["tool_calling"], "supported");
    }

    #[tokio::test]
    async fn provider_list_command_includes_provider_default_models() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_adapter = "openai"
default_model = "gpt-5"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key_env = "OPENAI_API_KEY"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let env = TestEnvironment {
            vars: BTreeMap::from([("OPENAI_API_KEY".to_owned(), "sk-test".to_owned())]),
        };
        let loader = ConfigLoader::new(env);
        let cli = AgenaCli {
            config: Some(path),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_provider_command(
                &loader,
                ProviderCommand {
                    command: Some(ProviderSubcommand::List(ProviderListArgs {
                        format: ConfigOutputFormat::Json,
                    })),
                },
            )
            .await
            .expect("provider list command should succeed");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");
        let providers = value["providers"]
            .as_array()
            .expect("providers should be an array");

        assert!(providers.iter().any(|item| {
            item["provider_id"] == "openai"
                && item["default_adapter"] == "openai"
                && item["default_model"] == "gpt-5"
        }));
    }

    #[tokio::test]
    async fn agents_list_renders_runtime_inventory() {
        let path = write_temp_config(
            r#"
[providers.openai]
default_model = "openai/gpt-5.4"

[providers.openai.auth]
mode = "api"
base_url = "https://api.openai.com/v1"
api_key = "dummy"

[providers.openai.adapters.openai]
enabled = true
"#,
        );
        let cli = AgenaCli {
            config: Some(path),
            overrides: Vec::new(),
            database_url: None,
            database_path: None,
            command: None,
        };

        let output = cli
            .render_agents_command(AgentsCommand {
                command: Some(AgentsSubcommand::List(AgentsListArgs {
                    format: ConfigOutputFormat::Json,
                })),
            })
            .await
            .expect("agents list should render");
        let value: Value = serde_json::from_str(output.as_str()).expect("output should be json");

        assert_eq!(value["default_agent"], "build");
        assert!(value["total_count"].as_u64().unwrap_or(0) >= 1);
        assert!(
            value["agents"]
                .as_array()
                .expect("agents should be array")
                .iter()
                .any(|agent| agent["name"] == "build")
        );
        assert!(
            value["agents"]
                .as_array()
                .expect("agents should be array")
                .iter()
                .any(|agent| agent["name"] == "scout")
        );
    }

    #[test]
    fn capability_support_json_serialization_uses_snake_case_strings() {
        let encoded =
            serde_json::to_string(&CapabilitySupport::Unsupported).expect("encoding should work");
        assert_eq!(encoded, "\"unsupported\"");
    }

    fn write_temp_config(content: &str) -> PathBuf {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should move forward")
            .as_nanos();
        let path = std::env::temp_dir().join(format!("agena-cli-{suffix}.toml"));
        fs::write(&path, content).expect("temp config should be written");
        path
    }
}
