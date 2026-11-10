use std::time::Duration;

use anyhow::Context as _;

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
    ) -> u64 {
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        let request_id = self.next_usage_request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_workspace_sessions_page(
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
                    request_id,
                    mode,
                    query,
                    page_index,
                    result,
                })
                .await;
        });
        request_id
    }

    pub(crate) fn request_session_search_subtree(&mut self, session_id: i64, query: String) -> u64 {
        self.next_usage_request_id = self.next_usage_request_id.saturating_add(1);
        let request_id = self.next_usage_request_id;
        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .list_session_subtree(session_id)
                .await
                .map_err(crate::UiFailure::internal);
            let _ = tx
                .send(AppMessage::SessionSearchSubtreeLoaded {
                    request_id,
                    session_id,
                    query,
                    result,
                })
                .await;
        });
        request_id
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
        mode: agena_domain::RewindMode,
    ) {
        self.sync_current_draft_slot();
        self.persist_draft_store_with_feedback(true);
        self.begin_run_operation(RunActivityTarget::Session(session_id), RunOperation::Rewind);

        let application = self.application.clone();
        let tx = self.tx.clone();
        tokio::spawn(async move {
            let result = application
                .rewind_session_to_message(session_id, at_message_id, mode)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRewound {
                    session_id,
                    message_document,
                    target,
                    mode,
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
            let result = application
                .get_session_state_with_transcript_page(session_id)
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
        fold: agena_tui_transcript::TranscriptFold,
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
            let run_ids = if fold.run_ids.is_empty() {
                vec![fold.run_id]
            } else {
                fold.run_ids.clone()
            };
            let result = application
                .list_session_transcript_fold_page(
                    session_id,
                    &run_ids,
                    reveal_count,
                    cursor.as_str(),
                )
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
        let key = (part_id, section);
        if self.transcript.tool_detail_is_current(part_id, section) {
            return;
        }
        if let Some((_, stamp)) = self.transcript.tool_detail_loads.get(&key) {
            if self
                .transcript
                .parts
                .iter()
                .find(|part| part.part_id == part_id)
                .is_some_and(|part| {
                    stamp != &(part.state.clone(), part.revision, part.updated_at_ms)
                })
            {
                self.transcript.tool_detail_pending.insert(key);
            }
            return;
        }
        if self
            .transcript
            .tool_detail_allowed_at
            .get(&key)
            .is_some_and(|at| *at > Instant::now())
        {
            self.transcript.tool_detail_pending.insert(key);
            return;
        }
        self.transcript.tool_detail_pending.remove(&key);
        let Some(requested_at) = self.transcript.begin_tool_detail_load(part_id, section) else {
            return;
        };
        let application = self.application.clone();
        let tx = self.tx.clone();
        let task = tokio::spawn(async move {
            let result = tokio::time::timeout(
                Duration::from_secs(30),
                application.get_tool_detail(session_id, part_id, section),
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
        self.transcript
            .tool_detail_tasks
            .insert(key, task.abort_handle());
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
        let in_flight_ids = self
            .transcript
            .parts
            .iter()
            .filter(|part| {
                part.role == "assistant"
                    && matches!(
                        part.run_state.as_deref().unwrap_or(&part.state),
                        "pending" | "running" | "in_progress"
                    )
            })
            .map(|part| part.part_id)
            .collect::<Vec<_>>();
        tokio::spawn(async move {
            let result = async {
                let mut refresh = crate::app_backend::session_refresh::refresh_session(
                    &application,
                    session_id,
                    after_seq,
                    force,
                )
                .await?;
                let mut known_ids = known_ids;
                if let Some(snapshot) = &refresh.snapshot {
                    // Consecutive queued executions can move an unfinished
                    // cached reply out of the recent window between polls.
                    // Read its final facts instead of leaving a stale spinner.
                    let window_ids = snapshot
                        .page
                        .parts
                        .iter()
                        .map(|part| part.part_id)
                        .collect::<std::collections::BTreeSet<_>>();
                    known_ids.extend(
                        in_flight_ids
                            .into_iter()
                            .filter(|id| !window_ids.contains(id)),
                    );
                    known_ids.sort_unstable();
                    known_ids.dedup();
                }
                if !known_ids.is_empty() {
                    let mut parts = Vec::new();
                    let mut reconciled_ids = Vec::new();
                    for ids in known_ids.chunks(256) {
                        match application
                            .client()
                            .session_parts_by_ids(session_id, ids)
                            .await
                        {
                            Ok(resource) => {
                                reconciled_ids.extend_from_slice(ids);
                                parts.extend(resource.parts);
                            }
                            Err(error) => {
                                refresh.reconciliation_error = Some(error.to_string());
                                break;
                            }
                        }
                    }
                    // Only successful membership reads may confirm removals.
                    // An auxiliary failure must not discard the snapshot or
                    // earlier chunks that already supplied usable parts.
                    refresh.reconciled_parts = Some((reconciled_ids, parts));
                }
                Ok::<_, anyhow::Error>(refresh)
            }
            .await
            .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionRefreshed {
                    session_id,
                    requested_at,
                    result: Box::new(result),
                })
                .await;
        });
    }

    pub(crate) fn request_submit_message(&mut self, session_id: i64, draft: ComposerDraft) {
        if self.transcript.session_id == Some(session_id) {
            self.transcript.scroll_to_bottom(
                self.layout.transcript_body.width,
                self.layout.transcript_body.height,
            );
        }
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
            let result = application
                .submit_document(session_id, document, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionMessageSubmitted {
                    session_id,
                    draft,
                    result,
                })
                .await;
        });
    }

    pub(crate) fn request_first_queued_message_action(&mut self, edit: bool) -> bool {
        let Some(execution) = self.transcript.execution.as_ref() else {
            return false;
        };
        let Some(queued) = execution.queued_messages.first() else {
            return false;
        };
        let session_id = execution.session.id;
        let execution_id = agena_domain::ExecutionId(queued.execution_id);
        if edit {
            self.request_edit_queued_message(session_id, execution_id);
        } else {
            self.request_delete_queued_message(session_id, execution_id);
        }
        true
    }

    pub(crate) fn request_delete_queued_message(
        &mut self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) {
        self.dispatch_backend_operation(
            move |application| async move {
                application
                    .client()
                    .delete_queued_message(session_id, execution_id)
                    .await
            },
            move |app, result| match result {
                Ok(state) => {
                    app.handle_session_execution_updated(session_id, state, true);
                    app.request_refresh(session_id, true);
                }
                Err(error) => {
                    app.flash_error(error);
                    app.request_refresh(session_id, true);
                }
            },
        );
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
            let result = application
                .continue_session(session_id, options)
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
            let result = application
                .compact_session(session_id, options)
                .await
                .map_err(crate::UiFailure::from_backend);
            let _ = tx
                .send(AppMessage::SessionCompacted { session_id, result })
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
            let result = application
                .reply_permission(
                    session_id,
                    options,
                    agena_api::resource::PermissionReply {
                        request_id,
                        kind: kind.into(),
                        scope: scope.map(Into::into),
                        reason: None,
                    },
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
            let result = application
                .reply_user_input(session_id, options, reply.into())
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
    App, AppMessage, ComposerDraft, DraftSlot, Instant, PermissionReplyKind, PermissionScope,
    ProviderPickerPurpose, RunActivityTarget, RunOperation, SessionLoadScope, UserInputReply,
    ui_text,
};
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

/// One target per completed user message. Its composer document owns both
/// ordering and identity for text, resources, commands, and pasted snippets.
fn rewind_targets_from_parts(
    parts: &[agena_api::part::PartResource],
) -> anyhow::Result<Vec<crate::RewindTarget>> {
    parts
        .iter()
        .filter(|part| part.is_message_boundary() && part.role == "user")
        .enumerate()
        .filter(|(_, marker)| marker.state == "completed")
        .map(|(index, marker)| {
            let document = marker.content.get("document").ok_or_else(|| {
                anyhow::anyhow!(
                    "user message {} is missing its composer document",
                    marker.part_id
                )
            })?;
            let document =
                serde_json::from_value::<agena_domain::ComposerDocument>(document.clone())
                    .context("invalid user composer document in rewind history")?;
            Ok(crate::RewindTarget::from_run(
                marker,
                index as i64 + 1,
                document,
            ))
        })
        .collect()
}

#[cfg(test)]
mod rewind_document_tests {
    use super::rewind_targets_from_parts;
    use agena_api::part::PartResource;
    use agena_domain::{ActivityId, ComposerDocument};
    use serde_json::{Value, json};

    fn user_message(id: i64, state: &str, document: Value) -> PartResource {
        PartResource {
            part_id: id,
            run_id: Some(id),
            message_id: Some(id),
            message_boundary: true,
            kind: "text".into(),
            role: "user".into(),
            state: state.into(),
            content: json!({"text":"display text is not a reconstruction source","document":document}),
            ..Default::default()
        }
    }

    #[test]
    fn rewind_preserves_document_order_identities_and_multiline_input() {
        let source = json!([
            {"type":"text","text":"First line\nSecond line"},
            {"type":"activity","activity":{"id":ActivityId::new(),"payload":{
                "activity_type":"resource","kind":"image","name":"picture.png","delivery":"model_input",
                "media_type":"image/png","size_bytes":8,"width":4,"height":2,
                "reference":{"reference_type":"artifact","uri":"agena:media:v1:fixture","sha256":"a".repeat(64)}
            }}},
            {"type":"activity","activity":{"id":ActivityId::new(),"payload":{
                "activity_type":"command_reference","name":"review","description":"Review changes",
                "content_hash":"command-revision","source":"agena.skills","skill_id":"skill:review","aliases":["r"]
            }}},
            {"type":"activity","activity":{"id":ActivityId::new(),"payload":{
                "activity_type":"text_artifact","label":"snippet","language":"rust","text":"fn main() {}\n"
            }}},
            {"type":"text","text":"Last line"}
        ]);
        let expected: ComposerDocument = serde_json::from_value(source.clone()).unwrap();
        let mut assistant = user_message(2, "completed", json!([]));
        assistant.role = "assistant".into();
        let parts = [
            user_message(1, "completed", source),
            assistant,
            user_message(3, "pending", json!([])),
            user_message(
                4,
                "completed",
                json!([{"type":"text","text":"Another\nmessage"}]),
            ),
        ];
        let targets = rewind_targets_from_parts(&parts).unwrap();
        assert_eq!(targets.len(), 2);
        assert_eq!(targets[0].at_message_id, 1);
        assert_eq!(targets[0].sequence, 1);
        assert_eq!(targets[0].message_document, expected);
        assert_eq!(targets[1].at_message_id, 4);
        assert_eq!(targets[1].sequence, 3);
        assert_eq!(targets[1].message_document.text(), "Another\nmessage");
    }

    #[test]
    fn rewind_rejects_missing_or_malformed_documents_without_guessing_old_parts() {
        let mut missing = user_message(1, "completed", json!([]));
        missing.content = json!({"text":"old text","attachments":[{"source":{"path":"old.png"}}]});
        let error = rewind_targets_from_parts(&[missing]).err().unwrap();
        assert!(error.to_string().contains("missing its composer document"));

        let malformed = user_message(2, "completed", json!({"text":"not a document"}));
        let error = rewind_targets_from_parts(&[malformed]).err().unwrap();
        assert!(error.to_string().contains("invalid user composer document"));
    }
}
