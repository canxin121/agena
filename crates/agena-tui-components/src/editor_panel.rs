//! Editor panel layout in workbenches.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    text::Text,
    widgets::{Block, Borders, Paragraph},
};

use crate::Editor;

/// Spec of the editor panel.
pub struct EditorPanelSpec<'a> {
    pub title: Option<Cow<'a, str>>,
    pub borders: Borders,
}

/// Render result of the editor panel.
pub struct EditorPanelRenderResult {
    pub cursor: (u16, u16),
}

pub fn render_editor_panel(
    frame: &mut Frame,
    area: Rect,
    spec: &EditorPanelSpec<'_>,
    input: &Editor,
) -> EditorPanelRenderResult {
    render_editor_panel_with_wrap(frame, area, spec, input, false)
}

/// Renders an editor panel whose long logical lines use terminal-width soft
/// wrapping. File and path entry use this so an entire current path remains
/// readable in a narrow terminal.
pub fn render_wrapped_editor_panel(
    frame: &mut Frame,
    area: Rect,
    spec: &EditorPanelSpec<'_>,
    input: &Editor,
) -> EditorPanelRenderResult {
    render_editor_panel_with_wrap(frame, area, spec, input, true)
}

fn render_editor_panel_with_wrap(
    frame: &mut Frame,
    area: Rect,
    spec: &EditorPanelSpec<'_>,
    input: &Editor,
    wrap: bool,
) -> EditorPanelRenderResult {
    let left_inset = u16::from(spec.borders.intersects(Borders::LEFT));
    let right_inset = u16::from(spec.borders.intersects(Borders::RIGHT));
    let top_inset = u16::from(spec.borders.intersects(Borders::TOP));
    let bottom_inset = u16::from(spec.borders.intersects(Borders::BOTTOM));
    let input_width = area
        .width
        .saturating_sub(left_inset)
        .saturating_sub(right_inset)
        .max(1);
    let input_height = area
        .height
        .saturating_sub(top_inset)
        .saturating_sub(bottom_inset)
        .max(1);
    let input_view = if wrap {
        input.render_wrapped_view(input_width, input_height)
    } else {
        input.render_view(input_width, input_height)
    };

    let mut paragraph = Paragraph::new(Text::from(input_view.lines.clone()));
    if spec.borders != Borders::NONE || spec.title.is_some() {
        let block = match spec.title.as_ref() {
            Some(title) => Block::default()
                .borders(spec.borders)
                .title(format!(" {} ", title)),
            None => Block::default().borders(spec.borders),
        };
        paragraph = paragraph.block(block);
    }
    frame.render_widget(paragraph, area);

    EditorPanelRenderResult {
        cursor: (
            area.x
                .saturating_add(left_inset)
                .saturating_add(input_view.cursor_x),
            area.y
                .saturating_add(top_inset)
                .saturating_add(input_view.cursor_y),
        ),
    }
}
