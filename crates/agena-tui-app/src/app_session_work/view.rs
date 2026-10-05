use super::*;

impl App {
    pub(crate) fn session_work_height(&self, available: u16) -> u16 {
        if !self.session_work_visible() || available == 0 {
            return 0;
        }
        if let Some(state) = self
            .transcript
            .session_id
            .and_then(|id| self.session_work.get(&id))
            .filter(|s| s.expanded && available >= 4)
        {
            let rows = if state.detail.is_some() {
                state.rendered.len().max(state.diff.lines().count()).max(3)
            } else if state.tab == Tab::Tasks {
                self.transcript
                    .execution
                    .as_ref()
                    .map_or(0, |e| e.background_activities.len())
            } else {
                state.files.as_ref().map_or(0, |p| p.files.len())
            };
            available.min((rows.max(1) + 3).min(14) as u16)
        } else {
            1
        }
    }

    pub(crate) fn render_session_work(&mut self, frame: &mut Frame, area: Rect) {
        self.work_area = area;
        let Some(id) = self.transcript.session_id else {
            return;
        };
        if area.is_empty() {
            return;
        }
        if !self.session_work_visible() {
            return;
        }
        let has_files = self.session_work_has_files();
        // Use the same visible vertical edges as the composer. A background
        // painted to the edges of the terminal cells appears wider than its border.
        let border = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::LEFT | ratatui::widgets::Borders::RIGHT);
        let inner = border.inner(area);
        frame.render_widget(border, area);
        let area = inner;
        let activities = self
            .transcript
            .execution
            .as_ref()
            .map(|e| e.background_activities.clone())
            .unwrap_or_default();
        let retry = self
            .transcript
            .execution
            .as_ref()
            .and_then(|e| e.execution.provider_retry.clone());
        let state = self.session_work.entry(id).or_default();
        if let Some(Detail::Task(task)) = &state.detail
            && let Some(activity) = activities.iter().find(|a| &a.id == task)
        {
            state.detail_activity = Some(activity.clone());
        }
        let expanded = state.expanded && area.height >= 4;
        let tasks_label = format!(
            "{} {}",
            self.i18n.text("session-work-tasks"),
            activities.len()
        );
        let files_label = match state.files.as_ref() {
            Some(files) => format!(
                "{} {}",
                self.i18n.text("session-work-files"),
                files.total_files
            ),
            None if state.file_task.is_some() => self.i18n.text("session-work-loading"),
            None => self.i18n.text("session-work-files"),
        };
        let status_label = retry.as_ref().map(|retry| {
            format!(
                "↻ {} · {}s",
                retry.attempt,
                ((retry.next_at_ms - chrono::Utc::now().timestamp_millis()).max(0) + 999) / 1000
            )
        });
        let toggle_label = format!(
            "{} {}",
            if expanded { "▾" } else { "▸" },
            self.i18n.text("session-work-title")
        );
        let header = Rect { height: 1, ..area };
        // Paint the complete row, aligned with the composer, even when only
        // a few action labels are present. Click empty space to toggle too.
        frame.render_widget(Paragraph::new("").style(theme::status_chip_style()), header);
        pointer::register(header, Some(PointerAction::Named("work-toggle")), None);
        let mut buttons = vec![(toggle_label.as_str(), PointerAction::Named("work-toggle"))];
        if let Some(label) = &status_label {
            buttons.push((label, PointerAction::Named("work-status")));
        }
        if !activities.is_empty() {
            buttons.push((&tasks_label, PointerAction::Named("work-tasks")));
        }
        if has_files {
            buttons.push((&files_label, PointerAction::Named("work-files")));
        }
        pointer::render_buttons(frame, Rect { height: 1, ..area }, &buttons);
        if !expanded {
            return;
        }
        let title = self.i18n.text(match state.tab {
            Tab::Tasks => "session-work-tasks",
            Tab::Files => "session-work-workspace",
            Tab::Status => "session-work-status",
        });
        let refresh = self.i18n.text("plan-viewer-refresh");
        let back = self.i18n.text("session-work-back");
        let switch = self.i18n.text("session-work-switch");
        let mut actions: Vec<(&str, PointerAction)> = Vec::new();
        if state.detail.is_some() {
            actions.insert(0, (&back, PointerAction::Named("work-back")));
        }
        let staged = self
            .i18n
            .text(if matches!(state.detail, Some(Detail::File(_, true))) {
                "session-work-staged"
            } else {
                "session-work-working"
            });
        if matches!(state.detail, Some(Detail::File(..))) {
            actions.push((&staged, PointerAction::Named("work-staged")));
        }
        let control_labels: Vec<_> = if let Some(Detail::Task(task)) = &state.detail {
            activities
                .iter()
                .find(|a| &a.id == task)
                .map(|a| {
                    a.controls
                        .iter()
                        .filter_map(|action| {
                            control_action(action).map(|name| {
                                (self.i18n.text(&format!("session-work-{action}")), name)
                            })
                        })
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        if state.control_request == 0 {
            for (label, action) in &control_labels {
                actions.push((label, PointerAction::Named(action)));
            }
        }
        // Keep task controls and diff source reachable before utility actions
        // when a narrow terminal cannot fit every button on this row.
        actions.push((&switch, PointerAction::Named("work-next-tab")));
        actions.push((&refresh, PointerAction::Named("work-refresh")));
        pointer::render_buttons(
            frame,
            Rect::new(area.x, area.y + 1, area.width, 1),
            &actions,
        );
        let body = Rect::new(
            area.x + 1,
            area.y + 2,
            area.width.saturating_sub(2),
            area.height.saturating_sub(3),
        );
        pointer::register(
            body,
            Some(PointerAction::Named("work-focus")),
            Some((
                PointerAction::Named("work-up"),
                PointerAction::Named("work-down"),
            )),
        );
        let error = state
            .control_error
            .as_ref()
            .or(state.detail_error.as_ref())
            .or(if state.tab == Tab::Files {
                state.file_error.as_ref()
            } else {
                None
            });
        let text = if let Some(error) = error {
            Some(error.clone())
        } else if state.detail.is_some() {
            Some(match &state.detail {
                Some(Detail::Task(_)) => state
                    .logs
                    .as_ref()
                    .map(|l| {
                        let mut heading = String::new();
                        if let Some(activity) = &state.detail_activity {
                            heading.push_str(&activity.title);
                            if !activity.description.is_empty()
                                && activity.description != activity.title
                            {
                                heading.push_str(&format!("\n{}", activity.description));
                            }
                            if let Some(failure) = &activity.failure {
                                heading.push_str(&format!("\n{}", failure.user.fallback));
                            }
                            if activity.status == "waiting"
                                && let Some(next) = activity
                                    .next_event_at_ms
                                    .and_then(chrono::DateTime::from_timestamp_millis)
                            {
                                heading.push_str(&format!(
                                    "\n{} {}",
                                    self.i18n.text("session-work-next-wake"),
                                    next.with_timezone(&chrono::Local).format("%m-%d %H:%M:%S")
                                ));
                            }
                        }
                        format!(
                            "{heading}\n{} · {}\n{}",
                            self.i18n.text("session-work-log-tail"),
                            self.i18n.text(&format!("session-work-status-{}", l.status)),
                            l.lines
                                .iter()
                                .map(|l| crate::sanitize_terminal_text(&l.text))
                                .collect::<Vec<_>>()
                                .join("\n")
                        )
                    })
                    .unwrap_or_else(|| self.i18n.text("plan-viewer-loading")),
                _ => state.diff.clone(),
            })
        } else if state.tab == Tab::Status {
            Some(retry.map(|r| r.message).unwrap_or_default())
        } else {
            None
        };
        if let Some(text) = text {
            let key = (text.clone(), body.width, theme::accent_color());
            if state.render_key.as_ref() != Some(&key) {
                state.rendered = if matches!(state.detail, Some(Detail::File(..)))
                    && error.is_none()
                    && !text.is_empty()
                {
                    agena_tui_transcript::render_diff_document(&text, body.width)
                        .into_iter()
                        .map(|l| l.rich_line.unwrap_or_else(|| Line::styled(l.text, l.style)))
                        .collect()
                } else {
                    crate::sanitize_terminal_text(&text)
                        .lines()
                        .map(|l| Line::from(l.to_owned()))
                        .collect()
                };
                if state.rendered.is_empty() {
                    state.rendered.push(Line::from(self.i18n.text(
                        if state.detail_task.is_some() {
                            "plan-viewer-loading"
                        } else {
                            "session-work-empty"
                        },
                    )));
                }
                state.render_key = Some(key);
            }
            let paragraph = Paragraph::new(state.rendered.clone()).wrap(Wrap { trim: false });
            state.max_scroll = paragraph
                .line_count(body.width)
                .saturating_sub(body.height as usize)
                .min(u16::MAX as usize) as u16;
            state.scroll = state.scroll.min(state.max_scroll);
            frame.render_widget(paragraph.scroll((state.scroll, 0)), body);
        } else {
            let rows: Vec<String> = if state.tab == Tab::Tasks {
                activities
                    .iter()
                    .map(|a| {
                        format!(
                            "{} · {}",
                            self.i18n.text(&format!("session-work-status-{}", a.status)),
                            crate::sanitize_terminal_text(&a.title)
                        )
                    })
                    .collect()
            } else {
                state
                    .files
                    .as_ref()
                    .map(|p| {
                        p.files
                            .iter()
                            .map(|f| {
                                format!(
                                    "{}{} {}",
                                    f.index,
                                    f.working_dir,
                                    crate::sanitize_terminal_text(&f.path)
                                )
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            };
            state.selected = state.selected.min(rows.len().saturating_sub(1));
            let start = state
                .selected
                .saturating_sub(body.height.saturating_sub(1) as usize);
            if rows.is_empty() {
                frame.render_widget(
                    Paragraph::new(self.i18n.text(if state.file_task.is_some() {
                        "plan-viewer-loading"
                    } else {
                        "session-work-empty"
                    }))
                    .style(theme::muted_style()),
                    body,
                );
            }
            for (line, (index, row)) in rows
                .iter()
                .enumerate()
                .skip(start)
                .take(body.height as usize)
                .enumerate()
            {
                let rect = Rect::new(body.x, body.y + line as u16, body.width, 1);
                frame.render_widget(
                    Paragraph::new(format!(
                        "{} {row}",
                        if index == state.selected { "›" } else { " " }
                    ))
                    .style(if index == state.selected {
                        theme::status_chip_style()
                    } else {
                        theme::muted_style()
                    }),
                    rect,
                );
                pointer::register(
                    rect,
                    Some(PointerAction::NamedIndex("work-row", index)),
                    Some((
                        PointerAction::Named("work-up"),
                        PointerAction::Named("work-down"),
                    )),
                );
            }
        }
        let footer = Rect::new(area.x, area.bottom() - 1, area.width, 1);
        if state.diff_truncated && matches!(state.detail, Some(Detail::File(..))) {
            pointer::render_buttons(
                frame,
                footer,
                &[(
                    &self.i18n.text("session-work-more-diff"),
                    PointerAction::Named("work-more-diff"),
                )],
            );
            return;
        }
        if state.tab == Tab::Files
            && state.detail.is_none()
            && (state.page > 0 || state.files.as_ref().is_some_and(|f| f.has_more))
        {
            pointer::render_buttons(
                frame,
                footer,
                &[
                    (
                        &self.i18n.text("session-work-previous"),
                        PointerAction::Named("work-prev"),
                    ),
                    (
                        &self.i18n.text("session-work-next"),
                        PointerAction::Named("work-next"),
                    ),
                ],
            );
        } else {
            frame.render_widget(
                Paragraph::new(format!(
                    "{title} · {}",
                    self.i18n.text("session-work-footer")
                ))
                .style(theme::muted_style()),
                footer,
            );
            pointer::register(footer, Some(PointerAction::Named("work-leave")), None);
        }
    }
}

fn control_action(action: &str) -> Option<&'static str> {
    match action {
        "stop" => Some("work-control-stop"),
        "pause" => Some("work-control-pause"),
        "resume" => Some("work-control-resume"),
        "dismiss" => Some("work-control-dismiss"),
        "delete" => Some("work-control-delete"),
        _ => None,
    }
}
