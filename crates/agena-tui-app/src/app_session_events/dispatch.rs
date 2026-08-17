impl App {
    pub(crate) fn handle_message(&mut self, message: AppMessage) {
        match message {
            AppMessage::AsyncOperationCompleted(completion) => completion(self),
            AppMessage::ActivitiesLoaded { request_id, result } => {
                self.handle_activities_loaded(request_id, result)
            }
            AppMessage::ActivitiesLogLoaded {
                activity_id,
                request_id,
                result,
            } => self.handle_activity_log_loaded(activity_id, request_id, result),
            AppMessage::ActivitiesStopped {
                activity_id,
                request_id,
                result,
            } => {
                if let Err(error) = &result {
                    self.flash_error(error.clone());
                }
                self.handle_activities_stopped(activity_id, request_id, result.is_ok_and(|ok| ok));
            }
            AppMessage::ActivitiesCleared { request_id, result } => {
                if let Err(error) = &result {
                    self.flash_error(error.clone());
                }
                self.handle_activities_cleared(request_id, result.is_ok_and(|ok| ok));
            }
            AppMessage::PlanViewerLoaded { request_id, result } => {
                self.handle_plan_viewer_loaded(request_id, result)
            }
            AppMessage::PlanAutorunToggled { request_id, result } => {
                self.handle_plan_autorun_toggled(request_id, result)
            }
            AppMessage::PlanDisplayRefreshed { session_id, result } => {
                self.handle_plan_display_refreshed(session_id, result)
            }
            AppMessage::UsageStatsLoaded { request_id, result } => {
                self.handle_usage_stats_loaded(request_id, result)
            }
            AppMessage::HubOverviewLoaded { request_id, result } => {
                self.handle_hub_overview_loaded(request_id, result)
            }
            AppMessage::SessionsLoaded {
                scope,
                subtree_root_id,
                result,
            } => self.handle_sessions_loaded(scope, subtree_root_id, result),
            AppMessage::SessionCreated {
                submit_draft,
                pending_message_id,
                model_stack,
                result,
            } => self.handle_session_created(submit_draft, pending_message_id, model_stack, result),
            AppMessage::SessionStateLoaded { session_id, result } => {
                self.handle_session_state_loaded(session_id, result)
            }
            AppMessage::TranscriptPartsLoaded { session_id, result } => {
                self.handle_transcript_parts_loaded(session_id, result)
            }
            AppMessage::TranscriptFoldPartsLoaded {
                session_id,
                run_id,
                anchor_part_id,
                expand_all,
                result,
            } => self.handle_transcript_fold_parts_loaded(
                session_id,
                run_id,
                anchor_part_id,
                expand_all,
                result,
            ),
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
                request_id,
                kind,
                label,
                result,
            } => self.handle_permission_replied(session_id, request_id, kind, label, result),
            AppMessage::UserInputReplied {
                session_id,
                request_id,
                result,
            } => self.handle_user_input_replied(session_id, request_id, result),
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
            AppMessage::RunCancelled { session_id, result } => {
                self.handle_turn_cancelled(session_id, result)
            }
            AppMessage::StatusLineUpdated { output } => self.handle_status_line_updated(output),
            AppMessage::SideSessionOpened {
                session_id,
                parent_id,
            } => {
                self.handle_side_session_opened(session_id, parent_id);
            }
        }
    }

    pub(crate) fn handle_status_line_updated(&mut self, output: Option<String>) {
        if let Some(status_line) = self.status_line.as_mut() {
            status_line.apply_refresh(output);
        }
    }

    pub(crate) fn handle_side_session_opened(&mut self, session_id: i64, parent_id: i64) {
        self.side_sessions.insert(session_id, parent_id);
    }

    /// Returns the side session to close when switching to `target_session_id`
    /// (codex's `side_thread_to_discard_after_switch` rule). Switching into
    /// the side itself or staying put keeps it; switching anywhere else —
    /// including back to its parent — closes it.
    pub(crate) fn side_session_to_discard_after_switch(
        &self,
        target_session_id: i64,
    ) -> Option<i64> {
        let active_session_id = self.transcript.session_id?;
        let (&side_session_id, &parent_session_id) = self.side_sessions.iter().next()?;
        if target_session_id == side_session_id || target_session_id == active_session_id {
            return None;
        }
        (active_session_id == side_session_id || active_session_id == parent_session_id)
            .then_some(side_session_id)
    }

    /// True when the current transcript is an open side conversation.
    pub(crate) fn current_session_is_side(&self) -> bool {
        self.transcript
            .session_id
            .is_some_and(|session_id| self.side_sessions.contains_key(&session_id))
    }

    /// True when the current transcript is the parent of an open side
    /// conversation (a hint to return into it from the main thread).
    pub(crate) fn current_session_is_side_parent(&self) -> bool {
        let Some(session_id) = self.transcript.session_id else {
            return false;
        };
        self.side_sessions
            .values()
            .any(|parent_session_id| *parent_session_id == session_id)
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
        self.session_load.requested_at = None;

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

                // Launch with no session at all: when nothing was explicitly
                // opened, selected, or requested on the command line, drop the
                // implicit highlight of the newest row so the transcript stays
                // empty and the next submit creates a fresh session instead of
                // silently targeting the most recent one.
                if selected_id.is_none() {
                    self.sessions.clear_selection();
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
        model_stack: Option<RunOptionsState>,
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
                // `open_session` clears the model stack because a selection
                // belongs to the session that produced it. When this session
                // was auto-created from a send with no session open, restore
                // the stack captured at create time so the first submit (and
                // the post-send sync) uses the switched model, not the
                // default. A subsequent session reload still reconciles to
                // the persisted execution selection.
                if let Some(model_stack) = model_stack {
                    self.run_options.replace_model_stack(
                        model_stack.model,
                        model_stack.thinking_mode,
                        model_stack.speed_mode,
                        model_stack.verbosity,
                        model_stack.parallel_tool_calls,
                    );
                }
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
        result: UiResult<crate::app_backend::SessionStateWithTranscriptPage>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.state_loading = false;
        self.transcript.state_load_in_flight_since = None;
        match result {
            Ok(load) => {
                let page = load.page;
                let execution = load.execution;
                let session_id = execution.session.id;
                let execution_is_terminal = execution.session.state.active_execution().is_none();
                if self.apply_transcript_execution(execution) {
                    self.sync_pending_interactive_after_execution(session_id);
                    self.sync_session_list_selection_to_current_execution();
                }
                if !self.transcript.transcript_older_pages_loaded {
                    self.transcript
                        .set_transcript_page(page.next_cursor, page.has_more);
                    self.transcript.set_transcript_folds(page.folds);
                }
                // A session (re)open can deliver the terminal state of a run
                // that finished while the user was elsewhere. Drain a parked
                // message so it is not stranded in the pending slot.
                if execution_is_terminal {
                    self.try_send_pending();
                }
            }
            Err(error) => self.flash_error(error),
        }
    }

    pub(crate) fn handle_transcript_parts_loaded(
        &mut self,
        session_id: i64,
        result: UiResult<crate::app_backend::SessionTranscriptPage>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }

        self.transcript.transcript_older_loading = false;
        self.transcript.transcript_older_in_flight_since = None;
        match result {
            Ok(page) => {
                let loaded_raw_parts = page.parts.len();
                let previous_cursor = self.transcript.transcript_next_cursor.clone();
                let cursor_progressed = page.next_cursor != previous_cursor;
                let visible_before = self.transcript.visible_navigation_boundaries();
                let loaded = self.transcript.prepend_transcript_parts(
                    page.parts,
                    self.layout.transcript_body.width,
                    self.layout.transcript_body.height,
                );
                if loaded && cursor_progressed {
                    let visible_after = self.transcript.visible_navigation_boundaries();
                    let visible_progressed = visible_before != visible_after;
                    self.transcript
                        .set_transcript_page(page.next_cursor, page.has_more);
                    self.transcript.merge_transcript_folds(page.folds);
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        execution.parts = self.transcript.parts.clone();
                    }
                    if visible_progressed || !page.has_more {
                        self.transcript.transcript_older_skip_parts = 0;
                    } else {
                        self.transcript.transcript_older_skip_parts = self
                            .transcript
                            .transcript_older_skip_parts
                            .saturating_add(loaded_raw_parts.max(1));
                        if self.transcript.transcript_older_skip_parts
                            < crate::app_backend::MAX_FOLD_SKIP_RAW_PARTS
                        {
                            // The page only contained hidden folded activity;
                            // continue across the whole raw span while the
                            // user's upward gesture is resolving.
                            self.request_older_transcript_parts_if_needed();
                        } else {
                            self.transcript.transcript_older_skip_parts = 0;
                        }
                    }
                } else {
                    // A repeated cursor or an empty page must not leave the
                    // top trigger in a retry loop forever.
                    self.transcript.set_transcript_page(None, false);
                    self.transcript.transcript_older_skip_parts = 0;
                }
            }
            Err(error) => {
                self.transcript.transcript_older_skip_parts = 0;
                self.flash_error(error);
            }
        }
    }

    pub(crate) fn handle_transcript_fold_parts_loaded(
        &mut self,
        session_id: i64,
        run_id: i64,
        anchor_part_id: i64,
        expand_all: bool,
        result: UiResult<crate::app_backend::SessionTranscriptPage>,
    ) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }
        match result {
            Ok(page) => {
                let next_cursor = page.next_cursor.clone();
                let has_more = page.has_more;
                let loaded = self.transcript.merge_fold_parts(
                    run_id,
                    anchor_part_id,
                    page.parts,
                    next_cursor,
                    has_more,
                );
                if loaded {
                    if let Some(execution) = self.transcript.execution.as_mut() {
                        execution.parts = self.transcript.parts.clone();
                    }
                    if expand_all
                        && page.has_more
                        && let Some(next_fold) = self
                            .transcript
                            .transcript_folds
                            .iter()
                            .find(|fold| fold.run_ids.contains(&run_id))
                            .cloned()
                    {
                        self.request_transcript_fold_parts(next_fold, true);
                    }
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
            // A refresh for a session the user navigated away from is ignored.
            // `open_session` resets the transcript (including `refreshing`),
            // so there is nothing to restore here; clearing state would
            // instead drop a pending refresh that belongs to the *current*
            // session.
            return;
        }

        self.transcript.refreshing = false;
        self.transcript.refresh_in_flight_since = None;
        let pending = self.pending_refresh.take();

        match result {
            Ok(refresh) => {
                if execution_update_is_stale(
                    self.transcript.last_event_seq,
                    refresh.latest_event_seq,
                ) {
                    // The server's durable watermark is authoritative over
                    // the local high-water mark: live-only events
                    // (ActivityV2, streamed text upserts, retry notices)
                    // consume global sequence numbers but are never
                    // persisted, so the local watermark can sit ahead of
                    // the durable log. Clamp back to the server's durable
                    // max so the next refresh can observe new durable
                    // events again.
                    let mut clamped = false;
                    if let Some(server_latest) = refresh.latest_event_seq
                        && self
                            .transcript
                            .last_event_seq
                            .is_some_and(|local| local > server_latest)
                    {
                        self.transcript.last_event_seq = Some(server_latest);
                        clamped = true;
                    }
                    // A terminal execution is the final durable state for
                    // its reply identity: never drop it merely because the
                    // seq looks stale. Apply it even now (the terminal-aware
                    // staleness check inside `apply_transcript_execution`
                    // still protects a running execution), then drain any
                    // parked message.
                    if let Some(execution) = refresh.execution {
                        let session_id = execution.session.id;
                        let execution_is_terminal =
                            execution.session.state.active_execution().is_none();
                        if self.apply_transcript_execution(execution) {
                            self.sync_pending_interactive_after_execution(session_id);
                            self.sync_session_list_selection_to_current_execution();
                            if execution_is_terminal {
                                self.try_send_pending();
                            }
                        }
                    } else if clamped {
                        // The stale response was an empty "no change"
                        // reply: the durable log had no new events because
                        // live-only events inflated the local watermark.
                        // With the watermark clamped, force one refresh so
                        // the full execution (terminal state + completed
                        // reply) is re-delivered instead of being skipped
                        // forever.
                        self.request_refresh(session_id, true);
                    }
                    if let Some((pending_session_id, force)) = pending {
                        self.request_refresh(pending_session_id, force);
                    }
                    return;
                }
                if let Some(execution) = refresh.execution {
                    let session_id = execution.session.id;
                    let execution_is_terminal =
                        execution.session.state.active_execution().is_none();
                    if self.apply_transcript_execution(execution) {
                        self.sync_pending_interactive_after_execution(session_id);
                        self.sync_session_list_selection_to_current_execution();
                    }
                    // A parked message is delivered when the run completes.
                    // The terminal state usually arrives through this refresh
                    // (the live event only schedules the refresh), so drain
                    // here as well as in the direct execution response path.
                    if execution_is_terminal {
                        self.try_send_pending();
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

        // Re-issue a refresh that arrived while this one was in flight so no
        // event that landed during the refresh window is ever lost.
        if let Some((pending_session_id, force)) = pending {
            self.request_refresh(pending_session_id, force);
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
                // queued message blindly. They can press Ctrl+P to recover
                // the queue contents.
            }
        }
    }

    /// Send the single pending message as the next user turn. Called
    /// whenever an active execution completes (or is successfully cancelled)
    /// so the user's parked message runs automatically.
    pub(crate) fn try_send_pending(&mut self) {
        if self.current_session_activity().is_busy()
            || self.current_session_pending_interactive_kind().is_some()
        {
            return;
        }
        let Some(draft) = self.queue.take() else {
            return;
        };
        // Reuse the normal submit path. We stash it into the editor
        // first so any error path can put the text back in front of the
        // user.
        self.restore_composer_draft(draft);
        self.submit_composer();
    }

    pub(crate) fn handle_turn_cancelled(&mut self, session_id: i64, result: UiResult<()>) {
        self.run_activity.clear_session(session_id);
        if self.transcript.session_id == Some(session_id) {
            self.request_refresh(session_id, true);
        }
        match result {
            Ok(()) => {
                self.flash_info(ui_text::t(&self.i18n, "flash-run-cancelled"));
                // Borrowed from codex's interrupt-and-send flow: cancelling
                // the active run makes queued messages the next user turn.
                // The terminal session event may arrive before or after this
                // handler, so drain from both paths to avoid leaving the
                // queue parked when the ordering is unfavourable.
                self.try_send_pending();
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
        let execution_is_terminal = execution.session.state.active_execution().is_none();
        let transcript_is_target = self.transcript.session_id == Some(session_id);
        if transcript_is_target && self.apply_transcript_execution(execution) {
            self.sync_pending_interactive_after_execution(session_id);
            self.sync_session_list_selection_to_current_execution();
        }
        if refresh && transcript_is_target {
            self.request_refresh(session_id, true);
        }
        self.request_sessions(false);
        if transcript_is_target && execution_is_terminal {
            self.try_send_pending();
        }
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
    App, AppMessage, ComposerDraft, DraftSlot, PendingUserMessage, RunActivityTarget, RunOperation,
    RunOptionsState, SessionExecutionResource, SessionLoadScope, SessionRefresh, SessionResource,
    UiResult, execution_update_is_stale, ui_text,
};
use agena_tui::main_focus::Focus;
