pub(crate) mod apply_patch;
pub(crate) mod ask_user;
pub(crate) mod bash;
pub(crate) mod catalog;
pub(crate) mod cron;
pub(crate) mod definition;
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
pub(crate) mod view_file;
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
            .map(|entry| entry.effective_tags())
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
        {
            if shell_command_from_invocation(invocation)
                .as_deref()
                .is_some_and(bash::is_read_only_command)
            {
                return Ok(());
            }
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

    fn catalogued_tools(&self) -> Vec<RegistryPluginEntry> {
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
                        input_schema: entry.sanitized_input_schema(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => {
                            entry.decl.description = Some(patched.description);
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
        self.catalogued_tools().into_iter().any(|entry| {
            entry.exposed_name == invocation.entry_name
                && entry.decl.concurrency_safe
                && !entry.has_tag(crate::plugin::sdk::ToolTag::Interactive)
        })
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
        self.plugins
            .lookup_entry(invocation.entry_name.as_str())
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
        if !self.tool_catalog().is_tool_enabled(&definition) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
            )));
        }
        let command = shell_command_from_invocation(invocation);
        Ok((
            tool_name.clone(),
            self.agent.authorize_tool(
                tool_name.as_str(),
                command.as_deref(),
                &definition.effective_tags(),
            ),
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
        self.plugins.lookup_entry(invocation.entry_name.as_str())
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
                .get("command")
                .filter(|value| !matches!(value.as_str(), Some("bash" | "powershell" | "monitor")))
                .or_else(|| input.pointer("/args/command"))
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
        if invocation.name.rsplit('/').next() != Some("bash") {
            return Ok((invocation.clone(), None));
        }
        let mut input_value = invocation_input_value(invocation);
        let bash_input: crate::message::BashToolInput = serde_json::from_value(input_value.clone())
            .map_err(|err| ToolError::InvalidInput(format!("bash input: {err}")))?;
        let prepared_shell = self.prepare_shell_command(&bash_input, session_id, call_id)?;
        let Some(prepared_shell) = prepared_shell.clone() else {
            return Ok((invocation.clone(), None));
        };
        if prepared_shell.command == bash_input.command {
            return Ok((invocation.clone(), Some(prepared_shell)));
        }
        let mut rewritten = bash_input;
        rewritten.command = prepared_shell.command.clone();
        input_value = serde_json::to_value(rewritten)
            .map_err(|err| ToolError::InvalidInput(format!("bash input: {err}")))?;
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
            .plugins
            .lookup_entry(plugin_invocation.entry_name.as_str())
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
            .plugins
            .lookup_entry(plugin_invocation.entry_name.as_str())
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

#[cfg(test)]
mod tests {

    use std::fs;
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use chrono::Utc;
    use serde_json::json;
    use uuid::Uuid;

    use crate::message::{
        ApplyPatchToolInput, BashToolInput, EnterPlanModeToolInput, EnterWorktreeToolInput,
        FileChangeKind, FilesystemAccess, FilesystemEffect, GlobToolInput, GrepToolInput, Message,
        OperationPart, PartContent, ReadToolInput, StructuredObject, TaskSubagentType,
        TaskToolInput, TimeRange, TodoItem, TodoPriority, TodoStatus, TodoWriteToolInput,
        ToolInvocation, ToolSearchToolInput, ViewFileToolInput, WebFetchToolInput,
    };
    use crate::permission::PermissionPolicy;
    use crate::plugin::sdk::host_api::{
        EventSubscription, HostTodoPriority, HostTodoStatus, LogLevel, SpawnSubtaskRequest,
        SpawnSubtaskResponse, ToolDescriptor,
    };
    use crate::plugin::sdk::prelude::*;
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, Result as SdkResult,
    };
    use crate::plugin::{PluginEntry, PluginHost, PluginHostBuilder, PluginsConfig};
    use crate::role::Role;

    use super::{
        ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadInput,
        ToolPayloadOutput,
    };
    use crate::plugins::provided::router as in_process_router;

    #[derive(Debug)]
    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root = std::env::temp_dir().join(format!("agena-tool-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("failed to create temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn build_executor(root: &Path) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::new(root, agent).with_plugin_manager(build_default_plugin_manager(root))
    }

    #[derive(Debug)]
    struct TestToolHost;

    #[async_trait::async_trait]
    impl HostClient for TestToolHost {
        async fn log(&self, _level: LogLevel, _message: String, _fields: serde_json::Value) {}

        async fn publish_event(&self, _env: EventEnvelope) -> SdkResult<()> {
            Ok(())
        }

        async fn subscribe_events(&self, _filter: EventFilter) -> SdkResult<EventSubscription> {
            Ok(EventSubscription { id: "sub".into() })
        }

        async fn ask_permission(&self, _req: PermissionAskInput) -> SdkResult<PermissionDecision> {
            Ok(PermissionDecision::Prompt)
        }

        async fn read_config(&self, _path: Option<String>) -> SdkResult<serde_json::Value> {
            Ok(serde_json::Value::Null)
        }

        async fn invoke_tool(
            &self,
            tool: String,
            _input: serde_json::Value,
        ) -> SdkResult<ToolInvokeOutput> {
            Err(PluginError::new(format!(
                "unexpected invoke_tool for {tool}"
            )))
        }

        async fn spawn_subtask(&self, req: SpawnSubtaskRequest) -> SdkResult<SpawnSubtaskResponse> {
            Ok(SpawnSubtaskResponse {
                final_text: format!("spawned {}", req.description),
                metadata: std::collections::BTreeMap::from([(
                    "session_id".to_string(),
                    "child-1".to_string(),
                )]),
            })
        }

        async fn list_tools(&self) -> SdkResult<Vec<ToolDescriptor>> {
            Ok(vec![ToolDescriptor {
                name: "fs_edit".to_string(),
                description: Some("Patch files in the workspace".to_string()),
                tags: vec![
                    crate::plugin::sdk::ToolTag::Mutating,
                    crate::plugin::sdk::ToolTag::FilesystemWrite,
                ],
                deferred: true,
                plugin_id: None,
            }])
        }

        async fn todo_write(
            &self,
            req: crate::plugin::sdk::host_api::HostTodoWriteRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            Ok(in_process_router::tool_execution_to_invoke_output(
                ToolPayloadExecution::new(
                    ToolPayloadOutput::TodoWrite {
                        items: req
                            .items
                            .into_iter()
                            .map(|item| TodoItem {
                                content: item.content,
                                status: match item.status {
                                    HostTodoStatus::Pending => TodoStatus::Pending,
                                    HostTodoStatus::InProgress => TodoStatus::InProgress,
                                    HostTodoStatus::Completed => TodoStatus::Completed,
                                    HostTodoStatus::Cancelled => TodoStatus::Cancelled,
                                },
                                priority: match item.priority {
                                    HostTodoPriority::High => TodoPriority::High,
                                    HostTodoPriority::Medium => TodoPriority::Medium,
                                    HostTodoPriority::Low => TodoPriority::Low,
                                },
                            })
                            .collect(),
                    },
                    ToolExecutionView::simple("Todo write", "Updated todo list"),
                ),
            ))
        }
    }

    #[derive(Debug, Default)]
    struct FixturePlugin;

    #[async_trait::async_trait]
    impl Plugin for FixturePlugin {
        fn manifest(&self) -> PluginManifest {
            PluginManifest::builder("fixture", "0.1.0")
                .description("fixture plugin")
                .hooks(
                    HookSubscription::TOOL_BEFORE
                        | HookSubscription::TOOL_AFTER
                        | HookSubscription::TOOL_INVOKE
                        | HookSubscription::SHELL_ENV,
                )
                .tool(
                    PluginToolDecl::new(
                        "plugin_echo",
                        json!({
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        }),
                    )
                    .description("Echo a message from the plugin.")
                    .tag(crate::plugin::sdk::ToolTag::ReadOnly),
                )
                .tool(
                    PluginToolDecl::new(
                        "plugin_paths",
                        json!({
                            "type": "object",
                            "properties": {
                                "file_path": { "type": "string" },
                                "extra_paths": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "dynamic_path": { "type": "string" },
                                "url": { "type": "string" },
                                "dynamic_network": { "type": "string" }
                            },
                            "required": ["file_path"]
                        }),
                    )
                    .description("Expose declared and dynamic permission paths.")
                    .tag(crate::plugin::sdk::ToolTag::ReadOnly)
                    .input_path(InputPathSpec {
                        jsonpath: "$.file_path".to_string(),
                        kind: PathKind::Read,
                        optional: false,
                    })
                    .input_path(InputPathSpec {
                        jsonpath: "$.extra_paths[*]".to_string(),
                        kind: PathKind::Read,
                        optional: true,
                    })
                    .input_network(InputNetworkSpec {
                        jsonpath: "$.url".to_string(),
                        optional: true,
                    }),
                )
                .build()
        }

        async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
            if input.tool_name == "plugin_paths" {
                return Ok(ToolInvokeOutput::text("ok").with_title("Plugin paths"));
            }

            let message = input
                .input
                .get("message")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PluginError::new("missing message"))?
                .to_string();
            Ok(ToolInvokeOutput {
                title: "Plugin echo".to_string(),
                output_text: message.clone(),
                payload: Some(json!({ "echoed": message })),
                metadata: std::collections::BTreeMap::from([(
                    "plugin".to_string(),
                    "fixture".to_string(),
                )]),
                attachments: Vec::new(),
            })
        }

        async fn tool_execute_before(
            &self,
            input: ToolBeforeInput,
        ) -> Result<Option<ToolBeforePatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut new_input = input.input.clone();
            let message = new_input
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            new_input["message"] = serde_json::Value::String(format!("{message} prepared"));
            Ok(Some(ToolBeforePatch {
                input: Some(new_input),
                title_override: Some("Prepared plugin echo".to_string()),
                metadata: Default::default(),
            }))
        }

        async fn tool_execute_after(
            &self,
            input: ToolAfterInput,
        ) -> Result<Option<ToolAfterPatch>> {
            if input.tool_name != "plugin_echo" {
                return Ok(None);
            }
            let mut payload = input.payload.clone().unwrap_or_else(|| json!({}));
            payload["after"] = serde_json::Value::Bool(true);
            Ok(Some(ToolAfterPatch {
                title: Some(format!("{} after", input.title)),
                output_text: Some(format!("{} after", input.output_text)),
                payload: Some(payload),
                metadata: std::collections::BTreeMap::from([(
                    "after_hook".to_string(),
                    "applied".to_string(),
                )]),
            }))
        }

        async fn permission_paths(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> Result<Vec<PathRequest>> {
            if tool != "plugin_paths" {
                return Ok(Vec::new());
            }
            let Some(dynamic_path) = input.get("dynamic_path").and_then(|value| value.as_str())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![PathRequest::write(dynamic_path)])
        }

        async fn permission_networks(
            &self,
            tool: &str,
            input: &serde_json::Value,
        ) -> Result<Vec<NetworkRequest>> {
            if tool != "plugin_paths" {
                return Ok(Vec::new());
            }
            let Some(target) = input
                .get("dynamic_network")
                .and_then(|value| value.as_str())
            else {
                return Ok(Vec::new());
            };
            Ok(vec![NetworkRequest::connect(target)])
        }

        async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
            Ok(Some(ShellEnvPatch::set("PLUGIN_FLAG", "from_plugin")))
        }
    }

    fn test_plugin_runtime() -> &'static tokio::runtime::Runtime {
        use std::sync::OnceLock;

        static TEST_RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
        TEST_RT.get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .worker_threads(2)
                .build()
                .expect("test plugin runtime")
        })
    }

    fn build_plugin_manager(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_id = super::skills_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let web_id = super::web_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_id,
            &lsp_id,
            &cron_id,
            &fs_id,
            &shell_id,
            &web_id,
            &workflow_id,
        ] {
            list.insert(
                (*id).clone(),
                PluginEntry::Static {
                    options: serde_json::Value::Null,
                    timeouts: Default::default(),
                },
            );
        }
        list.insert(
            "fixture".to_string(),
            PluginEntry::Static {
                options: serde_json::Value::Null,
                timeouts: Default::default(),
            },
        );
        let config = PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
            default_quota: Default::default(),
            quotas: Default::default(),
        };
        test_plugin_runtime().block_on(async {
            PluginHostBuilder::new(root, "test")
                .with_config(config)
                .with_host_client(Arc::new(TestToolHost))
                .register_static(skills_id, super::new_skills_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(web_id, super::new_web_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .register_static("fixture", FixturePlugin)
                .build()
                .await
                .expect("plugin host should build")
        })
    }

    fn build_default_plugin_manager(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_id = super::skills_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let web_id = super::web_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_id,
            &lsp_id,
            &cron_id,
            &fs_id,
            &shell_id,
            &web_id,
            &workflow_id,
        ] {
            list.insert(
                (*id).clone(),
                PluginEntry::Static {
                    options: serde_json::Value::Null,
                    timeouts: Default::default(),
                },
            );
        }
        let config = PluginsConfig {
            enabled: true,
            timeouts: Default::default(),
            list,
            trusted_keys: Default::default(),
            default_quota: Default::default(),
            quotas: Default::default(),
        };
        test_plugin_runtime().block_on(async {
            PluginHostBuilder::new(root, "test")
                .with_config(config)
                .with_host_client(Arc::new(TestToolHost))
                .register_static(skills_id, super::new_skills_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(web_id, super::new_web_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .build()
                .await
                .expect("default plugin host should build")
        })
    }

    fn loaded_tool_search_message(loaded_tools: &[&str]) -> Message {
        Message {
            id: 99,
            role: Role::Assistant,
            state: crate::message::MessageStatus::Completed,
            parts: vec![crate::message::MessagePart::with_content(
                1,
                99,
                Utc::now(),
                crate::message::ExecutionStatus::Completed,
                PartContent::Operation(OperationPart::completed(
                    1,
                    ToolPayloadInput::ToolSearch(ToolSearchToolInput {
                        query: "load mutating tools".to_string(),
                        load: loaded_tools.iter().map(|name| name.to_string()).collect(),
                        limit: None,
                    })
                    .into_invocation(),
                    "loaded deferred tools",
                    Vec::new(),
                    Vec::new(),
                    (ToolPayloadOutput::ToolSearch {
                        results: Vec::new(),
                        loaded_tools: loaded_tools.iter().map(|name| name.to_string()).collect(),
                    })
                    .into_tool_output(),
                    TimeRange::default(),
                )),
            )],
            created_at: Utc::now(),
            metadata: crate::message::MessageMetadata::default(),
            usage: None,
            finish: None,
        }
    }

    fn sample_png_bytes() -> Vec<u8> {
        STANDARD
            .decode(
                "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO9W7tYAAAAASUVORK5CYII=",
            )
            .expect("sample png should decode")
    }

    #[test]
    fn read_provided_returns_line_numbered_preview() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to seed file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: Some(2),
                limit: Some(2),
            }))
            .expect("read default tool should succeed");

        match result.output {
            ToolPayloadOutput::Read {
                preview,
                truncated,
                loaded_paths,
            } => {
                let preview = preview.expect("preview must exist");
                assert!(preview.contains("2: two"));
                assert!(preview.contains("3: three"));
                assert_eq!(truncated, Some(false));
                assert_eq!(loaded_paths, vec!["notes.txt".to_string()]);
            }
            other => panic!("expected read output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_provided_reports_typed_file_changes() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("keep.txt"), "before\n").expect("failed to seed keep.txt");
        fs::write(workspace.root.join("remove.txt"), "delete me\n")
            .expect("failed to seed remove.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Add File: added.txt
+created
*** Update File: keep.txt
@@
-before
+after
*** Delete File: remove.txt
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch should succeed");

        match result.output {
            ToolPayloadOutput::ApplyPatch { changes, .. } => {
                assert_eq!(changes.len(), 3);
                assert!(changes.iter().any(|change| {
                    change.path == "added.txt" && change.kind == FileChangeKind::Added
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "keep.txt" && change.kind == FileChangeKind::Updated
                }));
                assert!(changes.iter().any(|change| {
                    change.path == "remove.txt" && change.kind == FileChangeKind::Deleted
                }));
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn apply_patch_provided_moves_files_and_reports_diff() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("old.txt"), "before\n").expect("failed to seed old.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
                patch: "\
*** Begin Patch
*** Update File: old.txt
*** Move to: new.txt
@@
-before
+after
*** End Patch"
                    .to_string(),
            }))
            .expect("apply_patch move should succeed");

        assert!(!workspace.root.join("old.txt").exists());
        assert_eq!(
            fs::read_to_string(workspace.root.join("new.txt")).unwrap(),
            "after\n"
        );
        match result.output {
            ToolPayloadOutput::ApplyPatch {
                changes,
                diff,
                progress,
                ..
            } => {
                assert_eq!(changes.len(), 1);
                assert_eq!(changes[0].path, "new.txt");
                assert_eq!(changes[0].from_path.as_deref(), Some("old.txt"));
                assert_eq!(changes[0].kind, FileChangeKind::Moved);
                assert!(diff.contains("rename from old.txt"));
                assert!(diff.contains("+after"));
                assert!(
                    progress
                        .iter()
                        .any(|line| line == "applied move old.txt -> new.txt")
                );
            }
            other => panic!("expected apply_patch output, got {other:?}"),
        }
    }

    #[test]
    fn view_file_provided_returns_metadata_and_attachment() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("pixel.png");
        fs::write(&file_path, sample_png_bytes()).expect("failed to seed png");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ViewFile(ViewFileToolInput {
                path: "pixel.png".to_string(),
            }))
            .expect("view_file should succeed");

        match result.output {
            ToolPayloadOutput::ViewFile {
                path,
                kind,
                mime,
                size_bytes,
                filename,
                width,
                height,
                duration_ms,
                page_count,
            } => {
                assert_eq!(path, "pixel.png");
                assert_eq!(kind, crate::message::AttachmentKind::Image);
                assert_eq!(mime, "image/png");
                assert!(size_bytes > 0);
                assert_eq!(filename.as_deref(), Some("pixel.png"));
                assert_eq!(width, Some(1));
                assert_eq!(height, Some(1));
                assert_eq!(duration_ms, None);
                assert_eq!(page_count, None);
            }
            other => panic!("expected view_file output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.filename.as_deref(), Some("pixel.png"));
        assert_eq!(attachment.kind, crate::message::AttachmentKind::Image);
        assert_eq!(attachment.mime, "image/png");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn view_file_provided_attaches_generic_text_file() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "hello from agena\n").expect("failed to seed text file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ViewFile(ViewFileToolInput {
                path: "notes.txt".to_string(),
            }))
            .expect("view_file should succeed for text file");

        match result.output {
            ToolPayloadOutput::ViewFile {
                path,
                kind,
                mime,
                filename,
                width,
                height,
                ..
            } => {
                assert_eq!(path, "notes.txt");
                assert_eq!(kind, crate::message::AttachmentKind::File);
                assert_eq!(mime, "text/plain");
                assert_eq!(filename.as_deref(), Some("notes.txt"));
                assert_eq!(width, None);
                assert_eq!(height, None);
            }
            other => panic!("expected view_file output, got {other:?}"),
        }

        assert_eq!(result.view.attachments.len(), 1);
        let attachment = &result.view.attachments[0];
        assert_eq!(attachment.kind, crate::message::AttachmentKind::File);
        assert_eq!(attachment.mime, "text/plain");
        match &attachment.source {
            crate::message::AttachmentSource::Base64 { data } => assert!(!data.is_empty()),
            other => panic!("expected base64 attachment source, got {other:?}"),
        }
    }

    #[test]
    fn glob_and_grep_report_match_counts() {
        let workspace = TempWorkspace::new();
        fs::create_dir_all(workspace.root.join("src/nested")).expect("failed to create tree");
        fs::write(
            workspace.root.join("src/main.rs"),
            "fn main() { println!(\"hello\"); }\n",
        )
        .expect("failed to write main.rs");
        fs::write(
            workspace.root.join("src/nested/lib.rs"),
            "pub fn value() -> i32 { 7 }\n",
        )
        .expect("failed to write lib.rs");

        let executor = build_executor(&workspace.root);

        let glob_result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Glob(GlobToolInput {
                pattern: "**/*.rs".to_string(),
                path: Some("src".to_string()),
            }))
            .expect("glob should succeed");

        match glob_result.output {
            ToolPayloadOutput::Glob { count } => {
                assert_eq!(count, Some(2));
            }
            other => panic!("expected glob output, got {other:?}"),
        }

        let grep_result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Grep(GrepToolInput {
                pattern: "hello".to_string(),
                path: Some("src".to_string()),
                include: Some("**/*.rs".to_string()),
            }))
            .expect("grep should succeed");

        match grep_result.output {
            ToolPayloadOutput::Grep { matches } => {
                assert_eq!(matches, Some(1));
            }
            other => panic!("expected grep output, got {other:?}"),
        }
    }

    #[test]
    fn task_plugin_entry_generates_session_id() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolPayloadInput::Task(TaskToolInput {
            description: "inspect code".to_string(),
            prompt: "find modules".to_string(),
            subagent_type: TaskSubagentType::Explore,
            task_id: None,
            command: None,
        })
        .into_invocation();

        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("task plugin tool should succeed");

        assert_eq!(
            result
                .view
                .metadata
                .get("subagent_type")
                .map(String::as_str),
            Some("explore")
        );
        assert_eq!(
            result
                .view
                .metadata
                .get("profile_guidance")
                .map(String::as_str),
            Some(TaskSubagentType::Explore.guidance())
        );

        let payload = ToolPayloadOutput::from_tool_output("task", &result.output)
            .expect("task output should decode as tool payload");
        match payload {
            ToolPayloadOutput::Task { session_id, .. } => {
                assert!(session_id.is_some());
            }
            other => panic!("expected task payload, got {other:?}"),
        }
    }

    #[test]
    fn tool_search_provided_discovers_and_loads_deferred_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::ToolSearch(ToolSearchToolInput {
                query: "patch files".to_string(),
                load: vec!["fs_edit".to_string()],
                limit: None,
            }))
            .expect("tool_search should succeed");

        match result.output {
            ToolPayloadOutput::ToolSearch {
                results,
                loaded_tools,
            } => {
                assert!(results.iter().any(|name| name == "fs_edit"));
                assert_eq!(loaded_tools, vec!["fs_edit".to_string()]);
            }
            other => panic!("expected tool_search output, got {other:?}"),
        }

        assert!(result.view.output_text.contains("Loaded deferred tools"));
    }

    #[test]
    fn tool_search_messages_expose_deferred_tools_in_later_turns() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let initial = executor.available_tools();
        assert!(initial.iter().any(|tool| tool.exposed_name == "tools"));
        assert!(initial.iter().any(|tool| tool.exposed_name == "todo"));
        assert!(!initial.iter().any(|tool| tool.exposed_name == "shell"));
        assert!(!initial.iter().any(|tool| tool.exposed_name == "task"));

        let messages = vec![loaded_tool_search_message(&["shell", "task"])];
        let available = executor.available_tools_for_messages(messages.as_slice());

        assert!(available.iter().any(|tool| tool.exposed_name == "shell"));
        assert!(available.iter().any(|tool| tool.exposed_name == "task"));
    }

    #[test]
    fn plugin_entries_drive_available_tool_catalog() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let tools = executor.available_tools();
        let fs = tools
            .iter()
            .find(|tool| tool.exposed_name == "fs")
            .expect("fs tool should be available");
        assert_eq!(fs.plugin_name, super::fs_plugin_id());
        assert!(fs.has_tag(crate::plugin::sdk::ToolTag::FilesystemRead));

        let fs_count = tools
            .iter()
            .filter(|tool| tool.exposed_name == "fs")
            .count();
        assert_eq!(fs_count, 1);
    }

    #[test]
    fn available_tools_are_backed_by_plugin_registry() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        for definition in executor.available_tools() {
            assert!(
                executor
                    .plugin_manager()
                    .lookup_entry(definition.exposed_name.as_str())
                    .is_some(),
                "missing registry entry for {}",
                definition.exposed_name
            );
        }

        for definition in executor.searchable_tools() {
            assert!(
                executor
                    .plugin_manager()
                    .lookup_entry(definition.exposed_name.as_str())
                    .is_some(),
                "missing registry entry for {}",
                definition.exposed_name
            );
        }
    }

    #[test]
    fn plugin_entries_are_projected_into_available_tool_catalog() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let available = executor
            .searchable_tools()
            .into_iter()
            .map(|item| item.exposed_name)
            .collect::<std::collections::BTreeSet<_>>();
        let registry = executor
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .map(|entry| entry.exposed_name.clone())
            .collect::<std::collections::BTreeSet<_>>();

        assert_eq!(available, registry);
    }

    #[test]
    fn available_tools_are_sorted_stably_for_request_fingerprints() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let names = executor
            .available_tools()
            .into_iter()
            .map(|tool| tool.exposed_name)
            .collect::<Vec<_>>();
        let mut expected = names.clone();
        expected.sort();

        assert_eq!(names, expected);
    }

    #[test]
    fn todo_write_provided_returns_items_for_session_state() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::TodoWrite(TodoWriteToolInput {
                items: vec![TodoItem {
                    content: "Implement tool_search".to_string(),
                    status: TodoStatus::InProgress,
                    priority: TodoPriority::High,
                }],
            }))
            .expect("todo_write should succeed");

        match result.output {
            ToolPayloadOutput::TodoWrite { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "Implement tool_search");
                assert_eq!(items[0].status, TodoStatus::InProgress);
            }
            other => panic!("expected todo_write output, got {other:?}"),
        }
    }

    #[test]
    fn bash_provided_runs_command() {
        if cfg!(windows) {
            // Windows host environments can include PATH entries whose ACL cannot be audited
            // in shell preflight, which makes this smoke test flaky/non-portable.
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(BashToolInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: Vec::new(),
            }))
            .expect("bash default tool should succeed");

        match &result.output {
            ToolPayloadOutput::Bash {
                output,
                description,
            } => {
                let output = output
                    .as_deref()
                    .expect("output should exist")
                    .to_ascii_lowercase();
                assert!(output.contains("hello_agena"));
                assert!(description.is_some());
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn bash_provided_explains_no_match_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("notes.txt"), "alpha\nbeta\n")
            .expect("failed to seed notes file");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(BashToolInput {
                command: "grep missing notes.txt".to_string(),
                description: "search missing text".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![crate::message::FilesystemEffect {
                    path: "notes.txt".to_string(),
                    access: crate::message::FilesystemAccess::Read,
                }],
            }))
            .expect("bash default tool should succeed");

        match result.output {
            ToolPayloadOutput::Bash {
                output,
                description,
            } => {
                assert!(
                    output
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("no matches"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("no_matches")
        );
    }

    #[test]
    fn bash_provided_explains_diff_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("left.txt"), "alpha\n").expect("failed to write left file");
        fs::write(workspace.root.join("right.txt"), "beta\n").expect("failed to write right file");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(BashToolInput {
                command: "diff left.txt right.txt".to_string(),
                description: "compare files".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![
                    crate::message::FilesystemEffect {
                        path: "left.txt".to_string(),
                        access: crate::message::FilesystemAccess::Read,
                    },
                    crate::message::FilesystemEffect {
                        path: "right.txt".to_string(),
                        access: crate::message::FilesystemAccess::Read,
                    },
                ],
            }))
            .expect("bash default tool should succeed");

        match &result.output {
            ToolPayloadOutput::Bash { description, .. } => {
                assert!(
                    description
                        .as_deref()
                        .is_some_and(|text| text.contains("found differences"))
                );
            }
            other => panic!("expected bash output, got {other:?}"),
        }

        assert_eq!(
            result
                .view
                .metadata
                .get("exit_interpretation")
                .map(String::as_str),
            Some("differences_found")
        );
    }

    #[test]
    fn bash_provided_rejects_obvious_write_without_declared_effects() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(BashToolInput {
                command: "echo hi > created.txt".to_string(),
                description: "attempt write".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: Vec::new(),
            }))
            .expect_err("write command should be rejected before execution");

        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("modify files"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn readonly_model_profile_disables_apply_patch_and_task_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root).with_model_id("gpt-readonly");

        let availability = executor.available_tools();
        let names = availability
            .iter()
            .map(|item| item.exposed_name.as_str())
            .collect::<std::collections::BTreeSet<_>>();

        assert!(names.contains("fs"));
        assert!(!names.contains("fs_edit"));
        assert!(!names.contains("task"));
    }

    #[test]
    fn plugin_custom_tool_hooks_prepare_and_mutate_execution() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        assert!(
            executor.available_tools().iter().any(|tool| {
                tool.exposed_name == "plugin_echo" && tool.plugin_name == "fixture"
            })
        );

        let invocation = ToolInvocation {
            name: "plugin_echo".to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({ "message": "hello" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed");
        assert_eq!(
            prepared.title_override.as_deref(),
            Some("Prepared plugin echo")
        );

        let ToolInvocation { input, .. } = &prepared.invocation;
        let prepared_value = serde_json::Value::from(input.clone());
        assert_eq!(prepared_value["message"], "hello prepared");

        let execution = executor
            .execute_invocation_detailed(&prepared.invocation, 7, 9)
            .expect("plugin execution should succeed");

        let payload = serde_json::Value::from(execution.output.payload.clone());
        assert_eq!(payload["echoed"], "hello prepared");
        assert_eq!(payload["after"], true);

        assert_eq!(execution.view.title, "Plugin echo after");
        assert_eq!(execution.view.output_text, "hello prepared after");
        assert_eq!(
            execution
                .view
                .metadata
                .get("after_hook")
                .map(String::as_str),
            Some("applied")
        );
    }

    #[test]
    fn prepare_invocation_keeps_provided_calls_in_custom_wire_shape() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolPayloadInput::Read(ReadToolInput {
            file_path: "notes.txt".to_string(),
            offset: Some(3),
            limit: Some(5),
        })
        .into_invocation();

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed for provided");

        let ToolInvocation {
            name,
            input,
            plugin_name,
        } = prepared.invocation;
        assert_eq!(name, "fs");
        assert_eq!(plugin_name.as_deref(), Some(super::fs_plugin_id()));
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["command"], "read");
        assert_eq!(payload["args"]["file_path"], "notes.txt");
        assert_eq!(payload["args"]["offset"], 3);
        assert_eq!(payload["args"]["limit"], 5);
    }

    #[test]
    fn prepare_invocation_preserves_plugin_entry_name() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolInvocation {
            name: "mcp:docs:search".to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({ "query": "plugin host" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should preserve plugin tool invocation");

        let ToolInvocation {
            name,
            input,
            plugin_name,
        } = prepared.invocation;
        assert_eq!(name, "mcp:docs:search");
        assert_eq!(plugin_name.as_deref(), Some("custom"));
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["query"], "plugin host");
    }

    #[test]
    fn collect_permission_checks_for_plugin_invocation_uses_declared_and_dynamic_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));
        let invocation = ToolInvocation {
            name: "plugin_paths".to_string(),
            plugin_name: None,
            input: StructuredObject::try_from(json!({
                "file_path": "docs/spec.md",
                "extra_paths": ["notes/a.md", "notes/b.md"],
                "dynamic_path": "logs/output.txt",
                "url": "https://docs.rs/",
                "dynamic_network": "api.example.com:443"
            }))
            .expect("structured object should build"),
        };

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("docs/spec.md")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes/a.md")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes/b.md")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("logs/output.txt")),
        )));

        let network_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::NetworkAccess { host, port, .. } => {
                    Some((host.clone(), *port))
                }
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(network_actions.contains(&("docs.rs".to_string(), Some(443))));
        assert!(network_actions.contains(&("api.example.com".to_string(), Some(443))));
    }

    #[test]
    fn collect_permission_checks_for_provided_invocation_uses_dynamic_plugin_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::ApplyPatch(ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: notes.txt\n+hello\n*** Delete File: old.txt\n*** End Patch"
                .to_string(),
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("default permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("notes.txt")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("old.txt")),
        )));
    }

    #[test]
    fn collect_permission_checks_for_workflow_plan_uses_declared_path_access() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation =
            ToolPayloadInput::EnterPlanMode(EnterPlanModeToolInput::default()).into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("workflow permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join(".agena/plans")),
        )));
    }

    #[test]
    fn collect_permission_checks_for_workflow_worktree_uses_dynamic_plugin_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let named_invocation = ToolPayloadInput::EnterWorktree(EnterWorktreeToolInput {
            name: Some("demo".to_string()),
            path: None,
        })
        .into_invocation();
        let named_checks = executor
            .collect_permission_checks_for_invocation(&named_invocation)
            .expect("named worktree permission collection should succeed");
        let named_paths = named_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        assert!(named_paths.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join(".agena/worktrees/demo"),),
        )));

        let outside = workspace.root.with_file_name(format!(
            "{}-existing-worktree",
            workspace
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agena")
        ));
        let existing_invocation = ToolPayloadInput::EnterWorktree(EnterWorktreeToolInput {
            name: None,
            path: Some(outside.to_string_lossy().to_string()),
        })
        .into_invocation();
        let existing_checks = executor
            .collect_permission_checks_for_invocation(&existing_invocation)
            .expect("existing worktree permission collection should succeed");
        let existing_paths = existing_checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();
        let outside_display = super::normalize_path_for_display(&outside);
        assert!(existing_paths.contains(&("read".to_string(), outside_display.clone())));
        assert!(existing_paths.contains(&("write".to_string(), outside_display)));
    }

    #[test]
    fn web_fetch_uses_network_permission_policy() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all())
            .try_with_permission_config(&crate::agent::PermissionConfig {
                network: crate::agent::NetworkPermissionConfig {
                    loopback: Some(crate::permission::PermissionMode::Deny),
                    ..Default::default()
                },
                ..Default::default()
            })
            .expect("network permission config compiles");
        let executor = ToolExecutor::new(workspace.root.clone(), agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::WebFetch(WebFetchToolInput {
                url: "http://localhost:8000/".to_string(),
                prompt: None,
            }))
            .expect_err("loopback fetch should be denied before request");

        match err {
            ToolError::PermissionDenied(reason) => assert!(reason.contains("loopback")),
            other => panic!("expected network permission denial, got {other:?}"),
        }
    }

    #[test]
    fn collect_permission_checks_for_bash_invocation_uses_declared_filesystem_effects() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let outside = workspace.root.with_file_name(format!(
            "{}-outside.txt",
            workspace
                .root
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("agena")
        ));
        let invocation = ToolPayloadInput::Bash(BashToolInput {
            command: "cat src/lib.rs > target/out.txt".to_string(),
            description: "declared effects".to_string(),
            timeout_ms: Some(30_000),
            workdir: Some("packages/app".to_string()),
            filesystem_effects: vec![
                FilesystemEffect {
                    path: "src/lib.rs".to_string(),
                    access: FilesystemAccess::Read,
                },
                FilesystemEffect {
                    path: "target/out.txt".to_string(),
                    access: FilesystemAccess::Write,
                },
                FilesystemEffect {
                    path: "Cargo.lock".to_string(),
                    access: FilesystemAccess::ReadWrite,
                },
                FilesystemEffect {
                    path: outside.to_string_lossy().to_string(),
                    access: FilesystemAccess::Write,
                },
            ],
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");

        let path_actions = checks
            .iter()
            .filter_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } => Some((access_kind.clone(), target_path.clone())),
                _ => None,
            })
            .collect::<std::collections::HashSet<_>>();

        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/src/lib.rs")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/target/out.txt")),
        )));
        assert!(path_actions.contains(&(
            "read".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/Cargo.lock")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&workspace.root.join("packages/app/Cargo.lock")),
        )));
        assert!(path_actions.contains(&(
            "write".to_string(),
            super::normalize_path_for_display(&outside),
        )));
    }

    #[test]
    fn collect_permission_checks_for_declared_bash_write_uses_path_policy() {
        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new(
            "build",
            PermissionPolicy::new(
                crate::permission::PermissionMode::Allow,
                crate::permission::PermissionMode::Deny,
            ),
        );
        let executor = ToolExecutor::new(&workspace.root, agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(BashToolInput {
            command: "touch created.txt".to_string(),
            description: "declared write".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: vec![FilesystemEffect {
                path: "created.txt".to_string(),
                access: FilesystemAccess::Write,
            }],
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("permission collection should succeed");
        let write_decision = checks
            .iter()
            .find_map(|check| match &check.action {
                crate::permission::PermissionAction::PathAccess {
                    access_kind,
                    target_path,
                    ..
                } if access_kind == "write"
                    && target_path
                        == &super::normalize_path_for_display(
                            &workspace.root.join("created.txt"),
                        ) =>
                {
                    Some(&check.decision)
                }
                _ => None,
            })
            .expect("declared write path should be checked");

        match write_decision {
            crate::permission::PermissionDecision::Deny { reason } => {
                assert!(reason.contains("write"));
            }
            other => panic!("expected declared write to follow path policy, got {other:?}"),
        }
    }

    #[test]
    fn bash_input_requires_filesystem_effects_field() {
        let err = serde_json::from_value::<BashToolInput>(json!({
            "command": "pwd",
            "description": "",
            "timeout_ms": null,
            "workdir": null
        }))
        .expect_err("bash input should require filesystem_effects");

        assert!(err.to_string().contains("filesystem_effects"));
    }

    #[test]
    fn bash_tool_schema_requires_filesystem_effects_field() {
        let schema = crate::entry::definition::json_schema_for::<BashToolInput>();
        let required = schema
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("bash schema should declare required fields");

        assert!(
            required
                .iter()
                .any(|field| field.as_str() == Some("filesystem_effects"))
        );
        assert!(
            schema
                .pointer("/properties/filesystem_effects")
                .and_then(serde_json::Value::as_object)
                .is_some()
        );
    }

    #[test]
    fn bash_invocation_rejects_obvious_write_without_declared_effects() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));
        let invocation = ToolPayloadInput::Bash(BashToolInput {
            command: "touch created.txt".to_string(),
            description: "missing effects".to_string(),
            timeout_ms: Some(30_000),
            workdir: None,
            filesystem_effects: Vec::new(),
        })
        .into_invocation();

        let err = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect_err("mutating bash without filesystem effects should be rejected");
        match err {
            ToolError::InvalidInput(message) => {
                assert!(message.contains("filesystem_effects"));
                assert!(message.contains("modify files"));
            }
            other => panic!("expected invalid input, got {other:?}"),
        }
    }

    #[test]
    fn bash_execution_enforces_declared_filesystem_effect_permissions() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let agent = crate::agent::Agent::new(
            "build",
            PermissionPolicy::new(
                crate::permission::PermissionMode::Allow,
                crate::permission::PermissionMode::Deny,
            ),
        );
        let executor = ToolExecutor::new(&workspace.root, agent)
            .with_plugin_manager(build_default_plugin_manager(&workspace.root));

        let err = executor
            .execute_tool_payload_detailed(&ToolPayloadInput::Bash(BashToolInput {
                command: "printf ok".to_string(),
                description: "declared write denied".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
                filesystem_effects: vec![FilesystemEffect {
                    path: "created.txt".to_string(),
                    access: FilesystemAccess::Write,
                }],
            }))
            .expect_err("declared write should be denied by path policy");

        match err {
            ToolError::PermissionDenied(message) => {
                assert!(message.contains("write"));
            }
            other => panic!("expected permission denial, got {other:?}"),
        }
    }

    #[test]
    fn bash_invocation_applies_plugin_shell_env_overrides() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        let execution = executor
            .execute_invocation_detailed(
                &ToolPayloadInput::Bash(BashToolInput {
                    command: "printf %s \"$PLUGIN_FLAG\"".to_string(),
                    description: "print plugin env".to_string(),
                    timeout_ms: Some(30_000),
                    workdir: None,
                    filesystem_effects: Vec::new(),
                })
                .into_invocation(),
                10,
                11,
            )
            .expect("bash invocation should succeed");

        match ToolPayloadOutput::from_tool_output("bash", &execution.output) {
            Some(ToolPayloadOutput::Bash {
                output,
                description: _,
            }) => {
                assert_eq!(output.as_deref(), Some("from_plugin"));
            }
            other => panic!("expected bash output, got {other:?}"),
        }
    }

    #[test]
    fn enforce_plan_mode_allows_read_only_bash_and_blocks_mutating_bash() {
        use crate::session::PlanState;

        let workspace = TempWorkspace::new();
        let registry = super::plan_registry_for_executor();
        let executor = build_executor(&workspace.root).with_plan_registry(registry.clone());

        // Activate plan mode for session 7.
        registry.write().insert(
            7,
            PlanState {
                file_path: workspace.root.join(".agena/plans/test.md"),
                slug: "test".to_string(),
                started_at: chrono::Utc::now(),
            },
        );

        let bash_input = |cmd: &str| -> ToolInvocation {
            ToolPayloadInput::Bash(BashToolInput {
                command: cmd.to_string(),
                description: String::new(),
                timeout_ms: None,
                workdir: None,
                filesystem_effects: Vec::new(),
            })
            .into_invocation()
        };

        // Read-only bash is allowed in plan mode.
        executor
            .enforce_plan_mode_for(&bash_input("git status"), 7)
            .expect("git status is read-only and should be allowed");
        executor
            .enforce_plan_mode_for(&bash_input("ls -la"), 7)
            .expect("ls -la is read-only and should be allowed");

        // Mutating bash is blocked.
        let err = executor
            .enforce_plan_mode_for(&bash_input("rm -rf node_modules"), 7)
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));

        // Unknown / unclassified bash is blocked (safety default).
        let err = executor
            .enforce_plan_mode_for(&bash_input("./unknown-binary --do-it"), 7)
            .unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));

        // After leaving plan mode, everything is allowed again.
        registry.write().remove(&7);
        executor
            .enforce_plan_mode_for(&bash_input("rm -rf node_modules"), 7)
            .expect("plan mode is off, mutating bash should be allowed");

        // Drop guard
        let _ = bash_input;
    }

    #[test]
    fn enforce_plan_mode_uses_session_lookup() {
        use crate::session::PlanState;

        let workspace = TempWorkspace::new();
        let registry = super::plan_registry_for_executor();
        let executor = build_executor(&workspace.root).with_plan_registry(registry.clone());
        registry.write().insert(
            42,
            PlanState {
                file_path: workspace.root.join(".agena/plans/x.md"),
                slug: "x".to_string(),
                started_at: chrono::Utc::now(),
            },
        );

        let inv = ToolPayloadInput::Bash(BashToolInput {
            command: "rm -rf /".to_string(),
            description: String::new(),
            timeout_ms: None,
            workdir: None,
            filesystem_effects: Vec::new(),
        })
        .into_invocation();

        // Different session id — plan mode does not apply.
        executor
            .enforce_plan_mode_for(&inv, 1)
            .expect("session 1 is not in plan mode");

        // Same session id — plan mode blocks.
        let err = executor.enforce_plan_mode_for(&inv, 42).unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }
}
