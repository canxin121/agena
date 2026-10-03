use std::time::Duration;

impl App {
    pub(crate) fn request_sessions(&mut self, append: bool) {
        if append {
            return;
        }
        if self.session_load.loading {
            self.session_load.refresh_queued = true;
            return;
        }
        self.session_load.refresh_queued = false;
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
        let requested_at = Instant::now();
        self.session_load.requested_at = Some(requested_at);

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
                    requested_at,
                    scope,
                    subtree_root_id,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_session_favorite(&mut self, session_id: i64, favorite: bool) {
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .set_session_favorite(session_id, favorite)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::SessionFavoriteUpdated { session_id, result })
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
            let result = crate::app_backend::operations::list_workspace_sessions_page(
                &application,
                mode == SessionViewMode::Roots,
                mode == SessionViewMode::All,
                (!query.trim().is_empty()).then_some(query.as_str()),
                cursor,
                50,
            )
            .await
            .map_err(crate::UiFailure::internal);
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
            let result = load_rewind_targets(&application, session_id)
                .await
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
        at_message_id: i64,
        message_document: agena_domain::ComposerDocument,
        target: String,
    ) {
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        self.begin_run_operation(RunActivityTarget::Session(session_id), RunOperation::Rewind);

        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .rewind_session_to_message(session_id, at_message_id)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRewound {
                    session_id,
                    message_document,
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
        let requested_at = Instant::now();
        self.transcript.state_load_in_flight_since = Some(requested_at);
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = crate::app_backend::operations::get_session_state_with_transcript_page(
                &application,
                session_id,
            )
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionStateLoaded {
                    session_id,
                    requested_at,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_older_transcript_parts_if_needed(&mut self) {
        if self.transcript.viewport_top() > 3
            || self.transcript.transcript_older_loading
            || !self.transcript.transcript_has_more
            || self.transcript.last_history_load_at.is_some_and(|at| {
                at.elapsed()
                    < Duration::from_millis(if self.transcript.transcript_older_error.is_some() {
                        2_000
                    } else {
                        250
                    })
            })
        {
            return;
        }
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        let Some(cursor) = self.transcript.transcript_next_cursor.clone() else {
            return;
        };
        self.transcript.transcript_older_loading = true;
        self.transcript.transcript_older_error = None;
        let requested_at = Instant::now();
        self.transcript.transcript_older_in_flight_since = Some(requested_at);
        self.transcript.last_history_load_at = Some(requested_at);

        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_session_transcript_page(
                    session_id,
                    crate::app_backend::OLDER_TRANSCRIPT_PAGE_SIZE,
                    cursor.as_str(),
                )
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::TranscriptPartsLoaded {
                    session_id,
                    requested_at,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_transcript_fold_parts(
        &mut self,
        fold: agena_api::live::SessionTranscriptFoldResource,
        expand_all: bool,
        reveal_count: usize,
    ) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        let Some(cursor) = fold.next_cursor else {
            return;
        };
        if self
            .transcript
            .transcript_fold_loads
            .keys()
            .any(|(run_id, _)| *run_id == fold.run_id || fold.run_ids.contains(run_id))
        {
            return;
        }
        let requested_at = Instant::now();
        self.transcript
            .transcript_fold_loads
            .insert((fold.run_id, fold.anchor_part_id), requested_at);
        self.transcript.transcript_fold_errors.remove(&fold.run_id);
        self.transcript.invalidate_render();
        let application = self.application.clone();
        let tx = self.tx.clone();
        let reveal_count = reveal_count.clamp(1, 50) as u64;
        tokio::spawn(async move {
            let result = application
                .list_session_transcript_fold_page(session_id, reveal_count, cursor.as_str())
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::TranscriptFoldPartsLoaded {
                    session_id,
                    requested_at,
                    run_id: fold.run_id,
                    anchor_part_id: fold.anchor_part_id,
                    expand_all,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_tool_detail(
        &mut self,
        part_id: i64,
        section: agena_api::live::ToolDetailSection,
    ) {
        let Some(session_id) = self.transcript.session_id else {
            return;
        };
        let Some(requested_at) = self.transcript.begin_tool_detail_load(part_id, section) else {
            return;
        };
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                crate::app_backend::operations::get_tool_detail(
                    &application,
                    session_id,
                    part_id,
                    section,
                ),
            )
            .await
            .map_err(|_| crate::UiFailure::internal("Tool detail request timed out"))
            .and_then(|result| result.map_err(crate::UiFailure::from_backend));
            let _ = tx
                .send(AppMessage::ToolDetailLoaded {
                    session_id,
                    part_id,
                    section,
                    requested_at,
                    result,
                })
                .await;
        });
    }

    /// Park a forced refresh for the periodic tick to consume. `on_tick`
    /// runs one refresh per `REFRESH_INTERVAL_MS`, so a burst of streaming
    /// `PartUpdated` events collapses into a bounded refresh rate; the tick
    /// re-issues it as a force after the current request has completed.
    pub(crate) fn pending_refresh_for(&mut self, session_id: i64) {
        self.pending_refresh = Some((session_id, true));
    }

    pub(crate) fn request_refresh(&mut self, session_id: i64, force: bool) {
        if self.transcript.session_id != Some(session_id) {
            return;
        }
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
        let requested_at = Instant::now();
        self.transcript.refresh_in_flight_since = Some(requested_at);
        self.last_refresh_at = Instant::now();

        let application = self.application.clone();
        let tx = self.tx.clone();
        let after_seq = self.transcript.last_event_seq;
        let known_ids = if std::mem::take(&mut self.transcript.reconcile_loaded_parts) {
            self.transcript
                .parts
                .iter()
                .map(|part| part.part_id)
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        tokio::spawn(async move {
            let result = async {
                let mut refresh = crate::app_backend::session_refresh::refresh_session(
                    &application,
                    session_id,
                    after_seq,
                    force,
                )
                .await?;
                if !known_ids.is_empty() {
                    let mut parts = Vec::new();
                    for ids in known_ids.chunks(256) {
                        parts.extend(
                            application
                                .client()
                                .session_parts_by_ids(session_id, ids)
                                .await?
                                .parts
                                .into_iter()
                                .map(agena_api::resource::SessionTranscriptPart::from),
                        );
                    }
                    refresh.reconciled_parts = Some((known_ids, parts));
                }
                Ok::<_, anyhow::Error>(refresh)
            }
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRefreshed {
                    session_id,
                    requested_at,
                    result,
                })
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
            // Only the displayed transcript gets the ghost row and the scroll:
            // a send queued for another session must not draw into the session the
            // user is reading, and that session own view confirms it later.
            existing_pending_message_id.unwrap_or_else(|| {
                if self.transcript.session_id == Some(session_id) {
                    self.begin_pending_user_message(&draft)
                } else {
                    0
                }
            });
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
        let execution_id = self
            .transcript
            .execution
            .as_ref()
            .and_then(|execution| execution.session.state.active_execution())
            .map(|execution| agena_domain::ExecutionId(execution.execution_id));
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
                    .await;
            let result = match result {
                Ok(outcome)
                    if outcome.result == agena_domain::CancellationResult::ExecutionMismatch =>
                {
                    // The stop action is user intent for the current session,
                    // not a delayed command that should preserve a newer
                    // execution. Retry through the session-scoped endpoint so
                    // a short notification wake cannot slip through the exact
                    // execution-id race.
                    crate::app_backend::operations::cancel_run(&application, session_id, None).await
                }
                other => other,
            }
            .and_then(|outcome| match outcome.result {
                agena_domain::CancellationResult::CancellationRequested
                | agena_domain::CancellationResult::AlreadyTerminal
                | agena_domain::CancellationResult::NotFound => Ok(outcome),
                agena_domain::CancellationResult::ExecutionMismatch => Err(anyhow::anyhow!(
                    "the active execution changed before cancellation"
                )),
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

/// Rewind is an explicit history operation. Walk the collapsed transcript
/// pages so the picker includes older messages without expanding tool output.
async fn load_rewind_targets(
    application: &crate::app_backend::TuiBackend,
    session_id: i64,
) -> anyhow::Result<Vec<crate::RewindTarget>> {
    let initial = application
        .get_session_state_with_transcript_page(session_id)
        .await?;
    let mut page = initial.page;
    let mut parts = Vec::new();
    let mut cursors = std::collections::HashSet::new();
    loop {
        parts.extend(page.parts);
        if !page.has_more {
            break;
        }
        let cursor = page
            .next_cursor
            .filter(|cursor| !cursor.is_empty())
            .ok_or_else(|| anyhow::anyhow!("rewind history page is missing its next cursor"))?;
        if !cursors.insert(cursor.clone()) {
            anyhow::bail!("rewind history cursor did not advance");
        }
        page = application
            .list_session_transcript_page(session_id, 12, &cursor)
            .await?;
    }
    parts.sort_by_key(|part| (part.created_at_ms, part.part_id));
    parts.dedup_by_key(|part| part.part_id);
    rewind_targets_from_parts(&parts)
}

/// One target per completed user run marker. Text is selected by run_id,
/// which also handles interleaved runs and preserves multiline input.
fn rewind_targets_from_parts(
    parts: &[agena_api::resource::SessionTranscriptPart],
) -> anyhow::Result<Vec<crate::RewindTarget>> {
    let mut inputs_by_run = std::collections::HashMap::<i64, Vec<_>>::new();
    for part in parts {
        if part.role == "user"
            && let Some(run_id) = part.run_id
        {
            inputs_by_run.entry(run_id).or_default().push(part);
        }
    }
    parts
        .iter()
        .filter(|part| part.kind == "run" && part.role == "user")
        .enumerate()
        .filter(|(_, marker)| marker.state == "completed")
        .map(|(index, marker)| {
            let document = inputs_by_run
                .get(&marker.part_id)
                .into_iter()
                .flatten()
                .map(|part| rewind_composer_node(part))
                .collect::<anyhow::Result<Vec<_>>>()?;
            Ok(crate::RewindTarget::from_run(
                marker,
                index as i64 + 1,
                agena_domain::ComposerDocument(document),
            ))
        })
        .collect()
}

/// Rebuild ordered composer input from the canonical user payloads. A rewind
/// must retain resources and command references, as well as multiline text.
fn rewind_composer_node(
    part: &agena_api::resource::SessionTranscriptPart,
) -> anyhow::Result<agena_domain::ComposerNode> {
    use agena_domain::{
        ActivityId, ActivityPayload, ActivityProvenance, ComposerActivity, ComposerNode,
        ResourceDelivery, ResourceKind, ResourceReference,
    };
    let content = &part.content;
    if let Some(text) = content.get("text").and_then(serde_json::Value::as_str) {
        return Ok(ComposerNode::Text {
            text: text.to_owned(),
        });
    }
    let payload = if part.kind == "skill_ref" {
        let command = content
            .get("skills")
            .and_then(serde_json::Value::as_array)
            .and_then(|commands| commands.first())
            .ok_or_else(|| {
                anyhow::anyhow!("user command reference {} has no command", part.part_id)
            })?;
        ActivityPayload::CommandReference(serde_json::from_value(command.clone())?)
    } else {
        let attachment = content
            .get("attachments")
            .and_then(serde_json::Value::as_array)
            .and_then(|items| items.first());
        let source = attachment
            .and_then(|item| item.get("source"))
            .or_else(|| content.get("source"));
        let path = content
            .get("path")
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                source
                    .and_then(|source| source.get("path"))
                    .and_then(serde_json::Value::as_str)
            });
        let reference = if let Some(path) = path {
            ResourceReference::WorkspacePath {
                path: path.to_owned(),
            }
        } else if let Some(url) = source
            .and_then(|source| source.get("url"))
            .or_else(|| content.get("data_url"))
            .and_then(serde_json::Value::as_str)
        {
            ResourceReference::Url {
                url: url.to_owned(),
            }
        } else {
            anyhow::bail!(
                "user resource {} has no recoverable reference",
                part.part_id
            );
        };
        let mime = content
            .get("mime")
            .or_else(|| attachment.and_then(|item| item.get("mime")))
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default();
        let kind = if mime.starts_with("image/") {
            ResourceKind::Image
        } else if mime.starts_with("audio/") {
            ResourceKind::Audio
        } else if mime.starts_with("video/") {
            ResourceKind::Video
        } else if mime == "application/pdf" {
            ResourceKind::Pdf
        } else if mime == "inode/directory" {
            ResourceKind::Directory
        } else {
            ResourceKind::File
        };
        ActivityPayload::Resource(agena_domain::ResourceActivity {
            delivery: if content.get("delivery").and_then(serde_json::Value::as_str)
                == Some("model_input")
            {
                ResourceDelivery::ModelInput
            } else {
                ResourceDelivery::Reference
            },
            kind,
            reference,
            name: content
                .get("name")
                .or_else(|| attachment.and_then(|item| item.get("filename")))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("resource")
                .to_owned(),
            media_type: (!mime.is_empty()).then(|| mime.to_owned()),
            size_bytes: content
                .get("size_bytes")
                .and_then(serde_json::Value::as_u64),
            width: content
                .get("width")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| value.try_into().ok()),
            height: content
                .get("height")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| value.try_into().ok()),
            duration_ms: content
                .get("duration_ms")
                .and_then(serde_json::Value::as_u64),
            page_count: content
                .get("page_count")
                .and_then(serde_json::Value::as_u64)
                .and_then(|value| value.try_into().ok()),
        })
    };
    Ok(ComposerNode::activity(ComposerActivity {
        id: ActivityId::new(),
        payload,
        provenance: ActivityProvenance {
            content_hash: content
                .get("sha")
                .and_then(serde_json::Value::as_str)
                .map(str::to_owned),
            ..Default::default()
        },
    }))
}

#[cfg(test)]
mod rewind_tests {
    use super::rewind_targets_from_parts;
    use agena_api::resource::SessionTranscriptPart;
    use agena_domain::{ActivityPayload, ComposerNode, ResourceDelivery, ResourceReference};
    use serde_json::json;

    fn part(
        id: i64,
        kind: &str,
        role: &str,
        state: &str,
        run_id: Option<i64>,
        content: serde_json::Value,
    ) -> SessionTranscriptPart {
        SessionTranscriptPart {
            revision: 0,
            updated_at_ms: 0,
            part_id: id,
            kind: kind.to_owned(),
            role: role.to_owned(),
            state: state.to_owned(),
            content,
            presentation: None,
            summary: None,
            created_at_ms: id,
            parent_part_id: None,
            run_id,
        }
    }

    #[test]
    fn rewind_picker_uses_real_message_ids_and_restores_ordered_resources() {
        let parts = vec![
            part(
                10,
                "run",
                "user",
                "completed",
                None,
                json!({"run_kind": "user_send"}),
            ),
            part(
                11,
                "text",
                "user",
                "completed",
                Some(10),
                json!({"text": "before "}),
            ),
            part(
                12,
                "file_ref",
                "user",
                "completed",
                Some(10),
                json!({"path": "input.png", "name": "input.png", "mime": "image/png", "delivery": "model_input"}),
            ),
            part(
                13,
                "run",
                "assistant",
                "completed",
                None,
                json!({"turn_id": "b673e234-d6ae-4d27-8314-c3f9c4da8d5c"}),
            ),
            // A delayed user part still belongs to its run, regardless of an interleaved marker.
            part(
                14,
                "text",
                "user",
                "completed",
                Some(10),
                json!({"text": " after\nsecond line"}),
            ),
            part(
                15,
                "text",
                "assistant",
                "completed",
                Some(13),
                json!({"text": "assistant output"}),
            ),
            part(
                16,
                "run",
                "user",
                "pending",
                None,
                json!({"run_kind": "user_send"}),
            ),
        ];
        let targets = rewind_targets_from_parts(&parts).unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].at_message_id, 10);
        assert_eq!(targets[0].sequence, 1);
        assert_eq!(
            targets[0].message_document.text(),
            "before  after\nsecond line"
        );
        let nodes = &targets[0].message_document.0;
        assert_eq!(nodes.len(), 3);
        let ComposerNode::Activity { activity } = &nodes[1] else {
            panic!("resource must remain inline")
        };
        let ActivityPayload::Resource(resource) = &activity.payload else {
            panic!("expected resource")
        };
        assert_eq!(resource.delivery, ResourceDelivery::ModelInput);
        assert_eq!(
            resource.reference,
            ResourceReference::WorkspacePath {
                path: "input.png".to_owned()
            }
        );
    }

    #[test]
    fn rewind_restores_media_paths_and_message_scoped_command_references() {
        let parts = vec![
            part(
                1,
                "run",
                "user",
                "completed",
                None,
                json!({"run_kind": "user_send"}),
            ),
            part(
                2,
                "file_ref",
                "user",
                "completed",
                Some(1),
                json!({
                    "path": ".agena/uploads/image.png", "name": "image.png", "mime": "image/png",
                    "sha": "input-hash", "delivery": "model_input",
                    "attachments": [{"kind": "image", "mime": "image/png", "source": {
                        "source": "provider_data", "route": "original-route", "data": "aGVsbG8="
                    }}]
                }),
            ),
            part(
                3,
                "skill_ref",
                "user",
                "completed",
                Some(1),
                json!({
                    "command": "review", "skills": [{"name": "review", "description": "Review code",
                        "content_hash": "command-hash", "source": "agena.commands", "aliases": []}]
                }),
            ),
            part(
                4,
                "file_ref",
                "user",
                "completed",
                Some(1),
                json!({
                    "name": "reference", "source": {"source": "url", "url": "https://example.test/reference"}
                }),
            ),
        ];
        let targets = rewind_targets_from_parts(&parts).unwrap();
        let nodes = &targets[0].message_document.0;
        assert_eq!(nodes.len(), 3);
        let ComposerNode::Activity { activity: media } = &nodes[0] else {
            panic!("media reference")
        };
        let ActivityPayload::Resource(resource) = &media.payload else {
            panic!("media payload")
        };
        assert_eq!(
            resource.reference,
            ResourceReference::WorkspacePath {
                path: ".agena/uploads/image.png".to_owned()
            }
        );
        assert_eq!(resource.delivery, ResourceDelivery::ModelInput);
        assert_eq!(media.provenance.content_hash.as_deref(), Some("input-hash"));
        let ComposerNode::Activity { activity: command } = &nodes[1] else {
            panic!("command reference")
        };
        let ActivityPayload::CommandReference(reference) = &command.payload else {
            panic!("command payload")
        };
        assert_eq!(reference.name, "review");
        assert_eq!(reference.content_hash, "command-hash");
        assert_eq!(reference.source, "agena.commands");
        let ComposerNode::Activity { activity: link } = &nodes[2] else {
            panic!("URL reference")
        };
        let ActivityPayload::Resource(resource) = &link.payload else {
            panic!("URL payload")
        };
        assert_eq!(
            resource.reference,
            ResourceReference::Url {
                url: "https://example.test/reference".to_owned()
            }
        );
        agena_application::session::validate_input_document(&targets[0].message_document).unwrap();
    }
}
