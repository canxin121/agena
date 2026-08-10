impl App {
    pub(crate) fn open_session(&mut self, session_id: i64, title: String) {
        // Opening any other session while a side conversation is active
        // discards the ephemeral side (codex's
        // `side_thread_to_discard_after_switch` rule). Switching into the side
        // itself, or staying put, keeps it.
        if let Some(side_session_id) = self.side_session_to_discard_after_switch(session_id) {
            self.side_sessions.remove(&side_session_id);
        }
        self.sync_current_draft_slot();
        self.clear_composer_state();
        self.current_lineage = None;
        self.session_controller.current_session_id = Some(session_id);
        self.session_controller.active = false;
        self.session_controller.sequence = None;
        self.focus = Focus::Transcript;
        self.transcript.reset(session_id, title);
        // Re-publish the plan display contribution for the opened session so
        // the composer's bottom-right chip appears immediately even when the
        // in-memory contribution was lost (process restart, runtime reload, or
        // a plan created while another session was visible).
        self.request_plan_display_refresh(session_id);
        // A refresh requested for the previous session must not be re-issued
        // against the newly opened one.
        self.pending_refresh = None;
        self.refresh_pending = false;
        // A model selection belongs to the session that produced it. Clear the
        // previous session's stack until this session's persisted execution
        // context arrives.
        self.run_options.clear_model_stack();
        self.seen_permission_request_ids.clear();
        self.seen_user_input_request_ids.clear();
        let _ = self.sessions.select_by_id(session_id);
        self.restore_draft_for_slot(DraftSlot::Session(session_id));
        self.persist_draft_store_with_feedback(true);
        self.subscribe_session_events(session_id);
        self.request_lineage(session_id);
        self.request_session_state(session_id);
        if self.sessions.view_mode() == SessionViewMode::Subtree {
            self.request_sessions(false);
        }
    }

    /// Spawn a forwarder task that pumps live `LiveEvent`s from the unified
    /// bus into [`AppMessage::SessionEventArrived`]. Aborts any previous
    /// subscription so we never accumulate stale receivers.
    pub(crate) fn subscribe_session_events(&mut self, session_id: i64) {
        if let Some(handle) = self.active_subscription.take() {
            handle.abort();
        }
        let Some(mut rx) = self.backend.subscribe_session_events(session_id) else {
            return;
        };
        let tx = self.tx.clone();
        let handle = tokio::spawn(async move {
            while let Some(live) = rx.recv().await {
                if tx
                    .send(AppMessage::SessionEventArrived { session_id, live })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.active_subscription = Some(handle);
    }

    pub(crate) fn apply_transcript_execution(
        &mut self,
        execution: SessionExecutionResource,
    ) -> bool {
        if self.transcript.execution.as_ref().is_some_and(|current| {
            current.session.id == execution.session.id
                && current.session.version > execution.session.version
        }) {
            return false;
        }
        let is_terminal = execution.active_execution.is_none();
        let current_execution_absent = self.transcript.execution.is_none();
        if execution_update_is_stale_with_terminal(
            self.transcript.last_event_seq,
            execution.latest_event_seq,
            is_terminal,
            current_execution_absent,
        ) {
            return false;
        }
        let was_running = self
            .transcript
            .execution
            .as_ref()
            .is_some_and(|current| current.active_execution.is_some());
        let session_id = execution.session.id;
        let sequence = execution.latest_event_seq;
        self.transcript.apply_execution(execution);
        if is_terminal {
            // A terminal execution means no run is in flight for this
            // session. Clear any local run activity (submit/continue/
            // compact/rewind) whose response was lost, so the top-right
            // activity indicator agrees with the terminal title and progress
            // bar. Repeated applications are idempotent.
            self.run_activity.clear_session(session_id);
            // Safety net: a terminal execution must never leave a
            // non-terminal reply in the transcript. If the durable snapshot
            // did not converge (for example the refresh that carried it was
            // the last one before a lost response), force one refresh so the
            // reply status and inline body rendering catch up. Guarded by
            // `was_running` so repeated terminal refreshes cannot loop.
            if was_running && self.transcript.has_non_terminal_replies() {
                self.request_refresh(session_id, true);
            }
        }
        self.session_controller
            .apply(&agena_tui_session::SessionEvent::SessionRefreshed {
                session_id,
                sequence,
            });
        self.sync_run_model_stack_from_execution();
        self.sync_seen_pending_request_ids();
        self.sync_open_pending_interactive_overlay();
        // A run transitioned to terminal on the current session while we are
        // not already waiting on an interactive request. Raise terminal
        // attention so the completion is noticed even when the user is in
        // another tab. Re-application of an already-terminal state (for
        // example a refresh) is ignored.
        if is_terminal && was_running && !self.current_session_pending_interactive_kind().is_some()
        {
            self.raise_completion_notification();
        }
        true
    }

    /// Queues a "reply complete" terminal notification for the next frame.
    pub(crate) fn raise_completion_notification(&mut self) {
        use agena_tui_platform::terminal::integration::NotificationMethod;
        let method = crate::current_notification_method(self).unwrap_or(NotificationMethod::Bell);
        self.terminal_integration.queue_notification(method);
        // The title snapshot is stale until the next frame recomputes it from
        // the now-idle activity; mark it pending so the run loop refreshes it.
        self.terminal_integration.mark_title_pending();
    }

    pub(crate) fn sync_run_model_stack_from_execution(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.run_options.clear_model_stack();
            return;
        };
        let model = execution
            .execution
            .model_provider_id
            .as_deref()
            .zip(execution.execution.model_id.as_deref())
            .map(|(provider_id, model_id)| {
                execution
                    .execution
                    .model_adapter_id
                    .as_deref()
                    .map(|adapter_id| ModelRef::new_with_adapter(provider_id, adapter_id, model_id))
                    .unwrap_or_else(|| ModelRef::new(provider_id, model_id))
            });
        self.run_options.replace_model_stack(
            model,
            execution.execution.model_thinking_mode.clone(),
            execution.execution.model_speed_mode.clone(),
            execution.execution.model_verbosity.clone(),
            execution.execution.model_parallel_tool_calls,
        );
    }

    pub(crate) fn sync_open_pending_interactive_overlay(&mut self) {
        let keep_overlay = match self.overlay.as_ref() {
            Some(Overlay::Permission(dialog)) => permission_overlay_matches_pending_request(
                dialog,
                self.transcript.session_id,
                self.transcript.execution.as_ref(),
            ),
            Some(Overlay::UserInputReply(dialog)) => user_input_overlay_matches_pending_request(
                dialog,
                self.transcript.session_id,
                self.transcript.execution.as_ref(),
            ),
            _ => true,
        };

        if !keep_overlay {
            self.overlay = None;
        }
    }

    pub(crate) fn sync_seen_pending_request_ids(&mut self) {
        let Some(execution) = self.transcript.execution.as_ref() else {
            self.seen_permission_request_ids.clear();
            self.seen_user_input_request_ids.clear();
            return;
        };
        self.seen_permission_request_ids.retain(|request_id| {
            execution
                .pending_interactive_requests
                .iter()
                .any(|request| {
                    pending_interactive_request_matches_kind(
                        request,
                        PendingInteractiveKind::Permission,
                    ) && pending_interactive_request_id(request) == request_id
                })
        });
        self.seen_user_input_request_ids.retain(|request_id| {
            execution
                .pending_interactive_requests
                .iter()
                .any(|request| {
                    pending_interactive_request_matches_kind(
                        request,
                        PendingInteractiveKind::UserInput,
                    ) && pending_interactive_request_id(request) == request_id
                })
        });
    }

    pub(crate) fn handle_session_event_arrived(&mut self, session_id: i64, live: LiveEvent) {
        // Ignore events for sessions the user has already navigated away
        // from. The forwarder is normally aborted in that case but a few
        // in-flight messages may still land.
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        let refresh_needed_from_event = live.event.as_ref().is_some_and(|event| {
            self.transcript.apply_presentation_event(
                event,
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            )
        });
        if live.force_refresh || live.triggers_refresh || refresh_needed_from_event {
            // Park the refresh into the periodic tick budget instead of
            // flushing immediately: streaming flushes emit `PartUpdated` far
            // faster than the TUI can apply full-snapshot refreshes, and
            // spawning one per event leaves the transcript permanently behind
            // a running reply. `on_tick` repaints at most once per
            // `REFRESH_INTERVAL_MS`.
            self.pending_refresh_for(session_id, live.force_refresh);
        }
    }
}
use crate::{
    App, AppMessage, DraftSlot, LiveEvent, ModelRef, Overlay, PendingInteractiveKind,
    SessionExecutionResource, execution_update_is_stale_with_terminal,
    pending_interactive_request_id, pending_interactive_request_matches_kind,
    permission_overlay_matches_pending_request, user_input_overlay_matches_pending_request,
};
use agena_tui::main_focus::Focus;
use agena_tui_session::session_view::SessionViewMode;
