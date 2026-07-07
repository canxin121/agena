use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    text::Text,
    widgets::{Block, Borders, Paragraph},
};

use crate::Editor;

pub struct EditorPanelSpec<'a> {
    pub title: Option<Cow<'a, str>>,
    pub borders: Borders,
}

pub struct EditorPanelRenderResult {
    pub cursor: (u16, u16),
}

pub fn render_editor_panel(
    frame: &mut Frame,
    area: Rect,
    spec: &EditorPanelSpec<'_>,
    input: &Editor,
) -> EditorPanelRenderResult {
    let left_inset = u16::from(spec.borders.intersects(Borders::LEFT));
    let right_inset = u16::from(spec.borders.intersects(Borders::RIGHT));
    let top_inset = u16::from(spec.borders.intersects(Borders::TOP));
    let bottom_inset = u16::from(spec.borders.intersects(Borders::BOTTOM));
    let input_view = input.render_view(
        area.width
            .saturating_sub(left_inset)
            .saturating_sub(right_inset)
            .max(1),
        area.height
            .saturating_sub(top_inset)
            .saturating_sub(bottom_inset)
            .max(1),
    );

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
