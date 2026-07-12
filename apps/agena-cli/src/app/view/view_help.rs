impl App {
    pub(in crate::app) fn render_context_help(&mut self, frame: &mut Frame, area: Rect) {
        let Some(help) = self.context_help.as_mut() else {
            return;
        };
        let modal_title = help.modal_title.clone();
        let eyebrow = help.eyebrow.clone();
        let footer_hint = help.footer.clone();
        let badge = match help.kind {
            crate::app::InfoOverlayKind::Help => " ? ",
            crate::app::InfoOverlayKind::Diagnostics => " ◈ ",
        };

        let target_height = area.height.saturating_sub(4).clamp(12, 38);
        let outer = SurfaceMode::Overlay.outer_rect(area, 104, target_height);
        frame.render_widget(Clear, outer);
        let frame_block = Block::default()
            .title(Line::from(vec![
                Span::styled(
                    badge,
                    Style::default()
                        .fg(agena_tui_components::theme::active_palette().selection_fg)
                        .bg(agena_tui_components::theme::accent_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    format!(" {modal_title} "),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
            ]))
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(agena_tui_components::theme::accent_color()));
        let inner = frame_block.inner(outer);
        frame.render_widget(frame_block, outer);

        let header_height = if inner.height >= 12 { 5 } else { 3 };
        let rows = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(header_height),
                Constraint::Min(4),
                Constraint::Length(1),
            ])
            .split(inner);

        let header = Text::from(vec![
            Line::from(Span::styled(
                eyebrow.to_uppercase(),
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                sanitize_display_text(help.context.as_str()),
                Style::default()
                    .fg(agena_tui_components::theme::special_color())
                    .add_modifier(Modifier::BOLD),
            )),
            Line::from(Span::styled(
                sanitize_display_text(help.summary.as_str()),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )),
        ]);
        frame.render_widget(
            Paragraph::new(header)
                .block(
                    Block::default().borders(Borders::BOTTOM).border_style(
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    ),
                )
                .wrap(Wrap { trim: false }),
            rows[0],
        );

        let content_width = rows[1].width.saturating_sub(2).max(1);
        let lines = context_help_lines(help, content_width);
        help.max_scroll = lines
            .len()
            .saturating_sub(rows[1].height as usize)
            .min(u16::MAX as usize) as u16;
        help.scroll.clamp(help.max_scroll);
        frame.render_widget(
            Paragraph::new(Text::from(lines)).scroll((help.scroll.scroll, 0)),
            rows[1],
        );

        let position = if help.max_scroll > 0 {
            format!(
                "{}  ·  {}/{}",
                footer_hint,
                help.scroll.scroll.saturating_add(1),
                help.max_scroll.saturating_add(1)
            )
        } else {
            footer_hint
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                sanitize_display_text(position),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            )))
            .alignment(Alignment::Right),
            rows[2],
        );
    }
}

fn context_help_lines(help: &HelpOverlay, width: u16) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let rule_width = width.saturating_sub(2).max(8) as usize;
    for section in &help.sections {
        if !lines.is_empty() {
            lines.push(Line::default());
        }
        let title = sanitize_display_text(section.title.as_str());
        let remaining =
            rule_width.saturating_sub(UnicodeWidthStr::width(title.as_str()).saturating_add(4));
        lines.push(Line::from(vec![
            Span::styled(
                "╭─ ",
                Style::default().fg(agena_tui_components::theme::accent_color()),
            ),
            Span::styled(
                title,
                Style::default()
                    .fg(agena_tui_components::theme::accent_color())
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" {}", "─".repeat(remaining)),
                Style::default().fg(agena_tui_components::theme::muted_color()),
            ),
        ]));
        for entry in &section.entries {
            lines.extend(context_help_entry_lines(entry, width));
        }
        lines.push(Line::from(Span::styled(
            format!("╰{}", "─".repeat(rule_width.saturating_add(1))),
            Style::default().fg(agena_tui_components::theme::muted_color()),
        )));
    }

    if !help.tips.is_empty() {
        lines.push(Line::default());
        for tip in &help.tips {
            lines.push(Line::from(vec![
                Span::styled(
                    " ◆ ",
                    Style::default()
                        .fg(agena_tui_components::theme::special_color())
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    sanitize_display_text(tip.as_str()),
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
            ]));
        }
    }
    lines
}

fn context_help_entry_lines(entry: &HelpEntry, width: u16) -> Vec<Line<'static>> {
    let keys = sanitize_display_text(entry.keys.as_str());
    let description = sanitize_display_text(entry.description.as_str());
    if width >= 58 {
        let key_width = 40_usize.min((width as usize).saturating_div(2).max(12));
        let key = truncate_display_text(keys.as_str(), key_width);
        let description_width = (width as usize)
            .saturating_sub(key_width)
            .saturating_sub(6)
            .max(12);
        return textwrap::wrap(description.as_str(), description_width)
            .into_iter()
            .enumerate()
            .map(|(index, text)| {
                Line::from(vec![
                    Span::styled(
                        "│  ",
                        Style::default().fg(agena_tui_components::theme::muted_color()),
                    ),
                    Span::styled(
                        if index == 0 {
                            format!("{key:<key_width$}")
                        } else {
                            " ".repeat(key_width)
                        },
                        Style::default()
                            .fg(agena_tui_components::theme::special_color())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::raw("  "),
                    Span::raw(text.into_owned()),
                ])
            })
            .collect();
    }

    let mut lines = vec![Line::from(vec![
        Span::styled(
            "│  ",
            Style::default().fg(agena_tui_components::theme::muted_color()),
        ),
        Span::styled(
            keys,
            Style::default()
                .fg(agena_tui_components::theme::special_color())
                .add_modifier(Modifier::BOLD),
        ),
    ])];
    lines.extend(
        textwrap::wrap(
            description.as_str(),
            (width as usize).saturating_sub(5).max(8),
        )
        .into_iter()
        .map(|text| {
            Line::from(vec![
                Span::styled(
                    "│     ",
                    Style::default().fg(agena_tui_components::theme::muted_color()),
                ),
                Span::raw(text.into_owned()),
            ])
        }),
    );
    lines
}

use super::{
    Alignment, App, Block, BorderType, Borders, Clear, Constraint, Direction, Frame, HelpEntry,
    HelpOverlay, Layout, Line, Modifier, Paragraph, Rect, Span, Style, SurfaceMode, Text,
    UnicodeWidthStr, Wrap, sanitize_display_text, truncate_display_text,
};

#[cfg(test)]
mod tests {
    use super::{HelpEntry, HelpOverlay, context_help_lines};
    use crate::app::HelpSection;
    use agena_tui_components::{ScrollState, line_plain_text};

    fn fixture() -> HelpOverlay {
        HelpOverlay {
            kind: crate::app::InfoOverlayKind::Help,
            modal_title: "Help".to_string(),
            eyebrow: "Quick reference".to_string(),
            footer: "Esc close".to_string(),
            context: "Transcript".to_string(),
            summary: "Navigate the conversation".to_string(),
            sections: vec![HelpSection {
                title: "Navigation".to_string(),
                entries: vec![HelpEntry {
                    keys: "j / k · ↑ / ↓".to_string(),
                    description: "Move through messages, blocks, and lines.".to_string(),
                }],
            }],
            tips: vec!["Ctrl+H opens contextual help.".to_string()],
            scroll: ScrollState::default(),
            max_scroll: 0,
        }
    }

    #[test]
    fn contextual_help_uses_distinct_cards_and_key_columns() {
        let lines = context_help_lines(&fixture(), 80);
        let text = lines
            .iter()
            .map(line_plain_text)
            .collect::<Vec<_>>()
            .join("\n");

        assert!(text.contains("╭─ Navigation"));
        assert!(text.contains("j / k · ↑ / ↓"));
        assert!(text.contains("Move through messages"));
        assert!(text.contains("◆ Ctrl+H opens contextual help"));
        assert!(!text.contains("Session switcher"));
    }

    #[test]
    fn contextual_help_switches_to_stacked_entries_on_narrow_screens() {
        let lines = context_help_lines(&fixture(), 36);
        assert!(lines.len() >= 5);
    }
}
