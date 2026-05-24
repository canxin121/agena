use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Borders, Paragraph, Wrap},
};

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec,
    layout::{
        SurfaceMode, VerticalSectionSize, editor_input_panel_height, framed_sections_target_height,
        optional_overlay_text_height, split_vertical_sections,
    },
    render_editor_panel, render_framed_surface,
};

pub(crate) struct InputDialogSpec<'a> {
    pub title: &'a str,
    pub prompt: &'a str,
    pub footer: &'a str,
    pub target_width: u16,
    pub multiline: bool,
    pub prompt_height_bounds: (u16, u16),
    pub footer_height_bounds: (u16, u16),
}

pub(crate) fn render_input_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &InputDialogSpec<'_>,
    input: &Editor,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let prompt_height = optional_overlay_text_height(
        spec.prompt,
        content_width,
        spec.prompt_height_bounds.0,
        spec.prompt_height_bounds.1,
    );
    let footer_height = optional_overlay_text_height(
        spec.footer,
        content_width,
        spec.footer_height_bounds.0,
        spec.footer_height_bounds.1,
    );
    let input_height = editor_input_panel_height(input, spec.multiline);
    let mut sections = Vec::new();
    if prompt_height > 0 {
        sections.push(VerticalSectionSize::Fixed(prompt_height));
    }
    sections.push(VerticalSectionSize::Fixed(input_height));
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: spec.title.into(),
            target_width: spec.target_width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(frame_surface.inner, &sections);

    let mut row_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.prompt.to_string()).wrap(Wrap { trim: false }),
            rows[row_index],
        );
        row_index += 1;
    }

    let input_area = rows[row_index];
    row_index += 1;
    let result = render_editor_panel(
        frame,
        input_area,
        &EditorPanelSpec {
            title: None,
            borders: Borders::ALL,
        },
        input,
    );

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.footer.to_string()).wrap(Wrap { trim: false }),
            rows[row_index],
        );
    }

    frame.set_cursor_position(result.cursor);
}
