use super::SessionSearchOverlay;
use agena_tui::{i18n::I18n, sanitize_picker_text};
use agena_tui_components::{
    EditorPanelSpec, SearchPickerFocus, SearchPickerPhase,
    pointer::{self, PointerAction},
    render_editor_panel, theme,
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::Style,
    text::{Line, Span},
    widgets::{Block, Borders, Clear, Paragraph},
};

/// Session browsing uses the available route height, with a single search
/// line and single-line records. The generic picker still owns selection,
/// identity, focus, and pagination; its other callers keep their own chrome.
pub fn render_overlay(
    frame: &mut Frame<'_>,
    area: Rect,
    dialog: &SessionSearchOverlay,
    i18n: &I18n,
) {
    pointer::begin_surface();
    frame.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" {} ", sanitize_picker_text(&dialog.title)));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(if inner.height >= 12 { 2 } else { 0 }),
        Constraint::Length(1),
    ])
    .split(inner);
    let input_active = dialog.focus == SearchPickerFocus::Input;
    let label = format!(" {} ", i18n.text("hub-action-search"));
    let label_width = (Line::from(label.as_str()).width() as u16).min(rows[0].width);
    frame.render_widget(
        Paragraph::new(label).style(if input_active {
            Style::default().fg(theme::accent_color())
        } else {
            theme::muted_style()
        }),
        rows[0],
    );
    let input_area = Rect::new(
        rows[0].x + label_width,
        rows[0].y,
        rows[0].width.saturating_sub(label_width),
        rows[0].height,
    );
    pointer::register(rows[0], Some(PointerAction::PickerInput), None);
    let input = render_editor_panel(
        frame,
        input_area,
        &EditorPanelSpec {
            title: None,
            borders: Borders::NONE,
        },
        &dialog.input,
    );
    if input_active && !input_area.is_empty() {
        frame.set_cursor_position(input.cursor);
    }
    if dialog.input.text().is_empty() && !input_active {
        frame.render_widget(
            Paragraph::new(i18n.text("session-search-guide")).style(theme::muted_style()),
            input_area,
        );
    }

    let selected = dialog.selected_item();
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
    let previous = if inner.width < 72 {
        "←".into()
    } else {
        format!("← {}", i18n.text("session-page-previous"))
    };
    let next = if inner.width < 72 {
        "→".into()
    } else {
        format!("→ {}", i18n.text("session-page-next"))
    };
    let alt = |c| PointerAction::Key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT));
    pointer::render_buttons(
        frame,
        rows[1],
        &[
            (&pin, alt('p')),
            (&favorite, alt('f')),
            (&previous, pointer::key(KeyCode::PageUp)),
            (&next, pointer::key(KeyCode::PageDown)),
        ],
    );

    dialog.set_visible_page_size(usize::from(rows[2].height));
    let (start, end) = dialog.visible_page_bounds();
    let hides_results = matches!(
        dialog.phase,
        SearchPickerPhase::Searching { .. }
            | SearchPickerPhase::Error {
                keep_results: false
            }
    );
    if hides_results || dialog.items.is_empty() {
        let message = if dialog.is_loading() {
            i18n.text("overlay-picker-loading")
        } else if let Some(error) = &dialog.error_message {
            sanitize_picker_text(error)
        } else {
            i18n.text("overlay-resume-empty")
        };
        frame.render_widget(Paragraph::new(message).style(theme::muted_style()), rows[2]);
    } else {
        pointer::register(rows[2], Some(PointerAction::PickerResults), None);
        // Session search uses externally filtered/ordered items: row indices
        // and item indices are identical, including locally filtered subtrees.
        for (line, index) in (start..end).enumerate().take(usize::from(rows[2].height)) {
            let Some(item) = dialog.items.get(index) else {
                break;
            };
            let rect = Rect::new(rows[2].x, rows[2].y + line as u16, rows[2].width, 1);
            let selected = index == dialog.selected;
            let style = if selected {
                theme::selection_style()
            } else {
                Style::default()
            };
            let mut text = crate::session_chrome::session_line(
                rect.width.saturating_sub(2),
                &item.label,
                &item.summary,
                item.pinned,
                item.favorite,
                false,
            );
            text.spans
                .insert(0, Span::raw(if selected { "› " } else { "  " }));
            if selected {
                // Keep secondary text and markers legible on the selection
                // background in both terminal color schemes.
                for span in &mut text.spans {
                    span.style = span.style.patch(style);
                }
            }
            frame.render_widget(Paragraph::new(text).style(style), rect);
            pointer::register(rect, Some(PointerAction::PickerRow(index)), None);
        }
    }
    let page = i18n.text_args(
        "session-page-status",
        &agena_tui::fl_args!(
            "page" => (dialog.current_page() + 1) as i64,
            "pages" => dialog.page_count() as i64,
            "more" => if dialog.meta.has_more { "+" } else { "" },
            "count" => dialog.items.len() as i64,
        ),
    );
    let status = if dialog.is_loading() {
        format!("{page} · … {}", i18n.text("overlay-picker-loading"))
    } else if let Some(error) = &dialog.error_message {
        format!(
            "^R {} · {}",
            i18n.text("hub-action-refresh"),
            sanitize_picker_text(error)
        )
    } else {
        page
    };
    frame.render_widget(
        Paragraph::new(sanitize_picker_text(&status)).style(theme::muted_style()),
        rows[3],
    );
    if !hides_results && let Some(item) = dialog.selected_item() {
        crate::session_chrome::render_details(frame, rows[4], &item.label, &item.detail);
    }
    frame.render_widget(
        Paragraph::new(i18n.text(if input_active {
            "session-search-input-hints"
        } else if inner.width < 72 {
            "session-search-compact-hints"
        } else {
            "session-search-results-hints"
        }))
        .style(theme::muted_style()),
        rows[5],
    );
}
