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
    AskUserToolInput, CustomToolOutput, FirstPartyToolInput, FirstPartyToolOutput, Message,
    PartContent, PluginInvocation, StructuredObject, ToolExecutionPart, ToolInvocation, ToolOutput,
};
use crate::permission::{
    AccessKind, PermissionAction, PermissionDecision, PermissionRuleStore, PermissionRuntime,
    PermissionRuntimeDecision,
};
use crate::plugin::{
    EntryDefinitionInput as PluginEntryDefinitionInput, EntrySource as SdkEntrySource, PluginHost,
    PluginHostBuilder, ToolAfterInput as PluginToolAfterInput,
    ToolBeforeInput as PluginToolBeforeInput, ToolFailureInput as PluginToolFailureInput,
    ToolInvokeInput as PluginToolInvokeInput,
    ToolPermissionPathsInput as PluginToolPermissionPathsInput,
    sdk::{
        EntryBehavior as SdkEntryBehavior, EntryStreamingMode as SdkEntryStreamingMode,
        InputPathSpec as SdkInputPathSpec, PathKind as SdkPathKind,
        PlanModePolicy as SdkPlanModePolicy, ShellEnvInput as PluginShellEnvInput,
    },
};
use crate::plugins::bundled::{
    cron as bundled_cron, fs as bundled_fs, lsp as bundled_lsp, mcp, router as bundled_router,
    shell as bundled_shell, skills_fs, web as bundled_web, workflow as bundled_workflow,
};

pub use apply_patch::{AppliedFileChange, ApplyPatchExecution};
pub use catalog::{ModelToolProfile, ToolAvailability, ToolCatalog};
pub use definition::{EntryBehavior, EntryDefinition, EntryLoadPriority, EntrySource};
pub use monitor::{
    MonitorError, MonitorRead, MonitorRegistry, MonitorService, MonitorStart, MonitorStopOutcome,
    ReadParams as MonitorReadParams, StartParams as MonitorStartParams,
};
pub use plan::{PlanRegistry, registry_for_executor as plan_registry_for_executor};
pub use result::{FirstPartyExecution, ToolExecutionView, ToolInvocationExecution};
pub use shell::{ExecutionPolicy, ShellError, ShellOutput, ShellRequest};
pub use truncation::{ToolOutputTruncationPolicy, ToolOutputTruncator};
pub use worktree::{
    ActiveWorktree, ManagedWorktree, WorktreeRegistry, list_active as worktree_list_active,
    list_managed as worktree_list_managed, prune_stale as worktree_prune_stale,
    registry_for_executor as worktree_registry_for_executor,
};

pub fn skills_fs_plugin_id() -> &'static str {
    skills_fs::SKILLS_FS_PLUGIN_ID
}

pub fn new_skills_fs_plugin() -> impl crate::plugin::sdk::Plugin {
    skills_fs::SkillsFsPlugin::new()
}

pub fn lsp_plugin_id() -> &'static str {
    bundled_lsp::LSP_PLUGIN_ID
}

pub fn new_lsp_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_lsp::LspPlugin::new()
}

pub fn cron_plugin_id() -> &'static str {
    bundled_cron::CRON_PLUGIN_ID
}

pub fn new_cron_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_cron::CronPlugin::new()
}

pub fn fs_plugin_id() -> &'static str {
    bundled_fs::FS_PLUGIN_ID
}

pub fn new_fs_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_fs::new_plugin()
}

pub fn shell_plugin_id() -> &'static str {
    bundled_shell::SHELL_PLUGIN_ID
}

pub fn new_shell_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_shell::new_plugin()
}

pub fn web_plugin_id() -> &'static str {
    bundled_web::WEB_PLUGIN_ID
}

pub fn new_web_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_web::new_plugin()
}

pub fn workflow_plugin_id() -> &'static str {
    bundled_workflow::WORKFLOW_PLUGIN_ID
}

pub fn new_workflow_plugin() -> impl crate::plugin::sdk::Plugin {
    bundled_workflow::new_plugin()
}

/// First-party plugins (`agena.*`) are surfaced to the catalog and to model
/// callers as first_party tools, even though their entries now live in dedicated
/// plugins. Third-party plugins still show up as `EntrySource::Plugin`.
fn is_first_party_plugin(plugin_id: &str) -> bool {
    plugin_id.starts_with("agena.")
}

fn uses_local_first_party_bridge(plugin_id: &str) -> bool {
    matches!(
        plugin_id,
        bundled_fs::FS_PLUGIN_ID | bundled_shell::SHELL_PLUGIN_ID | bundled_web::WEB_PLUGIN_ID
    )
}

fn plugin_invocation_uses_local_first_party_bridge(
    plugins: &PluginHost,
    invocation: &PluginInvocation,
) -> bool {
    plugins
        .lookup_entry(invocation.entry_name.as_str())
        .is_some_and(|resolution| uses_local_first_party_bridge(&resolution.handle.plugin_id))
}

pub fn first_party_plugin_host(
    workspace_root: impl Into<PathBuf>,
) -> Result<Arc<PluginHost>, String> {
    let workspace_root = workspace_root.into();
    let skills_fs_id = skills_fs_plugin_id().to_string();
    let lsp_id = lsp_plugin_id().to_string();
    let cron_id = cron_plugin_id().to_string();
    let fs_id = fs_plugin_id().to_string();
    let shell_id = shell_plugin_id().to_string();
    let web_id = web_plugin_id().to_string();
    let workflow_id = workflow_plugin_id().to_string();
    let mut list = std::collections::BTreeMap::new();
    for id in [
        &skills_fs_id,
        &lsp_id,
        &cron_id,
        &fs_id,
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
            .register_static(skills_fs_id, new_skills_fs_plugin())
            .register_static(lsp_id, new_lsp_plugin())
            .register_static(cron_id, new_cron_plugin())
            .register_static(fs_id, new_fs_plugin())
            .register_static(shell_id, new_shell_plugin())
            .register_static(web_id, new_web_plugin())
            .register_static(workflow_id, new_workflow_plugin())
            .build()
            .await
    })
    .map_err(|err| err.to_string())
}
/// Stable id used to register configured MCP servers as plugin entries.
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PermissionExecutionMode {
    Enforced,
    Bypassed,
}

#[derive(Debug, Clone, Default)]
pub(super) struct FirstPartyExecutionContext {
    pub session_id: Option<i64>,
    pub call_id: Option<i64>,
    pub session_context: Option<crate::session::SessionExecutionContext>,
}

static SYNTHETIC_BUILTIN_CALL_ID: AtomicI64 = AtomicI64::new(-1);

pub struct StreamingToolExecution {
    pub stream_id: String,
    pub chunks: tokio::sync::mpsc::Receiver<crate::plugin::sdk::ToolStreamChunk>,
    pub end: tokio::sync::oneshot::Receiver<Result<ToolInvocationExecution, ToolError>>,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum PermissionedFirstPartyExecution {
    Executed(FirstPartyExecution),
    Pending(crate::permission::PendingPermission),
}

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
    monitor_registry: Option<Arc<dyn MonitorService>>,
    truncator: ToolOutputTruncator,
    sandbox_policy: ExecutionPolicy,
    plugins: Arc<PluginHost>,
    web_search_backend: crate::config::WebSearchBackend,
    plan_registry: Option<plan::PlanRegistry>,
    worktree_registry: Option<worktree::WorktreeRegistry>,
    scheduler: Option<Arc<agena_scheduler::Scheduler>>,
    lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
    permission_mode: PermissionExecutionMode,
}

impl ToolExecutor {
    pub fn new(workspace_root: impl Into<PathBuf>, agent: Agent) -> Self {
        Self::with_sandbox_policy(workspace_root, agent, ExecutionPolicy::workspace_write())
    }

    pub fn with_sandbox_policy(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        sandbox_policy: ExecutionPolicy,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            monitor_registry: monitor::default_registry(),
            truncator: ToolOutputTruncator::default(),
            sandbox_policy,
            plugins: PluginHost::new_empty(),
            web_search_backend: crate::config::WebSearchBackend::DuckDuckGoHtml,
            plan_registry: None,
            worktree_registry: None,
            scheduler: None,
            lsp_registry: None,
            permission_mode: PermissionExecutionMode::Enforced,
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
        let (behavior, policy) = self
            .invocation_manifest_metadata(invocation)
            .unwrap_or((SdkEntryBehavior::WriteSandboxed, SdkPlanModePolicy::Blocked));

        match policy {
            SdkPlanModePolicy::Allowed => Ok(()),
            SdkPlanModePolicy::Blocked => Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' is blocked in plan mode; call exit_plan_mode first"
            ))),
            SdkPlanModePolicy::ConditionalShellReadOnly => {
                if let Some(first_party) = self.first_party_from_invocation(invocation)? {
                    let is_read_only = match first_party {
                        FirstPartyToolInput::Bash(payload) => {
                            bash::is_read_only_command(payload.command.as_str())
                        }
                        FirstPartyToolInput::PowerShell(payload) => {
                            bash::is_read_only_command(payload.command.as_str())
                        }
                        _ => false,
                    };
                    if is_read_only {
                        return Ok(());
                    }
                }
                Err(ToolError::PermissionDenied(format!(
                    "tool '{tool_name}' is blocked in plan mode; call exit_plan_mode first"
                )))
            }
            SdkPlanModePolicy::Derived => {
                if matches!(behavior, SdkEntryBehavior::ReadOnly) {
                    return Ok(());
                }
                Err(ToolError::PermissionDenied(format!(
                    "tool '{tool_name}' is blocked in plan mode; call exit_plan_mode first"
                )))
            }
        }
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

    pub fn sandbox_policy(&self) -> &ExecutionPolicy {
        &self.sandbox_policy
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

    pub fn available_first_party_tools(&self) -> Vec<ToolAvailability> {
        let catalog = self.tool_catalog();
        self.plugins
            .entry_entries()
            .into_iter()
            .filter(|entry| is_first_party_plugin(&entry.plugin_name))
            .map(|entry| {
                EntryDefinition::from_decl(
                    entry.exposed_name.clone(),
                    &entry.decl,
                    EntrySource::FirstParty,
                )
            })
            .map(|definition| catalog.availability_for_definition(&self.agent, &definition))
            .collect()
    }

    fn catalogued_tools(&self) -> Vec<EntryDefinition> {
        let catalog = self.tool_catalog();
        let mut definitions = self
            .plugins
            .entry_entries()
            .into_iter()
            .map(|entry| {
                let source = if is_first_party_plugin(&entry.plugin_name) {
                    EntrySource::FirstParty
                } else {
                    EntrySource::Plugin {
                        plugin_name: entry.plugin_name.clone(),
                    }
                };
                EntryDefinition::from_decl(entry.exposed_name.clone(), &entry.decl, source)
            })
            .filter(|definition| catalog.is_behavior_enabled(definition.behavior))
            .collect::<Vec<_>>();

        definitions.sort_by(|left, right| {
            left.name
                .cmp(&right.name)
                .then_with(|| left.description.cmp(&right.description))
        });

        // Plugin chain: tool.definition. Let plugins rewrite descriptions /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            definitions = definitions
                .into_iter()
                .map(|def| {
                    let input = PluginEntryDefinitionInput {
                        tool_name: def.name.clone(),
                        source: local_to_sdk_source(&def.source),
                        description: def.description.clone(),
                        input_schema: def.input_schema.clone(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => EntryDefinition {
                            description: patched.description,
                            input_schema: patched.input_schema,
                            ..def
                        },
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %def.name,
                                "tool.definition hook failed (keeping original): {err}"
                            );
                            def
                        }
                    }
                })
                .collect();
        }

        definitions
    }

    pub fn searchable_tools(&self) -> Vec<EntryDefinition> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<EntryDefinition> {
        self.catalogued_tools()
            .into_iter()
            .filter(EntryDefinition::should_load_by_default)
            .collect()
    }

    pub fn is_concurrency_safe_invocation(&self, invocation: &ToolInvocation) -> bool {
        let invocation = PluginInvocation::from_tool_invocation(invocation);
        self.catalogued_tools().into_iter().any(|definition| {
            definition.name == invocation.entry_name
                && definition.concurrency_safe
                && !definition.requires_user_interaction
        })
    }

    pub fn available_tools_for_messages(&self, messages: &[Message]) -> Vec<EntryDefinition> {
        self.available_tools_for_messages_and_loaded(messages, &[])
    }

    pub fn available_tools_for_messages_and_loaded(
        &self,
        messages: &[Message],
        loaded_tools: &[String],
    ) -> Vec<EntryDefinition> {
        let loaded_tools = collect_loaded_tool_names(messages, loaded_tools);
        self.catalogued_tools()
            .into_iter()
            .filter(|definition| {
                definition.should_load_by_default()
                    || loaded_tools.contains(definition.name.as_str())
            })
            .collect()
    }

    fn invocation_definition(&self, invocation: &ToolInvocation) -> Option<EntryDefinition> {
        self.plugin_invocation_definition(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_definition(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<EntryDefinition> {
        self.catalogued_tools()
            .into_iter()
            .find(|definition| definition.name == invocation.entry_name)
    }

    fn invocation_source_for(&self, invocation: &ToolInvocation) -> EntrySource {
        self.plugin_invocation_source_for(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn plugin_invocation_source_for(&self, invocation: &PluginInvocation) -> EntrySource {
        if let Some(definition) = self.plugin_invocation_definition(invocation) {
            return definition.source;
        }

        self.plugins
            .lookup_entry(invocation.entry_name.as_str())
            .map(|res| EntrySource::Plugin {
                plugin_name: res.handle.plugin_id.clone(),
            })
            .unwrap_or_else(|| EntrySource::Plugin {
                plugin_name: "custom".to_string(),
            })
    }

    fn first_party_from_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Option<FirstPartyToolInput>, ToolError> {
        self.first_party_from_plugin_invocation(&PluginInvocation::from_tool_invocation(invocation))
    }

    fn first_party_from_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
    ) -> Result<Option<FirstPartyToolInput>, ToolError> {
        if !plugin_invocation_uses_local_first_party_bridge(&self.plugins, invocation) {
            return Ok(None);
        }
        let invocation_view =
            ToolInvocation::new(invocation.entry_name.clone(), invocation.input.clone());
        FirstPartyToolInput::from_invocation(&invocation_view)
            .map(Some)
            .ok_or_else(|| {
                ToolError::InvalidInput(format!(
                    "decode first_party input: {}",
                    invocation.entry_name
                ))
            })
    }

    fn invocation_manifest_metadata(
        &self,
        invocation: &ToolInvocation,
    ) -> Option<(SdkEntryBehavior, SdkPlanModePolicy)> {
        self.plugin_invocation_manifest_metadata(&PluginInvocation::from_tool_invocation(
            invocation,
        ))
    }

    fn plugin_invocation_manifest_metadata(
        &self,
        invocation: &PluginInvocation,
    ) -> Option<(SdkEntryBehavior, SdkPlanModePolicy)> {
        self.plugins
            .lookup_entry(invocation.entry_name.as_str())
            .map(|resolution| (resolution.decl.behavior, resolution.decl.plan_mode_policy))
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
            .map(|resolution| resolution.decl.streaming)
    }

    fn authorize_invocation(
        &self,
        invocation: &ToolInvocation,
        first_party: Option<&FirstPartyToolInput>,
    ) -> Result<(String, PermissionDecision), ToolError> {
        if let Some(first_party) = first_party {
            self.ensure_first_party_enabled(first_party)?;
            let tool_name = crate::permission::first_party_tool_name(first_party).to_string();
            return Ok((
                tool_name,
                self.agent.authorize_first_party_tool(first_party),
            ));
        }

        let tool_name = invocation_name(invocation);
        let definition = self
            .invocation_definition(invocation)
            .ok_or_else(|| ToolError::UnknownTool(tool_name.clone()))?;
        if !self.tool_catalog().is_behavior_enabled(definition.behavior) {
            return Err(ToolError::PermissionDenied(format!(
                "tool '{tool_name}' disabled for current model profile"
            )));
        }
        let sensitive = !matches!(definition.behavior, EntryBehavior::ReadOnly);
        Ok((
            tool_name.clone(),
            self.agent
                .authorize_tool_call(tool_name.as_str(), sensitive),
        ))
    }

    fn plugin_resolution_for_invocation(
        &self,
        invocation: &ToolInvocation,
        first_party: Option<&FirstPartyToolInput>,
    ) -> Option<crate::plugin::PluginEntryResolution> {
        self.plugin_resolution_for_plugin_invocation(
            &PluginInvocation::from_tool_invocation(invocation),
            first_party,
        )
    }

    fn plugin_resolution_for_plugin_invocation(
        &self,
        invocation: &PluginInvocation,
        first_party: Option<&FirstPartyToolInput>,
    ) -> Option<crate::plugin::PluginEntryResolution> {
        if let Some(first_party) = first_party {
            return self
                .plugins
                .lookup_entry(crate::permission::first_party_tool_name(first_party));
        }
        self.plugins.lookup_entry(invocation.entry_name.as_str())
    }

    fn collect_declared_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        input: &serde_json::Value,
        specs: &[SdkInputPathSpec],
    ) -> Result<(), ToolError> {
        for path_request in extract_input_path_requests(input, specs)? {
            self.push_requested_path_checks(checks, &path_request.path, path_request.kind);
        }
        Ok(())
    }

    fn collect_dynamic_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        handle: &crate::plugin::PluginEntryHandle,
        input: &serde_json::Value,
    ) -> Result<(), ToolError> {
        let result = self.plugins.dispatch_tool_permission_paths(
            handle,
            PluginToolPermissionPathsInput {
                tool_name: handle.original_name.clone(),
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

    fn push_requested_path_checks(
        &self,
        checks: &mut Vec<ToolPermissionCheck>,
        path: &str,
        kind: SdkPathKind,
    ) {
        let target = self.resolve_target_path(path);
        self.push_path_checks(checks, sdk_path_kind_to_access_kind(kind), &target);
    }

    pub fn execute_first_party_detailed(
        &self,
        input: &FirstPartyToolInput,
    ) -> Result<FirstPartyExecution, ToolError> {
        self.execute_first_party_detailed_with_context(input, FirstPartyExecutionContext::default())
    }

    pub fn execute_first_party_output_for_session(
        &self,
        input: &FirstPartyToolInput,
        session_id: i64,
    ) -> Result<FirstPartyToolOutput, ToolError> {
        self.execute_first_party_detailed_with_context(
            input,
            FirstPartyExecutionContext {
                session_id: Some(session_id),
                call_id: None,
                session_context: None,
            },
        )
        .map(|execution| execution.output)
    }

    #[cfg(test)]
    pub(crate) fn execute_first_party_payload_for_host(
        &self,
        tool_name: &str,
        input: serde_json::Value,
        session_id: Option<i64>,
        call_id: Option<i64>,
    ) -> Result<crate::plugin::ToolInvokeOutput, ToolError> {
        let first_party =
            bundled_router::parse_first_party_tool(tool_name, input).map_err(|err| {
                ToolError::InvalidInput(format!("parse first_party tool {tool_name}: {err}"))
            })?;
        let execution = orchestrator::execute_first_party(
            self,
            &first_party,
            FirstPartyExecutionContext {
                session_id,
                call_id,
                session_context: None,
            },
        )?;
        Ok(bundled_router::first_party_to_invoke_output(
            self.truncator.apply(execution),
        ))
    }

    fn execute_first_party_detailed_with_context(
        &self,
        input: &FirstPartyToolInput,
        context: FirstPartyExecutionContext,
    ) -> Result<FirstPartyExecution, ToolError> {
        let scoped_executor = context
            .session_context
            .as_ref()
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        scoped_executor.ensure_first_party_enabled(input)?;

        if scoped_executor.permission_mode == PermissionExecutionMode::Enforced {
            match scoped_executor.agent.authorize_first_party_tool(input) {
                PermissionDecision::Allow => {}
                PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
                PermissionDecision::Deny { reason } => {
                    return Err(ToolError::PermissionDenied(reason));
                }
            }
        }

        let scoped_executor = context
            .session_context
            .as_ref()
            .map(|session_context| self.for_session_context(session_context))
            .unwrap_or_else(|| self.clone());
        let tool_name = crate::permission::first_party_tool_name(input).to_string();
        let session_id = context.session_id.unwrap_or(-1);
        let call_id = context
            .call_id
            .unwrap_or_else(|| SYNTHETIC_BUILTIN_CALL_ID.fetch_sub(1, Ordering::Relaxed));
        let payload_value = first_party_input_payload(input)?;
        let response = bundled_router::with_executor(
            &scoped_executor,
            session_id,
            call_id,
            tool_name.clone(),
            || {
                bundled_router::invoke_first_party_tool(
                    &tool_name,
                    payload_value.clone(),
                    session_id,
                    call_id,
                )
            },
        )
        .map_err(|err| first_party_plugin_error(tool_name.as_str(), err))?;

        let output = bundled_router::payload_to_first_party_envelope(response.payload.as_ref())
            .map_err(|err| ToolError::Plugin(format!("decode {tool_name} output: {err}")))?;
        let view = ToolExecutionView {
            title: response.title,
            output_text: response.output_text,
            metadata: response.metadata.into_iter().collect(),
            attachments: response.attachments,
        };
        let mut execution = FirstPartyExecution::new(output.output, view);
        if let Some(apply) = output.apply_patch {
            execution = execution.with_apply_patch(apply);
        }
        Ok(scoped_executor.truncator.apply(execution))
    }

    pub fn collect_permission_checks(
        &self,
        input: &FirstPartyToolInput,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        self.collect_permission_checks_for_invocation(&input.clone().into_invocation())
    }

    pub fn prepare_invocation(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<PreparedToolInvocation, ToolError> {
        let tool_name = invocation_name(invocation).to_owned();
        let source = self.invocation_source_for(invocation);
        let input_json = invocation_input_json(invocation)?;
        let input_value: serde_json::Value = serde_json::from_str(&input_json)
            .map_err(|e| ToolError::InvalidInput(e.to_string()))?;

        let hooked = self
            .plugins
            .dispatch_tool_before(PluginToolBeforeInput {
                tool_name: tool_name.clone(),
                source: local_to_sdk_source(&source),
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

        Ok(PreparedToolInvocation {
            invocation: parse_invocation_from_json(
                tool_name.as_str(),
                input_json.as_str(),
                &source,
            )?,
            title_override: hooked.title_override,
            metadata: hooked.metadata.into_iter().collect(),
        })
    }

    pub fn collect_permission_checks_for_invocation(
        &self,
        invocation: &ToolInvocation,
    ) -> Result<Vec<ToolPermissionCheck>, ToolError> {
        let first_party = self.first_party_from_invocation(invocation)?;
        let (tool_name, decision) = self.authorize_invocation(invocation, first_party.as_ref())?;
        let action = if let Some(first_party) = first_party.as_ref() {
            crate::permission::builtin_tool_action(first_party, Some(&self.agent.tool_policy))
        } else {
            PermissionAction::BuiltinTool {
                tool_name: tool_name.clone(),
                qualifier: None,
            }
        };
        let mut checks = vec![ToolPermissionCheck { action, decision }];

        let input_value = invocation_input_value(invocation);
        if let Some(resolution) =
            self.plugin_resolution_for_invocation(invocation, first_party.as_ref())
        {
            self.collect_declared_path_checks(
                &mut checks,
                &input_value,
                &resolution.decl.input_paths,
            )?;
            self.collect_dynamic_path_checks(&mut checks, &resolution.handle, &input_value)?;
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
        if self
            .first_party_from_plugin_invocation(&plugin_invocation)?
            .is_some()
        {
            return Ok(None);
        }

        let resolution = self
            .plugins
            .lookup_entry(plugin_invocation.entry_name.as_str())
            .ok_or_else(|| ToolError::UnknownTool(plugin_invocation.entry_name.clone()))?;
        let stream = self
            .plugins
            .invoke_tool_stream(
                &resolution.handle,
                PluginToolInvokeInput {
                    tool_name: resolution.handle.original_name.clone(),
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
        let tool_name = plugin_invocation.entry_name.clone();
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
                    let mut execution = if let Ok(envelope) =
                        bundled_router::payload_to_first_party_envelope(end.payload.as_ref())
                    {
                        ToolInvocationExecution::new(
                            ToolOutput::Custom {
                                output: envelope.output.into_custom_output(),
                            },
                            view,
                        )
                        .with_apply_patch_option(envelope.apply_patch)
                    } else {
                        let payload = end
                            .payload
                            .as_ref()
                            .map(|value| parse_custom_payload(&value.to_string()))
                            .transpose()?
                            .unwrap_or_default();
                        ToolInvocationExecution::new(
                            ToolOutput::Custom {
                                output: CustomToolOutput {
                                    name: tool_name,
                                    payload,
                                },
                            },
                            view,
                        )
                    };
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
        let result = self.execute_invocation_detailed_inner(invocation, session_id, call_id);
        crate::metrics::record_tool_execution(result.is_ok());
        result
    }

    fn execute_invocation_detailed_inner(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let plugin_invocation = PluginInvocation::from_tool_invocation(invocation);
        let tool_name = plugin_invocation_name(&plugin_invocation);
        let _tool_span =
            tracing::info_span!("tool.call", session_id, call_id, tool = tool_name.as_str(),)
                .entered();
        if let Some(first_party) = self.first_party_from_plugin_invocation(&plugin_invocation)? {
            let mut execution: ToolInvocationExecution = self
                .execute_first_party_detailed_with_context(
                    &first_party,
                    FirstPartyExecutionContext {
                        session_id: Some(session_id),
                        call_id: Some(call_id),
                        session_context: None,
                    },
                )?
                .into();
            self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
            return Ok(execution);
        }
        let resolution = self
            .plugins
            .lookup_entry(plugin_invocation.entry_name.as_str())
            .ok_or_else(|| ToolError::UnknownTool(plugin_invocation.entry_name.clone()))?;

        let response = self
            .plugins
            .invoke_tool(
                &resolution.handle,
                PluginToolInvokeInput {
                    tool_name: resolution.handle.original_name.clone(),
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
        let mut execution = if let Ok(envelope) =
            bundled_router::payload_to_first_party_envelope(response.payload.as_ref())
        {
            ToolInvocationExecution::new(
                ToolOutput::Custom {
                    output: envelope.output.into_custom_output(),
                },
                view,
            )
            .with_apply_patch_option(envelope.apply_patch)
        } else {
            let payload = response
                .payload
                .as_ref()
                .map(|v| parse_custom_payload(&v.to_string()))
                .transpose()?
                .unwrap_or_default();
            ToolInvocationExecution::new(
                ToolOutput::Custom {
                    output: CustomToolOutput {
                        name: plugin_invocation.entry_name.clone(),
                        payload,
                    },
                },
                view,
            )
        };
        self.apply_after_hooks(invocation, session_id, call_id, &mut execution)?;
        Ok(execution)
    }

    pub fn execute_invocation_detailed_bypassing_permissions(
        &self,
        invocation: &ToolInvocation,
        session_id: i64,
        call_id: i64,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let mut trusted = self.clone();
        trusted.permission_mode = PermissionExecutionMode::Bypassed;
        trusted.execute_invocation_detailed(invocation, session_id, call_id)
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

    fn ensure_first_party_enabled(&self, input: &FirstPartyToolInput) -> Result<(), ToolError> {
        let tool_name = crate::permission::first_party_tool_name(input);
        let resolution = self
            .plugins
            .lookup_entry(tool_name)
            .filter(|resolution| is_first_party_plugin(&resolution.handle.plugin_id))
            .ok_or_else(|| ToolError::UnknownTool(tool_name.to_string()))?;
        let definition = EntryDefinition::from_decl(
            resolution.handle.exposed_name.clone(),
            &resolution.decl,
            EntrySource::FirstParty,
        );
        let availability = self
            .tool_catalog()
            .availability_for_definition(&self.agent, &definition);
        if !availability.enabled {
            return Err(ToolError::UnsupportedInvocation(availability.tool_name));
        }
        Ok(())
    }

    pub fn execute_first_party_with_permission_runtime<S>(
        &self,
        session_id: Option<i64>,
        runtime: &mut PermissionRuntime<S>,
        input: &FirstPartyToolInput,
    ) -> Result<PermissionedFirstPartyExecution, ToolError>
    where
        S: PermissionRuleStore,
    {
        let base = self.agent.authorize_first_party_tool(input);
        let action = crate::permission::builtin_tool_action(input, Some(&self.agent.tool_policy));
        match runtime.decide_or_request_with_plugins(session_id, action, base, Some(&self.plugins))
        {
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Allow)) => {
                Ok(PermissionedFirstPartyExecution::Executed(
                    self.execute_first_party_detailed(input)?,
                ))
            }
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Deny { reason })) => {
                Err(ToolError::PermissionDenied(reason))
            }
            Ok(PermissionRuntimeDecision::Immediate(PermissionDecision::Ask { reason })) => {
                Err(ToolError::PermissionAsk(reason))
            }
            Ok(PermissionRuntimeDecision::Pending(request)) => {
                Ok(PermissionedFirstPartyExecution::Pending(request))
            }
            Err(err) => Err(ToolError::InvalidInput(err.to_string())),
        }
    }

    pub fn execute_first_party(
        &self,
        input: &FirstPartyToolInput,
    ) -> Result<(FirstPartyToolOutput, Option<ApplyPatchExecution>), ToolError> {
        let execution = self.execute_first_party_detailed(input)?;
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
        let source = self.invocation_source_for(invocation);
        let payload_value: Option<serde_json::Value> = match &execution.output {
            ToolOutput::Custom { output } => Some(serde_json::Value::from(output.payload.clone())),
            _ => None,
        };

        let after_in = PluginToolAfterInput {
            tool_name,
            source: local_to_sdk_source(&source),
            session_id,
            call_id,
            workspace_root: self.workspace_root.to_string_lossy().to_string(),
            title: execution.view.title.clone(),
            output_text: execution.view.output_text.clone(),
            payload: payload_value,
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

        if let (Some(payload_value), ToolOutput::Custom { output }) =
            (hooked.payload, &mut execution.output)
        {
            output.payload = parse_custom_payload(&payload_value.to_string()).unwrap_or_default();
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
        let source = self.invocation_source_for(invocation);
        let input_value = invocation_input_json(invocation)
            .ok()
            .and_then(|j| serde_json::from_str::<serde_json::Value>(&j).ok())
            .unwrap_or(serde_json::Value::Null);
        let failure_input = PluginToolFailureInput {
            tool_name,
            source: local_to_sdk_source(&source),
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
        shell::execute(request, self.sandbox_policy()).map_err(ToolError::from)
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

    fn ensure_access_permission(
        &self,
        access: AccessKind,
        target_path: &Path,
    ) -> Result<(), ToolError> {
        if self.permission_mode == PermissionExecutionMode::Bypassed {
            return Ok(());
        }

        match self.agent.authorize_path_access(
            AccessKind::ExternalDirectory,
            self.workspace_root(),
            target_path,
        ) {
            PermissionDecision::Allow => {}
            PermissionDecision::Ask { reason } => return Err(ToolError::PermissionAsk(reason)),
            PermissionDecision::Deny { reason } => return Err(ToolError::PermissionDenied(reason)),
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
                access_kind: access_kind_name(AccessKind::ExternalDirectory).to_string(),
                workspace_root: workspace_root.clone(),
                target_path: target.clone(),
            },
            decision: self.agent.authorize_path_access(
                AccessKind::ExternalDirectory,
                self.workspace_root(),
                target_path,
            ),
        });
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
}

pub(crate) fn normalize_path_for_display(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

fn access_kind_name(access: AccessKind) -> &'static str {
    match access {
        AccessKind::Read => "read",
        AccessKind::Write => "write",
        AccessKind::ExternalDirectory => "external_directory",
    }
}

fn invocation_name(invocation: &ToolInvocation) -> String {
    plugin_invocation_name(&PluginInvocation::from_tool_invocation(invocation))
}

fn plugin_invocation_name(invocation: &PluginInvocation) -> String {
    invocation.entry_name.clone()
}

fn local_to_sdk_source(source: &EntrySource) -> SdkEntrySource {
    match source {
        EntrySource::FirstParty => SdkEntrySource::FirstParty,
        EntrySource::Plugin { plugin_name } => SdkEntrySource::Plugin {
            plugin: plugin_name.clone(),
        },
    }
}

/// Serialize a `FirstPartyToolInput` to the JSON shape expected by the
/// in-process first_party plugin. The plugin parses by tool name (passed as
/// `ToolInvokeInput::tool_name`) and consumes the inner payload only.
fn first_party_input_payload(input: &FirstPartyToolInput) -> Result<serde_json::Value, ToolError> {
    let value = serde_json::to_value(input)
        .map_err(|err| ToolError::Plugin(format!("encode first_party input: {err}")))?;
    let mut obj = match value {
        serde_json::Value::Object(o) => o,
        other => return Ok(other),
    };
    obj.remove("tool");
    Ok(serde_json::Value::Object(obj))
}

fn first_party_plugin_error(tool_name: &str, err: crate::plugin::PluginError) -> ToolError {
    let prefix = format!("{tool_name}: ");
    let detail = err.message.strip_prefix(&prefix).unwrap_or(&err.message);
    if let Some(reason) = detail.strip_prefix("permission denied: ") {
        return ToolError::PermissionDenied(reason.to_string());
    }
    if let Some(reason) = detail.strip_prefix("permission confirmation required: ") {
        return ToolError::PermissionAsk(reason.to_string());
    }
    ToolError::Plugin(err.message)
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
    _source: &EntrySource,
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
            Some(PartContent::ToolExecution(ToolExecutionPart::Completed { details, .. })) => {
                match details.as_first_party() {
                    Some(FirstPartyToolOutput::ToolSearch { loaded_tools, .. }) => {
                        Some(loaded_tools)
                    }
                    _ => None,
                }
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

fn parse_custom_payload(payload_json: &str) -> Result<StructuredObject, ToolError> {
    let value = if payload_json.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(payload_json)
            .map_err(|err| ToolError::InvalidInput(err.to_string()))?
    };
    StructuredObject::try_from(value).map_err(|err| ToolError::InvalidInput(err.to_string()))
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
        ApplyPatchToolInput, BashToolInput, FileChangeKind, FirstPartyToolInput,
        FirstPartyToolOutput, GlobToolInput, GrepToolInput, Message, PartContent, ReadToolInput,
        StructuredObject, TaskSubagentType, TaskToolInput, TimeRange, TodoItem, TodoPriority,
        TodoStatus, TodoWriteToolInput, ToolExecutionPart, ToolInvocation, ToolOutput,
        ToolSearchToolInput, ViewFileToolInput,
    };
    use crate::permission::PermissionPolicy;
    use crate::plugin::sdk::host_api::{
        EventSubscription, LogLevel, SpawnSubtaskRequest, SpawnSubtaskResponse, ToolDescriptor,
    };
    use crate::plugin::sdk::prelude::*;
    use crate::plugin::sdk::{
        EventEnvelope, EventFilter, PermissionAskInput, PermissionDecision, Result as SdkResult,
    };
    use crate::plugin::{PluginEntry, PluginHost, PluginHostBuilder, PluginsConfig};
    use crate::role::Role;

    use super::{EntrySource, ExecutionPolicy, ToolError, ToolExecutor};
    use crate::plugins::bundled::router as bundled_router;

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
        ToolExecutor::new(root, agent).with_plugin_manager(build_first_party_plugin_manager(root))
    }

    fn build_executor_with_policy(root: &Path, policy: ExecutionPolicy) -> ToolExecutor {
        let agent = crate::agent::Agent::new("build", PermissionPolicy::allow_all());
        ToolExecutor::with_sandbox_policy(root, agent, policy)
            .with_plugin_manager(build_first_party_plugin_manager(root))
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
                name: "apply_patch".to_string(),
                description: Some("Patch files in the workspace".to_string()),
                search_terms: vec!["patch".to_string(), "files".to_string()],
                behavior: Some("mutating".to_string()),
                deferred: true,
                read_only: false,
                plugin_id: None,
            }])
        }

        async fn todo_write(
            &self,
            req: crate::plugin::sdk::host_api::HostTodoWriteRequest,
        ) -> SdkResult<ToolInvokeOutput> {
            let context =
                crate::plugin::sdk::host_api::current_host_callback_context().unwrap_or_default();
            let session_id = context.session_id.unwrap_or(-1);
            let call_id = context.call_id.unwrap_or(-1);
            let executor =
                bundled_router::current_executor_for_test(session_id, call_id, "todo_write")
                    .ok_or_else(|| {
                        PluginError::new("test host: no executor available for todo_write")
                    })?;
            executor
                .execute_first_party_payload_for_host(
                    "todo_write",
                    serde_json::to_value(req).map_err(|err| PluginError::new(err.to_string()))?,
                    Some(session_id).filter(|id| *id >= 0),
                    Some(call_id).filter(|id| *id >= 0),
                )
                .map_err(|err| PluginError::new(err.to_string()))
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
                .entry(
                    PluginEntryDecl::new(
                        "plugin_echo",
                        json!({
                            "type": "object",
                            "properties": { "message": { "type": "string" } },
                            "required": ["message"]
                        }),
                    )
                    .description("Echo a message from the plugin.")
                    .behavior(crate::plugin::sdk::EntryBehavior::ReadOnly),
                )
                .entry(
                    PluginEntryDecl::new(
                        "plugin_paths",
                        json!({
                            "type": "object",
                            "properties": {
                                "file_path": { "type": "string" },
                                "extra_paths": {
                                    "type": "array",
                                    "items": { "type": "string" }
                                },
                                "dynamic_path": { "type": "string" }
                            },
                            "required": ["file_path"]
                        }),
                    )
                    .description("Expose declared and dynamic permission paths.")
                    .behavior(crate::plugin::sdk::EntryBehavior::ReadOnly)
                    .input_path(InputPathSpec {
                        jsonpath: "$.file_path".to_string(),
                        kind: PathKind::Read,
                        optional: false,
                    })
                    .input_path(InputPathSpec {
                        jsonpath: "$.extra_paths[*]".to_string(),
                        kind: PathKind::Read,
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

        let skills_fs_id = super::skills_fs_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let web_id = super::web_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_fs_id,
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
                .register_static(skills_fs_id, super::new_skills_fs_plugin())
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

    fn build_first_party_plugin_manager(root: &Path) -> Arc<PluginHost> {
        use std::collections::BTreeMap;

        let skills_fs_id = super::skills_fs_plugin_id().to_string();
        let lsp_id = super::lsp_plugin_id().to_string();
        let cron_id = super::cron_plugin_id().to_string();
        let fs_id = super::fs_plugin_id().to_string();
        let shell_id = super::shell_plugin_id().to_string();
        let web_id = super::web_plugin_id().to_string();
        let workflow_id = super::workflow_plugin_id().to_string();
        let mut list = BTreeMap::new();
        for id in [
            &skills_fs_id,
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
                .register_static(skills_fs_id, super::new_skills_fs_plugin())
                .register_static(lsp_id, super::new_lsp_plugin())
                .register_static(cron_id, super::new_cron_plugin())
                .register_static(fs_id, super::new_fs_plugin())
                .register_static(shell_id, super::new_shell_plugin())
                .register_static(web_id, super::new_web_plugin())
                .register_static(workflow_id, super::new_workflow_plugin())
                .build()
                .await
                .expect("first_party plugin host should build")
        })
    }

    fn loaded_tool_search_message(loaded_tools: &[&str]) -> Message {
        Message {
            id: 99,
            role: Role::Tool,
            state: crate::message::MessageStatus::Completed,
            parts: vec![crate::message::MessagePart::with_content(
                1,
                99,
                Utc::now(),
                crate::message::ExecutionStatus::Completed,
                PartContent::ToolExecution(ToolExecutionPart::Completed {
                    call_id: 1,
                    invocation: FirstPartyToolInput::ToolSearch(ToolSearchToolInput {
                        query: "load mutating tools".to_string(),
                        load: loaded_tools.iter().map(|name| name.to_string()).collect(),
                        limit: None,
                    })
                    .into_invocation(),
                    output_text: "loaded deferred tools".to_string(),
                    blocks: Vec::new(),
                    attachments: Vec::new(),
                    details: ToolOutput::Custom {
                        output: FirstPartyToolOutput::ToolSearch {
                            results: Vec::new(),
                            loaded_tools: loaded_tools
                                .iter()
                                .map(|name| name.to_string())
                                .collect(),
                        }
                        .into_custom_output(),
                    },
                    lifecycle: TimeRange::default(),
                }),
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
    fn read_first_party_returns_line_numbered_preview() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "one\ntwo\nthree\n").expect("failed to seed file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Read(ReadToolInput {
                file_path: "notes.txt".to_string(),
                offset: Some(2),
                limit: Some(2),
            }))
            .expect("read first_party should succeed");

        match result.output {
            FirstPartyToolOutput::Read {
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
    fn apply_patch_first_party_reports_typed_file_changes() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("keep.txt"), "before\n").expect("failed to seed keep.txt");
        fs::write(workspace.root.join("remove.txt"), "delete me\n")
            .expect("failed to seed remove.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::ApplyPatch(ApplyPatchToolInput {
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
            FirstPartyToolOutput::ApplyPatch { changes, .. } => {
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
    fn apply_patch_first_party_moves_files_and_reports_diff() {
        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("old.txt"), "before\n").expect("failed to seed old.txt");
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::ApplyPatch(ApplyPatchToolInput {
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
            FirstPartyToolOutput::ApplyPatch {
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
    fn view_file_first_party_returns_metadata_and_attachment() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("pixel.png");
        fs::write(&file_path, sample_png_bytes()).expect("failed to seed png");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::ViewFile(ViewFileToolInput {
                path: "pixel.png".to_string(),
            }))
            .expect("view_file should succeed");

        match result.output {
            FirstPartyToolOutput::ViewFile {
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
    fn view_file_first_party_attaches_generic_text_file() {
        let workspace = TempWorkspace::new();
        let file_path = workspace.root.join("notes.txt");
        fs::write(&file_path, "hello from agena\n").expect("failed to seed text file");

        let executor = build_executor(&workspace.root);
        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::ViewFile(ViewFileToolInput {
                path: "notes.txt".to_string(),
            }))
            .expect("view_file should succeed for text file");

        match result.output {
            FirstPartyToolOutput::ViewFile {
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
            .execute_first_party_detailed(&FirstPartyToolInput::Glob(GlobToolInput {
                pattern: "**/*.rs".to_string(),
                path: Some("src".to_string()),
            }))
            .expect("glob should succeed");

        match glob_result.output {
            FirstPartyToolOutput::Glob { count } => {
                assert_eq!(count, Some(2));
            }
            other => panic!("expected glob output, got {other:?}"),
        }

        let grep_result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Grep(GrepToolInput {
                pattern: "hello".to_string(),
                path: Some("src".to_string()),
                include: Some("**/*.rs".to_string()),
            }))
            .expect("grep should succeed");

        match grep_result.output {
            FirstPartyToolOutput::Grep { matches } => {
                assert_eq!(matches, Some(1));
            }
            other => panic!("expected grep output, got {other:?}"),
        }
    }

    #[test]
    fn task_plugin_entry_generates_session_id() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = FirstPartyToolInput::Task(TaskToolInput {
            description: "inspect code".to_string(),
            prompt: "find modules".to_string(),
            subagent_type: TaskSubagentType::Explore,
            task_id: None,
            command: None,
        })
        .into_invocation();

        let result = executor
            .execute_invocation_detailed(&invocation, 7, 9)
            .expect("task plugin entry should succeed");

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

        match result.output {
            ToolOutput::Custom { output } => {
                assert_eq!(output.name, "task");
                let payload = FirstPartyToolOutput::from_custom(&output)
                    .expect("task output should decode as first_party payload");
                match payload {
                    FirstPartyToolOutput::Task { session_id, .. } => {
                        assert!(session_id.is_some());
                    }
                    other => panic!("expected task payload, got {other:?}"),
                }
            }
            other => panic!("expected custom task output, got {other:?}"),
        }
    }

    #[test]
    fn tool_search_first_party_discovers_and_loads_deferred_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::ToolSearch(ToolSearchToolInput {
                query: "patch files".to_string(),
                load: vec!["apply_patch".to_string()],
                limit: None,
            }))
            .expect("tool_search should succeed");

        match result.output {
            FirstPartyToolOutput::ToolSearch {
                results,
                loaded_tools,
            } => {
                assert!(results.iter().any(|name| name == "apply_patch"));
                assert_eq!(loaded_tools, vec!["apply_patch".to_string()]);
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
        assert!(initial.iter().any(|tool| tool.name == "tool_search"));
        assert!(initial.iter().any(|tool| tool.name == "todo_write"));
        assert!(!initial.iter().any(|tool| tool.name == "bash"));
        assert!(!initial.iter().any(|tool| tool.name == "task"));

        let messages = vec![loaded_tool_search_message(&["bash", "task"])];
        let available = executor.available_tools_for_messages(messages.as_slice());

        assert!(available.iter().any(|tool| tool.name == "bash"));
        assert!(available.iter().any(|tool| tool.name == "task"));
    }

    #[test]
    fn first_partys_plugin_entries_drive_available_tool_catalog() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_first_party_plugin_manager(&workspace.root));

        let tools = executor.available_tools();
        let read = tools
            .iter()
            .find(|tool| tool.name == "read")
            .expect("read tool should be available");
        assert!(matches!(read.source, EntrySource::FirstParty));
        assert!(read.search_terms.iter().any(|term| term == "open file"));

        let read_count = tools.iter().filter(|tool| tool.name == "read").count();
        assert_eq!(read_count, 1);
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
                    .lookup_entry(definition.name.as_str())
                    .is_some(),
                "missing registry entry for {}",
                definition.name
            );
        }

        for definition in executor.searchable_tools() {
            assert!(
                executor
                    .plugin_manager()
                    .lookup_entry(definition.name.as_str())
                    .is_some(),
                "missing registry entry for {}",
                definition.name
            );
        }
    }

    #[test]
    fn available_first_party_tools_are_projected_from_first_party_plugin_entries() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_first_party_plugin_manager(&workspace.root));

        let available = executor
            .available_first_party_tools()
            .into_iter()
            .map(|item| item.tool_name)
            .collect::<std::collections::BTreeSet<_>>();
        let registry = executor
            .plugin_manager()
            .entry_entries()
            .into_iter()
            .filter(|entry| super::is_first_party_plugin(&entry.plugin_name))
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
            .map(|tool| tool.name)
            .collect::<Vec<_>>();
        let mut expected = names.clone();
        expected.sort();

        assert_eq!(names, expected);
    }

    #[test]
    fn todo_write_first_party_returns_items_for_session_state() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::TodoWrite(TodoWriteToolInput {
                items: vec![TodoItem {
                    content: "Implement tool_search".to_string(),
                    status: TodoStatus::InProgress,
                    priority: TodoPriority::High,
                }],
            }))
            .expect("todo_write should succeed");

        match result.output {
            FirstPartyToolOutput::TodoWrite { items } => {
                assert_eq!(items.len(), 1);
                assert_eq!(items[0].content, "Implement tool_search");
                assert_eq!(items[0].status, TodoStatus::InProgress);
            }
            other => panic!("expected todo_write output, got {other:?}"),
        }
    }

    #[test]
    fn bash_first_party_runs_command_with_read_only_policy() {
        if cfg!(windows) {
            // Windows host environments can include PATH entries whose ACL cannot be audited
            // in sandbox preflight, which makes this smoke test flaky/non-portable.
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(&workspace.root, ExecutionPolicy::read_only());

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Bash(BashToolInput {
                command: "echo hello_agena".to_string(),
                description: "smoke bash".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash first_party should succeed");

        match &result.output {
            FirstPartyToolOutput::Bash {
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
    fn bash_first_party_explains_no_match_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("notes.txt"), "alpha\nbeta\n")
            .expect("failed to seed notes file");
        let executor = build_executor_with_policy(&workspace.root, ExecutionPolicy::read_only());

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Bash(BashToolInput {
                command: "grep missing notes.txt".to_string(),
                description: "search missing text".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash first_party should succeed");

        match result.output {
            FirstPartyToolOutput::Bash {
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
    fn bash_first_party_explains_diff_exit_codes() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        fs::write(workspace.root.join("left.txt"), "alpha\n").expect("failed to write left file");
        fs::write(workspace.root.join("right.txt"), "beta\n").expect("failed to write right file");
        let executor = build_executor_with_policy(&workspace.root, ExecutionPolicy::read_only());

        let result = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Bash(BashToolInput {
                command: "diff left.txt right.txt".to_string(),
                description: "compare files".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect("bash first_party should succeed");

        match &result.output {
            FirstPartyToolOutput::Bash { description, .. } => {
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
    fn bash_first_party_blocks_obvious_write_commands_in_read_only_policy() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(&workspace.root, ExecutionPolicy::read_only());

        let err = executor
            .execute_first_party_detailed(&FirstPartyToolInput::Bash(BashToolInput {
                command: "echo hi > created.txt".to_string(),
                description: "attempt write".to_string(),
                timeout_ms: Some(30_000),
                workdir: None,
            }))
            .expect_err("write command should be rejected before execution");

        match err {
            ToolError::PermissionDenied(message) => {
                assert!(message.contains("read-only sandbox"));
                assert!(message.contains("output redirection"));
            }
            other => panic!("expected permission denial, got {other:?}"),
        }
    }

    #[test]
    fn readonly_model_profile_disables_apply_patch_and_task_tools() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root).with_model_id("gpt-readonly");

        let availability = executor.available_first_party_tools();
        let find = |tool_name: &str| {
            availability
                .iter()
                .find(|item| item.tool_name == tool_name)
                .expect("tool should exist")
                .enabled
        };

        assert!(find("read"));
        assert!(!find("apply_patch"));
        assert!(!find("task"));
    }

    #[test]
    fn plugin_custom_tool_hooks_prepare_and_mutate_execution() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        assert!(executor.available_tools().iter().any(|tool| {
            tool.name == "plugin_echo"
                && matches!(
                    tool.source,
                    EntrySource::Plugin { ref plugin_name } if plugin_name == "fixture"
                )
        }));

        let invocation = ToolInvocation {
            name: "plugin_echo".to_string(),
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

        match execution.output {
            ToolOutput::Custom { output } => {
                let payload = serde_json::Value::from(output.payload);
                assert_eq!(output.name, "plugin_echo");
                assert_eq!(payload["echoed"], "hello prepared");
                assert_eq!(payload["after"], true);
            }
            other => panic!("expected custom output, got {other:?}"),
        }

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
    fn prepare_invocation_keeps_first_party_calls_in_custom_wire_shape() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = FirstPartyToolInput::Read(ReadToolInput {
            file_path: "notes.txt".to_string(),
            offset: Some(3),
            limit: Some(5),
        })
        .into_invocation();

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should succeed for first_party");

        let ToolInvocation { name, input } = prepared.invocation;
        assert_eq!(name, "read");
        let payload = serde_json::Value::from(input);
        assert_eq!(payload["file_path"], "notes.txt");
        assert_eq!(payload["offset"], 3);
        assert_eq!(payload["limit"], 5);
    }

    #[test]
    fn prepare_invocation_preserves_plugin_entry_name() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root);
        let invocation = ToolInvocation {
            name: "mcp:docs:search".to_string(),
            input: StructuredObject::try_from(json!({ "query": "plugin host" }))
                .expect("structured object should build"),
        };

        let prepared = executor
            .prepare_invocation(&invocation, 7, 9)
            .expect("prepare should preserve plugin entry invocation");

        let ToolInvocation { name, input } = prepared.invocation;
        assert_eq!(name, "mcp:docs:search");
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
            input: StructuredObject::try_from(json!({
                "file_path": "docs/spec.md",
                "extra_paths": ["notes/a.md", "notes/b.md"],
                "dynamic_path": "logs/output.txt"
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
    }

    #[test]
    fn collect_permission_checks_for_first_party_invocation_uses_dynamic_plugin_paths() {
        let workspace = TempWorkspace::new();
        let executor = build_executor(&workspace.root)
            .with_plugin_manager(build_first_party_plugin_manager(&workspace.root));
        let invocation = FirstPartyToolInput::ApplyPatch(ApplyPatchToolInput {
            patch: "*** Begin Patch\n*** Add File: notes.txt\n+hello\n*** Delete File: old.txt\n*** End Patch"
                .to_string(),
        })
        .into_invocation();

        let checks = executor
            .collect_permission_checks_for_invocation(&invocation)
            .expect("first_party permission collection should succeed");

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
    fn bash_invocation_applies_plugin_shell_env_overrides() {
        if cfg!(windows) {
            return;
        }

        let workspace = TempWorkspace::new();
        let executor = build_executor_with_policy(&workspace.root, ExecutionPolicy::read_only())
            .with_plugin_manager(build_plugin_manager(&workspace.root));

        let execution = executor
            .execute_invocation_detailed(
                &FirstPartyToolInput::Bash(BashToolInput {
                    command: "printf %s \"$PLUGIN_FLAG\"".to_string(),
                    description: "print plugin env".to_string(),
                    timeout_ms: Some(30_000),
                    workdir: None,
                })
                .into_invocation(),
                10,
                11,
            )
            .expect("bash invocation should succeed");

        match execution.output.as_first_party() {
            Some(FirstPartyToolOutput::Bash {
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
        use crate::message::{StructuredField, StructuredObject, StructuredValue};
        use crate::session::PlanState;

        let workspace = TempWorkspace::new();
        let registry = super::plan_registry_for_executor();
        let executor =
            build_executor_with_policy(&workspace.root, ExecutionPolicy::workspace_write())
                .with_plan_registry(registry.clone());

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
            let mut input = StructuredObject::default();
            input.fields.push(StructuredField {
                name: "command".to_string(),
                value: StructuredValue::Text {
                    value: cmd.to_string(),
                },
            });
            ToolInvocation {
                name: "bash".to_string(),
                input,
            }
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
        use crate::message::{StructuredField, StructuredObject, StructuredValue};
        use crate::session::PlanState;

        let workspace = TempWorkspace::new();
        let registry = super::plan_registry_for_executor();
        let executor =
            build_executor_with_policy(&workspace.root, ExecutionPolicy::workspace_write())
                .with_plan_registry(registry.clone());
        registry.write().insert(
            42,
            PlanState {
                file_path: workspace.root.join(".agena/plans/x.md"),
                slug: "x".to_string(),
                started_at: chrono::Utc::now(),
            },
        );

        let mut input = StructuredObject::default();
        input.fields.push(StructuredField {
            name: "command".to_string(),
            value: StructuredValue::Text {
                value: "rm -rf /".to_string(),
            },
        });
        let inv = ToolInvocation {
            name: "bash".to_string(),
            input,
        };

        // Different session id — plan mode does not apply.
        executor
            .enforce_plan_mode_for(&inv, 1)
            .expect("session 1 is not in plan mode");

        // Same session id — plan mode blocks.
        let err = executor.enforce_plan_mode_for(&inv, 42).unwrap_err();
        assert!(matches!(err, ToolError::PermissionDenied(_)));
    }
}
