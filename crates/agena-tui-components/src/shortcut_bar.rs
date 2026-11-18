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

fn shortcut_hint_width(hint: &str, available: u16) -> u16 {
    u16::try_from(unicode_width::UnicodeWidthStr::width(hint))
        .unwrap_or(u16::MAX)
        .min(available)
}

/// Measure the same indivisible hint chips used by the footer renderer.
/// Ordinary paragraph wrapping can underestimate their required rows.
pub(crate) fn shortcut_footer_height(text: &str, width: u16) -> u16 {
    if width == 0 {
        return 0;
    }
    let mut rows: u16 = 0;
    let mut x: u16 = 0;
    for hint in text
        .split('·')
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
    {
        rows = rows.max(1);
        let hint_width = shortcut_hint_width(hint, width);
        if x.saturating_add(hint_width) > width {
            rows = rows.saturating_add(1);
            x = 0;
        }
        x = x.saturating_add(hint_width).saturating_add(2);
    }
    rows
}

/// Render trusted UI shortcut hints as compact buttons. Key labels are the
/// existing keyboard bindings; localized descriptions do not determine actions.
pub fn render_shortcut_footer(frame: &mut ratatui::Frame, area: ratatui::layout::Rect, text: &str) {
    use ratatui::{layout::Rect, widgets::Paragraph};
    let mut x = area.x;
    let mut y = area.y;
    for hint in text
        .split('·')
        .map(str::trim)
        .filter(|hint| !hint.is_empty())
    {
        let width = shortcut_hint_width(hint, area.width);
        if x.saturating_add(width) > area.right() {
            x = area.x;
            y = y.saturating_add(1);
        }
        if y >= area.bottom() || width == 0 {
            break;
        }
        let rect = Rect::new(x, y, width, 1);
        let label = crate::truncate_display_text(hint, usize::from(width));
        let key = hint.split_whitespace().next().and_then(shortcut_event);
        let style = if key.is_some() {
            crate::theme::status_chip_style()
        } else {
            crate::theme::muted_style()
        };
        frame.render_widget(Paragraph::new(label).style(style), rect);
        if let Some(key) = key {
            crate::pointer::register(rect, Some(crate::pointer::PointerAction::Key(key)), None);
        }
        x = x.saturating_add(width).saturating_add(2);
    }
}

fn shortcut_event(label: &str) -> Option<crossterm::event::KeyEvent> {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    // Combined navigation hints (↑/↓, q/Esc) describe several actions. Only
    // an unambiguous, visible key gets a pointer action.
    let (modifiers, label) = if let Some(key) = label.strip_prefix("Ctrl+") {
        (KeyModifiers::CONTROL, key)
    } else if let Some(key) = label.strip_prefix("Shift+") {
        (KeyModifiers::SHIFT, key)
    } else {
        (KeyModifiers::NONE, label)
    };
    let code = match label {
        "Enter" => KeyCode::Enter,
        "Esc" => KeyCode::Esc,
        "Tab" if modifiers == KeyModifiers::SHIFT => KeyCode::BackTab,
        "Tab" => KeyCode::Tab,
        "Space" => KeyCode::Char(' '),
        "↑" => KeyCode::Up,
        "↓" => KeyCode::Down,
        "←" => KeyCode::Left,
        "→" => KeyCode::Right,
        "PgUp" => KeyCode::PageUp,
        "PgDn" => KeyCode::PageDown,
        _ if label.len() == 1 && label.is_ascii() => {
            KeyCode::Char(if modifiers.contains(KeyModifiers::CONTROL) {
                label.chars().next()?.to_ascii_lowercase()
            } else {
                label.chars().next()?
            })
        }
        _ => return None,
    };
    Some(KeyEvent::new(code, modifiers))
}
