use super::{ExecutionControl, ExecutionControlError, execution_control_to_app_error};
use crate::{
    AppError,
    session::{
        Session, SessionManager,
        store::{
            OPERATION_ID_METADATA_KEY, execution_status_from_part_state, parts_into_runs,
            role_from_part_role, timestamp_millis_to_utc, typed_content_from_value,
            typed_content_to_value,
        },
    },
};
use agena_domain::{
    CancellationOutcome, CancellationResult, ComposerDocument, ExecutionStatus, SessionSummary,
    TurnId,
};
use agena_plugin_host::AgentCancelInput;
use agena_runtime::{SessionForkRequest, SessionRewindRequest};
use agena_runtime_contracts::part_content::{
    TypedContent, attachment_from_file_ref, operation_from_tool_call,
    skill_reference_from_skill_ref, user_problem_from_error,
};
use agena_storage::store::{Part, PartRole};

impl SessionManager {
    pub async fn fork_session(&self, request: SessionForkRequest) -> Result<Session, AppError> {
        let source = self.store.load_session(request.session_id).await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        // The public request names a message (its run-marker part id), while
        // storage forks at an inclusive part boundary. Resolve a marker to the
        // message's final member part so the fork includes the entire message,
        // not only its marker. A literal part id that is not a message marker
        // remains a valid precise cutoff for internal callers.
        let at_part_id = match request.at_message_id {
            Some(part_id) => last_part_id_for_run_marker(source.parts(), part_id),
            None => last_part_id_for_last_run(source.parts()),
        }
        .ok_or_else(|| {
            AppError::Internal(format!(
                "cannot fork session {}: it has no message to use as the cutoff",
                request.session_id
            ))
        })?;
        let title = request
            .title
            .unwrap_or_else(|| format!("Fork of {}", source.title));
        let child_id = self.store.fork(source.id, at_part_id, title).await?;
        self.store.load_session(child_id).await
    }

    /// Session-scoped cancellation with optional recovery of the original
    /// composer document when the new user turn never produced assistant
    /// output.
    pub async fn cancel_active_execution_with_outcome(
        &self,
        session_id: i64,
    ) -> Result<CancellationOutcome, AppError> {
        let root_control = self
            .execution_registry
            .execution_control(session_id, None)
            .await;
        let root_execution_id = self
            .execution_registry
            .execution(session_id)
            .await
            .and_then(|lifecycle| match lifecycle {
                agena_domain::ExecutionLifecycle::Active { execution_id, .. } => {
                    Some(execution_id.to_string())
                }
                agena_domain::ExecutionLifecycle::Terminal { .. } => None,
            });
        // Signal the requested execution before any database traversal. This
        // keeps Ctrl+C latency independent of session-tree size and storage
        // contention; descendant discovery continues after the active model
        // stream or tool has already received cancellation.
        let root_result = self.execution_registry.cancel_current(session_id).await;
        if let Some(execution_id) = root_execution_id {
            self.execution_state()
                .tool_executor
                .plugin_manager()
                .dispatch_agent_cancel(AgentCancelInput {
                    session_id,
                    execution_id,
                })
                .await;
        } else {
            // A queued delivery can be the only remaining work after a short
            // execution has already terminalized. Still fire the cancellation
            // hook so execution-local automation such as plan autorun is
            // disabled by an explicit user stop.
            self.execution_state()
                .tool_executor
                .plugin_manager()
                .dispatch_agent_cancel(AgentCancelInput {
                    session_id,
                    execution_id: "session-cancel".to_owned(),
                })
                .await;
        }
        self.cancel_host_interactive_waiters(session_id).await;
        let cancellation_order = match self.store.load_session(session_id).await {
            Ok(session) => {
                let tree = self.store.list_session_tree(session.root_id).await?;
                descendant_cancellation_order(session_id, session_tree_domain(tree)?.as_slice())
            }
            Err(error) => {
                tracing::warn!(
                    session_id,
                    diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                        "failed to load the session tree while cancelling; continuing with the requested session only",
                        &error,
                    ),
                    "descendant session cancellation could not be planned"
                );
                vec![session_id]
            }
        };

        let root_result_kind = match &root_result {
            Ok(()) => Some(CancellationResult::CancellationRequested),
            Err(ExecutionControlError::NoActiveExecution(_)) => Some(CancellationResult::NotFound),
            Err(..) => None,
        };
        let mut first_error = cancel_active_execution_result(root_result).err();
        if let Err(error) = self
            .store
            .fail_pending_background_deliveries(
                session_id,
                serde_json::json!({
                    "category": "cancelled",
                    "message": "background notification delivery cancelled with the session execution",
                }),
            )
            .await
            && first_error.is_none()
        {
            first_error = Some(error);
        }
        for target_id in cancellation_order
            .into_iter()
            .filter(|target_id| *target_id != session_id)
        {
            let result = self.execution_registry.cancel_current(target_id).await;
            if let Err(error) = self
                .store
                .fail_pending_background_deliveries(
                    target_id,
                    serde_json::json!({
                        "category": "cancelled",
                        "message": "background notification delivery cancelled with the session execution",
                    }),
                )
                .await
                && first_error.is_none()
            {
                first_error = Some(error);
            }
            // A plugin-hosted tool can be suspended in a host permission or
            // user-input callback. A cancellation token is only observed
            // between run-loop iterations, so release those one-shot waiters
            // as well; otherwise Ctrl+C leaves the executor blocked forever.
            self.cancel_host_interactive_waiters(target_id).await;
            if let Err(error) = cancel_active_execution_result(result)
                && first_error.is_none()
            {
                first_error = Some(error);
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }
        let result = root_result_kind.unwrap_or(CancellationResult::NotFound);
        let mut outcome = CancellationOutcome::from(result);
        if matches!(result, CancellationResult::CancellationRequested)
            && let Some(control) = root_control
        {
            let (document, run_id) = self
                .restore_unanswered_user_run(session_id, control)
                .await?;
            outcome.restored_user_message = document;
            outcome.restored_user_run_id = run_id;
        }
        Ok(outcome)
    }

    /// Exact external cancellation with optional recovery of the matching
    /// user execution. A mismatched request remains a strict no-op.
    pub async fn cancel_execution_with_outcome(
        &self,
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
    ) -> Result<CancellationOutcome, AppError> {
        let control = self
            .execution_registry
            .execution_control(session_id, Some(execution_id))
            .await;
        let result = self
            .execution_registry
            .cancel_exact(session_id, execution_id)
            .await
            .map_err(execution_control_to_app_error)?;
        if result != agena_domain::CancellationResult::CancellationRequested {
            // A stop request can arrive in the tiny gap after a short
            // notification execution has already terminalized. There is no
            // execution token left to cancel, but queued background wakes must
            // still be suppressed; otherwise the next delivery immediately
            // starts another execution. An execution mismatch remains a
            // strict no-op so a delayed request cannot cancel a newer user
            // turn.
            if matches!(
                result,
                agena_domain::CancellationResult::AlreadyTerminal
                    | agena_domain::CancellationResult::NotFound
            ) && let Err(error) = self
                .store
                .fail_pending_background_deliveries(
                    session_id,
                    serde_json::json!({
                        "category": "cancelled",
                        "message": "queued background notification delivery cancelled after the execution ended",
                    }),
                )
                .await
            {
                tracing::warn!(
                    target: "agena_background",
                    session_id,
                    %error,
                    "failed to suppress queued background deliveries after terminal cancellation"
                );
            }
            if matches!(
                result,
                agena_domain::CancellationResult::AlreadyTerminal
                    | agena_domain::CancellationResult::NotFound
            ) {
                self.execution_state()
                    .tool_executor
                    .plugin_manager()
                    .dispatch_agent_cancel(AgentCancelInput {
                        session_id,
                        execution_id: execution_id.to_string(),
                    })
                    .await;
            }
            let mut outcome = CancellationOutcome::from(result);
            if result == CancellationResult::AlreadyTerminal
                && let Some(control) = control
            {
                let (document, run_id) = self
                    .restore_unanswered_user_run(session_id, control)
                    .await?;
                outcome.restored_user_message = document;
                outcome.restored_user_run_id = run_id;
            }
            return Ok(outcome);
        }
        // The execution token is already in the cancellation state. Notify
        // plugins after that control decision so a hook can clear
        // execution-local automation (notably plan autorun) without ever
        // being able to prevent the actual cancellation.
        self.execution_state()
            .tool_executor
            .plugin_manager()
            .dispatch_agent_cancel(AgentCancelInput {
                session_id,
                execution_id: execution_id.to_string(),
            })
            .await;
        self.cancel_host_interactive_waiters(session_id).await;
        if let Err(error) = self
            .store
            .fail_pending_background_deliveries(
                session_id,
                serde_json::json!({
                    "category": "cancelled",
                    "message": "background notification delivery cancelled with the session execution",
                }),
            )
            .await
        {
            tracing::warn!(
                target: "agena_background",
                session_id,
                %error,
                "failed to suppress queued background deliveries during cancellation"
            );
        }

        if let Ok(session) = self.store.load_session(session_id).await {
            let tree = self.store.list_session_tree(session.root_id).await?;
            for target_id in
                descendant_cancellation_order(session_id, session_tree_domain(tree)?.as_slice())
                    .into_iter()
                    .filter(|target_id| *target_id != session_id)
            {
                if let Err(error) = self.execution_registry.cancel_current(target_id).await {
                    tracing::warn!(
                        session_id = target_id,
                        diagnostic = %agena_failure::diagnostic::format_error_chain(&error),
                        "failed to cancel a descendant execution while cancelling its session tree"
                    );
                }
                self.cancel_host_interactive_waiters(target_id).await;
                if let Err(error) = self
                    .store
                    .fail_pending_background_deliveries(
                        target_id,
                        serde_json::json!({
                            "category": "cancelled",
                            "message": "background notification delivery cancelled with the session execution",
                        }),
                    )
                    .await
                {
                    tracing::warn!(
                        target: "agena_background",
                        session_id = target_id,
                        %error,
                        "failed to suppress descendant background deliveries during cancellation"
                    );
                }
            }
        }
        let mut outcome = CancellationOutcome::from(result);
        if let Some(control) = control {
            let (document, run_id) = self
                .restore_unanswered_user_run(session_id, control)
                .await?;
            outcome.restored_user_message = document;
            outcome.restored_user_run_id = run_id;
        }
        Ok(outcome)
    }

    /// Wait until the cancelled execution has finished its durable cleanup,
    /// then decide from the authoritative session projection whether an
    /// assistant turn emitted a real part. If not, withdraw only the user run
    /// created by this execution and return its original composer document.
    async fn restore_unanswered_user_run(
        &self,
        session_id: i64,
        control: std::sync::Arc<ExecutionControl>,
    ) -> Result<(Option<ComposerDocument>, Option<i64>), AppError> {
        let Some(document) = control.restore_document().cloned() else {
            return Ok((None, None));
        };

        let released = tokio::time::timeout(
            Self::EXECUTION_CANCEL_UNREGISTER_GRACE,
            self.execution_registry
                .wait_until_execution_released(session_id, control.execution_id()),
        )
        .await
        .is_ok();
        if !released {
            tracing::warn!(
                target: "agena::session::cancel",
                session_id,
                execution_id = %control.execution_id(),
                "cancelled execution did not release before user-message recovery window"
            );
            return Ok((None, None));
        }

        let session = self.store.load_session(session_id).await?;
        if assistant_has_output_for_turn(&session, control.turn_id()) {
            return Ok((None, None));
        }

        // If cancellation won before submit_user_run returned, the marker id
        // may not have reached the in-memory control yet. Look it up by the
        // execution identity persisted in the marker. This closes the
        // commit/acknowledgement window without ever selecting a later user
        // message or an idempotency replay owned by another execution.
        if !control.user_run_submitted() {
            if let Some(run_id) = user_run_id_for_execution(&session, control.execution_id()) {
                let removed = self.store.withdraw_user_run(session_id, run_id).await?;
                return if removed.iter().any(|part| part.part_id == run_id) {
                    Ok((Some(document), Some(run_id)))
                } else {
                    Ok((None, None))
                };
            }

            // A keyed request with no matching execution marker can only be
            // an idempotency replay (or an alternate backend that does not
            // implement the execution-aware method). Never restore it: doing
            // so would put a duplicate message back into the editor while the
            // original message remains durable. Unkeyed requests have no
            // replay target, so they can still recover the editor if the
            // cancellation won before the transaction committed.
            return if control.user_idempotency_key().is_some() {
                Ok((None, None))
            } else {
                Ok((Some(document), None))
            };
        }
        if !control.user_run_created() {
            return Ok((None, None));
        }
        let Some(run_id) = control.user_run_id() else {
            return Ok((None, None));
        };
        let removed = self.store.withdraw_user_run(session_id, run_id).await?;
        if removed.iter().any(|part| part.part_id == run_id) {
            Ok((Some(document), Some(run_id)))
        } else {
            Ok((None, None))
        }
    }

    /// External entry: inject `parts` as a steer message into the active
    /// execution for `session_id`. Returns `Err` if no execution is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<TypedContent>,
    ) -> Result<(), AppError> {
        self.execution_registry
            .steer(session_id, parts)
            .await
            .map_err(execution_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let source = self.store.load_session(request.session_id).await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let message_id = user_message_id_for_turn(&source, request.turn_id)?;
        let user_marker = source
            .parts()
            .iter()
            .find(|part| part.is_run_marker() && part.part_id == message_id)
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "canonical turn {} has no projected user message in session {}",
                    request.turn_id, source.id
                ))
            })?;
        if !is_completed_user_rewind_target(user_marker) {
            return Err(AppError::Internal(format!(
                "rewind target must be a completed canonical user turn: {}",
                request.turn_id
            )));
        }
        let title = format!("Rewind of {}", source.title);
        let child_id = self.store.rewind(source.id, message_id, title).await?;
        self.store.load_session(child_id).await
    }

    /// Serialise `session_id` as a JSONL bundle (session header line followed
    /// by the session's ordered parts).
    pub async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.store.export_session_jsonl(session_id).await
    }

    /// Replay a JSONL bundle produced by [`Self::export_session_jsonl`] into
    /// this manager's workspace as a fresh session.
    pub async fn import_session_jsonl(&self, bundle: &str) -> Result<Session, AppError> {
        let workspace_id = self.current_workspace_id().await?;
        let session_id = self
            .store
            .import_session_jsonl(workspace_id, bundle)
            .await?;
        self.store.load_session(session_id).await
    }

    /// Return every session that shares the same `root_id`, ordered by
    /// `(depth, id)`. Useful for tree visualisation and bulk export.
    pub async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, AppError> {
        let summaries = self.store.list_session_tree(root_id).await?;
        summaries
            .into_iter()
            .map(crate::session::store::domain_summary_from_storage)
            .collect()
    }
}

/// Inclusive storage cutoff for a projected message named by its run-marker
/// part id. The marker is the message id; content parts follow it in canonical
/// `(created_at_ms, part_id)` order, so the final member with `run_id ==
/// marker_part_id` is the end of the message's shared prefix. A part id that
/// is not a run marker (a literal cutoff) passes through unchanged.
fn last_part_id_for_run_marker(parts: &[Part], marker_part_id: i64) -> Option<i64> {
    if !parts
        .iter()
        .any(|part| part.is_run_marker() && part.part_id == marker_part_id)
    {
        return Some(marker_part_id);
    }
    parts
        .iter()
        .rev()
        .find(|part| part.run_id == Some(marker_part_id))
        .map(|part| part.part_id)
        .or(Some(marker_part_id))
}

/// Whether the assistant turn identified by `turn_id` has emitted a real
/// assistant payload. The run marker itself, hook records, notices, and
/// system notifications are metadata; text, reasoning, tool activity, and
/// errors all mean that the model turn has produced observable output and the
/// user's message must remain in the transcript.
fn assistant_has_output_for_turn(session: &Session, turn_id: TurnId) -> bool {
    let Some(marker) = session.parts().iter().rev().find(|part| {
        part.is_run_marker()
            && part.role == PartRole::Assistant
            && part
                .content
                .get("turn_id")
                .and_then(serde_json::Value::as_str)
                .and_then(|value| uuid::Uuid::parse_str(value).ok())
                .map(agena_domain::TurnId)
                == Some(turn_id)
    }) else {
        return false;
    };

    session.parts().iter().any(|part| {
        part.run_id == Some(marker.part_id)
            && !matches!(
                part.kind.as_str(),
                "hook" | "notice" | "system_notification"
            )
    })
}

fn user_run_id_for_execution(
    session: &Session,
    execution_id: agena_domain::ExecutionId,
) -> Option<i64> {
    let execution_id = execution_id.to_string();
    session.parts().iter().find_map(|part| {
        (part.is_run_marker()
            && part.role == PartRole::User
            && part.origin_session_id == session.id
            && part
                .content
                .get("run_kind")
                .and_then(serde_json::Value::as_str)
                == Some("user_send")
            && part
                .content
                .get("execution_id")
                .and_then(serde_json::Value::as_str)
                == Some(execution_id.as_str()))
        .then_some(part.part_id)
    })
}

/// Inclusive storage cutoff for a session's final message: the id of the last
/// content part of the final run, or the marker id when that run is empty.
/// Sessions with no run markers fall back to the final part id (foreign data
/// may hold bare content parts).
fn last_part_id_for_last_run(parts: &[Part]) -> Option<i64> {
    match parts.iter().rev().find(|part| part.is_run_marker()) {
        Some(marker) => parts
            .iter()
            .rev()
            .find(|part| part.run_id == Some(marker.part_id))
            .map(|part| part.part_id)
            .or(Some(marker.part_id)),
        None => parts.last().map(|part| part.part_id),
    }
}

/// Resolve the canonical user message that owns `turn_id`.
///
/// Assistant run markers persist the conversation UUID pair on their content
/// (`turn_id`/`reply_id`), so the run that carries the turn id is the
/// assistant reply. The user input of the same canonical turn is the nearest
/// user-role run marker before that reply; user-run markers themselves do not
/// persist the UUID pair (they are written as `{"run_kind":"user_send"}`).
fn user_message_id_for_turn(
    session: &Session,
    turn_id: agena_domain::TurnId,
) -> Result<i64, AppError> {
    let parts = session.parts();
    let reply_index = parts
        .iter()
        .position(|part| {
            part.is_run_marker()
                && part
                    .content
                    .get("turn_id")
                    .and_then(serde_json::Value::as_str)
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(agena_domain::TurnId)
                    == Some(turn_id)
        })
        .ok_or_else(|| {
            AppError::Internal(format!(
                "canonical turn not found in session {}: {turn_id}",
                session.id
            ))
        })?;
    parts[..reply_index]
        .iter()
        .rev()
        .find(|part| part.is_run_marker() && part.role == PartRole::User)
        .map(|part| part.part_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "canonical turn {} has no user message in session {}",
                turn_id, session.id
            ))
        })
}

/// Recover every interactive request currently awaiting a reply in `session`,
/// from the parts projection: unanswered permissions and
/// unanswered user-input requests recorded on in-flight `tool_call` parts.
/// Requests are de-duplicated by `(kind, request_id)`.
fn pending_interactive_requests_from_session(
    session: &Session,
) -> Vec<agena_domain::PendingInteractiveRequest> {
    let mut seen = std::collections::HashSet::new();
    let mut requests = Vec::new();
    // Pending permissions live on the in-flight tool-call part's operation
    // authorization record (`operation.authorization.awaiting()`).
    for part in session.parts() {
        if part.kind != "tool_call" || !part.state.is_in_flight() {
            continue;
        }
        let Some(operation) = super::replies::operation_from_part(part) else {
            continue;
        };
        for permission in operation.authorization.awaiting() {
            let request = agena_domain::PendingInteractiveRequest::from(permission.request.clone());
            if seen.insert(format!("{:?}:{}", request.kind(), request.request_id())) {
                requests.push(request);
            }
        }
    }
    // Pending user-input requests live on the in-flight tool-call part's
    // `user_input` records (`operation.user_input.awaiting()`).
    for part in session.pending_interactions() {
        if part.kind == "tool_call" {
            let Some(operation) = super::replies::operation_from_part(part) else {
                continue;
            };
            for record in operation.user_input.awaiting() {
                let request = agena_domain::PendingInteractiveRequest::from(record.request.clone());
                if seen.insert(format!("{:?}:{}", request.kind(), request.request_id())) {
                    requests.push(request);
                }
            }
        }
    }
    requests
}

#[async_trait::async_trait]
impl agena_runtime::SessionQueryService for SessionManager {
    async fn list_session_summaries(
        &self,
        request: agena_domain::SessionListRequest,
    ) -> Result<Vec<agena_domain::SessionSummary>, agena_runtime::SessionQueryError> {
        self.list_session_summaries(request)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn session_presentation(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionPresentation, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let workflow_state = session.workflow_state();
        let message_count = session
            .parts()
            .iter()
            .filter(|part| part.is_run_marker())
            .count();
        Ok(agena_runtime::SessionPresentation {
            id: session.id,
            parent_id: session.parent_id,
            workspace_id: session.workspace_id,
            title: session.title,
            version: session.version,
            created_at: session.created_at,
            updated_at: session.updated_at,
            message_count,
            workflow_state,
        })
    }

    async fn list_projected_runs(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_runtime::SessionProjectedRun>, agena_runtime::SessionQueryError> {
        // Parts-native: `SessionManager::list_projected_runs` already
        // builds the stable `SessionProjectedRun` values directly from the
        // session's parts; only the error type needs adapting here.
        SessionManager::list_projected_runs(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn list_session_tree(
        &self,
        root_id: i64,
    ) -> Result<Vec<SessionSummary>, agena_runtime::SessionQueryError> {
        SessionManager::list_session_tree(self, root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn export_session_jsonl(
        &self,
        session_id: i64,
    ) -> Result<String, agena_runtime::SessionQueryError> {
        SessionManager::export_session_jsonl(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn latest_event_seq(
        &self,
        session_id: i64,
    ) -> Result<Option<i64>, agena_runtime::SessionQueryError> {
        // The session has no event log: its optimistic-lock version is the
        // monotonic per-session change sequence that consumers treat as the
        // durable high-water mark.
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        Ok(Some(session.version))
    }

    async fn session_usage(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionUsage, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        SessionManager::session_usage_async(self, &session)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn session_cost_summary(
        &self,
        session_id: i64,
    ) -> Result<agena_domain::SessionCostSummary, agena_runtime::SessionQueryError> {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        Ok(crate::session::cost::summarize(session.parts()))
    }

    async fn usage_stats(
        &self,
        query: agena_domain::UsageStatsQuery,
    ) -> Result<agena_domain::UsageStats, agena_runtime::SessionQueryError> {
        SessionManager::usage_stats(self, query)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))
    }

    async fn pending_interactive_requests(
        &self,
        session_id: i64,
    ) -> Result<Vec<agena_domain::PendingInteractiveRequestContext>, agena_runtime::SessionQueryError>
    {
        let session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let tree = SessionManager::list_session_tree(self, session.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let mut descendants = std::collections::HashSet::from([session.id]);
        loop {
            let previous_len = descendants.len();
            for summary in &tree {
                if summary
                    .parent_id
                    .is_some_and(|parent_id| descendants.contains(&parent_id))
                {
                    descendants.insert(summary.id);
                }
            }
            if descendants.len() == previous_len {
                break;
            }
        }

        let mut sessions = vec![session];
        for summary in tree {
            if summary.id == session_id
                || !descendants.contains(&summary.id)
                || SessionManager::active_execution(self, summary.id)
                    .await
                    .is_none()
            {
                continue;
            }
            sessions.push(
                SessionManager::get_session(self, summary.id)
                    .await
                    .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?,
            );
        }

        Ok(sessions
            .into_iter()
            .flat_map(|pending_session| {
                let session_id = pending_session.id;
                let parent_session_id = pending_session.parent_id;
                let task_id = pending_session.task_id.clone();
                pending_interactive_requests_from_session(&pending_session)
                    .into_iter()
                    .map(
                        move |request| agena_domain::PendingInteractiveRequestContext {
                            session_id,
                            parent_session_id,
                            task_id: task_id.clone(),
                            request,
                        },
                    )
            })
            .collect())
    }

    async fn execution_context(
        &self,
        session_id: i64,
    ) -> Result<agena_runtime::SessionExecutionContext, agena_runtime::SessionQueryError> {
        let mut session = SessionManager::get_session(self, session_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let state = self.execution_state();
        self.refresh_execution_policy(&mut session, state.as_ref());
        let runtime = session.runtime();
        Ok(agena_runtime::SessionExecutionContext {
            workflow_state: session.workflow_state(),
            agent_id: crate::identity::AGENA_AGENT_ID.to_string(),
            execution_access: runtime.execution.access,
            selected_permission: runtime.execution.selection.permission.clone(),
            effective_permission: runtime.execution.effective_permission.clone(),
            permission_ceiling: runtime.execution.permission_ceiling.clone(),
            model_provider_id: runtime.execution.selection.provider.clone(),
            model_adapter_id: runtime.execution.selection.adapter.clone(),
            model_id: runtime.execution.selection.model.clone(),
            model_thinking_mode: runtime.execution.selection.thinking_mode.clone(),
            model_speed_mode: runtime.execution.selection.speed_mode.clone(),
            model_verbosity: runtime.execution.selection.verbosity.clone(),
            model_parallel_tool_calls: runtime.execution.selection.parallel_tool_calls,
            effective_workspace_root: runtime
                .effective_workspace_root()
                .map(|path| path.display().to_string()),
            task_id: session.task_id.clone(),
            subtask_status: session.is_subagent().then_some(runtime.subtask.status),
            subtask_started_at: runtime
                .subtask
                .started_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_finished_at: runtime
                .subtask
                .finished_at_ms
                .and_then(chrono::DateTime::from_timestamp_millis),
            subtask_failure: runtime.subtask.failure.clone(),
        })
    }

    async fn is_descendant_session(
        &self,
        descendant_id: i64,
        ancestor_id: i64,
    ) -> Result<bool, agena_runtime::SessionQueryError> {
        let descendant = SessionManager::get_session(self, descendant_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let tree = SessionManager::list_session_tree(self, descendant.root_id)
            .await
            .map_err(|error| agena_runtime::SessionQueryError::internal_error(&error))?;
        let parents = tree
            .into_iter()
            .map(|summary| (summary.id, summary.parent_id))
            .collect::<std::collections::HashMap<_, _>>();
        let mut cursor = parents.get(&descendant_id).copied().flatten();
        let mut visited = std::collections::HashSet::new();
        while let Some(session_id) = cursor {
            if !visited.insert(session_id) {
                return Ok(false);
            }
            if session_id == ancestor_id {
                return Ok(true);
            }
            cursor = parents.get(&session_id).copied().flatten();
        }
        Ok(false)
    }
}

/// Project a session's parts into the stable transcript values, one per run.
///
/// Each run marker becomes a `SessionProjectedRun` whose parts are the
/// run's content parts, decoded from the canonical store payload. This is the
/// preserving the wire shape consumed by `agena-application` and `agena-cli`.
pub(crate) fn projected_runs_from_parts(
    parts: &[Part],
) -> Result<Vec<crate::session_query_service::SessionProjectedRun>, AppError> {
    let mut projected = Vec::new();
    for run in parts_into_runs(parts) {
        let marker = run.first().expect("run group has a marker");
        let mut projected_parts = Vec::with_capacity(run.len().saturating_sub(1));
        for (index, part) in run.iter().enumerate().skip(1) {
            if part.visibility.visible_to_user() {
                projected_parts.push(project_storage_part(part, marker.part_id, index as i32)?);
            }
        }
        if marker.visibility.visible_to_user() || !projected_parts.is_empty() {
            projected.push(crate::session_query_service::SessionProjectedRun {
                id: marker.part_id,
                role: role_from_part_role(marker.role),
                state: execution_status_from_part_state(marker.state),
                created_at: timestamp_millis_to_utc(marker.created_at_ms)?,
                // The run marker's content is the durable header payload and
                // is exposed directly as the projected run metadata.
                metadata: marker.content.clone(),
                usage: None,
                parts: projected_parts,
            });
        }
    }
    Ok(projected)
}

/// A decoded content part: the projection of one storage [`Part`] into the
/// fields the transcript and query surfaces need.
struct DecodedPart {
    id: i64,
    part_index: i32,
    status: ExecutionStatus,
    kind: String,
    name: Option<String>,
    summary: Option<String>,
    has_detail: bool,
    activity_id: Option<agena_domain::ActivityId>,
    segment_id: Option<agena_domain::TextSegmentId>,
    operation_id: Option<String>,
    created_at: chrono::DateTime<chrono::Utc>,
    content: Option<TypedContent>,
}

/// Decode one persisted content part (its `kind` column plus canonical JSON
/// payload) into the [`DecodedPart`] view used by transcript and query
/// projections.
fn decode_part(part: &Part, part_index: i32) -> Result<DecodedPart, AppError> {
    let content = typed_content_from_value(&part.kind, &part.content)?;
    // The coarse state column carries the lifecycle; the fine-grained status
    // (including denial outcomes) is reconstructed from the rich content.
    let status = match &content {
        TypedContent::ToolCall(tool_call) => operation_from_tool_call(tool_call).status(),
        _ => execution_status_from_part_state(part.state),
    };
    // Recover the provider operation id stashed by the tool-call serialization
    // so pending-tool correlation and prompt assembly survive a reload.
    let operation_id = match &content {
        TypedContent::ToolCall(tool_call) => operation_from_tool_call(tool_call)
            .metadata
            .get(OPERATION_ID_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned),
        _ => None,
    };
    let activity_kind = matches!(
        &content,
        TypedContent::Think(_)
            | TypedContent::ToolCall(_)
            | TypedContent::FileRef(_)
            | TypedContent::SkillRef(_)
            | TypedContent::Notice(_)
            | TypedContent::Hook(_)
            | TypedContent::Error(_)
    );
    Ok(DecodedPart {
        id: part.part_id,
        part_index,
        status,
        // Carry the precise storage kind through to the transcript surfaces so
        // each typed part has its own rendering dispatch.
        kind: part.kind.clone(),
        name: part_name_from_content(&content),
        summary: part.summary.clone(),
        has_detail: part.content.is_object(),
        activity_id: activity_kind.then(agena_domain::ActivityId::new),
        segment_id: matches!(&content, TypedContent::Text(_))
            .then(agena_domain::TextSegmentId::new),
        operation_id,
        created_at: timestamp_millis_to_utc(part.created_at_ms)?,
        content: Some(content),
    })
}

/// Derive the projected part name: text/reasoning use plain labels, tool calls
/// use their invocation name, and failures use their problem code.
fn part_name_from_content(content: &TypedContent) -> Option<String> {
    match content {
        TypedContent::Text(_) => Some("text".to_string()),
        TypedContent::Think(_) => Some("reasoning".to_string()),
        TypedContent::ToolCall(tool_call) => {
            let operation = operation_from_tool_call(tool_call);
            Some(operation.invocation.name)
        }
        TypedContent::SkillRef(_) => Some("skill_reference".to_string()),
        TypedContent::Error(error) => Some(user_problem_from_error(error).code.to_string()),
        TypedContent::FileRef(_) => Some("resource".to_string()),
        TypedContent::Hook(hook) => Some(format!("hook:{}", hook.hook)),
        TypedContent::Notice(_) => Some("notice".to_string()),
        TypedContent::SystemNotification(notification) => Some(format!(
            "{}:{}:{}",
            notification.operation_kind, notification.operation_id, notification.status
        )),
        TypedContent::Run(_) | TypedContent::PasteRef(_) | TypedContent::Compaction(_) => None,
    }
}

/// Project one persisted content part into the stable transcript part value.
fn project_storage_part(
    part: &Part,
    run_id: i64,
    part_index: i32,
) -> Result<agena_runtime::SessionProjectedPart, AppError> {
    let decoded = decode_part(part, part_index)?;
    Ok(agena_runtime::SessionProjectedPart {
        id: decoded.id,
        run_id,
        part_index: decoded.part_index,
        status: decoded.status,
        kind: decoded.kind,
        name: decoded.name,
        summary: decoded.summary,
        has_detail: decoded.has_detail,
        activity_id: decoded.activity_id,
        segment_id: decoded.segment_id,
        operation_id: decoded.operation_id,
        created_at: decoded.created_at,
        detail: decoded.content.as_ref().map(project_part_detail),
        content: decoded
            .content
            .as_ref()
            .map(|content| {
                typed_content_to_value(content)
                    .map_err(|error| AppError::Internal(format!("serialize part content: {error}")))
            })
            .transpose()?,
    })
}

fn project_part_detail(content: &TypedContent) -> agena_runtime::SessionProjectedPartDetail {
    match content {
        TypedContent::Text(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: value.synthetic,
        },
        TypedContent::Think(value) => agena_runtime::SessionProjectedPartDetail::Reasoning {
            summary: value.summary.clone(),
            raw_content: value.raw.clone(),
            encrypted_content: value.encrypted_content.clone(),
        },
        TypedContent::Error(value) => agena_runtime::SessionProjectedPartDetail::Error {
            problem: user_problem_from_error(value),
        },
        TypedContent::FileRef(value) => {
            agena_runtime::SessionProjectedPartDetail::Attachment(attachment_from_file_ref(value))
        }
        TypedContent::SkillRef(value) => agena_runtime::SessionProjectedPartDetail::SkillReference(
            skill_reference_from_skill_ref(value),
        ),
        TypedContent::ToolCall(value) => {
            agena_runtime::SessionProjectedPartDetail::ToolCall(Box::new((**value).clone()))
        }
        TypedContent::Hook(value) => agena_runtime::SessionProjectedPartDetail::Hook(Box::new(
            agena_runtime::SessionProjectedHookPart {
                hook: value.hook.clone(),
                plugin_id: value.plugin_id.clone(),
                summary: value.summary.clone(),
                detail: value.detail.clone(),
                message: value.message.clone(),
            },
        )),
        TypedContent::Notice(value) => agena_runtime::SessionProjectedPartDetail::Notice {
            summary: value.summary.clone(),
            detail: value.detail.clone(),
        },
        TypedContent::SystemNotification(value) => {
            agena_runtime::SessionProjectedPartDetail::SystemNotification {
                operation_id: value.operation_id.clone(),
                operation_kind: value.operation_kind.clone(),
                status: value.status.clone(),
                summary: value.summary.clone(),
                detail: value.detail.clone(),
                body: value.body.clone(),
                event_seq: value.event_seq,
            }
        }
        // These kinds have no dedicated transcript detail: run markers render
        // as empty text, while paste and compaction expose their text.
        TypedContent::Run(_) => agena_runtime::SessionProjectedPartDetail::Text {
            text: String::new(),
            synthetic: false,
        },
        TypedContent::PasteRef(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.text.clone(),
            synthetic: false,
        },
        TypedContent::Compaction(value) => agena_runtime::SessionProjectedPartDetail::Text {
            text: value.summary.clone().unwrap_or_default(),
            synthetic: false,
        },
    }
}

/// Convert a storage session tree into the shared domain DTO.
fn session_tree_domain(
    tree: Vec<agena_storage::store::SessionSummary>,
) -> Result<Vec<SessionSummary>, AppError> {
    tree.into_iter()
        .map(crate::session::store::domain_summary_from_storage)
        .collect()
}

fn descendant_cancellation_order(session_id: i64, tree: &[SessionSummary]) -> Vec<i64> {
    let mut included = std::collections::HashSet::from([session_id]);
    loop {
        let previous_len = included.len();
        for summary in tree {
            if summary
                .parent_id
                .is_some_and(|parent_id| included.contains(&parent_id))
            {
                included.insert(summary.id);
            }
        }
        if included.len() == previous_len {
            break;
        }
    }

    let mut descendants = tree
        .iter()
        .filter(|summary| included.contains(&summary.id))
        .map(|summary| (summary.depth, summary.id))
        .collect::<Vec<_>>();
    if !descendants.iter().any(|(_, id)| *id == session_id) {
        descendants.push((i64::MIN, session_id));
    }
    descendants.sort_by(|left, right| right.cmp(left));
    descendants.into_iter().map(|(_, id)| id).collect()
}

fn cancel_active_execution_result(
    result: Result<(), ExecutionControlError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) | Err(ExecutionControlError::NoActiveExecution(_)) => Ok(()),
        Err(error) => Err(execution_control_to_app_error(error)),
    }
}

fn is_completed_user_rewind_target(part: &Part) -> bool {
    part.role == PartRole::User
        && execution_status_from_part_state(part.state) == ExecutionStatus::Completed
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionControlError, cancel_active_execution_result, descendant_cancellation_order,
        is_completed_user_rewind_target,
    };
    use agena_domain::SessionSummary;
    use agena_domain::SubtaskStatus;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

    fn marker(part_id: i64, role: PartRole, state: PartState) -> Part {
        let now = chrono::Utc::now().timestamp_millis();
        Part {
            part_id,
            kind: "run".to_owned(),
            role,
            state,
            content: serde_json::json!({}),
            summary: None,
            visibility: PartVisibility::Both,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 0,
            started_at_ms: now,
            finished_at_ms: None,
            created_at_ms: now,
            updated_at_ms: now,
            provider_state: None,
        }
    }

    #[test]
    fn cancelling_a_completed_run_is_a_successful_no_op() {
        assert!(
            cancel_active_execution_result(Err(ExecutionControlError::NoActiveExecution(42)))
                .is_ok()
        );
        assert!(cancel_active_execution_result(Err(ExecutionControlError::SteerClosed)).is_err());
    }

    #[test]
    fn rewind_accepts_only_completed_user_messages() {
        let user = marker(1, PartRole::User, PartState::Completed);
        let assistant = marker(2, PartRole::Assistant, PartState::Completed);
        let pending_user = marker(3, PartRole::User, PartState::Pending);

        assert!(is_completed_user_rewind_target(&user));
        assert!(!is_completed_user_rewind_target(&assistant));
        assert!(!is_completed_user_rewind_target(&pending_user));
    }

    #[test]
    fn cancellation_orders_descendants_deepest_first() {
        let now = chrono::Utc::now();
        let summary = |id, parent_id, depth| SessionSummary {
            id,
            parent_id,
            depth,
            root_id: 1,
            workspace_id: 1,
            title: id.to_string(),
            favorite: false,
            pinned: false,
            version: 1,
            relation_kind: if parent_id.is_some() {
                agena_domain::SessionRelationKind::Subagent
            } else {
                agena_domain::SessionRelationKind::Root
            },
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            source_cutoff_seq_global: None,
            source_message_id: None,
            task_id: None,
            subtask_access: None,
            subtask_status: parent_id.map(|_| SubtaskStatus::Running),
            created_at: now,
            updated_at: now,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        };
        let tree = vec![
            summary(1, None, 0),
            summary(2, Some(1), 1),
            summary(3, Some(2), 2),
            summary(4, Some(1), 1),
        ];

        assert_eq!(descendant_cancellation_order(2, &tree), vec![3, 2]);
        assert_eq!(descendant_cancellation_order(1, &tree), vec![3, 4, 2, 1]);
    }
}
