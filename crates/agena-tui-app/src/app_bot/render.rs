use super::*;
use agena_tui_components::truncate_display_text_with_suffix;
use agena_tui_media::{
    image_halfblock_lines_shared, image_rendering_enabled, render_markdown_image,
};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect, Size},
    style::{Modifier, Style},
    text::{Line, Span},
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

fn avatar_lines(source: Option<&str>, name: &str, width: u16) -> [Line<'static>; 2] {
    if image_rendering_enabled()
        && let Some(source) = source.filter(|s| s.starts_with("data:image/png;base64,"))
        && let Ok(artifact) = render_markdown_image(source)
    {
        let mut lines =
            image_halfblock_lines_shared(&artifact, Size::new(width, width.div_ceil(2)))
                .into_iter();
        return std::array::from_fn(|_| {
            lines
                .next()
                .unwrap_or_else(|| Line::from(" ".repeat(usize::from(width))))
        });
    }
    let initial = agena_tui::sanitize_picker_text(name)
        .chars()
        .find(|ch| !ch.is_whitespace())
        .unwrap_or('◉')
        .to_string();
    let initial = short(&initial, width);
    let padding = usize::from(width).saturating_sub(UnicodeWidthStr::width(initial.as_str()));
    [
        Line::from(format!("{initial}{}", " ".repeat(padding))),
        Line::from(" ".repeat(usize::from(width))),
    ]
}

fn with_avatar(
    source: Option<&str>,
    name: &str,
    width: u16,
    lines: [Line<'static>; 2],
    selected: bool,
) -> Vec<Line<'static>> {
    avatar_lines(source, name, width)
        .into_iter()
        .zip(lines)
        .map(|(mut avatar, line)| {
            avatar.spans.push(Span::raw(" "));
            // List applies its highlight style after rendering every span.
            // Highlight text here so selection cannot swap image pixels.
            avatar.spans.extend(line.spans.into_iter().map(|mut span| {
                span.style = line
                    .style
                    .patch(span.style)
                    .patch(selection_style(selected));
                span
            }));
            avatar
        })
        .collect()
}

fn selection_style(selected: bool) -> Style {
    if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    }
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
            BotView::List if self.bot.focus_conversations => "bot-conversation-keys",
            BotView::List => "bot-list-keys",
            BotView::Opening => "bot-entry-keys",
            BotView::Archive | BotView::Reload | BotView::DeleteConversation => "bot-confirm-keys",
            BotView::Form
                if self.bot.accepts_text()
                    && self
                        .bot
                        .form
                        .as_ref()
                        .is_some_and(|f| f.index == BOT_AVATAR_FIELD) =>
            {
                "bot-avatar-editor-keys"
            }
            BotView::Form if self.bot.accepts_text() => "bot-editor-keys",
            BotView::Form
                if self
                    .bot
                    .form
                    .as_ref()
                    .is_some_and(|f| f.index == BOT_AVATAR_FIELD) =>
            {
                "bot-avatar-form-keys"
            }
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
                text(
                    if self.bot.view == BotView::Form && self.bot.form.is_some() {
                        if self.bot.form.as_ref().is_some_and(|form| {
                            matches!(form.avatar, BotAvatarDraft::File { data: None, .. })
                        }) {
                            "bot-avatar-loading"
                        } else {
                            "bot-saving"
                        }
                    } else {
                        "bot-opening"
                    },
                )
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
                // One scrolling list: the Bot rows, then the highlighted Bot's
                // conversations under surface headings. The cursor moves through
                // both halves, so the view is a flat List with section rows.
                let palette = agena_tui_components::theme::active_palette();
                let mut items: Vec<ListItem> = Vec::new();
                let selected = if self.bot.focus_conversations {
                    self.bot.flat_conversation_offset() + self.bot.conversation_index
                } else {
                    self.bot.index
                };
                // Flat index -> whether the item is selectable (a Bot row or a
                // conversation row); headings are not.
                let mut selectable = Vec::new();
                let muted = Style::default().fg(palette.muted);
                let heading = Style::default()
                    .fg(palette.muted)
                    .add_modifier(Modifier::BOLD);
                for (index, p) in self.bot.profiles.iter().filter(|p| !p.archived).enumerate() {
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
                    let avatar_width = if body.width >= 48 { 4 } else { 2 };
                    let content_width = body.width.saturating_sub(avatar_width + 3);
                    items.push(ListItem::new(with_avatar(
                        p.avatar.as_deref(),
                        &p.name,
                        avatar_width,
                        [
                            Line::from(format!(
                                "{} {marker}",
                                short(
                                    &p.name,
                                    content_width.saturating_sub(
                                        1 + UnicodeWidthStr::width(marker.as_str()) as u16
                                    )
                                )
                            )),
                            Line::from(short(&preview, content_width)),
                        ],
                        index == selected,
                    )));
                    selectable.push(Some(index));
                }
                if !self.bot.conversations.is_empty() {
                    let row_total = self.bot.conversation_rows().len();
                    if row_total == 0 {
                        items.push(
                            ListItem::new(Line::from(short(
                                &text("bot-conversations-empty"),
                                body.width.saturating_sub(2),
                            )))
                            .style(selection_style(items.len() == selected)),
                        );
                        selectable.push(None);
                    }
                    let focused = if self.bot.focus_conversations {
                        "▸ "
                    } else {
                        ""
                    };
                    for group in &self.bot.conversations {
                        let label = format!(
                            "{focused}{} ({})",
                            text(match group.surface {
                                "local" => "bot-surface-local",
                                "direct" => "bot-surface-direct",
                                "group" => "bot-surface-group",
                                _ => "bot-surface-broadcast",
                            }),
                            group.rows.len()
                        );
                        items.push(
                            ListItem::new(Line::from(short(&label, body.width.saturating_sub(2))))
                                .style(heading.patch(selection_style(items.len() == selected))),
                        );
                        selectable.push(None);
                        for row in &group.rows {
                            let mut meta = format!(
                                "{} · {}",
                                row.message_count,
                                text("bot-conversation-messages")
                            );
                            if row.running {
                                meta = text("bot-conversation-running");
                            }
                            if row.favorite {
                                meta.push_str(" · ★");
                            }
                            items.push(
                                ListItem::new(vec![
                                    Line::from(format!(
                                        "  {}",
                                        short(&row.title, body.width.saturating_sub(4))
                                    )),
                                    Line::from(short(
                                        &format!("  {meta}"),
                                        body.width.saturating_sub(2),
                                    ))
                                    .style(muted),
                                ])
                                .style(selection_style(items.len() == selected)),
                            );
                            selectable.push(None);
                        }
                    }
                }
                if items.is_empty() {
                    frame.render_widget(
                        Paragraph::new(text("bot-empty")).wrap(Wrap { trim: true }),
                        body,
                    );
                } else {
                    let mut state = ListState::default().with_selected(Some(selected));
                    let _ = selectable;
                    frame.render_stateful_widget(
                        List::new(items)
                            .highlight_symbol(Line::styled("› ", selection_style(true))),
                        body,
                        &mut state,
                    );
                }
            }
            BotView::Form => {
                if let Some(form) = &self.bot.form {
                    if let Some(editor) = &form.edit {
                        let label = text(if form.index == BOT_AVATAR_FIELD {
                            "bot-avatar-path"
                        } else {
                            BOT_FORM_FIELDS[form.index]
                        });
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
                        let rows: Vec<_> = BOT_FORM_FIELDS
                            .iter()
                            .enumerate()
                            .map(|(index, key)| {
                                if index == BOT_AVATAR_FIELD {
                                    let status = text(match &form.avatar {
                                        BotAvatarDraft::Keep if form.avatar_preview().is_some() => {
                                            "bot-avatar-set"
                                        }
                                        BotAvatarDraft::Keep => "bot-avatar-empty",
                                        BotAvatarDraft::Clear => "bot-avatar-clear-pending",
                                        BotAvatarDraft::File { data: Some(_), .. } => {
                                            "bot-avatar-ready"
                                        }
                                        BotAvatarDraft::File { data: None, .. } => {
                                            "bot-avatar-unready"
                                        }
                                    });
                                    let detail = match &form.avatar {
                                        BotAvatarDraft::File { path, .. } => path.clone(),
                                        _ => text("bot-avatar-formats"),
                                    };
                                    let width = body.width.saturating_sub(7);
                                    return ListItem::new(with_avatar(
                                        form.avatar_preview(),
                                        &form.name,
                                        4,
                                        [
                                            Line::from(short(
                                                &format!("{}: {status}", text(key)),
                                                width,
                                            )),
                                            Line::from(short(&detail, width)),
                                        ],
                                        index == form.index,
                                    ));
                                }
                                ListItem::new(short(
                                    &format!("{}: {}", text(key), form.value(index)),
                                    body.width.saturating_sub(2),
                                ))
                                .style(selection_style(index == form.index))
                            })
                            .collect();
                        let mut state = ListState::default().with_selected(Some(form.index));
                        frame.render_stateful_widget(
                            List::new(rows)
                                .highlight_symbol(Line::styled("› ", selection_style(true))),
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
            BotView::DeleteConversation => {
                // A conversation with a live execution says so before the user
                // confirms, because confirming stops that work as well.
                let running = self
                    .bot
                    .delete_conversation
                    .as_ref()
                    .is_some_and(|row| row.running);
                let prompt = text(if running {
                    "bot-conversation-stop-delete-confirm"
                } else {
                    "bot-conversation-delete-confirm"
                });
                let title = self
                    .bot
                    .delete_conversation
                    .as_ref()
                    .map_or("", |row| row.title.as_str());
                frame.render_widget(
                    Paragraph::new(format!(
                        "{}\n{prompt}",
                        agena_tui::sanitize_picker_text(title)
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
