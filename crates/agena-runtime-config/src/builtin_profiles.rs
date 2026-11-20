//! Built-in roles share the resolved catalog and execution path of user profiles.
use std::collections::BTreeMap;

use agena_domain::{PathAccessModes, PathPermissionConfig, PathPermissionMode, PermissionConfig};
use agena_prompt_layers::{FragmentId, LayerMode, PromptFragment, PromptLayer};

use crate::{AgentProfileConfig, AgentProfileMode};

pub const MAIN_AGENT_PROFILE: &str = "main";
pub const DEFAULT_SUBAGENT_PROFILE: &str = "worker";

fn role(
    name: &str,
    title: &str,
    description: &str,
    mode: AgentProfileMode,
    instructions: &str,
) -> AgentProfileConfig {
    let id =
        FragmentId::parse(&format!("agena-profile-{name}")).expect("valid built-in fragment ID");
    AgentProfileConfig {
        title: title.into(),
        description: description.into(),
        mode: Some(mode),
        prompts: PromptLayer {
            order: vec![id.clone()],
            inline: vec![PromptFragment::new(id, instructions.trim())],
            ..Default::default()
        },
        ..Default::default()
    }
}

fn readonly() -> PermissionConfig {
    PermissionConfig {
        path: Some(PathPermissionConfig {
            workspace: Some(PathAccessModes::from_mode(PathPermissionMode::Readonly)),
            external: Some(PathAccessModes::from_mode(PathPermissionMode::Readonly)),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub fn builtin_agent_profiles() -> BTreeMap<String, AgentProfileConfig> {
    let main = role(
        MAIN_AGENT_PROFILE,
        "Main",
        "Own the user's goal, coordinate specialist roles, implement and verify work, and deliver an integrated result.",
        AgentProfileMode::Primary,
        include_str!("profile_prompts/main.md"),
    );
    let worker = role(
        DEFAULT_SUBAGENT_PROFILE,
        "Worker",
        "Complete a bounded implementation, fix, or other substantive task. Give each worker clear ownership and acceptance criteria.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/worker.md"),
    );
    let mut explore = role(
        "explore",
        "Explore",
        "Read-only exploration: locate code, trace behavior, research a focused question, and return evidence before implementation.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/explore.md"),
    );
    explore.permission = readonly();
    explore.max_turns = Some(40);
    let mut reviewer = role(
        "reviewer",
        "Reviewer",
        "Independently review changes for correctness, regressions, security and test gaps; return prioritized, evidence-backed findings.",
        AgentProfileMode::All,
        include_str!("profile_prompts/reviewer.md"),
    );
    reviewer.permission = readonly();
    reviewer.max_turns = Some(40);
    BTreeMap::from([
        (MAIN_AGENT_PROFILE.into(), main),
        (DEFAULT_SUBAGENT_PROFILE.into(), worker),
        ("explore".into(), explore),
        ("reviewer".into(), reviewer),
    ])
}

/// Keep factory behavior when users customize only a model, limit or label.
/// User file layers have already composed by name before this final defaults pass.
pub(crate) fn compose_profile_overrides(
    builtins: &BTreeMap<String, AgentProfileConfig>,
    overrides: BTreeMap<String, AgentProfileConfig>,
) -> Result<BTreeMap<String, AgentProfileConfig>, crate::ConfigError> {
    let mut profiles = builtins.clone();
    for (name, overlay) in overrides {
        overlay.prompts.validate().map_err(|error| {
            crate::ConfigError::Validation(format!("profiles.{name}.prompts: {error}"))
        })?;
        if let Some(base) = profiles.get_mut(&name) {
            if !overlay.title.is_empty() {
                base.title = overlay.title;
            }
            if !overlay.description.is_empty() {
                base.description = overlay.description;
            }
            if overlay.mode.is_some() {
                base.mode = overlay.mode;
            }
            if overlay.hidden.is_some() {
                base.hidden = overlay.hidden;
            }
            if overlay.extends.is_some() {
                base.extends = overlay.extends;
            }
            if overlay.model.is_some() {
                base.model = overlay.model;
            }
            if overlay.max_turns.is_some() {
                base.max_turns = overlay.max_turns;
            }
            if overlay.permission_profile.is_some() {
                base.permission_profile = overlay.permission_profile;
                base.permission = overlay.permission;
            } else {
                base.permission.merge_from(overlay.permission);
            }
            if overlay.prompts.mode == LayerMode::Replace {
                base.prompts = overlay.prompts;
            } else {
                let layer = overlay.prompts;
                base.prompts.order.retain(|id| !layer.disabled.contains(id));
                base.prompts
                    .inline
                    .retain(|item| !layer.disabled.contains(&item.id));
                for id in layer.disabled {
                    if !base.prompts.disabled.contains(&id) {
                        base.prompts.disabled.push(id);
                    }
                }
                for id in layer.order {
                    base.prompts.order.retain(|old| old != &id);
                    base.prompts.order.push(id);
                }
                for fragment in layer.inline {
                    base.prompts.inline.retain(|old| old.id != fragment.id);
                    base.prompts.inline.push(fragment);
                }
            }
        } else {
            profiles.insert(name, overlay);
        }
    }
    Ok(profiles)
}
