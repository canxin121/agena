use std::borrow::Cow;

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::Style,
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};

use crate::{
    FramedSurfaceSpec,
    layout::{
        SurfaceMode, VerticalSectionSize, framed_sections_target_height,
        optional_overlay_text_height, split_vertical_sections,
    },
    render_framed_surface, wrapped_text_height_for_text,
};

pub(crate) struct TextDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub body: &'a Text<'a>,
    pub footer: Option<Cow<'a, str>>,
    pub target_width: u16,
    pub body_wrap: bool,
    pub body_scroll: Option<(u16, u16)>,
    pub body_alignment: Option<Alignment>,
    pub body_height_bounds: (u16, u16),
    pub footer_height_bounds: (u16, u16),
    pub footer_alignment: Option<Alignment>,
    pub footer_style: Style,
}

pub struct TextDialogLine<'a> {
    pub text: Cow<'a, str>,
    pub style: Style,
}

impl<'a> TextDialogLine<'a> {
    pub fn plain(text: impl Into<Cow<'a, str>>) -> Self {
        Self {
            text: text.into(),
            style: Style::default(),
        }
    }

    pub fn styled(text: impl Into<Cow<'a, str>>, style: Style) -> Self {
        Self {
            text: text.into(),
            style,
        }
    }
}

pub struct LineTextDialogSpec<'a> {
    pub title: Cow<'a, str>,
    pub lines: &'a [TextDialogLine<'a>],
    pub footer: Option<Cow<'a, str>>,
    pub target_width: u16,
    pub body_wrap: bool,
    pub body_scroll: Option<(u16, u16)>,
    pub body_alignment: Option<Alignment>,
    pub body_height_bounds: (u16, u16),
    pub footer_height_bounds: (u16, u16),
    pub footer_alignment: Option<Alignment>,
    pub footer_style: Style,
}

impl<'a> LineTextDialogSpec<'a> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        title: Cow<'a, str>,
        lines: &'a [TextDialogLine<'a>],
        footer: Option<Cow<'a, str>>,
        target_width: u16,
        body_wrap: bool,
        body_scroll: Option<(u16, u16)>,
        body_alignment: Option<Alignment>,
        body_height_bounds: (u16, u16),
        footer_height_bounds: (u16, u16),
        footer_alignment: Option<Alignment>,
        footer_style: Style,
    ) -> Self {
        Self {
            title,
            lines,
            footer,
            target_width,
            body_wrap,
            body_scroll,
            body_alignment,
            body_height_bounds,
            footer_height_bounds,
            footer_alignment,
            footer_style,
        }
    }
}

pub fn render_line_text_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &LineTextDialogSpec<'_>,
) {
    let body = Text::from(
        spec.lines
            .iter()
            .map(|line| Line::from(Span::styled(line.text.as_ref().to_string(), line.style)))
            .collect::<Vec<_>>(),
    );

    render_text_dialog(
        frame,
        area,
        surface,
        &TextDialogSpec {
            title: spec.title.clone(),
            body: &body,
            footer: spec.footer.clone(),
            target_width: spec.target_width,
            body_wrap: spec.body_wrap,
            body_scroll: spec.body_scroll,
            body_alignment: spec.body_alignment,
            body_height_bounds: spec.body_height_bounds,
            footer_height_bounds: spec.footer_height_bounds,
            footer_alignment: spec.footer_alignment,
            footer_style: spec.footer_style,
        },
    );
}

pub(crate) fn render_text_dialog(
    frame: &mut Frame,
    area: Rect,
    surface: SurfaceMode,
    spec: &TextDialogSpec<'_>,
) {
    let content_width = surface.content_width(area, spec.target_width);
    let body_height = text_dialog_body_height(spec.body, content_width, spec.body_wrap)
        .clamp(spec.body_height_bounds.0, spec.body_height_bounds.1);
    let footer_height = spec
        .footer
        .as_ref()
        .map(|footer| {
            optional_overlay_text_height(
                footer.as_ref(),
                content_width,
                spec.footer_height_bounds.0,
                spec.footer_height_bounds.1,
            )
        })
        .unwrap_or(0);
    let mut sections = vec![VerticalSectionSize::Flexible(body_height)];
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

    let mut body = Paragraph::new(spec.body.clone());
    if spec.body_wrap {
        body = body.wrap(Wrap { trim: false });
    }
    if let Some(scroll) = spec.body_scroll {
        body = body.scroll(scroll);
    }
    if let Some(alignment) = spec.body_alignment {
        body = body.alignment(alignment);
    }
    frame.render_widget(body, rows[0]);

    if let Some(footer) = spec
        .footer
        .as_ref()
        .filter(|footer| !footer.trim().is_empty())
    {
        let mut footer_paragraph = Paragraph::new(footer.as_ref().to_string())
            .wrap(Wrap { trim: false })
            .style(spec.footer_style);
        if let Some(alignment) = spec.footer_alignment {
            footer_paragraph = footer_paragraph.alignment(alignment);
        }
        frame.render_widget(footer_paragraph, rows[1]);
    }
}

fn text_dialog_body_height(text: &Text<'_>, width: u16, wrap: bool) -> u16 {
    if !wrap {
        return u16::try_from(text.lines.len()).unwrap_or(u16::MAX).max(1);
    }

    wrapped_text_height_for_text(text, width.max(1))
}
