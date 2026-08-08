//! Workbench widget: content area with overlays.

use std::{borrow::Cow, cmp::max};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Text,
};

use crate::{
    Editor, EditorDialogSpec, EditorDialogState, InputDialogState, WorkbenchFrameSpec,
    bordered_text_height,
    layout::{
        SurfaceMode, VerticalSectionSize, adaptive_detail_split, estimated_horizontal_panel_widths,
        should_stack_detail_layout, split_vertical_sections, top_aligned_panel_rect,
        top_aligned_vertical_areas,
    },
    panels::{
        BoundedListPanelHeight, ListPanelState, TextPanelSpec, render_list_panel_state,
        render_text_panel,
    },
    render_editor_dialog, render_workbench_frame,
};

#[derive(Clone)]
/// A text section of the workbench.
pub struct WorkbenchTextSection<'a> {
    pub title: Cow<'a, str>,
    pub body: Text<'a>,
    pub min_body_height: u16,
    pub max_body_height: u16,
    pub wrap: bool,
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
            wrap: true,
        }
    }

    pub fn wrap(mut self, wrap: bool) -> Self {
        self.wrap = wrap;
        self
    }

    fn resolve_height(&self, width: u16) -> u16 {
        if self.wrap {
            bordered_text_height(
                &self.body,
                width,
                self.min_body_height,
                self.max_body_height,
            )
        } else {
            u16::try_from(self.body.lines.len())
                .unwrap_or(u16::MAX)
                .max(1)
                .clamp(self.min_body_height, self.max_body_height)
                .saturating_add(2)
        }
    }
}

struct TwoPaneWorkbenchSpec<'a> {
    pub title: Cow<'a, str>,
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub left_panel_width: u16,
    pub left_min_width: Option<u16>,
    pub right_min_width: Option<u16>,
    pub left_panel_state: ListWorkbenchPanelState<'a>,
    pub right_sections: Vec<WorkbenchTextSection<'a>>,
}

/// Spec of a workbench overlay dialog.
pub struct WorkbenchOverlayDialogSpec<'a> {
    pub(crate) dialog: EditorDialogSpec<'a>,
    pub(crate) input: &'a Editor,
}

/// Source of workbench overlays.
pub trait WorkbenchOverlaySource {
    fn to_workbench_overlay_dialog_spec<'a>(&'a self) -> WorkbenchOverlayDialogSpec<'a>;
}

/// Spec of the list workbench dialog.
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

const MIN_TWO_PANE_DETAIL_WIDTH: u16 = 28;
const MIN_SECTIONED_CONTENT_WIDTH: u16 = 32;

impl<'a> ListWorkbenchDialogSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        footer: Cow<'a, str>,
        left_panel_state: ListWorkbenchPanelState<'a>,
        right_sections: Vec<WorkbenchTextSection<'a>>,
    ) -> Self {
        Self {
            title,
            summary: None,
            footer,
            target_width: 140,
            left_panel_width: 36,
            left_min_width: None,
            right_min_width: None,
            left_panel_state,
            right_sections,
            overlay: None,
        }
    }

    pub fn summary(mut self, summary: Option<Cow<'a, str>>) -> Self {
        self.summary = summary;
        self
    }

    pub fn target_width(mut self, target_width: u16) -> Self {
        self.target_width = target_width;
        self
    }

    pub fn left_panel_width(mut self, left_panel_width: u16) -> Self {
        self.left_panel_width = left_panel_width;
        self
    }

    pub fn adaptive_panel_widths(mut self, left_min_width: u16, right_min_width: u16) -> Self {
        self.left_min_width = Some(left_min_width);
        self.right_min_width = Some(right_min_width);
        self
    }

    pub fn overlay(mut self, overlay: Option<WorkbenchOverlayDialogSpec<'a>>) -> Self {
        self.overlay = overlay;
        self
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
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub nav_panel_width: u16,
    pub nav_panel_state: ListWorkbenchPanelState<'a>,
    pub section_panel: WorkbenchTextSection<'a>,
    pub items_panel_state: ListWorkbenchPanelState<'a>,
    pub detail_panel: WorkbenchTextSection<'a>,
}

/// Spec of the sectioned workbench dialog.
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
    pub fn new(
        title: Cow<'a, str>,
        footer: Cow<'a, str>,
        nav_panel_state: ListWorkbenchPanelState<'a>,
        section_panel: WorkbenchTextSection<'a>,
        items_panel_state: ListWorkbenchPanelState<'a>,
        detail_panel: WorkbenchTextSection<'a>,
    ) -> Self {
        Self {
            title,
            summary: None,
            footer,
            target_width: 140,
            nav_panel_width: 24,
            nav_panel_state,
            section_panel,
            items_panel_state,
            detail_panel,
        }
    }

    pub fn summary(mut self, summary: Option<Cow<'a, str>>) -> Self {
        self.summary = summary;
        self
    }

    pub fn target_width(mut self, target_width: u16) -> Self {
        self.target_width = target_width;
        self
    }

    pub fn navigation_width(mut self, nav_panel_width: u16) -> Self {
        self.nav_panel_width = nav_panel_width;
        self
    }
}

fn render_two_pane_workbench(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &TwoPaneWorkbenchSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let stacked = match (spec.left_min_width, spec.right_min_width) {
        (Some(left_min), Some(right_min)) => {
            should_stack_detail_layout(content_width, left_min, right_min)
        }
        _ => should_stack_detail_layout(
            content_width,
            spec.left_panel_width,
            MIN_TWO_PANE_DETAIL_WIDTH,
        ),
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
        .map(|section| section.resolve_height(right_width))
        .collect::<Vec<_>>();
    let right_total_height = right_section_heights.iter().copied().sum::<u16>();
    let content_height = if stacked {
        left_panel_height.saturating_add(right_total_height)
    } else {
        max(left_panel_height, right_total_height)
    };
    let workbench = render_workbench_frame(
        frame,
        area,
        surface,
        &WorkbenchFrameSpec::new(
            spec.title.clone(),
            spec.footer.clone(),
            spec.target_width,
            content_height,
        )
        .summary(spec.summary.clone()),
    );
    let (list_area, section_areas) = if stacked {
        let mut heights = Vec::with_capacity(1 + right_section_heights.len());
        heights.push(left_panel_height);
        heights.extend(right_section_heights.iter().copied());
        let areas = match surface {
            SurfaceMode::Overlay => top_aligned_vertical_areas(workbench.body, &heights),
            SurfaceMode::Route => split_vertical_sections(
                workbench.body,
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
                    adaptive_detail_split(workbench.body.width, left_min, right_min).to_vec()
                }
                _ => vec![
                    Constraint::Length(spec.left_panel_width),
                    Constraint::Min(1),
                ],
            })
            .split(workbench.body);
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
                wrap: section.wrap,
                scroll: None,
                alignment: None,
            },
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
    render_two_pane_workbench_dialog(
        frame,
        area,
        surface,
        &TwoPaneWorkbenchSpec {
            title: spec.title.clone(),
            summary: spec.summary.clone(),
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
    render_sectioned_workbench(
        frame,
        area,
        surface,
        &SectionedWorkbenchSpec {
            title: spec.title.clone(),
            summary: spec.summary.clone(),
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
    let stacked = should_stack_detail_layout(
        content_width,
        spec.nav_panel_width,
        MIN_SECTIONED_CONTENT_WIDTH,
    );
    let right_width = if stacked {
        content_width
    } else {
        content_width.saturating_sub(spec.nav_panel_width)
    }
    .max(1);
    let nav_height = spec.nav_panel_state.resolve_height();
    let section_height = spec.section_panel.resolve_height(right_width);
    let items_height = spec.items_panel_state.resolve_height();
    let detail_height = spec.detail_panel.resolve_height(right_width);
    let right_height = section_height
        .saturating_add(items_height)
        .saturating_add(detail_height);
    let content_height = if stacked {
        nav_height.saturating_add(right_height)
    } else {
        max(nav_height, right_height)
    };
    let workbench = render_workbench_frame(
        frame,
        area,
        surface,
        &WorkbenchFrameSpec::new(
            spec.title.clone(),
            spec.footer.clone(),
            spec.target_width,
            content_height,
        )
        .summary(spec.summary.clone()),
    );
    let (nav_area, right_area) = if stacked {
        let content = match surface {
            SurfaceMode::Overlay => {
                top_aligned_vertical_areas(workbench.body, &[nav_height, right_height])
            }
            SurfaceMode::Route => split_vertical_sections(
                workbench.body,
                &[
                    VerticalSectionSize::Fixed(nav_height),
                    VerticalSectionSize::Flexible(right_height),
                ],
            ),
        };
        (content[0], content[1])
    } else {
        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Length(spec.nav_panel_width), Constraint::Min(1)])
            .split(workbench.body);
        let nav_area = match surface {
            SurfaceMode::Overlay => top_aligned_panel_rect(content[0], nav_height),
            SurfaceMode::Route => content[0],
        };
        (nav_area, content[1])
    };
    render_list_panel_state(frame, nav_area, &spec.nav_panel_state);

    let right_areas = match surface {
        SurfaceMode::Overlay => {
            top_aligned_vertical_areas(right_area, &[section_height, items_height, detail_height])
        }
        SurfaceMode::Route => split_vertical_sections(
            right_area,
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
            wrap: spec.section_panel.wrap,
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
            wrap: spec.detail_panel.wrap,
            scroll: None,
            alignment: None,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::{
        BoundedListPanelHeight, ListWorkbenchDialogSpec, ListWorkbenchPanelState,
        MIN_SECTIONED_CONTENT_WIDTH, MIN_TWO_PANE_DETAIL_WIDTH, WorkbenchTextSection,
        render_list_workbench_dialog,
    };
    use crate::{SurfaceMode, layout::should_stack_detail_layout};
    use ratatui::{Terminal, backend::TestBackend, text::Text, widgets::ListItem};

    fn panel_title_positions(width: u16) -> ((usize, usize), (usize, usize)) {
        let backend = TestBackend::new(width, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let items = [ListItem::new("item")];
                let spec = ListWorkbenchDialogSpec::new(
                    "Workbench".into(),
                    "Esc close".into(),
                    ListWorkbenchPanelState::items(
                        BoundedListPanelHeight {
                            lines_per_item: 1,
                            min_body_height: 2,
                            max_body_height: 4,
                        },
                        Some("NavigationPanel".into()),
                        &items,
                        Some(0),
                        Default::default(),
                        ">> ".into(),
                    ),
                    vec![WorkbenchTextSection::new(
                        "DetailPanel".into(),
                        Text::from("detail"),
                        2,
                        4,
                    )],
                )
                .left_panel_width(36);
                render_list_workbench_dialog(frame, frame.area(), SurfaceMode::Route, &spec);
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let find = |needle: &str| {
            (0..buffer.area.height)
                .find_map(|y| {
                    let row = (0..buffer.area.width)
                        .map(|x| buffer[(x, y)].symbol())
                        .collect::<String>();
                    row.find(needle).map(|x| (x, usize::from(y)))
                })
                .unwrap()
        };
        (find("NavigationPanel"), find("DetailPanel"))
    }

    #[test]
    fn fixed_workbench_stacks_on_narrow_terminals_and_splits_when_wide() {
        let (narrow_navigation, narrow_detail) = panel_title_positions(50);
        assert!(narrow_navigation.1 < narrow_detail.1);

        let (wide_navigation, wide_detail) = panel_title_positions(100);
        assert_eq!(wide_navigation.1, wide_detail.1);
        assert!(wide_navigation.0 < wide_detail.0);
    }

    #[test]
    fn sectioned_and_fixed_workbenches_share_explicit_content_thresholds() {
        assert!(should_stack_detail_layout(
            50,
            24,
            MIN_SECTIONED_CONTENT_WIDTH
        ));
        assert!(!should_stack_detail_layout(
            100,
            24,
            MIN_SECTIONED_CONTENT_WIDTH
        ));
        assert!(should_stack_detail_layout(
            50,
            36,
            MIN_TWO_PANE_DETAIL_WIDTH
        ));
    }

    #[test]
    fn non_wrapping_table_sections_measure_physical_rows_only() {
        let wrapped = WorkbenchTextSection::new(
            "Table".into(),
            Text::from("a very long fixed-width table row"),
            1,
            20,
        );
        let unwrapped = wrapped.clone().wrap(false);

        assert!(wrapped.resolve_height(10) > unwrapped.resolve_height(10));
        assert_eq!(unwrapped.resolve_height(10), 3);
    }
}
