use std::borrow::Cow;

use ratatui::{Frame, layout::Rect};

use crate::{
    Editor, SurfaceMode,
    input_dialog::{InputDialogSpec, render_input_dialog},
};

pub struct EditorDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub prompt: Cow<'a, str>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub multiline: bool,
    pub prompt_height_bounds: (u16, u16),
    pub footer_height_bounds: (u16, u16),
}

pub fn render_editor_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &EditorDialogSpec<'_>,
    input: &Editor,
) {
    render_input_dialog(
        frame,
        area,
        surface,
        &InputDialogSpec {
            title: spec.title.as_ref(),
            prompt: spec.prompt.as_ref(),
            footer: spec.footer.as_ref(),
            target_width: spec.target_width,
            multiline: spec.multiline,
            prompt_height_bounds: spec.prompt_height_bounds,
            footer_height_bounds: spec.footer_height_bounds,
        },
        input,
    );
}

pub fn render_workbench_editor_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    title: Cow<'_, str>,
    prompt: Cow<'_, str>,
    footer: Cow<'_, str>,
    multiline: bool,
    input: &Editor,
) {
    render_editor_dialog(
        frame,
        area,
        surface,
        &EditorDialogSpec {
            title,
            prompt,
            footer,
            target_width: if multiline { 96 } else { 78 },
            multiline,
            prompt_height_bounds: (1, 3),
            footer_height_bounds: (1, 2),
        },
        input,
    );
}

pub fn render_overlay_line_input_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    title: Cow<'_, str>,
    prompt: Cow<'_, str>,
    footer: Cow<'_, str>,
    input: &Editor,
) {
    render_editor_dialog(
        frame,
        area,
        surface,
        &EditorDialogSpec {
            title,
            prompt,
            footer,
            target_width: 88,
            multiline: false,
            prompt_height_bounds: (1, 2),
            footer_height_bounds: (1, 1),
        },
        input,
    );
}
