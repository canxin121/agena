//! Session hub home screen: presentation state and rendering.
//!
//! The App supplies directory/session metadata. This presentation owns tree
//! expansion, filtering, selection and independent directory/quick-access
//! viewports; requests and durable session operations remain in the App.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};

use agena_tui::i18n::I18n;
use agena_tui_components::theme::{accent_color, danger_color, muted_style, selection_style};
use agena_tui_components::{ShortcutHint, build_shortcut_line};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, HighlightSpacing, List, ListItem, ListState, Paragraph},
};

mod columns;
use columns::HubColumns;

/// A session entry shown in a hub section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHubItem {
    pub session_id: i64,
    pub workspace_id: i64,
    pub pinned: bool,
    pub title: String,
    /// Durable user favorite state. Rendered as a persistent star marker and
    /// used by the hub's favorite toggle without conflating it with pinning.
    pub favorite: bool,
    /// First line of the row; the renderer never invents text, so the App
    /// pre-localizes it.
    pub label: String,
    /// Separate, pre-localized values for the fixed status and time columns.
    pub status: String,
    pub updated: String,
    /// Full metadata of the selected session.
    pub detail: String,
    /// True for the synthetic "new session" action inside its project.
    pub is_new_session: bool,
}

/// Identity of a project bucket or global activity section.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SessionHubSectionKind {
    New,
    Directories,
    Search,
    Pinned,
    Favorites,
    Running,
    Attention,
    Recent,
}

impl SessionHubSectionKind {
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::Search => "hub-section-search",
            Self::New => "hub-section-new",
            Self::Directories => "hub-section-directories",
            Self::Pinned => "hub-section-pinned",
            Self::Favorites => "hub-section-favorites",
            Self::Running => "hub-section-running",
            Self::Attention => "hub-section-attention",
            Self::Recent => "hub-section-recent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum HubPageTarget {
    Workspaces,
    Directory(i64),
    DirectorySection(i64, SessionHubSectionKind),
    Section(SessionHubSectionKind),
}

#[derive(Debug, Clone, Default)]
pub struct HubPagination {
    pub page: usize,
    pub has_more: bool,
    pub total: Option<usize>,
}

fn page_rows(target: HubPageTarget, page: &HubPagination) -> Vec<HubRow> {
    let mut rows = Vec::new();
    if page.page > 0 {
        rows.push(HubRow::Page {
            target,
            page: page.page - 1,
            forward: false,
        });
    }
    if page.has_more {
        rows.push(HubRow::Page {
            target,
            page: page.page + 1,
            forward: true,
        });
    }
    rows
}

/// A section of the hub. Items are display-only projections built by the App.
#[derive(Debug, Clone)]
pub struct SessionHubSection {
    pub kind: SessionHubSectionKind,
    pub items: Vec<SessionHubItem>,
}

impl SessionHubSection {
    pub fn new(kind: SessionHubSectionKind, items: Vec<SessionHubItem>) -> Self {
        Self { kind, items }
    }
}

/// A row of the flattened hub list: either a section header (not selectable)
/// or a session row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HubRow {
    Page {
        target: HubPageTarget,
        page: usize,
        forward: bool,
    },
    /// Section label line drawn inside the single hub box. Never selectable.
    Header(SessionHubSectionKind),
    /// A project-local bucket. It stays in the directory viewport and is
    /// never mistaken for the start of global quick access.
    DirectoryHeader {
        workspace_id: i64,
        kind: SessionHubSectionKind,
    },
    /// A selectable session row (including the "+ new session" action row).
    Item(SessionHubItem),
    Directory {
        workspace_id: i64,
        path: String,
        expanded: bool,
        count: usize,
    },
    More {
        workspace_id: i64,
        remaining: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum HubRowIdentity {
    Item(i64, i64),
    Directory(i64),
    More(i64),
    Page(HubPageTarget, bool),
}

impl HubRow {
    fn identity(&self) -> Option<HubRowIdentity> {
        match self {
            Self::Page {
                target, forward, ..
            } => Some(HubRowIdentity::Page(*target, *forward)),
            Self::Header(_) | Self::DirectoryHeader { .. } => None,
            Self::Item(item) => Some(HubRowIdentity::Item(item.session_id, item.workspace_id)),
            Self::Directory { workspace_id, .. } => Some(HubRowIdentity::Directory(*workspace_id)),
            Self::More { workspace_id, .. } => Some(HubRowIdentity::More(*workspace_id)),
        }
    }

    fn height(&self) -> u16 {
        1
    }
}

/// A display-only directory catalog. Pagination limits rendered rows; the App
/// owns fetching metadata and durable mutations, just as for the Web sidebar.
#[derive(Debug, Clone)]
pub struct SessionHubDirectory {
    pub workspace_id: i64,
    pub path: String,
    pub new_session: SessionHubItem,
    pub sections: Vec<SessionHubSection>,
    pub items: Vec<SessionHubItem>,
    pub expanded: bool,
    pub visible_limit: usize,
}

/// Display projection of the session hub.
///
/// Directory rows and quick-access sections share one semantic navigation
/// order. Their screen viewports scroll independently, keeping quick access
/// visible below long directory lists. Headers are not selectable.
#[derive(Debug, Clone)]
pub struct SessionHubPresentation {
    /// Full section catalog from the last overview (unfiltered). Re-filtered
    /// on every query edit, so the server call itself stays un-filtered.
    sections: Vec<SessionHubSection>,
    /// Flattened visible rows derived from `sections` through the filter.
    rows: Vec<HubRow>,
    /// Flat index into `rows` of the selected row (always an item row when the
    /// list is non-empty).
    selection: usize,
    /// Current client-side filter (trimmed, lowercased). Empty lists all rows.
    query: String,
    directories: Vec<SessionHubDirectory>,
    pagination: BTreeMap<HubPageTarget, HubPagination>,
    directory_offset: Cell<usize>,
    quick_offset: Cell<usize>,
    directory_height: Cell<usize>,
    quick_height: Cell<usize>,
}

impl SessionHubPresentation {
    /// An empty hub. The initial route is created before the first overview
    /// response lands, so it starts with no rows.
    pub fn empty() -> Self {
        Self {
            sections: Vec::new(),
            rows: Vec::new(),
            selection: 0,
            query: String::new(),
            directories: Vec::new(),
            pagination: BTreeMap::new(),
            directory_offset: Cell::new(0),
            quick_offset: Cell::new(0),
            directory_height: Cell::new(10),
            quick_height: Cell::new(10),
        }
    }

    /// Replace the section catalog and re-derive the visible rows, preserving
    /// the selected row by identity when it survives the rebuild.
    pub fn set_sections(&mut self, sections: Vec<SessionHubSection>) {
        self.sections = sections;
        self.rebuild_rows();
    }

    pub fn set_directories(&mut self, mut directories: Vec<SessionHubDirectory>) {
        for directory in &mut directories {
            if let Some(old) = self
                .directories
                .iter()
                .find(|old| old.workspace_id == directory.workspace_id)
            {
                directory.expanded = old.expanded;
                directory.visible_limit = old.visible_limit;
            }
        }
        self.directories = directories;
        self.rebuild_rows();
    }

    pub fn set_catalog(
        &mut self,
        directories: Vec<SessionHubDirectory>,
        sections: Vec<SessionHubSection>,
    ) {
        self.sections = sections;
        self.set_directories(directories);
    }

    pub fn set_pagination(&mut self, pagination: BTreeMap<HubPageTarget, HubPagination>) {
        self.pagination = pagination;
        self.rebuild_rows();
    }

    pub fn expanded_directories(&self) -> BTreeSet<i64> {
        self.directories
            .iter()
            .filter(|directory| directory.expanded)
            .map(|directory| directory.workspace_id)
            .collect()
    }

    pub fn selected_row(&self) -> Option<&HubRow> {
        self.rows.get(self.selection)
    }

    /// The backend page containing a row. Directory headings page workspaces;
    /// their children page sessions, and quick-access rows page their own bucket.
    fn row_page_target(&self, index: usize) -> HubPageTarget {
        match self.rows.get(index) {
            Some(HubRow::Page { target, .. }) => return *target,
            Some(HubRow::Directory { .. }) => return HubPageTarget::Workspaces,
            _ => {}
        }
        for row in self.rows[..index.min(self.rows.len())].iter().rev() {
            match row {
                HubRow::Header(kind) => return HubPageTarget::Section(*kind),
                HubRow::DirectoryHeader { workspace_id, kind } => {
                    return if *kind == SessionHubSectionKind::Recent {
                        HubPageTarget::Directory(*workspace_id)
                    } else {
                        HubPageTarget::DirectorySection(*workspace_id, *kind)
                    };
                }
                HubRow::Directory { workspace_id, .. } => {
                    return HubPageTarget::Directory(*workspace_id);
                }
                _ => {}
            }
        }
        HubPageTarget::Workspaces
    }

    pub fn selected_page_target(&self) -> HubPageTarget {
        self.row_page_target(self.selection)
    }

    pub fn page_request(&self, forward: bool) -> Option<(HubPageTarget, usize)> {
        let target = self.selected_page_target();
        let page = self.pagination.get(&target)?;
        if forward {
            page.has_more
                .then_some((target, page.page.saturating_add(1)))
        } else {
            page.page.checked_sub(1).map(|page| (target, page))
        }
    }

    /// After a remote page arrives, focus its first/last session instead of
    /// falling back to the new-session action or another duplicate bucket.
    pub fn select_page_target(&mut self, target: HubPageTarget, last: bool) {
        let indices = self
            .item_indices()
            .into_iter()
            .filter(|&index| {
                self.row_page_target(index) == target
                    && match &self.rows[index] {
                        HubRow::Item(item) => !item.is_new_session,
                        HubRow::Directory { .. } => true,
                        _ => false,
                    }
            })
            .collect::<Vec<_>>();
        if let Some(index) = if last {
            indices.last()
        } else {
            indices.first()
        } {
            self.selection = *index;
        }
    }

    pub fn viewport_page_size(&self) -> usize {
        if self.same_region(self.selection, 0) {
            self.directory_height.get().max(1)
        } else {
            self.quick_height.get().max(1)
        }
    }

    pub fn selected_workspace(&self) -> Option<i64> {
        match self.selected_row()? {
            HubRow::Page {
                target: HubPageTarget::Directory(id) | HubPageTarget::DirectorySection(id, _),
                ..
            } => Some(*id),
            HubRow::Item(item) => Some(item.workspace_id),
            HubRow::Directory { workspace_id, .. } | HubRow::More { workspace_id, .. } => {
                Some(*workspace_id)
            }
            _ => None,
        }
    }

    pub fn select_row(&mut self, index: usize) {
        if self
            .rows
            .get(index)
            .is_some_and(|row| row.identity().is_some())
        {
            self.selection = index;
        }
    }

    /// Called once on opening the hub, before the asynchronous catalog load.
    /// Subsequent refreshes preserve the user's selection by row and bucket.
    pub fn select_new_session(&mut self, workspace_id: i64) {
        if let Some(index) = self.rows.iter().position(|row| {
            matches!(row, HubRow::Item(item) if item.is_new_session && item.workspace_id == workspace_id)
        }) {
            self.selection = index;
        }
    }

    pub fn toggle_directory(&mut self, workspace_id: i64, expanded: Option<bool>) {
        if let Some(directory) = self
            .directories
            .iter_mut()
            .find(|directory| directory.workspace_id == workspace_id)
        {
            directory.expanded = expanded.unwrap_or(!directory.expanded);
        }
        self.rebuild_rows();
    }

    pub fn show_more(&mut self, workspace_id: i64) {
        if let Some(directory) = self
            .directories
            .iter_mut()
            .find(|directory| directory.workspace_id == workspace_id)
        {
            directory.visible_limit = directory.visible_limit.saturating_add(20);
        }
        let previous = self.selection;
        self.rebuild_rows();
        // Enter on "more" lands on the first newly revealed row.
        self.selection = previous.min(self.rows.len().saturating_sub(1));
        self.clamp_selection();
    }

    pub fn set_item_pinned(&mut self, session_id: i64, pinned: bool) {
        self.set_item_mark(session_id, SessionHubSectionKind::Pinned, pinned);
    }

    /// Update every duplicate, then adjust only the owning project's bucket.
    /// Pinning and favoriting are independent and have independent pages.
    fn set_item_mark(&mut self, session_id: i64, kind: SessionHubSectionKind, enabled: bool) {
        let mut source = None;
        let mut changed = false;
        for item in self
            .sections
            .iter_mut()
            .flat_map(|section| &mut section.items)
            .chain(self.directories.iter_mut().flat_map(|directory| {
                directory.items.iter_mut().chain(
                    directory
                        .sections
                        .iter_mut()
                        .flat_map(|section| &mut section.items),
                )
            }))
        {
            if item.session_id != session_id || item.is_new_session {
                continue;
            }
            let mark = if kind == SessionHubSectionKind::Pinned {
                &mut item.pinned
            } else {
                &mut item.favorite
            };
            changed |= *mark != enabled;
            *mark = enabled;
            source = Some(item.clone());
        }
        if let Some(item) = source {
            let target = HubPageTarget::DirectorySection(item.workspace_id, kind);
            let page = self.pagination.get_mut(&target);
            let first_page = page.as_ref().is_none_or(|page| page.page == 0);
            if changed
                && let Some(page) = page
                && let Some(total) = &mut page.total
            {
                *total = if enabled {
                    total.saturating_add(1)
                } else {
                    total.saturating_sub(1)
                };
            }
            if let Some(directory) = self
                .directories
                .iter_mut()
                .find(|directory| directory.workspace_id == item.workspace_id)
            {
                if let Some(section) = directory
                    .sections
                    .iter_mut()
                    .find(|section| section.kind == kind)
                {
                    if !enabled {
                        section.items.retain(|entry| entry.session_id != session_id);
                    } else if first_page
                        && !section
                            .items
                            .iter()
                            .any(|entry| entry.session_id == session_id)
                    {
                        section.items.insert(0, item);
                        section.items.truncate(directory.visible_limit);
                    }
                } else if enabled && first_page {
                    directory
                        .sections
                        .push(SessionHubSection::new(kind, vec![item]));
                }
            }
        }
        self.rebuild_rows();
    }

    /// Filter the sections to rows matching `query` (case-insensitive substring
    /// over title/label/session id). Create actions are hidden while searching.
    /// Selection is preserved by identity when the selected row still matches,
    /// otherwise it lands on the first row.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
        self.rebuild_rows();
    }

    /// Apply an authoritative favorite update to all visible copies.
    pub fn set_item_favorite(&mut self, session_id: i64, favorite: bool) {
        self.set_item_mark(session_id, SessionHubSectionKind::Favorites, favorite);
    }

    /// Total number of selectable session rows.
    pub fn total_count(&self) -> usize {
        self.item_indices().len()
    }

    /// The visible rows (headers and items) in display order.
    pub fn rows(&self) -> &[HubRow] {
        &self.rows
    }

    /// Flat index into `rows` of the selected row.
    pub fn selection(&self) -> usize {
        self.selection
    }

    /// The session to open on Enter.
    pub fn selected_item(&self) -> Option<&SessionHubItem> {
        match self.rows.get(self.selection) {
            Some(HubRow::Item(item)) => Some(item),
            _ => None,
        }
    }

    /// Move the selection `delta` rows over the selectable rows only, crossing
    /// section boundaries and skipping header lines, so `↑/↓` can always reach
    /// every bucket.
    pub fn move_selection(&mut self, delta: isize) {
        let items = self.item_indices();
        if items.is_empty() {
            return;
        }
        let current = items
            .iter()
            .position(|&index| index == self.selection)
            .unwrap_or(0);
        let target = (current as isize + delta).clamp(0, items.len() as isize - 1) as usize;
        self.selection = items[target];
    }

    pub fn same_region(&self, left: usize, right: usize) -> bool {
        let split = self
            .rows
            .iter()
            .position(|row| matches!(row, HubRow::Header(_)))
            .unwrap_or(self.rows.len());
        (left < split) == (right < split)
    }

    pub fn scroll_region(&mut self, delta: isize) {
        let split = self
            .rows
            .iter()
            .position(|row| matches!(row, HubRow::Header(_)))
            .unwrap_or(self.rows.len());
        let directory_region = self.selection < split;
        let items = self
            .item_indices()
            .into_iter()
            .filter(|index| (*index < split) == directory_region)
            .collect::<Vec<_>>();
        if items.is_empty() {
            return;
        }
        let current = items
            .iter()
            .position(|index| *index == self.selection)
            .unwrap_or(0);
        let next = (current as isize + delta).clamp(0, items.len() as isize - 1) as usize;
        self.selection = items[next];
    }

    /// Move the selection by a page of `page_size` selectable rows.
    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        let items = self.item_indices();
        if items.is_empty() {
            return;
        }
        let current = items
            .iter()
            .position(|&index| index == self.selection)
            .unwrap_or(0);
        let target = (current as isize + delta * page_size as isize)
            .clamp(0, items.len() as isize - 1) as usize;
        self.selection = items[target];
    }

    pub fn move_selection_home(&mut self) {
        if let Some(&first) = self.item_indices().first() {
            self.selection = first;
        }
    }

    pub fn move_selection_end(&mut self) {
        if let Some(&last) = self.item_indices().last() {
            self.selection = last;
        }
    }

    /// Jump to the first row of the section `delta` steps away (wrapping).
    /// Tab / Shift+Tab use this; empty sections are already hidden from
    /// `rows`, so the cursor never lands on an empty bucket.
    pub fn move_selection_section(&mut self, delta: isize) {
        let spans = self.section_spans();
        if spans.len() <= 1 {
            return;
        }
        let current = spans
            .iter()
            .position(|(_, first, count)| {
                self.selection >= *first && self.selection < first + count
            })
            .unwrap_or(0);
        let next = (current as isize + delta).rem_euclid(spans.len() as isize) as usize;
        self.selection = spans[next].1;
    }

    /// Keep the selection on the nearest selectable row after a rebuild.
    pub fn clamp_selection(&mut self) {
        let items = self.item_indices();
        if items.is_empty() {
            self.selection = 0;
            return;
        }
        if items.contains(&self.selection) {
            return;
        }
        self.selection = items
            .iter()
            .min_by_key(|&&index| (index as isize - self.selection as isize).unsigned_abs())
            .copied()
            .unwrap_or(items[0]);
    }

    /// Re-flatten `self.sections` through the current filter into `self.rows`,
    /// preserving the selected row by identity when it survives.
    fn rebuild_rows(&mut self) {
        let previous = self.selected_row().and_then(HubRow::identity);
        let previous_selection = self.selection;
        let previous_target = self.selected_page_target();
        let filtered = self
            .sections
            .iter()
            .map(|section| {
                let items = if self.query.is_empty() {
                    section.items.clone()
                } else {
                    section
                        .items
                        .iter()
                        .filter(|item| item.is_new_session || session_matches(item, &self.query))
                        .cloned()
                        .collect()
                };
                SessionHubSection::new(section.kind, items)
            })
            .collect::<Vec<_>>();
        self.rows = build_rows(filtered);
        // Insert bounded server pages after their quick-access section.
        for (target, page) in self.pagination.iter().rev() {
            let HubPageTarget::Section(kind) = target else {
                continue;
            };
            let controls = page_rows(*target, page);
            if controls.is_empty() {
                continue;
            }
            if let Some(start) = self
                .rows
                .iter()
                .position(|row| matches!(row, HubRow::Header(value) if value == kind))
            {
                let end = self
                    .rows
                    .iter()
                    .enumerate()
                    .skip(start + 1)
                    .find(|(_, row)| matches!(row, HubRow::Header(_)))
                    .map(|(index, _)| index)
                    .unwrap_or(self.rows.len());
                self.rows.splice(end..end, controls);
            } else {
                self.rows.push(HubRow::Header(*kind));
                self.rows.extend(controls);
            }
        }
        let mut directory_rows = Vec::new();
        for directory in &self.directories {
            let path_matches = directory.path.to_lowercase().contains(&self.query);
            let matches =
                |item: &&SessionHubItem| path_matches || session_matches(item, &self.query);
            let items = directory
                .items
                .iter()
                .filter(matches)
                .cloned()
                .collect::<Vec<_>>();
            let matching_bucket = directory
                .sections
                .iter()
                .any(|section| section.items.iter().any(|item| matches(&item)));
            if !self.query.is_empty() && !path_matches && items.is_empty() && !matching_bucket {
                continue;
            }
            let expanded = directory.expanded || !self.query.is_empty();
            let target = HubPageTarget::Directory(directory.workspace_id);
            directory_rows.push(HubRow::Directory {
                workspace_id: directory.workspace_id,
                path: directory.path.clone(),
                expanded,
                count: self
                    .pagination
                    .get(&target)
                    .and_then(|page| page.total)
                    .unwrap_or(items.len()),
            });
            if !expanded {
                continue;
            }
            if self.query.is_empty() {
                directory_rows.push(HubRow::Item(directory.new_session.clone()));
            }
            for kind in [
                SessionHubSectionKind::Pinned,
                SessionHubSectionKind::Favorites,
            ] {
                let target = HubPageTarget::DirectorySection(directory.workspace_id, kind);
                let items = directory
                    .sections
                    .iter()
                    .find(|section| section.kind == kind)
                    .map(|section| {
                        section
                            .items
                            .iter()
                            .filter(matches)
                            .cloned()
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                let controls = self
                    .pagination
                    .get(&target)
                    .map(|page| page_rows(target, page))
                    .unwrap_or_default();
                if items.is_empty() && controls.is_empty() {
                    continue;
                }
                directory_rows.push(HubRow::DirectoryHeader {
                    workspace_id: directory.workspace_id,
                    kind,
                });
                directory_rows.extend(items.into_iter().map(HubRow::Item));
                directory_rows.extend(controls);
            }
            let controls = self
                .pagination
                .get(&target)
                .map(|page| page_rows(target, page))
                .unwrap_or_default();
            if !items.is_empty() || !controls.is_empty() {
                directory_rows.push(HubRow::DirectoryHeader {
                    workspace_id: directory.workspace_id,
                    kind: SessionHubSectionKind::Recent,
                });
            }
            let remaining = items.len().saturating_sub(directory.visible_limit);
            directory_rows.extend(
                items
                    .into_iter()
                    .take(directory.visible_limit)
                    .map(HubRow::Item),
            );
            directory_rows.extend(controls);
            if !self.pagination.contains_key(&target) && remaining > 0 {
                directory_rows.push(HubRow::More {
                    workspace_id: directory.workspace_id,
                    remaining,
                });
            }
        }
        if let Some(page) = self.pagination.get(&HubPageTarget::Workspaces) {
            directory_rows.extend(page_rows(HubPageTarget::Workspaces, page));
        }
        self.rows.splice(0..0, directory_rows);
        self.selection = previous
            .and_then(|previous| {
                self.rows
                    .iter()
                    .enumerate()
                    .filter_map(|(index, row)| {
                        (row.identity() == Some(previous)
                            && self.row_page_target(index) == previous_target)
                            .then_some(index)
                    })
                    .min_by_key(|index| {
                        (*index as isize - previous_selection as isize).unsigned_abs()
                    })
            })
            .or_else(|| {
                self.item_indices()
                    .into_iter()
                    .filter(|&index| self.row_page_target(index) == previous_target)
                    .min_by_key(|&index| index.abs_diff(previous_selection))
            })
            .or_else(|| {
                // Removing the last pin/favorite can remove its bucket. Keep
                // focus on the same session in the project's ordinary list.
                let previous = previous?;
                self.rows.iter().enumerate().find_map(|(index, row)| {
                    (row.identity() == Some(previous)
                        && matches!(self.row_page_target(index), HubPageTarget::Directory(_)))
                    .then_some(index)
                })
            })
            .unwrap_or(0);
        self.clamp_selection();
    }

    /// Flat row indices of every selectable item row, in display order.
    fn item_indices(&self) -> Vec<usize> {
        self.rows
            .iter()
            .enumerate()
            .filter_map(|(index, row)| match row {
                HubRow::Item(_)
                | HubRow::Directory { .. }
                | HubRow::More { .. }
                | HubRow::Page { .. } => Some(index),
                HubRow::Header(_) | HubRow::DirectoryHeader { .. } => None,
            })
            .collect()
    }

    /// For every visible project or bucket, `(kind, first selectable row,
    /// selectable row count)`. Project-local headers are navigation boundaries.
    fn section_spans(&self) -> Vec<(SessionHubSectionKind, usize, usize)> {
        let mut spans: Vec<(SessionHubSectionKind, usize, usize)> = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            match row {
                HubRow::Header(kind) | HubRow::DirectoryHeader { kind, .. } => {
                    spans.push((*kind, index + 1, 0))
                }
                HubRow::Directory { .. } => {
                    spans.push((SessionHubSectionKind::Directories, index, 1))
                }
                HubRow::Item(_) | HubRow::More { .. } | HubRow::Page { .. } => {
                    match spans.last_mut() {
                        Some(span) => span.2 += 1,
                        None => spans.push((SessionHubSectionKind::New, index, 1)),
                    }
                }
            }
        }
        spans.retain(|(_, _, count)| *count > 0);
        spans
    }
}

/// Case-insensitive substring match of one hub row against the search query.
fn session_matches(item: &SessionHubItem, query_lower: &str) -> bool {
    item.label.to_lowercase().contains(query_lower)
        || item.title.to_lowercase().contains(query_lower)
        || item.detail.to_lowercase().contains(query_lower)
        || item.session_id.to_string().contains(query_lower)
}

/// Flatten global activity sections. Project actions and marked sessions live
/// in the directory tree, never as global rows.
fn build_rows(sections: Vec<SessionHubSection>) -> Vec<HubRow> {
    let mut rows = Vec::new();
    for section in sections {
        if section.items.is_empty()
            || matches!(
                section.kind,
                SessionHubSectionKind::New
                    | SessionHubSectionKind::Pinned
                    | SessionHubSectionKind::Favorites
            )
        {
            continue;
        }
        rows.push(HubRow::Header(section.kind));
        rows.extend(section.items.into_iter().map(HubRow::Item));
    }
    rows
}

/// Renders the session hub home screen into `area`.
///
/// `loading` and `error` are application-owned response state: loading hides
/// the empty state while the first overview response is in flight, and an
/// error is drawn as a banner above (possibly stale) rows. `search_active` is
/// whether the user is composing a search (shown as an active filter box);
/// `query` is the live filter — when non-empty the list narrows to matching
/// rows.
pub fn render_session_hub(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &SessionHubPresentation,
    loading: bool,
    error: Option<&str>,
    search_active: bool,
    query: &str,
    i18n: &I18n,
) {
    use agena_tui_components::pointer::{self, PointerAction};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    pointer::begin_surface();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", i18n.text("hub-title")));
    frame.render_widget(block, area);
    let [actions, search, content, details, footer] = hub_layout(area, error.is_some());
    let selected = presentation
        .selected_item()
        .filter(|item| !item.is_new_session);
    let pin = format!(
        "p {}",
        i18n.text(if selected.is_some_and(|item| item.pinned) {
            "session-action-unpin"
        } else {
            "session-action-pin"
        })
    );
    let favorite = format!(
        "f {}",
        i18n.text(if selected.is_some_and(|item| item.favorite) {
            "hub-action-unfavorite"
        } else {
            "hub-action-favorite"
        })
    );
    let create = format!("^N {}", i18n.text("hub-action-create"));
    let find = format!("/ {}", i18n.text("hub-action-search"));
    let ctrl = |c| PointerAction::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL));
    let alt = |c| PointerAction::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT));
    pointer::render_buttons(
        frame,
        actions,
        &[
            (&create, ctrl('n')),
            (&find, ctrl('f')),
            (&pin, alt('p')),
            (&favorite, alt('f')),
        ],
    );

    let mut search_line = if !query.is_empty() || search_active {
        Line::from(Span::styled(
            i18n.text_args(
                "hub-search-active",
                &agena_tui::fl_args!("query" => agena_tui::sanitize_picker_text(query)),
            ),
            accent_color(),
        ))
    } else {
        Line::from(Span::styled(i18n.text("session-hub-guide"), muted_style()))
    };
    if loading {
        search_line.spans.push(Span::styled(
            format!("  … {}", i18n.text("overlay-picker-loading")),
            muted_style(),
        ));
    }
    frame.render_widget(Paragraph::new(search_line), search);
    if let Some(error) = error {
        frame.render_widget(
            Paragraph::new(agena_tui::sanitize_picker_text(error))
                .style(Style::default().fg(danger_color())),
            Rect::new(content.x, content.y.saturating_sub(1), content.width, 1),
        );
    }
    if presentation.rows().is_empty() {
        frame.render_widget(
            Paragraph::new(i18n.text(if loading {
                "overlay-picker-loading"
            } else {
                "hub-empty"
            }))
            .style(muted_style()),
            content,
        );
    } else {
        render_rows(frame, content, presentation, i18n);
    }
    let (title, detail) = match presentation.selected_row() {
        Some(HubRow::Item(item)) => (item.label.clone(), item.detail.clone()),
        Some(HubRow::Directory { path, count, .. }) => (
            path.clone(),
            format!("{} · {count}", i18n.text("session-directory-guide")),
        ),
        _ => (String::new(), String::new()),
    };
    crate::session_chrome::render_details(frame, details, &title, &detail);
    let mut hints = vec![
        ShortcutHint::new("↑↓", i18n.text("hub-hint-move")),
        ShortcutHint::new("←→", i18n.text("session-action-page")),
        ShortcutHint::new("PgUp/Dn", i18n.text("session-action-scroll")),
        ShortcutHint::new("Tab", i18n.text("hub-hint-section")),
    ];
    if search_active || !query.is_empty() {
        hints.push(ShortcutHint::new(
            "Esc",
            i18n.text("hub-action-clear-search"),
        ));
    }
    frame.render_widget(Paragraph::new(build_shortcut_line(hints)), footer);
}

fn hub_layout(area: Rect, has_error: bool) -> [Rect; 5] {
    let inner = Block::default().borders(Borders::ALL).inner(area);
    let details_height = if inner.height >= 12 { 2 } else { 0 };
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(u16::from(inner.height >= 8)),
        Constraint::Length(u16::from(has_error)),
        Constraint::Min(1),
        Constraint::Length(details_height),
        Constraint::Length(1),
    ])
    .split(inner);
    [rows[0], rows[1], rows[3], rows[4], rows[5]]
}

/// Geometry is shared by rendering and hit testing, including sticky column
/// headers. The panes retain independent scrolling without cutting normal
/// terminal widths into two cramped tables.
struct HubRegions {
    rows: Vec<(usize, Rect)>,
    divider: Option<Rect>,
    headers: Vec<(Rect, bool)>,
}

fn hub_row_regions(area: Rect, presentation: &SessionHubPresentation) -> HubRegions {
    let split = presentation
        .rows
        .iter()
        .position(|row| matches!(row, HubRow::Header(_)))
        .unwrap_or(presentation.rows.len());
    let mut regions = HubRegions {
        rows: Vec::new(),
        divider: None,
        headers: Vec::new(),
    };
    if area.height < 10 || split == 0 || split == presentation.rows.len() {
        let height = table_region(
            area,
            presentation,
            0..presentation.rows.len(),
            &presentation.directory_offset,
            split > 0,
            &mut regions,
        );
        presentation.directory_height.set(height);
        presentation.quick_height.set(height);
        return regions;
    }
    let (directories, divider, quick) = if area.width >= 196 {
        // Both tables need room for useful names plus status and time.
        let left_width = area.width / 2;
        let right_x = area.x + left_width + 1;
        let right_width = area.width.saturating_sub(left_width + 1);
        (
            Rect::new(area.x, area.y, left_width, area.height),
            Rect::new(right_x, area.y, right_width, 1),
            Rect::new(
                right_x,
                area.y + 1,
                right_width,
                area.height.saturating_sub(1),
            ),
        )
    } else {
        // Reserve one heading per table plus the activity divider. Favor the
        // project tree while giving unused rows back to either region.
        let available = area.height.saturating_sub(3);
        let quick_rows =
            u16::try_from(presentation.rows.len().saturating_sub(split)).unwrap_or(u16::MAX);
        let directory_rows = u16::try_from(split).unwrap_or(u16::MAX);
        let quick_height = quick_rows
            .min((available / 3).max(3))
            .max(available.saturating_sub(directory_rows))
            .min(available.saturating_sub(2));
        let directory_height = available.saturating_sub(quick_height) + 1;
        (
            Rect::new(area.x, area.y, area.width, directory_height),
            Rect::new(area.x, area.y + directory_height, area.width, 1),
            Rect::new(
                area.x,
                area.y + directory_height + 1,
                area.width,
                quick_height + 1,
            ),
        )
    };
    presentation.directory_height.set(table_region(
        directories,
        presentation,
        0..split,
        &presentation.directory_offset,
        true,
        &mut regions,
    ));
    presentation.quick_height.set(table_region(
        quick,
        presentation,
        split..presentation.rows.len(),
        &presentation.quick_offset,
        false,
        &mut regions,
    ));
    regions.divider = Some(divider);
    regions
}

fn table_region(
    area: Rect,
    presentation: &SessionHubPresentation,
    range: std::ops::Range<usize>,
    offset: &Cell<usize>,
    indented: bool,
    regions: &mut HubRegions,
) -> usize {
    let data = if area.height >= 3 {
        regions
            .headers
            .push((Rect::new(area.x, area.y, area.width, 1), indented));
        Rect::new(area.x, area.y + 1, area.width, area.height - 1)
    } else {
        area
    };
    visible_region(data, presentation, range, offset, &mut regions.rows);
    usize::from(data.height)
}

fn visible_region(
    area: Rect,
    presentation: &SessionHubPresentation,
    range: std::ops::Range<usize>,
    offset: &Cell<usize>,
    out: &mut Vec<(usize, Rect)>,
) {
    if range.is_empty() || area.height == 0 {
        return;
    }
    let mut first = offset.get().clamp(range.start, range.end - 1);
    if range.contains(&presentation.selection) {
        first = first.min(presentation.selection);
        let mut height = presentation.rows[first..=presentation.selection]
            .iter()
            .map(|row| usize::from(row.height()))
            .sum::<usize>();
        while first < presentation.selection && height > usize::from(area.height) {
            height -= usize::from(presentation.rows[first].height());
            first += 1;
        }
    }
    offset.set(first);
    let mut y = area.y;
    for index in first..range.end {
        if y >= area.bottom() {
            break;
        }
        let height = presentation.rows[index].height().min(area.bottom() - y);
        out.push((index, Rect::new(area.x, y, area.width, height)));
        y += height;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HubHit {
    pub row: usize,
    pub activate: bool,
}

pub fn hub_hit_test(
    area: Rect,
    presentation: &SessionHubPresentation,
    has_error: bool,
    column: u16,
    row: u16,
    i18n: &I18n,
) -> Option<HubHit> {
    let content = hub_layout(area, has_error)[2];
    let regions = hub_row_regions(content, presentation);
    if let Some(divider) = regions.divider
        && divider.contains((column, row).into())
    {
        let mut x = usize::from(divider.x);
        for section in presentation
            .sections
            .iter()
            .filter(|section| section.kind != SessionHubSectionKind::New)
        {
            // Width comes from the same terminal-aware text primitive used
            // by the renderer, including double-width directory languages.
            let width = Line::from(quick_bucket_label(section, i18n)).width();
            if x + width > usize::from(divider.right()) {
                break;
            }
            if usize::from(column) >= x && usize::from(column) < x + width {
                return presentation
                    .rows
                    .iter()
                    .position(|row| matches!(row, HubRow::Header(kind) if *kind == section.kind))
                    .map(|index| HubHit {
                        row: index + 1,
                        activate: false,
                    });
            }
            x += width + 3;
        }
    }
    regions.rows.into_iter().find_map(|(index, rect)| {
        (rect.contains((column, row).into()) && presentation.rows[index].identity().is_some())
            .then_some(HubHit {
                row: index,
                activate: true,
            })
    })
}

fn quick_bucket_label(section: &SessionHubSection, i18n: &I18n) -> String {
    format!(
        "{} {}",
        i18n.text(section.kind.localization_key()),
        section.items.len()
    )
}

fn render_rows(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &SessionHubPresentation,
    i18n: &I18n,
) {
    let regions = hub_row_regions(area, presentation);
    for (header, indented) in &regions.headers {
        let columns = HubColumns::new(header.width.saturating_sub(1));
        let mut line = columns.header(i18n, *indented);
        line.spans.insert(0, Span::raw(" "));
        frame.render_widget(
            Paragraph::new(line).style(muted_style().add_modifier(Modifier::BOLD)),
            *header,
        );
    }
    if let Some(divider) = regions.divider {
        if divider.x > area.x {
            for y in area.y..area.bottom() {
                frame.render_widget(
                    Paragraph::new("│").style(muted_style()),
                    Rect::new(divider.x - 1, y, 1, 1),
                );
            }
        }
        let buckets = presentation
            .sections
            .iter()
            .filter(|section| section.kind != SessionHubSectionKind::New)
            .map(|section| quick_bucket_label(section, i18n))
            .collect::<Vec<_>>()
            .join(" · ");
        frame.render_widget(
            Paragraph::new(buckets).style(
                Style::default()
                    .fg(accent_color())
                    .add_modifier(Modifier::BOLD),
            ),
            divider,
        );
    }
    for (index, area) in regions.rows {
        let row = &presentation.rows[index];
        let indented = !presentation.directories.is_empty() && presentation.same_region(index, 0);
        let item = match row {
            HubRow::Header(kind) | HubRow::DirectoryHeader { kind, .. } => {
                ListItem::new(Line::from(Span::styled(
                    {
                        let target = match row {
                            HubRow::DirectoryHeader {
                                workspace_id,
                                kind: SessionHubSectionKind::Recent,
                            } => HubPageTarget::Directory(*workspace_id),
                            HubRow::DirectoryHeader { workspace_id, kind } => {
                                HubPageTarget::DirectorySection(*workspace_id, *kind)
                            }
                            _ => HubPageTarget::Section(*kind),
                        };
                        let page = presentation.pagination.get(&target);
                        let count = page.and_then(|p| p.total).unwrap_or_else(|| {
                            if let HubRow::DirectoryHeader { workspace_id, .. } = row {
                                presentation
                                    .directories
                                    .iter()
                                    .find(|directory| directory.workspace_id == *workspace_id)
                                    .map_or(0, |directory| {
                                        if *kind == SessionHubSectionKind::Recent {
                                            directory.items.len()
                                        } else {
                                            directory
                                                .sections
                                                .iter()
                                                .find(|section| section.kind == *kind)
                                                .map_or(0, |section| section.items.len())
                                        }
                                    })
                            } else {
                                presentation
                                    .sections
                                    .iter()
                                    .find(|section| section.kind == *kind)
                                    .map_or(0, |section| section.items.len())
                            }
                        });
                        let suffix = page
                            .map(|p| {
                                format!(
                                    " · {}{}",
                                    agena_tui::sanitize_picker_text(&i18n.text_args(
                                        "session-bucket-page",
                                        &agena_tui::fl_args!("page" => (p.page + 1) as i64)
                                    )),
                                    if p.has_more { "+" } else { "" }
                                )
                            })
                            .unwrap_or_default();
                        format!(
                            "{}{}  {}{}",
                            if matches!(row, HubRow::DirectoryHeader { .. }) {
                                " "
                            } else {
                                ""
                            },
                            i18n.text(kind.localization_key()),
                            count,
                            suffix
                        )
                    },
                    Style::default()
                        .fg(accent_color())
                        .add_modifier(Modifier::BOLD),
                )))
            }
            HubRow::Item(item) => {
                ListItem::new(HubColumns::new(area.width.saturating_sub(1)).item(item, indented))
            }
            HubRow::Directory {
                path,
                expanded,
                count,
                ..
            } => {
                let path = agena_tui::sanitize_picker_text(path);
                let name = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&path);
                ListItem::new(Line::from(vec![
                    Span::styled(
                        format!("{} {}", if *expanded { "▾" } else { "▸" }, name),
                        Style::default()
                            .fg(accent_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(format!("  {count}"), muted_style()),
                ]))
            }
            HubRow::Page { page, forward, .. } => {
                ListItem::new(agena_tui::sanitize_picker_text(&i18n.text_args(
                    if *forward {
                        "hub-page-next"
                    } else {
                        "hub-page-previous"
                    },
                    &agena_tui::fl_args!("page" => (*page + 1) as i64),
                )))
            }
            HubRow::More { remaining, .. } => {
                ListItem::new(agena_tui::sanitize_picker_text(&i18n.text_args(
                    "hub-directory-more",
                    &agena_tui::fl_args!("count" => *remaining as i64),
                )))
            }
        };
        let list = List::new([item])
            .highlight_style(selection_style())
            .highlight_spacing(HighlightSpacing::Always)
            .highlight_symbol("›");
        let mut state = ListState::default();
        if index == presentation.selection {
            state.select(Some(0));
        }
        frame.render_stateful_widget(list, area, &mut state);
    }
}

#[cfg(test)]
mod tests;
