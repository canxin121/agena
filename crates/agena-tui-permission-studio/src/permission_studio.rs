//! The permission studio is one access page with a sectioned content list.
//!
//! Rows preserve section ownership for editing, keyboard navigation and pointer
//! selection. Configuration and persistence remain application responsibilities.

use agena_tui_components::SectionedListFocus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStudioSectionId {
    PathDefaults,
    PathRules,
    NetworkDefaults,
    NetworkRules,
    NetworkDenyRules,
    ToolDefaults,
    ToolNames,
    ToolCommandRules,
    RememberedRules,
    NetworkPolicy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStudioPage {
    Access,
}

pub type PermissionStudioFocus = SectionedListFocus;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PermissionStudioRow {
    Section(usize),
    Item { section: usize, item: usize },
}

pub fn content_rows(item_counts: &[usize]) -> Vec<PermissionStudioRow> {
    item_counts
        .iter()
        .enumerate()
        .flat_map(|(section, &count)| {
            std::iter::once(PermissionStudioRow::Section(section))
                .chain((0..count).map(move |item| PermissionStudioRow::Item { section, item }))
        })
        .collect()
}

pub fn selected_content_row(
    rows: &[PermissionStudioRow],
    section: usize,
    item: usize,
) -> Option<usize> {
    rows.iter()
        .position(|row| *row == PermissionStudioRow::Item { section, item })
}

/// Move through items across section boundaries, skipping empty sections.
pub fn move_content_selection(
    item_counts: &[usize],
    selected: (usize, usize),
    delta: isize,
) -> Option<(usize, usize)> {
    let items = item_counts
        .iter()
        .enumerate()
        .flat_map(|(section, &count)| (0..count).map(move |item| (section, item)))
        .collect::<Vec<_>>();
    let current = items
        .iter()
        .position(|position| *position == selected)
        .unwrap_or(0);
    let next = current
        .saturating_add_signed(delta)
        .min(items.len().saturating_sub(1));
    items.get(next).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn access_rows_keep_section_ownership_without_sidebar_pages() {
        let rows = content_rows(&[2, 0, 3]);
        assert_eq!(rows.len(), 8);
        assert_eq!(rows[3], PermissionStudioRow::Section(1));
        assert_eq!(rows[4], PermissionStudioRow::Section(2));
        assert_eq!(selected_content_row(&rows, 2, 1), Some(6));
        assert_eq!(move_content_selection(&[2, 0, 3], (0, 1), 1), Some((2, 0)));
        assert_eq!(move_content_selection(&[2, 0, 3], (2, 0), -1), Some((0, 1)));
        assert_eq!(move_content_selection(&[2, 0, 3], (0, 0), 50), Some((2, 2)));
        assert_eq!(
            move_content_selection(&[2, 0, 3], (2, 2), -50),
            Some((0, 0))
        );
        assert_eq!(move_content_selection(&[], (0, 0), 1), None);
    }
}
