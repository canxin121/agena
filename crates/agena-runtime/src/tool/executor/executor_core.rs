impl ToolExecutor {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        subagent_registry: crate::agents::SubagentRegistry,
        plugins: Arc<PluginHost>,
        snapshot_registry: Option<crate::SnapshotRegistry>,
        scheduler: Option<Arc<agena_scheduler::Scheduler>>,
        lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
        tool_presentation: agena_plugin_host::ToolPresentationConfig,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subagent_registry,
            monitor_registry: agena_runtime::default_monitor_registry(),
            truncator: ToolOutputTruncator::default(),
            plugins,
            snapshot_registry,
            scheduler,
            lsp_registry,
            permission_mode: PermissionEnforcementMode::Enforced,
            tool_presentation,
            cancellation_token: None,
        }
    }

    pub fn subagent_registry(&self) -> &crate::agents::SubagentRegistry {
        &self.subagent_registry
    }

    pub fn snapshot_registry(&self) -> Option<&crate::SnapshotRegistry> {
        self.snapshot_registry.as_ref()
    }

    pub fn scheduler(&self) -> Option<&Arc<agena_scheduler::Scheduler>> {
        self.scheduler.as_ref()
    }

    pub fn lsp_registry(&self) -> Option<&Arc<agena_lsp::LspRegistry>> {
        self.lsp_registry.as_ref()
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
                .apply_permission_config_or_self(&session_context.effective_permission);
        }
        if !session_context.permission_ceiling.is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .apply_permission_ceiling_or_self(&session_context.permission_ceiling);
        }
        if !session_context.allowed_tools.is_empty() {
            scoped.agent = scoped.agent.clone().restricted_to_allowed_tools(
                session_context.allowed_tools.iter().map(String::as_str),
            );
        }
        if let Some(model_id) = session_context.selection.model.as_ref() {
            scoped.model_id = Some(model_id.clone());
        }
        scoped
    }

    /// Attach the execution-wide cancellation signal to every tool reached
    /// through this scoped executor, including execution tools reached through
    /// `tools_call`. Cloning an executor preserves the signal.
    pub(crate) fn with_cancellation_token(
        mut self,
        cancellation_token: Option<tokio_util::sync::CancellationToken>,
    ) -> Self {
        self.cancellation_token = cancellation_token;
        self
    }

    pub(crate) fn cancellation_token(&self) -> Option<&tokio_util::sync::CancellationToken> {
        self.cancellation_token.as_ref()
    }

    pub(crate) fn ensure_not_cancelled(&self) -> Result<(), ToolError> {
        if self
            .cancellation_token
            .as_ref()
            .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        {
            return Err(ToolError::Cancelled);
        }
        Ok(())
    }

    pub(crate) fn plugin_error_or_cancelled(
        &self,
        error: agena_plugin_host::sdk::PluginError,
    ) -> ToolError {
        if self
            .cancellation_token
            .as_ref()
            .is_some_and(tokio_util::sync::CancellationToken::is_cancelled)
        {
            ToolError::Cancelled
        } else {
            ToolError::Plugin(error.message)
        }
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

    pub fn builtin_tool_set(&self) -> BuiltinToolSet {
        BuiltinToolSet::for_model(self.model_id.as_deref())
    }

    pub(crate) fn registered_tools_with_definition_overrides(&self) -> Vec<RegisteredTool> {
        let mut tools = self
            .plugins
            .registered_tools()
            .into_iter()
            .collect::<Vec<_>>();

        tools.sort_by(|left, right| {
            left.canonical_name()
                .cmp(&right.canonical_name())
                .then_with(|| left.summary_text().cmp(&right.summary_text()))
        });

        // Plugin chain: tool.definition. Let plugins rewrite summaries /
        // input schemas before the list reaches the LLM.
        if !self.plugins.is_empty() {
            tools = tools
                .into_iter()
                .map(|mut entry| {
                    let input = PluginToolDefinitionInput {
                        tool: entry.tool_key().clone(),
                        summary: tool_summary(&entry),
                        help: entry.definition.docs.help.clone(),
                        description_mode: entry.definition.display.description_mode,
                        input_schema: entry.input_schema(),
                    };
                    match self.plugins.dispatch_tool_definition_blocking(input) {
                        Ok(patched) => {
                            entry.definition.docs.summary = Some(patched.summary);
                            entry.definition.docs.help = patched.help;
                            entry.definition.display.description_mode = patched.description_mode;
                            entry.definition.contract.input_schema = patched.input_schema;
                            entry
                        }
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::tool_definition",
                                tool = %entry.canonical_name(),
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

    pub(crate) fn available_registered_tools(&self) -> Vec<RegisteredTool> {
        let tool_set = self.builtin_tool_set();
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|entry| tool_set.is_tool_enabled(entry))
            .filter(|entry| self.is_tool_visible_to_agent(entry))
            .collect()
    }

    pub(crate) fn is_tool_visible_to_agent(&self, entry: &RegisteredTool) -> bool {
        // Tool API handlers form the provider protocol and carry no execution
        // authority of their own. They must remain available even when an
        // agent profile restricts the execution-tool catalog; the selected
        // target is filtered and authorized separately inside `tools_call`.
        if crate::tool::is_tool_api_handler(entry) {
            return self.agent.can_access_tool_api();
        }
        let model_name = entry.canonical_name();
        !matches!(
            self.agent
                .authorize_tool_names(&[model_name.as_str()], None, &entry.effective_tags()),
            PermissionDecision::Deny { .. }
        )
    }

    pub fn detailed_tools(&self) -> Vec<RegisteredTool> {
        self.available_registered_tools()
            .into_iter()
            .map(|entry| present_registered_tool_detailed(entry, &self.tool_presentation))
            .collect()
    }

    pub fn detailed_execution_tools(&self) -> Vec<crate::tool::ExecutionTool> {
        self.detailed_tools()
            .into_iter()
            .filter_map(crate::tool::ExecutionTool::from_registered_tool)
            .collect()
    }

    pub fn available_tools(&self) -> Vec<RegisteredTool> {
        self.available_registered_tools()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    pub fn available_execution_tools(&self) -> Vec<crate::tool::ExecutionTool> {
        self.available_tools()
            .into_iter()
            .filter_map(crate::tool::ExecutionTool::from_registered_tool)
            .collect()
    }

    pub fn available_tool_api_bindings(&self) -> Vec<ToolApiBinding> {
        let mut tools = self
            .available_tools()
            .into_iter()
            .filter_map(ToolApiBinding::from_registered_tool)
            .collect::<Vec<_>>();
        tools.sort_by_key(ToolApiBinding::function);
        tools
    }

    pub(crate) fn suggested_tool_names(&self, requested: &str) -> Vec<String> {
        let tools = self
            .available_registered_tools()
            .into_iter()
            .filter_map(crate::tool::ExecutionTool::from_registered_tool)
            .collect::<Vec<_>>();
        let mut candidates = crate::tool::execution_tool_names(&tools);
        candidates.sort();
        candidates.dedup();
        suggest_tool_names(requested, candidates, 1)
    }

    pub(crate) fn unknown_tool_error(&self, requested: &str) -> ToolError {
        let suggestions = self.suggested_tool_names(requested);
        if suggestions.is_empty() {
            ToolError::UnknownTool {
                tool: requested.to_string(),
            }
        } else {
            unknown_tool_hint(requested, suggestions)
        }
    }
}
use super::{
    Agent, Arc, BuiltinToolSet, MonitorService, Path, PathBuf, PermissionDecision,
    PermissionEnforcementMode, PluginHost, PluginToolDefinitionInput, RegisteredTool, ToolError,
    ToolExecutor, ToolOutputTruncator, present_registered_tool, present_registered_tool_detailed,
    suggest_tool_names, tool_summary, unknown_tool_hint,
};
use crate::tool::ToolApiBinding;
