//! Session hub home screen: presentation state and rendering.
//!
//! The hub is the landing view of the TUI. It groups the sessions the server
//! reports as needing attention, currently running, and recently used, and
//! offers a create-new-session action. It renders as a single list box whose
//! first row is the "+ new session" action; each non-empty bucket appears below
//! it as an in-list header followed by its rows. Like the other session
//! surfaces in this crate it owns only the display projection; the final
//! application maps selection to session-open effects and keeps
//! response/error handling at the application boundary.

use std::borrow::Cow;

use agena_tui::i18n::I18n;
use agena_tui_components::theme::{accent_color, danger_color, muted_style, selection_style};
use agena_tui_components::{ShortcutHint, build_accented_two_line_list_item, build_shortcut_line};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

/// A session entry shown in a hub section.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHubItem {
    pub session_id: i64,
    pub title: String,
    /// First line of the row; the renderer never invents text, so the App
    /// pre-localizes it.
    pub label: String,
    /// Second, muted line of the row (state, message count, recency, ...).
    pub detail: String,
    /// True for the synthetic "new session" action row the App places at the
    /// very top of the hub. Entering on it creates a fresh session instead of
    /// opening an existing one.
    pub is_new_session: bool,
}

/// Identity of a hub section. Ordering of the sections is fixed by the App:
/// new session (action), then running, then attention, then recent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHubSectionKind {
    New,
    Running,
    Attention,
    Recent,
}

impl SessionHubSectionKind {
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::New => "hub-section-new",
            Self::Running => "hub-section-running",
            Self::Attention => "hub-section-attention",
            Self::Recent => "hub-section-recent",
        }
    }
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
    /// Section label line drawn inside the single hub box. Never selectable.
    Header(SessionHubSectionKind),
    /// A selectable session row (including the "+ new session" action row).
    Item(SessionHubItem),
}

/// Display projection of the session hub.
///
/// The hub is one flat list. The new-session action rows come first and carry
/// no header — the create action is the first option of the hub — and every
/// other section that has items appears as a header line followed by its rows.
/// Empty sections are dropped entirely, so an empty bucket can never sit
/// between the user and the recent sessions. Navigation moves over the
/// selectable rows only; Tab / Shift+Tab jump between sections.
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
        }
    }

    /// Replace the section catalog and re-derive the visible rows, preserving
    /// the selected row by identity when it survives the rebuild.
    pub fn set_sections(&mut self, sections: Vec<SessionHubSection>) {
        self.sections = sections;
        self.rebuild_rows();
    }

    /// Filter the sections to rows matching `query` (case-insensitive substring
    /// over title/label/session id). The new-session action row always stays.
    /// Selection is preserved by identity when the selected row still matches,
    /// otherwise it lands on the first row.
    pub fn set_query(&mut self, query: &str) {
        self.query = query.trim().to_lowercase();
        self.rebuild_rows();
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
        let previous = self.selected_item().cloned();
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
        self.selection = previous
            .and_then(|previous| {
                self.rows
                    .iter()
                    .position(|row| matches!(row, HubRow::Item(item) if *item == previous))
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
                HubRow::Item(_) => Some(index),
                HubRow::Header(_) => None,
            })
            .collect()
    }

    /// For every visible section, `(kind, flat row of its first item, item
    /// count)`. The new-session rows come first and carry no header, so they
    /// form the initial span.
    fn section_spans(&self) -> Vec<(SessionHubSectionKind, usize, usize)> {
        let mut spans: Vec<(SessionHubSectionKind, usize, usize)> = Vec::new();
        for (index, row) in self.rows.iter().enumerate() {
            match row {
                HubRow::Header(kind) => spans.push((*kind, index + 1, 0)),
                HubRow::Item(_) => match spans.last_mut() {
                    Some(span) => span.2 += 1,
                    None => spans.push((SessionHubSectionKind::New, index, 1)),
                },
            }
        }
        spans
    }
}

/// Case-insensitive substring match of one hub row against the search query.
fn session_matches(item: &SessionHubItem, query_lower: &str) -> bool {
    item.label.to_lowercase().contains(query_lower)
        || item.title.to_lowercase().contains(query_lower)
        || item.session_id.to_string().contains(query_lower)
}

/// Flatten sections into display rows. The new-session action rows are first
/// and carry no header — the create action is the first option of the hub —
/// and every other section that has items appears as a header line followed by
/// its rows. Empty sections are dropped entirely so an empty bucket can never
/// sit between the user and the recent sessions.
fn build_rows(sections: Vec<SessionHubSection>) -> Vec<HubRow> {
    let mut rows = Vec::new();
    for section in sections {
        if section.items.is_empty() {
            continue;
        }
        if section.kind != SessionHubSectionKind::New {
            rows.push(HubRow::Header(section.kind));
        }
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
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", i18n.text("hub-title")));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let action_area = rows[0];
    let search_area = rows[1];
    let mut content_area = rows[2];
    let footer_area = rows[3];

    let action = if search_active {
        build_shortcut_line([ShortcutHint::new(
            "Esc",
            i18n.text("hub-action-clear-search"),
        )])
    } else {
        build_shortcut_line([
            ShortcutHint::new("Ctrl+N", i18n.text("hub-action-create")),
            ShortcutHint::new("/", i18n.text("hub-action-search")),
            ShortcutHint::new("l", i18n.text("hub-action-list")),
            ShortcutHint::new("Ctrl+R", i18n.text("hub-action-refresh")),
        ])
    };
    frame.render_widget(Paragraph::new(action), action_area);

    let search_title = if search_active {
        let prompt = if query.trim().is_empty() {
            i18n.text("hub-search-active-empty")
        } else {
            i18n.text_args("hub-search-active", &agena_tui::fl_args!("query" => query))
        };
        Line::from(Span::styled(prompt, accent_color()))
    } else {
        Line::from(Span::styled(
            i18n.text("hub-search-placeholder"),
            muted_style(),
        ))
    };
    frame.render_widget(Paragraph::new(search_title), search_area);

    if let Some(error) = error {
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(content_area);
        let sanitized = agena_tui::sanitize_picker_text(error);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                sanitized,
                Style::default().fg(danger_color()),
            ))),
            rows[0],
        );
        content_area = rows[1];
    }

    if presentation.rows().is_empty() {
        let message = if loading {
            i18n.text("overlay-picker-loading")
        } else {
            i18n.text("hub-empty")
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(message, muted_style()))),
            content_area,
        );
    } else {
        render_rows(frame, content_area, presentation, i18n);
    }

    frame.render_widget(
        Paragraph::new(build_shortcut_line([
            ShortcutHint::new("↑/↓", i18n.text("hub-hint-move")),
            ShortcutHint::new("Tab", i18n.text("hub-hint-section")),
            ShortcutHint::new("Enter", i18n.text("hub-hint-open")),
            ShortcutHint::new("Esc", i18n.text("hub-hint-back")),
        ])),
        footer_area,
    );
}

/// Draws the single hub list: section headers and session rows inside one box.
fn render_rows(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &SessionHubPresentation,
    i18n: &I18n,
) {
    let mut list_items = Vec::with_capacity(presentation.rows().len());
    for row in presentation.rows() {
        let item = match row {
            HubRow::Header(kind) => ListItem::new(Line::from(Span::styled(
                format!(" {} ", i18n.text(kind.localization_key())),
                Style::default()
                    .fg(accent_color())
                    .add_modifier(Modifier::BOLD),
            ))),
            HubRow::Item(item) => build_accented_two_line_list_item(
                Cow::Borrowed(item.label.as_str()),
                None,
                Some(Cow::Borrowed(item.detail.as_str())),
            ),
        };
        list_items.push(item);
    }
    let list = List::new(list_items)
        .highlight_style(selection_style())
        .highlight_symbol("> ");
    // `selected` is the ITEM index, not a line offset; the List widget itself
    // scrolls to the selected item (honoring multi-line item heights).
    let mut state = ListState::default();
    state.select(Some(presentation.selection()));
    frame.render_stateful_widget(list, area, &mut state);
}

#[cfg(test)]
mod tests {
    use super::{
        HubRow, SessionHubItem, SessionHubPresentation, SessionHubSection, SessionHubSectionKind,
    };

    fn item(id: i64) -> SessionHubItem {
        SessionHubItem {
            session_id: id,
            title: format!("session {id}"),
            label: format!("session {id}"),
            detail: "detail".to_string(),
            is_new_session: false,
        }
    }

    fn new_session_item() -> SessionHubItem {
        SessionHubItem {
            session_id: 0,
            title: String::new(),
            label: "new session".to_string(),
            detail: String::new(),
            is_new_session: true,
        }
    }

    fn section(kind: SessionHubSectionKind, ids: &[i64]) -> SessionHubSection {
        SessionHubSection::new(kind, ids.iter().copied().map(item).collect())
    }

    #[test]
    fn builds_one_box_with_new_session_first_and_hides_empty_sections() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[]), // empty → hidden
            section(SessionHubSectionKind::Attention, &[3]),
            section(SessionHubSectionKind::Recent, &[4]),
        ]);
        let rows = presentation.rows();
        assert_eq!(
            rows.len(),
            5,
            "new row + attention (header+row) + recent (header+row)"
        );
        assert_eq!(rows[0], HubRow::Item(new_session_item()));
        assert_eq!(rows[1], HubRow::Header(SessionHubSectionKind::Attention));
        assert_eq!(rows[2], HubRow::Item(item(3)));
        assert_eq!(rows[3], HubRow::Header(SessionHubSectionKind::Recent));
        assert_eq!(rows[4], HubRow::Item(item(4)));
        // No Running header — the empty bucket was dropped.
        assert!(!rows.contains(&HubRow::Header(SessionHubSectionKind::Running)));
        // Selection lands on the first row: the create action.
        assert!(
            presentation
                .selected_item()
                .map(|r| r.is_new_session)
                .unwrap_or(false)
        );
        assert_eq!(presentation.total_count(), 3);
    }

    #[test]
    fn arrows_cross_sections_and_skip_headers() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Recent, &[3]),
        ]);
        // rows: new, H(running), 1, 2, H(recent), 3
        presentation.move_selection(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(1));
        presentation.move_selection(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(2));
        // Crosses the recent header without stopping on it.
        presentation.move_selection(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(3));
        // Clamped at the end.
        presentation.move_selection(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(3));
        // Back over the running section.
        presentation.move_selection(-2);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(1));
    }

    #[test]
    fn tab_jumps_between_groups_and_wraps() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Attention, &[]), // empty → skipped
            section(SessionHubSectionKind::Recent, &[3]),
        ]);
        // Tab from the create row lands on the first running session.
        presentation.move_selection_section(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(1));
        // Tab again jumps past the empty attention section to recent.
        presentation.move_selection_section(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(3));
        // Wraps back to the create row.
        presentation.move_selection_section(1);
        assert!(
            presentation
                .selected_item()
                .map(|s| s.is_new_session)
                .unwrap_or(false)
        );
        // Shift+Tab walks back: recent → running → create.
        presentation.move_selection_section(-1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(3));
        presentation.move_selection_section(-1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(1));
    }

    #[test]
    fn tab_reaches_recent_when_running_and_attention_are_empty() {
        // Regression: with no running/attention sessions the user must still be
        // able to Tab straight to the recent sessions.
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[]),
            section(SessionHubSectionKind::Attention, &[]),
            section(SessionHubSectionKind::Recent, &[7, 8]),
        ]);
        presentation.move_selection_section(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(7));
    }

    #[test]
    fn query_filters_rows_and_keeps_the_new_session_action() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Attention, &[3]),
            section(SessionHubSectionKind::Recent, &[4]),
        ]);
        presentation.set_query("session 3");
        // Visible rows: the create action + header(attention) + the match.
        assert_eq!(presentation.total_count(), 2);
        assert_eq!(presentation.rows().len(), 3);
        // Selection stays on the create row by identity.
        assert!(
            presentation
                .selected_item()
                .map(|r| r.is_new_session)
                .unwrap_or(false)
        );
        // Down lands on the matching row.
        presentation.move_selection(1);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(3));
    }

    #[test]
    fn query_selection_survives_by_identity_when_selected_row_still_matches() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![
            SessionHubSection::new(SessionHubSectionKind::New, vec![new_session_item()]),
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Recent, &[3]),
        ]);
        // Select the second running row.
        presentation.move_selection(2);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(2));
        // Filtering to "session 2" keeps row 2 selected even though the list
        // shrank around it.
        presentation.set_query("session 2");
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(2));
        // Clearing the query restores the full list and keeps row 2 selected.
        presentation.set_query("");
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(2));
    }

    #[test]
    fn selection_clamps_to_nearest_item_when_sections_shrink() {
        let mut presentation = SessionHubPresentation::empty();
        presentation.set_sections(vec![section(SessionHubSectionKind::Running, &[1, 2, 3, 4])]);
        presentation.move_selection_end();
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(4));
        // The previously selected session is gone after a refresh; selection
        // clamps to the nearest row of the new list.
        presentation.set_sections(vec![section(SessionHubSectionKind::Recent, &[9])]);
        assert_eq!(presentation.selected_item().map(|s| s.session_id), Some(9));
    }
}
