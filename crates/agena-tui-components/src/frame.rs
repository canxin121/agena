//! Frame drawing helpers.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, BorderType, Borders, Clear},
};

use crate::layout::SurfaceMode;
use crate::theme::{modal_border_style, modal_surface_style};

/// Spec of a framed surface.
pub struct FramedSurfaceSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub target_height: u16,
}

/// A framed surface widget.
pub struct FramedSurface {
    pub outer: Rect,
    pub inner: Rect,
}

pub fn render_framed_surface(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &FramedSurfaceSpec<'_>,
) -> FramedSurface {
    render_framed_surface_with_modal_style(
        frame,
        area,
        surface,
        spec,
        matches!(surface, SurfaceMode::Overlay),
    )
}

pub(crate) fn render_modal_framed_surface(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &FramedSurfaceSpec<'_>,
) -> FramedSurface {
    render_framed_surface_with_modal_style(frame, area, surface, spec, true)
}

fn render_framed_surface_with_modal_style(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &FramedSurfaceSpec<'_>,
    modal: bool,
) -> FramedSurface {
    crate::pointer::begin_surface();
    let outer = surface.outer_rect(area, spec.target_width, spec.target_height);
    frame.render_widget(Clear, outer);
    let mut block = Block::default()
        .title(format!(
            " [×] {} ",
            crate::text::truncate_display_text(
                &spec.title,
                usize::from(outer.width.saturating_sub(8))
            )
        ))
        .borders(Borders::ALL);
    if modal {
        block = block
            .style(modal_surface_style())
            // Color is not a reliable discriminator on 16-color terminals or
            // under NO_COLOR-like configurations. A double outline preserves
            // the modal hierarchy even when RGB colors are quantized away.
            .border_type(BorderType::Double)
            .border_style(modal_border_style())
            .title_style(modal_border_style());
    }
    let inner = block.inner(outer);
    frame.render_widget(block, outer);
    crate::pointer::register(
        Rect::new(
            outer.x.saturating_add(1),
            outer.y,
            4.min(outer.width.saturating_sub(2)),
            u16::from(outer.height > 0),
        ),
        Some(crate::pointer::key(crossterm::event::KeyCode::Esc)),
        None,
    );
    FramedSurface { outer, inner }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    #[test]
    fn overlay_frame_has_color_and_color_independent_modal_boundaries() {
        let backend = TestBackend::new(40, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_framed_surface(
                    frame,
                    frame.area(),
                    SurfaceMode::Overlay,
                    &FramedSurfaceSpec {
                        title: "Permission".into(),
                        target_width: 24,
                        target_height: 6,
                    },
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        let palette = crate::theme::active_palette();
        assert_eq!(buffer[(2, 3)].symbol(), "╔");
        assert_eq!(buffer[(37, 8)].symbol(), "╝");
        assert_eq!(buffer[(3, 4)].bg, palette.modal_bg);
        assert_eq!(buffer[(2, 3)].fg, palette.modal_border);
    }

    #[test]
    fn route_frame_keeps_the_plain_canvas_style() {
        let backend = TestBackend::new(24, 6);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render_framed_surface(
                    frame,
                    frame.area(),
                    SurfaceMode::Route,
                    &FramedSurfaceSpec {
                        title: "Route".into(),
                        target_width: 24,
                        target_height: 6,
                    },
                );
            })
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "┌");
        assert_eq!(buffer[(1, 1)].bg, ratatui::style::Color::Reset);
    }
}
