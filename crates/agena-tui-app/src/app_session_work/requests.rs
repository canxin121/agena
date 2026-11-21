use super::*;

impl App {
    pub(super) fn request_work_files(&mut self, id: i64) {
        let state = self.session_work.entry(id).or_default();
        if state.file_task.is_some() {
            return;
        }
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.file_request = request_id;
        state.file_at = Some(Instant::now());
        state.file_dirty = false;
        let page = state.page;
        let summary = state.tab != Tab::Files;
        let application = self.application.clone();
        let tx = self.tx.clone();
        state.file_task = Some(ReadTask(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                let value = application
                    .client()
                    .session_file_changes(id, page * 40, summary, None, 256 * 1024)
                    .await?;
                Ok::<_, anyhow::Error>(WorkResult::Files(value))
            })
            .await
            .map_err(crate::UiFailure::internal)
            .and_then(|r| r.map_err(crate::UiFailure::internal));
            let _ = tx
                .send(AppMessage::SessionWorkLoaded {
                    session_id: id,
                    request_id,
                    channel: 0,
                    result,
                })
                .await;
        })));
    }

    pub(super) fn request_work_detail(&mut self, id: i64) {
        let Some(state) = self.session_work.get_mut(&id) else {
            return;
        };
        if state.detail_task.is_some() {
            return;
        }
        let Some(Detail::File(path)) = state.detail.clone() else {
            return;
        };
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.detail_request = request_id;
        state.detail_at = Some(Instant::now());
        state.detail_dirty = false;
        let max_bytes = state.diff_limit.max(256 * 1024);
        let history_label = self.i18n.text("session-work-operation-history");
        let operation_diff_label = self.i18n.text("session-work-operation-diff");
        let truncated_label = self.i18n.text("session-work-recorded-truncated");
        let no_diff_label = self.i18n.text("session-work-no-recorded-diff");
        let application = self.application.clone();
        let tx = self.tx.clone();
        state.detail_task = Some(ReadTask(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                let value = application
                    .client()
                    .session_file_changes(id, 0, false, Some(&path), max_bytes)
                    .await?;
                let mut text = String::new();
                let mut truncated = false;
                if let Some(file) = value.files.first() {
                    if file.operation_history {
                        text.push_str(&format!("{history_label}\n"));
                    }
                    for op in &file.operations {
                        text.push_str(&format!(
                            "\n#{} · {} · {} · {}\n",
                            op.part_id, op.tool, op.kind, file.path
                        ));
                        if let Some(from) = &op.from_path {
                            text.push_str(&format!("{from} → {}\n", file.path));
                        }
                        if op.before_sha256.is_some() || op.after_sha256.is_some() {
                            text.push_str(&format!(
                                "SHA {} → {}\n",
                                op.before_sha256.as_deref().unwrap_or("?"),
                                op.after_sha256.as_deref().unwrap_or("?")
                            ));
                        }
                        if op.diff_scope == "operation" {
                            text.push_str(&format!("{operation_diff_label}\n"));
                        }
                        text.push_str(op.diff.as_deref().unwrap_or_else(|| {
                            op.diff_unavailable_reason
                                .as_deref()
                                .unwrap_or(&no_diff_label)
                        }));
                        if op.diff_truncated {
                            text.push_str(&format!("\n{truncated_label}\n"));
                            truncated = true;
                        }
                    }
                }
                Ok(WorkResult::Diff(text, truncated))
            })
            .await
            .map_err(crate::UiFailure::internal)
            .and_then(|r: Result<WorkResult, anyhow::Error>| r.map_err(crate::UiFailure::internal));
            let _ = tx
                .send(AppMessage::SessionWorkLoaded {
                    session_id: id,
                    request_id,
                    channel: 1,
                    result,
                })
                .await;
        })));
    }

    pub(crate) fn handle_session_work_loaded(
        &mut self,
        id: i64,
        request_id: u64,
        channel: u8,
        result: crate::UiResult<WorkResult>,
    ) {
        let Some(state) = self.session_work.get_mut(&id) else {
            return;
        };
        let expected = match channel {
            0 => state.file_request,
            1 => state.detail_request,
            _ => state.schedule_request,
        };
        if request_id != expected || expected == 0 {
            return;
        }
        match channel {
            0 => {
                state.file_task = None;
                state.file_at = Some(Instant::now());
                state.file_error = None;
            }
            1 => {
                state.detail_task = None;
                state.detail_at = Some(Instant::now());
                state.detail_error = None;
            }
            _ => {
                state.schedule_request = 0;
                state.schedule_error = None;
            }
        }
        match result {
            Ok(WorkResult::Files(files)) => {
                if state.page > 0 && files.files.is_empty() && state.tab == Tab::Files {
                    state.page = 0;
                    state.file_at = None;
                }
                if state.tab == Tab::Files {
                    state.selected = state.selected.min(files.files.len().saturating_sub(1));
                }
                state.files = Some(files);
            }
            Ok(WorkResult::Diff(diff, truncated)) => {
                state.diff = diff;
                state.diff_truncated = truncated;
            }
            Ok(WorkResult::Schedules(schedules)) => {
                if matches!(&state.detail, Some(Detail::Schedule(id)) if !schedules.iter().any(|row| &row.id == id))
                {
                    state.select_detail(None);
                }
                if state.tab == Tab::Schedules {
                    state.selected = state.selected.min(schedules.len().saturating_sub(1));
                }
                state.schedules = schedules;
            }
            Err(error) => {
                let error = crate::sanitize_terminal_text(&error.to_string());
                match channel {
                    0 => state.file_error = Some(error),
                    1 => state.detail_error = Some(error),
                    _ => state.schedule_error = Some(error),
                }
            }
        }
        if channel == 0 {
            self.heal_session_work_selection(id);
        }
    }

    pub(super) fn control_work_schedule(&mut self, id: i64, delete: bool) {
        let Some(state) = self.session_work.get(&id) else {
            return;
        };
        let Some(Detail::Schedule(schedule_id)) = &state.detail else {
            return;
        };
        let Some(job) = state.schedules.iter().find(|job| &job.id == schedule_id) else {
            return;
        };
        let Ok(schedule_id) = uuid::Uuid::parse_str(schedule_id) else {
            return;
        };
        let action = if delete {
            ScheduleAction::Delete(schedule_id)
        } else if !job.completed {
            ScheduleAction::Pause(schedule_id, !job.paused)
        } else {
            return;
        };
        self.request_work_schedules(id, Some(action));
    }

    pub(super) fn request_work_schedules(&mut self, id: i64, action: Option<ScheduleAction>) {
        let state = self.session_work.entry(id).or_default();
        if state.schedule_request != 0 {
            return;
        }
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.schedule_request = request_id;
        state.schedules_at = Some(Instant::now());
        state.schedule_error = None;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let client = application.client();
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                match action {
                    Some(ScheduleAction::Pause(schedule_id, paused)) => {
                        client
                            .set_session_schedule_paused(id, schedule_id, paused)
                            .await?;
                    }
                    Some(ScheduleAction::Delete(schedule_id)) => {
                        client.delete_session_schedule(id, schedule_id).await?;
                    }
                    None => {}
                }
                client
                    .session_schedules(id)
                    .await
                    .map(WorkResult::Schedules)
            })
            .await
            .map_err(crate::UiFailure::internal)
            .and_then(|result| result.map_err(crate::UiFailure::internal));
            let _ = tx
                .send(AppMessage::SessionWorkLoaded {
                    session_id: id,
                    request_id,
                    channel: 2,
                    result,
                })
                .await;
        });
    }
}

pub(super) enum ScheduleAction {
    Pause(uuid::Uuid, bool),
    Delete(uuid::Uuid),
}
