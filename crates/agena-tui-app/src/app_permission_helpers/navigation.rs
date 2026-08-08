use super::permission_studio_sections;
use super::{
    I18n, PermissionStudioFocus, PermissionStudioOverlay, PermissionStudioPage,
    PermissionStudioPaneFocus, PermissionStudioSectionId, SectionedListState, SelectableListState,
};
use crate::ui_text;
pub(crate) use agena_tui_permission_studio::permission_studio::{
    nav_index_for_page as permission_studio_nav_index_for_page,
    nav_items as permission_studio_nav_items, nav_move_step as permission_studio_nav_move_step,
    nav_normalize_selection as permission_studio_nav_normalize_selection,
};

pub(crate) fn set_permission_studio_pane_focus(
    dialog: &mut PermissionStudioOverlay,
    pane_focus: PermissionStudioPaneFocus,
) {
    dialog.pane_focus = pane_focus;
    dialog.state.set_focus(match pane_focus {
        PermissionStudioPaneFocus::Navigation => PermissionStudioFocus::Navigation,
        PermissionStudioPaneFocus::Content => PermissionStudioFocus::Items,
    });
}

pub(crate) fn refresh_permission_studio_dialog(
    i18n: &I18n,
    dialog: &mut PermissionStudioOverlay,
    preferred_section: Option<PermissionStudioSectionId>,
    preferred_item_label: Option<&str>,
    preferred_focus: Option<PermissionStudioFocus>,
) {
    let nav_items = permission_studio_nav_items(i18n);
    let nav_selected =
        permission_studio_nav_index_for_page(&dialog.page).min(nav_items.len().saturating_sub(1));
    dialog.nav = SelectableListState::new(nav_items, nav_selected);
    permission_studio_nav_normalize_selection(&mut dialog.nav);
    if let Some(nav_item) = dialog.nav.selected_item() {
        dialog.page = nav_item.page.clone();
    }
    let current_section = dialog.state.selected_section().map(|section| section.id);
    let current_item_label = dialog
        .state
        .selected_item()
        .map(|item| item.label.as_str().to_string());
    let sections = permission_studio_sections(i18n, dialog);
    let selected_section = preferred_section
        .or(current_section)
        .and_then(|id| sections.iter().position(|section| section.id == id))
        .unwrap_or(0)
        .min(sections.len().saturating_sub(1));
    let section_items = sections
        .get(selected_section)
        .map(|section| section.items.as_slice())
        .unwrap_or(&[]);
    let selected_item = preferred_item_label
        .or(current_item_label.as_deref())
        .and_then(|label| section_items.iter().position(|item| item.label == label))
        .unwrap_or(0)
        .min(section_items.len().saturating_sub(1));
    let focus = preferred_focus
        .or(Some(dialog.state.focus()))
        .unwrap_or_else(|| permission_studio_default_focus(&dialog.page));
    dialog.title = permission_studio_title(i18n, dialog);
    dialog.footer = permission_studio_footer(i18n, &dialog.page);
    dialog.state = SectionedListState::new(sections, selected_section, selected_item, focus);
    if dialog.state.selected_item().is_none()
        && dialog.state.focus() == PermissionStudioFocus::Items
    {
        dialog.state.set_focus(PermissionStudioFocus::Navigation);
    }
}

pub(crate) fn permission_studio_title(i18n: &I18n, dialog: &PermissionStudioOverlay) -> String {
    let page = &dialog.page;
    format!(
        "{} · {} · {}",
        ui_text::t(i18n, "overlay-permission-studio-title"),
        dialog.title_context,
        permission_studio_page_label(i18n, page)
    )
}

pub(crate) fn permission_studio_footer(i18n: &I18n, page: &PermissionStudioPage) -> String {
    match page {
        PermissionStudioPage::PathDefaults
        | PermissionStudioPage::PathRules
        | PermissionStudioPage::NetworkZones
        | PermissionStudioPage::NetworkRules
        | PermissionStudioPage::ToolNames
        | PermissionStudioPage::ToolCommandRules => {
            ui_text::t(i18n, "overlay-permission-studio-footer-nested")
        }
    }
}

pub(crate) fn permission_studio_default_focus(
    _page: &PermissionStudioPage,
) -> PermissionStudioFocus {
    PermissionStudioFocus::Items
}

pub(crate) fn permission_studio_page_label(i18n: &I18n, page: &PermissionStudioPage) -> String {
    match page {
        PermissionStudioPage::PathDefaults => {
            ui_text::t(i18n, "permission-studio-page-path-defaults")
        }
        PermissionStudioPage::PathRules => ui_text::t(i18n, "permission-studio-page-path-rules"),
        PermissionStudioPage::NetworkZones => {
            ui_text::t(i18n, "permission-studio-page-network-zones")
        }
        PermissionStudioPage::NetworkRules => {
            ui_text::t(i18n, "permission-studio-page-network-rules")
        }
        PermissionStudioPage::ToolNames => ui_text::t(i18n, "permission-studio-page-tool-names"),
        PermissionStudioPage::ToolCommandRules => {
            ui_text::t(i18n, "permission-studio-page-tool-command-rules")
        }
    }
}
