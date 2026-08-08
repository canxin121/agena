//! Workbench frame drawing.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Paragraph, Wrap},
};

use crate::{
    FramedSurface, FramedSurfaceSpec,
    layout::{
        SurfaceMode, VerticalSectionSize, framed_sections_target_height,
        optional_overlay_text_height, split_vertical_sections,
    },
    render_framed_surface,
    theme::muted_style,
    title_with_summary,
};

/// Shared chrome for full-screen and modal workbenches.
///
/// Keeping title composition, footer sizing, and surface framing here prevents
/// settings-style screens from drifting as each screen grows new panels.
pub struct WorkbenchFrameSpec<'a> {
    pub title: Cow<'a, str>,
    pub summary: Option<Cow<'a, str>>,
    pub footer: Cow<'a, str>,
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
            target_width,
            body_height,
        }
    }

    pub fn summary(mut self, summary: Option<Cow<'a, str>>) -> Self {
        self.summary = summary;
        self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
    let footer_height = optional_overlay_text_height(spec.footer.as_ref(), content_width, 1, 2);
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
    let rows = split_vertical_sections(inner, &sections);
    let body = rows.first().copied().unwrap_or(inner);
    let footer = if footer_height > 0 {
        rows.get(1).copied()
    } else {
        None
    };
    if let Some(footer_area) = footer {
        frame.render_widget(
            Paragraph::new(spec.footer.as_ref())
                .style(muted_style())
                .wrap(Wrap { trim: false }),
            footer_area,
        );
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

#[cfg(test)]
mod tests {
    use super::{WorkbenchFrameSpec, render_workbench_frame, workbench_navigation_width};
    use crate::SurfaceMode;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn navigation_width_is_proportional_and_bounded() {
        assert_eq!(workbench_navigation_width(0), 0);
        assert_eq!(workbench_navigation_width(10), 9);
        assert_eq!(workbench_navigation_width(80), 18);
        assert_eq!(workbench_navigation_width(100), 22);
        assert_eq!(workbench_navigation_width(140), 28);
        assert_eq!(workbench_navigation_width(200), 28);
    }

    #[test]
    fn frame_composes_summary_and_reserves_a_shared_footer() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rendered_footer = None;
        terminal
            .draw(|frame| {
                let workbench = render_workbench_frame(
                    frame,
                    frame.area(),
                    SurfaceMode::Route,
                    &WorkbenchFrameSpec::new("Settings".into(), "Esc close".into(), 60, 8)
                        .summary(Some("Interface".into())),
                );
                rendered_footer = workbench.footer;
            })
            .unwrap();

        let rendered =
            terminal
                .backend()
                .buffer()
                .content()
                .iter()
                .fold(String::new(), |mut output, cell| {
                    output.push_str(cell.symbol());
                    output
                });
        assert!(rendered.contains("Settings"));
        assert!(rendered.contains("Interface"));
        assert!(rendered.contains("Esc close"));
        assert_eq!(rendered_footer.map(|area| area.height), Some(1));
    }

    #[test]
    fn frame_with_empty_footer_uses_the_single_body_section() {
        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut rendered = None;

        terminal
            .draw(|frame| {
                rendered = Some(render_workbench_frame(
                    frame,
                    frame.area(),
                    SurfaceMode::Route,
                    &WorkbenchFrameSpec::new("Plugins".into(), "".into(), 60, 8),
                ));
            })
            .expect("empty footer must not access a missing second section");

        let rendered = rendered.expect("rendered workbench");
        assert!(rendered.body.height > 0);
        assert_eq!(rendered.footer, None);
    }
}
