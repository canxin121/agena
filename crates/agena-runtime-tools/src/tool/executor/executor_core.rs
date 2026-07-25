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
            monitor_registry: crate::default_monitor_registry(),
            truncator: ToolOutputTruncator::default(),
            plugins,
            snapshot_registry,
            scheduler,
            lsp_registry,
            permission_mode: PermissionEnforcementMode::Enforced,
            tool_presentation,
            cancellation_token: None,
            permission_inspector: None,
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

    pub fn for_session_context<C: crate::ToolSessionContext + ?Sized>(
        &self,
        session_context: &C,
    ) -> Self {
        let mut scoped = self.clone();
        if let Some(root) = session_context.effective_workspace_root() {
            scoped.workspace_root = root.to_path_buf();
        }
        if !session_context.effective_permission().is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .apply_permission_config_or_self(session_context.effective_permission());
        }
        if !session_context.permission_ceiling().is_empty() {
            scoped.agent = scoped
                .agent
                .clone()
                .apply_permission_ceiling_or_self(session_context.permission_ceiling());
        }
        if !session_context.allowed_tools().is_empty() {
            scoped.agent = scoped.agent.clone().restricted_to_allowed_tools(
                session_context.allowed_tools().iter().map(String::as_str),
            );
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
            .filter(|tool| crate::tool::tool_exposure(tool) != crate::tool::ToolExposure::Hidden)
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
        tools.sort_by(|left, right| left.function_name().cmp(right.function_name()));
        tools
    }

    /// Provider-facing hybrid plan. Prompt-envelope models retain the stable
    /// five-function gateway; provider-protocol models additionally receive
    /// high-frequency direct tools while deferred tools remain discoverable
    /// through that gateway.
    pub fn available_model_tool_bindings(
        &self,
        provider_protocol: bool,
        direct_policy: &agena_provider::AgenaDirectToolsConfig,
    ) -> Vec<ToolApiBinding> {
        let mut bindings = self.available_tool_api_bindings();
        if !provider_protocol {
            return bindings;
        }
        let tools = self.available_execution_tools();
        let names = crate::tool::execution_tool_names(&tools);
        let mut direct_tools = tools
            .into_iter()
            .zip(names)
            .filter(|(tool, _)| {
                crate::tool::tool_exposure(tool.registered()) == crate::tool::ToolExposure::Direct
                    && direct_policy.permits(tool.canonical_name().as_str())
            })
            .map(|(tool, execution_name)| {
                let canonical_name = tool.canonical_name();
                (tool, execution_name, canonical_name)
            })
            .collect::<Vec<_>>();
        direct_tools.sort_by(|left, right| left.2.cmp(&right.2));

        let reserved = bindings
            .iter()
            .map(|binding| binding.function_name().to_owned())
            .collect::<Vec<_>>();
        let identities = direct_tools
            .iter()
            .map(|(_, execution_name, canonical_name)| {
                (execution_name.clone(), canonical_name.clone())
            })
            .collect::<Vec<_>>();
        let provider_names = unique_direct_provider_function_names(&identities, &reserved);
        let direct = direct_tools
            .into_iter()
            .zip(provider_names)
            .map(|((tool, execution_name, _), provider_name)| {
                ToolApiBinding::from_direct_tool(
                    tool.into_registered(),
                    provider_name,
                    execution_name,
                )
            })
            .collect::<Vec<_>>();
        let mut direct = enforce_direct_tool_budget(direct, direct_policy);
        direct.sort_by(|left, right| left.function_name().cmp(right.function_name()));
        bindings.extend(direct);
        bindings
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

/// Apply the route-local Direct surface budget after provider names and
/// schemas have been finalized. Candidate ordering is canonical before this
/// point, so a fixed config produces stable declarations and prompt caches.
fn enforce_direct_tool_budget(
    direct: Vec<ToolApiBinding>,
    policy: &agena_provider::AgenaDirectToolsConfig,
) -> Vec<ToolApiBinding> {
    let max_count = policy.max_tools.map(usize::from).unwrap_or(usize::MAX);
    let max_schema_tokens = policy.max_schema_tokens.map(u64::from).unwrap_or(u64::MAX);
    let mut used_schema_tokens = 0_u64;
    let mut retained = Vec::new();
    for binding in direct {
        if retained.len() >= max_count {
            break;
        }
        let serialized = serde_json::to_string(&binding.definition()).unwrap_or_default();
        let estimated_tokens = ((serialized.chars().count() as u64).saturating_add(3)) / 4;
        if used_schema_tokens.saturating_add(estimated_tokens) > max_schema_tokens {
            continue;
        }
        used_schema_tokens = used_schema_tokens.saturating_add(estimated_tokens);
        retained.push(binding);
    }
    retained
}

fn direct_provider_function_name(execution_name: &str) -> String {
    execution_name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '_' || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

/// Allocate deterministic provider-protocol function names without dropping
/// tools when compact names sanitize to the same value. Prefer the readable
/// execution name, fall back to the full canonical registry key, and finally
/// add a stable content-derived suffix when either form is ambiguous or too
/// long for common provider limits.
fn unique_direct_provider_function_names(
    identities: &[(String, String)],
    reserved: &[String],
) -> Vec<String> {
    const MAX_PROVIDER_NAME_LEN: usize = 64;

    let mut short_counts = std::collections::HashMap::<String, usize>::new();
    let mut full_counts = std::collections::HashMap::<String, usize>::new();
    for (execution_name, canonical_name) in identities {
        *short_counts
            .entry(direct_provider_function_name(execution_name))
            .or_default() += 1;
        *full_counts
            .entry(direct_provider_function_name(canonical_name))
            .or_default() += 1;
    }

    let mut used = reserved
        .iter()
        .cloned()
        .collect::<std::collections::HashSet<_>>();
    identities
        .iter()
        .map(|(execution_name, canonical_name)| {
            let short = direct_provider_function_name(execution_name);
            if short.len() <= MAX_PROVIDER_NAME_LEN
                && short_counts.get(&short).copied() == Some(1)
                && used.insert(short.clone())
            {
                return short;
            }

            let full = direct_provider_function_name(canonical_name);
            if full.len() <= MAX_PROVIDER_NAME_LEN
                && full_counts.get(&full).copied() == Some(1)
                && used.insert(full.clone())
            {
                return full;
            }

            use sha2::{Digest, Sha256};
            let digest = Sha256::digest(canonical_name.as_bytes())
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect::<String>();
            for suffix_len in [8_usize, 12, 16, 24, 32, 64] {
                let suffix = &digest[..suffix_len];
                let prefix_len = MAX_PROVIDER_NAME_LEN.saturating_sub(suffix.len() + 1);
                let prefix = full.chars().take(prefix_len).collect::<String>();
                let candidate = format!("{prefix}_{suffix}");
                if used.insert(candidate.clone()) {
                    return candidate;
                }
            }
            unreachable!("SHA-256 suffix exhausted while allocating provider tool names")
        })
        .collect()
}

#[cfg(test)]
mod direct_provider_name_tests {
    use agena_plugin_host::registry::RegisteredTool;
    use agena_plugin_sdk::{PluginKey, ToolDefinition};

    use super::{
        ToolApiBinding, enforce_direct_tool_budget, unique_direct_provider_function_names,
    };

    fn direct_binding(name: &str, description: &str) -> ToolApiBinding {
        let definition: ToolDefinition = serde_json::from_value(serde_json::json!({
            "name": name,
            "docs": { "summary": description },
            "contract": { "input_schema": { "type": "object" } }
        }))
        .expect("tool definition");
        let tool = RegisteredTool::new(
            PluginKey::new("agena", "fs").expect("plugin key"),
            definition,
        )
        .expect("registered tool");
        ToolApiBinding::from_direct_tool(tool, format!("fs_{name}"), format!("fs.{name}"))
    }

    #[test]
    fn colliding_sanitized_names_are_kept_and_stable() {
        let identities = vec![
            ("a_b.c".to_string(), "agena.a_b.c".to_string()),
            ("a.b_c".to_string(), "agena.a.b_c".to_string()),
        ];

        let first = unique_direct_provider_function_names(&identities, &[]);
        let second = unique_direct_provider_function_names(&identities, &[]);

        assert_eq!(first, second);
        assert_eq!(first.len(), 2);
        assert_ne!(first[0], first[1]);
        assert!(first.iter().all(|name| name.len() <= 64));
    }

    #[test]
    fn gateway_and_long_name_collisions_receive_hash_suffixes() {
        let identities = vec![
            (
                "tools.list".to_string(),
                "agena.tools_list.list".to_string(),
            ),
            (
                format!("plugin.{}", "x".repeat(90)),
                format!("agena.plugin.{}", "x".repeat(90)),
            ),
        ];

        let names = unique_direct_provider_function_names(&identities, &["tools_list".to_string()]);

        assert_eq!(names.len(), 2);
        assert_ne!(names[0], "tools_list");
        assert!(names.iter().all(|name| name.len() <= 64));
    }

    #[test]
    fn direct_budget_caps_count_and_can_disable_only_the_direct_surface() {
        let direct = vec![
            direct_binding("alpha", "first direct tool"),
            direct_binding("beta", "second direct tool"),
        ];
        let count_limited = enforce_direct_tool_budget(
            direct.clone(),
            &agena_provider::AgenaDirectToolsConfig {
                max_tools: Some(1),
                ..Default::default()
            },
        );
        assert_eq!(count_limited.len(), 1);
        assert_eq!(count_limited[0].function_name(), "fs_alpha");

        let gateway_only = enforce_direct_tool_budget(
            direct,
            &agena_provider::AgenaDirectToolsConfig {
                max_schema_tokens: Some(0),
                ..Default::default()
            },
        );
        assert!(gateway_only.is_empty());
    }
}
use super::{
    Agent, Arc, BuiltinToolSet, MonitorService, Path, PathBuf, PermissionDecision,
    PermissionEnforcementMode, PluginHost, PluginToolDefinitionInput, RegisteredTool, ToolError,
    ToolExecutor, ToolOutputTruncator, present_registered_tool, present_registered_tool_detailed,
    suggest_tool_names, tool_summary, unknown_tool_hint,
};
use crate::tool::ToolApiBinding;
