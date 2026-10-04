use super::*;

impl App {
    pub(super) fn request_work_files(&mut self, id: i64) {
        let state = self.session_work.entry(id).or_default();
        if state.file_task.is_some() || state.directory.is_empty() {
            return;
        }
        self.next_usage_request_id += 1;
        let request_id = self.next_usage_request_id;
        state.file_request = request_id;
        state.file_at = Some(Instant::now());
        let directory = state.directory.clone();
        let page = state.page;
        let summary = !state.expanded || state.tab != Tab::Files;
        let application = self.application.clone();
        let tx = self.tx.clone();
        state.file_task = Some(ReadTask(tokio::spawn(async move {
            let result = tokio::time::timeout(Duration::from_secs(15), async {
                let value = application
                    .client()
                    .workspace_git_status(&directory, page * 40, summary)
                    .await?;
                Ok::<_, anyhow::Error>(WorkResult::Files(serde_json::from_value(value)?))
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
        let directory = state.directory.clone();
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
        let old_path = if let Some(Detail::File(path, staged)) = &state.detail {
            state
                .files
                .as_ref()
                .and_then(|p| p.files.iter().find(|f| &f.path == path))
                .and_then(|f| {
                    if *staged {
                        f.index_old_path.clone()
                    } else {
                        f.working_old_path.clone()
                    }
                })
        } else {
            None
        };
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
                    Detail::File(path, staged) => {
                        let value = application
                            .client()
                            .workspace_git_diff(
                                &directory,
                                &path,
                                staged,
                                max_bytes,
                                old_path.as_deref(),
                            )
                            .await?;
                        Ok(WorkResult::Diff(
                            value
                                .get("diff")
                                .and_then(serde_json::Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            value
                                .get("truncated")
                                .and_then(serde_json::Value::as_bool)
                                .unwrap_or(false),
                        ))
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
        match channel {
            0 => {
                state.file_task = None;
                state.file_error = None;
            }
            1 => {
                state.detail_task = None;
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
            Ok(WorkResult::Logs(mut logs)) => {
                if let Some(old) = state
                    .logs
                    .take()
                    .filter(|old| old.activity_id == logs.activity_id)
                {
                    let cursor = old.lines.last().map_or(0, |l| l.seq);
                    let mut lines = old.lines;
                    lines.extend(logs.lines.into_iter().filter(|l| l.seq > cursor));
                    logs.lines = lines;
                }
                let excess = logs.lines.len().saturating_sub(200);
                logs.lines.drain(..excess);
                let mut budget = 128 * 1024;
                let mut keep = 0;
                for line in logs.lines.iter_mut().rev() {
                    if budget < 4 {
                        break;
                    }
                    if line.text.len() > budget {
                        let mut start = line.text.len() - budget + 3;
                        while !line.text.is_char_boundary(start) {
                            start += 1;
                        }
                        line.text = format!("…{}", &line.text[start..]);
                    }
                    budget = budget.saturating_sub(line.text.len());
                    keep += 1;
                }
                logs.lines.drain(..logs.lines.len() - keep);
                state.logs = Some(logs);
            }
            Ok(WorkResult::Diff(diff, truncated)) => {
                state.diff = diff;
                state.diff_truncated = truncated;
            }
            Ok(WorkResult::Control(activity)) => {
                if self.transcript.session_id == Some(id) {
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        if let Some(row) = execution
                            .background_activities
                            .iter_mut()
                            .find(|row| row.id == activity.id)
                        {
                            *row = activity;
                        }
                    }
                    self.request_refresh(id, false);
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
                r.map(WorkResult::Control)
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
