//! Dashboard workbench widgets (panels, overlays, sections).

use std::{borrow::Cow, cmp::max};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Text,
};

use crate::{
    DetailTextDialogSpec, DetailTextLine, WorkbenchFrameSpec, WorkbenchOverlayDialogSpec,
    WorkbenchOverlaySource,
    layout::{
        SurfaceMode, VerticalSectionSize, adaptive_detail_split, list_panel_height,
        should_stack_detail_layout, split_vertical_sections, top_aligned_panel_rect,
        top_aligned_vertical_areas,
    },
    panels::{
        ListPanelHeightResolver, ListPanelSpec, ListPanelState, TextPanelSpec,
        render_list_panel_state, render_text_panel,
    },
    render_detail_text_dialog, render_editor_dialog, render_workbench_frame,
    text::bordered_text_height,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardTextPanelHeight {
    Fixed(u16),
    AutoBody {
        min_body_height: u16,
        max_body_height: u16,
    },
}

impl DashboardTextPanelHeight {
    fn resolve(self, body: &Text<'_>, width: u16) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoBody {
                min_body_height,
                max_body_height,
            } => bordered_text_height(body, width, min_body_height, max_body_height),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DashboardListPanelHeight {
    Fixed(u16),
    AutoBody {
        lines_per_item: u16,
        min_body_height: u16,
        max_body_height: u16,
    },
}

impl DashboardListPanelHeight {
    fn resolve(self, panel: &ListPanelSpec<'_>) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoBody {
                lines_per_item,
                min_body_height,
                max_body_height,
            } => list_panel_height(
                panel.items.len(),
                lines_per_item,
                min_body_height,
                max_body_height,
            ),
        }
    }

    fn resolve_placeholder(self) -> u16 {
        match self {
            Self::Fixed(height) => height,
            Self::AutoBody {
                min_body_height,
                max_body_height,
                ..
            } => list_panel_height(1, 1, min_body_height, max_body_height),
        }
    }
}

impl ListPanelHeightResolver for DashboardListPanelHeight {
    fn resolve_placeholder_height(&self) -> u16 {
        self.resolve_placeholder()
    }

    fn resolve_items_height(&self, panel: &ListPanelSpec<'_>) -> u16 {
        self.resolve(panel)
    }
}

pub struct DashboardLeadPanelSpec<'a> {
    pub width: u16,
    pub min_right_width: u16,
    pub panel: DashboardListPanelState<'a>,
}

impl<'a> DashboardLeadPanelSpec<'a> {
    pub fn new(width: u16, min_right_width: u16, panel: DashboardListPanelState<'a>) -> Self {
        Self {
            width,
            min_right_width,
            panel,
        }
    }
}

pub struct DashboardSplitPanelsSpec<'a> {
    pub left_min_width: u16,
    pub right_min_width: u16,
    pub left: DashboardListPanelState<'a>,
    pub right: DashboardListPanelState<'a>,
}

impl<'a> DashboardSplitPanelsSpec<'a> {
    pub fn new(
        left_min_width: u16,
        right_min_width: u16,
        left: DashboardListPanelState<'a>,
        right: DashboardListPanelState<'a>,
    ) -> Self {
        Self {
            left_min_width,
            right_min_width,
            left,
            right,
        }
    }
}

pub struct DashboardTextSection<'a> {
    pub title: Option<Cow<'a, str>>,
    pub body: Text<'a>,
    pub height: DashboardTextPanelHeight,
}

impl<'a> DashboardTextSection<'a> {
    pub fn new(
        title: Option<Cow<'a, str>>,
        body: Text<'a>,
        height: DashboardTextPanelHeight,
    ) -> Self {
        Self {
            title,
            body,
            height,
        }
    }
}

pub struct DashboardWorkbenchSpec<'a> {
    pub title: Cow<'a, str>,
    pub footer: Cow<'a, str>,
    pub target_width: u16,
    pub lead_panel: Option<DashboardLeadPanelSpec<'a>>,
    pub top_panel: DashboardTextSection<'a>,
    pub bottom_panels: DashboardSplitPanelsSpec<'a>,
}

impl<'a> DashboardWorkbenchSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        footer: Cow<'a, str>,
        target_width: u16,
        lead_panel: Option<DashboardLeadPanelSpec<'a>>,
        top_panel: DashboardTextSection<'a>,
        bottom_panels: DashboardSplitPanelsSpec<'a>,
    ) -> Self {
        Self {
            title,
            footer,
            target_width,
            lead_panel,
            top_panel,
            bottom_panels,
        }
    }
}

pub type DashboardListPanelState<'a> = ListPanelState<'a, DashboardListPanelHeight>;

pub struct DashboardDetailOverlaySpec<'a> {
    pub dialog: DetailTextDialogSpec<'a>,
    pub lines: Vec<DetailTextLine<'a>>,
}

impl<'a> DashboardDetailOverlaySpec<'a> {
    pub fn new(dialog: DetailTextDialogSpec<'a>, lines: Vec<DetailTextLine<'a>>) -> Self {
        Self { dialog, lines }
    }
}

pub struct DashboardWorkbenchOverlaySpec<'a> {
    pub detail: Option<DashboardDetailOverlaySpec<'a>>,
    pub editor: Option<WorkbenchOverlayDialogSpec<'a>>,
}

impl<'a> DashboardWorkbenchOverlaySpec<'a> {
    pub fn new(
        detail: Option<DashboardDetailOverlaySpec<'a>>,
        editor: Option<WorkbenchOverlayDialogSpec<'a>>,
    ) -> Self {
        Self { detail, editor }
    }

    pub fn from_sources<TSource>(
        detail: Option<DashboardDetailOverlaySpec<'a>>,
        editor_source: Option<&'a TSource>,
    ) -> Self
    where
        TSource: WorkbenchOverlaySource + ?Sized,
    {
        Self {
            detail,
            editor: editor_source.map(WorkbenchOverlayDialogSpec::from_source),
        }
    }
}

pub fn render_dashboard_workbench(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &DashboardWorkbenchSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let lead_stacked = spec.lead_panel.as_ref().is_some_and(|panel| {
        should_stack_detail_layout(content_width, panel.width, panel.min_right_width)
    });
    let right_width =
        content_width_without_lead(content_width, spec.lead_panel.as_ref(), lead_stacked);
    let top_panel_height = spec
        .top_panel
        .height
        .resolve(&spec.top_panel.body, right_width);
    let left_height = spec.bottom_panels.left.resolve_height();
    let right_height = spec.bottom_panels.right.resolve_height();
    let bottom_height = dashboard_bottom_content_height(
        left_height,
        right_height,
        right_width,
        spec.bottom_panels.left_min_width,
        spec.bottom_panels.right_min_width,
    );
    let right_total_height = top_panel_height.saturating_add(bottom_height);
    let lead_height = spec
        .lead_panel
        .as_ref()
        .map(|panel| panel.panel.resolve_height())
        .unwrap_or(0);
    let content_height = if lead_stacked {
        lead_height.saturating_add(right_total_height)
    } else {
        max(lead_height, right_total_height)
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
        ),
    );

    let right_area = if let Some(lead_panel) = spec.lead_panel.as_ref() {
        if lead_stacked {
            let content = match surface {
                SurfaceMode::Overlay => {
                    top_aligned_vertical_areas(workbench.body, &[lead_height, right_total_height])
                }
                SurfaceMode::Route => split_vertical_sections(
                    workbench.body,
                    &[
                        VerticalSectionSize::Fixed(lead_height),
                        VerticalSectionSize::Flexible(right_total_height),
                    ],
                ),
            };
            render_dashboard_list_panel(frame, content[0], &lead_panel.panel);
            content[1]
        } else {
            let content = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([
                    Constraint::Length(lead_panel.width),
                    Constraint::Min(lead_panel.min_right_width),
                ])
                .split(workbench.body);
            let lead_area = match surface {
                SurfaceMode::Overlay => {
                    top_aligned_panel_rect(content[0], lead_panel.panel.resolve_height())
                }
                SurfaceMode::Route => content[0],
            };
            render_dashboard_list_panel(frame, lead_area, &lead_panel.panel);
            content[1]
        }
    } else {
        workbench.body
    };
    let right_rows = match surface {
        SurfaceMode::Overlay => {
            top_aligned_vertical_areas(right_area, &[top_panel_height, bottom_height])
        }
        SurfaceMode::Route => split_vertical_sections(
            right_area,
            &[
                VerticalSectionSize::Fixed(top_panel_height),
                VerticalSectionSize::Flexible(bottom_height),
            ],
        ),
    };
    render_text_panel(
        frame,
        right_rows[0],
        &TextPanelSpec {
            title: spec.top_panel.title.clone(),
            body: &spec.top_panel.body,
            wrap: true,
            scroll: None,
            alignment: None,
        },
    );

    let bottom_width = right_rows[1].width;
    let stacked = should_stack_detail_layout(
        bottom_width,
        spec.bottom_panels.left_min_width,
        spec.bottom_panels.right_min_width,
    );
    let split = if stacked {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(left_height),
                Constraint::Length(right_height),
            ])
            .split(right_rows[1])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints(adaptive_detail_split(
                bottom_width,
                spec.bottom_panels.left_min_width,
                spec.bottom_panels.right_min_width,
            ))
            .split(right_rows[1])
    };
    let (left_area, right_area) = if stacked {
        (split[0], split[1])
    } else {
        match surface {
            SurfaceMode::Overlay => (
                top_aligned_panel_rect(split[0], left_height),
                top_aligned_panel_rect(split[1], right_height),
            ),
            SurfaceMode::Route => (split[0], split[1]),
        }
    };
    render_dashboard_list_panel(frame, left_area, &spec.bottom_panels.left);
    render_dashboard_list_panel(frame, right_area, &spec.bottom_panels.right);
}

fn render_dashboard_list_panel(frame: &mut Frame, area: Rect, state: &DashboardListPanelState<'_>) {
    render_list_panel_state(frame, area, state);
}

pub fn render_dashboard_workbench_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &DashboardWorkbenchSpec<'_>,
    overlay: Option<DashboardWorkbenchOverlaySpec<'_>>,
) {
    render_dashboard_workbench(frame, area, surface, spec);

    if let Some(overlay) = overlay {
        if let Some(detail) = overlay.detail {
            render_detail_text_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &detail.dialog,
                detail.lines,
            );
        }
        if let Some(editor) = overlay.editor {
            render_editor_dialog(
                frame,
                area,
                SurfaceMode::Overlay,
                &editor.dialog,
                editor.input,
            );
        }
    }
}

fn content_width_without_lead(
    content_width: u16,
    lead_panel: Option<&DashboardLeadPanelSpec<'_>>,
    lead_stacked: bool,
) -> u16 {
    match (lead_panel, lead_stacked) {
        (Some(_), true) | (None, _) => content_width,
        (Some(panel), false) => content_width.saturating_sub(panel.width),
    }
}

fn dashboard_bottom_content_height(
    left_height: u16,
    right_height: u16,
    width: u16,
    left_min_width: u16,
    right_min_width: u16,
) -> u16 {
    if should_stack_detail_layout(width, left_min_width, right_min_width) {
        left_height.saturating_add(right_height)
    } else {
        max(left_height, right_height)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DashboardLeadPanelSpec, DashboardListPanelHeight, DashboardListPanelState,
        DashboardSplitPanelsSpec, DashboardTextPanelHeight, DashboardTextSection,
        DashboardWorkbenchSpec, content_width_without_lead, render_dashboard_workbench,
        should_stack_detail_layout,
    };
    use crate::SurfaceMode;
    use ratatui::{Terminal, backend::TestBackend, text::Text};

    fn lead_panel() -> DashboardLeadPanelSpec<'static> {
        DashboardLeadPanelSpec::new(
            28,
            54,
            DashboardListPanelState::empty(
                None,
                "empty".into(),
                DashboardListPanelHeight::Fixed(3),
            ),
        )
    }

    #[test]
    fn stacked_lead_panel_returns_the_full_width_to_dashboard_content() {
        let panel = lead_panel();

        assert!(should_stack_detail_layout(
            60,
            panel.width,
            panel.min_right_width
        ));
        assert!(!should_stack_detail_layout(
            120,
            panel.width,
            panel.min_right_width
        ));
        assert_eq!(content_width_without_lead(60, Some(&panel), true), 60);
        assert_eq!(content_width_without_lead(120, Some(&panel), false), 92);
        assert_eq!(content_width_without_lead(60, None, false), 60);
    }

    fn panel_title_positions(width: u16) -> ((usize, usize), (usize, usize)) {
        let backend = TestBackend::new(width, 36);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                let empty_panel = |title: &'static str| {
                    DashboardListPanelState::empty(
                        Some(title.into()),
                        "empty".into(),
                        DashboardListPanelHeight::Fixed(4),
                    )
                };
                let spec = DashboardWorkbenchSpec::new(
                    "Dashboard".into(),
                    "Esc close".into(),
                    width,
                    Some(DashboardLeadPanelSpec::new(
                        28,
                        54,
                        empty_panel("ProvidersPanel"),
                    )),
                    DashboardTextSection::new(
                        Some("TopPanel".into()),
                        Text::from("top"),
                        DashboardTextPanelHeight::Fixed(4),
                    ),
                    DashboardSplitPanelsSpec::new(
                        24,
                        28,
                        empty_panel("LeftPanel"),
                        empty_panel("RightPanel"),
                    ),
                );
                render_dashboard_workbench(frame, frame.area(), SurfaceMode::Route, &spec);
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
        (find("ProvidersPanel"), find("TopPanel"))
    }

    #[test]
    fn dashboard_lead_panel_stacks_when_narrow_and_splits_when_wide() {
        let (narrow_lead, narrow_top) = panel_title_positions(60);
        assert!(narrow_lead.1 < narrow_top.1);

        let (wide_lead, wide_top) = panel_title_positions(120);
        assert_eq!(wide_lead.1, wide_top.1);
        assert!(wide_lead.0 < wide_top.0);
    }
}
