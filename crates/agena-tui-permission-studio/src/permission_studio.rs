//! Navigation presentation for the permission studio.
//!
//! Permission configuration, editor values, and persistence remain App
//! responsibilities. This module owns the two-pane navigation vocabulary,
//! localized navigation tree, and pure selectable-list movement policy.

use agena_tui::i18n::I18n;
use agena_tui_components::{SectionedListFocus, SelectableListState};

#[derive(Debug, Clone)]
/// A navigation item of the permission studio.
pub struct PermissionStudioNavItem {
    pub label: String,
    pub level: usize,
    pub page: PermissionStudioPage,
    pub section: Option<PermissionStudioSectionId>,
    pub selectable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Pane focus of the permission studio.
pub enum PermissionStudioPaneFocus {
    Navigation,
    Content,
}

impl PermissionStudioPaneFocus {
    pub fn next(self) -> Self {
        match self {
            Self::Navigation => Self::Content,
            Self::Content => Self::Navigation,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Section of the permission studio.
pub enum PermissionStudioSectionId {
    RootPath,
    RootNetwork,
    RootTools,
    PathDefaults,
    PathRules,
    NetworkZones,
    NetworkRules,
    ToolNames,
    ToolCommandRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Page of the permission studio.
pub enum PermissionStudioPage {
    PathDefaults,
    PathRules,
    NetworkZones,
    NetworkRules,
    ToolNames,
    ToolCommandRules,
}

pub type PermissionStudioFocus = SectionedListFocus;

pub fn nav_items(i18n: &I18n) -> Vec<PermissionStudioNavItem> {
    vec![
        nav_item(
            i18n,
            "permission-studio-nav-filesystem",
            0,
            PermissionStudioPage::PathDefaults,
            Some(PermissionStudioSectionId::PathDefaults),
            false,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-default-zones",
            1,
            PermissionStudioPage::PathDefaults,
            Some(PermissionStudioSectionId::PathDefaults),
            true,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-path-rules",
            1,
            PermissionStudioPage::PathRules,
            Some(PermissionStudioSectionId::PathRules),
            true,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-network",
            0,
            PermissionStudioPage::NetworkZones,
            Some(PermissionStudioSectionId::NetworkZones),
            false,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-network-zones",
            1,
            PermissionStudioPage::NetworkZones,
            Some(PermissionStudioSectionId::NetworkZones),
            true,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-domain-rules",
            1,
            PermissionStudioPage::NetworkRules,
            Some(PermissionStudioSectionId::NetworkRules),
            true,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-tool-access",
            0,
            PermissionStudioPage::ToolNames,
            Some(PermissionStudioSectionId::ToolNames),
            false,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-name-rules",
            1,
            PermissionStudioPage::ToolNames,
            Some(PermissionStudioSectionId::ToolNames),
            true,
        ),
        nav_item(
            i18n,
            "permission-studio-nav-command-rules",
            1,
            PermissionStudioPage::ToolCommandRules,
            Some(PermissionStudioSectionId::ToolCommandRules),
            true,
        ),
    ]
}

fn nav_item(
    i18n: &I18n,
    label_key: &str,
    level: usize,
    page: PermissionStudioPage,
    section: Option<PermissionStudioSectionId>,
    selectable: bool,
) -> PermissionStudioNavItem {
    PermissionStudioNavItem {
        label: i18n.text(label_key),
        level,
        page,
        section,
        selectable,
    }
}

pub fn nav_index_for_page(page: &PermissionStudioPage) -> usize {
    match page {
        PermissionStudioPage::PathDefaults => 1,
        PermissionStudioPage::PathRules => 2,
        PermissionStudioPage::NetworkZones => 4,
        PermissionStudioPage::NetworkRules => 5,
        PermissionStudioPage::ToolNames => 7,
        PermissionStudioPage::ToolCommandRules => 8,
    }
}

pub fn nav_normalize_selection(nav: &mut SelectableListState<PermissionStudioNavItem>) {
    if nav.items.is_empty() {
        nav.selected = 0;
        return;
    }
    nav.selected = nav.selected.min(nav.items.len().saturating_sub(1));
    if nav.selected_item().is_some_and(|item| item.selectable) {
        return;
    }
    nav.selected = nav
        .items
        .iter()
        .position(|item| item.selectable)
        .unwrap_or_default();
}

pub fn nav_move_step(nav: &mut SelectableListState<PermissionStudioNavItem>, delta: isize) {
    if nav.items.is_empty() || delta == 0 {
        return;
    }
    let mut candidate = nav.selected;
    for _ in 0..nav.items.len() {
        let Some(next) = (if delta < 0 {
            candidate.checked_sub(1)
        } else {
            candidate
                .checked_add(1)
                .filter(|next| *next < nav.items.len())
        }) else {
            return;
        };
        candidate = next;
        if nav.items[candidate].selectable {
            nav.selected = candidate;
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{PermissionStudioPaneFocus, nav_items, nav_move_step};
    use agena_tui::i18n::I18n;
    use agena_tui_components::SelectableListState;

    #[test]
    fn pane_focus_cycles_and_navigation_skips_group_labels() {
        assert_eq!(
            PermissionStudioPaneFocus::Navigation.next(),
            PermissionStudioPaneFocus::Content
        );
        let items = nav_items(&I18n::english());
        let mut nav = SelectableListState::new(items, 0);
        nav_move_step(&mut nav, 1);
        assert!(nav.selected_item().is_some_and(|item| item.selectable));
    }
}
