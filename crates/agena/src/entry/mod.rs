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
pub(crate) mod monitor_tool;
pub(crate) mod notebook_edit;
pub(crate) mod orchestrator;
pub(crate) mod payload;
pub(crate) mod plan;
pub(crate) mod powershell;
pub(crate) mod read;
pub(crate) mod result;
pub(crate) mod shell;
pub(crate) mod task;
pub(crate) mod todo_write;
pub(crate) mod tool_search;
pub(crate) mod truncation;
pub(crate) mod web_fetch;
pub(crate) mod web_search;
pub(crate) mod worktree;

use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicI64, Ordering},
};

use thiserror::Error;

use crate::agent::Agent;
use crate::message::{
    AskUserToolInput, FilesystemEffect, Message, PartContent, PluginInvocation, StructuredObject,
    ToolInvocation, ToolOutput,
};
use crate::permission::{AccessKind, NetworkTarget, PermissionAction, PermissionDecision};
use crate::plugin::{
    PluginHost, PluginHostBuilder, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolDefinitionInput as PluginEntryDefinitionInput,
    ToolFailureInput as PluginToolFailureInput, ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionNetworksInput as PluginToolPermissionNetworksInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    registry::PluginEntry as RegistryPluginEntry,
    sdk::{
        InputNetworkSpec as SdkInputNetworkSpec, InputPathSpec as SdkInputPathSpec,
        NetworkAccessSpec as SdkNetworkAccessSpec, PathAccessSpec as SdkPathAccessSpec,
        PathKind as SdkPathKind, ShellEnvInput as PluginShellEnvInput,
        ToolStreamingMode as SdkEntryStreamingMode,
    },
};
use crate::plugins::provided::{
    cron as provided_cron, fs as provided_fs, lsp as provided_lsp, mcp,
    router as in_process_router, settings as provided_settings, shell as provided_shell, skills,
    web as provided_web, workflow as provided_workflow,
};

pub use crate::plugin::sdk::ToolLoadPriority;
pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use payload::{CronJobSummary, ToolPayloadInput, ToolPayloadOutput, WebSearchHit};
pub use plan::{PlanRegistry, registry_for_executor as plan_registry_for_executor};
pub use result::{ToolExecutionView, ToolInvocationExecution, ToolPayloadExecution};
pub use shell::{ShellError, ShellOutput, ShellRequest};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};
pub use worktree::{
    ActiveWorktree, ManagedWorktree, WorktreeRegistry, list_active as worktree_list_active,
    list_managed as worktree_list_managed, prune_stale as worktree_prune_stale,
    registry_for_executor as worktree_registry_for_executor,
};

pub fn skills_plugin_id() -> &'static str {
    skills::SKILLS_PLUGIN_ID
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

pub fn web_plugin_id() -> &'static str {
    provided_web::WEB_PLUGIN_ID
}

pub fn new_web_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_web::new_plugin()
}

pub fn workflow_plugin_id() -> &'static str {
    provided_workflow::WORKFLOW_PLUGIN_ID
}

pub fn new_workflow_plugin() -> impl crate::plugin::sdk::Plugin {
    provided_workflow::new_plugin()
}

pub fn default_tool_host(workspace_root: impl Into<PathBuf>) -> Result<Arc<PluginHost>, String> {
    let workspace_root = workspace_root.into();
    let skills_id = skills_plugin_id().to_string();
    let lsp_id = lsp_plugin_id().to_string();
    let cron_id = cron_plugin_id().to_string();
    let fs_id = fs_plugin_id().to_string();
    let settings_id = settings_plugin_id().to_string();
    let shell_id = shell_plugin_id().to_string();
    let web_id = web_plugin_id().to_string();
    let workflow_id = workflow_plugin_id().to_string();
    let mut list = std::collections::BTreeMap::new();
    for id in [
        &skills_id,
        &lsp_id,
        &cron_id,
        &fs_id,
        &settings_id,
        &shell_id,
        &web_id,
        &workflow_id,
    ] {
        list.insert(
            (*id).clone(),
            crate::plugin::PluginEntry::Static {
                options: serde_json::Value::Null,
                timeouts: Default::default(),
                disabled: false,
            },
        );
    }
    let config = crate::plugin::PluginsConfig {
        enabled: true,
        timeouts: Default::default(),
        list,
        trusted_keys: Default::default(),
        default_quota: Default::default(),
        quotas: Default::default(),
        tool_presentation: Default::default(),
    };
    mcp::block_on(async move {
        PluginHostBuilder::new(workspace_root, env!("CARGO_PKG_VERSION"))
            .with_config(config)
            .register_static(skills_id, new_skills_plugin())
            .register_static(lsp_id, new_lsp_plugin())
            .register_static(cron_id, new_cron_plugin())
            .register_static(fs_id, new_fs_plugin())
            .register_static(settings_id, new_settings_plugin())
            .register_static(shell_id, new_shell_plugin())
            .register_static(web_id, new_web_plugin())
            .register_static(workflow_id, new_workflow_plugin())
            .build()
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
    #[error("unknown tool: {0}")]
    UnknownTool(String),
    #[error("unsupported tool invocation in executor: {0}")]
    UnsupportedInvocation(String),
}

fn present_tool_entry(
    mut entry: RegistryPluginEntry,
    presentation: &crate::plugin::ToolPresentationConfig,
) -> RegistryPluginEntry {
    let mode = presentation.mode_for(
        entry.plugin_name.as_str(),
        entry.original_name.as_str(),
        entry.exposed_name.as_str(),
        entry.decl.description_mode,
    );
    if mode == crate::plugin::ToolDescriptionMode::Help {
        entry.decl.description = Some(compact_tool_description(&entry));
    }
    entry.decl.help = None;
    entry
}

fn compact_tool_description(entry: &RegistryPluginEntry) -> String {
    format!(
        "{} Full usage is available from the `tools` tool: call action `help` with tool `{}`.",
        tool_summary(entry),
        entry.exposed_name
    )
}

fn tool_summary(entry: &RegistryPluginEntry) -> String {
    if let Some(summary) = entry.summary_text() {
        return summary.to_string();
    }
    entry
        .description_text()
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(ToString::to_string)
        .unwrap_or_else(|| format!("Tool `{}`.", entry.exposed_name))
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
    web_search_backend: crate::config::WebSearchBackend,
    plan_registry: Option<plan::PlanRegistry>,
    worktree_registry: Option<worktree::WorktreeRegistry>,
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
            web_search_backend: crate::config::WebSearchBackend::DuckDuckGoHtml,
            plan_registry: None,
            worktree_registry: None,
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

    pub fn with_web_search_backend(mut self, backend: crate::config::WebSearchBackend) -> Self {
        self.web_search_backend = backend;
        self
    }

    pub fn web_search_backend(&self) -> crate::config::WebSearchBackend {
        self.web_search_backend.clone()
    }

    pub fn with_plan_registry(mut self, reg: plan::PlanRegistry) -> Self {
        self.plan_registry = Some(reg);
        self
    }

    pub fn plan_registry(&self) -> Option<&plan::PlanRegistry> {
        self.plan_registry.as_ref()
    }

    pub fn with_worktree_registry(mut self, reg: worktree::WorktreeRegistry) -> Self {
        self.worktree_registry = Some(reg);
        self
    }

    pub fn worktree_registry(&self) -> Option<&worktree::WorktreeRegistry> {
        self.worktree_registry.as_ref()
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

    /// Refuse mutating invocations while plan mode is active for this
    /// session.  Returns Ok when the invocation is safe (read-only) or
    /// when plan mode is off.  Returns `PermissionDenied` otherwise.
    pub fn enforce_plan_mode_for(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
    ) -> Result<(), ToolError> {
        let Some(reg) = self.plan_registry() else {
            return Ok(());
        };
        if !reg.read().contains_key(&session_id) {
            return Ok(());
        }

        let tool_name = invocation_name(invocation);
        let tags = self
            .invocation_definition(invocation)
            .map(|entry| invocation_effective_tags(&entry, invocation))
            .unwrap_or_default();

        if tags
            .iter()
            .any(|tag| tag == &crate::plugin::sdk::ToolTag::ReadOnly)
        {
            return Ok(());
        }

        if tags
            .iter()
            .any(|tag| tag == &crate::plugin::sdk::ToolTag::Shell)
            && shell_command_from_invocation(invocation)
                .as_deref()
                .is_some_and(bash::is_read_only_command)
        {
            return Ok(());
        }

        Err(ToolError::PermissionDenied(format!(
            "tool '{tool_name}' is blocked in plan mode; call exit_plan_mode first"
        )))
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
        if !session_context.agent_permission.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_permission_config(&session_context.agent_permission);
        }
        if !session_context.allowed_tools.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .with_allowed_tools(session_context.allowed_tools.iter().map(String::as_str));
        }
        if let Some(model_id) = session_context.model_id.as_ref() {
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

    fn catalogued_tools_raw(&self) -> Vec<RegistryPluginEntry> {
        let catalog = self.tool_catalog();
        let mut tools = self
            .plugins
            .entry_entries()
            .into_iter()
            .filter(|entry| catalog.is_tool_enabled(entry))
            .collect::<Vec<_>>();

        tools.sort_by(|left, right| {
            left.exposed_name
                .cmp(&right.exposed_name)
                .then_with(|| left.description_text().cmp(right.description_text()))
        });

        // Plugin chain: tool.definition. Let plugins rewrite descriptions /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            tools = tools
                .into_iter()
                .map(|mut entry| {
                    let input = PluginEntryDefinitionInput {
                        tool_name: entry.exposed_name.clone(),
                        plugin_name: entry.plugin_name.clone(),
                        description: entry.description_text().to_string(),
                        summary: entry.decl.summary.clone(),
                        help: entry.decl.help.clone(),
                        description_mode: entry.decl.description_mode,
                        input_schema: entry.sanitized_input_schema(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => {
                            entry.decl.description = Some(patched.description);
                            entry.decl.summary = patched.summary;
                            entry.decl.help = patched.help;
                            entry.decl.description_mode = patched.description_mode;
                            entry.decl.input_schema = patched.input_schema;
                            entry
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %entry.exposed_name,
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

    fn catalogued_tools(&self) -> Vec<RegistryPluginEntry> {
        self.catalogued_tools_raw()
            .into_iter()
            .map(|entry| present_tool_entry(entry, &self.tool_presentation))
            .collect()
    }

    pub fn detailed_tools(&self) -> Vec<RegistryPluginEntry> {
        self.catalogued_tools_raw()
    }

    pub fn searchable_tools(&self) -> Vec<RegistryPluginEntry> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<RegistryPluginEntry> {
        self.catalogued_tools()
            .into_iter()
            .filter(RegistryPluginEntry::should_load_by_default)
            .collect()
    }

    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let invocation = PluginInvocation::from_tool_invocation(invocation);
        let Some(entry) = self.plugin_invocation_definition(&invocation) else {
            return false;
        };
        entry.decl.concurrency_safe
            && !entry.has_tag(crate::plugin::sdk::ToolTag::Interactive)
            && is_concurrency_safe_entry_invocation(&entry, &invocation)
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<RegistryPluginEntry> {
        self.available_tools_for_messages_and_loaded(messages, &[])
    }

    pub fn available_tools_for_messages_and_loaded(
        &self,
        messages: &[Message],
        loaded_tools: &[String],
    ) -> Vec<RegistryPluginEntry> {
        let loaded_tools = collect_loaded_tool_names(messages, loaded_tools);
        self.catalogued_tools()
            .into_iter()
            .filter(|entry| {
                entry.should_load_by_default() || loaded_tools.contains(entry.exposed_name.as_str())
            })
            .collect()
    }

    fn invocation_definition(&self, invocation: &ToolInvocation) -> Option<RegistryPluginEntry> {
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_definition(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<RegistryPluginEntry> {
        self.catalogued_tools()
            .into_iter()
            .find(|entry| entry.exposed_name == invocation.entry_name)
            .or_else(|| {
                let canonical = canonical_entry_name(invocation.entry_name.as_str());
                self.catalogued_tools()
                    .into_iter()
                    .find(|entry| entry.exposed_name == canonical)
            })
    }

    fn invocation_plugin_name_for(&self, invocation: &ToolInvocation) -> String {
        self.plugin_invocation_plugin_name_for(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_plugin_name_for(&self, invocation: &PluginInvocation) -> String {
        if let Some(entry) = self.plugin_invocation_definition(invocation) {
            return entry.plugin_name;
        }

        self.plugins
            .lookup_entry(invocation.entry_name.as_str())
            .map(|entry| entry.plugin_name)
            .unwrap_or_else(|| "custom".to_string())
    }

    fn invocation_streaming_mode(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<SdkEntryStreamingMode> {
        self.plugin_invocation_streaming_mode(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_streaming_mode(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<SdkEntryStreamingMode> {
        self.plugin_resolution_for_plugin_invocation(invocation)
            .map(|entry| entry.decl.streaming)
    }

    fn authorize_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<(String, PermissionDecision), ToolError> {
        let tool_name = invocation_name(invocation);
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| ToolError::UnknownTool(tool_name.clone()))?;
        let tags = invocation_effective_tags(&definition, invocation);
        if !self.tool_catalog().are_tags_enabled(&tags) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
            )));
        }
        let command = shell_command_from_invocation(invocation);
        Ok((
            tool_name.clone(),
            self.agent
                .authorize_tool(tool_name.as_str(), command.as_deref(), &tags),
        ))
    }

    fn plugin_resolution_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<crate::plugin::registry::PluginEntry> {
        self.plugin_resolution_for_plugin_invocation(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<crate::plugin::registry::PluginEntry> {
        self.plugins
            .lookup_entry(invocation.entry_name.as_str())
            .or_else(|| {
                self.plugins
                    .lookup_entry(canonical_entry_name(invocation.entry_name.as_str()))
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
        entry: &crate::plugin::registry::PluginEntry,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_paths(
            entry,
            PluginToolPermissionPathsInput {
                tool_name: entry.original_name.clone(),
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
        entry: &crate::plugin::registry::PluginEntry,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_networks(
            entry,
            PluginToolPermissionNetworksInput {
                tool_name: entry.original_name.clone(),
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
            .ok_or_else(|| ToolError::UnknownTool(tool_name.to_string()))?;
        if !scoped_executor.tool_catalog().is_tool_enabled(&definition) {
            return Err(ToolError::UnsupportedInvocation(tool_name.to_string()));
        }

        if scoped_executor.permission_mode == PermissionEnforcementMode::Enforced {
            for check in scoped_executor.collect_permission_checks_for_invocation(&invocation)? {
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
        self.collect_permission_checks_for_invocation(&input.clone().into_invocation())
    }

    pub fn prepare_shell_command(
        &self,
        input: &crate::message::BashToolInput,
        session_id: i64,
        call_id: i64,
    ) -> Result<Option<PreparedShellCommand>, ToolError> {
        bash::prepare_command(self, input, session_id, call_id)
    }

    pub fn prepare_bash_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<(ToolInvocation, Option<PreparedShellCommand>), ToolError> {
        let Some(ToolPayloadInput::Bash(bash_input)) =
            ToolPayloadInput::from_invocation(invocation)
        else {
            return Ok((invocation.clone(), None));
        };
        let prepared_shell = self.prepare_shell_command(&bash_input, session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == bash_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = bash_input;
        rewritten.command = prepared_shell.command.clone();
        let input_value = if invocation.name.rsplit('/').next() == Some("bash") {
            serde_json::to_value(rewritten)
                .map_err(|err| ToolError::InvalidInput(format!("bash input: {err}")))?
        } else {
            let rewritten_invocation = ToolPayloadInput::Bash(rewritten).into_invocation();
            serde_json::Value::from(rewritten_invocation.input)
        };
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
        let tool_name = invocation_name(invocation).to_owned();
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let input_json = invocation_input_json(invocation)?;
        let input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let hooked = self
            .plugins
            .dispatch_tool_before(PluginToolBeforeInput {
                tool_name: tool_name.clone(),
                plugin_name: plugin_name.clone(),
                session_id,
                call_id,
                workspace_root: self.workspace_root.to_string_lossy().to_string(),
                input: input_value,
                title_override: None,
                metadata: Default::default(),
            })
            .map_err(|err| ToolError::Plugin(err.message))?;

        let input_json = serde_json::to_string(&hooked.input)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let mut prepared_invocation =
            parse_invocation_from_json(tool_name.as_str(), input_json.as_str())?;
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
        let (tool_name, decision) = self.authorize_invocation(invocation)?;
        let command = shell_command_from_invocation(invocation);
        let action = crate::permission::tool_action(
            tool_name.as_str(),
            command.as_deref(),
            Some(&self.agent.tool_policy),
        );
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        let input_value = invocation_input_value(invocation);
        if let Some(resolution) = self.plugin_resolution_for_invocation(invocation) {
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
                &resolution.decl.input_paths,
                &resolution.decl.path_access,
            )?;
            self.collect_dynamic_path_checks(&mut checks, &resolution, &input_value)?;
            self.collect_declared_network_checks(
                &mut checks,
                &input_value,
                &resolution.decl.input_networks,
                &resolution.decl.network_access,
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
            Some(SdkEntryStreamingMode::Streaming)
        ) {
            return Ok(None);
        }
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);

        let resolution = self
            .plugin_resolution_for_plugin_invocation(&plugin_invocation)
            .ok_or_else(|| ToolError::UnknownTool(plugin_invocation.entry_name.clone()))?;
        let _executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.original_name.clone(),
        );
        let stream = self
            .plugins
            .invoke_tool_stream(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.original_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: plugin_invocation_input_value(&plugin_invocation),
                },
            )
            .await
            .map_err(|err| ToolError::Plugin(err.message))?;
        let stream_id = stream.stream_id;
        let chunks = stream.chunks;
        let end = stream.end;
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
            .ok_or_else(|| ToolError::UnknownTool(plugin_invocation.entry_name.clone()))?;
        let _executor_guard = in_process_router::install_executor_context(
            self,
            session_id,
            call_id,
            resolution.original_name.clone(),
        );

        let response = self
            .plugins
            .invoke_tool(
                &resolution,
                PluginToolInvokeInput {
                    tool_name: resolution.original_name.clone(),
                    session_id,
                    call_id,
                    workspace_root: self.workspace_root.to_string_lossy().to_string(),
                    input: plugin_invocation_input_value(&plugin_invocation),
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
        let tool_name = invocation_name(invocation).to_owned();
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let after_in = PluginToolAfterInput {
            tool_name,
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
        let tool_name = invocation_name(invocation).to_owned();
        let plugin_name = self.invocation_plugin_name_for(invocation);
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool_name,
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
        let candidate = PathBuf::from(raw_path);
        if candidate.is_absolute() {
            return candidate;
        }
        self.effective_workspace_root(session_context)
            .join(candidate)
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
    if effects.is_empty()
        && let Some(reason) = bash::mutating_command_reason(command)
    {
        return Err(ToolError::InvalidInput(format!(
            "{tool_name} filesystem_effects must declare at least one path because the command appears to modify files: {reason}"
        )));
    }
    Ok(())
}

fn shell_command_from_invocation(invocation: &ToolInvocation) -> Option<String> {
    if let Some(payload) = ToolPayloadInput::from_invocation(invocation) {
        let command = match payload {
            ToolPayloadInput::Bash(payload) => Some(payload.command),
            ToolPayloadInput::PowerShell(payload) => Some(payload.command),
            ToolPayloadInput::Monitor(crate::message::MonitorToolInput::Start {
                command, ..
            }) => Some(command),
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
    invocation.entry_name.clone()
}

fn canonical_entry_name(name: &str) -> &str {
    name
}

fn command_from_input(input: &StructuredObject) -> Option<&str> {
    input
        .get("action")
        .and_then(crate::message::StructuredValue::as_text)
}

fn invocation_effective_tags(
    definition: &RegistryPluginEntry,
    invocation: &ToolInvocation,
) -> Vec<crate::plugin::sdk::ToolTag> {
    let mut tags = definition.effective_tags();
    let Some(command) = command_from_input(&invocation.input) else {
        return tags;
    };

    match (definition.exposed_name.as_str(), command) {
        ("fs", "read" | "glob" | "grep") => {
            set_invocation_access_tags(&mut tags, true, false, true, false)
        }
        ("fs", "apply_patch" | "notebook_edit") => {
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
        ("session", "get") => set_invocation_access_tags(&mut tags, true, false, false, false),
        ("session", "rename") => set_invocation_access_tags(&mut tags, false, true, false, false),
        ("goal", "get") => set_invocation_access_tags(&mut tags, true, false, false, false),
        ("goal", "create" | "clear" | "complete") => {
            set_invocation_access_tags(&mut tags, false, true, false, false)
        }
        ("mcp", "list_resources" | "read_resource" | "list_prompts" | "get_prompt") => {
            set_invocation_access_tags(&mut tags, true, false, false, false)
        }
        ("mcp", "call") => set_invocation_access_tags(&mut tags, false, true, false, false),
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

fn is_concurrency_safe_entry_invocation(
    entry: &RegistryPluginEntry,
    invocation: &PluginInvocation,
) -> bool {
    let Some(command) = command_from_input(&invocation.input) else {
        return entry.decl.concurrency_safe;
    };

    match (entry.exposed_name.as_str(), command) {
        ("fs", "read" | "glob" | "grep") => true,
        ("fs", "apply_patch" | "notebook_edit") => false,
        ("settings", "get" | "list" | "validate") => true,
        ("settings", "set" | "delete" | "patch") => false,
        ("schedule", "list") => true,
        ("schedule", "create" | "delete" | "wakeup") => false,
        ("session", "get") => true,
        ("session", "rename") => false,
        ("goal", "get") => true,
        ("goal", "create" | "clear" | "complete") => false,
        ("mcp", "list_resources" | "read_resource" | "list_prompts" | "get_prompt") => true,
        ("mcp", "call") => false,
        _ => entry.decl.concurrency_safe,
    }
}

fn apply_patch_execution_from_tool_output(output: &ToolOutput) -> Option<ApplyPatchExecution> {
    let payload = output.to_json_payload()?;
    let operation_id = payload.get("operation_id")?.as_str()?.to_string();
    let changes: Vec<crate::message::FileChangeEntry> =
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

fn collect_loaded_tool_names(
    messages: &[Message],
    runtime_loaded_tools: &[String],
) -> std::collections::HashSet<String> {
    let mut loaded = messages
        .iter()
        .flat_map(|message| message.parts.iter())
        .filter_map(|part| match part.content.as_ref() {
            Some(PartContent::Operation(operation))
                if part.status == crate::message::ExecutionStatus::Completed =>
            {
                loaded_tools_from_tool_output(&operation.details)
            }
            _ => None,
        })
        .flatten()
        .collect::<std::collections::HashSet<_>>();

    loaded.extend(
        runtime_loaded_tools
            .iter()
            .map(String::as_str)
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map(ToOwned::to_owned),
    );

    loaded
}

fn loaded_tools_from_tool_output(details: &ToolOutput) -> Option<Vec<String>> {
    let payload = details.to_json_payload()?;
    serde_json::from_value(payload.get("loaded_tools")?.clone()).ok()
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
