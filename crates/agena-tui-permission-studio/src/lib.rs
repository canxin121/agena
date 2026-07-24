//! Pure permission-rule and permission-configuration presentation contracts.

pub mod permission_rule_studio;
pub mod permission_studio;

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
