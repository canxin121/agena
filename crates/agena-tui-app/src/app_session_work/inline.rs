use super::*;
use std::collections::BTreeSet;

#[derive(Debug)]
pub(super) struct InlineLogState {
    activity: BackgroundActivityResource,
    logs: Option<BackgroundActivityLogResource>,
    task: Option<ReadTask>,
    request: u64,
    checked_at: Option<Instant>,
    allowed_at: Instant,
    failures: u32,
    error: Option<String>,
    pub(super) dirty: bool,
}

impl InlineLogState {
    fn new(activity: BackgroundActivityResource) -> Self {
        Self { activity, logs: None, task: None, request: 0, checked_at: None,
            allowed_at: Instant::now(), failures: 0, error: None, dirty: true }
    }

    fn text(&self, i18n: &agena_tui::i18n::I18n) -> String {
        let mut text = format!("{} · {}", i18n.text("session-work-log-tail"),
            i18n.text(&format!("session-work-status-{}", self.activity.status)));
        if let Some(exit) = self.activity.exit_code { text.push_str(&format!(" · exit {exit}")); }
        if let Some(error) = &self.error { text.push_str(&format!("\n{error}")); }
        let mut output = String::new();
        if let Some(logs) = &self.logs {
            for line in &logs.lines {
                if !line.chunk && !output.is_empty() && !output.ends_with('\n') { output.push('\n'); }
                output.push_str(&line.text);
                if !line.chunk { output.push('\n'); }
            }
        }
        let mut start = output.len().saturating_sub(8 * 1024);
        while !output.is_char_boundary(start) { start += 1; }
        text.push('\n');
        if start > 0 { text.push('…'); }
        text.push_str(&crate::sanitize_terminal_text(&output[start..]));
        text
    }
}

impl App {
    /// Read independent activity logs only for expanded launch receipts in
    /// the viewport. Completed durable tool output keeps its original meaning.
    pub(crate) fn heal_inline_activity_logs(&mut self) {
        let selected = self.transcript.session_id;
        let main = self.current_route_is_main();
        let has_viewport = main && self.layout.transcript_body.height > 0;
        for (id, state) in &mut self.session_work {
            if !has_viewport || Some(*id) != selected { state.inline_logs.clear(); }
        }
        if !has_viewport { return; }
        let Some(id) = selected else { return; };
        let width = self.layout.transcript_body.width.max(1);
        let top = self.transcript.viewport_top();
        let end = top.saturating_add(self.layout.transcript_body.height as usize);
        let nodes = &self.transcript.rendered(width).nodes;
        let expanded = nodes.iter().filter(|node| node.expanded).filter_map(|node| match node.key {
            agena_tui_transcript::TranscriptNodeKey::Activity {
                content_id: agena_tui_transcript::TranscriptContentId::StoredPart(part), ..
            } => Some(part),
            _ => None,
        }).collect::<BTreeSet<_>>();
        let visible = nodes.iter().filter(|node| node.expanded && node.start_line < end && node.end_line > top)
            .filter_map(|node| match node.key {
                agena_tui_transcript::TranscriptNodeKey::Activity {
                    content_id: agena_tui_transcript::TranscriptContentId::StoredPart(part), ..
                } => Some(part),
                _ => None,
            }).collect::<BTreeSet<_>>();
        let activities = self.transcript.execution.as_ref().map(|execution|
            execution.background_activities.iter().filter(|activity|
                activity.source_part_id.is_some_and(|part| visible.contains(&part)))
                .take(16).cloned().collect::<Vec<_>>()).unwrap_or_default();
        let sources = self.transcript.execution.as_ref().map(|execution|
            execution.background_activities.iter().filter_map(|activity| activity.source_part_id)
                .collect::<BTreeSet<_>>()).unwrap_or_default();
        let displayed = activities.iter().filter_map(|activity| activity.source_part_id).collect::<BTreeSet<_>>();
        let active = activities.iter().map(|activity| activity.id.clone()).collect::<BTreeSet<_>>();
        let state = self.session_work.entry(id).or_default();
        state.inline_logs.retain(|activity, _| active.contains(activity));
        let mut display = Vec::new();
        for activity in activities {
            let part = activity.source_part_id.expect("selected launch receipt");
            let entry = state.inline_logs.entry(activity.id.clone())
                .or_insert_with(|| InlineLogState::new(activity.clone()));
            let changed = entry.activity != activity;
            if entry.activity.last_seq != activity.last_seq || entry.activity.status != activity.status
                || entry.activity.dropped_lines != activity.dropped_lines {
                entry.dirty = true;
            }
            entry.activity = activity.clone();
            if changed || !self.transcript.background_output.contains_key(&part) {
                display.push((part, entry.text(&self.i18n)));
            }
            let now = Instant::now();
            if entry.task.is_some() || entry.allowed_at > now { continue; }
            if !entry.dirty && !entry.logs.as_ref().is_some_and(|logs| logs.has_more)
                && entry.checked_at.is_some_and(|at| now.duration_since(at) < Duration::from_secs(30)) { continue; }
            self.next_usage_request_id += 1;
            let request_id = self.next_usage_request_id;
            entry.request = request_id;
            entry.dirty = false;
            let cursor = entry.logs.as_ref().map_or(activity.last_seq.saturating_sub(200), |logs| logs.last_seq);
            let application = self.application.clone();
            let tx = self.tx.clone();
            entry.task = Some(ReadTask(tokio::spawn(async move {
                let result = tokio::time::timeout(Duration::from_secs(15),
                    crate::app_backend::activities::activity_logs(&application, &activity.id, cursor, Some(200), 0))
                    .await.map_err(crate::UiFailure::internal)
                    .and_then(|result| result.map(WorkResult::Logs).map_err(crate::UiFailure::internal));
                let _ = tx.send(AppMessage::SessionWorkLoaded {
                    session_id: id, request_id, channel: 3, result,
                }).await;
            })));
        }
        let before = self.transcript.background_output.len();
        self.transcript.background_output.retain(|part, _| expanded.contains(part) && sources.contains(part));
        let mut changed = before != self.transcript.background_output.len();
        for (part, text) in display {
            if self.transcript.background_output.get(&part) != Some(&text) {
                self.transcript.background_output.insert(part, text);
                changed = true;
            }
        }
        while self.transcript.background_output.len() > 64 {
            // Protect the active readers, even when their part ids are older
            // than cached offscreen output. Evicting them creates a render /
            // recreate loop on every tick without any new activity data.
            let victim = self.transcript.background_output.keys()
                .find(|part| !visible.contains(*part))
                .or_else(|| self.transcript.background_output.keys().find(|part| !displayed.contains(*part)))
                .copied();
            let Some(victim) = victim else { break; };
            self.transcript.background_output.remove(&victim);
            changed = true;
        }
        if changed { self.transcript.invalidate_render(); }
    }

    pub(super) fn handle_inline_activity_logs_loaded(&mut self, id: i64, request: u64, result: crate::UiResult<WorkResult>) {
        if self.transcript.session_id != Some(id) { return; }
        let Some(state) = self.session_work.get_mut(&id) else { return; };
        let Some(entry) = state.inline_logs.values_mut().find(|entry| entry.request == request && request != 0) else { return; };
        entry.task = None;
        entry.request = 0;
        entry.checked_at = Some(Instant::now());
        match result {
            Ok(WorkResult::Logs(logs)) if logs.activity_id == entry.activity.id => {
                entry.logs = Some(merge_log_tail(entry.logs.take(), logs));
                entry.failures = 0;
                entry.error = None;
                entry.allowed_at = Instant::now() + Duration::from_millis(750);
            }
            Err(error) => {
                entry.failures = entry.failures.saturating_add(1).min(5);
                entry.error = Some(crate::sanitize_terminal_text(&error.to_string()));
                entry.dirty = true;
                entry.allowed_at = Instant::now() + Duration::from_millis((5000 * 2_u64.pow(entry.failures - 1)).min(60_000));
            }
            _ => { entry.dirty = true; return; }
        }
        let Some(part) = entry.activity.source_part_id else { return; };
        let text = entry.text(&self.i18n);
        if self.transcript.background_output.get(&part) != Some(&text) {
            self.transcript.background_output.insert(part, text);
            self.transcript.invalidate_render();
        }
    }
}
