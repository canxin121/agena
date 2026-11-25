use agena_domain::{
    ExecutionSelection, PermissionConfig, SessionAutoCompactionConfig, SessionRelationKind,
};
use agena_storage::kernel::SessionConfiguration;

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
    pub default_subagent_profile: Option<String>,
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
            profiles: agena_runtime_config::builtin_agent_profiles(),
            default_profile: Some(agena_runtime_config::MAIN_AGENT_PROFILE.into()),
            default_subagent_profile: Some(agena_runtime_config::DEFAULT_SUBAGENT_PROFILE.into()),
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
        name.map(|name| {
            agena_runtime_config::resolve_agent_profile(&self.profiles, name)?
                .with_permission_profile(&self.access_control)
        })
        .transpose()
    }

    pub fn effective_profile_name<'a>(
        &self,
        profile: Option<&'a str>,
        relation: SessionRelationKind,
    ) -> &'a str {
        profile.unwrap_or(if relation.is_subagent() {
            agena_runtime_config::DEFAULT_SUBAGENT_PROFILE
        } else {
            agena_runtime_config::MAIN_AGENT_PROFILE
        })
    }

    pub fn resolve_session_profile(
        &self,
        profile: Option<&str>,
        relation: SessionRelationKind,
    ) -> Result<agena_runtime_config::ResolvedAgentProfile, String> {
        let name = self.effective_profile_name(profile, relation);
        agena_runtime_config::resolve_agent_profile(&self.profiles, name)?
            .with_permission_profile(&self.access_control)
    }

    pub(crate) fn visible_subagent_profiles(
        &self,
    ) -> impl Iterator<Item = (&str, agena_runtime_config::ResolvedAgentProfile)> {
        self.profiles.keys().filter_map(|name| {
            let profile = self
                .validate_session_profile(Some(name), SessionRelationKind::Subagent)
                .ok()?;
            (!profile.hidden).then_some((name.as_str(), profile))
        })
    }

    /// Mode controls new explicit selections, not already saved identities.
    /// Forks and restored sessions retain their selected role after branching.
    pub fn validate_session_profile(
        &self,
        profile: Option<&str>,
        relation: SessionRelationKind,
    ) -> Result<agena_runtime_config::ResolvedAgentProfile, String> {
        let name = self.effective_profile_name(profile, relation);
        let resolved = self.resolve_session_profile(profile, relation)?;
        let allowed = if relation.is_subagent() {
            resolved.mode.allows_subagent()
        } else {
            resolved.mode.allows_primary()
        };
        if !allowed {
            return Err(format!(
                "agent profile `{name}` cannot be used for a {} session",
                if relation.is_subagent() {
                    "subagent"
                } else {
                    "primary"
                },
            ));
        }
        Ok(resolved)
    }

    pub fn inherited_permission(
        &self,
        configuration: &SessionConfiguration,
    ) -> Result<PermissionConfig, String> {
        let profile = self.resolve_session_profile(
            configuration.profile.as_deref(),
            configuration.relation_kind,
        )?;
        Ok(profile.effective_permission(&self.permission))
    }

    pub fn effective_permission(
        &self,
        configuration: &SessionConfiguration,
    ) -> Result<PermissionConfig, String> {
        let profile = match configuration.permission_profile {
            Some(_) => Default::default(),
            None => self.resolve_session_profile(
                configuration.profile.as_deref(),
                configuration.relation_kind,
            )?,
        };
        Ok(self.permission_with_profile(configuration, &profile))
    }

    pub(crate) fn permission_with_profile(
        &self,
        configuration: &SessionConfiguration,
        profile: &agena_runtime_config::ResolvedAgentProfile,
    ) -> PermissionConfig {
        match configuration.permission_profile.as_deref() {
            // A stored reference may outlive a catalog entry. Keep it visible
            // and deny by default until the operator selects a new policy.
            Some(id) => self
                .access_control
                .standalone_permission(id)
                .unwrap_or_else(|_| PermissionConfig::default().standalone()),
            None => profile
                .effective_permission(&self.permission)
                .merged_with(&configuration.permission),
        }
    }

    pub fn session_selection(
        &self,
        configuration: &SessionConfiguration,
        selected: &ExecutionSelection,
    ) -> Result<ExecutionSelection, String> {
        let profile = self.resolve_session_profile(
            configuration.profile.as_deref(),
            configuration.relation_kind,
        )?;
        Ok(self.selection_with_profile(configuration, selected, &profile))
    }

    pub(crate) fn selection_with_profile(
        &self,
        configuration: &SessionConfiguration,
        selected: &ExecutionSelection,
        profile: &agena_runtime_config::ResolvedAgentProfile,
    ) -> ExecutionSelection {
        self.default_selection
            .overlay_with_cascade(&configuration.selection_defaults)
            .overlay_with_cascade(&profile.selection)
            .overlay_with_cascade(selected)
    }
}
