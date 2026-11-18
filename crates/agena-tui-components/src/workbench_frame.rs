//! Workbench frame drawing.

use std::borrow::Cow;

use ratatui::{Frame, layout::Rect};

use crate::{
    FramedSurface, FramedSurfaceSpec,
    layout::{
        SurfaceMode, VerticalSectionSize, framed_sections_target_height, split_vertical_sections,
    },
    render_framed_surface, title_with_summary,
};

/// Shared chrome for full-screen and modal workbenches.
///
/// Keeping title composition, footer sizing, and surface framing here prevents
/// settings-style screens from drifting as each screen grows new panels.
pub struct WorkbenchFrameSpec<'a> {
    pub title: Cow<'a, str>,
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
    pub footer_max_height: u16,
    pub target_width: u16,
    pub body_height: u16,
}

impl<'a> WorkbenchFrameSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        footer: Cow<'a, str>,
        target_width: u16,
        body_height: u16,
    ) -> Self {
        Self {
            title,
            summary: None,
            footer,
            footer_max_height: 2,
            target_width,
            body_height,
        }
    }

    pub fn footer_max_height(mut self, height: u16) -> Self {
        self.footer_max_height = height.max(1);
        self
    }

    pub fn summary(mut self, summary: Option<Cow<'a, str>>) -> Self {
        self.summary = summary;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Frame of a workbench.
pub struct WorkbenchFrame {
    pub outer: Rect,
    pub body: Rect,
    pub footer: Option<Rect>,
}

pub fn render_workbench_frame(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &WorkbenchFrameSpec<'_>,
) -> WorkbenchFrame {
    let content_width = surface.content_width(area, spec.target_width);
    let footer_height =
        crate::shortcut_bar::shortcut_footer_height(spec.footer.as_ref(), content_width)
            .min(spec.footer_max_height);
    let mut sections = vec![VerticalSectionSize::Flexible(spec.body_height.max(1))];
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }

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
            .to_owned()
        })
        .unwrap_or_else(|| spec.title.trim().to_owned());
    let FramedSurface { outer, inner } = render_framed_surface(
        frame,
        area,
        surface,
        &FramedSurfaceSpec {
            title: Cow::Owned(title),
            target_width: spec.target_width,
            target_height: framed_sections_target_height(&sections),
        },
    );
    // The natural body height sizes a modal, but is not a minimum once
    // the terminal has constrained the surface. Reserve the footer before
    // applying the flexible-body constraint so long lists cannot consume it.
    let footer_height = footer_height.min(inner.height.saturating_sub(1));
    sections[0] = VerticalSectionSize::Flexible(
        spec.body_height
            .max(1)
            .min(inner.height.saturating_sub(footer_height)),
    );
    if let Some(footer_section) = sections.get_mut(1) {
        *footer_section = VerticalSectionSize::Fixed(footer_height);
    }
    let rows = split_vertical_sections(inner, &sections);
    let body = rows.first().copied().unwrap_or(inner);
    let footer = if footer_height > 0 {
        rows.get(1).copied()
    } else {
        None
    };
    if let Some(footer_area) = footer {
        crate::render_shortcut_footer(frame, footer_area, spec.footer.as_ref());
    }

    WorkbenchFrame {
        outer,
        body,
        footer,
    }
}

/// Standard responsive width for a settings/workbench navigation rail.
pub fn workbench_navigation_width(total_width: u16) -> u16 {
    total_width
        .saturating_mul(22)
        .saturating_div(100)
        .clamp(18, 28)
        .min(total_width.saturating_sub(1))
}
