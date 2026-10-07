//! Declarative aggregate permission configuration.

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::{
    ApprovalModelSelection, NetworkPermissionConfig, PathAccessModes, PathPermissionConfig,
    PermissionMode, ToolPermissionConfig, ToolPermissionRules,
};

/// Stable, serializable permission configuration independent of policy
/// compilation and host-specific tag interpretation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConfig {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathPermissionConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermissionConfig>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "deserialize_tools"
    )]
    pub tools: Option<ToolPermissionConfig>,
    /// Model used to make an automatic permission decision. The runtime must
    /// fail closed to an interactive `ask` when this reference is absent or
    /// cannot be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_model: Option<ApprovalModelSelection>,
}

fn deserialize_tools<'de, D>(deserializer: D) -> Result<Option<ToolPermissionConfig>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    Option::<ToolPermissionConfig>::deserialize(deserializer)
        .map(|tools| tools.map(ToolPermissionConfig::with_current_shell_names))
}

/// The path patterns of the runtime's own per-workspace state directory
/// (`~/.agena/projects/<workspace-key>/`), shared with `agena-runtime-tools`'
/// `project_state_dir`. The `<home>` alias resolves to the same `HOME` /
/// `USERPROFILE` the runtime's `agena_home_dir()` reads.
pub const RUNTIME_STATE_PATH: &str = "<home>/agena/projects";
/// The same directory and everything under it. The alias itself matches only
/// the root, so the contents need their own entry.
pub const RUNTIME_STATE_PATH_GLOB: &str = "<home>/agena/projects/**";

/// The command-class keywords a `tools.rules.<tool>` entry may use instead of a
/// command pattern.
pub const COMMAND_CLASS_NO_OP: &str = "no-op";
pub const COMMAND_CLASS_ROUTINE: &str = "routine";
pub const COMMAND_CLASS_DANGEROUS: &str = "dangerous";
/// The keyword for the read-only tool class. It is written under the wildcard
/// tool name, `tools.rules."*"`, because the class is derived from a tool's
/// declared read_only tag rather than from its name.
pub const TOOL_CLASS_READ_ONLY: &str = "read-only";

fn allow_rw() -> PathAccessModes {
    PathAccessModes {
        read: Some(PermissionMode::Allow),
        write: Some(PermissionMode::Allow),
    }
}

/// Which of the approval prompt's conditional path promises are in effect.
///
/// Each flag is true when the sandbox's compiled path policy approves that
/// class of action outright, i.e. the approval model is free to stop worrying
/// about it. The prompt is assembled from these because telling the model that
/// temp-directory writes need no thought would silently defeat a user who
/// narrowed the rule covering them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PathClassPromptFlags {
    /// Paths inside the runtime's own per-workspace state directory.
    pub internal_paths_allowed: bool,
    /// Paths inside the system temporary directory.
    pub temp_paths_allowed: bool,
}

impl PathClassPromptFlags {
    /// The flags for a sandbox whose path policy approves both classes
    /// outright - the shipped configuration.
    pub fn all_allowed() -> Self {
        Self {
            internal_paths_allowed: true,
            temp_paths_allowed: true,
        }
    }

    /// Whether every conditional promise is in effect, i.e. the prompt renders
    /// its full class-default bullet.
    pub fn every_class_allowed(&self) -> bool {
        self.internal_paths_allowed && self.temp_paths_allowed
    }
}

impl PermissionConfig {
    /// The shipped configuration: what a user gets when their file says nothing
    /// about permissions.
    ///
    /// The class defaults live here as ordinary entries of the sections they
    /// belong to, not as keys of their own — the temp and runtime-state paths
    /// are `path.rules` entries, the interaction tool is a `tools.names` entry,
    /// and the read-only and command classes are `tools.rules` entries. That is
    /// what makes them editable and restorable (a user writes the same key, or
    /// deletes it) without the schema growing a parallel set of keys.
    pub fn global_default() -> Self {
        let auto = PermissionMode::Auto;
        let allow = PermissionMode::Allow;
        let deny = PermissionMode::Deny;
        Self {
            path: Some(PathPermissionConfig {
                workspace: Some(PathAccessModes {
                    read: Some(allow),
                    write: Some(auto),
                }),
                external: Some(PathAccessModes {
                    read: Some(auto),
                    write: Some(auto),
                }),
                rules: IndexMap::from([
                    ("<tmp>".to_string(), allow_rw()),
                    ("<tmp>/**".to_string(), allow_rw()),
                    (RUNTIME_STATE_PATH.to_string(), allow_rw()),
                    (RUNTIME_STATE_PATH_GLOB.to_string(), allow_rw()),
                ]),
            }),
            network: Some(NetworkPermissionConfig {
                internet: Some(auto),
                private: Some(auto),
                loopback: Some(auto),
                ..Default::default()
            }),
            tools: Some(ToolPermissionConfig {
                // Ordinary execution tools default to Allow: their effects are
                // already constrained by the path, network, and shell-command
                // policies. Ask/Deny are opt-in per tool name or via the shell
                // command pattern table.
                default: Some(allow),
                names: BTreeMap::from([
                    // `agena.interaction.ask` *is* the prompt: a permission
                    // `Ask` here would confirm the question instead of asking
                    // it, so the tool is allowed and raises its own
                    // `UserInputRequired`. Both spellings are listed because
                    // the executor answers to the canonical and the compact
                    // tool name.
                    ("agena.interaction.ask".to_string(), allow),
                    ("interaction.ask".to_string(), allow),
                    // Web fetch/search are allowlisted because network policy
                    // already governs their targets.
                    ("agena.web.search".to_string(), allow),
                    ("agena.web.fetch".to_string(), allow),
                ]),
                rules: BTreeMap::from([
                    (
                        "agena.shell.exec".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([
                            (COMMAND_CLASS_NO_OP.to_string(), allow),
                            (COMMAND_CLASS_ROUTINE.to_string(), allow),
                            (COMMAND_CLASS_DANGEROUS.to_string(), deny),
                        ])),
                    ),
                    (
                        "agena.shell.spawn".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([
                            (COMMAND_CLASS_NO_OP.to_string(), allow),
                            (COMMAND_CLASS_ROUTINE.to_string(), allow),
                            (COMMAND_CLASS_DANGEROUS.to_string(), deny),
                        ])),
                    ),
                    (
                        "agena.shell.watch".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([
                            (COMMAND_CLASS_NO_OP.to_string(), allow),
                            (COMMAND_CLASS_ROUTINE.to_string(), allow),
                            (COMMAND_CLASS_DANGEROUS.to_string(), deny),
                        ])),
                    ),
                    (
                        "agena.shell.open".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([
                            (COMMAND_CLASS_NO_OP.to_string(), allow),
                            (COMMAND_CLASS_ROUTINE.to_string(), allow),
                            (COMMAND_CLASS_DANGEROUS.to_string(), deny),
                        ])),
                    ),
                    (
                        "agena.shell.write".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([(
                            COMMAND_CLASS_DANGEROUS.to_string(),
                            deny,
                        )])),
                    ),
                    // Tools whose contract is read-only, and neither shell nor
                    // interactive. Reading cannot change anything.
                    (
                        "*".to_string(),
                        ToolPermissionRules::Ordered(IndexMap::from([(
                            TOOL_CLASS_READ_ONLY.to_string(),
                            allow,
                        )])),
                    ),
                ]),
                ..Default::default()
            }),
            approval_model: None,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.path
            .as_ref()
            .is_none_or(PathPermissionConfig::is_empty)
            && self
                .network
                .as_ref()
                .is_none_or(NetworkPermissionConfig::is_empty)
            && self
                .tools
                .as_ref()
                .is_none_or(ToolPermissionConfig::is_empty)
            && self.approval_model.is_none()
    }

    pub fn merge_from(&mut self, overlay: Self) {
        merge_path_section(&mut self.path, overlay.path);
        merge_network_section(&mut self.network, overlay.network);
        merge_tool_section(&mut self.tools, overlay.tools);
        if overlay.approval_model.is_some() {
            self.approval_model = overlay.approval_model;
        }
    }

    pub fn merged_with(&self, overlay: &Self) -> Self {
        let mut merged = self.clone();
        merged.merge_from(overlay.clone());
        merged
    }
}

fn merge_path_section(
    current: &mut Option<PathPermissionConfig>,
    overlay: Option<PathPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_network_section(
    current: &mut Option<NetworkPermissionConfig>,
    overlay: Option<NetworkPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}

fn merge_tool_section(
    current: &mut Option<ToolPermissionConfig>,
    overlay: Option<ToolPermissionConfig>,
) {
    match (current.as_mut(), overlay) {
        (Some(current), Some(overlay)) => current.merge_from(overlay),
        (None, Some(overlay)) => *current = Some(overlay),
        (_, None) => {}
    }
}
