pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

pub(crate) fn tool_value_name(name: &str) -> String {
    let trimmed = name.trim();
    let mut parts = trimmed.splitn(3, '.');
    let _namespace = parts.next();
    let plugin = parts.next();
    let tool = parts.next();
    match (plugin, tool) {
        (Some(plugin), Some(tool)) if plugin == tool => plugin.to_string(),
        (Some(plugin), Some(tool)) => format!("{plugin}.{tool}"),
        _ => trimmed.to_string(),
    }
}

pub(crate) fn is_model_tools_gateway(tool: &RegisteredTool) -> bool {
    tool.plugin_name() == "tools"
        && matches!(
            tool.tool_name(),
            "list" | "search" | "help" | "tags" | "call"
        )
}

pub(crate) fn gateway_help_tool_name() -> &'static str {
    MODEL_TOOLS_HELP
}

pub(crate) fn gateway_call_tool_name() -> &'static str {
    MODEL_TOOLS_CALL
}

pub(super) fn gateway_model_tool_name(tool: &RegisteredTool) -> Option<&'static str> {
    if tool.plugin_name() != "tools" {
        return None;
    }
    match tool.tool_name() {
        "list" => Some(MODEL_TOOLS_LIST),
        "search" => Some(MODEL_TOOLS_SEARCH),
        "help" => Some(MODEL_TOOLS_HELP),
        "tags" => Some(MODEL_TOOLS_TAGS),
        "call" => Some(MODEL_TOOLS_CALL),
        _ => None,
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelToolSpec {
    pub canonical_name: String,
    pub model_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub strict: bool,
    pub execution: ModelToolExecution,
    pub definition_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelToolExecution {
    Local,
}

impl ModelToolSpec {
    pub fn from_registered_tool(tool: &RegisteredTool) -> Self {
        let canonical_name = tool.model_name();
        let model_name = if let Some(gateway_name) = gateway_model_tool_name(tool) {
            gateway_name.to_string()
        } else {
            tool_value_name(canonical_name.as_str())
        };
        Self {
            canonical_name,
            model_name,
            description: compact_tool_description(tool),
            input_schema: tool.input_schema(),
            output_schema: tool.output_schema(),
            strict: tool.definition.contract.strict,
            execution: ModelToolExecution::Local,
            definition_identity: tool.definition_identity(),
        }
    }
}

pub fn model_tool_specs(tools: &[RegisteredTool]) -> Vec<ModelToolSpec> {
    tools
        .iter()
        .map(ModelToolSpec::from_registered_tool)
        .collect()
}

pub(crate) fn suggest_tool_names<I, T>(requested: &str, candidates: I, limit: usize) -> Vec<String>
where
    I: IntoIterator<Item = T>,
    T: AsRef<str>,
{
    let requested = requested.trim();
    let requested_lower = requested.to_ascii_lowercase();
    let mut ranked: Vec<(usize, String)> = Vec::new();

    for candidate in candidates {
        let name = candidate.as_ref().trim();
        if name.is_empty() {
            continue;
        }
        let score = normalized_tool_name_distance(requested, name);
        if score == 0 {
            continue;
        }
        let name_lower = name.to_ascii_lowercase();
        if score <= 4
            || name_lower.contains(requested_lower.as_str())
            || requested_lower.contains(name_lower.as_str())
        {
            ranked.push((score, name.to_string()));
        }
    }

    ranked.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));

    let mut suggestions = Vec::new();
    for (_, name) in ranked {
        if !suggestions.contains(&name) {
            suggestions.push(name);
        }
        if suggestions.len() >= limit {
            break;
        }
    }
    suggestions
}

pub(crate) fn unknown_tool_message(requested: &str, suggestions: &[String]) -> String {
    if suggestions.is_empty() {
        return format!("unknown tool '{requested}'");
    }
    format!(
        "unknown tool '{requested}'. Did you mean {}?",
        suggestions
            .iter()
            .map(|tool| format!("`{tool}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

pub(crate) fn unknown_tool_hint(requested: &str, suggestions: Vec<String>) -> ToolError {
    let suggestion_text = unknown_tool_message(requested, &suggestions);
    ToolError::UnknownToolHint {
        tool: requested.to_string(),
        suggestions,
        suggestion_text,
    }
}

pub(super) fn normalized_tool_name_distance(left: &str, right: &str) -> usize {
    let left = left.trim().to_ascii_lowercase();
    let right = right.trim().to_ascii_lowercase();
    if left == right {
        return 0;
    }
    let left_chars = left.chars().collect::<Vec<_>>();
    let right_chars = right.chars().collect::<Vec<_>>();
    let mut prev = (0..=right_chars.len()).collect::<Vec<_>>();
    let mut curr = vec![0; right_chars.len() + 1];
    for (i, left_ch) in left_chars.iter().enumerate() {
        curr[0] = i + 1;
        for (j, right_ch) in right_chars.iter().enumerate() {
            let replace = prev[j] + usize::from(left_ch != right_ch);
            let insert = curr[j] + 1;
            let delete = prev[j + 1] + 1;
            curr[j + 1] = replace.min(insert.min(delete));
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[right_chars.len()]
}

pub(crate) fn tool_matches_model_name(registered_tool: &RegisteredTool, name: &str) -> bool {
    let trimmed = name.trim();
    registered_tool.model_name() == trimmed
        || tool_value_name(registered_tool.model_name().as_str()) == trimmed
        || gateway_model_tool_name(registered_tool)
            .is_some_and(|gateway_name| gateway_name == trimmed)
}

pub(super) fn expand_registered_tool_for_model(
    base: &RegisteredTool,
    out: &mut Vec<RegisteredTool>,
) {
    out.push(base.clone());
}

pub fn new_skills_plugin() -> impl crate::plugin::sdk::Plugin {
    skills::SkillsPlugin::new()
}

pub fn lsp_plugin_id() -> &'static str {
    provided_lsp::LSP_PLUGIN_ID
}

pub fn new_lsp_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_lsp::LspPlugin::new()
}

pub fn cron_plugin_id() -> &'static str {
    provided_cron::CRON_PLUGIN_ID
}

pub fn new_cron_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_cron::CronPlugin::new()
}

pub fn code_plugin_id() -> &'static str {
    provided_code::CODE_PLUGIN_ID
}

pub fn new_code_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_code::new_plugin()
}

pub fn fs_plugin_id() -> &'static str {
    provided_fs::FS_PLUGIN_ID
}

pub fn new_fs_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_fs::new_plugin()
}

pub fn settings_plugin_id() -> &'static str {
    provided_settings::SETTINGS_PLUGIN_ID
}

pub fn new_settings_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_settings::SettingsPlugin::new()
}

pub fn process_plugin_id() -> &'static str {
    provided_process::PROCESS_PLUGIN_ID
}

pub fn new_process_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_process::new_plugin()
}

pub fn catalog_plugin_id() -> &'static str {
    provided_catalog::TOOLS_PLUGIN_ID
}

pub fn new_catalog_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_catalog::ToolsPlugin::new()
}

pub fn agent_plugin_id() -> &'static str {
    provided_agent::AGENT_PLUGIN_ID
}

pub fn new_agent_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_agent::AgentPlugin::new()
}

pub fn session_plugin_id() -> &'static str {
    provided_session::SESSION_PLUGIN_ID
}

pub fn new_session_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_session::SessionPlugin::new()
}

pub fn interaction_plugin_id() -> &'static str {
    provided_interaction::INTERACTION_PLUGIN_ID
}

pub fn new_interaction_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_interaction::InteractionPlugin::new()
}

pub fn plan_plugin_id() -> &'static str {
    provided_planning::PLAN_PLUGIN_ID
}

pub fn new_plan_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_planning::PlanPlugin::new()
}

pub fn tasks_plugin_id() -> &'static str {
    provided_tasks::TASKS_PLUGIN_ID
}

pub fn new_tasks_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_tasks::TasksPlugin::new()
}

pub fn snapshot_plugin_id() -> &'static str {
    provided_repo::SNAPSHOT_PLUGIN_ID
}

pub fn new_snapshot_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_repo::SnapshotPlugin::new()
}

#[cfg(feature = "schema-lab")]
pub const fn schema_lab_builtin_enabled() -> bool {
    true
}

#[cfg(not(feature = "schema-lab"))]
pub const fn schema_lab_builtin_enabled() -> bool {
    false
}

pub fn schema_lab_plugin_id() -> &'static str {
    provided_schema_lab::SCHEMA_LAB_PLUGIN_ID
}

pub fn new_schema_lab_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_schema_lab::SchemaLabPlugin::new()
}

pub fn default_tool_host(workspace_root: impl Into<PathBuf>) -> Result<Arc<PluginHost>, String> {
    let workspace_root = workspace_root.into();
    let config =
        crate::plugins::sources::resolve_plugin_config(crate::plugin::PluginsConfig::default());
    mcp::block_on(async move {
        let mcp_config =
            mcp::config_from_plugins(&config).map_err(crate::plugin::HostError::Config)?;
        let mcp_manager = mcp::build_manager(&mcp_config).await;
        PluginHost::new(PluginHostBuildConfig {
            static_plugins: crate::plugins::sources::static_plugin_registrations(Some(mcp_manager)),
            config,
            workspace_root,
            agena_version: env!("CARGO_PKG_VERSION").to_string(),
            callback_base_url: None,
            host_client: None,
            previous: None,
            previous_plugins: std::collections::HashMap::new(),
        })
        .await
    })
    .map_err(|err| err.to_string())
}
/// Stable id used to register configured MCP servers as plugin tools.
pub fn mcp_plugin_id() -> &'static str {
    mcp::MCP_PLUGIN_ID
}

/// Construct the in-process plugin that exposes configured MCP server tools.
pub fn new_mcp_plugin(
    manager: Arc<agena_mcp_client::McpConnectionManager>,
) -> impl crate::plugin::sdk::Plugin {
    mcp::McpPlugin::new(manager)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolPermissionCheck {
    pub action: PermissionAction,
    pub decision: PermissionDecision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedToolInvocation {
    pub invocation: ToolInvocation,
    pub title_override: Option<String>,
    pub metadata: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShellCommand {
    pub command: String,
    pub cwd: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PermissionEnforcementMode {
    Enforced,
    Bypassed,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ToolRuntimeContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub session_context: Option<crate::session::SessionExecutionContext>,
    pub prepared_shell_command: Option<PreparedShellCommand>,
}

pub(super) static SYNTHETIC_TOOL_CALL_ID: AtomicI64 = AtomicI64::new(-1);

pub struct StreamingToolExecution {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<crate::plugin::sdk::ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolInvocationExecution, ToolError>>,
    pub(super) _executor_guard: Option<in_process_router::ExecutorContextGuard>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("tool execution cancelled")]
    Cancelled,
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("user input required")]
    UserInputRequired(Box<AskUserToolInput>),
    #[error("invalid patch: {0}")]
    InvalidPatch(String),
    #[error("invalid tool input: {0}")]
    InvalidInput(String),
    #[error("invalid glob pattern: {0}")]
    InvalidGlobPattern(#[from] globset::Error),
    #[error("invalid regex pattern: {0}")]
    InvalidRegexPattern(#[from] regex::Error),
    #[error("shell error: {0}")]
    Shell(#[from] ShellError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("plugin error: {0}")]
    Plugin(String),
    #[error("unknown tool: {tool}")]
    UnknownTool { tool: String },
    #[error("{suggestion_text}")]
    UnknownToolHint {
        tool: String,
        suggestions: Vec<String>,
        suggestion_text: String,
    },
    #[error("stale tool call: {tool}")]
    StaleToolCall { tool: String },
    #[error("unsupported tool invocation in executor: {0}")]
    UnsupportedInvocation(String),
}

pub(super) fn present_registered_tool(
    mut registered_tool: RegisteredTool,
    presentation: &crate::plugin::ToolPresentationConfig,
) -> RegisteredTool {
    apply_registered_tool_presentation_mode(&mut registered_tool, presentation);
    if registered_tool.definition.preferred_description_mode()
        == Some(crate::plugin::ToolDescriptionMode::Brief)
    {
        registered_tool.definition.docs.help = None;
    }
    registered_tool
}

pub(super) fn present_registered_tool_detailed(
    mut registered_tool: RegisteredTool,
    presentation: &crate::plugin::ToolPresentationConfig,
) -> RegisteredTool {
    apply_registered_tool_presentation_mode(&mut registered_tool, presentation);
    registered_tool
}

fn apply_registered_tool_presentation_mode(
    registered_tool: &mut RegisteredTool,
    presentation: &crate::plugin::ToolPresentationConfig,
) {
    let mode = presentation.mode_for(
        registered_tool.plugin_key(),
        registered_tool.tool_key(),
        registered_tool.definition.preferred_description_mode(),
    );
    registered_tool.definition.display.description_mode = Some(mode);
}

pub(super) fn compact_tool_description(registered_tool: &RegisteredTool) -> String {
    if is_model_tools_gateway(registered_tool) {
        return "Discover tools, inspect help, and invoke internal tools through the gateway."
            .to_string();
    }
    let summary = tool_summary_sentence(registered_tool);
    format!(
        "{summary} Use `{}` for `{}`.",
        gateway_help_tool_name(),
        tool_value_name(registered_tool.model_name().as_str())
    )
}

pub(super) fn tool_summary_sentence(registered_tool: &RegisteredTool) -> String {
    let summary = tool_summary(registered_tool);
    if matches!(summary.chars().last(), Some('.' | '!' | '?')) {
        return summary;
    }
    format!("{summary}.")
}

pub(super) fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    if is_model_tools_gateway(registered_tool) {
        return "Tool gateway.".to_string();
    }
    format!(
        "Tool `{}`.",
        tool_value_name(registered_tool.model_name().as_str())
    )
}

pub(super) fn render_model_tool_index_entry(tool: &RegisteredTool) -> String {
    let summary = match tool.definition.preferred_description_mode() {
        Some(crate::plugin::ToolDescriptionMode::Detailed) => tool
            .help_text()
            .map(str::trim)
            .filter(|help| !help.is_empty())
            .map(|help| format!("{} {help}", tool_summary_sentence(tool)))
            .unwrap_or_else(|| tool_summary(tool)),
        Some(crate::plugin::ToolDescriptionMode::Brief) | None => tool_summary(tool),
    };
    format!(
        "- {}: {}",
        tool_value_name(tool.model_name().as_str()),
        summary.trim()
    )
}

#[derive(Clone)]
pub struct ToolExecutor {
    pub(super) workspace_root: PathBuf,
    pub(super) agent: Agent,
    pub(super) model_id: Option<String>,
    pub(super) subagent_registry: crate::agents::SubagentRegistry,
    pub(super) monitor_registry: Option<Arc<dyn MonitorService>>,
    pub(super) truncator: ToolOutputTruncator,
    pub(super) plugins: Arc<PluginHost>,
    pub(super) snapshot_registry: Option<snapshot::SnapshotRegistry>,
    pub(super) scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    pub(super) lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    pub(super) permission_mode: PermissionEnforcementMode,
    pub(super) tool_presentation: crate::plugin::ToolPresentationConfig,
    pub(super) cancellation_token: Option<tokio_util::sync::CancellationToken>,
}
use super::{
    Agent, Arc, AskUserToolInput, AtomicI64, Error, MODEL_TOOLS_CALL, MODEL_TOOLS_HELP,
    MODEL_TOOLS_LIST, MODEL_TOOLS_SEARCH, MODEL_TOOLS_TAGS, MonitorService, PathBuf,
    PermissionAction, PermissionDecision, PluginHost, PluginHostBuildConfig, RegisteredTool,
    ShellError, ToolInvocation, ToolInvocationExecution, ToolOutputTruncator, in_process_router,
    mcp, provided_agent, provided_catalog, provided_code, provided_cron, provided_fs,
    provided_interaction, provided_lsp, provided_planning, provided_process, provided_repo,
    provided_schema_lab, provided_session, provided_settings, provided_tasks, skills, snapshot,
};
