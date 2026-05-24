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
    pub fn new(body: &'a Text<'a>, height_bounds: (u16, u16)) -> Self {
        Self {
            body,
            height_bounds,
            wrap: true,
            borders: Borders::NONE,
        }
    }

    pub fn with_wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    pub fn with_borders(mut self, borders: Borders) -> Self {
        self.borders = borders;
        self
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
    pub fn new(
        title: Cow<'a, str>,
        target_width: u16,
        prompt: &'a Text<'a>,
        prompt_height_bounds: (u16, u16),
        input: &'a Editor,
        preview: &'a Text<'a>,
        preview_height_bounds: (u16, u16),
    ) -> Self {
        Self {
            title,
            target_width,
            prompt,
            prompt_height_bounds,
            help: None,
            input,
            input_borders: Borders::ALL,
            preview,
            preview_height_bounds,
            set_cursor: false,
        }
    }

    pub fn with_help(mut self, help: EditorPreviewHelpSpec<'a>) -> Self {
        self.help = Some(help);
        self
    }

    pub fn with_input_borders(mut self, input_borders: Borders) -> Self {
        self.input_borders = input_borders;
        self
    }

    pub fn with_cursor(mut self, set_cursor: bool) -> Self {
        self.set_cursor = set_cursor;
        self
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

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use ratatui::{text::Text, widgets::Borders};

    use super::{EditorPreviewDialogSpec, EditorPreviewHelpSpec};
    use crate::Editor;

    #[test]
    fn editor_preview_dialog_spec_builder_preserves_help_and_cursor() {
        let prompt = Text::from("prompt");
        let help = Text::from("help");
        let preview = Text::from("preview");
        let editor = Editor::from_text("value".to_string());
        let spec = EditorPreviewDialogSpec::new(
            Cow::Borrowed("Title"),
            80,
            &prompt,
            (1, 2),
            &editor,
            &preview,
            (2, 8),
        )
        .with_help(
            EditorPreviewHelpSpec::new(&help, (3, 6))
                .with_wrap(true)
                .with_borders(Borders::BOTTOM),
        )
        .with_input_borders(Borders::BOTTOM)
        .with_cursor(true);

        assert_eq!(spec.title.as_ref(), "Title");
        assert!(spec.help.is_some());
        assert_eq!(spec.input_borders, Borders::BOTTOM);
        assert!(spec.set_cursor);
    }
}
