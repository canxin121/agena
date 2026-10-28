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

/// Keep modal theme and pointer chrome while allowing the caller to supply an
/// exact responsive rectangle with `SurfaceMode::Route`. Compact dialog sizing
/// remains unchanged for callers that use `SurfaceMode::Overlay`.
pub fn render_modal_framed_surface(
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
