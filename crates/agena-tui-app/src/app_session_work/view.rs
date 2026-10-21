use super::*;
use agena_tui_components::{FramedSurfaceSpec, SurfaceMode, render_framed_surface};

impl App {
    /// Number of workspace files this session recorded edits for, when the
    /// summary has loaded and is non-empty.
    pub(crate) fn session_work_file_count(&self) -> Option<usize> {
        if !self.session_work_has_files() {
            return None;
        }
        let id = self.transcript.session_id?;
        let files = self.session_work.get(&id)?.files.as_ref()?;
        Some(files.total_files)
    }

    /// Background activities attached to the current session execution.
    pub(crate) fn session_work_task_count(&self) -> usize {
        self.transcript
            .execution
            .as_ref()
            .map_or(0, |execution| execution.background_activities.len())
    }

    /// Open the full-screen session work surface on `tab`.
    pub(crate) fn open_session_work_panel(&mut self, tab: Tab) {
        let Some(id) = self.current_or_selected_session_id() else {
            self.flash_warning(self.i18n.text("flash-session-work-requires-session"));
            return;
        };
        if self.transcript.session_id != Some(id) {
            let title = self.current_or_selected_session_title().unwrap_or_default();
            self.open_session(id, title);
        }
        let state = self.session_work.entry(id).or_default();
        state.tab = tab;
        state.selected = 0;
        state.select_detail(None);
        state.detail_at = None;
        state.file_at = None;
        self.route_stack.clear();
        self.current_route = Route::SessionWork(id);
        self.focus = Focus::Transcript;
        self.heal_session_work();
    }

    pub(crate) fn close_session_work_panel(&mut self) {
        if let Some(id) = self.transcript.session_id
            && let Some(state) = self.session_work.get_mut(&id)
        {
            state.cancel_detail();
        }
        self.current_route = Route::Main;
        self.focus = Focus::Composer;
    }

    pub(crate) fn render_session_work_panel(&mut self, frame: &mut Frame, area: Rect) {
        let surface = render_framed_surface(
            frame,
            area,
            SurfaceMode::Route,
            &FramedSurfaceSpec {
                title: self.i18n.text("session-work-title").into(),
                target_width: area.width,
                target_height: area.height,
            },
        );
        let area = surface.inner;
        if area.is_empty() {
            return;
        }
        let Some(id) = self.transcript.session_id else {
            return;
        };
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
        let has_files = self.session_work_has_files();
        let state = self.session_work.entry(id).or_default();
        if let Some(Detail::Task(task)) = &state.detail
            && let Some(activity) = activities.iter().find(|a| &a.id == task)
        {
            state.detail_activity = Some(activity.clone());
        }
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
        let back = self.i18n.text("session-work-back");
        let header = Rect { height: 1, ..area };
        frame.render_widget(Paragraph::new("").style(theme::status_chip_style()), header);
        let mut header_buttons = vec![(back.as_str(), PointerAction::Named("work-leave"))];
        if !activities.is_empty() {
            header_buttons.push((&tasks_label, PointerAction::Named("work-tasks")));
        }
        if has_files {
            header_buttons.push((&files_label, PointerAction::Named("work-files")));
        }
        if let Some(label) = &status_label {
            header_buttons.push((label, PointerAction::Named("work-status")));
        }
        pointer::render_buttons(frame, header, &header_buttons);
        let mut title = self.i18n.text(match state.tab {
            Tab::Tasks => "session-work-tasks",
            Tab::Files => "session-work-workspace",
            Tab::Status => "session-work-status",
        });
        if state.tab == Tab::Files && state.files.as_ref().is_some_and(|f| f.recording_incomplete) {
            title.push_str(" · ");
            title.push_str(&self.i18n.text("session-work-recording-incomplete"));
        }
        let refresh = self.i18n.text("plan-viewer-refresh");
        let switch = self.i18n.text("session-work-switch");
        let mut actions: Vec<(&str, PointerAction)> = Vec::new();
        if state.detail.is_some() {
            actions.insert(0, (&back, PointerAction::Named("work-back")));
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
                                .map(|l| format!(
                                    "{}{}",
                                    crate::sanitize_terminal_text(&l.text),
                                    if l.chunk || l.text.ends_with('\n') {
                                        ""
                                    } else {
                                        "\n"
                                    }
                                ))
                                .collect::<Vec<_>>()
                                .join("")
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
                                    "{}× {}",
                                    f.operation_count,
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
        let footer = Rect::new(area.x, area.bottom().saturating_sub(1), area.width, 1);
        if state.diff_truncated
            && state.diff_limit < 2 * 1024 * 1024
            && matches!(state.detail, Some(Detail::File(..)))
        {
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
            return;
        }
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
