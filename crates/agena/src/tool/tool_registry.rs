pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

/// A registered tool that may be listed, described, or run through the Tool
/// API. The five Tool API handlers can never inhabit this type.
#[derive(Debug, Clone, PartialEq)]
pub struct ExecutionTool {
    registered: RegisteredTool,
}

impl ExecutionTool {
    pub fn from_registered_tool(registered: RegisteredTool) -> Option<Self> {
        (!is_tool_api_handler(&registered)).then_some(Self { registered })
    }

    pub fn registered(&self) -> &RegisteredTool {
        &self.registered
    }

    pub fn into_registered(self) -> RegisteredTool {
        self.registered
    }
}

impl std::ops::Deref for ExecutionTool {
    type Target = RegisteredTool;

    fn deref(&self) -> &Self::Target {
        self.registered()
    }
}

/// Convert an internal tool key such as `agena.session.rename` into the compact
/// execution-tool name used by `tools_help` and `tools_call`, such as
/// `session.rename`.
pub(crate) fn compact_tool_call_name(name: &str) -> String {
    let mut parts = name.splitn(3, '.');
    let _namespace = parts.next();
    let plugin = parts.next();
    let tool = parts.next();
    match (plugin, tool) {
        (Some(plugin), Some(tool)) if plugin == tool => plugin.to_string(),
        (Some(plugin), Some(tool)) => format!("{plugin}.{tool}"),
        _ => name.to_string(),
    }
}

/// Produce the callable name for each execution tool. Compact `plugin.tool`
/// names are preferred, but collisions retain the full
/// `namespace.plugin.tool` key so every advertised name resolves to exactly
/// one registered tool.
pub(crate) fn execution_tool_names(tools: &[ExecutionTool]) -> Vec<String> {
    let mut compact_name_counts = std::collections::HashMap::<String, usize>::new();
    for tool in tools {
        *compact_name_counts
            .entry(compact_tool_call_name(tool.canonical_name().as_str()))
            .or_default() += 1;
    }
    tools
        .iter()
        .map(|tool| {
            let canonical = tool.canonical_name();
            let compact = compact_tool_call_name(canonical.as_str());
            if compact_name_counts
                .get(compact.as_str())
                .copied()
                .unwrap_or_default()
                > 1
            {
                canonical
            } else {
                compact
            }
        })
        .collect()
}

pub(crate) fn is_tool_api_handler(tool: &RegisteredTool) -> bool {
    ToolApiFunction::from_handler_parts(tool.namespace(), tool.plugin_name(), tool.tool_name())
        .is_some()
}

pub(crate) fn tools_help_function_name() -> &'static str {
    ToolApiFunction::Help.function_name()
}

pub(crate) fn tools_call_function_name() -> &'static str {
    ToolApiFunction::Call.function_name()
}

pub(super) fn tool_api_function_name(tool: &RegisteredTool) -> Option<&'static str> {
    ToolApiFunction::from_handler_parts(tool.namespace(), tool.plugin_name(), tool.tool_name())
        .map(ToolApiFunction::function_name)
}

/// A registry handler proven to implement one of Agena's five provider-facing
/// Tool API functions. Execution tools cannot inhabit this type, so a
/// `CompletionRequest` cannot accidentally advertise them as functions.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct ToolApiBinding {
    function: ToolApiFunction,
    handler: RegisteredTool,
}

impl<'de> serde::Deserialize<'de> for ToolApiBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct SerializedToolApiBinding {
            function: ToolApiFunction,
            handler: RegisteredTool,
        }

        let serialized = SerializedToolApiBinding::deserialize(deserializer)?;
        let binding = Self::from_registered_tool(serialized.handler)
            .ok_or_else(|| serde::de::Error::custom("handler is not an Agena Tool API function"))?;
        if binding.function != serialized.function {
            return Err(serde::de::Error::custom(format!(
                "provider function `{}` does not match handler `{}`",
                serialized.function.function_name(),
                binding.handler.canonical_name()
            )));
        }
        Ok(binding)
    }
}

impl ToolApiBinding {
    pub fn from_registered_tool(handler: RegisteredTool) -> Option<Self> {
        let function = ToolApiFunction::from_handler_parts(
            handler.namespace(),
            handler.plugin_name(),
            handler.tool_name(),
        )?;
        Some(Self { function, handler })
    }

    pub const fn function(&self) -> ToolApiFunction {
        self.function
    }

    pub fn function_name(&self) -> &'static str {
        self.function.function_name()
    }

    pub fn handler_key(&self) -> String {
        self.handler.canonical_name()
    }

    pub fn handler(&self) -> &RegisteredTool {
        &self.handler
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ToolApiDefinition {
    pub handler_key: String,
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub strict: bool,
    pub definition_identity: String,
}

impl ToolApiDefinition {
    pub fn from_binding(binding: &ToolApiBinding) -> Self {
        let handler = binding.handler();
        Self {
            handler_key: binding.handler_key(),
            name: binding.function_name().to_owned(),
            description: tool_api_description(binding.function()).to_owned(),
            input_schema: handler.input_schema(),
            output_schema: handler.output_schema(),
            strict: handler.definition.contract.strict,
            definition_identity: handler.definition_identity(),
        }
    }
}

pub fn tool_api_definitions(tools: &[ToolApiBinding]) -> Vec<ToolApiDefinition> {
    tools.iter().map(ToolApiDefinition::from_binding).collect()
}

fn tool_api_description(function: ToolApiFunction) -> &'static str {
    match function {
        ToolApiFunction::List => {
            "List the Agena execution tools available in this session. Each result contains a tool name such as `fs.read`; use that exact name in `tools_help.tool` or `tools_call.tool`. Execution-tool names are not function names. Supports pagination and tag filters."
        }
        ToolApiFunction::Search => {
            "Search the Agena execution tools available in this session by capability, name, summary, or tag. Use a returned tool name in `tools_help.tool` or `tools_call.tool`; never use an execution-tool name as a function name."
        }
        ToolApiFunction::Help => {
            "Get the input schema, examples, and usage notes for one Agena execution tool. Set `tool` to an exact name returned by `tools_list` or `tools_search`. This Tool API function describes the tool but does not run or authorize it; the returned help is reusable."
        }
        ToolApiFunction::Tags => {
            "List tags used by the Agena execution tools available in this session. Use returned tags to filter `tools_list` or `tools_search`. This Tool API function does not run an execution tool."
        }
        ToolApiFunction::Call => {
            "Run one Agena execution tool. Set `tool` to the exact tool name returned by `tools_list` or `tools_search`, and set `input` to the tool's complete argument object, using `tools_help` when its schema is unfamiliar. Example: {\"tool\":\"fs.read\",\"input\":{\"path\":\"Cargo.toml\"}}. The function name is always `tools_call`; never use a tool name such as `fs.read` as the function name, and never put a Tool API function name in `tool`. Preserve every required input field. If validation fails, the error includes the tool's complete help and schema; correct the input and retry `tools_call` directly."
        }
    }
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

pub(crate) fn registered_tool_matches_name(registered_tool: &RegisteredTool, name: &str) -> bool {
    registered_tool.canonical_name() == name
        || compact_tool_call_name(registered_tool.canonical_name().as_str()) == name
        || tool_api_function_name(registered_tool)
            .is_some_and(|function_name| function_name == name)
}

pub(crate) fn unique_registered_tool_match(
    tools: impl IntoIterator<Item = RegisteredTool>,
    name: &str,
) -> Option<RegisteredTool> {
    let tools = tools.into_iter().collect::<Vec<_>>();
    if let Some(exact) = tools.iter().find(|tool| tool.canonical_name() == name) {
        return Some(exact.clone());
    }
    let mut aliases = tools
        .into_iter()
        .filter(|tool| registered_tool_matches_name(tool, name));
    let first = aliases.next()?;
    aliases.next().is_none().then_some(first)
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

pub fn shell_plugin_id() -> &'static str {
    provided_shell::SHELL_PLUGIN_ID
}

pub fn new_shell_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_shell::new_plugin()
}

pub fn tool_api_plugin_id() -> &'static str {
    provided_tool_api::TOOL_API_PLUGIN_ID
}

pub fn new_tool_api_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_tool_api::ToolApiPlugin::new()
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

pub(super) fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    if is_tool_api_handler(registered_tool) {
        return "Tool API function.".to_string();
    }
    format!(
        "Tool `{}`.",
        compact_tool_call_name(registered_tool.canonical_name().as_str())
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
    Agent, Arc, AskUserToolInput, AtomicI64, Error, MonitorService, PathBuf, PermissionAction,
    PermissionDecision, PluginHost, PluginHostBuildConfig, RegisteredTool, ShellError,
    ToolInvocation, ToolInvocationExecution, ToolOutputTruncator, in_process_router, mcp,
    provided_agent, provided_code, provided_cron, provided_fs, provided_interaction, provided_lsp,
    provided_planning, provided_repo, provided_schema_lab, provided_session, provided_settings,
    provided_shell, provided_tasks, provided_tool_api, skills, snapshot,
};
use crate::tool_api::ToolApiFunction;

#[cfg(test)]
mod tool_api_binding_tests {
    use super::{
        ExecutionTool, ToolApiBinding, ToolApiDefinition, execution_tool_names,
        unique_registered_tool_match,
    };
    use crate::plugin::registry::RegisteredTool;
    use crate::plugin::sdk::{PluginKey, ToolDefinition};
    use crate::tool_api::ToolApiFunction;

    fn registered_tool(plugin: &str, name: &str) -> RegisteredTool {
        namespaced_registered_tool("agena", plugin, name)
    }

    fn namespaced_registered_tool(namespace: &str, plugin: &str, name: &str) -> RegisteredTool {
        RegisteredTool::new(
            PluginKey::new(namespace, plugin).expect("plugin key"),
            ToolDefinition {
                name: name.to_owned(),
                contract: Default::default(),
                model: Default::default(),
                docs: Default::default(),
                runtime: Default::default(),
                permissions: Default::default(),
                display: Default::default(),
                capabilities: Vec::new(),
            },
        )
        .expect("registered tool")
    }

    #[test]
    fn only_tool_api_handlers_can_become_tool_api_bindings() {
        let session_rename = registered_tool("session", "rename");
        assert!(ToolApiBinding::from_registered_tool(session_rename.clone()).is_none());
        assert!(ExecutionTool::from_registered_tool(session_rename).is_some());
        assert!(
            ToolApiBinding::from_registered_tool(namespaced_registered_tool(
                "third_party",
                "tools",
                "help"
            ))
            .is_none()
        );

        let tools_help = registered_tool("tools", "help");
        assert!(ExecutionTool::from_registered_tool(tools_help.clone()).is_none());
        let binding = ToolApiBinding::from_registered_tool(tools_help).expect("Tool API handler");
        assert_eq!(binding.function(), ToolApiFunction::Help);
        assert_eq!(binding.function_name(), "tools_help");
        assert_eq!(binding.handler_key(), "agena.tools.help");

        let spec = ToolApiDefinition::from_binding(&binding);
        assert_eq!(spec.name, "tools_help");
        assert_eq!(spec.handler_key, "agena.tools.help");
    }

    #[test]
    fn deserialization_cannot_forge_a_provider_binding() {
        let valid = ToolApiBinding::from_registered_tool(registered_tool("tools", "help"))
            .expect("Tool API handler");
        let valid_value = serde_json::to_value(valid).expect("serialize Tool API binding");

        let mut mismatched_function = valid_value.clone();
        mismatched_function["function"] = serde_json::Value::String("tools_call".to_owned());

        let error = serde_json::from_value::<ToolApiBinding>(mismatched_function)
            .expect_err("mismatched function/handler must fail");
        assert!(error.to_string().contains("does not match handler"));

        let mut execution_tool = valid_value;
        execution_tool["handler"] =
            serde_json::to_value(registered_tool("session", "rename")).expect("execution tool");
        let error = serde_json::from_value::<ToolApiBinding>(execution_tool)
            .expect_err("execution tool must not inhabit Tool API function collection");
        assert!(error.to_string().contains("not an Agena Tool API function"));
    }

    #[test]
    fn tool_api_definitions_distinguish_functions_from_execution_tools() {
        let mut list = registered_tool("tools", "list");
        list.definition.docs.summary = Some("List available execution tools".to_owned());
        let mut call = registered_tool("tools", "call");
        call.definition.docs.summary = Some("Run one execution tool".to_owned());

        let list = ToolApiDefinition::from_binding(
            &ToolApiBinding::from_registered_tool(list).expect("list Tool API handler"),
        );
        let call = ToolApiDefinition::from_binding(
            &ToolApiBinding::from_registered_tool(call).expect("call Tool API handler"),
        );

        assert!(
            list.description
                .contains("execution tools available in this session")
        );
        assert!(
            list.description
                .contains("Execution-tool names are not function names")
        );
        assert!(call.description.contains("complete argument object"));
        assert!(
            call.description
                .contains("function name is always `tools_call`")
        );
        assert!(
            call.description
                .contains("never put a Tool API function name")
        );
        assert!(call.description.contains("retry `tools_call` directly"));
        assert_ne!(list.description, call.description);
    }

    #[test]
    fn colliding_compact_tool_names_use_unambiguous_internal_keys() {
        let alpha = namespaced_registered_tool("alpha", "notes", "format");
        let beta = namespaced_registered_tool("beta", "notes", "format");
        let execution_tools = vec![
            ExecutionTool::from_registered_tool(alpha.clone()).expect("execution tool"),
            ExecutionTool::from_registered_tool(beta.clone()).expect("execution tool"),
        ];

        assert_eq!(
            execution_tool_names(&execution_tools),
            vec!["alpha.notes.format", "beta.notes.format"]
        );
        let tools = vec![alpha, beta];
        assert!(unique_registered_tool_match(tools.clone(), "notes.format").is_none());
        assert_eq!(
            unique_registered_tool_match(tools, "beta.notes.format")
                .expect("unambiguous execution tool")
                .canonical_name(),
            "beta.notes.format"
        );
    }

    #[test]
    fn execution_tool_name_resolution_does_not_trim_names() {
        let tool = namespaced_registered_tool("agena", "session", "rename");

        assert!(unique_registered_tool_match(vec![tool.clone()], "session.rename").is_some());
        assert!(unique_registered_tool_match(vec![tool.clone()], " session.rename").is_none());
        assert!(unique_registered_tool_match(vec![tool], "session.rename ").is_none());
    }
}
