impl ToolExecutor {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        agent: Agent,
        subagent_registry: crate::agents::SubagentRegistry,
        plugins: Arc<PluginHost>,
        snapshot_registry: Option<snapshot::SnapshotRegistry>,
        scheduler: Option<Arc<agena_scheduler::Scheduler>>,
        lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
        tool_presentation: crate::plugin::ToolPresentationConfig,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            agent,
            model_id: None,
            subagent_registry,
            monitor_registry: monitor::default_registry(),
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

    pub fn snapshot_registry(&self) -> Option<&snapshot::SnapshotRegistry> {
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
    /// through this scoped executor, including nested in-process gateway
    /// calls. Cloning an executor preserves the signal.
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
        error: crate::plugin::sdk::PluginError,
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

    pub fn tool_catalog(&self) -> ToolCatalog {
        ToolCatalog::for_model(self.model_id.as_deref())
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

    pub(crate) fn catalogued_tools_raw(&self) -> Vec<RegisteredTool> {
        let catalog = self.tool_catalog();
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|entry| catalog.is_tool_enabled(entry))
            .filter(|entry| self.is_tool_visible_to_agent(entry))
            .collect()
    }

    pub(crate) fn is_tool_visible_to_agent(&self, entry: &RegisteredTool) -> bool {
        let model_name = entry.canonical_name();
        !matches!(
            self.agent
                .authorize_tool_names(&[model_name.as_str()], None, &entry.effective_tags()),
            PermissionDecision::Deny { .. }
        )
    }

    pub(crate) fn catalogued_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool(entry, &self.tool_presentation))
            .collect()
    }

    pub fn detailed_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools_raw()
            .into_iter()
            .map(|entry| present_registered_tool_detailed(entry, &self.tool_presentation))
            .collect()
    }

    pub fn searchable_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_tools(&self) -> Vec<RegisteredTool> {
        self.catalogued_tools()
    }

    pub fn available_gateway_tools(&self) -> Vec<GatewayToolBinding> {
        let mut tools = self
            .catalogued_tools()
            .into_iter()
            .filter_map(GatewayToolBinding::from_registered_tool)
            .collect::<Vec<_>>();
        tools.sort_by_key(GatewayToolBinding::function);
        tools
    }

    pub fn gateway_tool_prompt_text(&self) -> Option<String> {
        let tools = self
            .detailed_tools()
            .into_iter()
            .filter(|tool| !is_gateway_handler(tool))
            .collect::<Vec<_>>();
        if tools.is_empty() {
            return None;
        }

        let mut lines = vec![
            "Tool protocol: only gateway tools are callable function tools.".to_string(),
            format!(
                "Use `{}`, `{}`, `{}`, `{}`, and `{}` for tool discovery and execution.",
                GATEWAY_FUNCTION_LIST,
                GATEWAY_FUNCTION_SEARCH,
                GATEWAY_FUNCTION_HELP,
                GATEWAY_FUNCTION_TAGS,
                GATEWAY_FUNCTION_CALL
            ),
            format!(
                "To execute a real tool, call `{}` with `{{ \"tool\": \"...\", \"input\": {{ ... }} }}`.",
                GATEWAY_FUNCTION_CALL
            ),
            "Before every tools_call, inspect tools_help for that exact target. One help result authorizes one later tools_call of the same target and is consumed by the call.".to_string(),
            "Available tools:".to_string(),
        ];
        lines.extend(tools.iter().map(render_catalog_tool_index_entry));
        Some(lines.join("\n"))
    }

    pub(crate) fn suggested_tool_names(&self, requested: &str) -> Vec<String> {
        let mut candidates = self
            .catalogued_tools_raw()
            .into_iter()
            .map(|tool| catalog_target_name(tool.canonical_name().as_str()))
            .collect::<Vec<_>>();
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
    Agent, Arc, GATEWAY_FUNCTION_CALL, GATEWAY_FUNCTION_HELP, GATEWAY_FUNCTION_LIST,
    GATEWAY_FUNCTION_SEARCH, GATEWAY_FUNCTION_TAGS, MonitorService, Path, PathBuf,
    PermissionDecision, PermissionEnforcementMode, PluginHost, PluginToolDefinitionInput,
    RegisteredTool, ToolCatalog, ToolError, ToolExecutor, ToolOutputTruncator, catalog_target_name,
    is_gateway_handler, monitor, present_registered_tool, present_registered_tool_detailed,
    render_catalog_tool_index_entry, snapshot, suggest_tool_names, tool_summary, unknown_tool_hint,
};
use crate::tool::GatewayToolBinding;
