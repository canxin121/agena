use super::permission_studio_sections;
use super::{
    I18n, PermissionStudioFocus, PermissionStudioOverlay, PermissionStudioPage,
    PermissionStudioSectionId, SectionedListState,
};
pub(crate) use agena_tui_permission_studio::permission_studio::{
    PermissionStudioRow, content_rows, move_content_selection, selected_content_row,
};

pub(crate) fn refresh_permission_studio_dialog(
    i18n: &I18n,
    dialog: &mut PermissionStudioOverlay,
    preferred_section: Option<PermissionStudioSectionId>,
    preferred_item_label: Option<&str>,
    _preferred_focus: Option<PermissionStudioFocus>,
) {
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
    let focus = PermissionStudioFocus::Items;
    dialog.title = permission_studio_title(i18n, dialog);
    dialog.footer = if dialog.network_rule.is_some() {
        i18n.text("permission-studio-network-form-footer")
    } else if !dialog.editable {
        i18n.text("permission-studio-access-read-only-footer")
    } else {
        permission_studio_footer(i18n, &dialog.page)
    };
    dialog.state = SectionedListState::new(sections, selected_section, selected_item, focus);
}

pub(crate) fn permission_studio_title(i18n: &I18n, dialog: &PermissionStudioOverlay) -> String {
    format!(
        "{} · {}",
        permission_studio_page_label(i18n, &dialog.page),
        dialog.title_context
    )
}

pub(crate) fn permission_studio_footer(i18n: &I18n, _page: &PermissionStudioPage) -> String {
    i18n.text("permission-studio-access-footer")
}

pub(crate) fn permission_studio_page_label(i18n: &I18n, _page: &PermissionStudioPage) -> String {
    i18n.text("permission-studio-page-access")
}

pub(crate) fn permission_studio_content_rows(
    dialog: &PermissionStudioOverlay,
) -> Vec<PermissionStudioRow> {
    content_rows(
        &dialog
            .state
            .sections()
            .iter()
            .map(|section| section.items.len())
            .collect::<Vec<_>>(),
    )
}

fn permission_studio_item_counts(dialog: &PermissionStudioOverlay) -> Vec<usize> {
    dialog
        .state
        .sections()
        .iter()
        .map(|section| {
            if dialog.network_rule.is_some()
                && section.id != PermissionStudioSectionId::NetworkRules
            {
                0
            } else {
                section.items.len()
            }
        })
        .collect()
}

pub(crate) fn move_permission_studio_selection(dialog: &mut PermissionStudioOverlay, delta: isize) {
    let counts = permission_studio_item_counts(dialog);
    if let Some((section, item)) = move_content_selection(
        &counts,
        (
            dialog.state.selected_section_index(),
            dialog.state.selected_item_index(),
        ),
        delta,
    ) {
        dialog.state.set_indices(section, item);
    }
}

pub(crate) fn jump_permission_studio_section(dialog: &mut PermissionStudioOverlay, delta: isize) {
    let counts = permission_studio_item_counts(dialog);
    let sections = counts
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count > 0).then_some(index))
        .collect::<Vec<_>>();
    let current = sections
        .iter()
        .position(|index| *index == dialog.state.selected_section_index())
        .unwrap_or(0);
    let next = current
        .saturating_add_signed(delta)
        .min(sections.len().saturating_sub(1));
    if let Some(&section) = sections.get(next) {
        dialog.state.set_indices(section, 0);
    }
}

/// A header click jumps to its first item; an item click can activate that row.
/// Pending network edits keep their original section until saved or cancelled.
pub(crate) fn select_permission_studio_row(
    dialog: &mut PermissionStudioOverlay,
    row: usize,
) -> Option<bool> {
    let (section, item, activate) = match permission_studio_content_rows(dialog).get(row)? {
        PermissionStudioRow::Section(section) => (*section, 0, false),
        PermissionStudioRow::Item { section, item } => (*section, *item, true),
    };
    let selected = dialog.state.sections().get(section)?;
    if selected.items.is_empty()
        || (dialog.network_rule.is_some() && selected.id != PermissionStudioSectionId::NetworkRules)
    {
        return None;
    }
    dialog.state.set_indices(section, item);
    Some(activate)
}
