impl App {
    pub(crate) fn handle_message(&mut self, message: AppMessage) {
        match message {
            AppMessage::UsageStatsLoaded { request_id, result } => {
                self.handle_usage_stats_loaded(request_id, result)
            }
            AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            } => self.handle_sessions_loaded(scope, subtree_root_id, result),
            AppMessage::SessionCreated {
                submit_draft,
                pending_message_id,
                result,
            } => self.handle_session_created(submit_draft, pending_message_id, result),
            AppMessage::SessionStateLoaded { session_id, result } => {
                self.handle_session_state_loaded(session_id, result)
            }
            AppMessage::SessionRefreshed { session_id, result } => {
                self.handle_session_refreshed(session_id, result)
            }
            AppMessage::SessionMessageSubmitted {
                session_id,
                pending_message_id,
                draft,
                result,
            } => self.handle_session_turn_submitted(session_id, pending_message_id, draft, result),
            AppMessage::SessionContinued { session_id, result } => {
                self.handle_session_continued(session_id, RunOperation::Continue, result)
            }
            AppMessage::SessionCompacted { session_id, result } => {
                self.handle_session_continued(session_id, RunOperation::Compact, result)
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
                message_text,
                target,
                result,
            } => self.handle_session_rewound(session_id, message_text, target, result),
            AppMessage::SessionEventArrived { session_id, live } => {
                self.handle_session_event_arrived(session_id, live)
            }
            AppMessage::SteerSubmitted {
                session_id,
                pending_message_id,
                draft,
                result,
            } => self.handle_steer_submitted(session_id, pending_message_id, draft, result),
            AppMessage::RunCancelled { session_id, result } => {
                self.handle_turn_cancelled(session_id, result)
            }
            AppMessage::StatusLineUpdated { output } => self.handle_status_line_updated(output),
        }
    }

    pub(crate) fn handle_status_line_updated(&mut self, output: Option<String>) {
        if let Some(status_line) = self.status_line.as_mut() {
            status_line.apply_refresh(output);
        }
    }

    pub(crate) fn handle_sessions_loaded(
        &mut self,
        scope: SessionLoadScope,
        subtree_root_id: Option<i64>,
        result: UiResult<Vec<SessionResource>>,
    ) {
        if self.session_load.pending_scope.as_ref() != Some(&scope) {
            return;
        }

        self.session_load.pending_scope = None;
        self.session_load.loading = false;

        let selected_id = self
            .sessions
            .current_selected_id()
            .or(self.transcript.session_id)
            .or(self.launch.initial_session_id);

        match result {
            Ok(items) => {
                self.sessions.replace_items(
                    items
                        .into_iter()
                        .map(|session| agena_tui_session::session_list::SessionListItem {
                            session_id: session.id,
                            parent_session_id: session.parent_id,
                            title: session.title,
                            updated_at_millis: session.updated_at.timestamp_millis(),
                        })
                        .collect(),
                    subtree_root_id,
                    selected_id,
                );
                self.session_load.initialized = true;

                if self.transcript.session_id.is_none()
                    && self.launch.initial_session_id.is_none()
                    && let Some(session) = self.sessions.current_selected().cloned()
                {
                    self.open_session(session.session_id, session.title);
                }
            }
            Err(error) => {
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn handle_session_created(
        &mut self,
        submit_draft: Option<ComposerDraft>,
        pending_message_id: Option<u64>,
        result: UiResult<SessionResource>,
    ) {
        if submit_draft.is_some() {
            self.finish_run_operation(RunActivityTarget::NewSession, RunOperation::CreateSession);
        }
        match result {
            Ok(session) => {
                self.request_sessions(false);
                if submit_draft.is_some() {
                    self.clear_draft_for_slot(DraftSlot::NewSession);
                }
                self.open_session(session.id, session.title.clone());
                self.focus = Focus::Composer;

                if let Some(draft) = submit_draft {
                    let pending_message_id = match pending_message_id {
                        Some(pending_message_id) => {
                            self.transcript
                                .add_pending_user_message(PendingUserMessage {
                                    id: pending_message_id,
                                    document: draft.document.clone(),
                                    confirmed: false,
                                });
                            pending_message_id
                        }
                        None => self.begin_pending_user_message(&draft),
                    };
                    self.request_submit_message_with_pending(
                        session.id,
                        draft,
                        Some(pending_message_id),
                    );
                } else {
                    self.flash_success(self.i18n.text_args(
                        "flash-created-session",
                        &agena_tui::fl_args!("title" => session.title.clone()),
                    ));
                }
            }
            Err(error) => {
                if let Some(pending_message_id) = pending_message_id {
                    self.transcript
                        .remove_pending_user_message(pending_message_id);
                }
                if let Some(draft) =
                    submit_draft.or_else(|| self.session_composer.pending_restore_draft.take())
                {
                    self.session_composer.pending_restore_draft = None;
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn handle_session_state_loaded(
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

    pub(crate) fn handle_session_refreshed(
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
                if refresh.event_count > 0 {
                    self.sync_session_list_selection_to_current_execution();
                }
                if refresh.latest_event_seq.is_some() {
                    self.transcript.last_event_seq = refresh.latest_event_seq;
                }
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn handle_session_turn_submitted(
        &mut self,
        session_id: i64,
        pending_message_id: u64,
        draft: ComposerDraft,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::SubmitMessage,
        );
        match result {
            Ok(execution) => {
                self.transcript
                    .confirm_pending_user_message(pending_message_id);
                self.record_prompt_history_from_draft(&draft);
                self.session_composer.pending_restore_draft = None;
                self.clear_draft_for_slot(DraftSlot::Session(session_id));
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
                self.transcript
                    .remove_pending_user_message(pending_message_id);
                self.session_composer.pending_restore_draft = None;
                if self.transcript.session_id == Some(session_id) {
                    self.restore_composer_draft(draft);
                }
                self.flash_error(error);
                // Pause draining: a failed run typically means the user
                // wants to inspect the error rather than fire the next
                // queued message blindly. They can press Ctrl+Up to recover
                // the queue contents.
            }
        }
    }

    /// Pop one editable message from the queue and submit it. Called
    /// whenever an active execution completes successfully so the user sees
    /// their pending messages run automatically.
    pub(crate) fn try_drain_queue_one(&mut self) {
        if self.current_session_activity().is_busy()
            || self.current_session_pending_interactive_kind().is_some()
        {
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

    pub(crate) fn handle_steer_submitted(
        &mut self,
        session_id: i64,
        pending_message_id: u64,
        draft: ComposerDraft,
        result: UiResult<()>,
    ) {
        match result {
            Ok(()) => {
                self.transcript
                    .confirm_pending_user_message(pending_message_id);
                self.record_prompt_history_from_draft(&draft);
                if self.transcript.session_id == Some(session_id) {
                    self.request_refresh(session_id, true);
                }
            }
            Err(error) => {
                self.transcript
                    .remove_pending_user_message(pending_message_id);
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

    pub(crate) fn handle_turn_cancelled(&mut self, session_id: i64, result: UiResult<()>) {
        self.run_activity.clear_session(session_id);
        if self.transcript.session_id == Some(session_id) {
            self.request_refresh(session_id, true);
        }
        match result {
            Ok(()) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
            }
            Err(error) => {
                // Even on error we already cleared local activity state —
                // surface the failure but don't try to recover state.
                self.flash_warning(format!(
                    "{}: {}",
                    ui_text::t(&self.i18n, "flash-cancel-failed"),
                    error
                ));
            }
        }
    }

    pub(crate) fn handle_session_execution_updated(
        &mut self,
        session_id: i64,
        execution: SessionExecutionResource,
        refresh: bool,
    ) {
        let transcript_is_target = self.transcript.session_id == Some(session_id);
        if transcript_is_target && self.apply_transcript_execution(execution) {
            self.sync_pending_interactive_after_execution(session_id);
            self.sync_session_list_selection_to_current_execution();
        }
        if refresh && transcript_is_target {
            self.request_refresh(session_id, true);
        }
        self.request_sessions(false);
    }

    pub(crate) fn handle_session_continued(
        &mut self,
        session_id: i64,
        operation: RunOperation,
        result: UiResult<SessionExecutionResource>,
    ) {
        self.finish_run_operation(RunActivityTarget::Session(session_id), operation);
        match result {
            Ok(execution) => {
                self.handle_session_execution_updated(session_id, execution, true);
                if operation == RunOperation::Compact {
                    self.flash_success(ui_text::t(&self.i18n, "flash-session-compacted"));
                }
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn handle_session_renamed(
        &mut self,
        session_id: i64,
        result: UiResult<SessionResource>,
    ) {
        match result {
            Ok(session) => {
                self.sessions
                    .replace_item(agena_tui_session::session_list::SessionListItem {
                        session_id: session.id,
                        parent_session_id: session.parent_id,
                        title: session.title.clone(),
                        updated_at_millis: session.updated_at.timestamp_millis(),
                    });
                if self.transcript.session_id == Some(session_id) {
                    self.transcript.session_title = session.title.clone();
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        execution.session = session.clone();
                    }
                }
                self.request_sessions(false);
                self.flash_success(self.i18n.text_args(
                    "flash-session-renamed",
                    &agena_tui::fl_args!("title" => session.title),
                ));
                self.overlay = None;
            }
            Err(error) => self.flash_error(error),
        }
    }
}
use crate::{
    App, AppMessage, ComposerDraft, DraftSlot, PendingUserMessage, QueuePriority, QueuedMessage,
    RunActivityTarget, RunOperation, SessionExecutionResource, SessionLoadScope, SessionRefresh,
    SessionResource, UiResult, execution_update_is_stale, ui_text,
};
use agena_tui::main_focus::Focus;
