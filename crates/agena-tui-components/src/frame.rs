use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Block, Borders, Clear},
};

use crate::layout::SurfaceMode;

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
    let outer = surface.outer_rect(area, spec.target_width, spec.target_height);
    frame.render_widget(Clear, outer);
    let block = Block::default()
        .title(format!(" {} ", spec.title))
        .borders(Borders::ALL);
    let inner = block.inner(outer);
    frame.render_widget(block, outer);
    FramedSurface { outer, inner }
}
