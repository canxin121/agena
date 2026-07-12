use std::{borrow::Cow, cmp::max};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Text,
    widgets::{Paragraph, Wrap},
};

use crate::{
    Editor, EditorDialogSpec, EditorDialogState, FramedSurfaceSpec, InputDialogState,
    bordered_text_height,
    layout::{
        SurfaceMode, VerticalSectionSize, adaptive_detail_split, estimated_horizontal_panel_widths,
        framed_sections_target_height, optional_overlay_text_height, should_stack_detail_layout,
        split_vertical_sections, top_aligned_panel_rect, top_aligned_vertical_areas,
    },
    panels::{
        BoundedListPanelHeight, ListPanelState, TextPanelSpec, render_list_panel_state,
        render_text_panel,
    },
    render_editor_dialog, render_framed_surface, title_with_summary,
};

#[derive(Clone)]
pub struct WorkbenchTextSection<'a> {
    pub title: Cow<'a, str>,
    pub body: Text<'a>,
    pub min_body_height: u16,
    pub max_body_height: u16,
}

impl<'a> WorkbenchTextSection<'a> {
    pub fn new(
        title: Cow<'a, str>,
        body: Text<'a>,
        min_body_height: u16,
        max_body_height: u16,
    ) -> Self {
        Self {
            title,
            body,
            min_body_height,
            max_body_height,
        }
    }
}

struct TwoPaneWorkbenchSpec<'a> {
    pub title: Cow<'a, str>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub left_panel_width: u16,
    pub left_min_width: Option<u16>,
    pub right_min_width: Option<u16>,
    pub left_panel_state: ListWorkbenchPanelState<'a>,
    pub right_sections: Vec<WorkbenchTextSection<'a>>,
}

pub struct WorkbenchOverlayDialogSpec<'a> {
    pub(crate) dialog: EditorDialogSpec<'a>,
    pub(crate) input: &'a Editor,
}

pub trait WorkbenchOverlaySource {
    fn to_workbench_overlay_dialog_spec<'a>(&'a self) -> WorkbenchOverlayDialogSpec<'a>;
}

pub struct ListWorkbenchDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub left_panel_width: u16,
    pub left_min_width: Option<u16>,
    pub right_min_width: Option<u16>,
    pub left_panel_state: ListWorkbenchPanelState<'a>,
    pub right_sections: Vec<WorkbenchTextSection<'a>>,
    pub overlay: Option<WorkbenchOverlayDialogSpec<'a>>,
}

pub type ListWorkbenchPanelState<'a> = ListPanelState<'a, BoundedListPanelHeight>;

impl<'a> ListWorkbenchDialogSpec<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: Cow<'a, str>,
        summary: Option<Cow<'a, str>>,
        footer: Cow<'a, str>,
        target_width: u16,
        left_panel_width: u16,
        left_min_width: Option<u16>,
        right_min_width: Option<u16>,
        left_panel_state: ListWorkbenchPanelState<'a>,
        right_sections: Vec<WorkbenchTextSection<'a>>,
        overlay: Option<WorkbenchOverlayDialogSpec<'a>>,
    ) -> Self {
        Self {
            title,
            summary,
            footer,
            target_width,
            left_panel_width,
            left_min_width,
            right_min_width,
            left_panel_state,
            right_sections,
            overlay,
        }
    }
}

impl<'a> WorkbenchOverlayDialogSpec<'a> {
    pub fn workbench_editor(
        title: Cow<'a, str>,
        prompt: Cow<'a, str>,
        footer: Cow<'a, str>,
        multiline: bool,
        input: &'a Editor,
    ) -> Self {
        Self {
            dialog: EditorDialogSpec {
                title,
                prompt,
                footer,
                target_width: if multiline { 96 } else { 78 },
                multiline,
                prompt_height_bounds: (1, 3),
                footer_height_bounds: (1, 2),
            },
            input,
        }
    }

    pub fn line_input(title: Cow<'a, str>, prompt: Cow<'a, str>, input: &'a Editor) -> Self {
        Self {
            dialog: EditorDialogSpec {
                title,
                prompt,
                footer: Cow::Borrowed(""),
                target_width: 76,
                multiline: false,
                prompt_height_bounds: (1, 2),
                footer_height_bounds: (0, 0),
            },
            input,
        }
    }

    pub fn from_source<TSource>(source: &'a TSource) -> Self
    where
        TSource: WorkbenchOverlaySource + ?Sized,
    {
        source.to_workbench_overlay_dialog_spec()
    }

    pub fn from_editor_state<TAction>(editor: &'a EditorDialogState<TAction>) -> Self {
        Self::from_source(editor)
    }

    pub fn from_input_state<TAction>(dialog: &'a InputDialogState<TAction>) -> Self {
        Self::from_source(dialog)
    }
}

impl<TAction> WorkbenchOverlaySource for EditorDialogState<TAction> {
    fn to_workbench_overlay_dialog_spec<'a>(&'a self) -> WorkbenchOverlayDialogSpec<'a> {
        WorkbenchOverlayDialogSpec::workbench_editor(
            Cow::Owned(self.title.clone()),
            Cow::Owned(self.prompt.clone()),
            Cow::Owned(self.footer.clone()),
            self.multiline,
            &self.input,
        )
    }
}

impl<TAction> WorkbenchOverlaySource for InputDialogState<TAction> {
    fn to_workbench_overlay_dialog_spec<'a>(&'a self) -> WorkbenchOverlayDialogSpec<'a> {
        WorkbenchOverlayDialogSpec::line_input(
            Cow::Owned(self.title.clone()),
            Cow::Owned(self.prompt.clone()),
            &self.input,
        )
    }
}

struct SectionedWorkbenchSpec<'a> {
    pub title: Cow<'a, str>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub nav_panel_width: u16,
    pub nav_panel_state: ListWorkbenchPanelState<'a>,
    pub section_panel: WorkbenchTextSection<'a>,
    pub items_panel_state: ListWorkbenchPanelState<'a>,
    pub detail_panel: WorkbenchTextSection<'a>,
}

pub struct SectionedWorkbenchDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub nav_panel_width: u16,
    pub nav_panel_state: ListWorkbenchPanelState<'a>,
    pub section_panel: WorkbenchTextSection<'a>,
    pub items_panel_state: ListWorkbenchPanelState<'a>,
    pub detail_panel: WorkbenchTextSection<'a>,
}

impl<'a> SectionedWorkbenchDialogSpec<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: Cow<'a, str>,
        summary: Option<Cow<'a, str>>,
        footer: Cow<'a, str>,
        target_width: u16,
        nav_panel_width: u16,
        nav_panel_state: ListWorkbenchPanelState<'a>,
        section_panel: WorkbenchTextSection<'a>,
        items_panel_state: ListWorkbenchPanelState<'a>,
        detail_panel: WorkbenchTextSection<'a>,
    ) -> Self {
        Self {
            title,
            summary,
            footer,
            target_width,
            nav_panel_width,
            nav_panel_state,
            section_panel,
            items_panel_state,
            detail_panel,
        }
    }
}

fn render_two_pane_workbench(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &TwoPaneWorkbenchSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let footer_height = optional_overlay_text_height(spec.footer.as_ref(), content_width, 1, 2);
    let stacked = match (spec.left_min_width, spec.right_min_width) {
        (Some(left_min), Some(right_min)) => {
            should_stack_detail_layout(content_width, left_min, right_min)
        }
        _ => false,
    };
    let right_width = match (stacked, spec.left_min_width, spec.right_min_width) {
        (true, _, _) => content_width.saturating_sub(2).max(1),
        (false, Some(left_min), Some(right_min)) => {
            estimated_horizontal_panel_widths(content_width, left_min, right_min)
                .1
                .saturating_sub(2)
                .max(1)
        }
        _ => content_width.saturating_sub(spec.left_panel_width).max(1),
    };
    let left_panel_height = spec.left_panel_state.resolve_height();
    let right_section_heights = spec
        .right_sections
        .iter()
        .map(|section| {
            bordered_text_height(
                &section.body,
                right_width,
                section.min_body_height,
                section.max_body_height,
            )
        })
        .collect::<Vec<_>>();
    let right_total_height = right_section_heights.iter().copied().sum::<u16>();
    let content_height = if stacked {
        left_panel_height.saturating_add(right_total_height)
    } else {
        max(left_panel_height, right_total_height)
    };
    let mut sections = vec![VerticalSectionSize::Flexible(content_height)];
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(frame_surface.inner, &sections);
    let (list_area, section_areas) = if stacked {
        let mut heights = Vec::with_capacity(1 + right_section_heights.len());
        heights.push(left_panel_height);
        heights.extend(right_section_heights.iter().copied());
        let areas = match surface {
            SurfaceMode::Overlay => top_aligned_vertical_areas(rows[0], &heights),
            SurfaceMode::Route => split_vertical_sections(
                rows[0],
                &heights
                    .iter()
                    .enumerate()
                    .map(|(index, height)| {
                        if index == 0 {
                            VerticalSectionSize::Flexible(*height)
                        } else {
                            VerticalSectionSize::Fixed(*height)
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
        };
        (areas[0], areas[1..].to_vec())
    } else {
        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(match (spec.left_min_width, spec.right_min_width) {
                (Some(left_min), Some(right_min)) => {
                    adaptive_detail_split(rows[0].width, left_min, right_min).to_vec()
                }
                _ => vec![
                    Constraint::Length(spec.left_panel_width),
                    Constraint::Min(1),
                ],
            })
            .split(rows[0]);
        let list_area = match surface {
            SurfaceMode::Overlay => top_aligned_panel_rect(content[0], left_panel_height),
            SurfaceMode::Route => content[0],
        };
        let section_areas = match surface {
            SurfaceMode::Overlay => top_aligned_vertical_areas(content[1], &right_section_heights),
            SurfaceMode::Route => split_vertical_sections(
                content[1],
                &right_section_heights
                    .iter()
                    .enumerate()
                    .map(|(index, height)| {
                        if index + 1 == right_section_heights.len() {
                            VerticalSectionSize::Flexible(*height)
                        } else {
                            VerticalSectionSize::Fixed(*height)
                        }
                    })
                    .collect::<Vec<_>>(),
            ),
        };
        (list_area, section_areas)
    };
    render_list_panel_state(frame, list_area, &spec.left_panel_state);

    for (section, section_area) in spec.right_sections.iter().zip(section_areas) {
        render_text_panel(
            frame,
            section_area,
            &TextPanelSpec {
                title: Some(section.title.clone()),
                body: &section.body,
                wrap: true,
                scroll: None,
                alignment: None,
            },
        );
    }

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.footer.as_ref()).wrap(Wrap { trim: false }),
            rows[1],
        );
    }
}

fn render_two_pane_workbench_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &TwoPaneWorkbenchSpec<'_>,
    overlay: Option<WorkbenchOverlayDialogSpec<'_>>,
) {
    render_two_pane_workbench(frame, area, surface, spec);

    if let Some(overlay) = overlay {
        render_editor_dialog(
            frame,
            area,
            SurfaceMode::Overlay,
            &overlay.dialog,
            overlay.input,
        );
    }
}

pub fn render_list_workbench_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &ListWorkbenchDialogSpec<'_>,
) {
    let title = spec
        .summary
        .as_ref()
        .map(|summary| {
            title_with_summary(
                spec.title.as_ref(),
                summary.as_ref(),
                surface.outer_width(area, spec.target_width),
            )
            .trim()
            .to_string()
        })
        .unwrap_or_else(|| spec.title.as_ref().trim().to_string());
    render_two_pane_workbench_dialog(
        frame,
        area,
        surface,
        &TwoPaneWorkbenchSpec {
            title: Cow::Owned(title),
            footer: spec.footer.clone(),
            target_width: spec.target_width,
            left_panel_width: spec.left_panel_width,
            left_min_width: spec.left_min_width,
            right_min_width: spec.right_min_width,
            left_panel_state: spec.left_panel_state.clone(),
            right_sections: spec.right_sections.clone(),
        },
        spec.overlay
            .as_ref()
            .map(|overlay| WorkbenchOverlayDialogSpec {
                dialog: EditorDialogSpec {
                    title: overlay.dialog.title.clone(),
                    prompt: overlay.dialog.prompt.clone(),
                    footer: overlay.dialog.footer.clone(),
                    target_width: overlay.dialog.target_width,
                    multiline: overlay.dialog.multiline,
                    prompt_height_bounds: overlay.dialog.prompt_height_bounds,
                    footer_height_bounds: overlay.dialog.footer_height_bounds,
                },
                input: overlay.input,
            }),
    );
}

pub fn render_sectioned_workbench_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &SectionedWorkbenchDialogSpec<'_>,
) {
    let title = spec
        .summary
        .as_ref()
        .map(|summary| {
            title_with_summary(
                spec.title.as_ref(),
                summary.as_ref(),
                surface.outer_width(area, spec.target_width),
            )
            .trim()
            .to_string()
        })
        .unwrap_or_else(|| spec.title.as_ref().trim().to_string());
    render_sectioned_workbench(
        frame,
        area,
        surface,
        &SectionedWorkbenchSpec {
            title: Cow::Owned(title),
            footer: spec.footer.clone(),
            target_width: spec.target_width,
            nav_panel_width: spec.nav_panel_width,
            nav_panel_state: spec.nav_panel_state.clone(),
            section_panel: spec.section_panel.clone(),
            items_panel_state: spec.items_panel_state.clone(),
            detail_panel: spec.detail_panel.clone(),
        },
    );
}

fn render_sectioned_workbench(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &SectionedWorkbenchSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let footer_height = optional_overlay_text_height(spec.footer.as_ref(), content_width, 1, 2);
    let right_width = content_width.saturating_sub(spec.nav_panel_width).max(1);
    let nav_height = spec.nav_panel_state.resolve_height();
    let section_height = bordered_text_height(
        &spec.section_panel.body,
        right_width,
        spec.section_panel.min_body_height,
        spec.section_panel.max_body_height,
    );
    let items_height = spec.items_panel_state.resolve_height();
    let detail_height = bordered_text_height(
        &spec.detail_panel.body,
        right_width,
        spec.detail_panel.min_body_height,
        spec.detail_panel.max_body_height,
    );
    let content_height = max(
        nav_height,
        section_height
            .saturating_add(items_height)
            .saturating_add(detail_height),
    );
    let mut sections = vec![VerticalSectionSize::Flexible(content_height)];
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let frame_surface = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: spec.title.clone(),
            target_width: spec.target_width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    let rows = split_vertical_sections(frame_surface.inner, &sections);
    let content = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(spec.nav_panel_width), Constraint::Min(1)])
        .split(rows[0]);

    let nav_area = match surface {
        SurfaceMode::Overlay => top_aligned_panel_rect(content[0], nav_height),
        SurfaceMode::Route => content[0],
    };
    render_list_panel_state(frame, nav_area, &spec.nav_panel_state);

    let right_areas = match surface {
        SurfaceMode::Overlay => {
            top_aligned_vertical_areas(content[1], &[section_height, items_height, detail_height])
        }
        SurfaceMode::Route => split_vertical_sections(
            content[1],
            &[
                VerticalSectionSize::Fixed(section_height),
                VerticalSectionSize::Flexible(items_height),
                VerticalSectionSize::Fixed(detail_height),
            ],
        ),
    };
    render_text_panel(
        frame,
        right_areas[0],
        &TextPanelSpec {
            title: Some(spec.section_panel.title.clone()),
            body: &spec.section_panel.body,
            wrap: true,
            scroll: None,
            alignment: None,
        },
    );

    render_list_panel_state(frame, right_areas[1], &spec.items_panel_state);

    render_text_panel(
        frame,
        right_areas[2],
        &TextPanelSpec {
            title: Some(spec.detail_panel.title.clone()),
            body: &spec.detail_panel.body,
            wrap: true,
            scroll: None,
            alignment: None,
        },
    );

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.footer.as_ref()).wrap(Wrap { trim: false }),
            rows[1],
        );
    }
}
