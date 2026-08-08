//! Detail dialog widget for long-form content.

use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
};

use crate::{
    DetailTextLine, DetailTextSpec, SurfaceMode, build_detail_text,
    text_dialog::{TextDialogSpec, render_text_dialog},
};

pub struct DetailTextDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub footer: Option<Cow<'a, str>>,
    pub target_width: u16,
    pub detail_spec: DetailTextSpec<'a>,
    pub body_height_bounds: (u16, u16),
    pub footer_height_bounds: (u16, u16),
    pub footer_alignment: Option<Alignment>,
    pub footer_style: Style,
}

impl<'a> DetailTextDialogSpec<'a> {
    pub fn new(
        title: Cow<'a, str>,
        footer: Option<Cow<'a, str>>,
        target_width: u16,
        detail_spec: DetailTextSpec<'a>,
        body_height_bounds: (u16, u16),
        footer_height_bounds: (u16, u16),
        footer_alignment: Option<Alignment>,
        footer_style: Style,
    ) -> Self {
        Self {
            title,
            footer,
            target_width,
            detail_spec,
            body_height_bounds,
            footer_height_bounds,
            footer_alignment,
            footer_style,
        }
    }
}

pub fn render_detail_text_dialog<'a, I>(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &DetailTextDialogSpec<'a>,
    lines: I,
) where
    I: IntoIterator<Item = DetailTextLine<'a>>,
{
    let body = build_detail_text(lines, &spec.detail_spec);
    render_text_dialog(
        frame,
        area,
        surface,
        &TextDialogSpec {
            title: spec.title.clone(),
            body: &body,
            footer: spec.footer.clone(),
            target_width: spec.target_width,
            body_wrap: true,
            body_scroll: None,
            body_alignment: None,
            body_height_bounds: spec.body_height_bounds,
            footer_height_bounds: spec.footer_height_bounds,
            footer_alignment: spec.footer_alignment,
            footer_style: spec.footer_style,
        },
    );
}
