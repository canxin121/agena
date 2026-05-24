use std::{borrow::Cow, cmp::max};

use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Text,
    widgets::{Paragraph, Wrap},
};

use crate::{
    DetailTextDialogSpec, DetailTextLine, FramedSurfaceSpec, WorkbenchOverlayDialogSpec,
    WorkbenchOverlaySource,
    layout::{
        SurfaceMode, VerticalSectionSize, adaptive_detail_split, framed_sections_target_height,
        list_panel_height, optional_overlay_text_height, should_stack_detail_layout,
        split_vertical_sections, top_aligned_panel_rect, top_aligned_vertical_areas,
    },
    panels::{
        ListPanelHeightResolver, ListPanelSpec, ListPanelState, TextPanelSpec,
        render_list_panel_state, render_text_panel,
    },
    render_detail_text_dialog, render_editor_dialog, render_framed_surface,
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
        top_panel: DashboardTextSection<'a>,
        bottom_panels: DashboardSplitPanelsSpec<'a>,
    ) -> Self {
        Self {
            title,
            footer,
            target_width,
            lead_panel: None,
            top_panel,
            bottom_panels,
        }
    }

    pub fn with_lead_panel(mut self, lead_panel: DashboardLeadPanelSpec<'a>) -> Self {
        self.lead_panel = Some(lead_panel);
        self
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

    pub fn with_optional_editor_source<TSource>(mut self, source: Option<&'a TSource>) -> Self
    where
        TSource: WorkbenchOverlaySource + ?Sized,
    {
        self.editor = source.map(WorkbenchOverlayDialogSpec::from_source);
        self
    }
}

pub fn render_dashboard_workbench(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &DashboardWorkbenchSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let footer_height = optional_overlay_text_height(spec.footer.as_ref(), content_width, 1, 2);
    let right_width = content_width_without_lead(content_width, spec.lead_panel.as_ref());
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
    let content_height = max(lead_height, right_total_height);
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

    let right_area = if let Some(lead_panel) = spec.lead_panel.as_ref() {
        let content = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Length(lead_panel.width),
                Constraint::Min(lead_panel.min_right_width),
            ])
            .split(rows[0]);
        let lead_area = top_aligned_panel_rect(content[0], lead_panel.panel.resolve_height());
        render_dashboard_list_panel(frame, lead_area, &lead_panel.panel);
        content[1]
    } else {
        rows[0]
    };
    let right_rows = top_aligned_vertical_areas(right_area, &[top_panel_height, bottom_height]);
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
        (
            top_aligned_panel_rect(split[0], left_height),
            top_aligned_panel_rect(split[1], right_height),
        )
    };
    render_dashboard_list_panel(frame, left_area, &spec.bottom_panels.left);
    render_dashboard_list_panel(frame, right_area, &spec.bottom_panels.right);

    if footer_height > 0 {
        frame.render_widget(
            Paragraph::new(spec.footer.as_ref()).wrap(Wrap { trim: false }),
            rows[1],
        );
    }
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
) -> u16 {
    lead_panel
        .map(|panel| {
            content_width
                .saturating_sub(panel.width)
                .max(panel.min_right_width)
        })
        .unwrap_or(content_width)
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
    use super::*;
    use crate::Editor;
    use ratatui::{style::Style, text::Text, widgets::ListItem};

    #[test]
    fn dashboard_bottom_content_height_stacks_when_space_is_tight() {
        assert_eq!(dashboard_bottom_content_height(6, 8, 32, 24, 28), 14);
    }

    #[test]
    fn dashboard_bottom_content_height_uses_max_when_split_horizontally() {
        assert_eq!(dashboard_bottom_content_height(6, 8, 80, 24, 28), 8);
    }

    #[test]
    fn dashboard_text_panel_height_auto_uses_body_content() {
        let body = Text::from("alpha beta gamma");

        let height = DashboardTextPanelHeight::AutoBody {
            min_body_height: 4,
            max_body_height: 16,
        }
        .resolve(&body, 10);

        assert_eq!(height, 6);
    }

    #[test]
    fn dashboard_list_panel_height_auto_uses_item_count() {
        let items = Vec::new();
        let panel = ListPanelSpec {
            title: None,
            items: items.as_slice(),
            selected: None,
            highlight_style: Style::default(),
            highlight_symbol: "> ".into(),
        };

        let height = DashboardListPanelHeight::AutoBody {
            lines_per_item: 2,
            min_body_height: 4,
            max_body_height: 12,
        }
        .resolve(&panel);

        assert_eq!(height, 4);
    }

    #[test]
    fn dashboard_list_panel_state_preserves_placeholder_and_item_variants() {
        let loading = DashboardListPanelState::Loading {
            title: Some("Adapters".into()),
            message: "Loading".into(),
            panel_height: DashboardListPanelHeight::AutoBody {
                lines_per_item: 1,
                min_body_height: 4,
                max_body_height: 10,
            },
        };
        let items = DashboardListPanelState::Items {
            panel_height: DashboardListPanelHeight::AutoBody {
                lines_per_item: 2,
                min_body_height: 4,
                max_body_height: 10,
            },
            panel: ListPanelSpec {
                title: Some("Models".into()),
                items: &[ListItem::new("one")],
                selected: Some(0),
                highlight_style: Style::default(),
                highlight_symbol: "> ".into(),
            },
        };

        assert_eq!(loading.resolve_height(), 3);
        assert_eq!(items.resolve_height(), 5);
    }

    #[test]
    fn dashboard_overlay_spec_preserves_detail_and_editor() {
        let editor = crate::EditorDialogState::new(
            "Title".to_string(),
            "Prompt".to_string(),
            "Footer".to_string(),
            true,
            Editor::from_text("value".to_string()),
            (),
        );
        let overlay = DashboardWorkbenchOverlaySpec {
            detail: Some(DashboardDetailOverlaySpec {
                dialog: DetailTextDialogSpec {
                    title: "Detail".into(),
                    footer: None,
                    target_width: 72,
                    detail_spec: crate::DetailTextSpec::with_label_width(8),
                    body_height_bounds: (4, 12),
                    footer_height_bounds: (0, 0),
                    footer_alignment: None,
                    footer_style: Style::default(),
                },
                lines: vec![DetailTextLine::plain("body", Style::default())],
            }),
            editor: None,
        }
        .with_optional_editor_source(Some(&editor));

        assert_eq!(
            overlay.detail.as_ref().map(|detail| detail.lines.len()),
            Some(1)
        );
        assert!(
            overlay
                .editor
                .as_ref()
                .is_some_and(|dialog| dialog.dialog.multiline)
        );
    }
}
