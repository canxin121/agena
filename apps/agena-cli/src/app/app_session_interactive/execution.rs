impl App {
    pub(in crate::app) fn refresh_status_line_if_due(&mut self, now: Instant) {
        let Some(status_line) = self.status_line.as_mut() else {
            return;
        };
        if status_line.running || now < status_line.next_refresh_at {
            return;
        }

        status_line.running = true;
        status_line.next_refresh_at = now + status_line.refresh_interval;
        let command = status_line.command.clone();
        let tx = self.tx.clone();
        let session_id = self.transcript.session_id.map(|id| id.to_string());
        let focus = self.focus.label().to_string();
        tokio::task::spawn_blocking(move || {
            let output = run_status_line_command(command, session_id, focus);
            let _ = tx.send(AppMessage::StatusLineUpdated { output });
        });
    }

    pub(in crate::app) fn create_session(&mut self, submit_draft: Option<ComposerDraft>) {
        self.create_session_with_parent(submit_draft, None);
    }

    pub(in crate::app) fn create_session_with_parent(
        &mut self,
        submit_draft: Option<ComposerDraft>,
        parent_id: Option<i64>,
    ) {
        let pending_message_id = submit_draft
            .as_ref()
            .map(|draft| self.begin_pending_user_message(draft));
        if let Some(draft) = submit_draft.as_ref().cloned() {
            self.transcript.submitting = true;
            self.transcript.pending_restore_draft = Some(draft.clone());
            self.set_draft_for_slot(self.current_draft_slot(), draft);
            self.persist_draft_store_with_feedback(true);
        }

        let title = submit_draft
            .as_ref()
            .and_then(draft_title_source)
            .map(|text| derive_session_title(&self.i18n, text.as_str()))
            .unwrap_or_else(|| ui_text::default_session_title(&self.i18n));

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .create_session(title, parent_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionCreated {
                submit_draft,
                pending_message_id,
                result,
            });
        });
    }

    pub(in crate::app) fn is_local_command(&self, input: &str) -> bool {
        if commands::parse_command(input).is_some() {
            return true;
        }
        let Some((name, _)) = commands::parse_invocation(input) else {
            return false;
        };
        self.backend.runtime_tool_exists(name)
    }

    /// Primary submit action (Ctrl+Enter by default). When the AI is
    /// idle, submits the message immediately. When the AI is mid-run, attempts to
    /// `steer_input` (Phase 3) — i.e. inject the message into the live
    /// run so the model sees it on its next step. If the backend rejects
    /// the steer (e.g. the run is in a non-steerable phase), we fall
    /// back to enqueueing the message so it isn't lost.
    pub(in crate::app) fn submit_or_steer(&mut self) {
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally regardless of AI state.
        if self.is_local_command(draft.text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if !self.transcript.submitting {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        let Some(session_id) = self.transcript.session_id else {
            // No active session — fall back to normal submit which will
            // create one.
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        };
        let parts = match self.build_submission_parts(&draft) {
            Ok(parts) => parts,
            Err(error) => {
                self.restore_composer_draft(draft);
                self.flash_error(error);
                return;
            }
        };
        self.request_steer_input(session_id, parts, draft);
    }

    /// Secondary submit action (bare Enter by default). When the AI is idle,
    /// sends immediately. When the AI is mid-run, the message is appended to
    /// the local pending queue and drained on run completion.
    pub(in crate::app) fn queue_or_submit(&mut self) {
        // During a multi-character paste burst, an Enter inside it should be
        // treated as a literal newline rather than a submit/queue.
        if self.composer.should_insert_newline_on_enter() {
            self.composer.insert_newline_from_enter();
            return;
        }
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally — never queue.
        if self.is_local_command(draft.text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if self.transcript.submitting {
            self.queue.enqueue(draft);
            self.flash_info(ui_text::t(&self.i18n, "flash-message-queued"));
            return;
        }
        self.restore_composer_draft(draft);
        self.submit_composer();
    }

    pub(in crate::app) fn submit_composer(&mut self) {
        self.composer.flush_all_pending_input();
        let draft = self.take_composer_draft();
        if draft.is_empty() || self.transcript.submitting {
            self.restore_composer_draft(draft);
            return;
        }
        self.reset_prompt_history_recall();

        if let Some(parsed) = commands::parse_command(draft.text.as_str()) {
            if !draft.items.is_empty() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            self.execute_command(parsed.spec, parsed.args.as_str());
            return;
        }

        if let Some((name, args)) = commands::parse_invocation(draft.text.as_str()) {
            if !draft.items.is_empty() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            if self.backend.runtime_tool_exists(name) {
                self.execute_runtime_tool_prompt(name, args);
                return;
            }
        }

        let draft = if draft.text.starts_with("//") {
            composer_draft_with_text_prefix_stripped(draft, 1)
        } else {
            draft
        };

        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());

        match target_session_id {
            Some(session_id) => self.request_submit_message(session_id, draft),
            None => self.create_session(Some(draft)),
        }
    }

    pub(in crate::app) fn continue_current_session(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.request_continue(session_id);
    }

    pub(in crate::app) fn compact_current_session(&mut self) {
        let Some(session_id) = self.transcript.session_id else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        };
        if self.prompt_for_pending_interactive_on_session(session_id) {
            return;
        }
        if self.session_is_busy(session_id) {
            self.flash_warning(ui_text::t(&self.i18n, "flash-session-busy"));
            return;
        }
        self.request_compact(session_id);
    }

    pub(in crate::app) fn reply_permission(&mut self, kind: PermissionReplyKind) {
        let Some((session_id, request)) = self.pending_permission_overlay_target() else {
            self.flash_warning(ui_text::t(&self.i18n, "flash-no-permission-request"));
            return;
        };
        self.submit_permission_reply(
            session_id,
            request,
            kind,
            None,
            ui_text::permission_reply_label(&self.i18n, kind),
        );
    }

    pub(in crate::app) fn submit_permission_reply(
        &mut self,
        session_id: i64,
        request: PermissionRequest,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.pending_permission_replay = Some(PermissionReplayState {
            session_id,
            fingerprint: permission_request_fingerprint(&request),
            last_request_id: request.request_id.clone(),
            kind,
            scope,
            label: label.clone(),
        });
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.request_permission_reply(session_id, request.request_id, kind, scope, label);
    }

    pub(in crate::app) fn maybe_auto_reply_duplicate_permission_request(
        &mut self,
        session_id: i64,
    ) -> bool {
        let Some(replay) = self.pending_permission_replay.clone() else {
            return false;
        };
        if replay.session_id != session_id || self.transcript.session_id != Some(session_id) {
            self.pending_permission_replay = None;
            return false;
        }

        let Some((pending_session_id, request)) = self.pending_permission_overlay_target() else {
            self.pending_permission_replay = None;
            return false;
        };
        if pending_session_id != session_id {
            self.pending_permission_replay = None;
            return false;
        }

        if permission_request_fingerprint(&request) != replay.fingerprint {
            self.pending_permission_replay = None;
            return false;
        }

        if request.request_id == replay.last_request_id {
            return false;
        }

        self.pending_permission_replay = Some(PermissionReplayState {
            last_request_id: request.request_id.clone(),
            ..replay.clone()
        });
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.overlay = None;
        self.request_permission_reply(
            session_id,
            request.request_id,
            replay.kind,
            replay.scope,
            replay.label,
        );
        true
    }

    pub(in crate::app) fn sync_pending_interactive_after_execution(&mut self, session_id: i64) {
        if !self.maybe_auto_reply_duplicate_permission_request(session_id) {
            self.maybe_auto_open_pending_interactive_overlay();
        }
    }
}
use crate::app::{
    App, AppMessage, ComposerDraft, Instant, PermissionReplayState, PermissionReplyKind,
    PermissionRequest, PermissionScope, commands, composer_draft_with_text_prefix_stripped,
    derive_session_title, draft_title_source, permission_request_fingerprint,
    run_status_line_command, ui_text,
};
