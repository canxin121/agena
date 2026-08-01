impl App {
    pub(crate) fn request_sessions(&mut self, append: bool) {
        if append {
            return;
        }
        if self.session_load.loading {
            return;
        }
        self.session_load.loading = true;

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let scope = SessionLoadScope {
            mode: self.sessions.view_mode(),
            anchor_session_id: match self.sessions.view_mode() {
                SessionViewMode::Subtree => self.current_or_selected_session_id(),
                SessionViewMode::All | SessionViewMode::Roots => None,
            },
        };
        if scope.mode == SessionViewMode::Subtree && scope.anchor_session_id.is_none() {
            self.session_load.loading = false;
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.session_load.pending_scope = Some(scope.clone());

        tokio::spawn(async move {
            let (result, subtree_root_id) = match scope.mode {
                SessionViewMode::All => (
                    backend
                        .list_workspace_sessions(false)
                        .await
                        .map_err(crate::UiFailure::internal),
                    None,
                ),
                SessionViewMode::Roots => (
                    backend
                        .list_workspace_sessions(true)
                        .await
                        .map_err(crate::UiFailure::internal),
                    None,
                ),
                SessionViewMode::Subtree => {
                    let anchor_session_id = scope
                        .anchor_session_id
                        .expect("subtree scope requires anchor");
                    let result = backend
                        .list_session_subtree(anchor_session_id)
                        .await
                        .map_err(crate::UiFailure::internal);
                    let subtree_root_id = result.as_ref().ok().and_then(|items| {
                        items
                            .iter()
                            .find(|item| item.parent_id.is_none())
                            .map(|item| item.id)
                    });
                    (result, subtree_root_id)
                }
            };
            let _ = tx.send(AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            });
        });
    }

    pub(crate) fn request_providers(&mut self, purpose: ProviderPickerPurpose) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Ok(backend.list_configured_providers());
            let _ = tx.send(AppMessage::ProvidersLoaded { purpose, result });
        });
    }

    pub(crate) fn request_session_search_page(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        cursor: Option<String>,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_workspace_sessions_page(
                    mode == SessionViewMode::Roots,
                    (!query.trim().is_empty()).then_some(query.as_str()),
                    cursor,
                    50,
                )
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::SessionSearchPageLoaded {
                mode,
                query,
                page_index,
                result,
            });
        });
    }

    pub(crate) fn request_session_search_subtree(&mut self, session_id: i64, query: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::SessionSearchSubtreeLoaded {
                session_id,
                query,
                result,
            });
        });
    }

    pub(crate) fn request_lineage(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::LineageLoaded { session_id, result });
        });
    }

    pub(crate) fn request_rewind_messages(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .get_session_state(session_id)
                .await
                .map(|state| state.transcript.turns)
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::RewindMessagesLoaded { session_id, result });
        });
    }

    pub(crate) fn request_child_sessions(&mut self, parent_session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_child_sessions(parent_session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::ChildSessionsLoaded {
                parent_session_id,
                result,
            });
        });
    }

    pub(crate) fn request_session_rename(&mut self, session_id: i64, title: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rename_session(session_id, title)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::SessionRenamed { session_id, result });
        });
    }

    pub(crate) fn request_timeline(&mut self, session_id: i64, limit: u64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_timeline(session_id, limit)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx.send(AppMessage::TimelineLoaded { session_id, result });
        });
    }

    pub(crate) fn request_session_rewind(
        &mut self,
        session_id: i64,
        turn_id: agena_domain::TurnId,
        message_text: String,
        target: String,
    ) {
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        self.begin_run_operation(RunActivityTarget::Session(session_id), RunOperation::Rewind);

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rewind_session_to_turn(session_id, turn_id)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionRewound {
                session_id,
                message_text,
                target,
                result,
            });
        });
    }

    pub(crate) fn request_session_state(&mut self, session_id: i64) {
        if self.transcript.state_loading {
            return;
        }

        self.transcript.state_loading = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .get_session_state(session_id)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionStateLoaded { session_id, result });
        });
    }

    pub(crate) fn request_refresh(&mut self, session_id: i64, force: bool) {
        if self.transcript.refreshing {
            return;
        }
        self.transcript.refreshing = true;
        self.last_refresh_at = Instant::now();

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let after_seq = self.transcript.last_event_seq;
        tokio::spawn(async move {
            let result = backend
                .refresh_session(session_id, after_seq, force)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionRefreshed { session_id, result });
        });
    }

    pub(crate) fn begin_pending_user_message(&mut self, draft: &ComposerDraft) -> u64 {
        let id = self.next_pending_user_message_id;
        self.next_pending_user_message_id = self.next_pending_user_message_id.saturating_add(1);
        self.transcript
            .add_pending_user_message(PendingUserMessage {
                id,
                document: draft.document.clone(),
                confirmed: false,
            });
        self.transcript.scroll_to_bottom(
            self.layout.transcript_body.width,
            self.layout.transcript_body.height,
        );
        id
    }

    pub(crate) fn request_submit_message(&mut self, session_id: i64, draft: ComposerDraft) {
        self.request_submit_message_with_pending(session_id, draft, None);
    }

    pub(crate) fn request_submit_message_with_pending(
        &mut self,
        session_id: i64,
        draft: ComposerDraft,
        existing_pending_message_id: Option<u64>,
    ) {
        if self
            .pending_interactive_kind_for_session(session_id)
            .is_some()
        {
            self.restore_composer_draft(draft);
            self.focus = Focus::Composer;
            self.prompt_for_pending_interactive_on_session(session_id);
            return;
        }
        let pending_message_id =
            existing_pending_message_id.unwrap_or_else(|| self.begin_pending_user_message(&draft));
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::SubmitMessage,
        );
        self.session_composer.pending_restore_draft = Some(draft.clone());
        self.set_draft_for_slot(DraftSlot::Session(session_id), draft.clone());
        self.persist_draft_store_with_feedback(true);

        let document = match self.build_submission_document(&draft) {
            Ok(document) => document,
            Err(error) => {
                self.transcript
                    .remove_pending_user_message(pending_message_id);
                self.session_composer.pending_restore_draft = None;
                self.finish_run_operation(
                    RunActivityTarget::Session(session_id),
                    RunOperation::SubmitMessage,
                );
                self.restore_composer_draft(draft);
                self.flash_error(error);
                return;
            }
        };
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .submit_document_with_options(session_id, document, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionMessageSubmitted {
                session_id,
                pending_message_id,
                draft,
                result,
            });
        });
    }

    pub(crate) fn request_continue(&mut self, session_id: i64) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::Continue,
        );
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .continue_session_with_options(session_id, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionContinued { session_id, result });
        });
    }

    pub(crate) fn request_compact(&mut self, session_id: i64) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::Compact,
        );
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .compact_session_with_options(session_id, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SessionCompacted { session_id, result });
        });
    }

    /// Steer the active execution by injecting `parts` as a new user message
    /// the model will see on its next step. If the backend reports the
    /// run is no longer steerable, fall back to enqueueing the original
    /// draft so it isn't lost.
    pub(crate) fn request_steer_input(
        &mut self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
        draft: ComposerDraft,
    ) {
        let pending_message_id = self.begin_pending_user_message(&draft);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .steer_input(session_id, document)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::SteerSubmitted {
                session_id,
                pending_message_id,
                draft,
                result,
            });
        });
        self.flash_info(ui_text::t(&self.i18n, "flash-steer-sent"));
    }

    /// Ask the backend to cancel the active execution for `session_id`.
    /// Best-effort: even if the backend hasn't fully wired cancellation,
    /// clear the stale local execution marker immediately so the composer and
    /// activity indicator respond in the same frame as Ctrl+C.
    pub(crate) fn request_cancel_run(&mut self, session_id: i64) {
        let Some(execution_id) = self
            .transcript
            .execution
            .as_ref()
            .and_then(|execution| execution.active_execution.as_ref())
            .map(|execution| agena_domain::ExecutionId(execution.execution_id))
        else {
            self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
            return;
        };
        self.run_activity.clear_session(session_id);
        if self.transcript.session_id == Some(session_id) {
            // The cached resource may still advertise an active execution (or
            // the permission request that just launched an approved tool).
            // Drop that stale control-plane snapshot until the cancel response
            // triggers a fresh load; transcript messages are stored separately.
            self.transcript.execution = None;
        }
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .cancel_run(session_id, execution_id)
                .await
                .and_then(|result| match result {
                    agena_domain::CancellationResult::CancellationRequested
                    | agena_domain::CancellationResult::AlreadyTerminal
                    | agena_domain::CancellationResult::NotFound => Ok(()),
                    agena_domain::CancellationResult::ExecutionMismatch => Err(anyhow::anyhow!(
                        "the active execution changed before cancellation"
                    )),
                })
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::RunCancelled { session_id, result });
        });
    }

    pub(crate) fn request_permission_reply(
        &mut self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::PermissionReply,
        );
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        let replied_request_id = request_id.clone();
        tokio::spawn(async move {
            let result = backend
                .reply_permission_with_options(session_id, request_id, kind, scope, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::PermissionReplied {
                session_id,
                request_id: replied_request_id,
                label,
                result,
            });
        });
    }

    pub(crate) fn request_user_input_reply(&mut self, session_id: i64, reply: UserInputReply) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::UserInputReply,
        );
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        let request_id = reply.request_id.clone();
        tokio::spawn(async move {
            let result = backend
                .reply_user_input_with_options(session_id, reply, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx.send(AppMessage::UserInputReplied {
                session_id,
                request_id,
                result,
            });
        });
    }
}
use crate::{
    App, AppMessage, ComposerDraft, DraftSlot, Instant, PendingUserMessage, PermissionReplyKind,
    PermissionScope, ProviderPickerPurpose, RunActivityTarget, RunOperation, SessionLoadScope,
    UserInputReply, ui_text,
};
use agena_tui::main_focus::Focus;
use agena_tui_session::session_view::SessionViewMode;
