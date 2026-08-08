//! Stacked dialog container.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::Text,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};

use crate::{
    Editor, EditorPanelSpec, FramedSurfaceSpec,
        layout::{
        SurfaceMode, VerticalSectionSize, editor_input_panel_height, framed_overlay_height,
        list_panel_height, split_vertical_sections,
    },
    panels::{ListPanelSpec, TextPanelSpec, render_list_panel_with_offset, render_text_panel},
    render_editor_panel, render_framed_surface,
    text::{bordered_text_height, wrapped_text_height_for_text},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Height of a stacked dialog section.
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

/// Spec of the stacked dialog.
pub struct StackedDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub sections: Vec<StackedDialogSection<'a>>,
}

/// A section of the stacked dialog.
pub enum StackedDialogSection<'a> {
    Paragraph(ParagraphSection<'a>),
    TextPanel(TextPanelSection<'a>),
    ListPanel(ListPanelSection<'a>),
    ChoicePanel(ChoicePanelSection<'a>),
    EditorPanel(EditorSection<'a>),
}

/// A paragraph section of the stacked dialog.
pub struct ParagraphSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub title: Option<Cow<'a, str>>,
    pub borders: Borders,
    pub body: Text<'a>,
    pub wrap: bool,
    pub scroll: Option<(u16, u16)>,
    pub alignment: Option<Alignment>,
}

/// A text panel section of the stacked dialog.
pub struct TextPanelSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: TextPanelSpec<'a>,
}

/// A list panel section of the stacked dialog.
pub struct ListPanelSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: ListPanelSpec<'a>,
}

/// A choices section of the stacked dialog: a bordered list plus an optional
/// inline single-line editor rendered directly under the choice rows. Question
/// flows use this so a custom reply is typed at the custom option row instead
/// of in a separate bottom editor panel.
pub struct ChoicePanelSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub title: Option<Cow<'a, str>>,
    pub items: &'a [ListItem<'a>],
    pub selected: Option<usize>,
    pub highlight_style: Style,
    pub highlight_symbol: Cow<'a, str>,
    pub inline_editor: Option<&'a Editor>,
    pub editing: bool,
    pub set_cursor: bool,
}

/// An editor section of the stacked dialog.
pub struct EditorSection<'a> {
    pub height: StackedDialogSectionHeight,
    pub spec: EditorPanelSpec<'a>,
    pub input: &'a Editor,
    pub set_cursor: bool,
}

/// Render result of the stacked dialog.
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
            Self::ChoicePanel(section) => section.height.resolve_choice_panel(
                section.items.len(),
                section.inline_editor.is_some(),
            ),
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

    fn resolve_choice_panel(self, item_count: usize, has_inline_editor: bool) -> u16 {
        let base = match self {
            Self::Fixed(height) => height,
            Self::AutoList {
                lines_per_item,
                min_body,
                max_body,
            } => list_panel_height(item_count, lines_per_item, min_body, max_body),
            Self::AutoText { .. } | Self::AutoEditor { .. } => 0,
        };
        if has_inline_editor {
            base.saturating_add(1)
        } else {
            base
        }
    }
}

/// Scroll metrics of a stacked dialog on a given surface.
#[derive(Debug, Clone, Copy)]
pub struct StackedDialogScrollMetrics {
    /// Total natural height of all sections before any clipping.
    pub content_height: u16,
    /// Maximum whole-dialog scroll that still exposes the bottom of the
    /// content inside the available surface.
    pub max_scroll: u16,
}

/// Computes the natural content height and reachable scroll range of a stacked
/// dialog. Callers use `max_scroll` to clamp their dialog scroll state before
/// calling [`render_stacked_dialog_scrollable`].
pub fn stacked_dialog_scroll_metrics(
    area: Rect,
    surface: SurfaceMode,
    spec: &StackedDialogSpec<'_>,
) -> StackedDialogScrollMetrics {
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
    let outer_height = surface
        .outer_rect(
            area,
            spec.target_width,
            framed_overlay_height(content_height),
        )
        .height;
    let available = outer_height.saturating_sub(2).max(1);
    StackedDialogScrollMetrics {
        content_height,
        max_scroll: content_height.saturating_sub(available),
    }
}

/// Renders a stacked dialog. Equivalent to the scrollable variant with no
/// scroll.
pub fn render_stacked_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &StackedDialogSpec<'_>,
) -> StackedDialogRenderResult {
    render_stacked_dialog_scrollable(frame, area, surface, spec, 0)
}

/// Renders a stacked dialog with whole-dialog vertical scrolling.
///
/// Sections above `scroll` are hidden entirely; visible sections are laid out
/// top-aligned inside the surface, and each section's own content is offset by
/// the portion of the section that scrolled out of view. This lets overlays
/// with more content than the terminal surface (for example a question flow on
/// a short window) reveal every section instead of clipping the bottom.
pub fn render_stacked_dialog_scrollable(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &StackedDialogSpec<'_>,
    scroll: u16,
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

    if surface == SurfaceMode::Route {
        // Full-canvas routes keep the historical flexible-last-section layout;
        // scroll has no meaning on an unbounded canvas.
        return render_route_sections(frame, frame_surface.inner, spec, &heights);
    }

    let inner = frame_surface.inner;
    let available = inner.height.max(1);
    let scroll = scroll.min(content_height.saturating_sub(available));

    let mut cursor = None;
    let mut cumulative = 0_u16;
    for (section, height) in spec.sections.iter().zip(heights) {
        let section_start = cumulative;
        let section_end = cumulative.saturating_add(height);
        cumulative = section_end;
        let window_end = scroll.saturating_add(available);
        if section_end <= scroll || section_start >= window_end {
            continue;
        }
        let visible_start = section_start.max(scroll);
        let visible_end = section_end.min(window_end);
        let section_area = Rect {
            x: inner.x,
            y: inner.y.saturating_add(visible_start.saturating_sub(scroll)),
            width: inner.width,
            height: visible_end.saturating_sub(visible_start),
        };
        let section_scroll = visible_start.saturating_sub(section_start);
        render_stacked_section(frame, section_area, section, section_scroll, &mut cursor);
    }

    StackedDialogRenderResult { cursor }
}

fn render_route_sections(
    frame: &mut Frame,
    inner: Rect,
    spec: &StackedDialogSpec<'_>,
    heights: &[u16],
) -> StackedDialogRenderResult {
    let section_areas = split_vertical_sections(
        inner,
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
    );
    let mut cursor = None;
    for (section, section_area) in spec.sections.iter().zip(section_areas) {
        render_stacked_section(frame, section_area, section, 0, &mut cursor);
    }
    StackedDialogRenderResult { cursor }
}

fn render_stacked_section(
    frame: &mut Frame,
    section_area: Rect,
    section: &StackedDialogSection<'_>,
    section_scroll: u16,
    cursor: &mut Option<(u16, u16)>,
) {
    match section {
        StackedDialogSection::Paragraph(section) => {
            let mut paragraph = Paragraph::new(section.body.clone());
            if section.wrap {
                paragraph = paragraph.wrap(ratatui::widgets::Wrap { trim: false });
            }
            let vertical = section.scroll.map_or(0, |scroll| scroll.0);
            let horizontal = section.scroll.map_or(0, |scroll| scroll.1);
            paragraph = paragraph.scroll((vertical.saturating_add(section_scroll), horizontal));
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
            let mut spec = section.spec.clone();
            let vertical = spec.scroll.map_or(0, |scroll| scroll.0);
            let horizontal = spec.scroll.map_or(0, |scroll| scroll.1);
            spec.scroll = Some((vertical.saturating_add(section_scroll), horizontal));
            render_text_panel(frame, section_area, &spec);
        }
        StackedDialogSection::ListPanel(section) => {
            render_list_panel_with_offset(frame, section_area, &section.spec, section_scroll);
        }
        StackedDialogSection::ChoicePanel(section) => {
            if let Some(position) = render_choice_panel(frame, section_area, section, section_scroll)
            {
                *cursor = Some(position);
            }
        }
        StackedDialogSection::EditorPanel(section) => {
            let result = render_editor_panel(frame, section_area, &section.spec, section.input);
            if section.set_cursor && section_scroll == 0 {
                *cursor = Some(result.cursor);
            }
        }
    }
}

/// Renders a choices panel: a bordered list plus an optional inline editor
/// row. Returns the terminal cursor position when the inline editor should own
/// it.
fn render_choice_panel(
    frame: &mut Frame,
    area: Rect,
    section: &ChoicePanelSection<'_>,
    list_scroll: u16,
) -> Option<(u16, u16)> {
    let block = match section.title.as_ref() {
        Some(title) => Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", title)),
        None => Block::default().borders(Borders::ALL),
    };
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let has_editor = section.inline_editor.is_some();
    let list_height = if has_editor {
        inner.height.saturating_sub(1).max(1)
    } else {
        inner.height
    };
    let list_area = Rect {
        x: inner.x,
        y: inner.y,
        width: inner.width,
        height: list_height,
    };
    let list = List::new(section.items.iter().cloned())
        .highlight_style(section.highlight_style)
        .highlight_symbol(section.highlight_symbol.as_ref());
    let mut state = ListState::default()
        .with_offset(usize::from(list_scroll))
        .with_selected(section.selected);
    frame.render_stateful_widget(list, list_area, &mut state);

    let input = section.inline_editor?;
    let editor_area = Rect {
        x: inner.x,
        y: inner.y.saturating_add(list_height),
        width: inner.width,
        height: inner.height.saturating_sub(list_height).max(1),
    };
    let view = input.render_view(editor_area.width, 1);
    let mut paragraph = Paragraph::new(Text::from(view.lines.clone()));
    if section.editing {
        paragraph = paragraph.style(section.highlight_style);
    }
    frame.render_widget(paragraph, editor_area);
        section.set_cursor.then(|| {
        (
            editor_area.x.saturating_add(view.cursor_x),
            editor_area.y.saturating_add(view.cursor_y),
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{
        Terminal,
        backend::TestBackend,
        text::Line as TextLine,
    };

    fn fixed_paragraph(height: u16, text: &str) -> StackedDialogSection<'static> {
        StackedDialogSection::Paragraph(ParagraphSection {
            height: StackedDialogSectionHeight::Fixed(height),
            title: None,
            borders: Borders::NONE,
            body: Text::from(TextLine::from(text.to_owned())),
            wrap: true,
            scroll: None,
            alignment: None,
        })
    }

    fn buffer_contains(terminal: &Terminal<TestBackend>, ch: char) -> bool {
        terminal
            .backend()
            .buffer()
            .content()
            .iter()
            .any(|cell| cell.symbol().contains(ch))
    }

    #[test]
    fn short_surface_reveals_bottom_sections_when_scrolled() {
        let spec = StackedDialogSpec {
            title: Cow::Borrowed("X"),
            target_width: 30,
            sections: vec![
                fixed_paragraph(4, "top"),
                fixed_paragraph(4, "middle"),
                fixed_paragraph(4, "bottom"),
            ],
        };
        let area = Rect::new(0, 0, 40, 10);
        let metrics =
            stacked_dialog_scroll_metrics(area, SurfaceMode::Overlay, &spec);
        assert!(
            metrics.max_scroll > 0,
            "overflowing dialog must be scrollable"
        );

        let mut terminal = Terminal::new(TestBackend::new(40, 10)).unwrap();
        terminal
            .draw(|frame| {
                render_stacked_dialog_scrollable(frame, area, SurfaceMode::Overlay, &spec, 0);
            })
            .unwrap();
        assert!(buffer_contains(&terminal, 'p'), "top section visible");
        assert!(buffer_contains(&terminal, 'd'), "middle section visible");
        assert!(
            !buffer_contains(&terminal, 'b'),
            "bottom section hidden before scrolling"
        );

        terminal
            .draw(|frame| {
                render_stacked_dialog_scrollable(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    &spec,
                    metrics.max_scroll,
                );
            })
            .unwrap();
        assert!(
            !buffer_contains(&terminal, 'p'),
            "top section hidden after scrolling"
        );
        assert!(
            !buffer_contains(&terminal, 'd'),
            "middle section scrolled out with its text"
        );
        assert!(buffer_contains(&terminal, 'm'), "bottom section revealed");
    }

    #[test]
    fn choice_panel_reports_inline_editor_cursor() {
        let input = Editor::from_text("hello".to_string());
        let spec = StackedDialogSpec {
            title: Cow::Borrowed("X"),
            target_width: 30,
            sections: vec![StackedDialogSection::ChoicePanel(ChoicePanelSection {
                height: StackedDialogSectionHeight::Fixed(10),
                title: None,
                items: &[],
                selected: None,
                highlight_style: Style::default(),
                highlight_symbol: Cow::Borrowed("> "),
                inline_editor: Some(&input),
                editing: true,
                set_cursor: true,
            })],
        };
        let area = Rect::new(0, 0, 30, 10);
        let mut terminal = Terminal::new(TestBackend::new(30, 10)).unwrap();
        let mut render_result = None;
        terminal
            .draw(|frame| {
                render_result = Some(render_stacked_dialog_scrollable(
                    frame,
                    area,
                    SurfaceMode::Overlay,
                    &spec,
                    0,
                ));
            })
            .unwrap();
        let cursor = render_result
            .expect("render must run")
            .cursor
            .expect("inline editor must report a terminal cursor");
        // The modal frame is height-clamped to 8 rows and centered at (1,1):
        // outer=(1,1,28,8), inner=(2,2,26,6). The editor row is the last inner
        // row (y=6) with the border column at x=3 plus the 5-char text.
        assert_eq!(cursor, (3 + 5, 6));
    }
}
