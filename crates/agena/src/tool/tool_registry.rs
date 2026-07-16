pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

/// Convert an internal canonical registry key such as
/// `agena.session.rename` into the compact name carried as gateway payload
/// data, such as `session.rename`.
pub(crate) fn catalog_target_name(name: &str) -> String {
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

/// Produce the payload address for each catalog tool. Compact
/// `plugin.tool` addresses are preferred, but collisions retain the full
/// `namespace.plugin.tool` registry key so every advertised address resolves
/// to exactly one target.
pub(crate) fn catalog_target_addresses(tools: &[RegisteredTool]) -> Vec<String> {
    let mut compact_name_counts = std::collections::HashMap::<String, usize>::new();
    for tool in tools {
        *compact_name_counts
            .entry(catalog_target_name(tool.canonical_name().as_str()))
            .or_default() += 1;
    }
    tools
        .iter()
        .map(|tool| {
            let canonical = tool.canonical_name();
            let compact = catalog_target_name(canonical.as_str());
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

pub(crate) fn is_gateway_handler(tool: &RegisteredTool) -> bool {
    GatewayFunction::from_handler_parts(tool.namespace(), tool.plugin_name(), tool.tool_name())
        .is_some()
}

pub(crate) fn gateway_help_tool_name() -> &'static str {
    GatewayFunction::ToolsHelp.protocol_name()
}

pub(crate) fn gateway_call_tool_name() -> &'static str {
    GatewayFunction::ToolsCall.protocol_name()
}

pub(super) fn gateway_protocol_name(tool: &RegisteredTool) -> Option<&'static str> {
    GatewayFunction::from_handler_parts(tool.namespace(), tool.plugin_name(), tool.tool_name())
        .map(GatewayFunction::protocol_name)
}

/// A registry tool proven to be one of Agena's provider-facing gateway
/// functions. Catalog tools cannot be placed in a [`GatewayToolBinding`], so a
/// `CompletionRequest` cannot accidentally advertise them to a provider.
#[derive(Debug, Clone, serde::Serialize, PartialEq)]
pub struct GatewayToolBinding {
    function: GatewayFunction,
    handler: RegisteredTool,
}

impl<'de> serde::Deserialize<'de> for GatewayToolBinding {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct SerializedGatewayToolBinding {
            function: GatewayFunction,
            handler: RegisteredTool,
        }

        let serialized = SerializedGatewayToolBinding::deserialize(deserializer)?;
        let binding = Self::from_registered_tool(serialized.handler)
            .ok_or_else(|| serde::de::Error::custom("handler is not an Agena gateway function"))?;
        if binding.function != serialized.function {
            return Err(serde::de::Error::custom(format!(
                "provider function `{}` does not match handler `{}`",
                serialized.function.protocol_name(),
                binding.handler.canonical_name()
            )));
        }
        Ok(binding)
    }
}

impl GatewayToolBinding {
    pub fn from_registered_tool(handler: RegisteredTool) -> Option<Self> {
        let function = GatewayFunction::from_handler_parts(
            handler.namespace(),
            handler.plugin_name(),
            handler.tool_name(),
        )?;
        Some(Self { function, handler })
    }

    pub const fn function(&self) -> GatewayFunction {
        self.function
    }

    pub fn protocol_name(&self) -> &'static str {
        self.function.protocol_name()
    }

    pub fn canonical_name(&self) -> String {
        self.handler.canonical_name()
    }

    pub fn handler(&self) -> &RegisteredTool {
        &self.handler
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct GatewayFunctionSpec {
    pub handler_name: String,
    pub protocol_name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
    pub output_schema: serde_json::Value,
    pub strict: bool,
    pub definition_identity: String,
}

impl GatewayFunctionSpec {
    pub fn from_gateway_binding(binding: &GatewayToolBinding) -> Self {
        let handler = binding.handler();
        Self {
            handler_name: binding.canonical_name(),
            protocol_name: binding.protocol_name().to_owned(),
            description: gateway_function_description(binding.function()).to_owned(),
            input_schema: handler.input_schema(),
            output_schema: handler.output_schema(),
            strict: handler.definition.contract.strict,
            definition_identity: handler.definition_identity(),
        }
    }
}

pub fn gateway_function_specs(tools: &[GatewayToolBinding]) -> Vec<GatewayFunctionSpec> {
    tools
        .iter()
        .map(GatewayFunctionSpec::from_gateway_binding)
        .collect()
}

fn gateway_function_description(function: GatewayFunction) -> &'static str {
    match function {
        GatewayFunction::ToolsList => {
            "Discover the dotted catalog targets currently available in this Agena runtime. Call this provider function when you need to know which capabilities exist instead of relying on a system-prompt tool list or prior knowledge. Results are catalog target names for tools_help or tools_call; they are not provider function names. Supports pagination and tag filters."
        }
        GatewayFunction::ToolsSearch => {
            "Search the live Agena catalog for dotted target names and summaries. Use this provider function when you know the capability you need but not its exact target name. Search results are payload values: inspect an unfamiliar result with tools_help or execute it with tools_call; never call a dotted result as a provider function."
        }
        GatewayFunction::ToolsHelp => {
            "Inspect the live input schema, usage, and examples for one exact dotted catalog target. This provider function performs discovery only: it does not execute or authorize the target, and its help is reusable. After reading the result, execute the target with tools_call and one complete target input object."
        }
        GatewayFunction::ToolsTags => {
            "List tags from the live Agena catalog for capability discovery and filtering. Use returned tags with tools_list or tools_search. This provider function does not execute catalog targets."
        }
        GatewayFunction::ToolsCall => {
            "Execute one exact dotted catalog target discovered through tools_list or tools_search. The function arguments must have exactly this routing shape: {\"tool\":\"DOTTED_TARGET\",\"input\":{\"TARGET_ARGUMENT\":\"TASK_VALUE\"}}. Copy every target-specific key and value supplied by the task or tools_help into the single open `input` object; never replace a populated object with `{}` or make an empty, default, or preliminary call when the target requires fields. Dotted targets are payload values, not provider function names. Do not put tools_list, tools_search, tools_help, tools_tags, or tools_call in `tool`; call those provider functions directly. If target-schema validation rejects the input, the failed receipt already includes complete target help, so read it and retry tools_call directly without a separate tools_help call."
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
        || catalog_target_name(registered_tool.canonical_name().as_str()) == name
        || gateway_protocol_name(registered_tool).is_some_and(|gateway_name| gateway_name == name)
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

pub(super) fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    if is_gateway_handler(registered_tool) {
        return "Tool gateway.".to_string();
    }
    format!(
        "Tool `{}`.",
        catalog_target_name(registered_tool.canonical_name().as_str())
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
    provided_agent, provided_catalog, provided_code, provided_cron, provided_fs,
    provided_interaction, provided_lsp, provided_planning, provided_repo, provided_schema_lab,
    provided_session, provided_settings, provided_shell, provided_tasks, skills, snapshot,
};
use crate::tool_protocol::GatewayFunction;

#[cfg(test)]
mod gateway_binding_tests {
    use super::{
        GatewayFunctionSpec, GatewayToolBinding, catalog_target_addresses,
        unique_registered_tool_match,
    };
    use crate::plugin::registry::RegisteredTool;
    use crate::plugin::sdk::{PluginKey, ToolDefinition};
    use crate::tool_protocol::GatewayFunction;

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
    fn only_gateway_handlers_can_become_gateway_bindings() {
        assert!(
            GatewayToolBinding::from_registered_tool(registered_tool("session", "rename"))
                .is_none()
        );
        assert!(
            GatewayToolBinding::from_registered_tool(namespaced_registered_tool(
                "third_party",
                "tools",
                "help"
            ))
            .is_none()
        );

        let binding = GatewayToolBinding::from_registered_tool(registered_tool("tools", "help"))
            .expect("gateway handler");
        assert_eq!(binding.function(), GatewayFunction::ToolsHelp);
        assert_eq!(binding.protocol_name(), "tools_help");
        assert_eq!(binding.canonical_name(), "agena.tools.help");

        let spec = GatewayFunctionSpec::from_gateway_binding(&binding);
        assert_eq!(spec.protocol_name, "tools_help");
        assert_eq!(spec.handler_name, "agena.tools.help");
    }

    #[test]
    fn deserialization_cannot_forge_a_provider_binding() {
        let valid = GatewayToolBinding::from_registered_tool(registered_tool("tools", "help"))
            .expect("gateway handler");
        let valid_value = serde_json::to_value(valid).expect("serialize gateway binding");

        let mut mismatched_function = valid_value.clone();
        mismatched_function["function"] = serde_json::Value::String("tools_call".to_owned());

        let error = serde_json::from_value::<GatewayToolBinding>(mismatched_function)
            .expect_err("mismatched function/handler must fail");
        assert!(error.to_string().contains("does not match handler"));

        let mut catalog_handler = valid_value;
        catalog_handler["handler"] =
            serde_json::to_value(registered_tool("session", "rename")).expect("catalog handler");
        let error = serde_json::from_value::<GatewayToolBinding>(catalog_handler)
            .expect_err("catalog handler must not inhabit provider tool collection");
        assert!(error.to_string().contains("not an Agena gateway function"));
    }

    #[test]
    fn gateway_specs_explain_discovery_and_execution_in_function_definitions() {
        let mut list = registered_tool("tools", "list");
        list.definition.docs.summary = Some("List available catalog targets".to_owned());
        let mut call = registered_tool("tools", "call");
        call.definition.docs.summary = Some("Invoke one catalog target".to_owned());

        let list = GatewayFunctionSpec::from_gateway_binding(
            &GatewayToolBinding::from_registered_tool(list).expect("list gateway"),
        );
        let call = GatewayFunctionSpec::from_gateway_binding(
            &GatewayToolBinding::from_registered_tool(call).expect("call gateway"),
        );

        assert!(
            list.description
                .contains("Discover the dotted catalog targets")
        );
        assert!(
            list.description
                .contains("instead of relying on a system-prompt tool list")
        );
        assert!(
            call.description
                .contains("every target-specific key and value")
        );
        assert!(
            call.description
                .contains("never replace a populated object with `{}`")
        );
        assert!(call.description.contains("Do not put tools_list"));
        assert!(
            call.description
                .contains("without a separate tools_help call")
        );
        assert_ne!(list.description, call.description);
    }

    #[test]
    fn colliding_compact_catalog_names_are_advertised_and_resolved_canonically() {
        let alpha = namespaced_registered_tool("alpha", "notes", "format");
        let beta = namespaced_registered_tool("beta", "notes", "format");
        let tools = vec![alpha.clone(), beta.clone()];

        assert_eq!(
            catalog_target_addresses(&tools),
            vec!["alpha.notes.format", "beta.notes.format"]
        );
        assert!(unique_registered_tool_match(tools.clone(), "notes.format").is_none());
        assert_eq!(
            unique_registered_tool_match(tools, "beta.notes.format")
                .expect("canonical target")
                .canonical_name(),
            "beta.notes.format"
        );
    }

    #[test]
    fn catalog_target_resolution_does_not_trim_payload_names() {
        let tool = namespaced_registered_tool("agena", "session", "rename");

        assert!(unique_registered_tool_match(vec![tool.clone()], "session.rename").is_some());
        assert!(unique_registered_tool_match(vec![tool.clone()], " session.rename").is_none());
        assert!(unique_registered_tool_match(vec![tool], "session.rename ").is_none());
    }
}
