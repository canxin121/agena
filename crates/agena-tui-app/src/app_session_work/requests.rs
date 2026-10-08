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
        let summary = !state.expanded || state.tab != Tab::Files;
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
        if state.detail_task.is_some() || !state.expanded {
            return;
        }
        let Some(detail) = state.detail.clone() else {
            return;
        };
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.detail_request = request_id;
        state.detail_at = Some(Instant::now());
        state.detail_dirty = false;
        let cursor = state.logs.as_ref().map_or_else(
            || {
                let Some(Detail::Task(task)) = &state.detail else {
                    return 0;
                };
                self.transcript
                    .execution
                    .as_ref()
                    .and_then(|e| e.background_activities.iter().find(|a| &a.id == task))
                    .map_or(0, |a| a.last_seq.saturating_sub(200))
            },
            |l| l.last_seq,
        );
        let max_bytes = state.diff_limit.max(256 * 1024);
        let history_label = self.i18n.text("session-work-operation-history");
        let operation_diff_label = self.i18n.text("session-work-operation-diff");
        let truncated_label = self.i18n.text("session-work-recorded-truncated");
        let no_diff_label = self.i18n.text("session-work-no-recorded-diff");
        let application = self.application.clone();
        let tx = self.tx.clone();
        state.detail_task = Some(ReadTask(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                match detail {
                    Detail::Task(task) => Ok(WorkResult::Logs(
                        crate::app_backend::activities::activity_logs(
                            &application,
                            &task,
                            cursor,
                            Some(200),
                            0,
                        )
                        .await?,
                    )),
                    Detail::File(path) => {
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
                    }
                }
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
            _ => state.control_request,
        };
        if request_id != expected || expected == 0 {
            return;
        }
        let control_observation = if channel == 2 {
            state.control_observation.take()
        } else {
            None
        };
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
                state.control_request = 0;
                state.control_error = None;
            }
        }
        match result {
            Ok(WorkResult::Files(files)) => {
                if state.page > 0
                    && files.files.is_empty()
                    && state.expanded
                    && state.tab == Tab::Files
                {
                    state.page = 0;
                    state.file_at = None;
                }
                state.selected = state.selected.min(files.files.len().saturating_sub(1));
                state.files = Some(files);
            }
            Ok(WorkResult::Logs(logs)) => {
                state.logs = Some(merge_log_tail(state.logs.take(), logs));
            }
            Ok(WorkResult::Diff(diff, truncated)) => {
                state.diff = diff;
                state.diff_truncated = truncated;
            }
            Ok(WorkResult::Control(activity)) => {
                if let Some((task, action, observed)) = control_observation
                    && task == activity.id
                    && observed == state.activity_times.get(&task).copied()
                    && self.transcript.session_id == Some(id)
                    && let Some(execution) = self.transcript.execution.as_mut()
                {
                    if matches!(action.as_str(), "dismiss" | "delete") {
                        execution.background_activities.retain(|row| row.id != task);
                    } else if let Some(row) = execution
                        .background_activities
                        .iter_mut()
                        .find(|row| row.id == task)
                    {
                        if row != activity.as_ref() {
                            *row = *activity;
                        }
                    } else {
                        execution.background_activities.insert(0, *activity);
                    }
                    if matches!(&state.detail, Some(Detail::Task(selected)) if selected == &task) {
                        state.detail_dirty = true;
                    }
                }
            }
            Err(error) => {
                let error = crate::sanitize_terminal_text(&error.to_string());
                match channel {
                    0 => state.file_error = Some(error),
                    1 => state.detail_error = Some(error),
                    _ => state.control_error = Some(error),
                }
            }
        }
        if channel == 0 {
            self.heal_session_work_selection(id);
        }
    }

    pub(super) fn control_work_activity(&mut self, action: &str) {
        let Some(id) = self.transcript.session_id else {
            return;
        };
        let Some(state) = self.session_work.get_mut(&id) else {
            return;
        };
        if state.control_request != 0 {
            return;
        }
        let Some(Detail::Task(task)) = &state.detail else {
            return;
        };
        let Some(activity) = self
            .transcript
            .execution
            .as_ref()
            .and_then(|e| e.background_activities.iter().find(|a| &a.id == task))
        else {
            return;
        };
        if !activity.controls.iter().any(|a| a == action) {
            return;
        }
        let task = task.clone();
        let action = action.to_owned();
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.control_request = request_id;
        state.control_observation = Some((
            task.clone(),
            action.clone(),
            state.activity_times.get(&task).copied(),
        ));
        let application = self.application.clone();
        let tx = self.tx.clone();
        // Mutations retain their original activity/session even after navigation.
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(15),
                crate::app_backend::activities::control_activity(&application, &task, &action),
            )
            .await
            .map_err(crate::UiFailure::internal)
            .and_then(|r| {
                r.map(|activity| WorkResult::Control(Box::new(activity)))
                    .map_err(crate::UiFailure::internal)
            });
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
