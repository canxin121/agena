//! Session hub home screen: presentation state and rendering.
//!
//! The hub is the landing view of the TUI. It groups the sessions the server
//! reports as needing attention, currently running, and recently used, and
//! offers a create-new-session action. Like the other session surfaces in
//! this crate it owns only the display projection; the final application maps
//! selection to session-open effects and keeps response/error handling at the
//! application boundary.

use std::borrow::Cow;

use agena_tui::i18n::I18n;
use agena_tui_components::theme::{accent_color, danger_color, muted_style, selection_style};
use agena_tui_components::{
    ListPanelSpec, SectionedListFocus, SectionedListSection, SectionedListState, ShortcutHint,
    build_accented_two_line_list_item, build_shortcut_line, render_list_panel,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, ListItem, Paragraph},
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

/// Identity of a hub section. Ordering of the sections is fixed by the
/// renderer: new session (action), then running, then attention, then recent.
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

    pub fn empty_localization_key(self) -> &'static str {
        match self {
            Self::New => "hub-empty-new",
            Self::Running => "hub-empty-running",
            Self::Attention => "hub-empty-attention",
            Self::Recent => "hub-empty-recent",
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

impl SectionedListSection for SessionHubSection {
    type Item = SessionHubItem;

    fn items(&self) -> &[Self::Item] {
        &self.items
    }
}

/// Display projection of the session hub.
#[derive(Debug, Clone)]
pub struct SessionHubPresentation {
    pub state: SectionedListState<SessionHubSection>,
}

impl SessionHubPresentation {
    /// An empty hub in the given focus. The initial route is created before
    /// the first overview response lands, so it starts with no sections.
    pub fn empty(focus: SectionedListFocus) -> Self {
        Self {
            state: SectionedListState::new(Vec::new(), 0, 0, focus),
        }
    }

    /// Replace the section catalog while preserving the current selection and
    /// focus, then clamp the selection to the new data.
    pub fn set_sections(&mut self, sections: Vec<SessionHubSection>) {
        let selected_section = self.state.selected_section_index();
        let selected_item = self.state.selected_item_index();
        let focus = self.state.focus();
        self.state = SectionedListState::new(sections, selected_section, selected_item, focus);
    }

    /// Total number of selectable session rows across all sections.
    pub fn total_count(&self) -> usize {
        self.state.sections().iter().map(|section| section.items.len()).sum()
    }

    pub fn move_selection(&mut self, delta: isize) {
        self.state.move_selection(delta);
    }

    pub fn move_selection_page(&mut self, delta: isize, page_size: usize) {
        self.state.move_selection_page(delta, page_size);
    }

    pub fn move_selection_home(&mut self) {
        self.state.move_selection_home();
    }

    pub fn move_selection_end(&mut self) {
        self.state.move_selection_end();
    }

    /// Toggle between the section-navigation focus and the items focus.
    pub fn toggle_focus(&mut self) {
        let next = match self.state.focus() {
            SectionedListFocus::Navigation => SectionedListFocus::Items,
            SectionedListFocus::Items => SectionedListFocus::Navigation,
        };
        self.state.set_focus(next);
    }

    /// The session to open on Enter: only meaningful while focused on items
    /// and a row is selected.
    pub fn selected_session(&self) -> Option<&SessionHubItem> {
        if self.state.focus() != SectionedListFocus::Items {
            return None;
        }
        self.state.selected_item()
    }

    /// Keep the item selection off a section that has no rows when the focus
    /// switches to items, mirroring the permission studio's clamp behavior.
    pub fn clamp_selection(&mut self) {
        self.state.clamp_selection();
    }

    /// Filter the sections to rows matching `query` (case-insensitive substring
    /// over title/label/session id). The new-session action row always stays.
    /// Selection is preserved by identity when the selected row still matches,
    /// otherwise it lands on the first row.
    pub fn set_query(&mut self, query: &str) {
        let previous = self.selected_session().cloned();
        let query = query.trim().to_lowercase();
        let sections = self
            .state
            .sections()
            .iter()
            .map(|section| {
                let items = if query.is_empty() {
                    section.items.clone()
                } else {
                    section
                        .items
                        .iter()
                        .filter(|item| item.is_new_session || session_matches(item, &query))
                        .cloned()
                        .collect()
                };
                SessionHubSection::new(section.kind, items)
            })
            .collect::<Vec<_>>();
        let focus = self.state.focus();
        let (section_index, item_index) = match previous {
            Some(previous) => sections
                .iter()
                .enumerate()
                .find_map(|(section_index, section)| {
                    section
                        .items
                        .iter()
                        .position(|item| *item == previous)
                        .map(|item_index| (section_index, item_index))
                })
                .unwrap_or((0, 0)),
            None => (0, 0),
        };
        self.state = SectionedListState::new(sections, section_index, item_index, focus);
    }
}

/// Case-insensitive substring match of one hub row against the search query.
fn session_matches(item: &SessionHubItem, query_lower: &str) -> bool {
    item.label.to_lowercase().contains(query_lower)
        || item.title.to_lowercase().contains(query_lower)
        || item.session_id.to_string().contains(query_lower)
}

/// Renders the session hub home screen into `area`.
///
/// `loading` and `error` are application-owned response state: loading hides
/// the empty state while the first overview response is in flight, and an
/// error is drawn as a banner above (possibly stale) sections. `search_active`
/// is whether the user is composing a search (shown as an active filter box);
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
        Line::from(Span::styled(i18n.text("hub-search-placeholder"), muted_style()))
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

    let sections = presentation.state.sections();
    if sections.is_empty() {
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
        render_sections(frame, content_area, presentation, i18n);
    }

    frame.render_widget(
        Paragraph::new(build_shortcut_line([
            ShortcutHint::new("↑/↓", i18n.text("hub-hint-move")),
            ShortcutHint::new("Tab", i18n.text("hub-hint-focus")),
            ShortcutHint::new("Enter", i18n.text("hub-hint-open")),
            ShortcutHint::new("Esc", i18n.text("hub-hint-back")),
        ])),
        footer_area,
    );
}

fn render_sections(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &SessionHubPresentation,
    i18n: &I18n,
) {
    let sections = presentation.state.sections();
    let focus = presentation.state.focus();
    let selected_section = presentation.state.selected_section_index();
    let selected_item = presentation.state.selected_item_index();

    let section_areas = split_section_areas(area, sections);
    for (index, section) in sections.iter().enumerate() {
        let section_area = section_areas
            .get(index)
            .copied()
            .unwrap_or(Rect::ZERO);
        let is_selected = index == selected_section;
        let navigation_selected = is_selected && focus == SectionedListFocus::Navigation;
        let item_selected =
            is_selected && focus == SectionedListFocus::Items;
        render_section(
            frame,
            section_area,
            section,
            i18n.text(section.kind.localization_key()),
            i18n.text(section.kind.empty_localization_key()),
            navigation_selected,
            if item_selected { Some(selected_item) } else { None },
        );
    }
}

fn split_section_areas(area: Rect, sections: &[SessionHubSection]) -> Vec<Rect> {
    if sections.is_empty() || area.height == 0 {
        return Vec::new();
    }
    // The new-session action section is always a single fixed-height row at
    // the top; every other section shares the remaining space proportionally
    // to its row count so non-empty sections never get starved.
    let (new_index, new_area, remaining) = match sections.iter().position(|section| {
        section.kind == SessionHubSectionKind::New && !section.items.is_empty()
    }) {
        Some(index) => {
            let new_area = Rect {
                x: area.x,
                y: area.y,
                width: area.width,
                height: 1.min(area.height),
            };
            let remaining = Rect {
                x: area.x,
                y: area.y + 1,
                width: area.width,
                height: area.height.saturating_sub(1),
            };
            (Some(index), new_area, remaining)
        }
        None => (None, Rect::ZERO, area),
    };
    let mut areas = vec![Rect::ZERO; sections.len()];
    if let Some(index) = new_index {
        areas[index] = new_area;
    }
    if remaining.height == 0 {
        return areas;
    }
    let total = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if Some(index) == new_index {
                0
            } else {
                section.items.len().max(1) as u32
            }
        })
        .sum::<u32>()
        .max(1);
    let constraints = sections
        .iter()
        .enumerate()
        .map(|(index, section)| {
            if Some(index) == new_index {
                Constraint::Length(1)
            } else {
                Constraint::Ratio(section.items.len().max(1) as u32, total)
            }
        })
        .collect::<Vec<_>>();
    let rest_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(remaining)
        .to_vec();
    for (index, area) in rest_areas.into_iter().enumerate() {
        if areas[index] == Rect::ZERO {
            areas[index] = area;
        }
    }
    areas
}

fn render_section(
    frame: &mut Frame<'_>,
    area: Rect,
    section: &SessionHubSection,
    title: String,
    empty_message: String,
    navigation_selected: bool,
    item_selected: Option<usize>,
) {
    if area.width == 0 || area.height == 0 {
        return;
    }
    if section.items.is_empty() {
        let border_style = if navigation_selected {
            Style::default().fg(accent_color())
        } else {
            Style::default()
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(format!(" {} ", title));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(empty_message, muted_style()))),
            inner,
        );
        return;
    }

    let items = section
        .items
        .iter()
        .map(|item| {
            build_accented_two_line_list_item(
                Cow::Borrowed(item.label.as_str()),
                None,
                Some(Cow::Borrowed(item.detail.as_str())),
            )
        })
        .collect::<Vec<ListItem<'_>>>();

    let highlight_style = if item_selected.is_some() {
        selection_style()
    } else if navigation_selected {
        Style::default()
            .fg(accent_color())
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
    };

    let spec = ListPanelSpec::new(
        Some(Cow::Owned(title)),
        &items,
        item_selected,
        highlight_style,
        Cow::Borrowed("> "),
    );
    render_list_panel(frame, area, &spec);
}

#[cfg(test)]
mod tests {
    use super::{
        SessionHubItem, SessionHubPresentation, SessionHubSection, SessionHubSectionKind,
    };
    use agena_tui_components::SectionedListFocus;

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
    fn set_sections_preserves_selection_and_focus() {
        let mut presentation = SessionHubPresentation::empty(SectionedListFocus::Items);
        presentation.set_sections(vec![
            section(SessionHubSectionKind::Attention, &[1, 2]),
            section(SessionHubSectionKind::Running, &[3]),
        ]);
        assert_eq!(presentation.total_count(), 3);
        assert_eq!(
            presentation.state.selected_section_index(),
            0,
            "selection lands on the first non-empty section by default"
        );
        assert_eq!(presentation.state.focus(), SectionedListFocus::Items);

        // Move to the second section's only item, then reload with fewer rows.
        presentation.state.set_focus(SectionedListFocus::Navigation);
        presentation.state.move_selection(1);
        presentation.state.set_focus(SectionedListFocus::Items);
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(3));

        presentation.set_sections(vec![section(SessionHubSectionKind::Recent, &[9])]);
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(9));
    }

    #[test]
    fn selected_session_requires_items_focus() {
        let mut presentation = SessionHubPresentation::empty(SectionedListFocus::Navigation);
        presentation.set_sections(vec![
            section(SessionHubSectionKind::Attention, &[1]),
            section(SessionHubSectionKind::Running, &[]),
            section(SessionHubSectionKind::Recent, &[2]),
        ]);
        assert_eq!(presentation.selected_session(), None);
        presentation.toggle_focus();
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(1));
    }

    #[test]
    fn query_filters_rows_and_keeps_the_new_session_action() {
        let mut presentation = SessionHubPresentation::empty(SectionedListFocus::Items);
        let mut sections = vec![
            SessionHubSection::new(
                SessionHubSectionKind::New,
                vec![new_session_item()],
            ),
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Attention, &[3]),
            section(SessionHubSectionKind::Recent, &[4]),
        ];
        // A synthetic new-session row placed outside the New section must
        // survive filtering too: only the New section emits it today, but the
        // filter treats action rows as always-visible.
        sections.push(SessionHubSection::new(
            SessionHubSectionKind::Recent,
            vec![SessionHubItem {
                is_new_session: true,
                ..item(5)
            }],
        ));
        presentation.set_sections(sections);

        presentation.set_query("session 3");

        let ids: Vec<i64> = presentation
            .state
            .sections()
            .iter()
            .flat_map(|section| section.items.iter().map(|row| row.session_id))
            .collect();
        assert_eq!(
            ids,
            vec![0, 3, 5],
            "the new-session action row, the matching row, and any action row all survive"
        );
        // Selection stays on the previously selected (new-session) row.
        assert!(
            presentation
                .selected_session()
                .map(|row| row.is_new_session)
                .unwrap_or(false)
        );
    }

    #[test]
    fn query_selection_survives_by_identity_when_selected_row_still_matches() {
        let mut presentation = SessionHubPresentation::empty(SectionedListFocus::Items);
        presentation.set_sections(vec![
            section(SessionHubSectionKind::Running, &[1, 2]),
            section(SessionHubSectionKind::Attention, &[3]),
        ]);
        // Select the second row of the Running section.
        presentation.move_selection(1);
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(2));
        // Filtering to "session 2" keeps row 2 selected even though the list
        // shrank around it.
        presentation.set_query("session 2");
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(2));
        // Clearing the query restores the full list and keeps row 2 selected.
        presentation.set_query("");
        assert_eq!(presentation.selected_session().map(|s| s.session_id), Some(2));
    }
}
