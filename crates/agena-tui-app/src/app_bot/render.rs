use super::*;
use agena_tui_components::truncate_display_text_with_suffix;
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Line,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use unicode_width::UnicodeWidthStr;
fn short(text: &str, width: u16) -> String {
    truncate_display_text_with_suffix(
        &agena_tui::sanitize_picker_text(text),
        usize::from(width),
        "…",
    )
}
impl App {
    pub(crate) fn render_bot_entry(&self, frame: &mut Frame<'_>, area: Rect) {
        let title = self.i18n.text(match self.bot.view {
            BotView::Form if self.bot.form.as_ref().is_some_and(|f| f.base.is_none()) => {
                "bot-create-title"
            }
            BotView::Form | BotView::Reload => "bot-edit-title",
            _ => "bot-manager-title",
        });
        let block = Block::default()
            .borders(Borders::ALL)
            .title(format!(" {} ", short(&title, area.width.saturating_sub(4))));
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let text = |key: &str| self.i18n.text(key);
        let view = &self.bot.view;
        let help_key = match view {
            BotView::List => "bot-list-keys",
            BotView::Opening => "bot-entry-keys",
            BotView::Archive | BotView::Reload => "bot-confirm-keys",
            BotView::Form if self.bot.accepts_text() => "bot-editor-keys",
            _ => "bot-form-keys",
        };
        let help = text(help_key);
        let help_lines = Paragraph::new(help.clone())
            .wrap(Wrap { trim: true })
            .line_count(inner.width.max(1))
            .min(5) as u16;
        let error = self.bot.error.as_ref().map(|e| {
            format!(
                "{} {}",
                agena_tui::sanitize_picker_text(e),
                text("bot-error-guidance")
            )
        });
        let status = error.unwrap_or_else(|| {
            if self.bot.task.is_some() {
                text("bot-opening")
            } else {
                String::new()
            }
        });
        let status_height = if status.is_empty() {
            0
        } else {
            Paragraph::new(status.clone())
                .wrap(Wrap { trim: true })
                .line_count(inner.width.max(1))
                .min(4) as u16
        };
        let [body, status_area, help_area] = Layout::vertical([
            Constraint::Min(1),
            Constraint::Length(status_height),
            Constraint::Length(help_lines),
        ])
        .areas(inner);
        frame.render_widget(
            Paragraph::new(status)
                .style(Style::default().fg(if self.bot.error.is_some() {
                    agena_tui_components::theme::active_palette().danger
                } else {
                    agena_tui_components::theme::active_palette().muted
                }))
                .wrap(Wrap { trim: true }),
            status_area,
        );
        frame.render_widget(Paragraph::new(help).wrap(Wrap { trim: true }), help_area);
        match view {
            BotView::List => {
                let rows: Vec<_> = self
                    .bot
                    .profiles
                    .iter()
                    .filter(|p| !p.archived)
                    .map(|p| {
                        let marker = if Some(p.id) == self.bot.selected {
                            text("bot-current")
                        } else {
                            String::new()
                        };
                        let preview = if p.persona.instructions.is_empty() {
                            text("bot-instructions-empty")
                        } else {
                            p.persona.instructions.clone()
                        };
                        ListItem::new(vec![
                            Line::from(format!(
                                "{} {marker}",
                                short(
                                    &p.name,
                                    body.width.saturating_sub(
                                        3 + UnicodeWidthStr::width(marker.as_str()) as u16
                                    )
                                )
                            )),
                            Line::from(short(&preview, body.width.saturating_sub(2))),
                        ])
                    })
                    .collect();
                if rows.is_empty() {
                    frame.render_widget(
                        Paragraph::new(text("bot-empty")).wrap(Wrap { trim: true }),
                        body,
                    );
                } else {
                    let mut state = ListState::default().with_selected(Some(self.bot.index));
                    frame.render_stateful_widget(
                        List::new(rows)
                            .highlight_symbol("› ")
                            .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                        body,
                        &mut state,
                    );
                }
            }
            BotView::Form => {
                if let Some(form) = &self.bot.form {
                    if let Some(editor) = &form.edit {
                        let label = text(["assistant-name", "bot-instructions"][form.index]);
                        let output = agena_tui_components::render_wrapped_editor_panel(
                            frame,
                            body,
                            &agena_tui_components::EditorPanelSpec {
                                title: Some(label.into()),
                                borders: Borders::ALL,
                            },
                            editor,
                        );
                        if self.bot.task.is_none() {
                            frame.set_cursor_position(output.cursor);
                        }
                    } else {
                        let rows: Vec<_> = ["assistant-name", "bot-instructions"]
                            .iter()
                            .enumerate()
                            .map(|(index, key)| {
                                ListItem::new(short(
                                    &format!("{}: {}", text(key), form.value(index)),
                                    body.width.saturating_sub(2),
                                ))
                            })
                            .collect();
                        let mut state = ListState::default().with_selected(Some(form.index));
                        frame.render_stateful_widget(
                            List::new(rows)
                                .highlight_symbol("› ")
                                .highlight_style(Style::default().add_modifier(Modifier::REVERSED)),
                            body,
                            &mut state,
                        );
                    }
                }
            }
            BotView::Archive => {
                let name = self.bot.highlighted().map_or("", |p| p.name.as_str());
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n{}",
                        agena_tui::sanitize_picker_text(name),
                        text("bot-archive-confirm")
                    ))
                    .wrap(Wrap { trim: true }),
                    body,
                );
            }
            BotView::Reload => {
                frame.render_widget(
                    Paragraph::new(text("bot-reload-confirm")).wrap(Wrap { trim: true }),
                    body,
                );
            }
            BotView::Opening => {
                frame.render_widget(
                    Paragraph::new(text("bot-description")).wrap(Wrap { trim: true }),
                    body,
                );
            }
        }
    }
}
