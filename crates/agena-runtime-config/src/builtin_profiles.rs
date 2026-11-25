//! Built-in roles share the resolved catalog and execution path of user profiles.
use std::collections::BTreeMap;

use agena_domain::{
    PathAccessModes, PathPermissionConfig, PathPermissionMode, PermissionConfig, PermissionMode,
    SHELL_COMMAND_RULE_GROUP, TOOL_CLASS_READ_ONLY, ToolPermissionConfig, ToolPermissionRules,
};
use agena_prompt_layers::{FragmentId, LayerMode, PromptFragment, PromptLayer};
use agena_runtime_contracts::identity::{
    IMPLEMENTATION_PROMPT_ID, INVESTIGATION_PROMPT_ID, PLANNING_PROMPT_ID, READ_ONLY_PROMPT_ID,
    VERIFICATION_PROMPT_ID,
};

use crate::{AgentProfileConfig, AgentProfileMode, ProjectInstructionsMode};

pub const MAIN_AGENT_PROFILE: &str = "main";
pub const DEFAULT_SUBAGENT_PROFILE: &str = "worker";

fn role(
    name: &str,
    title: &str,
    description: &str,
    mode: AgentProfileMode,
    instructions: &str,
    task_prompts: &[&str],
) -> AgentProfileConfig {
    let id =
        FragmentId::parse(&format!("agena-profile-{name}")).expect("valid built-in fragment ID");
    AgentProfileConfig {
        title: title.into(),
        description: description.into(),
        mode: Some(mode),
        prompts: PromptLayer {
            order: std::iter::once(id.clone())
                .chain(
                    std::iter::once(INVESTIGATION_PROMPT_ID)
                        .chain(task_prompts.iter().copied())
                        .map(|id| FragmentId::parse(id).expect("valid reusable prompt ID")),
                )
                .collect(),
            inline: vec![PromptFragment {
                title: format!("{title} role"),
                ..PromptFragment::new(id, instructions.trim())
            }],
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

pub(crate) fn read_only_agent_permission() -> PermissionConfig {
    PermissionConfig {
        tools: Some(ToolPermissionConfig {
            default: Some(PermissionMode::Deny),
            rules: BTreeMap::from([
                (
                    "*".into(),
                    ToolPermissionRules::Ordered(
                        [(TOOL_CLASS_READ_ONLY.into(), PermissionMode::Allow)]
                            .into_iter()
                            .collect(),
                    ),
                ),
                (
                    SHELL_COMMAND_RULE_GROUP.into(),
                    ToolPermissionRules::Ordered(
                        [("*".into(), PermissionMode::Deny)].into_iter().collect(),
                    ),
                ),
            ]),
            ..Default::default()
        }),
        ..readonly()
    }
}

pub fn builtin_agent_profiles() -> BTreeMap<String, AgentProfileConfig> {
    let main = role(
        MAIN_AGENT_PROFILE,
        "Main",
        "Own the user's goal, coordinate specialist roles, implement and verify work, and deliver an integrated result.",
        AgentProfileMode::Primary,
        include_str!("profile_prompts/main.md"),
        &[
            PLANNING_PROMPT_ID,
            IMPLEMENTATION_PROMPT_ID,
            VERIFICATION_PROMPT_ID,
        ],
    );
    let worker = role(
        DEFAULT_SUBAGENT_PROFILE,
        "Worker",
        "Complete a bounded implementation, fix, or other substantive task. Give each worker clear ownership and acceptance criteria.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/worker.md"),
        &[IMPLEMENTATION_PROMPT_ID, VERIFICATION_PROMPT_ID],
    );
    let mut explore = role(
        "explore",
        "Explore",
        "Read-only exploration: locate code, trace behavior, research a focused question, and return evidence before implementation.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/explore.md"),
        &[READ_ONLY_PROMPT_ID],
    );
    explore.permission = readonly();
    explore.read_only = Some(true);
    explore.context.project_instructions = Some(ProjectInstructionsMode::OnDemand);
    explore.max_turns = Some(40);
    let mut reviewer = role(
        "reviewer",
        "Reviewer",
        "Independently review changes for correctness, regressions, security and test gaps; return prioritized, evidence-backed findings.",
        AgentProfileMode::All,
        include_str!("profile_prompts/reviewer.md"),
        &[READ_ONLY_PROMPT_ID],
    );
    reviewer.permission = readonly();
    reviewer.max_turns = Some(40);
    let mut planner = role(
        "planner",
        "Planner",
        "Plan a change when the approach involves meaningful tradeoffs, multiple components, or unclear acceptance criteria. Return a grounded implementation sequence and checks.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/planner.md"),
        &[PLANNING_PROMPT_ID, READ_ONLY_PROMPT_ID],
    );
    planner.permission = read_only_agent_permission();
    planner.read_only = Some(true);
    planner.context.project_instructions = Some(ProjectInstructionsMode::OnDemand);
    planner.max_turns = Some(40);
    let mut verifier = role(
        "verifier",
        "Verifier",
        "Run independent acceptance checks on an existing change. Return exact commands, outcomes, and evidence for tests, builds, or other requested checks.",
        AgentProfileMode::Subagent,
        include_str!("profile_prompts/verifier.md"),
        &[VERIFICATION_PROMPT_ID],
    );
    verifier.permission = PermissionConfig {
        tools: Some(ToolPermissionConfig {
            default: Some(PermissionMode::Deny),
            names: [
                ("agena.shell.exec", PermissionMode::Allow),
                ("agena.shell.spawn", PermissionMode::Allow),
                ("agena.shell.list", PermissionMode::Allow),
                ("agena.shell.read", PermissionMode::Allow),
                ("agena.shell.stop", PermissionMode::Allow),
                ("agena.shell.open", PermissionMode::Deny),
                ("agena.shell.watch", PermissionMode::Deny),
            ]
            .map(|(name, mode)| (name.to_owned(), mode))
            .into(),
            rules: BTreeMap::from([(
                "*".into(),
                ToolPermissionRules::Ordered(
                    [(TOOL_CLASS_READ_ONLY.into(), PermissionMode::Allow)]
                        .into_iter()
                        .collect(),
                ),
            )]),
            ..Default::default()
        }),
        ..readonly()
    };
    verifier.max_turns = Some(40);
    BTreeMap::from([
        (MAIN_AGENT_PROFILE.into(), main),
        (DEFAULT_SUBAGENT_PROFILE.into(), worker),
        ("explore".into(), explore),
        ("reviewer".into(), reviewer),
        ("planner".into(), planner),
        ("verifier".into(), verifier),
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
            if overlay.read_only.is_some() {
                base.read_only = overlay.read_only;
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
            base.context.merge_from(&overlay.context);
            if overlay.isolation.is_some() {
                base.isolation = overlay.isolation;
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
                // A legacy override that suppressed the former role prompt
                // also suppressed the instructions now shared by reference.
                if base
                    .prompts
                    .inline
                    .iter()
                    .any(|fragment| layer.disabled.contains(&fragment.id))
                {
                    base.prompts.order.retain(|id| !id.is_reusable_builtin());
                }
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
