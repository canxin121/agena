pub(crate) mod apply_patch;
pub(crate) mod ask_user;
pub(crate) mod bash;
pub(crate) mod catalog;
pub(crate) mod cron;
pub(crate) mod definition;
pub(crate) mod file_attachment;
pub(crate) mod glob;
pub(crate) mod grep;
pub(crate) mod lsp;
pub(crate) mod monitor;
pub(crate) mod orchestrator;
pub(crate) mod payload;
pub(crate) mod powershell;
pub(crate) mod process_tool;
pub(crate) mod read;
pub(crate) mod result;
pub(crate) mod shell;
pub(crate) mod shell_tools;
pub(crate) mod snapshot;
pub(crate) mod task;
pub(crate) mod tool_search;
pub(crate) mod truncation;

use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    AskUserToolInput, FilesystemEffect, Message, NetworkEffect, PluginInvocation, StructuredObject,
    ToolInvocation, ToolOutput,
};
use crate::permission::{AccessKind, NetworkTarget, PermissionAction, PermissionDecision};
use crate::plugin::{
    PluginHost, PluginHostBuildConfig, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolDefinitionInput as PluginToolDefinitionInput,
    ToolFailureInput as PluginToolFailureInput, ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionNetworksInput as PluginToolPermissionNetworksInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    registry::RegisteredTool,
    sdk::{
        InputNetworkSpec as SdkInputNetworkSpec, InputPathSpec as SdkInputPathSpec,
        NetworkAccessSpec as SdkNetworkAccessSpec, PathAccessSpec as SdkPathAccessSpec,
        PathKind as SdkPathKind, ShellEnvInput as PluginShellEnvInput,
        ToolResultPolicy as SdkToolResultPolicy, ToolStreamingMode as SdkToolStreamingMode,
    },
};
use crate::plugins::provided::{
    catalog as provided_catalog, code as provided_code, cron as provided_cron, fs as provided_fs,
    lsp as provided_lsp, mcp, planning as provided_planning, process as provided_process,
    repo as provided_repo, router as in_process_router, runtime as provided_runtime,
    schema_lab as provided_schema_lab, settings as provided_settings, skills,
    tasks as provided_tasks,
};

const TOOL_MODEL_OUTPUT_MAX_LINES: usize = 2_000;
const TOOL_MODEL_OUTPUT_MAX_BYTES: usize = 50 * 1024;

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use payload::{CronJobSummary, ToolPayloadInput, ToolPayloadOutput, WebSearchHit};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use shell::{ShellError, ShellOutput, ShellRequest};
pub use snapshot::{
    ActiveSnapshot, ManagedSnapshot, SnapshotBackend, SnapshotBackendCapabilities,
    SnapshotBackendSupport, SnapshotRegistry,
    backend_capabilities as snapshot_backend_capabilities, list_active as snapshot_list_active,
    list_managed as snapshot_list_managed, prune_stale as snapshot_prune_stale,
    registry_for_executor as snapshot_registry_for_executor,
};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};

pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
}

pub(crate) fn model_safe_tool_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return "tool".to_owned();
    }

    if trimmed
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    {
        return trimmed.to_owned();
    }

    crate::plugin::registry::model_tool_name_segment(trimmed)
}

#[derive(Debug, Clone, PartialEq)]
pub struct ModelToolSpec {
    pub canonical_name: String,
    pub model_name: String,
    pub provider_safe_name: String,
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
        let model_name = tool.model_name.clone();
        Self {
            canonical_name: tool.model_name.clone(),
            provider_safe_name: model_safe_tool_name(model_name.as_str()),
            model_name,
            description: tool.description_text().to_string(),
            input_schema: model_safe_tool_schema(&tool.sanitized_input_schema()),
            output_schema: tool.sanitized_output_schema(),
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

fn normalized_tool_name_distance(left: &str, right: &str) -> usize {
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
    registered_tool.model_name == trimmed
        || model_safe_tool_name(registered_tool.model_name.as_str()) == trimmed
}

pub(crate) fn model_safe_tool_schema(schema: &serde_json::Value) -> serde_json::Value {
    let serde_json::Value::Object(mut object) = schema.clone() else {
        return empty_object_schema();
    };

    for key in ["oneOf", "anyOf", "allOf"] {
        let Some(serde_json::Value::Array(variants)) = object.remove(key) else {
            continue;
        };
        if variants
            .iter()
            .all(|variant| json_schema_object(variant).is_some())
        {
            return merge_top_level_object_variants(object, variants);
        }
        return empty_object_schema();
    }

    let is_object = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
        || object.contains_key("properties");
    if !is_object {
        return empty_object_schema();
    }
    object
        .entry("type".to_owned())
        .or_insert_with(|| serde_json::Value::String("object".to_owned()));
    object
        .entry("properties".to_owned())
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    serde_json::Value::Object(object)
}

fn empty_object_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

fn merge_top_level_object_variants(
    mut base: serde_json::Map<String, serde_json::Value>,
    variants: Vec<serde_json::Value>,
) -> serde_json::Value {
    base.insert(
        "type".to_owned(),
        serde_json::Value::String("object".to_owned()),
    );
    let mut properties = base
        .remove("properties")
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default();
    let mut required_intersection: Option<BTreeSet<String>> = required_set(&base);

    for variant in variants {
        let Some(variant) = json_schema_object(&variant) else {
            continue;
        };
        if let Some(variant_properties) = variant
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            for (name, schema) in variant_properties {
                properties
                    .entry(name.clone())
                    .and_modify(|existing| *existing = merge_property_schema(existing, schema))
                    .or_insert_with(|| schema.clone());
            }
        }
        if let Some(variant_required) = required_set(variant) {
            required_intersection = Some(match required_intersection.take() {
                Some(existing) => existing
                    .intersection(&variant_required)
                    .cloned()
                    .collect::<BTreeSet<_>>(),
                None => variant_required,
            });
        }
    }

    base.insert(
        "properties".to_owned(),
        serde_json::Value::Object(properties),
    );
    if let Some(required) = required_intersection.filter(|required| !required.is_empty()) {
        base.insert(
            "required".to_owned(),
            serde_json::Value::Array(
                required
                    .into_iter()
                    .map(serde_json::Value::String)
                    .collect(),
            ),
        );
    } else {
        base.remove("required");
    }
    serde_json::Value::Object(base)
}

fn json_schema_object(
    value: &serde_json::Value,
) -> Option<&serde_json::Map<String, serde_json::Value>> {
    let object = value.as_object()?;
    let is_object = object
        .get("type")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|kind| kind == "object")
        || object.contains_key("properties")
        || object.contains_key("required");
    is_object.then_some(object)
}

fn required_set(object: &serde_json::Map<String, serde_json::Value>) -> Option<BTreeSet<String>> {
    object
        .get("required")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}

fn merge_property_schema(
    existing: &serde_json::Value,
    next: &serde_json::Value,
) -> serde_json::Value {
    let Some(mut literals) = string_literals(existing) else {
        return existing.clone();
    };
    let Some(next_literals) = string_literals(next) else {
        return existing.clone();
    };
    literals.extend(next_literals);
    serde_json::json!({
        "type": "string",
        "enum": literals.into_iter().collect::<Vec<_>>()
    })
}

fn string_literals(value: &serde_json::Value) -> Option<BTreeSet<String>> {
    let object = value.as_object()?;
    if let Some(value) = object.get("const").and_then(serde_json::Value::as_str) {
        return Some(BTreeSet::from([value.to_owned()]));
    }
    object
        .get("enum")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
}

fn expand_registered_tool_for_model(base: &RegisteredTool, out: &mut Vec<RegisteredTool>) {
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
    provided_catalog::CATALOG_PLUGIN_ID
}

pub fn new_catalog_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_catalog::CatalogPlugin::new()
}

pub fn runtime_plugin_id() -> &'static str {
    provided_runtime::RUNTIME_PLUGIN_ID
}

pub fn new_runtime_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_runtime::RuntimePlugin::new()
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
enum PermissionEnforcementMode {
    Enforced,
    Bypassed,
}

#[derive(Debug, Clone, Default)]
pub(super) struct ToolRuntimeContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub session_context: Option<crate::session::SessionExecutionContext>,
    pub prepared_shell_command: Option<PreparedShellCommand>,
}

static SYNTHETIC_TOOL_CALL_ID: AtomicI64 = AtomicI64::new(-1);

pub struct StreamingToolExecution {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<crate::plugin::sdk::ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolInvocationExecution, ToolError>>,
    _executor_guard: Option<in_process_router::ExecutorContextGuard>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Error)]
pub enum ToolError {
    #[error("permission denied: {0}")]
    PermissionDenied(String),
    #[error("permission confirmation required: {0}")]
    PermissionAsk(String),
    #[error("user input required")]
    UserInputRequired(AskUserToolInput),
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

fn present_registered_tool(
    mut registered_tool: RegisteredTool,
    presentation: &crate::plugin::ToolPresentationConfig,
) -> RegisteredTool {
    let mode = presentation.mode_for(
        registered_tool.plugin_full_name().as_str(),
        registered_tool.tool_name.as_str(),
        registered_tool.model_name.as_str(),
        registered_tool.definition.preferred_description_mode(),
    );
    if mode == crate::plugin::ToolDescriptionMode::Brief {
        registered_tool.definition.model.description =
            Some(compact_tool_description(&registered_tool));
    }
    registered_tool.definition.docs.help = None;
    registered_tool
}

fn compact_tool_description(registered_tool: &RegisteredTool) -> String {
    let summary = tool_summary_sentence(registered_tool);
    format!(
        "{summary} See `tools.help` for `{}`.",
        registered_tool.model_name
    )
}

fn tool_summary_sentence(registered_tool: &RegisteredTool) -> String {
    let summary = tool_summary(registered_tool);
    if matches!(summary.chars().last(), Some('.' | '!' | '?')) {
        return summary;
    }
    format!("{summary}.")
}

fn tool_summary(registered_tool: &RegisteredTool) -> String {
    if let Some(summary) = registered_tool.summary_text() {
        return summary.to_string();
    }
    registered_tool
        .description_text()
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Tool `{}`.", registered_tool.model_name))
}

#[derive(Clone)]
pub struct ToolExecutor {
    workspace_root: PathBuf,
    agent: Agent,
    model_id: Option<String>,
    subagent_registry: crate::agents::SubagentRegistry,
    monitor_registry: Option<Arc<dyn MonitorService>>,
    truncator: ToolOutputTruncator,
    plugins: Arc<PluginHost>,
    snapshot_registry: Option<snapshot::SnapshotRegistry>,
    scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    permission_mode: PermissionEnforcementMode,
    tool_presentation: crate::plugin::ToolPresentationConfig,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subagent_registry: crate::agents::SubagentRegistry::empty(),
            monitor_registry: monitor::default_registry(),
            truncator: ToolOutputTruncator::default(),
            plugins: PluginHost::new_empty(),
            snapshot_registry: None,
            scheduler: None,
            lsp_registry: None,
            permission_mode: PermissionEnforcementMode::Enforced,
            tool_presentation: crate::plugin::ToolPresentationConfig::default(),
        }
    }

    pub fn with_monitor_registry(mut self, registry: Arc<dyn MonitorService>) -> Self {
        self.monitor_registry = Some(registry);
        self
    }

    pub fn without_monitor_registry(mut self) -> Self {
        self.monitor_registry = None;
        self
    }

    pub fn with_model_id(mut self, model_id: impl Into<String>) -> Self {
        self.model_id = Some(model_id.into());
        self
    }

    pub fn with_subagent_registry(mut self, registry: crate::agents::SubagentRegistry) -> Self {
        self.subagent_registry = registry;
        self
    }

    pub fn subagent_registry(&self) -> &crate::agents::SubagentRegistry {
        &self.subagent_registry
    }

    pub fn with_plugin_manager(mut self, manager: Arc<PluginHost>) -> Self {
        self.plugins = manager;
        self
    }

    pub fn with_tool_presentation(
        mut self,
        presentation: crate::plugin::ToolPresentationConfig,
    ) -> Self {
        self.tool_presentation = presentation;
        self
    }

    pub fn with_snapshot_registry(mut self, reg: snapshot::SnapshotRegistry) -> Self {
        self.snapshot_registry = Some(reg);
        self
    }

    pub fn snapshot_registry(&self) -> Option<&snapshot::SnapshotRegistry> {
        self.snapshot_registry.as_ref()
    }

    pub fn with_scheduler(mut self, scheduler: Arc<agena_scheduler::Scheduler>) -> Self {
        self.scheduler = Some(scheduler);
        self
    }

    pub fn scheduler(&self) -> Option<&Arc<agena_scheduler::Scheduler>> {
        self.scheduler.as_ref()
    }

    pub fn with_lsp_registry(mut self, registry: Arc<agena_lsp::LspRegistry>) -> Self {
        self.lsp_registry = Some(registry);
        self
    }

    pub fn lsp_registry(&self) -> Option<&Arc<agena_lsp::LspRegistry>> {
        self.lsp_registry.as_ref()
    }

    pub fn with_truncation_policy(mut self, policy: ToolOutputTruncationPolicy) -> Self {
        self.truncator = ToolOutputTruncator::new(policy);
        self
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn for_session_context(
        &self,
        session_context: &crate::session::SessionExecutionContext,
    ) -> Self {
        let mut scoped = self.clone();
        if let Some(root) = session_context.effective_workspace_root.as_ref() {
            scoped.workspace_root = root.clone();
        }
        if !session_context.effective_permission.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_permission_config(&session_context.effective_permission);
        }
        if !session_context.allowed_tools.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_allowed_tools(session_context.allowed_tools.iter().map(String::as_str));
        }
        if let Some(model_id) = session_context.selection.model.as_ref() {
            scoped.model_id = Some(model_id.clone());
        }
        scoped
    }

    pub fn agent(&self) -> &Agent {
        &self.agent
    }

    pub fn monitor_registry(&self) -> Option<&Arc<dyn MonitorService>> {
        self.monitor_registry.as_ref()
    }

    pub fn plugin_manager(&self) -> &Arc<PluginHost> {
        &self.plugins
    }

    pub fn tool_catalog(&self) -> ToolCatalog {
        ToolCatalog::for_model(self.model_id.as_deref())
    }

    fn registered_tools_with_definition_overrides(&self) -> Vec<RegisteredTool> {
        let mut tools = self
            .plugins
            .registered_tools()
            .into_iter()
            .collect::<Vec<_>>();

        tools.sort_by(|left, right| {
            left.model_name
                .cmp(&right.model_name)
                .then_with(|| left.description_text().cmp(right.description_text()))
        });

        // Plugin chain: tool.definition. Let plugins rewrite descriptions /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            tools = tools
                .into_iter()
                .map(|mut entry| {
                    let input = PluginToolDefinitionInput {
                        tool_name: entry.tool_name.clone(),
                        plugin_name: entry.plugin_full_name(),
                        description: entry.description_text().to_string(),
                        summary: entry.definition.docs.summary.clone(),
                        help: entry.definition.docs.help.clone(),
                        description_mode: entry.definition.display.description_mode,
                        input_schema: entry.sanitized_input_schema(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => {
                            entry.definition.model.description = Some(patched.description);
                            entry.definition.docs.summary = patched.summary;
                            entry.definition.docs.help = patched.help;
                            entry.definition.display.description_mode = patched.description_mode;
                            entry.definition.contract.input_schema = patched.input_schema;
                            entry
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %entry.model_name,
                                "tool.definition hook failed (keeping original): {err}"
                            );
                            entry
                        }
                    }
                })
                .collect();
        }

        tools
    }

    fn catalogued_tools_raw(&self) -> Vec<RegisteredTool> {
        let catalog = self.tool_catalog();
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|entry| catalog.is_tool_enabled(entry))
            .filter(|entry| self.is_tool_visible_to_agent(entry))
            .collect()
    }

    fn catalogued_model_tools_raw(&self) -> Vec<RegisteredTool> {
        let mut expanded = Vec::new();
        for tool in self.registered_tools_with_definition_overrides() {
            expand_registered_tool_for_model(&tool, &mut expanded);
        }
        let catalog = self.tool_catalog();
        expanded.retain(|entry| catalog.is_tool_enabled(entry));
        expanded.retain(|entry| self.is_tool_visible_to_agent(entry));
        expanded.sort_by(|left, right| {
            left.model_name
                .cmp(&right.model_name)
                .then_with(|| left.description_text().cmp(right.description_text()))
        });
        expanded
    }

    fn is_tool_visible_to_agent(&self, entry: &RegisteredTool) -> bool {
        !matches!(
            self.agent.authorize_tool_names(
                &[entry.model_name.as_str()],
                None,
                &entry.effective_tags()
            ),
            PermissionDecision::Deny { .. }
        )
    }

    fn catalogued_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    fn catalogued_model_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_model_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    pub fn detailed_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
    }

    pub fn detailed_model_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_model_tools_raw()
    }

    pub fn searchable_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_model_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_model_tools()
    }

    fn suggested_tool_names(&self, requested: &str) -> Vec<String> {
        let mut candidates = self
            .catalogued_tools_raw()
            .into_iter()
            .map(|tool| tool.model_name)
            .collect::<Vec<_>>();
        candidates.extend(
            self.catalogued_model_tools_raw()
                .into_iter()
                .map(|tool| tool.model_name),
        );
        candidates.sort();
        candidates.dedup();
        suggest_tool_names(requested, candidates, 1)
    }

    fn unknown_tool_error(&self, requested: &str) -> ToolError {
        let suggestions = self.suggested_tool_names(requested);
        if suggestions.is_empty() {
            ToolError::UnknownTool {
                tool: requested.to_string(),
            }
        } else {
            unknown_tool_hint(requested, suggestions)
        }
    }

    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let invocation = PluginInvocation::from_tool_invocation(invocation);
        let Some(entry) = self.plugin_invocation_definition(&invocation) else {
            return false;
        };
        entry.definition.runtime.concurrency_safe
            && !entry.has_tag(crate::plugin::sdk::ToolTag::Interactive)
            && is_concurrency_safe_tool_invocation(&entry, &invocation)
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<RegisteredTool> {
        let _ = messages;
        self.available_tools()
    }

    fn invocation_definition(&self, invocation: &ToolInvocation) -> Option<RegisteredTool> {
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    pub fn validate_advertised_tool_identity(
        &self,
        invocation: &ToolInvocation,
        advertised_identity: Option<&str>,
    ) -> Result<(), ToolError> {
        let Some(advertised_identity) = advertised_identity
            .map(str::trim)
            .filter(|value| !value.is_empty())
        else {
            return Ok(());
        };
        let current = self
            .invocation_definition(invocation)
            .map(|definition| definition.definition_identity());
        if current.as_deref() == Some(advertised_identity) {
            return Ok(());
        }
        Err(ToolError::StaleToolCall {
            tool: invocation_name(invocation).to_string(),
        })
    }

    fn plugin_invocation_definition(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<RegisteredTool> {
        self.catalogued_tools()
            .into_iter()
            .find(|entry| tool_matches_model_name(entry, invocation.tool_name.as_str()))
            .or_else(|| {
                self.catalogued_model_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, invocation.tool_name.as_str()))
            })
            .or_else(|| {
                let canonical = canonical_tool_name(invocation.tool_name.as_str());
                self.catalogued_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, canonical))
            })
            .or_else(|| {
                let canonical = canonical_tool_name(invocation.tool_name.as_str());
                self.catalogued_model_tools()
                    .into_iter()
                    .find(|entry| tool_matches_model_name(entry, canonical))
            })
    }

    fn invocation_plugin_name_for(&self, invocation: &ToolInvocation) -> String {
        self.plugin_invocation_plugin_name_for(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_plugin_name_for(&self, invocation: &PluginInvocation) -> String {
        if let Some(entry) = self.plugin_invocation_definition(invocation) {
            return entry.plugin_full_name();
        }

        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
            .map(|entry| entry.plugin_full_name())
            .unwrap_or_else(|| "custom".to_string())
    }

    fn invocation_streaming_mode(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.plugin_invocation_streaming_mode(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_streaming_mode(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<SdkToolStreamingMode> {
        self.plugin_resolution_for_plugin_invocation(invocation)
            .map(|entry| entry.definition.runtime.streaming)
    }

    fn authorize_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<(String, PermissionDecision), ToolError> {
        let tool_name = invocation_name(invocation);
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| self.unknown_tool_error(tool_name.as_str()))?;
        let tags = invocation_effective_tags(&definition, invocation);
        if !self.tool_catalog().are_tags_enabled(&tags) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
            )));
        }
        let command = shell_command_from_invocation(invocation);
        let resolution = self.plugin_resolution_for_invocation(invocation);
        let mut tool_name_aliases = vec![tool_name.as_str()];
        if let Some(resolution) = resolution.as_ref()
            && resolution.tool_name != tool_name
            && self.plugin_tool_name_is_unambiguous(resolution.tool_name.as_str())
        {
            tool_name_aliases.push(resolution.tool_name.as_str());
        }
        Ok((
            tool_name.clone(),
            self.agent
                .authorize_tool_names(&tool_name_aliases, command.as_deref(), &tags),
        ))
    }

    fn plugin_tool_name_is_unambiguous(&self, plugin_tool_name: &str) -> bool {
        self.plugins
            .registered_tools()
            .into_iter()
            .filter(|tool| tool.tool_name == plugin_tool_name)
            .take(2)
            .count()
            == 1
    }

    fn plugin_resolution_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugin_resolution_for_plugin_invocation(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<crate::plugin::registry::RegisteredTool> {
        self.plugins
            .lookup_tool(invocation.tool_name.as_str())
            .or_else(|| {
                self.plugins
                    .lookup_tool(canonical_tool_name(invocation.tool_name.as_str()))
            })
            .or_else(|| {
                self.plugins
                    .registered_tools()
                    .into_iter()
                    .find(|tool| tool_matches_model_name(tool, invocation.tool_name.as_str()))
            })
            .or_else(|| {
                self.catalogued_model_tools_raw()
                    .into_iter()
                    .find(|tool| tool_matches_model_name(tool, invocation.tool_name.as_str()))
            })
    }

    fn collect_declared_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        specs: &[SdkInputPathSpec],
        static_specs: &[SdkPathAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_requested_path_checks(checks, spec.path.as_str(), spec.kind);
        }
        for path_request in extract_input_path_requests(input, specs)? {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    fn collect_dynamic_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &crate::plugin::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_paths(
            registered_tool,
            PluginToolPermissionPathsInput {
                tool_name: registered_tool.tool_name.clone(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
        );

        let path_requests = match result {
            Ok(path_requests) => path_requests,
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
        };

        for path_request in path_requests {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    fn collect_declared_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        input_specs: &[SdkInputNetworkSpec],
        static_specs: &[SdkNetworkAccessSpec],
    ) -> Result<(), ToolError> {
        for spec in static_specs {
            self.push_network_check(checks, spec.target.as_str())?;
        }
        for request in extract_input_network_requests(input, input_specs)? {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    fn collect_dynamic_network_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        registered_tool: &crate::plugin::registry::RegisteredTool,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_networks(
            registered_tool,
            PluginToolPermissionNetworksInput {
                tool_name: registered_tool.tool_name.clone(),
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input.clone(),
            },
        );

        let network_requests = match result {
            Ok(network_requests) => network_requests,
            Err(err)
                if err.code == crate::plugin::sdk::PluginErrorCode::NotImplemented
                    || err.message.contains("method not found")
                    || err.message.contains("not implemented") =>
            {
                return Ok(());
            }
            Err(err) if err.code == crate::plugin::sdk::PluginErrorCode::InvalidParams => {
                return Err(ToolError::InvalidInput(err.message));
            }
            Err(err) => return Err(ToolError::Plugin(err.message)),
        };

        for request in network_requests {
            self.push_network_check(checks, request.target.as_str())?;
        }
        Ok(())
    }

    fn collect_declared_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        tool_name: &str,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        if let Some(effects) = filesystem_effects_from_input(input)? {
            let command = input
                .pointer("/args/command")
                .or_else(|| {
                    input.get("command").filter(|value| {
                        !matches!(
                            value.as_str(),
                            Some(
                                "bash"
                                    | "powershell"
                                    | "exec"
                                    | "monitor"
                                    | "monitor_start"
                                    | "monitor_list"
                                    | "monitor_read"
                                    | "monitor_stop"
                            )
                        )
                    })
                })
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            if !command.is_empty() {
                validate_shell_filesystem_effects(tool_name, command, effects.as_slice())?;
            }
            let workdir = input
                .get("workdir")
                .or_else(|| input.pointer("/args/workdir"))
                .and_then(serde_json::Value::as_str);
            let base = self.shell_effect_base_path(workdir);
            self.push_filesystem_effect_checks(checks, effects.as_slice(), base.as_path());
        }
        Ok(())
    }

    fn push_requested_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        path: &str,
        kind: SdkPathKind,
    ) {
        let target = self.resolve_target_path(path);
        self.push_path_checks(checks, sdk_path_kind_to_access_kind(kind), &target);
    }

    pub(crate) fn requested_path_permission_check(
        &self,
        path: &str,
        kind: SdkPathKind,
    ) -> ToolPermissionCheck {
        let mut checks = Vec::with_capacity(1);
        self.push_requested_path_checks(&mut checks, path, kind);
        checks.remove(0)
    }

    fn push_filesystem_effect_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) {
        for effect in effects {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.push_path_checks(checks, AccessKind::Read, &target);
            }
            if effect.access.includes_write() {
                self.push_path_checks(checks, AccessKind::Write, &target);
            }
        }
    }

    pub fn execute_tool_payload_detailed(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<ToolPayloadExecution, ToolError> {
        self.execute_tool_payload_detailed_with_context(input, ToolRuntimeContext::default())
    }

    pub fn execute_tool_payload_output_for_session(
        &self,
        input: &ToolPayloadInput,
        session_id: i64,
    ) -> Result<ToolPayloadOutput, ToolError> {
        self.execute_tool_payload_detailed_with_context(
            input,
            ToolRuntimeContext {
                session_id: Some(session_id),
                call_id: None,
                session_context: None,
                prepared_shell_command: None,
            },
        )
        .map(|execution| execution.output)
    }

    pub fn execute_tool_payload_for_host(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        call_id: Option<i64>,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> Result<crate::plugin::ToolInvokeOutput, ToolError> {
        let scoped_executor = session_context
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let execution = orchestrator::execute_tool(
            &scoped_executor,
            tool_name,
            input,
            ToolRuntimeContext {
                session_id,
                call_id,
                session_context: None,
                prepared_shell_command: None,
            },
        )?;
        Ok(in_process_router::tool_execution_to_invoke_output(
            scoped_executor.truncator.apply(execution),
        ))
    }

    fn execute_tool_payload_detailed_with_context(
        &self,
        input: &ToolPayloadInput,
        context: ToolRuntimeContext,
    ) -> Result<ToolPayloadExecution, ToolError> {
        let scoped_executor = context
            .session_context
            .as_ref()
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let invocation = input.clone().into_invocation();
        let tool_name = input.tool_name();
        let definition = scoped_executor
            .invocation_definition(&invocation)
            .ok_or_else(|| scoped_executor.unknown_tool_error(tool_name))?;
        if !scoped_executor.tool_catalog().is_tool_enabled(&definition) {
            return Err(ToolError::UnsupportedInvocation(tool_name.to_string()));
        }

        if scoped_executor.permission_mode == PermissionEnforcementMode::Enforced {
            for check in scoped_executor.collect_permission_checks_for_invocation_in_session(
                &invocation,
                context.session_id,
            )? {
                match check.decision {
                    PermissionDecision::Allow => {}
                    PermissionDecision::Ask { reason } => {
                        return Err(ToolError::PermissionAsk(reason));
                    }
                    PermissionDecision::Deny { reason } => {
                        return Err(ToolError::PermissionDenied(reason));
                    }
                }
            }
        }
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context
            .call_id
            .unwrap_or_else(|| SYNTHETIC_TOOL_CALL_ID.fetch_sub(1, Ordering::Relaxed));
        let execution = scoped_executor.execute_invocation_detailed_inner(
            &invocation,
            session_id,
            call_id,
            context.prepared_shell_command,
        )?;
        let output =
            ToolPayloadOutput::from_tool_output(tool_name, &execution.output).ok_or_else(|| {
                ToolError::Plugin(format!(
                    "decode {tool_name} output: payload did not match tool payload schema"
                ))
            })?;
        Ok(scoped_executor.truncator.apply(ToolPayloadExecution {
            output,
            view: execution.view,
            apply_patch: execution.apply_patch,
        }))
    }

    pub fn collect_permission_checks(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(
            &input.clone().into_invocation(),
            None,
        )
    }

    pub fn prepare_shell_command(
        &self,
        input: &crate::message::ShellCommandInput,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<PreparedShellCommand>, ToolError> {
        bash::prepare_command(self, input, session_id, call_id)
    }

    pub fn prepare_process_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<(ToolInvocation, Option<PreparedShellCommand>), ToolError> {
        let Some(ToolPayloadInput::Process(crate::message::ProcessToolInput::Run {
            shell: crate::message::ProcessShell::Bash,
            command: process_input,
            background,
        })) = ToolPayloadInput::from_invocation(invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell = self.prepare_shell_command(&process_input, session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == process_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = process_input;
        rewritten.command = prepared_shell.command.clone();
        let rewritten_invocation =
            ToolPayloadInput::Process(crate::message::ProcessToolInput::Run {
                shell: crate::message::ProcessShell::Bash,
                command: rewritten,
                background,
            })
            .into_invocation();
        let input_value = serde_json::Value::from(rewritten_invocation.input);
        let input = StructuredObject::try_from(input_value)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?;
        Ok((
            ToolInvocation {
                name: invocation.name.clone(),
                plugin_name: invocation.plugin_name.clone(),
                input,
            },
            Some(prepared_shell),
        ))
    }

    pub fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        let model_tool_name = invocation_name(invocation).to_owned();
        let definition = self.invocation_definition(invocation);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        if definition.is_none() {
            let mut prepared_invocation = invocation.clone();
            prepared_invocation.plugin_name = Some(plugin_name);
            return Ok(PreparedToolInvocation {
                invocation: prepared_invocation,
                title_override: None,
                metadata: Default::default(),
            });
        }
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name)
            .unwrap_or_else(|| model_tool_name.clone());
        let input_json = invocation_input_json(invocation)?;
        let parsed_input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;
        let input_value = parsed_input_value;

        let effective_tags = definition
            .as_ref()
            .map(|definition| invocation_effective_tags(definition, invocation))
            .unwrap_or_default();

        let hooked = self
            .plugins
            .dispatch_tool_before(PluginToolBeforeInput {
                tool_name: hook_tool_name,
                plugin_name: plugin_name.clone(),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                tags: effective_tags,
                input: input_value,
                title_override: None,
                metadata: Default::default(),
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let mut prepared_invocation =
            parse_invocation_from_json(model_tool_name.as_str(), input_json.as_str())?;
        prepared_invocation.plugin_name = Some(plugin_name);

        Ok(PreparedToolInvocation {
            invocation: prepared_invocation,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation_in_session(invocation, None)
    }

    pub fn collect_permission_checks_for_invocation_in_session(
        &self,
        invocation: &ToolInvocation,
        _session_id: Option<i64>,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        let (tool_name, decision) = self.authorize_invocation(invocation)?;
        let command = shell_command_from_invocation(invocation);
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            Some(&self.agent.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
            let input_value = resolved_tool_input_value(&resolution, invocation);
            if resolution.has_tag(crate::plugin::sdk::ToolTag::Shell) {
                self.collect_declared_filesystem_effect_checks(
                    &mut checks,
                    tool_name.as_str(),
                    &input_value,
                )?;
            }
            self.collect_declared_path_checks(
                &mut checks,
                &input_value,
                &resolution.definition.permissions.input_paths,
                &resolution.definition.permissions.path_access,
            )?;
            self.collect_dynamic_path_checks(&mut checks, &resolution, &input_value)?;
            self.collect_declared_network_checks(
                &mut checks,
                &input_value,
                &resolution.definition.permissions.input_networks,
                &resolution.definition.permissions.network_access,
            )?;
            self.collect_dynamic_network_checks(&mut checks, &resolution, &input_value)?;
        }
        Ok(checks)
    }

    pub async fn execute_invocation_streaming(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<StreamingToolExecution>, ToolError> {
        if !matches!(
            self.invocation_streaming_mode(invocation),
            Some(SdkToolStreamingMode::Streaming)
        ) {
            return Ok(None);
        }
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);

        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.tool_name.clone(),
        );
        let stream = self
            .plugins
            .invoke_tool_stream(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.tool_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
            )
            .await
            .map_err(|err| ToolError::Plugin(err.message))?;
        let stream_id = stream.stream_id;
        let chunks = stream.chunks;
        let end = stream.end;
        let result_policy = resolution.definition.runtime.result_policy.clone();
        let model_tool_name = resolution.model_name.clone();
        let executor = self.clone();
        let invocation = invocation.clone();
        let (end_tx, end_rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let result = match end.await {
                Ok(Ok(end)) => (|| {
                    let view = ToolExecutionView {
                        title: end.title,
                        output_text: end.output_text,
                        metadata: end.metadata.into_iter().collect(),
                        attachments: end.attachments,
                    };
                    let output = ToolOutput::from_json_payload(end.payload.as_ref())
                        .map_err(ToolError::InvalidInput)?;
                    let mut execution = ToolInvocationExecution::new(output.clone(), view)
                        .with_apply_patch_option(apply_patch_execution_from_tool_output(&output));
                    executor.apply_after_hooks(&invocation, session_id, call_id, &mut execution)?;
                    executor.apply_result_policy(
                        model_tool_name.as_str(),
                        &result_policy,
                        call_id,
                        &mut execution,
                    )?;
                    executor.apply_model_output_boundary(
                        model_tool_name.as_str(),
                        call_id,
                        &mut execution,
                    )?;
                    Ok(execution)
                })(),
                Ok(Err(err)) => Err(ToolError::Plugin(err.message)),
                Err(_) => Err(ToolError::Plugin(
                    "stream ended without a terminal frame".to_string(),
                )),
            };
            let _ = end_tx.send(result);
        });
        Ok(Some(StreamingToolExecution {
            stream_id,
            chunks,
            end: end_rx,
            _executor_guard: Some(executor_guard),
        }))
    }

    pub fn execute_invocation_detailed(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_with_prepared_shell(invocation, session_id, call_id, None)
    }

    pub fn execute_invocation_detailed_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let result = self.execute_invocation_detailed_inner(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        );
        crate::metrics::record_tool_execution(result.is_ok());
        result
    }

    fn execute_invocation_detailed_inner(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        _prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        let _tool_span =
            tracing::info_span!("tool.call", session_id, call_id, tool = tool_name.as_str(),)
                .entered();
        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| self.unknown_tool_error(plugin_invocation.tool_name.as_str()))?;
        let _executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.tool_name.clone(),
        );

        let response = self
            .plugins
            .invoke_tool(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.tool_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: resolved_plugin_invocation_input_value(&resolution, &plugin_invocation),
                },
            )
            .map_err(|err| ToolError::Plugin(err.message))?;

        let view = ToolExecutionView {
            title: response.title.clone(),
            output_text: response.output_text.clone(),
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let output = ToolOutput::from_json_payload(response.payload.as_ref())
            .map_err(ToolError::InvalidInput)?;
        let mut execution = ToolInvocationExecution::new(output.clone(), view)
            .with_apply_patch_option(apply_patch_execution_from_tool_output(&output));
        self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
        self.apply_result_policy(
            resolution.model_name.as_str(),
            &resolution.definition.runtime.result_policy,
            call_id,
            &mut execution,
        )?;
        self.apply_model_output_boundary(resolution.model_name.as_str(), call_id, &mut execution)?;
        Ok(execution)
    }

    pub fn execute_invocation_detailed_bypassing_permissions(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        self.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            invocation, session_id, call_id, None,
        )
    }

    pub fn execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        prepared_shell_command: Option<PreparedShellCommand>,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let mut trusted = self.clone();
        trusted.permission_mode = PermissionEnforcementMode::Bypassed;
        trusted.execute_invocation_detailed_with_prepared_shell(
            invocation,
            session_id,
            call_id,
            prepared_shell_command,
        )
    }

    pub fn shell_env_overrides(
        &self,
        cwd: &Path,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<std::collections::HashMap<String, String>, ToolError> {
        let patch = self
            .plugins
            .dispatch_shell_env(PluginShellEnvInput {
                cwd: cwd.to_path_buf(),
                session_id,
                call_id,
            })
            .map_err(|err| ToolError::Plugin(err.message))?;
        Ok(patch.set.into_iter().collect())
    }

    pub fn execute_tool_payload(
        &self,
        input: &ToolPayloadInput,
    ) -> Result<(ToolPayloadOutput, Option<ApplyPatchExecution>), ToolError> {
        let execution = self.execute_tool_payload_detailed(input)?;
        Ok((execution.output, execution.apply_patch))
    }

    fn apply_after_hooks(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let model_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name)
            .unwrap_or(model_tool_name);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let after_in = PluginToolAfterInput {
            tool_name: hook_tool_name,
            plugin_name: plugin_name.clone(),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: execution.view.title.clone(),
            output_text: execution.view.output_text.clone(),
            payload: execution.output.to_json_payload(),
            metadata: execution.view.metadata.clone().into_iter().collect(),
        };

        let hooked = self
            .plugins
            .dispatch_tool_after(after_in)
            .map_err(|err| ToolError::Plugin(err.message))?;

        execution.view.title = hooked.title;
        execution.view.output_text = hooked.output_text;
        for (k, v) in hooked.metadata {
            execution.view.metadata.insert(k, v);
        }

        if let Some(payload_value) = hooked.payload {
            execution.output = ToolOutput::from_json_payload(Some(&payload_value))
                .map_err(ToolError::InvalidInput)?;
        }

        Ok(())
    }

    fn apply_result_policy(
        &self,
        model_tool_name: &str,
        policy: &SdkToolResultPolicy,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        if policy.is_default() {
            return Ok(());
        }

        execution.view.metadata.insert(
            "result_policy_ui_render_kind".to_string(),
            format!("{:?}", policy.ui_render_kind).to_ascii_lowercase(),
        );
        if let Some(preview_lines) = policy.preview_lines {
            execution.view.metadata.insert(
                "result_policy_preview_lines".to_string(),
                preview_lines.to_string(),
            );
        }

        let original = execution.view.output_text.clone();
        if original.is_empty() {
            return Ok(());
        }

        let mut preview = original.clone();
        let mut truncated = false;

        if let Some(max_lines) = policy.preview_lines
            && max_lines > 0
        {
            let mut lines = preview.lines();
            let selected = lines.by_ref().take(max_lines).collect::<Vec<_>>();
            if lines.next().is_some() {
                preview = selected.join("\n");
                truncated = true;
            }
        }

        if let Some(max_chars) = policy.max_model_chars
            && max_chars > 0
            && preview.chars().count() > max_chars
        {
            preview = truncate_to_char_count(preview.as_str(), max_chars);
            truncated = true;
        }

        if !truncated {
            return Ok(());
        }

        execution
            .view
            .metadata
            .insert("result_policy_truncated".to_string(), "true".to_string());
        execution.view.metadata.insert(
            "result_policy_original_chars".to_string(),
            original.chars().count().to_string(),
        );
        execution.view.metadata.insert(
            "result_policy_model_chars".to_string(),
            preview.chars().count().to_string(),
        );

        if policy.persist_large_output {
            if let Some(path) = persist_tool_result_output(
                self.workspace_root(),
                model_tool_name,
                call_id,
                &original,
            )? {
                execution.view.metadata.insert(
                    "result_policy_persisted_path".to_string(),
                    path.display().to_string(),
                );
                preview.push_str("\n\n[output truncated; full output persisted at ");
                preview.push_str(path.display().to_string().as_str());
                preview.push(']');
            }
        } else {
            preview.push_str("\n\n[output truncated by tool result policy]");
        }

        execution.view.output_text = preview;
        Ok(())
    }

    fn apply_model_output_boundary(
        &self,
        model_tool_name: &str,
        call_id: i64,
        execution: &mut ToolInvocationExecution,
    ) -> Result<(), ToolError> {
        let contextual = model_output_boundary_context(execution);
        if contextual.trim().is_empty()
            || !model_output_exceeds_boundary(
                contextual.as_str(),
                TOOL_MODEL_OUTPUT_MAX_LINES,
                TOOL_MODEL_OUTPUT_MAX_BYTES,
            )
        {
            return Ok(());
        }

        let Some(path) = persist_tool_result_output(
            self.workspace_root(),
            model_tool_name,
            call_id,
            contextual.as_str(),
        )?
        else {
            return Ok(());
        };

        let path_text = path.display().to_string();
        let marker = format!("... output truncated; full content saved to {path_text} ...");
        let preview = bounded_model_output_preview(
            contextual.as_str(),
            marker.as_str(),
            TOOL_MODEL_OUTPUT_MAX_LINES,
            TOOL_MODEL_OUTPUT_MAX_BYTES,
        );

        if execution.view.output_text.trim().is_empty()
            || model_output_exceeds_boundary(
                execution.view.output_text.as_str(),
                TOOL_MODEL_OUTPUT_MAX_LINES,
                TOOL_MODEL_OUTPUT_MAX_BYTES,
            )
        {
            execution.view.output_text = preview;
        } else if !execution.view.output_text.contains(marker.as_str()) {
            execution.view.output_text.push_str("\n\n");
            execution.view.output_text.push_str(marker.as_str());
        }

        execution.output.mark_truncated(path_text.clone());
        execution
            .view
            .metadata
            .insert("model_output_truncated".to_string(), "true".to_string());
        execution
            .view
            .metadata
            .insert("model_output_full_path".to_string(), path_text);
        execution.view.metadata.insert(
            "model_output_original_bytes".to_string(),
            contextual.len().to_string(),
        );
        Ok(())
    }

    /// Fire-and-forget notification to plugins about a tool execution failure.
    pub fn broadcast_tool_failure(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
        error: &str,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let model_tool_name = invocation_name(invocation).to_owned();
        let hook_tool_name = self
            .plugin_resolution_for_invocation(invocation)
            .map(|entry| entry.tool_name)
            .unwrap_or(model_tool_name);
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool_name: hook_tool_name,
            plugin_name,
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            input: input_value,
            error: error.to_owned(),
            is_interrupt: false,
        };
        let plugins = Arc::clone(&self.plugins);
        tokio::spawn(async move {
            plugins.broadcast_tool_failure(failure_input).await;
        });
    }

    pub fn broadcast_notification(
        &self,
        kind: impl Into<String>,
        session_id: Option<i64>,
        title: impl Into<String>,
        message: impl Into<String>,
        payload: serde_json::Value,
    ) {
        if self.plugins.is_empty() {
            return;
        }
        let plugins = Arc::clone(&self.plugins);
        let input = crate::plugin::NotificationInput {
            kind: kind.into(),
            session_id,
            title: title.into(),
            message: message.into(),
            payload,
        };
        tokio::spawn(async move {
            plugins.broadcast_notification(input).await;
        });
    }

    pub(crate) fn resolve_target_path(&self, raw_path: &str) -> PathBuf {
        self.resolve_target_path_with_context(raw_path, None)
    }

    pub(crate) fn shell_effect_base_path(&self, workdir: Option<&str>) -> PathBuf {
        workdir
            .map(|workdir| self.resolve_target_path(workdir))
            .unwrap_or_else(|| self.workspace_root().to_path_buf())
    }

    pub(crate) fn resolve_filesystem_effect_path(
        &self,
        raw_path: &str,
        base_path: &Path,
    ) -> PathBuf {
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            candidate
        } else {
            base_path.join(candidate)
        }
    }

    pub(crate) fn resolve_target_path_with_context(
        &self,
        raw_path: &str,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> PathBuf {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Some(path) = resolve_managed_project_path_alias(raw_path, workspace_root) {
            return path;
        }
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            return candidate;
        }
        workspace_root.join(candidate)
    }

    pub(crate) fn execute_shell_command(
        &self,
        request: &ShellRequest,
    ) -> Result<ShellOutput, ToolError> {
        shell::execute(request).map_err(ToolError::from)
    }

    pub(crate) fn effective_workspace_root<'a>(
        &'a self,
        session_context: Option<&'a crate::session::SessionExecutionContext>,
    ) -> &'a Path {
        session_context
            .and_then(|context| context.effective_workspace_root.as_deref())
            .unwrap_or(self.workspace_root())
    }

    pub(crate) fn display_path(&self, path: &Path) -> String {
        self.display_path_with_context(path, None)
    }

    pub(crate) fn display_path_with_context(
        &self,
        path: &Path,
        session_context: Option<&crate::session::SessionExecutionContext>,
    ) -> String {
        let workspace_root = self.effective_workspace_root(session_context);
        if let Ok(relative) = path.strip_prefix(workspace_root) {
            let normalized = normalize_path_for_display(relative);
            if normalized.is_empty() {
                return ".".to_string();
            }
            return normalized;
        }
        normalize_path_for_display(path)
    }

    pub(crate) fn ensure_read_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Read, target_path)
    }

    pub(crate) fn ensure_edit_permission(&self, target_path: &Path) -> Result<(), ToolError> {
        self.ensure_access_permission(AccessKind::Write, target_path)
    }

    pub(crate) fn ensure_filesystem_effects_permission(
        &self,
        effects: &[FilesystemEffect],
        base_path: &Path,
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target = self.resolve_filesystem_effect_path(effect.path.as_str(), base_path);
            if effect.access.includes_read() {
                self.ensure_access_permission(AccessKind::Read, &target)?;
            }
            if effect.access.includes_write() {
                self.ensure_access_permission(AccessKind::Write, &target)?;
            }
        }
        Ok(())
    }

    pub(crate) fn ensure_network_effects_permission(
        &self,
        effects: &[NetworkEffect],
    ) -> Result<(), ToolError> {
        for effect in effects {
            let target = NetworkTarget::parse(effect.target.as_str()).map_err(|err| {
                ToolError::InvalidInput(format!(
                    "invalid network effect target `{}`: {err}",
                    effect.target
                ))
            })?;
            self.ensure_network_permission(&target)?;
        }
        Ok(())
    }

    pub(crate) fn ensure_network_permission(
        &self,
        target: &NetworkTarget,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        match self.agent.authorize_network_connect(target) {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionEnforcementMode::Bypassed {
            return Ok(());
        }

        match self
            .agent
            .authorize_path_access(access, self.workspace_root(), target_path)
        {
            PermissionDecision::Allow => Ok(()),
            PermissionDecision::Ask { reason } => Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => Err(ToolError::PermissionDenied(reason)),
        }
    }

    fn push_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        access: AccessKind,
        target_path: &Path,
    ) {
        let workspace_root = normalize_path_for_display(self.workspace_root());
        let target = normalize_path_for_display(target_path);

        checks.push(ToolPermissionCheck {
            action: PermissionAction::PathAccess {
                access_kind: access_kind_name(access).to_string(),
                workspace_root,
                target_path: target,
            },
            decision: self
                .agent
                .authorize_path_access(access, self.workspace_root(), target_path),
        });
    }

    fn push_network_check(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        target: &str,
    ) -> Result<(), ToolError> {
        let target = NetworkTarget::parse(target).map_err(|err| {
            ToolError::InvalidInput(format!(
                "invalid network permission target `{target}`: {err}"
            ))
        })?;
        checks.push(ToolPermissionCheck {
            action: PermissionAction::NetworkAccess {
                target: target.original().to_string(),
                host: target.host().to_string(),
                port: target.port(),
            },
            decision: self.agent.authorize_network_connect(&target),
        });
        Ok(())
    }

    pub(crate) fn network_permission_check(
        &self,
        target: &str,
    ) -> Result<ToolPermissionCheck, ToolError> {
        let mut checks = Vec::with_capacity(1);
        self.push_network_check(&mut checks, target)?;
        Ok(checks.remove(0))
    }
}

pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn truncate_to_char_count(value: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let Some((idx, _)) = value.char_indices().nth(max_chars) else {
        return value.to_string();
    };
    value[..idx].to_string()
}

fn model_output_boundary_context(execution: &ToolInvocationExecution) -> String {
    let output_text = execution.view.output_text.as_str();
    let payload_text = execution
        .output
        .to_json_payload()
        .and_then(|payload| serde_json::to_string_pretty(&payload).ok())
        .unwrap_or_default();

    if payload_text.len() > output_text.len() {
        payload_text
    } else {
        output_text.to_string()
    }
}

fn model_output_exceeds_boundary(value: &str, max_lines: usize, max_bytes: usize) -> bool {
    line_count(value) > max_lines || value.len() > max_bytes
}

fn line_count(value: &str) -> usize {
    value.bytes().filter(|byte| *byte == b'\n').count() + usize::from(!value.is_empty())
}

fn bounded_model_output_preview(
    value: &str,
    marker: &str,
    max_lines: usize,
    max_bytes: usize,
) -> String {
    let mut selected = value
        .lines()
        .take(max_lines.saturating_sub(1))
        .collect::<Vec<_>>()
        .join("\n");
    selected = truncate_to_utf8_bytes(
        selected.as_str(),
        max_bytes.saturating_sub(marker.len() + 2),
    );
    if selected.trim().is_empty() {
        marker.to_string()
    } else {
        format!("{selected}\n{marker}")
    }
}

fn truncate_to_utf8_bytes(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    if max_bytes == 0 {
        return String::new();
    }
    let mut end = max_bytes.min(value.len());
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

fn persist_tool_result_output(
    workspace_root: &Path,
    model_tool_name: &str,
    call_id: i64,
    output_text: &str,
) -> Result<Option<PathBuf>, ToolError> {
    if output_text.is_empty() {
        return Ok(None);
    }

    let dir = workspace_root.join(".agena").join("tool-results");
    fs::create_dir_all(&dir)?;
    let digest = blake3::hash(output_text.as_bytes()).to_hex().to_string();
    let short_digest = digest.get(..12).unwrap_or(digest.as_str());
    let safe_tool = model_safe_tool_name(model_tool_name).replace("__", "_");
    let call_part = if call_id >= 0 {
        call_id.to_string()
    } else {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_millis().to_string())
            .unwrap_or_else(|_| "synthetic".to_string())
    };
    let path = dir.join(format!("{call_part}-{safe_tool}-{short_digest}.txt"));
    let mut file = fs::File::create(&path)?;
    file.write_all(output_text.as_bytes())?;
    Ok(Some(path))
}

fn access_kind_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
    }
}

fn validate_shell_filesystem_effects(
    tool_name: &str,
    command: &str,
    effects: &[FilesystemEffect],
) -> Result<(), ToolError> {
    shell_tools::validate_declared_filesystem_effects(tool_name, command, effects)
}

fn shell_command_from_invocation(invocation: &ToolInvocation) -> Option<String> {
    if let Some(payload) = ToolPayloadInput::from_invocation(invocation) {
        let command = match payload {
            ToolPayloadInput::Process(crate::message::ProcessToolInput::Run {
                command, ..
            }) => Some(command.command),
            _ => None,
        };
        if command.is_some() {
            return command;
        }
    }
    let value = invocation_input_value(invocation);
    value
        .get("command")
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|command| !command.is_empty())
        .map(str::to_string)
}

fn filesystem_effects_from_input(
    input: &serde_json::Value,
) -> Result<Option<Vec<FilesystemEffect>>, ToolError> {
    let Some(value) = input
        .get("filesystem_effects")
        .or_else(|| input.pointer("/args/filesystem_effects"))
    else {
        return Ok(None);
    };
    let effects = serde_json::from_value(value.clone())
        .map_err(|err| ToolError::InvalidInput(format!("filesystem_effects: {err}")))?;
    Ok(Some(effects))
}

fn invocation_name(invocation: &ToolInvocation) -> String {
    plugin_invocation_name(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_name(invocation: &PluginInvocation) -> String {
    invocation.tool_name.clone()
}

fn canonical_tool_name(name: &str) -> &str {
    name
}

fn command_from_input_value(input: &serde_json::Value) -> Option<&str> {
    input.get("action").and_then(serde_json::Value::as_str)
}

fn resolved_tool_input_value(
    _registered_tool: &RegisteredTool,
    invocation: &ToolInvocation,
) -> serde_json::Value {
    invocation_input_value(invocation)
}

fn resolved_plugin_invocation_input_value(
    _registered_tool: &RegisteredTool,
    invocation: &PluginInvocation,
) -> serde_json::Value {
    plugin_invocation_input_value(invocation)
}

fn resolve_managed_project_path_alias(raw_path: &str, workspace_root: &Path) -> Option<PathBuf> {
    let normalized = raw_path.trim().replace('\\', "/");
    let prefix = "~/agena/projects/<workspace>";
    let rest = normalized.strip_prefix(prefix)?;
    let rest = rest.trim_start_matches('/');
    let mut resolved = crate::project_paths::project_state_dir(workspace_root);
    if !rest.is_empty() {
        resolved = resolved.join(rest);
    }
    Some(resolved)
}

fn invocation_effective_tags(
    definition: &RegisteredTool,
    invocation: &ToolInvocation,
) -> Vec<crate::plugin::sdk::ToolTag> {
    let mut tags = definition.effective_tags();
    let input = resolved_tool_input_value(definition, invocation);
    let Some(command) = command_from_input_value(&input) else {
        return tags;
    };

    let behavior_model_name = definition.tool_name.as_str();
    match (behavior_model_name, command) {
        ("fs" | "agena_fs__fs", "read" | "glob" | "grep") => {
            set_invocation_access_tags(&mut tags, true, false, true, false)
        }
        ("fs" | "agena_fs__fs", "apply_patch") => {
            set_invocation_access_tags(&mut tags, false, true, false, true)
        }
        ("settings", "get" | "list" | "validate") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("settings", "set" | "delete" | "patch") => {
            set_invocation_access_tags(&mut tags, false, true, false, true)
        }
        ("schedule", "list") => set_invocation_access_tags(&mut tags, true, false, false, false),
        ("schedule", "create" | "delete" | "wakeup") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("process", "list" | "logs") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("process", "run" | "stop") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("session", "get") => set_invocation_access_tags(&mut tags, true, false, false, false),
        ("session", "rename") => set_invocation_access_tags(&mut tags, false, true, false, false),
        (
            "resources.list" | "resources.read" | "prompts.list" | "prompts.get" | "agena_mcp__mcp",
            "list_resources" | "read_resource" | "list_prompts" | "get_prompt",
        ) => set_invocation_access_tags(&mut tags, true, false, false, false),
        ("tools.call" | "agena_mcp__mcp", "call") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        _ => {}
    }

    tags
}

fn set_invocation_access_tags(
    tags: &mut Vec<crate::plugin::sdk::ToolTag>,
    read_only: bool,
    mutating: bool,
    filesystem_read: bool,
    filesystem_write: bool,
) {
    tags.retain(|tag| {
        !matches!(
            tag,
            crate::plugin::sdk::ToolTag::ReadOnly
                | crate::plugin::sdk::ToolTag::Mutating
                | crate::plugin::sdk::ToolTag::FilesystemRead
                | crate::plugin::sdk::ToolTag::FilesystemWrite
        )
    });
    if read_only {
        tags.push(crate::plugin::sdk::ToolTag::ReadOnly);
    }
    if mutating {
        tags.push(crate::plugin::sdk::ToolTag::Mutating);
    }
    if filesystem_read {
        tags.push(crate::plugin::sdk::ToolTag::FilesystemRead);
    }
    if filesystem_write {
        tags.push(crate::plugin::sdk::ToolTag::FilesystemWrite);
    }
}

fn is_concurrency_safe_tool_invocation(
    registered_tool: &RegisteredTool,
    invocation: &PluginInvocation,
) -> bool {
    let input = resolved_plugin_invocation_input_value(registered_tool, invocation);
    let Some(command) = command_from_input_value(&input) else {
        return registered_tool.definition.runtime.concurrency_safe;
    };

    let behavior_model_name = registered_tool.tool_name.as_str();
    match (behavior_model_name, command) {
        ("fs" | "agena_fs__fs", "read" | "glob" | "grep") => true,
        ("fs" | "agena_fs__fs", "apply_patch") => false,
        ("process", "list" | "logs") => true,
        ("process", "run" | "stop") => false,
        ("settings", "get" | "list" | "validate") => true,
        ("settings", "set" | "delete" | "patch") => false,
        ("schedule", "list") => true,
        ("schedule", "create" | "delete" | "wakeup") => false,
        ("session", "get") => true,
        ("session", "rename") => false,
        (
            "resources.list" | "resources.read" | "prompts.list" | "prompts.get" | "agena_mcp__mcp",
            "list_resources" | "read_resource" | "list_prompts" | "get_prompt",
        ) => true,
        ("tools.call" | "agena_mcp__mcp", "call") => false,
        _ => registered_tool.definition.runtime.concurrency_safe,
    }
}

fn apply_patch_execution_from_tool_output(output: &ToolOutput) -> Option<ApplyPatchExecution> {
    let payload = output.to_json_payload()?;
    let operation_id = payload.get("operation_id")?.as_str()?.to_string();
    let changes: Vec<crate::message::FileChangeRecord> =
        serde_json::from_value(payload.get("changes")?.clone()).ok()?;
    let before_hash = payload
        .get("before_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let after_hash = payload
        .get("after_hash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let inverse_patch = payload.get("inverse_patch")?.as_str()?.to_string();
    let diff = payload
        .get("diff")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_default()
        .to_string();
    let progress = serde_json::from_value(
        payload
            .get("progress")
            .cloned()
            .unwrap_or_else(|| serde_json::json!([])),
    )
    .ok()?;
    Some(ApplyPatchExecution {
        operation_id,
        files: changes
            .into_iter()
            .map(|change| AppliedFileChange {
                path: change.path,
                kind: match change.kind {
                    crate::message::FileChangeKind::Added => apply_patch::PatchOpKind::Add,
                    crate::message::FileChangeKind::Updated => apply_patch::PatchOpKind::Update,
                    crate::message::FileChangeKind::Deleted => apply_patch::PatchOpKind::Delete,
                    crate::message::FileChangeKind::Moved => apply_patch::PatchOpKind::Move,
                },
                from_path: change.from_path,
            })
            .collect(),
        before_hash,
        after_hash,
        inverse_patch,
        diff,
        progress,
    })
}

fn invocation_input_json(invocation: &ToolInvocation) -> Result<String, ToolError> {
    plugin_invocation_input_json(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_input_json(invocation: &PluginInvocation) -> Result<String, ToolError> {
    serde_json::to_string(&serde_json::Value::from(invocation.input.clone()))
        .map_err(|err| ToolError::InvalidInput(err.to_string()))
}

fn invocation_input_value(invocation: &ToolInvocation) -> serde_json::Value {
    plugin_invocation_input_value(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_input_value(invocation: &PluginInvocation) -> serde_json::Value {
    serde_json::Value::from(invocation.input.clone())
}

fn parse_invocation_from_json(
    tool_name: &str,
    input_json: &str,
) -> Result<ToolInvocation, ToolError> {
    let value = if input_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(input_json).map_err(|err| ToolError::InvalidInput(err.to_string()))?
    };
    let input = StructuredObject::try_from(value)
        .map_err(|err| ToolError::InvalidInput(err.to_string()))?;

    Ok(ToolInvocation {
        name: tool_name.to_string(),
        plugin_name: None,
        input,
    })
}

fn sdk_path_kind_to_access_kind(kind: SdkPathKind) -> AccessKind {
    match kind {
        SdkPathKind::Read => AccessKind::Read,
        SdkPathKind::Write => AccessKind::Write,
    }
}

fn extract_input_path_requests(
    input: &serde_json::Value,
    specs: &[SdkInputPathSpec],
) -> Result<Vec<crate::plugin::sdk::PathRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if spec.optional {
                continue;
            }
            return Err(ToolError::InvalidInput(format!(
                "missing required input path '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(path) = value.as_str() else {
                return Err(ToolError::InvalidInput(format!(
                    "input path '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(crate::plugin::sdk::PathRequest {
                path: path.to_string(),
                kind: spec.kind,
            });
        }
    }
    Ok(requests)
}

fn extract_input_network_requests(
    input: &serde_json::Value,
    specs: &[SdkInputNetworkSpec],
) -> Result<Vec<crate::plugin::sdk::NetworkRequest>, ToolError> {
    let mut requests = Vec::new();
    for spec in specs {
        let matches = extract_jsonpath_values(input, spec.jsonpath.as_str())?;
        if matches.is_empty() {
            if spec.optional {
                continue;
            }
            return Err(ToolError::InvalidInput(format!(
                "missing required input network '{}'",
                spec.jsonpath
            )));
        }
        for value in matches {
            let Some(target) = value.as_str() else {
                return Err(ToolError::InvalidInput(format!(
                    "input network '{}' must resolve to a string",
                    spec.jsonpath
                )));
            };
            requests.push(crate::plugin::sdk::NetworkRequest {
                target: target.to_string(),
            });
        }
    }
    Ok(requests)
}

fn extract_jsonpath_values<'a>(
    input: &'a serde_json::Value,
    jsonpath: &str,
) -> Result<Vec<&'a serde_json::Value>, ToolError> {
    let segments = parse_input_jsonpath(jsonpath)?;
    let mut current = vec![input];
    for segment in segments {
        let mut next = Vec::new();
        for value in current {
            match segment {
                InputJsonPathSegment::Key(ref key) => {
                    if let Some(object) = value.as_object()
                        && let Some(child) = object.get(key.as_str())
                    {
                        next.push(child);
                    }
                }
                InputJsonPathSegment::ArrayAll => {
                    if let Some(items) = value.as_array() {
                        next.extend(items.iter());
                    }
                }
            }
        }
        current = next;
        if current.is_empty() {
            break;
        }
    }
    Ok(current)
}

fn parse_input_jsonpath(jsonpath: &str) -> Result<Vec<InputJsonPathSegment>, ToolError> {
    if jsonpath == "$" {
        return Ok(Vec::new());
    }
    let Some(mut rest) = jsonpath.strip_prefix("$.") else {
        return Err(ToolError::InvalidInput(format!(
            "unsupported input path jsonpath '{jsonpath}'"
        )));
    };

    let mut segments = Vec::new();
    while !rest.is_empty() {
        let key_end = rest.find(['.', '[']).unwrap_or(rest.len());
        let key = &rest[..key_end];
        if key.is_empty() {
            return Err(ToolError::InvalidInput(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        }
        segments.push(InputJsonPathSegment::Key(key.to_string()));
        rest = &rest[key_end..];

        while let Some(tail) = rest.strip_prefix("[*]") {
            segments.push(InputJsonPathSegment::ArrayAll);
            rest = tail;
        }

        if rest.is_empty() {
            break;
        }
        let Some(tail) = rest.strip_prefix('.') else {
            return Err(ToolError::InvalidInput(format!(
                "unsupported input path jsonpath '{jsonpath}'"
            )));
        };
        rest = tail;
    }

    Ok(segments)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum InputJsonPathSegment {
    Key(String),
    ArrayAll,
}
