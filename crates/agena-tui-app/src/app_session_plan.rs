//! Session-owned plan progress read from the same resource as the Web client.
//! Plugin display contributions are an in-memory projection and can be stale
//! or absent after a reload; they are not the source of plan state.

use std::time::{Duration, Instant};

use crate::{App, AppMessage, UiResult, app_plan_document::plan_progress};

#[derive(Debug, Default)]
pub(crate) struct SessionPlanState {
    session_id: Option<i64>,
    pub(crate) progress: Option<String>,
    request_id: Option<u64>,
    task: Option<tokio::task::JoinHandle<()>>,
    dirty: bool,
    failures: u8,
    retry_at: Option<Instant>,
}

impl Drop for SessionPlanState {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

impl SessionPlanState {
    fn begin_read(&mut self, session_id: i64, request_id: u64) -> bool {
        if self.session_id != Some(session_id) {
            *self = Self::default();
            self.session_id = Some(session_id);
        }
        if self.request_id.is_some()
            || self
                .retry_at
                .is_some_and(|deadline| Instant::now() < deadline)
        {
            self.dirty = true;
            return false;
        }
        self.request_id = Some(request_id);
        self.dirty = false;
        self.retry_at = None;
        true
    }

    /// Returns whether an invalidation received during the read needs another
    /// read. A completion from a previous session or request has no effect.
    fn complete_read(
        &mut self,
        session_id: i64,
        request_id: u64,
        result: UiResult<Option<String>>,
    ) -> bool {
        if self.session_id != Some(session_id) || self.request_id != Some(request_id) {
            return false;
        }
        self.request_id = None;
        self.task = None;
        match result {
            Ok(progress) => {
                self.progress = progress;
                self.failures = 0;
                std::mem::take(&mut self.dirty)
            }
            Err(error) => {
                tracing::warn!(session_id, %error, "failed to refresh TUI plan progress");
                self.failures = self.failures.saturating_add(1).min(5);
                self.retry_at = Some(
                    Instant::now() + Duration::from_secs((5_u64 << (self.failures - 1)).min(60)),
                );
                false
            }
        }
    }
}

impl App {
    pub(crate) fn request_plan_display_refresh(&mut self, session_id: i64) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        let request_id = self.next_usage_request_id.saturating_add(1);
        if !self.session_plan.begin_read(session_id, request_id) {
            return;
        }
        self.next_usage_request_id = request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        self.session_plan.task = Some(tokio::spawn(async move {
            let result = application
                .invoke_plugin_tool(
                    "agena.plan",
                    "get",
                    serde_json::json!({ "view": "summary" }),
                    Some(session_id),
                )
                .await
                .map(|response| plan_progress(response.payload.as_ref()))
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::SessionPlanLoaded {
                    session_id,
                    request_id,
                    result,
                })
                .await;
        }));
    }

    pub(crate) fn handle_session_plan_loaded(
        &mut self,
        session_id: i64,
        request_id: u64,
        result: UiResult<Option<String>>,
    ) {
        if self.transcript.session_id != Some(session_id)
            || self.session_plan.session_id != Some(session_id)
            || self.session_plan.request_id != Some(request_id)
        {
            return;
        }
        let updated = result.is_ok();
        let refresh_again = self
            .session_plan
            .complete_read(session_id, request_id, result);
        if updated
            && let crate::Route::PlanViewer(state) = &mut self.current_route
            && state.session_id == session_id
        {
            state.summary = self.session_plan.progress.clone();
        }
        if refresh_again {
            self.request_plan_display_refresh(session_id);
        }
    }

    pub(crate) fn retry_session_plan_if_due(&mut self, now: Instant) {
        if self
            .session_plan
            .retry_at
            .is_some_and(|deadline| now >= deadline)
            && let Some(session_id) = self.transcript.session_id
        {
            self.request_plan_display_refresh(session_id);
        }
    }

    pub(crate) fn session_plan_progress(&self, session_id: i64) -> Option<&str> {
        (self.session_plan.session_id == Some(session_id))
            .then_some(self.session_plan.progress.as_deref())
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalidations_during_a_read_are_coalesced_and_replayed() {
        let mut state = SessionPlanState::default();
        assert!(state.begin_read(1, 1));
        assert!(!state.begin_read(1, 2));
        assert!(!state.begin_read(1, 3));
        assert!(state.complete_read(1, 1, Ok(Some("▶ 1/3".into()))));
        assert!(state.begin_read(1, 4));
        assert!(!state.complete_read(1, 4, Ok(Some("✓ 3/3".into()))));
        assert_eq!(state.progress.as_deref(), Some("✓ 3/3"));
    }

    #[test]
    fn switching_sessions_discards_progress_and_ignores_late_responses() {
        let mut state = SessionPlanState::default();
        assert!(state.begin_read(1, 1));
        assert!(!state.complete_read(1, 1, Ok(Some("▶ 1/3".into()))));
        assert!(state.begin_read(1, 2));
        assert!(state.begin_read(2, 3));
        assert!(state.progress.is_none());
        assert!(!state.complete_read(1, 2, Ok(Some("✓ 3/3".into()))));
        assert_eq!(state.request_id, Some(3));
        assert!(!state.complete_read(2, 3, Ok(Some("⏳ 0/1".into()))));
        assert!(state.begin_read(1, 4));
        assert!(!state.complete_read(1, 2, Ok(Some("✓ 3/3".into()))));
        assert_eq!(state.request_id, Some(4));
        assert!(state.progress.is_none());
    }

    #[test]
    fn clearing_a_plan_removes_the_chip() {
        let mut state = SessionPlanState::default();
        assert!(state.begin_read(1, 1));
        state.complete_read(1, 1, Ok(Some("▶ 1/3".into())));
        assert!(state.begin_read(1, 2));
        state.complete_read(1, 2, Ok(None));
        assert!(state.progress.is_none());
    }

    #[test]
    fn failed_reads_keep_progress_and_retry_with_backoff() {
        let mut state = SessionPlanState::default();
        assert!(state.begin_read(1, 1));
        state.complete_read(1, 1, Ok(Some("▶ 1/3".into())));
        assert!(state.begin_read(1, 2));
        assert!(!state.complete_read(
            1,
            2,
            Err(crate::UiFailure::internal(anyhow::anyhow!("offline"))),
        ));
        assert_eq!(state.progress.as_deref(), Some("▶ 1/3"));
        assert!(!state.begin_read(1, 3));
        state.retry_at = Some(Instant::now());
        assert!(state.begin_read(1, 4));
        state.complete_read(1, 4, Ok(Some("✓ 3/3".into())));
        assert_eq!(state.failures, 0);
        assert!(state.retry_at.is_none());
    }
}
