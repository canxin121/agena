use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear},
};

use crate::layout::SurfaceMode;
use crate::theme::{modal_border_style, modal_surface_style};

pub struct FramedSurfaceSpec<'a> {
    pub title: Cow<'a, str>,
    pub target_width: u16,
    pub target_height: u16,
}

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
    let outer = surface.outer_rect(area, spec.target_width, spec.target_height);
    frame.render_widget(Clear, outer);
    let mut block = Block::default()
        .title(format!(" {} ", spec.title))
        .borders(Borders::ALL);
    if modal {
        block = block
            .style(modal_surface_style())
            .border_style(modal_border_style())
            .title_style(modal_border_style());
    }
    let inner = block.inner(outer);
    frame.render_widget(block, outer);
    FramedSurface { outer, inner }
}
