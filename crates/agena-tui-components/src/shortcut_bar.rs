//! Shortcut bar widget.

use std::borrow::Cow;

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span, Text},
};

use crate::theme::{accent_color, muted_style};

#[derive(Clone, Debug, PartialEq, Eq)]
/// A shortcut hint shown in the shortcut bar.
pub struct ShortcutHint<'a> {
    pub key: Cow<'a, str>,
    pub label: Cow<'a, str>,
}

impl<'a> ShortcutHint<'a> {
    pub fn new(key: impl Into<Cow<'a, str>>, label: impl Into<Cow<'a, str>>) -> Self {
        Self {
            key: key.into(),
            label: label.into(),
        }
    }
}

pub fn build_shortcut_line<'a>(hints: impl IntoIterator<Item = ShortcutHint<'a>>) -> Line<'a> {
    let mut spans = Vec::new();
    for (index, hint) in hints.into_iter().enumerate() {
        if index > 0 {
            spans.push(Span::raw("  ·  "));
        }
        spans.push(Span::styled(
            hint.key,
            Style::default()
                .fg(accent_color())
                .add_modifier(Modifier::BOLD),
        ));
        if !hint.label.trim().is_empty() {
            spans.push(Span::styled(
                Cow::Owned(format!(" {}", hint.label)),
                muted_style(),
            ));
        }
    }
    Line::from(spans)
}

pub fn build_shortcut_bar<'a>(hints: impl IntoIterator<Item = ShortcutHint<'a>>) -> Text<'a> {
    Text::from(build_shortcut_line(hints))
}

#[cfg(test)]
mod tests {
    use super::{ShortcutHint, build_shortcut_line};
    use crate::line_plain_text;

    #[test]
    fn shortcut_hints_have_one_consistent_separator() {
        let line = build_shortcut_line([
            ShortcutHint::new("Ctrl+S", "save"),
            ShortcutHint::new("Esc", "back"),
        ]);

        assert_eq!(line_plain_text(&line), "Ctrl+S save  ·  Esc back");
    }
}
