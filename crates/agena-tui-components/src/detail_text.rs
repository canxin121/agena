use std::borrow::Cow;

use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};
use unicode_width::UnicodeWidthStr;

pub struct DetailDocument<'a> {
    pub text: Text<'a>,
    pub plain: String,
}

#[derive(Clone, Debug)]
pub enum DetailTextLine<'a> {
    Plain {
        text: Cow<'a, str>,
        style: Style,
    },
    Labeled {
        label: Cow<'a, str>,
        value: Cow<'a, str>,
        label_style: Style,
        value_style: Style,
    },
}

impl<'a> DetailTextLine<'a> {
    pub fn plain(text: impl Into<Cow<'a, str>>, style: Style) -> Self {
        Self::Plain {
            text: text.into(),
            style,
        }
    }

    pub fn labeled(
        label: impl Into<Cow<'a, str>>,
        value: impl Into<Cow<'a, str>>,
        label_style: Style,
        value_style: Style,
    ) -> Self {
        Self::Labeled {
            label: label.into(),
            value: value.into(),
            label_style,
            value_style,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DetailTextSpec<'a> {
    pub label_width: usize,
    pub separator: Cow<'a, str>,
}

impl<'a> DetailTextSpec<'a> {
    pub fn label_width(label_width: usize) -> Self {
        Self {
            label_width,
            separator: Cow::Borrowed("  "),
        }
    }
}

impl Default for DetailTextSpec<'_> {
    fn default() -> Self {
        Self::label_width(16)
    }
}

pub fn build_detail_text<'a, I>(lines: I, spec: &DetailTextSpec<'a>) -> Text<'a>
where
    I: IntoIterator<Item = DetailTextLine<'a>>,
{
    Text::from(
        lines
            .into_iter()
            .map(|line| build_detail_line(line, spec))
            .collect::<Vec<_>>(),
    )
}

pub fn build_detail_text_plain(lines: &[DetailTextLine<'_>], spec: &DetailTextSpec<'_>) -> String {
    lines
        .iter()
        .map(|line| match line {
            DetailTextLine::Plain { text, .. } => text.to_string(),
            DetailTextLine::Labeled { label, value, .. } => {
                detail_row_display_text(label.as_ref(), value.as_ref(), spec)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn build_detail_document<'a>(
    lines: &[DetailTextLine<'a>],
    spec: &DetailTextSpec<'a>,
) -> DetailDocument<'a> {
    DetailDocument {
        text: build_detail_text(lines.iter().cloned(), spec),
        plain: build_detail_text_plain(lines, spec),
    }
}

pub fn detail_row_display_text(label: &str, value: &str, spec: &DetailTextSpec<'_>) -> String {
    format!(
        "{}{}{}",
        pad_label_to_width(label, spec.label_width),
        spec.separator,
        value
    )
}

fn build_detail_line<'a>(line: DetailTextLine<'a>, spec: &DetailTextSpec<'a>) -> Line<'a> {
    match line {
        DetailTextLine::Plain { text, style } => Line::from(Span::styled(text, style)),
        DetailTextLine::Labeled {
            label,
            value,
            label_style,
            value_style,
        } => Line::from(vec![
            Span::styled(
                pad_label_to_width(label.as_ref(), spec.label_width),
                label_style,
            ),
            Span::raw(spec.separator.clone()),
            Span::styled(value, value_style),
        ]),
    }
}

fn pad_label_to_width(label: &str, width: usize) -> String {
    let display_width = UnicodeWidthStr::width(label);
    if display_width >= width {
        label.to_string()
    } else {
        format!("{}{}", " ".repeat(width - display_width), label)
    }
}
