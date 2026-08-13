impl App {
    pub(crate) fn refresh_status_line_if_due(&mut self, now: Instant) {
        let Some(status_line) = self.status_line.as_mut() else {
            return;
        };
        let agena_tui::status_line::StatusLineEffect::Refresh { command } = status_line.tick(now)
        else {
            return;
        };
        let tx = self.tx.clone();
        let session_id = self.transcript.session_id.map(|id| id.to_string());
        let focus = self.focus.label().to_string();
        tokio::spawn(async move {
            let output = run_status_line_command(command, session_id, focus).await;
            let _ = tx.try_send(AppMessage::StatusLineUpdated { output });
        });
    }

    pub(crate) fn create_session(&mut self, submit_draft: Option<ComposerDraft>) {
        let pending_message_id = submit_draft
            .as_ref()
            .map(|draft| self.begin_pending_user_message(draft));
        if let Some(draft) = submit_draft.as_ref().cloned() {
            self.begin_run_operation(RunActivityTarget::NewSession, RunOperation::CreateSession);
            self.session_composer.pending_restore_draft = Some(draft.clone());
            self.set_draft_for_slot(self.current_draft_slot(), draft);
            self.persist_draft_store_with_feedback(true);
        }

        let title = submit_draft
            .as_ref()
            .and_then(draft_title_source)
            .map(|text| derive_session_title(&self.i18n, text.as_str()))
            .unwrap_or_else(|| ui_text::default_session_title(&self.i18n));

        let application = self.application.clone();
        let tx = self.tx.clone();
        // When this create-and-submit path is entered with no session open,
        // the run-options model stack holds the model the user switched to.
        // `open_session` clears it before the first submit reads it, so carry
        // it along and restore it in `handle_session_created`.
        let model_stack = submit_draft.as_ref().map(|_| self.run_options.clone());
        tokio::spawn(async move {
            let result = application.create_session(title, None).await
                .map_err(|error| crate::UiFailure::from_backend(anyhow::Error::new(error)));
            let _ = tx
                .send(AppMessage::SessionCreated {
                    submit_draft,
                    pending_message_id,
                    model_stack,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn is_local_command(&self, input: &str) -> bool {
        if commands::parse_command(input).is_some() {
            return true;
        }
        let Some((name, _)) = commands::parse_invocation(input) else {
            return false;
        };
        self.plugin_slash_commands()
            .iter()
            .any(|entry| plugin_command_matches_name(entry, name))
    }

    /// Primary submit action (Ctrl+Enter by default). When the AI is
    /// idle, submits the message immediately. When the AI is mid-run,
    /// interrupts the active run and delivers the message as the next
    /// user turn (interrupt-and-send), so the assistant's reply renders
    /// below the new message instead of being appended to the message
    /// above. When the session is busy with an interactive request
    /// (permission / user input) the run is not cancelled; the message
    /// is parked in the queue like bare Enter.
    pub(crate) fn submit_or_steer(&mut self) {
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            // Ctrl+Enter with nothing typed re-flushes a parked message
            // once the run is over; while the run is still active the
            // guard inside `try_send_pending` keeps it parked.
            self.try_send_pending();
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally regardless of AI state.
        let draft_text = draft.text();
        if self.is_local_command(draft_text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if !self.current_session_activity().is_busy() {
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
        // Only an actively generating run is interrupted. Interactive
        // phases (permission / user-input / blocked) must not be cancelled
        // because that would discard the pending request.
        let has_active_execution = self
            .transcript
            .execution
            .as_ref()
            .and_then(|execution| execution.active_execution.as_ref())
            .is_some();
        if self.session_activity(session_id).is_running() && has_active_execution {
            self.request_cancel_run(session_id);
            let replaced = self.queue.set(draft);
            self.flash_info(ui_text::t(
                &self.i18n,
                if replaced {
                    "flash-message-replaced"
                } else {
                    "flash-message-interrupting"
                },
            ));
            return;
        }
        let replaced = self.queue.set(draft);
        self.flash_info(ui_text::t(
            &self.i18n,
            if replaced {
                "flash-message-replaced"
            } else {
                "flash-message-queued"
            },
        ));
    }

    /// Secondary submit action (bare Enter by default). When the AI is idle,
    /// sends immediately. When the AI is mid-run, the message is parked in the
    /// single pending slot and delivered on run completion.
    pub(crate) fn queue_or_submit(&mut self) {
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            // A bare Enter with nothing typed re-flushes a parked message
            // when the run has already ended but the terminal event hasn't
            // drained it yet. While the run is still active the guard inside
            // `try_send_pending` keeps the message parked.
            self.try_send_pending();
            return;
        }
        self.reset_prompt_history_recall();
        // Slash-commands always run locally — never queue.
        let draft_text = draft.text();
        if self.is_local_command(draft_text.as_str()) {
            self.restore_composer_draft(draft);
            self.submit_composer();
            return;
        }
        if self.current_session_activity().is_busy() {
            let replaced = self.queue.set(draft);
            self.flash_info(ui_text::t(
                &self.i18n,
                if replaced {
                    "flash-message-replaced"
                } else {
                    "flash-message-queued"
                },
            ));
            return;
        }
        self.restore_composer_draft(draft);
        self.submit_composer();
    }

    pub(crate) fn submit_composer(&mut self) {
        let draft = self.take_composer_draft();
        if draft.is_empty() || self.current_session_activity().is_busy() {
            self.restore_composer_draft(draft);
            return;
        }
        self.reset_prompt_history_recall();

        let draft_text = draft.text();
        if let Some(parsed) = commands::parse_command(draft_text.as_str()) {
            if draft.activities().next().is_some() {
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

        if let Some((name, args)) = commands::parse_invocation(draft_text.as_str()) {
            if draft.activities().next().is_some() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            if let Some(entry) = self
                .plugin_slash_commands()
                .into_iter()
                .find(|entry| plugin_command_matches_name(entry, name))
            {
                self.execute_plugin_slash_command(entry, args);
                return;
            }
        }

        let draft = if draft_text.starts_with("//") {
            composer_draft_with_text_prefix_stripped(draft, 1)
        } else {
            draft
        };

        let target_session_id = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id());

        match target_session_id {
            Some(session_id) => self.request_submit_message_with_pending(session_id, draft, None),
            None => self.create_session(Some(draft)),
        }
    }

    pub(crate) fn continue_current_session(&mut self) {
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

    pub(crate) fn compact_current_session(&mut self) {
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

    pub(crate) fn reply_permission(&mut self, kind: PermissionReplyKind) {
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

    pub(crate) fn submit_permission_reply(
        &mut self,
        session_id: i64,
        request: PermissionRequest,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.seen_permission_request_ids
            .insert(request.request_id.clone());
        self.request_permission_reply(session_id, request.request_id, kind, scope, label);
    }

    pub(crate) fn sync_pending_interactive_after_execution(&mut self, _session_id: i64) {
        self.maybe_auto_open_pending_interactive_overlay();
        // A request that arrived before its part existed (execution snapshot
        // landed first) auto-reveals here, once `apply_transcript_execution`
        // has populated the transcript parts.
        self.reveal_outstanding_pending_user_input_interactions();
    }
}
use crate::{
    App, AppMessage, ComposerDraft, Instant, PermissionReplyKind, PermissionRequest,
    PermissionScope, RunActivityTarget, RunOperation, commands,
    composer_draft_with_text_prefix_stripped, derive_session_title, draft_title_source,
    plugin_command_matches_name, run_status_line_command, ui_text,
};
