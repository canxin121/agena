use agena_domain::{ExecutionSelection, PermissionConfig, SessionAutoCompactionConfig};

/// Configuration consumed by the concrete session manager.
#[derive(Debug, Clone)]
/// Configuration of the runtime session manager.
pub struct RuntimeSessionManagerConfig {
    pub default_selection: ExecutionSelection,
    pub auxiliary_selection: ExecutionSelection,
    /// Configuration prompt layers plus fragment directories.
    pub prompts: agena_runtime_config::PromptLayersConfig,
    pub profiles: std::collections::BTreeMap<String, agena_runtime_config::AgentProfileConfig>,
    pub default_profile: Option<String>,
    pub auto_title: agena_runtime_config::SessionAutoTitleConfig,
    pub permission: PermissionConfig,
    pub access_control: agena_domain::AccessControlConfig,
    /// Resolved settings flag used for configuration validation, independent
    /// of trusted invocation bypasses and task-local execution scopes.
    pub sandbox_enabled: bool,
    pub auto_compaction: SessionAutoCompactionConfig,
    pub max_concurrent_tools: usize,
    /// Cap on model turns within one stable run. `None` falls back to
    /// `DEFAULT_MAX_MODEL_TURNS` (500) in the supervised kernel run.
    pub max_turns: Option<usize>,
}

impl Default for RuntimeSessionManagerConfig {
    fn default() -> Self {
        Self {
            default_selection: Default::default(),
            auxiliary_selection: Default::default(),
            prompts: Default::default(),
            profiles: Default::default(),
            default_profile: None,
            auto_title: Default::default(),
            permission: Default::default(),
            access_control: Default::default(),
            sandbox_enabled: true,
            auto_compaction: Default::default(),
            max_concurrent_tools: 32,
            // Same default as `DEFAULT_MAX_MODEL_TURNS` in
            // the supervised kernel run (mirrors gemini's MAX_TURNS).
            max_turns: Some(500),
        }
    }
}

impl RuntimeSessionManagerConfig {
    pub fn resolve_profile(
        &self,
        name: Option<&str>,
    ) -> Result<Option<agena_runtime_config::ResolvedAgentProfile>, String> {
        name.map(|name| agena_runtime_config::resolve_agent_profile(&self.profiles, name))
            .transpose()
    }

    pub fn inherited_permission(&self, profile: Option<&str>) -> Result<PermissionConfig, String> {
        Ok(self.resolve_profile(profile)?.map_or_else(
            || self.permission.clone(),
            |profile| self.permission.merged_with(&profile.permission),
        ))
    }

    pub fn session_selection(
        &self,
        profile: Option<&str>,
        selected: &ExecutionSelection,
    ) -> Result<ExecutionSelection, String> {
        let inherited = self.resolve_profile(profile)?.map_or_else(
            || self.default_selection.clone(),
            |profile| {
                self.default_selection
                    .overlay_with_cascade(&profile.selection)
            },
        );
        Ok(inherited.overlay_with_cascade(selected))
    }
}
