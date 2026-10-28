use super::*;

fn reply_for(reference: &str, read: &InlineUiRead) -> InlineReply {
    let mut reply = InlineReply {
        focused: true,
        ..Default::default()
    };
    if let Some(document) = read.document() {
        reply.documents.insert(reference.into(), document.clone());
    }
    reply.states.insert(reference.into(), read.states.clone());
    reply
        .presentation
        .insert(reference.into(), read.presentation.clone());
    if let Some(snapshot) = &read.snapshot {
        reply
            .phases
            .insert(reference.into(), (snapshot.revision, snapshot.phase));
    }
    reply
}
pub(super) fn rendered_controls(rows: &[RenderedLine]) -> Vec<(usize, RenderedTranscriptControl)> {
    let mut result = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        for control in row.controls.iter().filter(|control| control.enabled) {
            if matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.event!=InlineEvent::Open)
                && !result
                    .iter()
                    .any(|(_, seen): &(usize, RenderedTranscriptControl)| {
                        seen.action == control.action
                    })
            {
                result.push((index, control.clone()));
            }
        }
    }
    result
}
pub(super) fn focus_target(
    dialog: &InlineUiDialog,
    rows: &[RenderedLine],
) -> Option<(usize, InlineControl)> {
    let controls = rendered_controls(rows);
    let found=dialog.native_focus.as_ref().and_then(|target|controls.iter().find(|(_,control)|control.action==TranscriptControlAction::InlineUi(target.clone())))
        .or_else(||controls.iter().find(|(_,control)|matches!(&control.action,TranscriptControlAction::InlineUi(target) if target.node==dialog.focused)))
        .or_else(||controls.first());
    let (line, control) = found?;
    let TranscriptControlAction::InlineUi(target) = &control.action else {
        return None;
    };
    Some((*line, target.clone()))
}

pub(crate) fn render_modal(
    frame: &mut Frame,
    area: Rect,
    dialog: &InlineUiDialog,
    read: Option<&InlineUiRead>,
    i18n: &agena_tui::i18n::I18n,
) {
    use agena_tui_components::{FramedSurfaceSpec, SurfaceMode, render_framed_surface, theme};
    let editing = dialog.editing.is_some() || dialog.picker;
    let target_width = if editing {
        area.width.min(72)
    } else {
        area.width.min(150)
    };
    let width = SurfaceMode::Overlay
        .content_width(area, target_width)
        .max(1);
    let mut title = i18n.text("inline-ui-title");
    let mut rows = Vec::new();
    let mut choice_focus = None;
    if editing {
        if let (Some(read), Some(target)) = (read, &dialog.native_focus) {
            if let Some((_, _, node)) = resolve(read, target) {
                title = field(&node, "label");
                if dialog.picker {
                    if let Some(spec) = &node.field {
                        for option in &spec.options {
                            let selected = dialog
                                .choice
                                .as_ref()
                                .map_or(node.value.as_str() == Some(&option.value), |value| {
                                    value == &option.value
                                });
                            let text =
                                format!("{} {}", if selected { "●" } else { "○" }, option.label);
                            let mut row = RenderedLine::plain(
                                agena_tui_components::truncate_display_text(
                                    &agena_tui_transcript::sanitize_terminal_text(&text)
                                        .replace(['\n', '\t'], " "),
                                    usize::from(width),
                                ),
                                if selected {
                                    theme::selection_style()
                                } else {
                                    Style::default()
                                },
                            );
                            let option_target = InlineControl {
                                event: InlineEvent::Choose(option.value.clone()),
                                ..target.clone()
                            };
                            if selected {
                                choice_focus = Some((rows.len(), option_target.clone()));
                            }
                            row.controls.push(RenderedTranscriptControl {
                                columns: 0..row.text.width(),
                                action: TranscriptControlAction::InlineUi(option_target),
                                enabled: true,
                            });
                            rows.push(row);
                        }
                    }
                } else {
                    let value = dialog.editing.as_deref().unwrap_or("");
                    let cursor = dialog.edit_cursor.unwrap_or(value.len()).min(value.len());
                    rows.push(RenderedLine::plain(
                        editor_line(value, cursor, width),
                        Style::default().fg(theme::accent_color()),
                    ));
                    let description = field(&node, "description");
                    if !description.is_empty() {
                        rows.extend(plain_lines(&description, width, theme::muted_style()));
                    }
                }
            } else {
                rows.push(RenderedLine::dim(i18n.text("inline-ui-control-changed")));
            }
        }
    } else if let Some(read) = read {
        let reply = reply_for(&dialog.reference, read);
        if let Some(document) = reply.documents.get(&dialog.reference) {
            rows = agena_tui_transcript::inline_ui::render_document(
                document,
                &dialog.reference,
                &reply,
                width,
                i18n,
            );
        }
    }
    if rows.is_empty() {
        rows.push(RenderedLine::dim(
            read.and_then(|read| read.error.clone())
                .unwrap_or_else(|| i18n.text("inline-ui-loading")),
        ));
    }
    let focus = choice_focus.or_else(|| focus_target(dialog, &rows));
    let footer_text = if width < 65 {
        i18n.text("inline-ui-keys-short")
    } else if editing {
        i18n.text("inline-ui-editor-help")
    } else {
        i18n.text("inline-ui-native-help")
    };
    let mut footer = plain_lines(&footer_text, width, theme::muted_style());
    if let Some(error) = dialog
        .error
        .as_ref()
        .or_else(|| read.and_then(|read| read.error.as_ref()))
    {
        footer.extend(plain_lines(
            error,
            width,
            Style::default().fg(theme::danger_color()),
        ));
    }
    let desired = rows
        .len()
        .saturating_add(footer.len())
        .saturating_add(2)
        .min(usize::from(u16::MAX)) as u16;
    let surface = render_framed_surface(
        frame,
        area,
        SurfaceMode::Overlay,
        &FramedSurfaceSpec {
            title: agena_tui_transcript::sanitize_terminal_text(&title)
                .replace(['\n', '\t'], " ")
                .into(),
            target_width,
            target_height: desired.max(5),
        },
    );
    if surface.inner.width == 0 || surface.inner.height == 0 {
        return;
    }
    let split = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),
            Constraint::Length((footer.len() as u16).min(surface.inner.height.saturating_sub(1))),
        ])
        .split(surface.inner);
    let max = rows
        .len()
        .saturating_sub(usize::from(split[0].height))
        .min(usize::from(u16::MAX)) as u16;
    dialog.scroll_limit.set(max);
    let mut scroll = dialog.scroll.min(max);
    if (dialog.reveal_native.replace(false) || dialog.picker)
        && let Some((line, _)) = &focus
    {
        if *line < usize::from(scroll) {
            scroll = *line as u16;
        } else if *line >= usize::from(scroll) + usize::from(split[0].height) {
            scroll = (*line + 1).saturating_sub(usize::from(split[0].height)) as u16;
        }
    }
    // The layout carries full semantic actions for the next actual pointer event.
    *dialog.native_layout.borrow_mut() = NativeLayout {
        body: split[0],
        rows: rows.clone(),
        scroll,
    };
    let focus_action = focus.map(|(_, target)| TranscriptControlAction::InlineUi(target));
    let lines = rows
        .iter()
        .skip(usize::from(scroll))
        .take(usize::from(split[0].height))
        .map(|row| {
            row.rich_line
                .clone()
                .unwrap_or_else(|| Line::styled(row.text.clone(), row.style))
        })
        .collect::<Vec<_>>();
    frame.render_widget(Paragraph::new(lines), split[0]);
    if !editing && let Some(focus) = focus_action {
        for (index, row) in rows
            .iter()
            .enumerate()
            .skip(usize::from(scroll))
            .take(usize::from(split[0].height))
        {
            for control in &row.controls {
                if control.enabled && control.action == focus {
                    let y = split[0].y + (index - usize::from(scroll)) as u16;
                    for col in control.columns.clone() {
                        if col < usize::from(split[0].width) {
                            frame.buffer_mut()[(split[0].x + col as u16, y)]
                                .set_style(theme::selection_style());
                        }
                    }
                }
            }
        }
    }
    frame.render_widget(
        Paragraph::new(
            footer
                .into_iter()
                .map(|row| {
                    row.rich_line
                        .unwrap_or_else(|| Line::styled(row.text, row.style))
                })
                .collect::<Vec<_>>(),
        ),
        split[1],
    );
}
fn plain_lines(text: &str, width: u16, style: Style) -> Vec<RenderedLine> {
    text.split('\n')
        .flat_map(|line| {
            agena_tui_transcript::wrap_rich_line(
                &[ratatui::text::Span::styled(line.to_owned(), style)],
                usize::from(width.max(1)),
                usize::from(width.max(1)),
            )
            .into_iter()
            .map(RenderedLine::rich)
        })
        .collect()
}

fn editor_line(value: &str, mut cursor: usize, width: u16) -> String {
    use unicode_segmentation::UnicodeSegmentation;
    if !value.is_char_boundary(cursor) {
        cursor = value.len();
    }
    let width = usize::from(width.max(1));
    let (before, after) = value.split_at(cursor);
    let before = agena_tui_transcript::sanitize_terminal_text(before).replace(['\n', '\t'], " ");
    let after = agena_tui_transcript::sanitize_terminal_text(after).replace(['\n', '\t'], " ");
    let tail = after.width().min(width / 3);
    let before_budget = width.saturating_sub(1 + tail);
    let mut used = 0;
    let mut start = before.len();
    for (index, grapheme) in before.grapheme_indices(true).rev() {
        if used + grapheme.width() > before_budget {
            break;
        }
        used += grapheme.width();
        start = index;
    }
    let suffix =
        agena_tui_components::truncate_display_text(&after, width.saturating_sub(used + 1));
    format!("{}▏{suffix}", &before[start..])
}
