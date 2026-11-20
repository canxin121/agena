//! Reusable, named defaults for one session's execution.
//!
//! Profiles compose the existing model, permission and prompt contracts. They
//! never replace the instance's identity policy or a session permission ceiling.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use agena_domain::{
    AccessControlConfig, ExecutionSelection, ModelSelectionConfig, PermissionConfig,
};
use agena_prompt_layers::{FragmentId, PromptLayer, PromptScope};
use serde::{Deserialize, Serialize};

use crate::{ConfigError, PromptLayersConfig, ResolvedConfig};

pub const MAX_AGENT_PROFILES: usize = 256;
pub const MAX_AGENT_PROFILE_DEPTH: usize = 64;

/// Where a role may be selected. Existing custom profiles support both scopes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProfileMode {
    Primary,
    Subagent,
    #[default]
    All,
}

impl AgentProfileMode {
    pub fn allows_primary(self) -> bool {
        matches!(self, Self::Primary | Self::All)
    }

    pub fn allows_subagent(self) -> bool {
        matches!(self, Self::Subagent | Self::All)
    }
}

/// A file-backed profile. Workspace entries replace global entries with the
/// same name; use `extends` to explicitly share settings with another profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AgentProfileConfig {
    #[serde(skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<AgentProfileMode>,
    /// Hide from model-facing delegation discovery while keeping explicit use.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<ModelSelectionConfig>,
    #[serde(skip_serializing_if = "PromptLayer::is_empty")]
    pub prompts: PromptLayer,
    #[serde(skip_serializing_if = "PermissionConfig::is_empty")]
    pub permission: PermissionConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission_profile: Option<String>,
    /// `0` means unlimited, matching `session.max_turns`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_turns: Option<usize>,
}

/// A profile after resolving its single-parent inheritance chain. Prompt
/// layers remain separate so append, replace and suppression retain their
/// normal meaning, including across a chain of profiles.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResolvedAgentProfile {
    pub title: String,
    pub description: String,
    pub mode: AgentProfileMode,
    pub hidden: bool,
    pub prompts: Vec<PromptLayer>,
    pub permission: PermissionConfig,
    pub permission_profile: Option<String>,
    pub selection: ExecutionSelection,
    pub max_turns: Option<usize>,
}

impl ResolvedAgentProfile {
    pub fn with_permission_profile(
        mut self,
        catalog: &AccessControlConfig,
    ) -> Result<Self, String> {
        if let Some(id) = &self.permission_profile {
            let profile = catalog
                .profiles
                .get(id)
                .filter(|profile| profile.complete)
                .ok_or_else(|| {
                    format!("permission configuration `{id}` is missing or is not complete")
                })?;
            self.permission = profile
                .permission
                .standalone()
                .merged_with(&self.permission);
        }
        Ok(self)
    }

    pub fn effective_permission(&self, inherited: &PermissionConfig) -> PermissionConfig {
        if self.permission_profile.is_some() {
            self.permission.clone()
        } else {
            inherited.merged_with(&self.permission)
        }
    }
}

pub fn validate_agent_profile_name(name: &str) -> Result<(), String> {
    FragmentId::parse(name)
        .map(|_| ())
        .map_err(|error| format!("invalid agent profile name `{name}`: {error}"))
}

/// Resolve one profile without changing the source catalog. Unknown names,
/// missing parents and cycles are errors, including after a settings reload.
pub fn resolve_agent_profile(
    profiles: &BTreeMap<String, AgentProfileConfig>,
    name: &str,
) -> Result<ResolvedAgentProfile, String> {
    validate_agent_profile_name(name)?;
    let mut chain = Vec::new();
    let mut visited = BTreeSet::new();
    let mut current = name;
    loop {
        if !visited.insert(current) {
            return Err(format!(
                "agent profile `{name}` has an inheritance cycle at `{current}`"
            ));
        }
        if chain.len() >= MAX_AGENT_PROFILE_DEPTH {
            return Err(format!(
                "agent profile `{name}` exceeds the inheritance depth limit of {MAX_AGENT_PROFILE_DEPTH}"
            ));
        }
        let profile = profiles
            .get(current)
            .ok_or_else(|| format!("unknown agent profile `{current}` while resolving `{name}`"))?;
        chain.push(profile);
        match profile.extends.as_deref() {
            Some(parent) => {
                validate_agent_profile_name(parent)?;
                current = parent;
            }
            None => break,
        }
    }

    let mut resolved = ResolvedAgentProfile::default();
    for profile in chain.into_iter().rev() {
        if !profile.title.is_empty() {
            resolved.title.clone_from(&profile.title);
        }
        if !profile.description.is_empty() {
            resolved.description.clone_from(&profile.description);
        }
        if let Some(mode) = profile.mode {
            resolved.mode = mode;
        }
        if let Some(hidden) = profile.hidden {
            resolved.hidden = hidden;
        }
        if !profile.prompts.is_empty() {
            resolved.prompts.push(profile.prompts.clone());
        }
        if let Some(id) = &profile.permission_profile {
            resolved.permission_profile = Some(id.clone());
            resolved.permission = PermissionConfig::default();
        }
        resolved.permission.merge_from(profile.permission.clone());
        if let Some(model) = &profile.model {
            let mut selection = ExecutionSelection {
                provider: model.provider.clone(),
                adapter: model.adapter.clone(),
                model: model.model.clone(),
                thinking_mode: model.thinking_mode.clone(),
                speed_mode: model.speed_mode.clone(),
                verbosity: model.verbosity.clone(),
                parallel_tool_calls: model.parallel_tool_calls,
                ..Default::default()
            };
            selection.normalize();
            resolved.selection = resolved.selection.overlay_with_cascade(&selection);
        }
        if profile.max_turns.is_some() {
            resolved.max_turns = profile.max_turns;
        }
    }
    Ok(resolved)
}

/// Structural validation runs for every entry, including unselected profiles.
pub(crate) fn validate_agent_profiles(config: &ResolvedConfig) -> Result<(), ConfigError> {
    if config.profiles.len() > MAX_AGENT_PROFILES {
        return Err(ConfigError::Validation(format!(
            "profiles exceeds the limit of {MAX_AGENT_PROFILES} entries"
        )));
    }
    for (name, profile) in &config.profiles {
        validate_agent_profile_name(name).map_err(ConfigError::Validation)?;
        profile.prompts.validate().map_err(|error| {
            ConfigError::Validation(format!("profiles.{name}.prompts: {error}"))
        })?;
        agena_runtime_contracts::authorization::validate_permission_config(&profile.permission)
            .map_err(|error| {
                ConfigError::Validation(format!("profiles.{name}.permission: {error}"))
            })?;
        if profile.permission_profile.is_some() && !profile.permission.is_empty() {
            return Err(ConfigError::Validation(format!(
                "profiles.{name}: use either a complete permission configuration or inline permissions"
            )));
        }
        let resolved = resolve_agent_profile(&config.profiles, name)
            .and_then(|profile| profile.with_permission_profile(&config.access_control))
            .map_err(ConfigError::Validation)?;
        let selection = config
            .default_selection
            .overlay_with_cascade(&resolved.selection);
        // Variant-only defaults also work when the provider catalog chooses
        // the route at run time instead of an explicit instance selection.
        if !resolved.selection.is_empty()
            && (selection.provider.is_some()
                || selection.adapter.is_some()
                || selection.model.is_some())
        {
            let model = ModelSelectionConfig {
                provider: selection.provider,
                adapter: selection.adapter,
                model: selection.model,
                thinking_mode: selection.thinking_mode,
                speed_mode: selection.speed_mode,
                verbosity: selection.verbosity,
                parallel_tool_calls: selection.parallel_tool_calls,
            };
            crate::config::raw::resolve_model_selection(
                &format!("profiles.{name}.model"),
                Some(&model),
                &config.providers,
            )?;
        }
    }
    for (field, name, primary) in [
        ("profiles.main", Some(crate::MAIN_AGENT_PROFILE), true),
        (
            "profiles.worker",
            Some(crate::DEFAULT_SUBAGENT_PROFILE),
            false,
        ),
        ("default_profile", config.default_profile.as_deref(), true),
        (
            "default_subagent_profile",
            config.default_subagent_profile.as_deref(),
            false,
        ),
    ] {
        if let Some(name) = name {
            let profile = resolve_agent_profile(&config.profiles, name)
                .map_err(|error| ConfigError::Validation(format!("{field}: {error}")))?;
            if (primary && !profile.mode.allows_primary())
                || (!primary && !profile.mode.allows_subagent())
            {
                return Err(ConfigError::Validation(format!(
                    "{field}: profile `{name}` does not support {} use",
                    if primary { "primary" } else { "subagent" }
                )));
            }
        }
    }
    Ok(())
}

/// Validate the exact permission overlay and libraries used by a session in
/// this scope. Settings edits and startup call the same validator.
pub(crate) fn validate_agent_profiles_in_workspace(
    config: &ResolvedConfig,
    workspace: &Path,
) -> Result<(), ConfigError> {
    if config.profiles.is_empty() {
        return Ok(());
    }
    for name in config.profiles.keys() {
        let profile = resolve_agent_profile(&config.profiles, name)
            .and_then(|profile| profile.with_permission_profile(&config.access_control))
            .map_err(ConfigError::Validation)?;
        crate::config::raw::validate_permission_config_in_workspace(
            &profile.effective_permission(&config.permission),
            workspace,
            config.sandbox,
        )
        .map_err(|error| ConfigError::Validation(format!("profiles.{name}.permission: {error}")))?;
    }
    check_agent_profile_libraries(&config.prompts, &config.profiles)
}

/// Check references in inheritance order, making each ancestor's inline
/// fragments available to its descendants, just like broader prompt scopes.
pub fn check_agent_profile_libraries(
    prompts: &PromptLayersConfig,
    profiles: &BTreeMap<String, AgentProfileConfig>,
) -> Result<(), ConfigError> {
    if profiles.is_empty() {
        return Ok(());
    }
    let mut available = prompts.library(PromptScope::Global)?;
    for fragment in &prompts.global.inline {
        available.insert(fragment.id.clone(), fragment.clone());
    }
    available.extend(prompts.library(PromptScope::Workspace)?);
    for fragment in &prompts.workspace.inline {
        available.insert(fragment.id.clone(), fragment.clone());
    }
    for name in profiles.keys() {
        let profile = resolve_agent_profile(profiles, name).map_err(ConfigError::Validation)?;
        let mut library = available.clone();
        for layer in &profile.prompts {
            for fragment in &layer.inline {
                library.insert(fragment.id.clone(), fragment.clone());
            }
            for id in &layer.order {
                if !library.contains_key(id) {
                    return Err(ConfigError::Validation(format!(
                        "profiles.{name}.prompts references `{id}`, but no saved, file-backed or inline fragment provides it"
                    )));
                }
            }
        }
    }
    Ok(())
}
