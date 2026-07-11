impl App {
    pub(in crate::app) fn request_sessions(&mut self, append: bool) {
        if append {
            return;
        }
        if self.sessions.loading {
            return;
        }
        self.sessions.loading = true;
        self.sessions.loading_more = false;
        self.sessions.has_more = false;
        self.sessions.next_cursor = None;

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let scope = SessionLoadScope {
            mode: self.sessions.view_mode,
            anchor_session_id: match self.sessions.view_mode {
                SessionViewMode::Subtree => self.current_or_selected_session_id(),
                SessionViewMode::All | SessionViewMode::Roots => None,
            },
        };
        if scope.mode == SessionViewMode::Subtree && scope.anchor_session_id.is_none() {
            self.sessions.loading = false;
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.sessions.pending_scope = Some(scope.clone());

        tokio::spawn(async move {
            let (result, subtree_root_id) = match scope.mode {
                SessionViewMode::All => (
                    backend
                        .list_workspace_sessions(false)
                        .await
                        .map_err(|error| error.to_string()),
                    None,
                ),
                SessionViewMode::Roots => (
                    backend
                        .list_workspace_sessions(true)
                        .await
                        .map_err(|error| error.to_string()),
                    None,
                ),
                SessionViewMode::Subtree => {
                    let anchor_session_id = scope
                        .anchor_session_id
                        .expect("subtree scope requires anchor");
                    let result = backend
                        .list_session_subtree(anchor_session_id)
                        .await
                        .map_err(|error| error.to_string());
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

    pub(in crate::app) fn request_providers(&mut self, purpose: ProviderPickerPurpose) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Ok(match purpose {
                ProviderPickerPurpose::SetProvider => backend.list_providers(),
                ProviderPickerPurpose::Configure => backend.list_configured_providers(),
            });
            let _ = tx.send(AppMessage::ProvidersLoaded { purpose, result });
        });
    }

    pub(in crate::app) fn request_agent_list(&mut self) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = Ok(backend.list_agent_descriptors());
            let _ = tx.send(AppMessage::AgentsLoaded { result });
        });
    }

    pub(in crate::app) fn request_session_search_page(
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
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionSearchPageLoaded {
                mode,
                query,
                page_index,
                result,
            });
        });
    }

    pub(in crate::app) fn request_session_search_subtree(
        &mut self,
        session_id: i64,
        query: String,
    ) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionSearchSubtreeLoaded {
                session_id,
                query,
                result,
            });
        });
    }

    pub(in crate::app) fn request_lineage(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_subtree(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::LineageLoaded { session_id, result });
        });
    }

    pub(in crate::app) fn request_rewind_messages(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_all_messages(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::RewindMessagesLoaded { session_id, result });
        });
    }

    pub(in crate::app) fn request_child_sessions(&mut self, parent_session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_child_sessions(parent_session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::ChildSessionsLoaded {
                parent_session_id,
                result,
            });
        });
    }

    pub(in crate::app) fn request_session_rename(&mut self, session_id: i64, title: String) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rename_session(session_id, title)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRenamed { session_id, result });
        });
    }

    pub(in crate::app) fn request_timeline(&mut self, session_id: i64, limit: u64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_session_timeline(session_id, limit)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::TimelineLoaded { session_id, result });
        });
    }

    pub(in crate::app) fn request_session_rewind(
        &mut self,
        session_id: i64,
        message_id: i64,
        target: String,
    ) {
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = true;
        }

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .rewind_session_to_message(session_id, message_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRewound {
                session_id,
                target,
                result,
            });
        });
    }

    pub(in crate::app) fn request_session_state(&mut self, session_id: i64) {
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
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionStateLoaded { session_id, result });
        });
    }

    pub(in crate::app) fn request_messages(&mut self, session_id: i64, mode: MessageLoadMode) {
        match mode {
            MessageLoadMode::Replace => {
                if self.transcript.loading_initial {
                    return;
                }
                self.transcript.loading_initial = true;
            }
            MessageLoadMode::Prepend => {
                if self.transcript.loading_older || !self.transcript.has_more_older {
                    return;
                }
                self.transcript.loading_older = true;
            }
        }

        let cursor = match mode {
            MessageLoadMode::Replace => None,
            MessageLoadMode::Prepend => self.transcript.older_cursor.clone(),
        };

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .list_messages(session_id, cursor, MESSAGE_PAGE_SIZE)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::MessagesLoaded {
                session_id,
                mode,
                result,
            });
        });
    }

    pub(in crate::app) fn request_refresh(&mut self, session_id: i64, force: bool) {
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
                .refresh_session(session_id, after_seq, MESSAGE_PAGE_SIZE, force)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionRefreshed { session_id, result });
        });
    }

    pub(in crate::app) fn request_submit_message(&mut self, session_id: i64, draft: ComposerDraft) {
        if self
            .pending_interactive_kind_for_session(session_id)
            .is_some()
        {
            self.restore_composer_draft(draft);
            self.focus = Focus::Composer;
            self.prompt_for_pending_interactive_on_session(session_id);
            return;
        }
        self.transcript.submitting = true;
        self.transcript.pending_restore_draft = Some(draft.clone());
        self.submitting_session_ids.insert(session_id);
        self.set_draft_for_slot(DraftSlot::Session(session_id), draft.clone());
        self.persist_draft_store_with_feedback(true);

        let parts = match self.build_submission_parts(&draft) {
            Ok(parts) => parts,
            Err(error) => {
                self.transcript.submitting = false;
                self.transcript.pending_restore_draft = None;
                self.submitting_session_ids.remove(&session_id);
                self.restore_composer_draft(draft);
                self.flash_error(error);
                return;
            }
        };
        self.record_prompt_history_from_draft(&draft);

        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .submit_parts_message_with_options(session_id, parts, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionMessageSubmitted {
                session_id,
                draft,
                result,
            });
        });
    }

    pub(in crate::app) fn request_continue(&mut self, session_id: i64) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .continue_session_with_options(session_id, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionContinued { session_id, result });
        });
    }

    pub(in crate::app) fn request_compact(&mut self, session_id: i64) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .compact_session_with_options(session_id, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SessionCompacted { session_id, result });
        });
    }

    /// Steer the in-flight run by injecting `parts` as a new user message
    /// the model will see on its next step. If the backend reports the
    /// run is no longer steerable, fall back to enqueueing the original
    /// draft so it isn't lost.
    pub(in crate::app) fn request_steer_input(
        &mut self,
        session_id: i64,
        parts: Vec<PartContent>,
        draft: ComposerDraft,
    ) {
        self.record_prompt_history_from_draft(&draft);
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .steer_input(session_id, parts)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::SteerSubmitted {
                session_id,
                draft,
                result,
            });
        });
        self.flash_info(ui_text::t(&self.i18n, "flash-steer-sent"));
    }

    /// Ask the backend to cancel the in-flight run for `session_id`.
    /// Best-effort: even if the backend hasn't fully wired cancellation,
    /// we clear the local `submitting` flag so the user regains control.
    pub(in crate::app) fn request_cancel_run(&mut self, session_id: i64) {
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = backend
                .cancel_run(session_id)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::RunCancelled { session_id, result });
        });
    }

    pub(in crate::app) fn request_permission_reply(
        &mut self,
        session_id: i64,
        request_id: String,
        kind: PermissionReplyKind,
        scope: Option<PermissionScope>,
        label: String,
    ) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .reply_permission_with_options(session_id, request_id, kind, scope, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::PermissionReplied {
                session_id,
                label,
                result,
            });
        });
    }

    pub(in crate::app) fn request_user_input_reply(
        &mut self,
        session_id: i64,
        reply: UserInputReply,
    ) {
        self.transcript.submitting = true;
        let backend = self.backend.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = backend
                .reply_user_input_with_options(session_id, reply, options)
                .await
                .map_err(|error| error.to_string());
            let _ = tx.send(AppMessage::UserInputReplied { session_id, result });
        });
    }
}
use crate::app::{
    App, AppMessage, ComposerDraft, DraftSlot, Focus, Instant, MESSAGE_PAGE_SIZE, MessageLoadMode,
    PartContent, PermissionReplyKind, PermissionScope, ProviderPickerPurpose, SessionLoadScope,
    SessionViewMode, UserInputReply, ui_text,
};
