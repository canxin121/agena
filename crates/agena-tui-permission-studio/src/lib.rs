//! # agena-tui-permission-studio
//!
//! Pure permission-rule and permission-configuration presentation contracts.
//!
//! Owns the display model and reducer for the permission studio:
//! rule helpers ([`permission_helpers`]), the rule studio
//! ([`permission_rule_studio`]), the main studio ([`permission_studio`]),
//! and the mode/action/effect enums driving the UI.

pub mod permission_helpers;
pub mod permission_rule_studio;
pub mod permission_studio;

pub use permission_helpers::{
    normalize_permission_config, parse_permission_studio_key_input,
    parse_permission_studio_optional_mode_input, parse_permission_studio_optional_path_mode_input,
    path_access_modes_summary, path_permission_mode_label, path_permission_mode_token,
    path_rule_summary, permission_mode_label, permission_mode_token,
    permission_studio_mode_target_value, permission_studio_path_target_value, rename_path_rule,
    rename_tool_name, rename_tool_rule, set_path_default_mode,
};
pub use permission_rule_studio::{
    PermissionRuleStudioEffect, PermissionRuleStudioItem, PermissionRuleStudioPresentation,
    handle_key as handle_rule_key,
};
pub use permission_studio::{
    PermissionStudioFocus, PermissionStudioPage, PermissionStudioRow, PermissionStudioSectionId,
    content_rows, move_content_selection, selected_content_row,
};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Target of a permission studio mode.
pub enum PermissionStudioModeTarget {
    PathWorkspace,
    PathExternal,
    NetworkEgressDefault,
    NetworkIngressDefault,
    NetworkHostLoopback,
    ToolDefault,
    PathRule { pattern: String },
    ToolName { key: String },
    ToolRule { tool_name: String },
    ToolCommandPattern { tool_name: String, pattern: String },
}

impl PermissionStudioModeTarget {
    pub fn is_network(&self) -> bool {
        matches!(
            self,
            Self::NetworkEgressDefault | Self::NetworkIngressDefault | Self::NetworkHostLoopback
        )
    }

    pub fn is_path(&self) -> bool {
        matches!(
            self,
            Self::PathWorkspace | Self::PathExternal | Self::PathRule { .. }
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Action of the permission studio.
pub enum PermissionStudioAction {
    SelectPage(PermissionStudioPage),
    EditRule { rule_id: String },
    Save,
    Delete { rule_id: String },
    Refresh,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Effect produced by the permission studio.
pub enum PermissionStudioEffect {
    LoadRules,
    SaveRule { rule_id: String, value: String },
    DeleteRule { rule_id: String },
    Refresh,
}
