use std::borrow::Cow;

use ratatui::{Frame, layout::Rect, text::Text, widgets::Borders};

use crate::{
    Editor, EditorPanelSpec, EditorSection, ParagraphSection, StackedDialogRenderResult,
    StackedDialogSection, StackedDialogSectionHeight, StackedDialogSpec, SurfaceMode,
    TextPanelSection, TextPanelSpec, render_stacked_dialog,
};

pub struct QuestionFlowCustomInputSpec<'a> {
    pub title: Cow<'a, str>,
    pub input: &'a Editor,
    pub editing: bool,
}

impl<'a> QuestionFlowCustomInputSpec<'a> {
    pub fn new(title: Cow<'a, str>, input: &'a Editor, editing: bool) -> Self {
        Self {
            title,
            input,
            editing,
        }
    }
}

pub enum QuestionFlowDialogMode<'a> {
    Empty {
        detail_title: Cow<'a, str>,
        detail_body: &'a Text<'a>,
        detail_height: u16,
    },
    Review {
        summary_title: Cow<'a, str>,
        summary_body: &'a Text<'a>,
        footer: &'a Text<'a>,
    },
    Question {
        prompt_title: Cow<'a, str>,
        prompt_body: &'a Text<'a>,
        choices_title: Cow<'a, str>,
        choices_body: &'a Text<'a>,
        custom_input: Option<QuestionFlowCustomInputSpec<'a>>,
        footer: &'a Text<'a>,
    },
}

impl<'a> QuestionFlowDialogMode<'a> {
    pub fn empty(
        detail_title: Cow<'a, str>,
        detail_body: &'a Text<'a>,
        detail_height: u16,
    ) -> Self {
        Self::Empty {
            detail_title,
            detail_body,
            detail_height,
        }
    }

    pub fn review(
        summary_title: Cow<'a, str>,
        summary_body: &'a Text<'a>,
        footer: &'a Text<'a>,
    ) -> Self {
        Self::Review {
            summary_title,
            summary_body,
            footer,
        }
    }

    pub fn question(
        prompt_title: Cow<'a, str>,
        prompt_body: &'a Text<'a>,
        choices_title: Cow<'a, str>,
        choices_body: &'a Text<'a>,
        custom_input: Option<QuestionFlowCustomInputSpec<'a>>,
        footer: &'a Text<'a>,
    ) -> Self {
        Self::Question {
            prompt_title,
            prompt_body,
            choices_title,
            choices_body,
            custom_input,
            footer,
        }
    }
}

pub struct QuestionFlowDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub nav_title: Cow<'a, str>,
    pub nav_body: Option<&'a Text<'a>>,
    pub mode: QuestionFlowDialogMode<'a>,
}

impl<'a> QuestionFlowDialogSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        target_width: u16,
        nav_title: Cow<'a, str>,
        mode: QuestionFlowDialogMode<'a>,
    ) -> Self {
        Self {
            title,
            target_width,
            nav_title,
            nav_body: None,
            mode,
        }
    }

    pub fn with_nav_body(mut self, nav_body: &'a Text<'a>) -> Self {
        self.nav_body = Some(nav_body);
        self
    }
}

pub fn render_question_flow_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &QuestionFlowDialogSpec<'_>,
) -> StackedDialogRenderResult {
    let mut sections = Vec::new();
    if let Some(nav_body) = spec.nav_body {
        sections.push(StackedDialogSection::TextPanel(TextPanelSection {
            height: StackedDialogSectionHeight::AutoText { min: 2, max: 4 },
            spec: TextPanelSpec {
                title: Some(spec.nav_title.clone()),
                body: nav_body,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        }));
    }

    match &spec.mode {
        QuestionFlowDialogMode::Empty {
            detail_title,
            detail_body,
            detail_height,
        } => sections.push(StackedDialogSection::TextPanel(TextPanelSection {
            height: StackedDialogSectionHeight::Fixed(*detail_height),
            spec: TextPanelSpec {
                title: Some(detail_title.clone()),
                body: detail_body,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        })),
        QuestionFlowDialogMode::Review {
            summary_title,
            summary_body,
            footer,
        } => {
            sections.push(StackedDialogSection::TextPanel(TextPanelSection {
                height: StackedDialogSectionHeight::AutoText { min: 4, max: 12 },
                spec: TextPanelSpec {
                    title: Some(summary_title.clone()),
                    body: summary_body,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            }));
            sections.push(StackedDialogSection::Paragraph(ParagraphSection {
                height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
                title: None,
                borders: Borders::NONE,
                body: (*footer).clone(),
                wrap: true,
                scroll: None,
                alignment: None,
            }));
        }
        QuestionFlowDialogMode::Question {
            prompt_title,
            prompt_body,
            choices_title,
            choices_body,
            custom_input,
            footer,
        } => {
            sections.push(StackedDialogSection::TextPanel(TextPanelSection {
                height: StackedDialogSectionHeight::AutoText { min: 3, max: 6 },
                spec: TextPanelSpec {
                    title: Some(prompt_title.clone()),
                    body: prompt_body,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            }));
            sections.push(StackedDialogSection::TextPanel(TextPanelSection {
                height: StackedDialogSectionHeight::AutoText { min: 4, max: 12 },
                spec: TextPanelSpec {
                    title: Some(choices_title.clone()),
                    body: choices_body,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            }));
            if let Some(custom_input) = custom_input.as_ref() {
                sections.push(StackedDialogSection::EditorPanel(EditorSection {
                    height: StackedDialogSectionHeight::AutoEditor { multiline: true },
                    spec: EditorPanelSpec {
                        title: Some(custom_input.title.clone()),
                        borders: Borders::ALL,
                    },
                    input: custom_input.input,
                    set_cursor: custom_input.editing,
                }));
            }
            sections.push(StackedDialogSection::Paragraph(ParagraphSection {
                height: StackedDialogSectionHeight::AutoText { min: 1, max: 2 },
                title: None,
                borders: Borders::NONE,
                body: (*footer).clone(),
                wrap: true,
                scroll: None,
                alignment: None,
            }));
        }
    }

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

    use ratatui::text::Text;

    use super::{QuestionFlowCustomInputSpec, QuestionFlowDialogMode, QuestionFlowDialogSpec};
    use crate::Editor;

    #[test]
    fn question_flow_dialog_spec_builder_preserves_nav_and_mode() {
        let nav = Text::from("nav");
        let prompt = Text::from("prompt");
        let choices = Text::from("choices");
        let footer = Text::from("footer");
        let editor = Editor::from_text("custom".to_string());
        let spec = QuestionFlowDialogSpec::new(
            Cow::Borrowed("Title"),
            92,
            Cow::Borrowed("Questions"),
            QuestionFlowDialogMode::question(
                Cow::Borrowed("Prompt"),
                &prompt,
                Cow::Borrowed("Choices"),
                &choices,
                Some(QuestionFlowCustomInputSpec::new(
                    Cow::Borrowed("Other"),
                    &editor,
                    true,
                )),
                &footer,
            ),
        )
        .with_nav_body(&nav);

        assert_eq!(spec.title.as_ref(), "Title");
        assert_eq!(spec.nav_title.as_ref(), "Questions");
        assert!(spec.nav_body.is_some());
        match spec.mode {
            QuestionFlowDialogMode::Question { custom_input, .. } => {
                assert!(custom_input.is_some());
            }
            _ => panic!("expected question mode"),
        }
    }
}
