impl App {
    pub(crate) fn request_sessions(&mut self, append: bool) {
        if append {
            return;
        }
        if self.session_load.loading {
            return;
        }
        self.session_load.loading = true;

        let application = self.application.clone();
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
            self.session_load.requested_at = None;
            self.flash_warning(ui_text::t(&self.i18n, "flash-command-requires-session"));
            return;
        }
        self.session_load.pending_scope = Some(scope.clone());
        self.session_load.requested_at = Some(Instant::now());

        tokio::spawn(async move {
            let (result, subtree_root_id) = match scope.mode {
                SessionViewMode::All => (
                    application
                        .list_workspace_sessions(false)
                        .await
                        .map_err(crate::UiFailure::internal),
                    None,
                ),
                SessionViewMode::Roots => (
                    application
                        .list_workspace_sessions(true)
                        .await
                        .map_err(crate::UiFailure::internal),
                    None,
                ),
                SessionViewMode::Subtree => {
                    let anchor_session_id = scope
                        .anchor_session_id
                        .expect("subtree scope requires anchor");
                    let result = application
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
            let _ = tx
                .send(AppMessage::SessionsLoaded {
                    scope,
                    subtree_root_id,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_providers(&mut self, purpose: ProviderPickerPurpose) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_providers()
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::ProvidersLoaded { purpose, result })
                .await;
        });
    }

    pub(crate) fn request_session_search_page(
        &mut self,
        mode: SessionViewMode,
        query: String,
        page_index: usize,
        cursor: Option<String>,
    ) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let is_default_overview = mode == SessionViewMode::All
                && query.trim().is_empty()
                && page_index == 0
                && cursor.is_none();
            let result = if is_default_overview {
                application
                    .session_overview(None, 50)
                    .await
                    .map(|overview| {
                        let mut items = overview.attention;
                        items.extend(overview.running);
                        items.extend(overview.recent);
                        let returned = items.len() as u64;
                        agena_api::pagination::PaginatedResponse {
                            items,
                            page: agena_api::pagination::PageInfo {
                                next_cursor: None,
                                has_more: false,
                                returned,
                            },
                        }
                    })
                    .map_err(crate::UiFailure::internal)
            } else {
                crate::app_backend::operations::list_workspace_sessions_page(
                    &application,
                    mode == SessionViewMode::Roots,
                    mode == SessionViewMode::All,
                    (!query.trim().is_empty()).then_some(query.as_str()),
                    cursor,
                    50,
                )
                .await
                .map_err(crate::UiFailure::internal)
            };
            let _ = tx
                .send(AppMessage::SessionSearchPageLoaded {
                    mode,
                    query,
                    page_index,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_session_search_subtree(&mut self, session_id: i64, query: String) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_session_subtree(session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::SessionSearchSubtreeLoaded {
                    session_id,
                    query,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_lineage(&mut self, session_id: i64) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_session_subtree(session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::LineageLoaded { session_id, result })
                .await;
        });
    }

    pub(crate) fn request_rewind_messages(&mut self, session_id: i64) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                crate::app_backend::operations::get_session_state(&application, session_id)
                    .await
                    .map(|state| rewind_targets_from_parts(&state.parts))
                    .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::RewindMessagesLoaded { session_id, result })
                .await;
        });
    }

    pub(crate) fn request_child_sessions(&mut self, parent_session_id: i64) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_child_sessions(parent_session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::ChildSessionsLoaded {
                    parent_session_id,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_session_rename(&mut self, session_id: i64, title: String) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .rename_session(session_id, title)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::SessionRenamed { session_id, result })
                .await;
        });
    }

    pub(crate) fn request_timeline(&mut self, session_id: i64, limit: u64) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = crate::app_backend::timeline::list_session_timeline(
                &application,
                session_id,
                limit,
            )
            .await
            .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::TimelineLoaded { session_id, result })
                .await;
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

        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .rewind_session_to_turn(session_id, turn_id)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRewound {
                    session_id,
                    message_text,
                    target,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_session_state(&mut self, session_id: i64) {
        if self.transcript.state_loading {
            return;
        }

        self.transcript.state_loading = true;
        self.transcript.state_load_in_flight_since = Some(Instant::now());
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                crate::app_backend::operations::get_session_state(&application, session_id)
                    .await
                    .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionStateLoaded { session_id, result })
                .await;
        });
    }

    /// Park a forced refresh for the periodic tick to consume. `on_tick`
    /// runs one refresh per `REFRESH_INTERVAL_MS`, so a burst of streaming
    /// `PartUpdated` events collapses into a bounded refresh rate; the tick
    /// (or the in-flight refresh completion) re-issues it as a force.
    pub(crate) fn pending_refresh_for(&mut self, session_id: i64) {
        self.pending_refresh = Some((session_id, true));
    }

    pub(crate) fn request_refresh(&mut self, session_id: i64, force: bool) {
        if self.transcript.refreshing {
            // A refresh is already in flight. Remember the request instead of
            // dropping it: the transcript may have advanced past the snapshot
            // the in-flight refresh was built from, and without a follow-up
            // the UI would stall until the next event or a restart. Merge
            // force upward so a permission/terminal event is never demoted.
            let merged_force = match self.pending_refresh {
                Some((_, pending_force)) => force || pending_force,
                None => force,
            };
            self.pending_refresh = Some((session_id, merged_force));
            return;
        }
        self.transcript.refreshing = true;
        self.transcript.refresh_in_flight_since = Some(Instant::now());
        self.last_refresh_at = Instant::now();

        let application = self.application.clone();
        let tx = self.tx.clone();
        let after_seq = self.transcript.last_event_seq;
        tokio::spawn(async move {
            let result = crate::app_backend::session_refresh::refresh_session(
                &application,
                session_id,
                after_seq,
                force,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRefreshed { session_id, result })
                .await;
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
        let application = self.application.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = crate::app_backend::operations::submit_document_with_options(
                &application,
                session_id,
                document,
                options,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionMessageSubmitted {
                    session_id,
                    pending_message_id,
                    draft,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_continue(&mut self, session_id: i64) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::Continue,
        );
        let application = self.application.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = crate::app_backend::operations::continue_session_with_options(
                &application,
                session_id,
                options,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionContinued { session_id, result })
                .await;
        });
    }

    pub(crate) fn request_compact(&mut self, session_id: i64) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::Compact,
        );
        let application = self.application.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        tokio::spawn(async move {
            let result = crate::app_backend::operations::compact_session_with_options(
                &application,
                session_id,
                options,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionCompacted { session_id, result })
                .await;
        });
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
            .and_then(|execution| execution.session.state.active_execution())
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
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result =
                crate::app_backend::operations::cancel_run(&application, session_id, execution_id)
                    .await
                    .and_then(|result| match result {
                        agena_domain::CancellationResult::CancellationRequested
                        | agena_domain::CancellationResult::AlreadyTerminal
                        | agena_domain::CancellationResult::NotFound => Ok(()),
                        agena_domain::CancellationResult::ExecutionMismatch => Err(
                            anyhow::anyhow!("the active execution changed before cancellation"),
                        ),
                    })
                    .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::RunCancelled { session_id, result })
                .await;
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
        let application = self.application.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        let replied_request_id = request_id.clone();
        let replied_kind = kind;
        tokio::spawn(async move {
            let result = crate::app_backend::operations::reply_permission_with_options(
                &application,
                session_id,
                request_id,
                kind,
                scope,
                options,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::PermissionReplied {
                    session_id,
                    request_id: replied_request_id,
                    kind: replied_kind,
                    label,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_user_input_reply(&mut self, session_id: i64, reply: UserInputReply) {
        self.begin_run_operation(
            RunActivityTarget::Session(session_id),
            RunOperation::UserInputReply,
        );
        let application = self.application.clone();
        let tx = self.tx.clone();
        let options = self.run_options.to_request();
        let request_id = reply.request_id.clone();
        tokio::spawn(async move {
            let result = crate::app_backend::operations::reply_user_input_with_options(
                &application,
                session_id,
                reply,
                options,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::UserInputReplied {
                    session_id,
                    request_id,
                    result,
                })
                .await;
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

/// Derive rewind picker targets from the v2 part projection: one target per
/// user `run` marker, with the run's text parts joined as the message preview.
/// Mirrors the v1 turn list (one target per user turn boundary).
fn rewind_targets_from_parts(
    parts: &[agena_api::resource::SessionTranscriptPart],
) -> Vec<crate::RewindTarget> {
    let mut targets = Vec::new();
    let mut sequence = 0i64;
    let mut current_run: Option<&agena_api::resource::SessionTranscriptPart> = None;
    let mut run_text = String::new();
    for part in parts {
        if part.kind == "run" {
            if let Some(marker) = current_run.take()
                && marker.role == "user"
            {
                targets.push(crate::RewindTarget::from_run(marker, sequence, &run_text));
                sequence += 1;
            }
            current_run = Some(part);
            run_text = String::new();
        } else if part.kind == "text"
            && let Some(text) = part.content.get("text").and_then(serde_json::Value::as_str)
        {
            run_text.push_str(text);
            run_text.push('\n');
        }
    }
    if let Some(marker) = current_run
        && marker.role == "user"
    {
        targets.push(crate::RewindTarget::from_run(marker, sequence, &run_text));
    }
    targets
}
