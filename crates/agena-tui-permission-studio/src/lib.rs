//! Pure permission-rule and permission-configuration presentation contracts.

pub mod permission_helpers;
pub mod permission_rule_studio;
pub mod permission_studio;

pub use permission_helpers::{
    normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, path_access_modes_summary, path_rule_modes,
    path_rule_summary, permission_mode_label, permission_mode_token,
    permission_studio_mode_target_value, rename_network_rule, rename_path_rule,
    rename_tool_capability, rename_tool_name, rename_tool_rule, set_path_default_mode,
    tool_permission_rules_summary,
};
pub use permission_rule_studio::{
    PermissionRuleStudioEffect, PermissionRuleStudioItem, PermissionRuleStudioPresentation,
    handle_key as handle_rule_key,
};
pub use permission_studio::{
    PermissionStudioFocus, PermissionStudioNavItem, PermissionStudioPage,
    PermissionStudioPaneFocus, PermissionStudioSectionId, nav_index_for_page, nav_items,
    nav_move_step, nav_normalize_selection,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionStudioModeTarget {
    PathWorkspaceRead,
    PathWorkspaceWrite,
    PathExternalRead,
    PathExternalWrite,
    NetworkInternet,
    NetworkPrivate,
    NetworkLoopback,
    ToolDefault,
    PathRuleRead { pattern: String },
    PathRuleWrite { pattern: String },
    NetworkRule { target: String },
    ToolCapability { key: String },
    ToolName { key: String },
    ToolRule { tool_name: String },
    ToolCommandPattern { tool_name: String, pattern: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionStudioAction {
    SelectPage(PermissionStudioPage),
    EditRule { rule_id: String },
    Save,
    Delete { rule_id: String },
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PermissionStudioEffect {
    LoadRules,
    SaveRule { rule_id: String, value: String },
    DeleteRule { rule_id: String },
    Refresh,
}
