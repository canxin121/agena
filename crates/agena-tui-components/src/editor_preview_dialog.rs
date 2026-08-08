//! Editor preview dialog (rendered output).

use std::borrow::Cow;

use ratatui::{Frame, layout::Rect, text::Text, widgets::Borders};

use crate::{
    Editor, EditorPanelSpec, EditorSection, ParagraphSection, StackedDialogRenderResult,
    StackedDialogSection, StackedDialogSectionHeight, StackedDialogSpec, SurfaceMode,
    render_stacked_dialog,
};

pub struct EditorPreviewHelpSpec<'a> {
    pub body: &'a Text<'a>,
    pub height_bounds: (u16, u16),
    pub wrap: bool,
    pub borders: Borders,
}

impl<'a> EditorPreviewHelpSpec<'a> {
    pub fn new(
        body: &'a Text<'a>,
        height_bounds: (u16, u16),
        wrap: bool,
        borders: Borders,
    ) -> Self {
        Self {
            body,
            height_bounds,
            wrap,
            borders,
        }
    }
}

pub struct EditorPreviewDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub prompt: &'a Text<'a>,
    pub prompt_height_bounds: (u16, u16),
    pub help: Option<EditorPreviewHelpSpec<'a>>,
    pub input: &'a Editor,
    pub input_borders: Borders,
    pub preview: &'a Text<'a>,
    pub preview_height_bounds: (u16, u16),
    pub set_cursor: bool,
}

impl<'a> EditorPreviewDialogSpec<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: Cow<'a, str>,
        target_width: u16,
        prompt: &'a Text<'a>,
        prompt_height_bounds: (u16, u16),
        help: Option<EditorPreviewHelpSpec<'a>>,
        input: &'a Editor,
        input_borders: Borders,
        preview: &'a Text<'a>,
        preview_height_bounds: (u16, u16),
        set_cursor: bool,
    ) -> Self {
        Self {
            title,
            target_width,
            prompt,
            prompt_height_bounds,
            help,
            input,
            input_borders,
            preview,
            preview_height_bounds,
            set_cursor,
        }
    }
}

pub fn render_editor_preview_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &EditorPreviewDialogSpec<'_>,
) -> StackedDialogRenderResult {
    let mut sections = vec![StackedDialogSection::Paragraph(ParagraphSection {
        height: StackedDialogSectionHeight::AutoText {
            min: spec.prompt_height_bounds.0,
            max: spec.prompt_height_bounds.1,
        },
        title: None,
        borders: Borders::NONE,
        body: spec.prompt.clone(),
        wrap: false,
        scroll: None,
        alignment: None,
    })];

    if let Some(help) = spec.help.as_ref() {
        sections.push(StackedDialogSection::Paragraph(ParagraphSection {
            height: StackedDialogSectionHeight::AutoText {
                min: help.height_bounds.0,
                max: help.height_bounds.1,
            },
            title: None,
            borders: help.borders,
            body: help.body.clone(),
            wrap: help.wrap,
            scroll: None,
            alignment: None,
        }));
    }

    sections.push(StackedDialogSection::EditorPanel(EditorSection {
        height: StackedDialogSectionHeight::AutoEditor { multiline: false },
        spec: EditorPanelSpec {
            title: None,
            borders: spec.input_borders,
        },
        input: spec.input,
        set_cursor: spec.set_cursor,
    }));
    sections.push(StackedDialogSection::Paragraph(ParagraphSection {
        height: StackedDialogSectionHeight::AutoText {
            min: spec.preview_height_bounds.0,
            max: spec.preview_height_bounds.1,
        },
        title: None,
        borders: Borders::NONE,
        body: spec.preview.clone(),
        wrap: true,
        scroll: None,
        alignment: None,
    }));

    render_stacked_dialog(
        frame,
        area,
        surface,
        &StackedDialogSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            sections,
        },
    )
}
