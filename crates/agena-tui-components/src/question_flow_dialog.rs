//! Dialog widget rendering a question flow.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    text::Text,
    widgets::{Borders, ListItem},
};

use crate::{
    ChoicePanelSection, Editor, ParagraphSection, StackedDialogRenderResult, StackedDialogSection,
    StackedDialogSectionHeight, StackedDialogSpec, SurfaceMode, TextPanelSection, TextPanelSpec,
    render_stacked_dialog_scrollable,
};

/// Custom input spec of the question flow dialog.
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

/// Mode of the question flow dialog.
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
        choices_items: &'a [ListItem<'a>],
        choices_selected: Option<usize>,
        preview_title: Option<Cow<'a, str>>,
        preview_body: Option<&'a Text<'a>>,
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

    #[allow(clippy::too_many_arguments)]
    pub fn question(
        prompt_title: Cow<'a, str>,
        prompt_body: &'a Text<'a>,
        choices_title: Cow<'a, str>,
        choices_items: &'a [ListItem<'a>],
        choices_selected: Option<usize>,
        preview_title: Option<Cow<'a, str>>,
        preview_body: Option<&'a Text<'a>>,
        custom_input: Option<QuestionFlowCustomInputSpec<'a>>,
        footer: &'a Text<'a>,
    ) -> Self {
        Self::Question {
            prompt_title,
            prompt_body,
            choices_title,
            choices_items,
            choices_selected,
            preview_title,
            preview_body,
            custom_input,
            footer,
        }
    }
}

/// Spec of the question flow dialog.
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
        nav_body: Option<&'a Text<'a>>,
        mode: QuestionFlowDialogMode<'a>,
    ) -> Self {
        Self {
            title,
            target_width,
            nav_title,
            nav_body,
            mode,
        }
    }
}

/// Renders a question flow dialog without whole-dialog scrolling.
pub fn render_question_flow_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &QuestionFlowDialogSpec<'_>,
) -> StackedDialogRenderResult {
    render_question_flow_dialog_scrollable(frame, area, surface, spec, 0)
}

/// Renders a question flow dialog with whole-dialog vertical scrolling.
///
/// Short terminals can scroll (PgUp/PgDn and friends) so the choices, preview,
/// inline custom editor, and footer are never permanently clipped.
pub fn render_question_flow_dialog_scrollable(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &QuestionFlowDialogSpec<'_>,
    scroll: u16,
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
                height: StackedDialogSectionHeight::AutoText { min: 4, max: 18 },
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
            choices_items,
            choices_selected,
            preview_title,
            preview_body,
            custom_input,
            footer,
        } => {
            sections.push(StackedDialogSection::TextPanel(TextPanelSection {
                height: StackedDialogSectionHeight::AutoText { min: 4, max: 12 },
                spec: TextPanelSpec {
                    title: Some(prompt_title.clone()),
                    body: prompt_body,
                    wrap: true,
                    scroll: None,
                    alignment: None,
                },
            }));
            sections.push(StackedDialogSection::ChoicePanel(ChoicePanelSection {
                height: StackedDialogSectionHeight::AutoList {
                    lines_per_item: 2,
                    min_body: 4,
                    max_body: 12,
                },
                title: Some(choices_title.clone()),
                items: choices_items,
                selected: *choices_selected,
                highlight_style: crate::theme::selection_style(),
                highlight_symbol: Cow::Borrowed("> "),
                inline_editor: custom_input.as_ref().map(|custom| custom.input),
                editing: custom_input.as_ref().is_some_and(|custom| custom.editing),
                set_cursor: custom_input.as_ref().is_some_and(|custom| custom.editing),
            }));
            if let (Some(preview_title), Some(preview_body)) =
                (preview_title.as_ref(), preview_body.as_ref())
            {
                sections.push(StackedDialogSection::TextPanel(TextPanelSection {
                    height: StackedDialogSectionHeight::AutoText { min: 3, max: 12 },
                    spec: TextPanelSpec {
                        title: Some(preview_title.clone()),
                        body: preview_body,
                        wrap: true,
                        scroll: None,
                        alignment: None,
                    },
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

    render_stacked_dialog_scrollable(
        frame,
        area,
        surface,
        &StackedDialogSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            sections,
        },
        scroll,
    )
}
