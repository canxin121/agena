use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::Text,
    widgets::{Block, Borders, Paragraph},
};

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec,
    layout::{
        SurfaceMode, VerticalSectionSize, editor_input_panel_height, framed_overlay_height,
        list_panel_height, split_vertical_sections, top_aligned_vertical_areas,
    },
    panels::{ListPanelSpec, TextPanelSpec, render_list_panel, render_text_panel},
    render_editor_panel, render_framed_surface,
    text::{bordered_text_height, wrapped_text_height_for_text},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StackedDialogSectionHeight {
    Fixed(u16),
    AutoText {
        min: u16,
        max: u16,
    },
    AutoList {
        lines_per_item: u16,
        min_body: u16,
        max_body: u16,
    },
    AutoEditor {
        multiline: bool,
    },
}

pub struct StackedDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub sections: Vec<StackedDialogSection<'a>>,
}

pub enum StackedDialogSection<'a> {
    Paragraph(ParagraphSection<'a>),
    TextPanel(TextPanelSection<'a>),
    ListPanel(ListPanelSection<'a>),
    EditorPanel(EditorSection<'a>),
}

pub struct ParagraphSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub title: Option<Cow<'a, str>>,
    pub borders: Borders,
    pub body: Text<'a>,
    pub wrap: bool,
    pub scroll: Option<(u16, u16)>,
    pub alignment: Option<Alignment>,
}

pub struct TextPanelSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: TextPanelSpec<'a>,
}

pub struct ListPanelSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: ListPanelSpec<'a>,
}

pub struct EditorSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: EditorPanelSpec<'a>,
    pub input: &'a Editor,
    pub set_cursor: bool,
}

pub struct StackedDialogRenderResult {
    pub cursor: Option<(u16, u16)>,
}

impl<'a> StackedDialogSection<'a> {
    fn height(&self, width: u16) -> u16 {
        match self {
            Self::Paragraph(section) => section.height.resolve_paragraph(
                &section.body,
                width,
                section.borders,
                section.title.is_some(),
            ),
            Self::TextPanel(section) => section.height.resolve_text_panel(section.spec.body, width),
            Self::ListPanel(section) => section.height.resolve_list_panel(section.spec.items.len()),
            Self::EditorPanel(section) => section.height.resolve_editor(section.input),
        }
    }
}

impl StackedDialogSectionHeight {
    fn resolve_paragraph(
        self,
        body: &Text<'_>,
        width: u16,
        borders: Borders,
        has_title: bool,
    ) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoText { min, max } => {
                if borders == Borders::NONE && !has_title {
                    wrapped_text_height_for_text(body, width).clamp(min, max)
                } else {
                    bordered_text_height(body, width, min, max)
                }
            }
            Self::AutoList { .. } | Self::AutoEditor { .. } => 0,
        }
    }

    fn resolve_text_panel(self, body: &Text<'_>, width: u16) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoText { min, max } => bordered_text_height(body, width, min, max),
            Self::AutoList { .. } | Self::AutoEditor { .. } => 0,
        }
    }

    fn resolve_list_panel(self, item_count: usize) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoList {
                lines_per_item,
                min_body,
                max_body,
            } => list_panel_height(item_count, lines_per_item, min_body, max_body),
            Self::AutoText { .. } | Self::AutoEditor { .. } => 0,
        }
    }

    fn resolve_editor(self, input: &Editor) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoEditor { multiline } => editor_input_panel_height(input, multiline),
            Self::AutoText { .. } | Self::AutoList { .. } => 0,
        }
    }
}

pub fn render_stacked_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &StackedDialogSpec<'_>,
) -> StackedDialogRenderResult {
    let content_width = surface
        .content_width(area, spec.target_width)
        .saturating_sub(2)
        .max(1);
    let heights = spec
        .sections
        .iter()
        .map(|section| section.height(content_width))
        .collect::<Vec<_>>();
    let content_height = heights.iter().copied().fold(0_u16, u16::saturating_add);
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            target_height: framed_overlay_height(content_height),
        },
    );
    let section_areas = match surface {
        SurfaceMode::Overlay => top_aligned_vertical_areas(frame_surface.inner, heights.as_slice()),
        SurfaceMode::Route => split_vertical_sections(
            frame_surface.inner,
            &heights
                .iter()
                .enumerate()
                .map(|(index, height)| {
                    if index + 1 == heights.len() {
                        VerticalSectionSize::Flexible(*height)
                    } else {
                        VerticalSectionSize::Fixed(*height)
                    }
                })
                .collect::<Vec<_>>(),
        ),
    };

    let mut cursor = None;
    for (section, section_area) in spec.sections.iter().zip(section_areas) {
        match section {
            StackedDialogSection::Paragraph(section) => {
                let mut paragraph = Paragraph::new(section.body.clone());
                if section.wrap {
                    paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
                }
                if let Some(scroll) = section.scroll {
                    paragraph = paragraph.scroll(scroll);
                }
                if let Some(alignment) = section.alignment {
                    paragraph = paragraph.alignment(alignment);
                }
                if section.borders != Borders::NONE || section.title.is_some() {
                    let block = match section.title.as_ref() {
                        Some(title) => Block::default()
                            .borders(section.borders)
                            .title(format!(" {} ", title)),
                        None => Block::default().borders(section.borders),
                    };
                    paragraph = paragraph.block(block);
                }
                frame.render_widget(paragraph, section_area);
            }
            StackedDialogSection::TextPanel(section) => {
                render_text_panel(frame, section_area, &section.spec);
            }
            StackedDialogSection::ListPanel(section) => {
                render_list_panel(frame, section_area, &section.spec);
            }
            StackedDialogSection::EditorPanel(section) => {
                let result = render_editor_panel(frame, section_area, &section.spec, section.input);
                if section.set_cursor {
                    cursor = Some(result.cursor);
                }
            }
        }
    }

    StackedDialogRenderResult { cursor }
}
