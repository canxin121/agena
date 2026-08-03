impl ToolExecutor {
    pub fn new(
        workspace_root: impl Into<PathBuf>,
        principal: ExecutionPrincipal,
        plugins: Arc<PluginHost>,
        snapshot_registry: Option<crate::SnapshotRegistry>,
        scheduler: Option<Arc<agena_scheduler::Scheduler>>,
        lsp_registry: Option<Arc<agena_lsp::LspRegistry>>,
        tool_presentation: agena_plugin_host::ToolPresentationConfig,
    ) -> Self {
        Self {
            workspace_root: workspace_root.into(),
            principal,
            allowed_tool_names: None,
            model_id: None,
            monitor_registry: crate::default_monitor_registry(),
            plugins,
            snapshot_registry,
            scheduler,
            lsp_registry,
            tool_presentation,
            cancellation_token: None,
            permission_inspector: None,
            command_event_sink: None,
        }
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

    pub fn for_session_context<C: crate::ToolSessionContext + ?Sized>(
        &self,
        session_context: &C,
    ) -> Self {
        let mut scoped = self.clone();
        if let Some(root) = session_context.effective_workspace_root() {
            scoped.workspace_root = root.to_path_buf();
        }
        if !session_context.effective_permission().is_empty() {
            scoped.principal = scoped
                .principal
                .clone()
                .apply_permission_config_or_self(session_context.effective_permission());
        }
        if !session_context.permission_ceiling().is_empty() {
            scoped.principal = scoped
                .principal
                .clone()
                .apply_permission_ceiling_or_self(session_context.permission_ceiling());
        }
        if session_context.execution_access() == agena_domain::ExecutionAccess::ReadOnly {
            let allowed_tools = scoped
                .registered_tools_with_definition_overrides()
                .into_iter()
                .filter(|entry| {
                    let permissions = &entry.definition.permissions;
                    permissions.read_only
                        && !permissions.shell
                        && !permissions.interactive
                        && !crate::tool::is_tool_api_handler(entry)
                })
                .map(|entry| entry.canonical_name())
                .collect::<Vec<_>>();
            scoped.allowed_tool_names = Some(allowed_tools.into_iter().collect());
        }
        if !session_context.capability_denied_tool_names().is_empty() {
            let denied = session_context.capability_denied_tool_names();
            let allowed_tools = scoped
                .registered_tools_with_definition_overrides()
                .into_iter()
                .filter(|entry| {
                    !denied
                        .iter()
                        .any(|name| crate::tool::registered_tool_matches_name(entry, name.as_str()))
                })
                .map(|entry| entry.canonical_name())
                .collect::<std::collections::HashSet<_>>();
            match scoped.allowed_tool_names.as_mut() {
                Some(existing) => existing.retain(|name| allowed_tools.contains(name)),
                None => scoped.allowed_tool_names = Some(allowed_tools),
            }
        }
        if let Some(model_id) = session_context.selected_model() {
            scoped.model_id = Some(model_id.to_owned());
        }
        scoped
    }

    /// Attach the execution-wide cancellation signal to every tool reached
    /// through this scoped executor, including execution tools reached through
    /// `tools_call`. Cloning an executor preserves the signal.
    pub fn with_cancellation_token(
        mut self,
        cancellation_token: Option<tokio_util::sync::CancellationToken>,
    ) -> Self {
        self.cancellation_token = cancellation_token;
        self
    }

    pub fn with_permission_inspector(
        mut self,
        inspector: Option<Arc<dyn crate::tool::ExecutionPermissionInspector>>,
    ) -> Self {
        self.permission_inspector = inspector;
        self
    }

    /// Attach a runtime-owned sink for process command lifecycle/output
    /// events. The sink is intentionally optional and is preserved when the
    /// executor is scoped or cloned for a concurrent tool task.
    pub fn with_command_event_sink(
        mut self,
        sink: Option<agena_tool::ToolRuntimeEventSink>,
    ) -> Self {
        self.command_event_sink = sink;
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
            match error.kind {
                agena_plugin_host::sdk::PluginErrorKind::PolicyDenied => error
                    .diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| {
                        data.get("denial")
                            .or_else(|| data.pointer("/details/denial"))
                    })
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .map(|denial| ToolError::PolicyDenied(Box::new(denial)))
                    .unwrap_or_else(|| ToolError::from_plugin_error(error)),
                agena_plugin_host::sdk::PluginErrorKind::UserDeclined => error
                    .diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| {
                        data.get("decline")
                            .or_else(|| data.pointer("/details/decline"))
                    })
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .map(|decline| ToolError::UserDeclined(Box::new(decline)))
                    .unwrap_or_else(|| ToolError::from_plugin_error(error)),
                agena_plugin_host::sdk::PluginErrorKind::CapabilityUnavailable => error
                    .diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| data.get("unavailable"))
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .map(|unavailable| ToolError::CapabilityUnavailable(Box::new(unavailable)))
                    .unwrap_or_else(|| ToolError::from_plugin_error(error)),
                agena_plugin_host::sdk::PluginErrorKind::ToolUnavailable => error
                    .diagnostic
                    .data
                    .as_ref()
                    .and_then(|data| data.get("unavailable"))
                    .cloned()
                    .and_then(|value| serde_json::from_value(value).ok())
                    .map(|unavailable| ToolError::ToolUnavailable(Box::new(unavailable)))
                    .unwrap_or_else(|| ToolError::from_plugin_error(error)),
                _ => ToolError::from_plugin_error(error),
            }
        }
    }

    pub fn principal(&self) -> &ExecutionPrincipal {
        &self.principal
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

        // Plugin chain: tool.definition. Every ordinary execution tool follows
        // this same path, including provider-backed plugins such as agena.openai.
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
            .filter(|entry| self.is_tool_available(entry))
            .collect()
    }

    pub(crate) fn tool_is_within_capability(&self, entry: &RegisteredTool) -> bool {
        self.allowed_tool_names
            .as_ref()
            .is_none_or(|allowed| allowed.contains(entry.canonical_name().as_str()))
    }

    fn has_execution_tool_capability(&self) -> bool {
        if self.principal.blocked {
            return false;
        }
        let tool_set = self.builtin_tool_set();
        self.registered_tools_with_definition_overrides()
            .into_iter()
            .filter(|entry| tool_set.is_tool_enabled(entry))
            .filter(|entry| !crate::tool::is_tool_api_handler(entry))
            .any(|entry| self.tool_is_within_capability(&entry))
    }

    pub(crate) fn is_tool_available(&self, entry: &RegisteredTool) -> bool {
        // The five Tool API handlers form the provider protocol and carry no
        // execution authority. Every other entry is an ordinary execution tool
        // governed by the same execution principal and permission policy.
        // Permission Deny is intentionally not a catalog filter: the model
        // must be able to call a present capability and receive a structured
        // PolicyDenied result with user-rule provenance. Only hard capability
        // boundaries remove tools from the catalog.
        if crate::tool::is_tool_api_handler(entry) {
            return self.has_execution_tool_capability();
        }
        self.tool_is_within_capability(entry)
    }

    pub fn detailed_tools(&self) -> Vec<RegisteredTool> {
        self.available_registered_tools()
            .into_iter()
            .map(|entry| present_registered_tool_detailed(entry, &self.tool_presentation))
            .collect()
    }

    /// Return every ordinary execution tool visible to this session. There is
    /// no direct/deferred/hidden exposure tier; only the five Tool API handlers
    /// are excluded because they are protocol functions rather than targets.
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

    /// The only functions ever declared through an AI provider's official
    /// function/tool protocol are the five stable agena.tools gateway handlers.
    pub fn available_tool_api_bindings(&self) -> Vec<ToolApiBinding> {
        let mut tools = self
            .available_tools()
            .into_iter()
            .filter_map(ToolApiBinding::from_registered_tool)
            .collect::<Vec<_>>();
        if tools
            .iter()
            .any(|binding| binding.function() != agena_domain::ToolApiFunction::Call)
            && !tools
                .iter()
                .any(|binding| binding.function() == agena_domain::ToolApiFunction::Call)
        {
            tools.push(ToolApiBinding::call_gateway());
        }
        tools.sort_by(|left, right| left.function_name().cmp(right.function_name()));
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
            ToolError::ToolUnavailable(Box::new(agena_domain::ToolUnavailableResult {
                tool_name: requested.to_string(),
                reason: format!("tool '{requested}' is not registered"),
                suggestions,
                source: "tool_registry".to_string(),
                retryable: false,
            }))
        } else {
            unknown_tool_hint(requested, suggestions)
        }
    }
}

use super::{
    Arc, BuiltinToolSet, ExecutionPrincipal, MonitorService, Path, PathBuf, PluginHost,
    PluginToolDefinitionInput, RegisteredTool, ToolError, ToolExecutor, present_registered_tool,
    present_registered_tool_detailed, suggest_tool_names, tool_summary, unknown_tool_hint,
};
use crate::tool::ToolApiBinding;
