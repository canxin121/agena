impl App {
    pub(crate) fn composer_media_finished(&mut self, successful: bool) {
        if !successful {
            if self.session_composer.pending_submit.take().is_some() {
                self.flash_warning(ui_text::t(&self.i18n, "flash-attachment-prepare-failed"));
            }
            return;
        }
        if self.pending_composer_media_count() > 0 {
            return;
        }
        if self.session_composer.pending_submit.take().is_some() {
            // Every attachment part is ready, so the whole document goes out
            // as one request now.
            self.submit_or_steer();
        }
    }

    fn defer_composer_submit(&mut self, mode: PendingComposerSubmit) {
        self.session_composer.pending_submit = Some(mode);
        self.flash_info(ui_text::t(&self.i18n, "flash-attachment-preparing"));
    }

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
            if let Err(error) = tx.try_send(AppMessage::StatusLineUpdated { output }) {
                tracing::debug!(
                    diagnostic = %error,
                    "TUI status-line update was superseded because the UI queue was unavailable"
                );
            }
        });
    }

    pub(crate) fn create_session(&mut self, submit_draft: Option<ComposerDraft>) {
        if let Some(draft) = submit_draft.as_ref().cloned() {
            self.begin_run_operation(RunActivityTarget::NewSession, RunOperation::CreateSession);
            self.session_composer.pending_restore_draft = Some(draft.clone());
            self.set_draft_for_slot(self.current_draft_slot(), draft);
            self.persist_draft_store_with_feedback(true);
        }

        let title = submit_draft
            .as_ref()
            .and_then(|draft| draft_title_source(&self.i18n, draft))
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
            let result = application
                .create_session(title, None)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionCreated {
                    submit_draft,
                    model_stack,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn is_local_command(&self, input: &str) -> bool {
        let Some((name, _)) = commands::parse_invocation(input) else {
            return false;
        };
        if self.find_client_command(name).is_some() {
            return true;
        }
        self.plugin_slash_commands()
            .iter()
            .any(|entry| plugin_command_matches_name(entry, name))
    }

    /// Submit the entire document immediately. The server persists and queues
    /// it behind existing work without interrupting the active execution.
    pub(crate) fn submit_or_steer(&mut self) {
        self.dispatch_composer_send();
    }

    /// Both composer submit keys use server-owned admission.
    pub(crate) fn queue_or_submit(&mut self) {
        self.dispatch_composer_send();
    }

    fn dispatch_composer_send(&mut self) {
        if self.pending_composer_media_count() > 0 {
            // Attachments that are still preparing are part of this one send,
            // so wait for them instead of submitting a document without its
            // parts.
            self.defer_composer_submit(PendingComposerSubmit::Send);
            return;
        }
        let draft = self.take_composer_draft();
        if draft.is_empty() {
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
        self.send_captured_draft(draft);
    }

    fn send_captured_draft(&mut self, draft: ComposerDraft) {
        let Some(session_id) = self
            .transcript
            .session_id
            .or_else(|| self.sessions.current_selected_id())
        else {
            self.create_session(Some(draft));
            return;
        };
        // Only an outstanding HTTP submission owns the draft restoration slot.
        // Once acknowledged, another message can be admitted while AI is busy.
        if self.run_activity.has_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::SubmitMessage,
        ) {
            self.restore_composer_draft(draft);
            self.flash_warning(ui_text::t(&self.i18n, "flash-send-in-flight"));
            return;
        }
        self.submit_user_draft(draft);
    }

    /// Command entry point: execute a local slash command when the draft is
    /// one, otherwise deliver the captured document through the same
    /// single-send path the composer keys use.
    pub(crate) fn submit_composer(&mut self) {
        if self.pending_composer_media_count() > 0 {
            self.defer_composer_submit(PendingComposerSubmit::Send);
            return;
        }
        let draft = self.take_composer_draft();
        if draft.is_empty() {
            return;
        }
        self.reset_prompt_history_recall();

        let draft_text = draft.text();
        let parsed = commands::parse_invocation(draft_text.as_str());
        let client_command = parsed.and_then(|(name, _)| self.find_client_command(name));
        let plugin_command = parsed.and_then(|(name, args)| {
            self.plugin_slash_commands()
                .into_iter()
                .find(|entry| plugin_command_matches_name(entry, name))
                .map(|entry| (entry, args.to_string()))
        });
        if client_command.is_some() || plugin_command.is_some() {
            if draft.activities().next().is_some() {
                self.restore_composer_draft(draft);
                self.flash_warning(ui_text::t(
                    &self.i18n,
                    "flash-command-does-not-support-attachments",
                ));
                return;
            }
            let args = parsed.map(|(_, args)| args.to_string()).unwrap_or_default();
            if let Some(command) = client_command {
                // A `Client` target is this client's own command; it never
                // reaches the server.
                self.execute_command(&command, args.as_str());
            } else if let Some((entry, args)) = plugin_command {
                self.execute_plugin_slash_command(entry, args.as_str());
            }
            return;
        }

        self.send_captured_draft(draft);
    }

    /// Submit an already captured message without taking the current editor.
    /// An HTTP acknowledgment can arrive while the user writes the next draft;
    /// routing through `submit_composer` would read that newer editor state.
    pub(crate) fn submit_user_draft(&mut self, draft: ComposerDraft) {
        let draft = if draft.text().starts_with("//") {
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

    pub(crate) fn submit_permission_reply(
        &mut self,
        session_id: i64,
        request: PermissionRequest,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        if self.run_activity.has_operation(
            crate::RunActivityTarget::Session(session_id),
            crate::RunOperation::PermissionReply,
        ) {
            return;
        }
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
    App, AppMessage, ComposerDraft, Instant, PendingComposerSubmit, PermissionReplyKind,
    PermissionRequest, PermissionScope, RunActivityTarget, RunOperation, commands,
    composer_draft_with_text_prefix_stripped, derive_session_title, draft_title_source,
    plugin_command_matches_name, run_status_line_command, ui_text,
};
