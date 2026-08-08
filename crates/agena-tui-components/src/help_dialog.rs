//! Help dialog widget.

use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    style::{Modifier, Style},
    text::{Line, Span, Text},
    widgets::{Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;

use crate::{
    FramedSurfaceSpec, ScrollState, SurfaceMode, TextPanelSpec, VerticalSectionSize,
    optional_overlay_text_height, render_text_panel, search_picker_dialog_area,
    split_vertical_sections, theme, truncate_display_text,
};

/// One key binding or labeled value in a help dialog section.
#[derive(Debug, Clone)]
pub struct HelpDialogEntry {
    pub keys: String,
    pub description: String,
}

/// A titled group of related help entries.
#[derive(Debug, Clone)]
pub struct HelpDialogSection {
    pub title: String,
    pub entries: Vec<HelpDialogEntry>,
}

/// Shared state for contextual help and other help-shaped information dialogs.
///
/// `TKind` lets an application distinguish variants such as regular help and
/// diagnostics without duplicating the component or teaching it app-specific
/// behavior.
#[derive(Debug, Clone)]
pub struct HelpDialogState<TKind> {
    pub kind: TKind,
    pub modal_title: String,
    pub eyebrow: String,
    pub footer: String,
    pub context: String,
    pub summary: String,
    pub sections: Vec<HelpDialogSection>,
    pub tips: Vec<String>,
    pub scroll: ScrollState,
    pub max_scroll: u16,
}

/// Renders the canonical help surface.
///
/// Help uses the same wide, centered geometry and shared frame as search
/// pickers. Its square content panel, semantic accent color, and quiet footer
/// deliberately follow the confirmation and selection dialog vocabulary.
pub fn render_help_dialog<TKind, F>(
    frame: &mut Frame,
    area: Rect,
    dialog: &mut HelpDialogState<TKind>,
    normalize_text: F,
) where
    F: Fn(&str) -> String,
{
    let area = search_picker_dialog_area(area);
    let title = help_dialog_title(dialog, &normalize_text);
    // Help is centered before framing, so Route describes only its geometry;
    // semantically it remains a modal above the current conversation.
    let framed = crate::frame::render_modal_framed_surface(
        frame,
        area,
        SurfaceMode::Route,
        &FramedSurfaceSpec {
            title: title.into(),
            target_width: area.width,
            target_height: area.height,
        },
    );
    if framed.inner.width == 0 || framed.inner.height == 0 {
        dialog.max_scroll = 0;
        dialog.scroll.clamp(0);
        return;
    }

    let prompt = normalize_text(&dialog.summary);
    let desired_prompt_height =
        optional_overlay_text_height(&prompt, framed.inner.width.max(1), 1, 3);
    let footer_hint = normalize_text(&dialog.footer);
    let footer_probe = if footer_hint.trim().is_empty() {
        "65535/65535".to_owned()
    } else {
        format!("{footer_hint} · 65535/65535")
    };
    let desired_footer_height =
        optional_overlay_text_height(&footer_probe, framed.inner.width.max(1), 1, 2);
    let minimum_panel_height = framed.inner.height.min(3);
    let chrome_height = framed.inner.height.saturating_sub(minimum_panel_height);
    let prompt_height = desired_prompt_height.min(chrome_height);
    let footer_height = desired_footer_height.min(chrome_height.saturating_sub(prompt_height));
    let mut sections = Vec::new();
    if prompt_height > 0 {
        sections.push(VerticalSectionSize::Fixed(prompt_height));
    }
    sections.push(VerticalSectionSize::Flexible(minimum_panel_height.max(1)));
    if footer_height > 0 {
        sections.push(VerticalSectionSize::Fixed(footer_height));
    }
    let rows = split_vertical_sections(framed.inner, &sections);

    let mut row_index = 0;
    if prompt_height > 0 {
        frame.render_widget(
            Paragraph::new(prompt).wrap(Wrap { trim: false }),
            rows[row_index],
        );
        row_index += 1;
    }

    let body_area = rows[row_index];
    row_index += 1;
    let content_width = body_area.width.saturating_sub(2);
    let lines = help_dialog_lines(dialog, content_width, &normalize_text);
    let visible_body_height = body_area.height.saturating_sub(2) as usize;
    dialog.max_scroll = lines
        .len()
        .saturating_sub(visible_body_height)
        .min(u16::MAX as usize) as u16;
    dialog.scroll.clamp(dialog.max_scroll);
    let body = Text::from(lines);
    let panel_title = normalize_text(&dialog.eyebrow);
    render_text_panel(
        frame,
        body_area,
        &TextPanelSpec {
            title: (!panel_title.trim().is_empty()).then(|| panel_title.into()),
            body: &body,
            wrap: false,
            scroll: Some((dialog.scroll.scroll, 0)),
            alignment: None,
        },
    );

    if footer_height > 0 {
        let footer = help_dialog_footer(dialog, &normalize_text);
        frame.render_widget(
            Paragraph::new(footer)
                .style(theme::muted_style())
                .alignment(Alignment::Right)
                .wrap(Wrap { trim: false }),
            rows[row_index],
        );
    }
}

fn help_dialog_title<TKind, F>(dialog: &HelpDialogState<TKind>, normalize_text: &F) -> String
where
    F: Fn(&str) -> String,
{
    let title = normalize_text(&dialog.modal_title);
    let context = normalize_text(&dialog.context);
    if context.trim().is_empty() {
        title
    } else {
        format!("{title} · {context}")
    }
}

fn help_dialog_footer<TKind, F>(dialog: &HelpDialogState<TKind>, normalize_text: &F) -> String
where
    F: Fn(&str) -> String,
{
    let footer = normalize_text(&dialog.footer);
    if dialog.max_scroll == 0 {
        return footer;
    }
    let position = format!(
        "{}/{}",
        dialog.scroll.scroll.saturating_add(1),
        dialog.max_scroll.saturating_add(1),
    );
    if footer.trim().is_empty() {
        position
    } else {
        format!("{footer} · {position}")
    }
}

fn help_dialog_lines<TKind, F>(
    dialog: &HelpDialogState<TKind>,
    width: u16,
    normalize_text: &F,
) -> Vec<Line<'static>>
where
    F: Fn(&str) -> String,
{
    let mut lines = Vec::new();
    for section in &dialog.sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let title = truncate_display_text(&normalize_text(&section.title), (width as usize).max(1));
        lines.push(Line::from(Span::styled(
            title,
            Style::default().add_modifier(Modifier::BOLD),
        )));
        for entry in &section.entries {
            lines.extend(help_entry_lines(entry, width, normalize_text));
        }
    }

    if !dialog.tips.is_empty() {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        for tip in &dialog.tips {
            let tip = normalize_text(tip);
            lines.extend(
                textwrap::wrap(&tip, (width as usize).saturating_sub(2).max(1))
                    .into_iter()
                    .enumerate()
                    .map(|(index, text)| {
                        Line::from(vec![
                            Span::styled(
                                if index == 0 { "! " } else { "  " },
                                Style::default()
                                    .fg(theme::warning_color())
                                    .add_modifier(Modifier::BOLD),
                            ),
                            Span::styled(text.into_owned(), theme::muted_style()),
                        ])
                    }),
            );
        }
    }
    lines
}

fn help_entry_lines<F>(
    entry: &HelpDialogEntry,
    width: u16,
    normalize_text: &F,
) -> Vec<Line<'static>>
where
    F: Fn(&str) -> String,
{
    let keys = normalize_text(&entry.keys);
    let description = normalize_text(&entry.description);
    let content_width = (width as usize).max(1);
    let key_style = Style::default()
        .fg(theme::accent_color())
        .add_modifier(Modifier::BOLD);

    if content_width >= 54 {
        let leading_width = 2;
        let key_width = 32_usize.min(
            content_width
                .saturating_sub(leading_width)
                .saturating_div(3)
                .max(12),
        );
        let key = truncate_display_text(&keys, key_width);
        let description_width = content_width
            .saturating_sub(leading_width)
            .saturating_sub(key_width)
            .saturating_sub(2)
            .max(12);
        return textwrap::wrap(&description, description_width)
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                let text = text.into_owned();
                Line::from(vec![
                    Span::raw("  "),
                    Span::styled(
                        if index == 0 {
                            format!(
                                "{key}{}",
                                " ".repeat(
                                    key_width.saturating_sub(UnicodeWidthStr::width(key.as_str()))
                                )
                            )
                        } else {
                            " ".repeat(key_width)
                        },
                        key_style,
                    ),
                    Span::raw("  "),
                    Span::raw(text),
                ])
            })
            .collect();
    }

    let mut lines = Vec::new();
    let key_width = content_width.saturating_sub(2).max(1);
    let key = truncate_display_text(&keys, key_width);
    lines.push(Line::from(vec![
        Span::raw("  "),
        Span::styled(key, key_style),
    ]));

    for text in textwrap::wrap(&description, content_width.saturating_sub(4).max(1)) {
        lines.push(Line::from(vec![
            Span::raw("    "),
            Span::raw(text.into_owned()),
        ]));
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::{Terminal, backend::TestBackend};

    fn fixture() -> HelpDialogState<()> {
        HelpDialogState {
            kind: (),
            modal_title: "Help".to_owned(),
            eyebrow: "Quick reference".to_owned(),
            footer: "Esc close".to_owned(),
            context: "Transcript".to_owned(),
            summary: "Navigate the conversation".to_owned(),
            sections: vec![HelpDialogSection {
                title: "Navigation".to_owned(),
                entries: vec![HelpDialogEntry {
                    keys: "j / k · ↑ / ↓".to_owned(),
                    description: "Move through messages, blocks, and lines.".to_owned(),
                }],
            }],
            tips: vec!["Ctrl+H opens contextual help.".to_owned()],
            scroll: ScrollState::default(),
            max_scroll: 0,
        }
    }

    #[test]
    fn help_uses_the_search_picker_geometry_and_shared_square_frame() {
        let mut dialog = fixture();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_help_dialog(frame, frame.area(), &mut dialog, str::to_owned))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(4, 4)].symbol(), "╔");
        assert_eq!(buffer[(115, 4)].symbol(), "╗");
        assert_eq!(buffer[(4, 25)].symbol(), "╚");
        assert_eq!(buffer[(115, 25)].symbol(), "╝");
        assert_eq!(buffer[(5, 6)].symbol(), "┌");
        assert_eq!(buffer[(114, 6)].symbol(), "┐");
        let rendered = buffer
            .content()
            .iter()
            .map(|cell| cell.symbol())
            .collect::<String>();
        assert!(rendered.contains("Help · Transcript"));
        assert!(rendered.contains("Quick reference"));
        assert!(rendered.contains("Navigation"));
        assert!(rendered.contains("j / k · ↑ / ↓"));
        assert!(rendered.contains("! Ctrl+H opens contextual help"));
        assert!(!rendered.contains('╭'));
        assert!(!rendered.contains('◆'));
    }

    #[test]
    fn narrow_help_stacks_keys_above_descriptions() {
        let dialog = fixture();
        let lines = help_dialog_lines(&dialog, 36, &str::to_owned);
        let rendered = lines
            .iter()
            .map(crate::line_plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(rendered.contains("j / k · ↑ / ↓"));
        assert!(rendered.contains("    Move through messages"));
    }

    #[test]
    fn tiny_help_gracefully_uses_the_full_terminal() {
        let mut dialog = fixture();
        let backend = TestBackend::new(8, 4);
        let mut terminal = Terminal::new(backend).unwrap();

        terminal
            .draw(|frame| render_help_dialog(frame, frame.area(), &mut dialog, str::to_owned))
            .unwrap();

        let buffer = terminal.backend().buffer();
        assert_eq!(buffer[(0, 0)].symbol(), "╔");
        assert_eq!(buffer[(7, 3)].symbol(), "╝");
    }
}
