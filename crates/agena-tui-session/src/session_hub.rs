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
#[derive(Debug, Clone)]
pub struct SessionHubItem {
    pub session_id: i64,
    pub title: String,
    /// First line of the row; the renderer never invents text, so the App
    /// pre-localizes it.
    pub label: String,
    /// Second, muted line of the row (state, message count, recency, ...).
    pub detail: String,
}

/// Identity of a hub section. Ordering of the sections is fixed by the
/// renderer: attention, then running, then recent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionHubSectionKind {
    Attention,
    Running,
    Recent,
}

impl SessionHubSectionKind {
    pub fn localization_key(self) -> &'static str {
        match self {
            Self::Attention => "hub-section-attention",
            Self::Running => "hub-section-running",
            Self::Recent => "hub-section-recent",
        }
    }

    pub fn empty_localization_key(self) -> &'static str {
        match self {
            Self::Attention => "hub-empty-attention",
            Self::Running => "hub-empty-running",
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
}

/// Renders the session hub home screen into `area`.
///
/// `loading` and `error` are application-owned response state: loading hides
/// the empty state while the first overview response is in flight, and an
/// error is drawn as a banner above (possibly stale) sections.
pub fn render_session_hub(
    frame: &mut Frame<'_>,
    area: Rect,
    presentation: &SessionHubPresentation,
    loading: bool,
    error: Option<&str>,
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
            Constraint::Min(1),
            Constraint::Length(1),
        ])
        .split(inner);
    let action_area = rows[0];
    let mut content_area = rows[1];
    let footer_area = rows[2];

    frame.render_widget(
        Paragraph::new(build_shortcut_line([
            ShortcutHint::new("Ctrl+N", i18n.text("hub-action-create")),
            ShortcutHint::new("l", i18n.text("hub-action-list")),
            ShortcutHint::new("Ctrl+R", i18n.text("hub-action-refresh")),
        ])),
        action_area,
    );

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
    let total = sections
        .iter()
        .map(|section| section.items.len().max(1) as u16)
        .sum::<u16>()
        .max(1);
    let constraints = sections
        .iter()
        .map(|section| Constraint::Ratio(section.items.len().max(1) as u16, total))
        .collect::<Vec<_>>();
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area)
        .to_vec()
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
}
