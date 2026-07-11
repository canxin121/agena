impl App {
    pub(in crate::app) fn handle_message(&mut self, message: AppMessage) {
        match message {
            AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            } => self.handle_sessions_loaded(scope, subtree_root_id, result),
            AppMessage::SessionCreated {
                submit_draft,
                result,
            } => self.handle_session_created(submit_draft, result),
            AppMessage::SessionStateLoaded { session_id, result } => {
                self.handle_session_state_loaded(session_id, result)
            }
            AppMessage::MessagesLoaded {
                session_id,
                mode,
                result,
            } => self.handle_messages_loaded(session_id, mode, result),
            AppMessage::SessionRefreshed { session_id, result } => {
                self.handle_session_refreshed(session_id, result)
            }
            AppMessage::SessionMessageSubmitted {
                session_id,
                draft,
                result,
            } => self.handle_session_turn_submitted(session_id, draft, result),
            AppMessage::SessionContinued { session_id, result } => {
                self.handle_session_continued(session_id, result)
            }
            AppMessage::SessionCompacted { session_id, result } => {
                self.handle_session_continued(session_id, result)
            }
            AppMessage::SessionRenamed { session_id, result } => {
                self.handle_session_renamed(session_id, result)
            }
            AppMessage::PermissionReplied {
                session_id,
                label,
                result,
            } => self.handle_permission_replied(session_id, label, result),
            AppMessage::UserInputReplied { session_id, result } => {
                self.handle_user_input_replied(session_id, result)
            }
            AppMessage::SessionSearchPageLoaded {
                mode,
                query,
                page_index,
                result,
            } => self.handle_session_search_page_loaded(mode, query, page_index, result),
            AppMessage::SessionSearchSubtreeLoaded {
                session_id,
                query,
                result,
            } => self.handle_session_search_subtree_loaded(session_id, query, result),
            AppMessage::LineageLoaded { session_id, result } => {
                self.handle_lineage_loaded(session_id, result)
            }
            AppMessage::RewindMessagesLoaded { session_id, result } => {
                self.handle_rewind_messages_loaded(session_id, result)
            }
            AppMessage::ProvidersLoaded { purpose, result } => {
                self.handle_providers_loaded(purpose, result)
            }
            AppMessage::AgentsLoaded { result } => self.handle_agents_loaded(result),
            AppMessage::ModelCatalogLoaded {
                query,
                offset,
                result,
            } => self.handle_model_catalog_loaded(query, offset, result),
            AppMessage::ProviderStudioAdapterModelsLoaded {
                request_key,
                result,
            } => self.handle_provider_studio_adapter_models_loaded(request_key, result),
            AppMessage::ProviderStudioAuthCompleted {
                request_key,
                result,
            } => self.handle_provider_studio_auth_completed(request_key, result),
            AppMessage::ProviderStudioSaved {
                provider_id,
                result,
            } => self.handle_provider_studio_saved(provider_id, result),
            AppMessage::ModelCatalogRefreshed { result } => {
                self.handle_model_catalog_refreshed(result)
            }
            AppMessage::ChildSessionsLoaded {
                parent_session_id,
                result,
            } => self.handle_child_sessions_loaded(parent_session_id, result),
            AppMessage::TimelineLoaded { session_id, result } => {
                self.handle_timeline_loaded(session_id, result)
            }
            AppMessage::SessionRewound {
                session_id,
                target,
                result,
            } => self.handle_session_rewound(session_id, target, result),
            AppMessage::SessionEventArrived { session_id, live } => {
                self.handle_session_event_arrived(session_id, live)
            }
            AppMessage::SteerSubmitted {
                session_id,
                draft,
                result,
            } => self.handle_steer_submitted(session_id, draft, result),
            AppMessage::RunCancelled { session_id, result } => {
                self.handle_turn_cancelled(session_id, result)
            }
            AppMessage::StatusLineUpdated { output } => self.handle_status_line_updated(output),
        }
    }

    pub(in crate::app) fn handle_status_line_updated(&mut self, output: Option<String>) {
        if let Some(status_line) = self.status_line.as_mut() {
            status_line.running = false;
            status_line.text = output;
        }
    }

    pub(in crate::app) fn handle_sessions_loaded(
        &mut self,
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    ) {
        if self.sessions.pending_scope.as_ref() != Some(&scope) {
            return;
        }

        self.sessions.pending_scope = None;
        self.sessions.loading = false;
        self.sessions.loading_more = false;

        let selected_id = self
            .sessions
            .current_selected_id()
            .or(self.transcript.session_id)
            .or(self.launch.initial_session_id);

        match result {
            Ok(items) => {
                self.sessions.source_items = items;
                self.sessions.subtree_root_id = subtree_root_id;
                self.sessions.initialized = true;
                self.rebuild_visible_sessions(selected_id);

                if let Some(id) = selected_id {
                    let _ = self.sessions.select_by_id(id);
                }

                if self.transcript.session_id.is_none()
                    && self.launch.initial_session_id.is_none()
                    && let Some(session) = self.sessions.current_selected().cloned()
                {
                    self.open_session(session.id, session.title);
                }
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn handle_session_created(
        &mut self,
        submit_draft: Option<ComposerDraft>,
        result: UiResult<SessionResource>,
    ) {
        match result {
            Ok(session) => {
                self.request_sessions(false);
                if submit_draft.is_some() {
                    self.clear_draft_for_slot(DraftSlot::NewSession);
                }
                self.open_session(session.id, session.title.clone());
                self.focus = Focus::Composer;

                if let Some(draft) = submit_draft {
                    self.request_submit_message(session.id, draft);
                } else {
                    self.flash_success(self.i18n.text_args(
                        "flash-created-session",
                        &crate::fl_args!("title" => session.title.clone()),
                    ));
                }
            }
            Err(error) => {
                self.transcript.submitting = false;
                if let Some(draft) =
                    submit_draft.or_else(|| self.transcript.pending_restore_draft.take())
                {
                    self.transcript.pending_restore_draft = None;
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn handle_session_state_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.state_loading = false;
        match result {
            Ok(execution) => {
                let session_id = execution.session.id;
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn handle_messages_loaded(
        &mut self,
        session_id: i64,
        mode: MessageLoadMode,
        result: UiResult<PaginatedResponse<MessageResource>>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        match mode {
            MessageLoadMode::Replace => self.transcript.loading_initial = false,
            MessageLoadMode::Prepend => self.transcript.loading_older = false,
        }

        match result {
            Ok(page) => match mode {
                MessageLoadMode::Replace => {
                    self.transcript.replace_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                MessageLoadMode::Prepend => {
                    self.transcript.prepend_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
            },
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn handle_session_refreshed(
        &mut self,
        session_id: i64,
        result: UiResult<SessionRefresh>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.refreshing = false;

        match result {
            Ok(refresh) => {
                if execution_update_is_stale(
                    self.transcript.last_event_seq,
                    refresh.latest_event_seq,
                ) {
                    return;
                }
                if let Some(execution) = refresh.execution {
                    let session_id = execution.session.id;
                    if self.apply_transcript_execution(execution) {
                        self.sync_pending_interactive_after_execution(session_id);
                        self.sync_session_list_selection_to_current_execution();
                    }
                }
                if let Some(page) = refresh.latest_messages {
                    self.transcript.merge_latest_messages(
                        page,
                        self.layout.transcript_body.width,
                        self.layout.transcript_body.height,
                    );
                }
                if refresh.event_count > 0 {
                    self.sync_session_list_selection_to_current_execution();
                }
                if refresh.latest_event_seq.is_some() {
                    self.transcript.last_event_seq = refresh.latest_event_seq;
                }
                self.maybe_request_older_messages();
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(in crate::app) fn handle_session_turn_submitted(
        &mut self,
        session_id: i64,
        draft: ComposerDraft,
        result: UiResult<SessionExecutionResource>,
    ) {
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = false;
        }
        self.submitting_session_ids.remove(&session_id);
        match result {
            Ok(execution) => {
                self.transcript.pending_restore_draft = None;
                self.clear_draft_for_slot(DraftSlot::Session(session_id));
                cleanup_temporary_composer_items(draft.items.as_slice());
                if self.transcript.session_id != Some(session_id) {
                    self.open_session(session_id, execution.session.title.clone());
                }
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                self.request_refresh(session_id, true);
                self.request_sessions(false);
                // Pop the next pending message and submit it after the run.
                self.try_drain_queue_one();
            }
            Err(error) => {
                self.transcript.pending_restore_draft = None;
                if self.transcript.session_id == Some(session_id) {
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
                // Pause draining: a failed run typically means the user
                // wants to inspect the error rather than fire the next
                // queued message blindly. They can press Up to recover
                // the queue contents.
            }
        }
    }

    /// Pop one editable message from the queue and submit it. Called
    /// whenever an in-flight run completes successfully so the user sees
    /// their pending messages run automatically.
    pub(in crate::app) fn try_drain_queue_one(&mut self) {
        if self.transcript.submitting || self.current_session_pending_interactive_kind().is_some() {
            return;
        }
        let Some(msg) = self.queue.pop_next() else {
            return;
        };
        // Reuse the normal submit path. We stash it into the editor
        // first so any error path can put the text back in front of the
        // user.
        self.restore_composer_draft(msg.draft);
        self.submit_composer();
    }

    pub(in crate::app) fn handle_steer_submitted(
        &mut self,
        _session_id: i64,
        draft: ComposerDraft,
        result: UiResult<()>,
    ) {
        match result {
            Ok(()) => {}
            Err(error) => {
                // Backend rejected the steer (run no longer steerable).
                // Don't drop the user's message — push it onto the front
                // of the queue so it goes out at the next run boundary.
                self.queue.push(QueuedMessage {
                    draft,
                    priority: QueuePriority::Now,
                    editable: true,
                });
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-steer-failed-fallback-queue"),
                    error
                ));
            }
        }
    }

    pub(in crate::app) fn handle_turn_cancelled(&mut self, session_id: i64, result: UiResult<()>) {
        if self.transcript.session_id == Some(session_id) {
            self.transcript.submitting = false;
        }
        self.submitting_session_ids.remove(&session_id);
        if self.transcript.session_id == Some(session_id) {
            self.request_refresh(session_id, true);
        }
        match result {
            Ok(()) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
            }
            Err(error) => {
                // Even on error we already cleared submitting locally —
                // surface the failure but don't try to recover state.
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-cancel-failed"),
                    error
                ));
            }
        }
    }

    pub(in crate::app) fn handle_session_execution_updated(
        &mut self,
        session_id: i64,
        execution: SessionExecutionResource,
        refresh: bool,
    ) {
        let transcript_is_target = self.transcript.session_id == Some(session_id);
        if transcript_is_target {
            self.transcript.submitting = false;
            if self.apply_transcript_execution(execution) {
                self.sync_pending_interactive_after_execution(session_id);
                self.sync_session_list_selection_to_current_execution();
            }
        }
        self.submitting_session_ids.remove(&session_id);
        if refresh && transcript_is_target {
            self.request_refresh(session_id, true);
        }
        self.request_sessions(false);
    }

    pub(in crate::app) fn handle_session_continued(
        &mut self,
        session_id: i64,
        result: UiResult<SessionExecutionResource>,
    ) {
        match result {
            Ok(execution) => self.handle_session_execution_updated(session_id, execution, true),
            Err(error) => {
                self.transcript.submitting = false;
                self.submitting_session_ids.remove(&session_id);
                self.flash_error(error);
            }
        }
    }

    pub(in crate::app) fn handle_session_renamed(
        &mut self,
        session_id: i64,
        result: UiResult<SessionResource>,
    ) {
        match result {
            Ok(session) => {
                if let Some(existing) = self
                    .sessions
                    .source_items
                    .iter_mut()
                    .find(|item| item.id == session_id)
                {
                    *existing = session.clone();
                }
                if let Some(existing) = self
                    .sessions
                    .list
                    .items
                    .iter_mut()
                    .find(|item| item.id == session_id)
                {
                    *existing = session.clone();
                }
                if self.transcript.session_id == Some(session_id) {
                    self.transcript.session_title = session.title.clone();
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        execution.session = session.clone();
                    }
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-renamed",
                    &crate::fl_args!("title" => session.title),
                ));
                self.overlay = None;
            }
            Err(error) => self.flash_error(error),
        }
    }
}
use crate::app::{
    App, AppMessage, ComposerDraft, DraftSlot, Focus, MessageLoadMode, MessageResource,
    PaginatedResponse, QueuePriority, QueuedMessage, SessionExecutionResource, SessionLoadScope,
    SessionRefresh, SessionResource, UiResult, cleanup_temporary_composer_items,
    execution_update_is_stale, ui_text,
};
