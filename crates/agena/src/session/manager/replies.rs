use std::path::Path;

use super::*;
use crate::session::model::SessionPartRef;

#[derive(Debug, Clone)]
pub(super) struct AggregatedPermissionRequest {
    pub(super) action: PermissionAction,
    pub(super) related_actions: Vec<PermissionAction>,
    pub(super) requested_actions: Vec<PermissionAction>,
    pub(super) reason: String,
    pub(super) explanation: String,
    pub(super) source: Option<String>,
    pub(super) scope: Option<PermissionScope>,
    pub(super) operator: Option<String>,
    pub(super) risk: PermissionRiskLevel,
    pub(super) trace: Vec<DecisionTraceStep>,
}

pub(super) enum AggregatedPermissionOutcome {
    Allow,
    Request(Box<AggregatedPermissionRequest>),
    Deny { reason: String },
}

enum PendingReplyLookup<P> {
    Pending(P),
    Duplicate,
}

fn pending_reply_not_found_error(request_kind: &str, request_id: &str) -> AppError {
    AppError::Internal(format!(
        "pending {request_kind} request not found: {request_id}"
    ))
}

fn pending_reply_payload_missing_error(request_kind: &str, request_id: &str) -> AppError {
    AppError::Internal(format!(
        "pending {request_kind} request payload missing: {request_id}"
    ))
}

fn pending_reply_part_missing_error(request_kind: &str, request_id: &str) -> AppError {
    AppError::Internal(format!(
        "pending {request_kind} part not found: {request_id}"
    ))
}

fn pending_tool_part_not_found_error(part_ref: &SessionPartRef) -> AppError {
    AppError::Internal(format!(
        "pending tool part not found: message={}, part={}",
        part_ref.message_id, part_ref.part_id
    ))
}

fn assistant_message_for_part(
    session: &Session,
    part_ref: &SessionPartRef,
) -> Result<Message, AppError> {
    session
        .messages
        .get(part_ref.message_index)
        .cloned()
        .ok_or_else(|| pending_tool_part_not_found_error(part_ref))
}

fn update_resolved_tool_message(
    session: &mut Session,
    resolved: &ResolvedPendingTool,
    update: impl FnOnce(&mut MessagePart),
) -> Result<Message, AppError> {
    {
        let tool_part = session
            .part_mut(&resolved.pending.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
        update(tool_part);
    }
    assistant_message_for_part(session, &resolved.pending.part)
}

fn pending_operation_for_resolved(
    resolved: &ResolvedPendingTool,
    invocation: ToolInvocation,
    title: impl Into<String>,
    lifecycle: TimeRange,
) -> OperationPart {
    let mut operation = OperationPart::pending(resolved.call_id, invocation, title, lifecycle);
    if let Some(identity) = resolved.advertised_tool_identity.as_deref() {
        operation.set_advertised_tool_identity(identity.to_string());
    }
    operation
}

fn append_resolved_message_part(
    session: &mut Session,
    resolved: &ResolvedPendingTool,
    part: MessagePart,
) -> Result<Message, AppError> {
    session
        .messages
        .get_mut(resolved.pending.part.message_index)
        .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?
        .parts
        .push(part);
    assistant_message_for_part(session, &resolved.pending.part)
}

fn interactive_request_kind_label(
    request_kind: crate::message::PendingInteractiveRequestKind,
) -> &'static str {
    match request_kind {
        crate::message::PendingInteractiveRequestKind::Permission => "permission",
        crate::message::PendingInteractiveRequestKind::UserInput => "user input",
    }
}

fn matching_request_part_refs(
    session: &Session,
    request_id: &str,
    request_kind: crate::message::PendingInteractiveRequestKind,
    pending_only: bool,
) -> Vec<SessionPartRef> {
    session
        .messages
        .iter()
        .enumerate()
        .flat_map(|(message_index, message)| {
            message
                .parts
                .iter()
                .enumerate()
                .filter_map(move |(part_index, part)| {
                    if pending_only && part.status != ExecutionStatus::Pending {
                        return None;
                    }

                    let Some(_operation_id) = part.operation_id.as_deref() else {
                        return None;
                    };
                    let matches_request = match (request_kind, part.content.as_ref()) {
                        (
                            crate::message::PendingInteractiveRequestKind::Permission,
                            Some(PartContent::Request(RequestPart::Permission(request))),
                        ) => request.request_id() == request_id,
                        (
                            crate::message::PendingInteractiveRequestKind::UserInput,
                            Some(PartContent::Request(RequestPart::UserInput(request))),
                        ) => request.request_id() == request_id,
                        _ => false,
                    };
                    matches_request.then(|| SessionPartRef {
                        message_index,
                        part_index,
                        message_id: message.id,
                        part_id: part.id,
                    })
                })
        })
        .collect()
}

fn supersede_duplicate_request_parts(
    session: &mut Session,
    request_parts: &[SessionPartRef],
    request_kind: crate::message::PendingInteractiveRequestKind,
    request_id: &str,
) -> Result<(), AppError> {
    if request_parts.len() < 2 {
        return Ok(());
    }

    let summary = format!(
        "Superseded duplicate {} request: {}",
        interactive_request_kind_label(request_kind),
        request_id
    );
    for duplicate in request_parts.iter().skip(1) {
        let part = session.part_mut(duplicate).ok_or_else(|| {
            pending_reply_part_missing_error(
                interactive_request_kind_label(request_kind),
                request_id,
            )
        })?;
        part.status = ExecutionStatus::Cancelled;
        part.summary = Some(summary.clone());
    }

    Ok(())
}

fn push_unique_permission_action(actions: &mut Vec<PermissionAction>, action: PermissionAction) {
    if !actions.iter().any(|existing| existing == &action) {
        actions.push(action);
    }
}

fn mode_request_override_for_adapter(
    request_override: &ModelSpeedModeRequestOverride,
    adapter_overrides: &std::collections::BTreeMap<String, ModelSpeedModeRequestOverride>,
    resolved_adapter_id: Option<&crate::model::AdapterId>,
) -> ModelSpeedModeRequestOverride {
    let mut merged = request_override.clone();
    if let Some(adapter_id) = resolved_adapter_id.map(AsRef::<str>::as_ref)
        && let Some(adapter_override) = adapter_overrides.get(adapter_id)
    {
        merged = merged.merged_with(adapter_override);
    }
    merged
}

fn should_execute_pending_tools_concurrently(
    request_override: &ModelSpeedModeRequestOverride,
) -> bool {
    request_override.parallel_tool_calls() != Some(false)
}

impl SessionManager {
    fn lookup_pending_reply<P>(
        &self,
        session: &Session,
        session_id: i64,
        request_id: &str,
        request_kind: &str,
        find_pending: impl FnOnce(&Session, &str) -> Option<P>,
        has_replied: impl FnOnce(&Session, &str) -> bool,
    ) -> Result<PendingReplyLookup<P>, AppError> {
        match find_pending(session, request_id) {
            Some(pending) => Ok(PendingReplyLookup::Pending(pending)),
            None if has_replied(session, request_id)
                || session.has_finished_operation(request_id) =>
            {
                tracing::debug!(
                    target: "agena::session::reply",
                    session_id,
                    request_kind,
                    request_id = %request_id,
                    "ignoring duplicate reply for completed request"
                );
                Ok(PendingReplyLookup::Duplicate)
            }
            None => Err(pending_reply_not_found_error(request_kind, request_id)),
        }
    }

    fn clone_pending_reply_request<P, T>(
        &self,
        session: &Session,
        pending: &P,
        request_id: &str,
        request_kind: &str,
        request: impl FnOnce(&Session, &P) -> Option<T>,
    ) -> Result<T, AppError> {
        request(session, pending)
            .ok_or_else(|| pending_reply_payload_missing_error(request_kind, request_id))
    }

    fn complete_reply_request_parts(
        &self,
        session: &mut Session,
        request_id: &str,
        request_kind: crate::message::PendingInteractiveRequestKind,
        content: PartContent,
    ) -> Result<(), AppError> {
        let request_parts = matching_request_part_refs(session, request_id, request_kind, true);
        if request_parts.is_empty() {
            return Err(pending_reply_part_missing_error(
                interactive_request_kind_label(request_kind),
                request_id,
            ));
        }

        for request_part in request_parts {
            let part = session.part_mut(&request_part).ok_or_else(|| {
                pending_reply_part_missing_error(
                    interactive_request_kind_label(request_kind),
                    request_id,
                )
            })?;
            part.set_content(content.clone());
            part.status = ExecutionStatus::Completed;
        }
        Ok(())
    }

    fn upsert_existing_pending_request_part(
        &self,
        session: &mut Session,
        resolved: &ResolvedPendingTool,
        request_id: &str,
        request_kind: crate::message::PendingInteractiveRequestKind,
        request: RequestPart,
    ) -> Result<Option<Message>, AppError> {
        let request_parts = matching_request_part_refs(session, request_id, request_kind, true);
        let Some(primary) = request_parts.first() else {
            return Ok(None);
        };

        supersede_duplicate_request_parts(
            session,
            request_parts.as_slice(),
            request_kind,
            request_id,
        )?;

        let part = session.part_mut(primary).ok_or_else(|| {
            pending_reply_part_missing_error(
                interactive_request_kind_label(request_kind),
                request_id,
            )
        })?;
        let status = request.status();
        part.set_content(PartContent::Request(request));
        part.status = status;
        Ok(Some(assistant_message_for_part(
            session,
            &resolved.pending.part,
        )?))
    }

    async fn load_reply_session(
        &self,
        session_id: i64,
        options: &mut SessionRunOptions,
    ) -> Result<(Arc<SessionManagerState>, Session), AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let session = self
            .apply_requested_agent_profile(session, options, state.clone())
            .await?;
        Ok((state, session))
    }

    async fn continue_reply_session(
        &self,
        mut session: Session,
        session_id: i64,
        options: SessionRunOptions,
        run_source: RunSource,
        state: Arc<SessionManagerState>,
        task_error_context: &str,
    ) -> Result<Session, AppError> {
        let options = self.apply_execution_context_to_run_options(&session, options)?;
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }

        let manager = self.background_handle();
        tokio::task::spawn(async move {
            manager
                .run_until_stable_for(session_id, session, &options, run_source, state)
                .await
        })
        .await
        .map_err(|err| AppError::Internal(format!("{task_error_context}: {err}")))?
    }

    async fn persist_tool_completion(
        &self,
        session: Session,
        assistant_message: Message,
        resolved: &ResolvedPendingTool,
        persisted_rules: Vec<PersistedPermissionRule>,
        output: TranscriptToolOutput,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let tool_call_id = tool_call_id_for(resolved);
        let completed_part = assistant_message
            .parts
            .iter()
            .find(|part| {
                part.kind == crate::message::PartKind::Operation
                    && part.operation_id.as_deref() == Some(tool_call_id.as_ref())
            })
            .cloned();
        let session = self
            .persist_session_changes_with_rules(
                session,
                vec![assistant_message.clone()],
                Vec::new(),
                persisted_rules,
                state.clone(),
            )
            .await?;
        let events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            message_id: HistoryMessageId(assistant_message.id),
            call_id: tool_call_id,
            run_id: HistoryRunId::new(),
            tool_name: resolved.invocation.name.clone().into(),
            part: completed_part,
            output,
            completed_at: Utc::now(),
        })];
        self.store
            .append_history_items(session, events, state.cache_policy())
            .await
    }

    pub async fn reply_permission(
        &self,
        mut request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        let request_id = request.request.reply.request_id.clone();
        let reply_lock = self.reply_session_lock(request.request.session_id).await;
        let _reply_guard = reply_lock.lock().await;
        let (state, mut session) = self
            .load_reply_session(request.request.session_id, &mut request.request.options)
            .await?;
        let pending = match self.lookup_pending_reply(
            &session,
            request.request.session_id,
            request_id.as_str(),
            "permission",
            Session::find_pending_permission_by_request_id,
            Session::has_replied_permission_request,
        )? {
            PendingReplyLookup::Pending(pending) => pending,
            PendingReplyLookup::Duplicate => return Ok(session),
        };

        let permission_request = self.clone_pending_reply_request(
            &session,
            &pending,
            request_id.as_str(),
            "permission",
            |session, pending| session.pending_permission_request(pending).cloned(),
        )?;
        let reply_reason = request
            .request
            .reply
            .reason
            .clone()
            .unwrap_or_else(|| permission_request.reason.clone());

        self.complete_reply_request_parts(
            &mut session,
            request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::Permission,
            PartContent::request(RequestPart::Permission(InteractiveRequestPart::replied(
                permission_request.clone(),
                request.request.reply.clone(),
            ))),
        )?;
        let replied_assistant_message = assistant_message_for_part(&session, &pending.tool.part)?;

        let persisted_actions = if permission_request.requested_actions.is_empty() {
            vec![permission_request.action.clone()]
        } else {
            permission_request.requested_actions.clone()
        };
        let persisted_rules = persisted_rules_for_reply(
            &self.store,
            request.request.session_id,
            persisted_actions.as_slice(),
            &request.request.reply,
            request.operator.as_deref(),
        )
        .await?;
        session = self
            .persist_session_changes_with_rules(
                session,
                vec![replied_assistant_message],
                vec![EventKind::PermissionReplied(PermissionRepliedEvent {
                    session_id: request.request.session_id,
                    request_id: request.request.reply.request_id.clone(),
                    kind: request.request.reply.kind,
                    reason: request.request.reply.reason.clone(),
                    scope: request.request.reply.scope.map(permission_scope_label),
                    ts_ms: Utc::now().timestamp_millis(),
                })],
                persisted_rules.clone(),
                state.clone(),
            )
            .await?;

        if request_id.starts_with("host-permission:") {
            if let Some(waiter) = self
                .host_permission_waiters
                .lock()
                .await
                .remove(request_id.as_str())
            {
                let _ = waiter.response.send(request.request.reply.clone());
                return Ok(session);
            }

            let reason =
                "host-invoked permission continuation is unavailable; retry the tool".to_string();
            session = self
                .apply_tool_failure_with_rules(
                    session,
                    &pending.tool,
                    reason,
                    Vec::new(),
                    state.clone(),
                )
                .await?;
            return self
                .continue_reply_session(
                    session,
                    request.request.session_id,
                    request.request.options,
                    RunSource::PermissionReply,
                    state,
                    "permission continuation task failed",
                )
                .await;
        }

        match request.request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
                let granted_actions = if permission_request.requested_actions.is_empty() {
                    vec![permission_request.action.clone()]
                } else {
                    permission_request.requested_actions.clone()
                };
                let _permission_grant = self.install_host_permission_grant_for_pending_tool(
                    state.as_ref(),
                    session.id,
                    &resolved_tool,
                    granted_actions,
                );
                match self.execute_pending_tool_after_approval(
                    state.as_ref(),
                    session.id,
                    &resolved_tool,
                ) {
                    Ok(execution) => {
                        session = self
                            .apply_tool_success_with_rules(
                                session,
                                &pending.tool,
                                execution,
                                Vec::new(),
                                state.clone(),
                            )
                            .await?;
                    }
                    Err(ToolError::UserInputRequired(input)) => {
                        session = self
                            .apply_user_input_request(session, &pending.tool, input, state.clone())
                            .await?;
                    }
                    Err(err) => {
                        session = self
                            .apply_tool_failure_with_rules(
                                session,
                                &pending.tool,
                                err.to_string(),
                                Vec::new(),
                                state.clone(),
                            )
                            .await?;
                    }
                }
            }
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                session = self
                    .apply_tool_failure_with_rules(
                        session,
                        &pending.tool,
                        reply_reason,
                        Vec::new(),
                        state.clone(),
                    )
                    .await?;
            }
        }

        self.continue_reply_session(
            session,
            request.request.session_id,
            request.request.options,
            RunSource::PermissionReply,
            state,
            "permission continuation task failed",
        )
        .await
    }

    pub async fn reply_user_input(
        &self,
        mut request: SessionExecutionReplyRequest<UserInputReply>,
    ) -> Result<Session, AppError> {
        let request_id = request.reply.request_id.clone();
        let reply_lock = self.reply_session_lock(request.session_id).await;
        let _reply_guard = reply_lock.lock().await;
        let (state, mut session) = self
            .load_reply_session(request.session_id, &mut request.options)
            .await?;
        let pending = match self.lookup_pending_reply(
            &session,
            request.session_id,
            request_id.as_str(),
            "user input",
            Session::find_pending_user_input_by_request_id,
            Session::has_replied_user_input_request,
        )? {
            PendingReplyLookup::Pending(pending) => pending,
            PendingReplyLookup::Duplicate => return Ok(session),
        };

        let user_input_request = self.clone_pending_reply_request(
            &session,
            &pending,
            request_id.as_str(),
            "user input",
            |session, pending| session.pending_user_input_request(pending).cloned(),
        )?;
        self.complete_reply_request_parts(
            &mut session,
            request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::UserInput,
            PartContent::request(RequestPart::UserInput(InteractiveRequestPart::replied(
                user_input_request.clone(),
                request.reply.clone(),
            ))),
        )?;

        let is_host_request = request_id.starts_with("host-input:");
        if is_host_request {
            let response = host_user_input_response(&user_input_request, &request.reply)?;
            let tool_part_ref = session
                .resolve_part_ref(&pending.tool.part)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "pending tool part not found: message={}, part={}",
                        pending.tool.part.message_id, pending.tool.part.part_id
                    ))
                })?;
            let assistant_message = session.messages[tool_part_ref.message_index].clone();
            session = self
                .persist_session_changes(
                    session,
                    vec![assistant_message],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
            if let Some(waiter) = self
                .host_user_input_waiters
                .lock()
                .await
                .remove(request_id.as_str())
            {
                let _ = waiter.response.send(response);
                return Ok(session);
            }
            tracing::info!(
                target: "agena::session::reply",
                session_id = request.session_id,
                request_id = %request_id,
                "host user input waiter missing; resuming by replaying the pending tool"
            );
            session = self
                .resolve_pending_tool(session, pending.tool.clone(), state.clone())
                .await?;
        } else {
            match request.reply.kind {
                UserInputReplyKind::Submit => {
                    let execution = user_input_execution(&user_input_request, &request.reply)?;
                    session = self
                        .apply_tool_success(session, &pending.tool, execution, None, state.clone())
                        .await?;
                }
                UserInputReplyKind::Cancel => {
                    let reason = request.reply.reason.clone().unwrap_or_else(|| {
                        "user declined to answer requested questions".to_string()
                    });
                    session = self
                        .apply_tool_failure(session, &pending.tool, reason, None, state.clone())
                        .await?;
                }
            }
        }

        self.continue_reply_session(
            session,
            request.session_id,
            request.options,
            RunSource::UserInputReply,
            state,
            "user input continuation task failed",
        )
        .await
    }

    /// Convenience wrapper that registers a fresh `RunControl` for
    /// `session_id`, runs the loop, then unregisters. Used by entry points
    /// that don't already own a control (continuation-style: permission
    /// reply, user-input reply).
    async fn run_until_stable_for(
        &self,
        session_id: i64,
        session: Session,
        options: &SessionRunOptions,
        run_source: RunSource,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let (control, steer_rx) = self.run_registry.register(session_id).await;
        let result = self
            .run_until_stable(
                session,
                options,
                false,
                run_source,
                state,
                control.clone(),
                steer_rx,
            )
            .await;
        self.run_registry
            .unregister_if_matches(session_id, &control)
            .await;
        result
    }

    pub(super) async fn run_until_stable(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        allow_goal_continuation: bool,
        base_run_source: RunSource,
        state: Arc<SessionManagerState>,
        control: Arc<RunControl>,
        mut steer_rx: mpsc::UnboundedReceiver<Vec<PartContent>>,
    ) -> Result<Session, AppError> {
        let _ = allow_goal_continuation;
        loop {
            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            if control.cancel.is_cancelled() {
                if control.is_superseded() {
                    return Ok(session);
                }
                self.persist_run_failed_event(
                    session.id,
                    "run cancelled by user".to_string(),
                    state.clone(),
                )
                .await?;
                return Ok(session);
            }

            session = self
                .drain_steer_input(session, &mut steer_rx, &current_options, state.clone())
                .await?;

            let current_options =
                self.apply_execution_context_to_run_options(&session, options.clone())?;
            session.refresh_derived();
            if session.blocked() {
                return Ok(session);
            }

            if let Some(hit) = crate::session::doom_loop::detect(
                session.messages.as_slice(),
                state.config.doom_loop,
            ) {
                tracing::warn!(
                    target: "agena::session::doom_loop",
                    session_id = session.id,
                    tool = %hit.tool_label,
                    repeat = hit.repeat_count,
                    "aborting run: doom-loop detected"
                );
                self.persist_run_failed_event(session.id, hit.message(), state.clone())
                    .await?;
                return Ok(session);
            }

            let pending_tools = session.pending_tools();
            if !pending_tools.is_empty() {
                session = self
                    .resolve_pending_tools(session, pending_tools, &current_options, state.clone())
                    .await?;
                continue;
            }

            match session.status() {
                SessionStatus::Idle => {
                    let last_assistant_text = session
                        .messages
                        .iter()
                        .rev()
                        .find(|m| m.role == crate::role::Role::Assistant)
                        .map(|m| m.as_text_lossy());
                    let stop_input = crate::plugin::AgentStopInput {
                        session_id: session.id,
                        stop_hook_active: false,
                        last_assistant_message: last_assistant_text,
                    };
                    match state
                        .tool_executor
                        .plugin_manager()
                        .dispatch_agent_stop(stop_input)
                        .await
                    {
                        Ok(patch) if patch.continue_with_message.is_some() => {
                            let follow_up = patch.continue_with_message.unwrap_or_default();
                            let ids = self.store.reserve_message_ids(1).await?;
                            let user_message = build_message(
                                ids,
                                Role::User,
                                MessageStatus::Completed,
                                vec![PartContent::text(follow_up)],
                                MessageMetadata {
                                    source: MessageSource::System,
                                    parent_message_id: session
                                        .last_conversation_message()
                                        .map(|m| m.id),
                                    generated_by_call_id: None,
                                    model_provider_id: current_options
                                        .model
                                        .provider_id
                                        .to_string(),
                                    model_adapter_id: current_options
                                        .model
                                        .adapter_id
                                        .as_ref()
                                        .map(ToString::to_string),
                                    model_id: current_options.model.model_id.to_string(),
                                    model_thinking_mode: current_options.thinking_mode.clone(),
                                    model_speed_mode: current_options.speed_mode.clone(),
                                },
                            );
                            session.messages.push(user_message.clone());
                            session = self
                                .persist_session_changes(
                                    session,
                                    vec![user_message],
                                    Vec::new(),
                                    None,
                                    state.clone(),
                                )
                                .await?;
                            continue;
                        }
                        Ok(_) => return Ok(session),
                        Err(err) => {
                            tracing::warn!(
                                target: "agena_plugin_host::agent_stop",
                                "agent.stop hook failed (stopping normally): {err}"
                            );
                            return Ok(session);
                        }
                    }
                }
                SessionStatus::AwaitingModel => {}
            }

            let last_message_id = session.messages.last().map(|message| message.id);
            let already_auto_compacted_at_boundary = session
                .runtime
                .prompt_window
                .compaction
                .as_ref()
                .and_then(|compaction| compaction.compacted_by_message_id)
                == last_message_id;
            let session_usage = self.session_usage(&session)?;
            if state.config.auto_compaction.enabled
                && !already_auto_compacted_at_boundary
                && session_usage.limit_basis == Some(SessionUsageLimitBasis::ContextWindow)
                && let Some(limit_tokens) = session_usage.limit_tokens
                && session_usage
                    .projected_tokens
                    .unwrap_or(session_usage.current_tokens)
                    >= limit_tokens
            {
                let projected_tokens = session_usage
                    .projected_tokens
                    .unwrap_or(session_usage.current_tokens);
                tracing::info!(
                    target: "agena::session::compact",
                    session_id = session.id,
                    current_tokens = session_usage.current_tokens,
                    projected_tokens,
                    usable_tokens = limit_tokens,
                    reserved_tokens = session_usage.reserved_tokens.unwrap_or_default(),
                    "automatic session compaction triggered before model run"
                );
                session = Box::pin(self.auto_compact_session(
                    session,
                    &current_options,
                    state.clone(),
                    control.clone(),
                ))
                .await?;
            }

            let session_id = session.id;
            let model = format!(
                "{}/{}",
                current_options.model.provider_id, current_options.model.model_id
            );
            let message_count = session.messages.len();
            let pre_run_input = crate::plugin::PreRunInput {
                session_id,
                model: model.clone(),
                message_count,
            };
            state
                .tool_executor
                .plugin_manager()
                .broadcast_pre_run(pre_run_input)
                .await;

            match Box::pin(self.run_model_turn(
                session,
                &current_options,
                base_run_source,
                state.clone(),
                control.clone(),
            ))
            .await
            {
                Ok(next_session) => {
                    session = next_session;
                    let post_run_input = crate::plugin::PostRunInput {
                        session_id: session.id,
                        model,
                        status: format!("{:?}", session.status()),
                        message_count: session.messages.len(),
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_run(post_run_input)
                        .await;
                }
                Err(err) => {
                    let post_run_input = crate::plugin::PostRunInput {
                        session_id,
                        model,
                        status: format!("error: {err}"),
                        message_count,
                    };
                    state
                        .tool_executor
                        .plugin_manager()
                        .broadcast_post_run(post_run_input)
                        .await;
                    return Err(err);
                }
            }
        }
    }

    pub(super) async fn run_model_turn(
        &self,
        mut session: Session,
        options: &SessionRunOptions,
        run_source: RunSource,
        state: Arc<SessionManagerState>,
        control: Arc<RunControl>,
    ) -> Result<Session, AppError> {
        let run_span = tracing::info_span!(
            "session.run",
            session_id = session.id,
            provider_id = %options.model.provider_id,
            model_id = %options.model.model_id,
        );
        {
            let active_messages = prompt_window::active_prompt_messages(&session);
            let scoped_executor = state
                .tool_executor
                .for_session_context(&session.runtime.execution);
            let tool_protocol = scoped_executor.model_tool_prompt_text();
            let tools = scoped_executor.available_model_tools();
            let request_tools = tools.clone();
            let request_system = super::merge_system_prompt_with_tool_protocol(
                options.system.as_deref(),
                tool_protocol.as_deref(),
            );
            let prompt_budget = self.prompt_budget_for_run(
                &session,
                options,
                request_system.as_deref(),
                tools.as_slice(),
                state.as_ref(),
            );
            let provider_request_shape = state.processor.prompt_cache_shape(&options.model)?;
            let continuation_supported =
                state.processor.supports_prompt_continuation(&options.model);
            let prompt_request_options = PromptRequestOptions {
                provider_id: options.model.provider_id.as_ref(),
                model_id: options.model.model_id.as_ref(),
                system: request_system.as_deref(),
                temperature: options.temperature,
                max_output_tokens: options.max_output_tokens,
                tools: tools.as_slice(),
                provider_request_shape: provider_request_shape.as_ref(),
                continuation_supported,
            };
            let prompt_fingerprints =
                prompt_window::prompt_request_fingerprints(&prompt_request_options);
            let prompt_exceeds_runtime_budget = prompt_window::estimate_prompt_tokens_from_runtime(
                &session,
                active_messages.as_slice(),
                prompt_fingerprints.system_fingerprint.as_str(),
                prompt_fingerprints.request_options_fingerprint.as_str(),
            )
            .is_some_and(|estimate| estimate.total_tokens > prompt_budget.max_prompt_tokens);
            if prompt_exceeds_runtime_budget
                || state.processor.prompt_exceeds_budget(
                    active_messages.as_slice(),
                    prompt_budget.max_prompt_chars,
                )
            {
                tracing::warn!(
                    session_id = session.id,
                    prompt_message_count = active_messages.len(),
                    max_prompt_chars = prompt_budget.max_prompt_chars,
                    max_prompt_tokens = prompt_budget.max_prompt_tokens,
                    "prompt exceeds configured budget threshold; preserving append-only provider prefix and sending the full prompt"
                );
            }

            let prepared = prompt_window::build_prepared_prompt(&session, prompt_request_options);
            let provider_request_shape_fingerprint = prepared
                .provider_request_shape
                .as_ref()
                .map(crate::provider::PromptCacheShape::fingerprint);
            let provider_shape_change_keys = prepared
                .continuation_diagnostic
                .provider_shape_change_keys();
            tracing::debug!(
                session_id = session.id,
                provider_id = %options.model.provider_id,
                model_id = %options.model.model_id,
                prompt_window_generation = prepared.prompt_window_generation,
                prompt_cache_key = %prepared.prompt_cache_key,
                previous_response_id_present = prepared.previous_response_id.is_some(),
                continuation_reason = prepared.continuation_reason.as_ref(),
                provider_request_shape_fingerprint = provider_request_shape_fingerprint
                    .as_deref()
                    .unwrap_or(""),
                provider_request_shape_changed = prepared
                    .continuation_diagnostic
                    .provider_shape_changed(),
                provider_request_shape_change_keys = ?provider_shape_change_keys,
                prompt_message_count = prepared.messages.len(),
                system_included = prepared.system.is_some(),
                "prepared prompt for session run"
            );

            session.runtime.run.record_run_request(
                run_source,
                options.model.provider_id.to_string(),
                options.model.adapter_id.as_ref().map(ToString::to_string),
                options.model.model_id.to_string(),
                options.thinking_mode.clone(),
                options.speed_mode.clone(),
                options.verbosity.clone(),
                options.request_override.parallel_tool_calls(),
                prepared.prompt_cache_key.clone(),
                prepared.prompt_window_generation,
            );
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;

            let processor_ids = self.store.reserve_processor_ids().await?;
            let run_id = crate::session::RunId::new();
            let turn_started_at_unix_ms = Utc::now().timestamp_millis();
            let native_tools = state
                .processor
                .provider_registry()
                .native_tools_config(&options.model)?;
            let mut completion = options.completion_request(
                prepared.system.clone(),
                prepared.messages.clone(),
                tools,
                native_tools,
                Some(prepared.prompt_cache_key.clone()),
                prepared.previous_response_id.clone(),
                Some(prepared.prompt_window_generation),
            );
            completion.responses_api_metadata = Some(
                responses_api_request_metadata(
                    &session,
                    prepared.prompt_cache_key.as_str(),
                    prepared.prompt_window_generation,
                    run_id,
                    turn_started_at_unix_ms,
                )
                .await,
            );
            let run = SessionRunRequest {
                run_id,
                session_id: session.id,
                model: options.model.clone(),
                model_thinking_mode: options.thinking_mode.clone(),
                model_speed_mode: options.speed_mode.clone(),
                completion,
                next_message_id: processor_ids.message_id,
                part_ids: processor_ids.part_ids,
                next_call_id: session.next_call_id(),
                event_publisher: Some(Arc::clone(&self.publisher)),
                cancel: Some(control.cancel.clone()),
            };

            self.store
                .append_client_events(
                    session.id,
                    vec![EventKind::ExecutionStarted(ExecutionStartedEvent {
                        session_id: session.id,
                        ts_ms: turn_started_at_unix_ms,
                    })],
                )
                .await?;

            let processor_fut = state.processor.run_turn(run).instrument(run_span.clone());
            let run_outcome = tokio::select! {
                res = processor_fut => res,
                _ = control.cancel.cancelled() => {
                    Err(AppError::Internal("run cancelled by user".to_string()))
                }
            };
            match run_outcome {
                Ok(result) => {
                    let run_id = result.run_id;
                    let terminal_error = result.terminal_error;
                    if terminal_error.as_ref().is_some_and(is_user_cancelled_error)
                        && control.is_superseded()
                    {
                        return Ok(session);
                    }
                    let assistant_message = result
                        .state
                        .into_iter()
                        .find(|message| message.id == result.assistant_message_id)
                        .ok_or_else(|| {
                            AppError::Internal(format!(
                                "assistant message not found after processor run: {}",
                                result.assistant_message_id
                            ))
                        })?;
                    let transcript_digest = {
                        let mut transcript_messages =
                            prompt_window::active_prompt_messages(&session);
                        transcript_messages.push(assistant_message.clone());
                        prompt_window::prompt_transcript_digest(transcript_messages.as_slice())
                    };
                    let anchored_provider_request_shape = match state
                        .processor
                        .prompt_cache_shape(&options.model)
                    {
                        Ok(shape) => shape,
                        Err(err) => {
                            tracing::warn!(
                                session_id = session.id,
                                provider_id = %options.model.provider_id,
                                model_id = %options.model.model_id,
                                error = %err,
                                "failed to refresh provider request shape after run; falling back to prepared shape"
                            );
                            prepared.provider_request_shape.clone()
                        }
                    };
                    let anchored_prompt_request_options = PromptRequestOptions {
                        provider_id: options.model.provider_id.as_ref(),
                        model_id: options.model.model_id.as_ref(),
                        system: options.system.as_deref(),
                        temperature: options.temperature,
                        max_output_tokens: options.max_output_tokens,
                        tools: request_tools.as_slice(),
                        provider_request_shape: anchored_provider_request_shape.as_ref(),
                        continuation_supported,
                    };
                    let anchored_fingerprints = prompt_window::prompt_request_fingerprints(
                        &anchored_prompt_request_options,
                    );
                    if let Some(usage) = assistant_message.usage.as_ref() {
                        session.runtime.record_prompt_tokens(
                            assistant_message.id,
                            usage,
                            prepared.prompt_window_generation,
                            prompt_budget.model_context_window_tokens,
                            anchored_fingerprints.system_fingerprint.clone(),
                            anchored_fingerprints.request_options_fingerprint.clone(),
                            transcript_digest.clone(),
                        );
                    }
                    if let Some(response_id) =
                        prompt_window::extract_response_id(result.provider_metadata.as_ref())
                    {
                        session.runtime.set_provider_anchor(ProviderPromptAnchor {
                            provider_id: options.model.provider_id.to_string(),
                            model_id: options.model.model_id.to_string(),
                            previous_response_id: response_id,
                            assistant_message_id: assistant_message.id,
                            prompt_window_generation: prepared.prompt_window_generation,
                            system_fingerprint: anchored_fingerprints.system_fingerprint,
                            request_options_fingerprint: anchored_fingerprints
                                .request_options_fingerprint,
                            provider_request_shape: anchored_provider_request_shape,
                            transcript_digest,
                        });
                    } else {
                        session.runtime.clear_provider_anchor(
                            options.model.provider_id.as_ref(),
                            options.model.model_id.as_ref(),
                        );
                    }
                    drop(request_tools);
                    drop(prepared);

                    let client_events = result.client_events;
                    session.messages.push(assistant_message.clone());
                    let mut persisted_session = self
                        .persist_session_changes(
                            session,
                            vec![assistant_message],
                            client_events,
                            None,
                            state.clone(),
                        )
                        .await?;

                    let mut run_events: Vec<EventKind> = Vec::new();
                    run_events.push(EventKind::RunStarted(RunStarted {
                        run_id,
                        source: run_source,
                        model_id: options.model.model_id.as_ref().into(),
                        provider_id: options.model.provider_id.as_ref().into(),
                        request_digest: None,
                    }));
                    run_events.extend(result.history_items);
                    if let Some(err) = terminal_error.as_ref() {
                        run_events.push(EventKind::RunAborted(RunAborted {
                            run_id,
                            reason: RunAbortReason::ProviderError,
                            message: Some(err.to_string()),
                        }));
                    } else {
                        run_events.push(EventKind::RunCompleted(RunCompleted {
                            run_id,
                            finish_reason: FinishReason::default(),
                        }));
                    }
                    let store = Arc::clone(&self.store);
                    let cache_policy = state.cache_policy();
                    persisted_session = tokio::task::spawn(async move {
                        store
                            .append_history_items(persisted_session, run_events, cache_policy)
                            .await
                    })
                    .await
                    .map_err(|err| {
                        AppError::Internal(format!("history append task failed: {err}"))
                    })??;

                    if let Some(err) = terminal_error {
                        if is_user_cancelled_error(&err) {
                            if control.is_superseded() {
                                return Ok(persisted_session);
                            }
                        }
                        self.persist_run_failed_event(persisted_session.id, err.to_string(), state)
                            .await?;
                        return Err(err);
                    }

                    Ok(persisted_session)
                }
                Err(err) => {
                    if is_user_cancelled_error(&err) {
                        if control.is_superseded() {
                            return Ok(session);
                        }
                    }
                    self.persist_run_failed_event(session.id, err.to_string(), state)
                        .await?;
                    Err(err)
                }
            }
        }
    }

    fn prompt_budget_for_run(
        &self,
        _session: &Session,
        options: &SessionRunOptions,
        system: Option<&str>,
        tools: &[crate::plugin::registry::RegisteredTool],
        state: &SessionManagerState,
    ) -> PromptTurnBudget {
        let fallback_budget = state.processor.max_prompt_chars();
        let metadata = state
            .processor
            .model_metadata(&options.model)
            .unwrap_or_default();
        let context_window_tokens = metadata.limits.context_window_tokens;
        let max_prompt_chars = prompt_window::prompt_char_budget(
            context_window_tokens,
            options
                .max_output_tokens
                .or(metadata.limits.max_output_tokens),
            fallback_budget,
            system,
            tools,
        );

        PromptTurnBudget {
            max_prompt_chars,
            max_prompt_tokens: prompt_window::approximate_tokens_from_chars(max_prompt_chars),
            model_context_window_tokens: context_window_tokens,
        }
    }

    async fn resolve_pending_tools(
        &self,
        mut session: Session,
        pending_tools: Vec<SessionPendingTool>,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        // `parallel_tool_calls: false` is a model-request contract, not just
        // a provider hint. Keep provider-emitted calls in their transcript
        // order even when every individual tool is otherwise safe to fan out.
        // This also gives gateway calls a single host callback chain at a time.
        if !should_execute_pending_tools_concurrently(&options.request_override) {
            if let Some(tool) = session.next_pending_tool() {
                return self.resolve_pending_tool(session, tool, state).await;
            }
            return Ok(session);
        }

        let mut resolved_tools = Vec::new();
        for pending_tool in pending_tools {
            let Some(resolved) = self
                .prepare_concurrent_pending_tool(&mut session, &pending_tool, state.as_ref())
                .await?
            else {
                break;
            };
            resolved_tools.push(resolved);
        }

        if resolved_tools.len() < 2 {
            if let Some(tool) = session.next_pending_tool() {
                return self.resolve_pending_tool(session, tool, state).await;
            }
            return Ok(session);
        }

        let executions = self
            .execute_pending_tools_concurrently(state.clone(), session.id, resolved_tools.clone())
            .await?;
        for (resolved, result) in resolved_tools.into_iter().zip(executions) {
            match result {
                Ok(execution) => {
                    session = self
                        .apply_tool_success(
                            session,
                            &resolved.pending,
                            execution,
                            None,
                            state.clone(),
                        )
                        .await?;
                }
                Err(ToolError::UserInputRequired(input)) => {
                    return self
                        .apply_user_input_request(session, &resolved.pending, input, state)
                        .await;
                }
                Err(err) => {
                    session = self
                        .apply_tool_failure(
                            session,
                            &resolved.pending,
                            err.to_string(),
                            None,
                            state.clone(),
                        )
                        .await?;
                }
            }
        }

        Ok(session)
    }

    async fn prepare_concurrent_pending_tool(
        &self,
        session: &mut Session,
        pending_tool: &SessionPendingTool,
        state: &SessionManagerState,
    ) -> Result<Option<ResolvedPendingTool>, AppError> {
        let before_prepare = session.clone();
        let mut resolved = resolve_pending_tool(session, pending_tool)?;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        if let Err(err) = scoped_executor.validate_advertised_tool_identity(
            &resolved.invocation,
            resolved.advertised_tool_identity.as_deref(),
        ) {
            *session = before_prepare;
            tracing::debug!(
                target: "agena::session::tools",
                session_id = session.id,
                call_id = resolved.call_id,
                error = %err,
                "deferring stale tool call to sequential failure handling"
            );
            return Ok(None);
        }
        let prepared = match scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring tool preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };
        let (prepared_invocation, prepared_shell_command) = match scoped_executor
            .prepare_process_invocation(&prepared.invocation, session.id, resolved.call_id)
        {
            Ok(prepared) => prepared,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring shell preparation error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Operation(operation)) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
        }

        if !scoped_executor.is_concurrency_safe_invocation(&resolved.invocation) {
            *session = before_prepare;
            return Ok(None);
        }

        let permission_checks = match scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            ) {
            Ok(checks) => checks,
            Err(err) => {
                tracing::debug!(
                    target: "agena::session::tools",
                    session_id = session.id,
                    call_id = resolved.call_id,
                    error = %err,
                    "deferring permission-check error to sequential failure handling"
                );
                *session = before_prepare;
                return Ok(None);
            }
        };

        for check in permission_checks {
            if !matches!(
                self.resolve_permission_decision(Some(session.id), &check)
                    .await?
                    .decision,
                PermissionDecision::Allow
            ) {
                *session = before_prepare;
                return Ok(None);
            }
        }

        Ok(Some(resolved))
    }

    #[tracing::instrument(skip(self, state, pending_tools), fields(session_id, tool_count = pending_tools.len()))]
    async fn execute_pending_tools_concurrently(
        &self,
        state: Arc<SessionManagerState>,
        session_id: i64,
        pending_tools: Vec<ResolvedPendingTool>,
    ) -> Result<Vec<Result<ToolInvocationExecution, ToolError>>, AppError> {
        // Cap concurrent blocking tool executions so a wide tool fan-out
        // cannot exhaust the tokio blocking pool.
        static TOOL_BLOCKING_LIMIT: std::sync::OnceLock<Arc<Semaphore>> =
            std::sync::OnceLock::new();
        let semaphore = TOOL_BLOCKING_LIMIT
            .get_or_init(|| Arc::new(Semaphore::new(32)))
            .clone();

        let mut handles = Vec::with_capacity(pending_tools.len());
        for pending_tool in pending_tools {
            let executor = state.tool_executor.clone();
            let scoped_executor =
                executor.for_session_context(&pending_tool.session_runtime.execution);
            let permit = semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|err| AppError::Internal(format!("tool semaphore closed: {err}")))?;
            handles.push(tokio::task::spawn_blocking(move || {
                let _permit = permit;
                scoped_executor.validate_advertised_tool_identity(
                    &pending_tool.invocation,
                    pending_tool.advertised_tool_identity.as_deref(),
                )?;
                scoped_executor.execute_invocation_detailed_bypassing_permissions(
                    &pending_tool.invocation,
                    session_id,
                    pending_tool.call_id,
                )
            }));
        }

        let mut results = Vec::with_capacity(handles.len());
        for handle in handles {
            results.push(handle.await.map_err(|err| {
                AppError::Internal(format!("concurrent tool task failed: {err}"))
            })?);
        }
        Ok(results)
    }

    async fn resolve_pending_tool(
        &self,
        mut session: Session,
        pending_tool: SessionPendingTool,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut resolved = resolve_pending_tool(&session, &pending_tool)?;
        let scoped_executor = state
            .tool_executor
            .for_session_context(&session.runtime.execution);
        if let Err(err) = scoped_executor.validate_advertised_tool_identity(
            &resolved.invocation,
            resolved.advertised_tool_identity.as_deref(),
        ) {
            return Box::pin(self.apply_tool_failure(
                session,
                &resolved.pending,
                err.to_string(),
                None,
                state,
            ))
            .await;
        }
        let prepared = match scoped_executor.prepare_invocation(
            &resolved.invocation,
            session.id,
            resolved.call_id,
        ) {
            Ok(prepared) => prepared,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };
        let (prepared_invocation, prepared_shell_command) = match scoped_executor
            .prepare_process_invocation(&prepared.invocation, session.id, resolved.call_id)
        {
            Ok(prepared) => prepared,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };
        resolved.prepared_shell_command = prepared_shell_command;
        resolved.invocation = prepared_invocation;
        let mut session_changed = false;
        if prepared.invocation != resolved.invocation || prepared.title_override.is_some() {
            let current_title = match session
                .part(&resolved.pending.part)
                .and_then(|part| part.content.as_ref())
            {
                Some(PartContent::Operation(operation)) => operation.title.clone(),
                _ => format!("Tool {}", tool_name(&resolved.invocation)),
            };

            resolved.invocation = prepared.invocation.clone();
            let tool_part = session.part_mut(&resolved.pending.part).ok_or_else(|| {
                AppError::Internal(format!(
                    "pending tool part not found: message={}, part={}",
                    resolved.pending.part.message_id, resolved.pending.part.part_id
                ))
            })?;
            tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                &resolved,
                prepared.invocation,
                prepared.title_override.unwrap_or(current_title),
                resolved.lifecycle.clone(),
            )));
            session_changed = true;
        }

        let permission_checks = match scoped_executor
            .collect_permission_checks_for_invocation_in_session(
                &resolved.invocation,
                Some(session.id),
            ) {
            Ok(checks) => checks,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };

        match self
            .aggregate_permission_outcome(Some(session.id), permission_checks.as_slice())
            .await?
        {
            AggregatedPermissionOutcome::Allow => {}
            AggregatedPermissionOutcome::Request(request) => {
                let request = *request;
                return self
                    .apply_permission_request(
                        session,
                        &resolved.pending,
                        request.action,
                        request.related_actions,
                        request.requested_actions,
                        request.reason,
                        request.explanation,
                        request.source,
                        request.scope,
                        request.operator,
                        request.risk,
                        request.trace,
                        state,
                    )
                    .await;
            }
            AggregatedPermissionOutcome::Deny { reason } => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    reason,
                    None,
                    state,
                ))
                .await;
            }
        }

        if session_changed {
            let assistant_message = session.messages[resolved.pending.part.message_index].clone();
            session = self
                .persist_session_changes(
                    session,
                    vec![assistant_message],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
        }

        let streaming_tool = match state
            .tool_executor
            .for_session_context(&session.runtime.execution)
            .execute_invocation_streaming_after_authorization(
                &resolved.invocation,
                session.id,
                resolved.call_id,
            )
            .await
        {
            Ok(stream) => stream,
            Err(err) => {
                return Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await;
            }
        };

        if let Some(stream) = streaming_tool {
            return self
                .apply_streaming_tool_execution(session, &resolved.pending, stream, state)
                .await;
        }

        match self.execute_pending_tool(state.as_ref(), session.id, &resolved) {
            Ok(execution) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                self.apply_tool_success(session, &resolved.pending, execution, None, state)
                    .await
            }
            Err(ToolError::UserInputRequired(input)) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                self.apply_user_input_request(session, &resolved.pending, input, state)
                    .await
            }
            Err(err) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                Box::pin(self.apply_tool_failure(
                    session,
                    &resolved.pending,
                    err.to_string(),
                    None,
                    state,
                ))
                .await
            }
        }
    }

    pub async fn resolve_tool_permission_check(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        self.resolve_permission_decision(session_id, check).await
    }

    async fn resolve_permission_decision(
        &self,
        session_id: Option<i64>,
        check: &ToolPermissionCheck,
    ) -> Result<crate::permission::PermissionResolution, AppError> {
        let key = permission_action_key(&check.action)?;
        let persisted_rules = self
            .store
            .resolve_permission_rules(key.as_str(), session_id)
            .await?;
        let mut resolution =
            resolve_permission_with_persisted_rules(check.decision.clone(), &persisted_rules);

        if persisted_rules.is_empty() {
            let plugins = self
                .execution_state()
                .tool_executor
                .plugin_manager()
                .clone();
            if !plugins.is_empty() {
                let default_decision = match resolution.decision {
                    PermissionDecision::Allow => crate::plugin::PermissionDecision::Allow,
                    PermissionDecision::Deny { .. } => crate::plugin::PermissionDecision::Deny,
                    PermissionDecision::Ask { .. } => crate::plugin::PermissionDecision::Prompt,
                };
                let req = crate::plugin::PermissionAskInput {
                    session_id: session_id.unwrap_or(-1),
                    action: format!("{:?}", check.action),
                    subject: permission_subject(&check.action),
                    default_decision,
                };
                match plugins.dispatch_permission_ask_blocking(req) {
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Allow,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Allow;
                        resolution.risk = PermissionRiskLevel::Low;
                        resolution.explanation = format!(
                            "allowed by plugin decision from {plugin_id} ({})",
                            authority.trust_level
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "allowed by plugin decision from {plugin_id} (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Deny,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Deny {
                            reason: format!("denied by plugin {plugin_id}"),
                        };
                        resolution.risk = PermissionRiskLevel::High;
                        resolution.explanation = format!(
                            "denied by plugin decision from {plugin_id} ({})",
                            authority.trust_level
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "denied by plugin decision from {plugin_id} (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Decision {
                        plugin_id,
                        decision: crate::plugin::PermissionDecision::Prompt,
                        authority,
                    })) => {
                        resolution.decision = PermissionDecision::Ask {
                            reason: resolution.explanation.clone(),
                        };
                        resolution.risk = PermissionRiskLevel::Medium;
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "plugin {plugin_id} requested confirmation (trust={}, capabilities={})",
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(Some(crate::plugin::host::PermissionAskOutcome::Advice {
                        plugin_id,
                        advice,
                        authority,
                    })) => {
                        let explanation = if advice.reason.trim().is_empty() {
                            format!("permission advised by plugin {plugin_id}")
                        } else {
                            format!("{} (plugin: {plugin_id})", advice.reason)
                        };
                        resolution.explanation = explanation.clone();
                        let plugin_risk = plugin_risk_to_core(advice.risk);
                        resolution.decision = apply_advisory_permission_decision(
                            resolution.decision.clone(),
                            advice.decision,
                            &explanation,
                        );
                        resolution.risk = max_permission_risk(
                            max_permission_risk(resolution.risk, plugin_risk),
                            risk_for_permission_decision(&resolution.decision),
                        );
                        resolution.trace.push(DecisionTraceStep {
                            source_kind: PolicySourceKind::PluginAdvice,
                            summary: format!(
                                "{} (trust={}, capabilities={})",
                                explanation,
                                authority.trust_level,
                                authority.plugin_capabilities.join(", ")
                            ),
                            source: Some(plugin_id),
                            scope: None,
                            operator: None,
                        });
                    }
                    Ok(None) => {}
                    Err(err) => {
                        tracing::warn!(
                            target: "agena_plugin_host::permission",
                            "permission plugin failed: {err}"
                        );
                    }
                }
            }
        }

        Ok(resolution)
    }

    pub(super) async fn aggregate_permission_outcome(
        &self,
        session_id: Option<i64>,
        checks: &[ToolPermissionCheck],
    ) -> Result<AggregatedPermissionOutcome, AppError> {
        let mut related_actions = Vec::with_capacity(checks.len());
        let mut requested_actions = Vec::new();
        let mut primary_request: Option<AggregatedPermissionRequest> = None;

        for check in checks {
            let action = check.action.clone();
            push_unique_permission_action(&mut related_actions, action.clone());
            let resolution = self.resolve_permission_decision(session_id, check).await?;
            match resolution.decision {
                PermissionDecision::Allow => {}
                PermissionDecision::Deny { reason } => {
                    return Ok(AggregatedPermissionOutcome::Deny { reason });
                }
                PermissionDecision::Ask { reason } => {
                    push_unique_permission_action(&mut requested_actions, action.clone());
                    let (source, scope, operator) = match resolution.source {
                        crate::permission::PermissionResolutionSource::PersistedRule {
                            scope,
                            source,
                            operator,
                            ..
                        } => (Some(source), Some(scope), operator),
                        crate::permission::PermissionResolutionSource::StaticPolicy => {
                            (Some("static_policy".to_string()), None, None)
                        }
                    };

                    if let Some(existing) = primary_request.as_mut() {
                        existing.risk = max_permission_risk(existing.risk, resolution.risk);
                        existing.trace.extend(resolution.trace);
                    } else {
                        primary_request = Some(AggregatedPermissionRequest {
                            action,
                            related_actions: Vec::new(),
                            requested_actions: Vec::new(),
                            reason,
                            explanation: resolution.explanation,
                            source,
                            scope,
                            operator,
                            risk: resolution.risk,
                            trace: resolution.trace,
                        });
                    }
                }
            }
        }

        if let Some(mut request) = primary_request {
            request.related_actions = related_actions;
            request.requested_actions = requested_actions;
            if request.requested_actions.len() > 1 {
                let additional = request.requested_actions.len() - 1;
                request.reason = format!(
                    "{} (plus {additional} more permission checks for this tool call)",
                    request.reason
                );
            }
            return Ok(AggregatedPermissionOutcome::Request(Box::new(request)));
        }

        Ok(AggregatedPermissionOutcome::Allow)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn apply_permission_request(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        action: PermissionAction,
        related_actions: Vec<PermissionAction>,
        requested_actions: Vec<PermissionAction>,
        reason: String,
        explanation: String,
        source: Option<String>,
        scope: Option<PermissionScope>,
        operator: Option<String>,
        risk: crate::permission::PermissionRiskLevel,
        trace: Vec<crate::permission::DecisionTraceStep>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let request_id = resolve_pending_tool(&session, pending_tool)?.operation_id;
        self.apply_permission_request_with_id(
            session,
            pending_tool,
            request_id,
            action,
            related_actions,
            requested_actions,
            reason,
            explanation,
            source,
            scope,
            operator,
            risk,
            trace,
            state,
        )
        .await
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) async fn apply_permission_request_with_id(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        request_id: String,
        action: PermissionAction,
        related_actions: Vec<PermissionAction>,
        requested_actions: Vec<PermissionAction>,
        reason: String,
        explanation: String,
        source: Option<String>,
        scope: Option<PermissionScope>,
        operator: Option<String>,
        risk: crate::permission::PermissionRiskLevel,
        trace: Vec<crate::permission::DecisionTraceStep>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = PermissionRequest {
            request_id,
            session_id: Some(session.id),
            action,
            related_actions: related_actions.clone(),
            requested_actions: requested_actions.clone(),
            reason: reason.clone(),
            explanation: explanation.clone(),
            source,
            scope,
            operator,
            risk,
            trace: trace.clone(),
            created_at: Utc::now(),
        };

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    format!("Awaiting permission: {reason}"),
                    resolved.lifecycle.clone(),
                )));
                tool_part.status = ExecutionStatus::Pending;
                tool_part.summary = Some(reason.clone());
            })?;

        let permission_request_part =
            RequestPart::Permission(InteractiveRequestPart::pending(request.clone()));
        let assistant_message = match self.upsert_existing_pending_request_part(
            &mut session,
            &resolved,
            request.request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::Permission,
            permission_request_part,
        )? {
            Some(message) => message,
            None => {
                let permission_part_id = self.store.reserve_part_id().await?;
                append_resolved_message_part(
                    &mut session,
                    &resolved,
                    build_request_part(
                        permission_part_id,
                        resolved.pending.part.message_id,
                        resolved.operation_id.as_str(),
                        RequestPart::Permission(InteractiveRequestPart::pending(request.clone())),
                    ),
                )?
            }
        };
        let session_id = session.id;
        self.persist_session_changes(
            session,
            vec![assistant_message],
            vec![EventKind::PermissionRequested(PermissionRequestedEvent {
                session_id,
                request_id: request.request_id.clone(),
                action: request.action.clone(),
                related_actions,
                requested_actions,
                reason: reason.clone(),
                explanation,
                source: request.source.clone(),
                scope: request.scope.map(permission_scope_label),
                operator: request.operator.clone(),
                risk: request.risk,
                trace,
                ts_ms: Utc::now().timestamp_millis(),
            })],
            None,
            state.clone(),
        )
        .await
    }

    async fn apply_user_input_request(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::AskUserToolInput,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let request_id = resolve_pending_tool(&session, pending_tool)?.operation_id;
        self.apply_user_input_request_with_id(session, pending_tool, input, request_id, state)
            .await
    }

    pub(super) async fn apply_user_input_request_with_id(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        input: crate::message::AskUserToolInput,
        request_id: String,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let request = UserInputRequest {
            request_id,
            session_id: Some(session.id),
            title: input.title,
            body_markdown: input.body_markdown,
            kind: input.kind,
            submit_label: input.submit_label,
            cancel_label: input.cancel_label,
            questions: input.questions,
            created_at: Utc::now(),
        };

        let _assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                tool_part.set_content(PartContent::Operation(pending_operation_for_resolved(
                    &resolved,
                    resolved.invocation.clone(),
                    ask_user_title(&request),
                    resolved.lifecycle.clone(),
                )));
                tool_part.status = ExecutionStatus::Pending;
                tool_part.summary = Some(match request.questions.len() {
                    0 => "Ask user".to_string(),
                    1 => "Waiting for answer".to_string(),
                    count => format!("Waiting for {count} answers"),
                });
            })?;

        let user_input_request_part =
            RequestPart::UserInput(InteractiveRequestPart::pending(request.clone()));
        let assistant_message = match self.upsert_existing_pending_request_part(
            &mut session,
            &resolved,
            request.request_id.as_str(),
            crate::message::PendingInteractiveRequestKind::UserInput,
            user_input_request_part,
        )? {
            Some(message) => message,
            None => {
                let input_part_id = self.store.reserve_part_id().await?;
                append_resolved_message_part(
                    &mut session,
                    &resolved,
                    build_request_part(
                        input_part_id,
                        resolved.pending.part.message_id,
                        resolved.operation_id.as_str(),
                        RequestPart::UserInput(InteractiveRequestPart::pending(request.clone())),
                    ),
                )?
            }
        };
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
            .await
    }

    async fn apply_streaming_tool_execution(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        mut stream: StreamingToolExecution,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let stream_id = stream.stream_id.clone();
        while let Some(chunk) = stream.chunks.recv().await {
            let Some(delta) = chunk.text_delta.as_deref() else {
                continue;
            };
            if delta.is_empty() {
                continue;
            }

            session = self
                .append_streaming_tool_output_delta(session.id, pending_tool, delta, state.clone())
                .await?;
        }

        let execution = match stream.end.await {
            Ok(Ok(execution)) => execution,
            Ok(Err(err)) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_failure(session, pending_tool, err.to_string(), None, state)
                    .await;
            }
            Err(_) => {
                let session = self
                    .store
                    .load_session(session.id, state.cache_policy())
                    .await?;
                return self
                    .apply_tool_failure(
                        session,
                        pending_tool,
                        format!("tool stream ended without terminal result: {stream_id}"),
                        None,
                        state,
                    )
                    .await;
            }
        };

        let session = self
            .store
            .load_session(session.id, state.cache_policy())
            .await?;
        self.apply_tool_success(session, pending_tool, execution, None, state)
            .await
    }

    /// Persist one text chunk for a pending tool operation. This is shared by
    /// ordinary direct streaming invocations and streaming targets executed
    /// through the tools.call gateway.
    pub(super) async fn append_streaming_tool_output_delta(
        &self,
        session_id: i64,
        pending_tool: &SessionPendingTool,
        delta: &str,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let tool_part_ref = session
            .resolve_part_ref(&pending_tool.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
        {
            let tool_part = session
                .part_mut(&tool_part_ref)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending_tool.part))?;
            if !tool_part.append_tool_output_delta(delta) {
                return Err(AppError::Internal(format!(
                    "streaming tool part refused output delta: message={}, part={}",
                    pending_tool.part.message_id, pending_tool.part.part_id
                )));
            }
            if matches!(
                tool_part.status,
                ExecutionStatus::Pending | ExecutionStatus::InProgress
            ) {
                tool_part.status = ExecutionStatus::InProgress;
            }
        }

        let assistant_message = assistant_message_for_part(&session, &pending_tool.part)?;
        self.persist_session_changes(session, vec![assistant_message], Vec::new(), None, state)
            .await
    }

    async fn apply_tool_success(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.apply_tool_success_with_rules(
            session,
            pending_tool,
            execution,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    async fn apply_tool_success_with_rules(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        execution: ToolInvocationExecution,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let tool_output = execution.output.clone();
        let output_text = execution.view.output_text.clone();
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = operation_blocks_from_tool_output(
            &resolved.invocation,
            &tool_output,
            execution.view.attachments.as_slice(),
            output_text.as_str(),
        );
        let completion_title = {
            let execution_title = execution.view.title.trim();
            if !execution_title.is_empty() {
                execution_title.to_string()
            } else {
                session
                    .part(&resolved.pending.part)
                    .and_then(|part| part.content.as_ref())
                    .and_then(|content| match content {
                        PartContent::Operation(operation) => Some(operation.title.clone()),
                        _ => None,
                    })
                    .filter(|title| !title.trim().is_empty())
                    .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)))
            }
        };
        self.apply_tool_success_execution_context(&mut session, &resolved.invocation, &execution);

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::completed(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    output_text.clone(),
                    blocks.clone(),
                    execution.view.attachments.clone(),
                    tool_output.clone(),
                    lifecycle.clone(),
                );
                operation.set_title(completion_title.clone());
                operation.result.metadata.extend(
                    execution.view.metadata.iter().map(|(key, value)| {
                        (key.clone(), serde_json::Value::String(value.clone()))
                    }),
                );
                tool_part.set_content(PartContent::Operation(operation));
                tool_part.status = ExecutionStatus::Completed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            TranscriptToolOutput::Text {
                text: execution.view.output_text.clone(),
            },
            state,
        )
        .await
    }

    async fn apply_tool_failure(
        &self,
        session: Session,
        pending_tool: &SessionPendingTool,
        reason: String,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.apply_tool_failure_with_rules(
            session,
            pending_tool,
            reason,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    async fn apply_tool_failure_with_rules(
        &self,
        mut session: Session,
        pending_tool: &SessionPendingTool,
        reason: String,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let resolved = resolve_pending_tool(&session, pending_tool)?;
        let lifecycle = completed_lifecycle(&resolved.lifecycle);
        let blocks = text_result_blocks(reason.as_str());
        let failure_title = session
            .part(&resolved.pending.part)
            .and_then(|part| part.content.as_ref())
            .and_then(|content| match content {
                PartContent::Operation(operation) => Some(operation.title.clone()),
                _ => None,
            })
            .filter(|title| !title.trim().is_empty())
            .unwrap_or_else(|| format!("Tool {}", tool_name(&resolved.invocation)));

        // Notify plugins about the tool failure (fire-and-forget).
        state.tool_executor.broadcast_tool_failure(
            &resolved.invocation,
            session.id,
            resolved.call_id,
            &reason,
        );

        let assistant_message =
            update_resolved_tool_message(&mut session, &resolved, |tool_part| {
                let mut operation = OperationPart::failed(
                    resolved.call_id,
                    resolved.invocation.clone(),
                    reason.clone(),
                    reason.clone(),
                    blocks.clone(),
                    Vec::new(),
                    ToolOutput::default(),
                    lifecycle.clone(),
                );
                operation.set_title(failure_title.clone());
                tool_part.set_content(PartContent::Operation(operation));
                tool_part.status = ExecutionStatus::Failed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            TranscriptToolOutput::Error { message: reason },
            state,
        )
        .await
    }

    pub(super) async fn persist_session_changes(
        &self,
        session: Session,
        touched_messages: Vec<Message>,
        client_events: Vec<EventKind>,
        persisted_rule: Option<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.persist_session_changes_with_rules(
            session,
            touched_messages,
            client_events,
            persisted_rule.into_iter().collect(),
            state,
        )
        .await
    }

    pub(super) async fn persist_session_changes_with_rules(
        &self,
        session: Session,
        touched_messages: Vec<Message>,
        client_events: Vec<EventKind>,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        self.store
            .persist(
                SessionCommit {
                    session,
                    touched_messages,
                    client_events,
                    persisted_rules,
                },
                state.cache_policy(),
            )
            .await
    }

    pub(super) fn apply_run_selection_to_session(
        &self,
        session: &mut Session,
        options: &SessionRunOptions,
    ) -> bool {
        let next_model_provider_id = options.model.provider_id.to_string();
        let next_model_adapter_id = options.model.adapter_id.as_ref().map(ToString::to_string);
        let next_model_id = options.model.model_id.to_string();
        let next_thinking_mode = options.thinking_mode.clone();
        let next_speed_mode = options.speed_mode.clone();
        let next_verbosity = options.verbosity.clone();
        let next_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let changed = session.runtime.execution.selection.provider.as_deref()
            != Some(next_model_provider_id.as_str())
            || session.runtime.execution.selection.adapter.as_deref()
                != next_model_adapter_id.as_deref()
            || session.runtime.execution.selection.model.as_deref() != Some(next_model_id.as_str())
            || session.runtime.execution.selection.thinking_mode != next_thinking_mode
            || session.runtime.execution.selection.speed_mode != next_speed_mode
            || session.runtime.execution.selection.verbosity != next_verbosity
            || session.runtime.execution.selection.parallel_tool_calls != next_parallel_tool_calls;
        session.runtime.set_model_override(
            Some(next_model_provider_id),
            next_model_adapter_id,
            Some(next_model_id),
        );
        session.runtime.set_model_mode_overrides(
            next_thinking_mode,
            next_speed_mode,
            next_verbosity,
            next_parallel_tool_calls,
        );
        changed
    }

    pub(super) fn apply_execution_context_to_run_options(
        &self,
        session: &Session,
        mut options: SessionRunOptions,
    ) -> Result<SessionRunOptions, AppError> {
        self.apply_selection_modes_to_run_options(session, &mut options)?;
        if let Some(system) = session.runtime.execution.system_prompt_override.as_ref() {
            options.system = Some(system.clone());
        }
        if options.temperature.is_none() {
            let execution = self.execution_state();
            let provider_registry = execution.processor.provider_registry();
            if let Ok(metadata) = provider_registry.model_metadata(&options.model) {
                options.temperature = metadata.parsed_default_temperature();
            }
        }
        if options.agent_profile.is_none() {
            options.agent_profile = session.runtime.execution.selection.agent.clone();
        }
        Ok(options)
    }

    fn apply_selection_modes_to_run_options(
        &self,
        session: &Session,
        options: &mut SessionRunOptions,
    ) -> Result<(), AppError> {
        let state = self.execution_state();
        let effective_selection = state
            .config
            .default_selection
            .overlay_with_cascade(&session.runtime.execution.selection);
        let selection_model = effective_selection.model_ref().map_err(|error| {
            AppError::Internal(format!(
                "session {} contains invalid execution model selection: {error}",
                session.id
            ))
        })?;
        let modes_belong_to_options_model = selection_model
            .as_ref()
            .is_some_and(|model| model == &options.model);
        if options.thinking_mode.is_none() {
            options.thinking_mode = modes_belong_to_options_model
                .then(|| effective_selection.thinking_mode.clone())
                .flatten();
        }
        if options.speed_mode.is_none() {
            options.speed_mode = modes_belong_to_options_model
                .then(|| effective_selection.speed_mode.clone())
                .flatten();
        }
        if options.request_override.parallel_tool_calls().is_none() {
            options.request_override.set_parallel_tool_calls(
                modes_belong_to_options_model
                    .then_some(effective_selection.parallel_tool_calls)
                    .flatten(),
            );
        }
        if options.verbosity.is_none() {
            options.verbosity = modes_belong_to_options_model
                .then(|| effective_selection.verbosity.clone())
                .flatten();
        }
        self.apply_model_mode_requests(options)
    }

    fn apply_model_mode_requests(&self, options: &mut SessionRunOptions) -> Result<(), AppError> {
        let execution = self.execution_state();
        let provider_registry = execution.processor.provider_registry();
        let resolved_adapter_id = options.model.adapter_id.clone().or_else(|| {
            provider_registry
                .get(options.model.provider_id.as_ref())
                .and_then(|provider| provider.default_adapter().cloned())
        });

        let requested_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let mut merged_override = options.request_override.clone();
        merged_override.set_parallel_tool_calls(None);
        if let Some(thinking_mode_name) = options.thinking_mode.as_deref() {
            let thinking_modes = provider_registry.model_thinking_modes(&options.model)?;
            let thinking_mode = thinking_modes.get(thinking_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no think mode `{thinking_mode_name}`",
                    options.model
                ))
            })?;
            options.thinking = thinking_mode.thinking.clone();
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &thinking_mode.request_override,
                &thinking_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        if let Some(speed_mode_name) = options.speed_mode.as_deref() {
            let speed_modes = provider_registry.model_speed_modes(&options.model)?;
            let speed_mode = speed_modes.get(speed_mode_name).ok_or_else(|| {
                AppError::Config(format!(
                    "model `{}` has no speed mode `{speed_mode_name}`",
                    options.model
                ))
            })?;
            merged_override = merged_override.merged_with(&mode_request_override_for_adapter(
                &speed_mode.request_override,
                &speed_mode.adapter_overrides,
                resolved_adapter_id.as_ref(),
            ));
        }
        if requested_parallel_tool_calls.is_some() {
            merged_override.set_parallel_tool_calls(requested_parallel_tool_calls);
        }
        options.request_override = merged_override;
        Ok(())
    }

    pub(super) fn resolve_effective_session_permission(
        &self,
        session: &Session,
        state: &SessionManagerState,
        agent_permission: Option<&crate::agent::PermissionConfig>,
    ) -> crate::agent::PermissionConfig {
        let mut effective = state.config.permission.clone();
        if let Some(agent_permission) = agent_permission {
            effective.merge_from(agent_permission.clone());
        }
        effective.merge_from(managed_project_state_permission(
            state.tool_executor.workspace_root(),
        ));
        effective.merge_from(session.runtime.execution.selection.permission.clone());
        effective
    }

    fn model_from_session_selection(
        &self,
        session: &Session,
    ) -> Result<Option<ModelRef>, AppError> {
        session
            .runtime
            .execution
            .selection
            .model_ref()
            .map_err(|error| {
                AppError::Internal(format!(
                    "session {} contains invalid execution model selection: {error}",
                    session.id
                ))
            })
    }

    pub(super) fn default_model_from_config(
        &self,
        state: &SessionManagerState,
    ) -> Result<Option<ModelRef>, AppError> {
        state
            .processor
            .provider_registry()
            .resolve_default_model_selection(&state.config.default_selection)
    }

    pub(super) fn model_from_session_or_default(
        &self,
        session: &Session,
        state: &SessionManagerState,
    ) -> Result<ModelRef, AppError> {
        self.model_from_session_selection(session)?
            .map(Ok)
            .unwrap_or_else(|| {
                self.default_model_from_config(state)?.ok_or_else(|| {
                    AppError::Internal(format!(
                        "model is required for session {}; set a session model or global default model",
                        session.id
                    ))
                })
            })
    }

    pub(super) fn run_options_from_session(
        &self,
        session: &Session,
        state: Arc<SessionManagerState>,
    ) -> Result<SessionRunOptions, AppError> {
        let model = self.model_from_session_or_default(session, &state)?;

        self.apply_execution_context_to_run_options(
            session,
            SessionRunOptions {
                model,
                thinking_mode: None,
                speed_mode: None,
                verbosity: None,
                thinking: None,
                request_override: Default::default(),
                system: None,
                temperature: None,
                max_output_tokens: None,
                agent_profile: None,
            },
        )
    }

    pub(super) async fn clear_session_agent_profile(
        &self,
        mut session: Session,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        session.runtime.execution.selection.agent = None;
        session.runtime.execution.system_prompt_override = None;
        session.runtime.set_allowed_tools(Vec::new());
        session.runtime.execution.effective_permission =
            self.resolve_effective_session_permission(&session, &state, None);
        session.runtime.set_model_override(None, None, None);
        session
            .runtime
            .set_model_mode_overrides(None, None, None, None);
        Ok(session)
    }

    pub(super) async fn apply_requested_agent_profile(
        &self,
        session: Session,
        options: &mut SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let requested = options
            .agent_profile
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let persisted = session
            .runtime
            .execution
            .selection
            .agent
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned);
        let requested_explicitly = requested.is_some();
        let persisted_explicitly = persisted.is_some();
        let has_explicit_tool_restrictions = !session.runtime.allowed_tools().is_empty();
        let effective = requested.or(persisted).or_else(|| {
            (!has_explicit_tool_restrictions)
                .then(|| state.config.default_agent.clone())
                .flatten()
        });
        let Some(agent_name) = effective else {
            let mut session = session;
            session.runtime.execution.effective_permission =
                self.resolve_effective_session_permission(&session, &state, None);
            return Ok(session);
        };
        let profile = state
            .tool_executor
            .subagent_registry()
            .require(agent_name.as_str())
            .map_err(|err| AppError::Config(err.to_string()))?;
        options.agent_profile = Some(profile.name.clone());
        if session.runtime.execution.selection.agent.as_deref() == Some(profile.name.as_str())
            && session.runtime.execution.system_prompt_override.is_some()
        {
            *options = self.apply_execution_context_to_run_options(&session, options.clone())?;
            return Ok(session);
        }
        self.apply_agent_profile_to_session(
            session,
            options,
            profile,
            state,
            requested_explicitly || persisted_explicitly,
        )
        .await
    }

    async fn apply_agent_profile_to_session(
        &self,
        mut session: Session,
        options: &mut SessionRunOptions,
        profile: crate::agents::AgentProfile,
        state: Arc<SessionManagerState>,
        apply_profile_model_override: bool,
    ) -> Result<Session, AppError> {
        let next_allowed_tools = crate::agents::internal_allowed_tools(profile.name.as_str());
        let next_permission = self.resolve_effective_session_permission(
            &session,
            &state,
            Some(&profile.frontmatter.permission),
        );
        let next_system = profile.prompt.trim().to_string();
        let next_model = self.resolve_root_agent_model(
            &session,
            options,
            &state,
            if apply_profile_model_override {
                Some(&profile.frontmatter.defaults)
            } else {
                None
            },
        )?;
        let next_model_provider_id = next_model.provider_id.to_string();
        let next_model_adapter_id = next_model.adapter_id.as_ref().map(ToString::to_string);
        let next_model_id = next_model.model_id.to_string();
        options.model = next_model.clone();
        self.apply_selection_modes_to_run_options(&session, options)?;
        let next_thinking_mode = options.thinking_mode.clone();
        let next_speed_mode = options.speed_mode.clone();
        let next_verbosity = options.verbosity.clone();
        let next_parallel_tool_calls = options.request_override.parallel_tool_calls();
        let changed = session.runtime.execution.selection.agent.as_deref()
            != Some(profile.name.as_str())
            || session.runtime.execution.system_prompt_override.as_deref()
                != Some(next_system.as_str())
            || session.runtime.allowed_tools() != next_allowed_tools.as_slice()
            || session.runtime.execution.effective_permission != next_permission
            || session.runtime.execution.selection.provider.as_deref()
                != Some(next_model_provider_id.as_str())
            || session.runtime.execution.selection.adapter.as_deref()
                != next_model_adapter_id.as_deref()
            || session.runtime.execution.selection.model.as_deref() != Some(next_model_id.as_str())
            || session.runtime.execution.selection.thinking_mode != next_thinking_mode
            || session.runtime.execution.selection.speed_mode != next_speed_mode
            || session.runtime.execution.selection.verbosity != next_verbosity
            || session.runtime.execution.selection.parallel_tool_calls != next_parallel_tool_calls;
        session.runtime.execution.selection.agent = Some(profile.name.clone());
        session.runtime.execution.system_prompt_override = Some(next_system);
        session.runtime.set_allowed_tools(next_allowed_tools);
        session.runtime.execution.effective_permission = next_permission;
        session.runtime.set_model_override(
            Some(next_model_provider_id.clone()),
            next_model_adapter_id.clone(),
            Some(next_model_id.clone()),
        );
        session.runtime.set_model_mode_overrides(
            next_thinking_mode.clone(),
            next_speed_mode.clone(),
            next_verbosity.clone(),
            next_parallel_tool_calls,
        );
        options.model = next_model;
        options.thinking_mode = next_thinking_mode;
        options.speed_mode = next_speed_mode;
        options.verbosity = next_verbosity;
        options.system = session.runtime.execution.system_prompt_override.clone();
        if !changed {
            return Ok(session);
        }
        self.persist_session_changes(session, Vec::new(), Vec::new(), None, state)
            .await
    }

    fn resolve_root_agent_model(
        &self,
        _session: &Session,
        options: &SessionRunOptions,
        state: &SessionManagerState,
        requested_selection: Option<&crate::agents::AgentSelectionConfig>,
    ) -> Result<ModelRef, AppError> {
        let base_model = options.model.clone();
        match requested_selection.filter(|value| !value.is_empty()) {
            Some(selection) => self.resolve_agent_selection_model_ref(
                state.processor.provider_registry(),
                &base_model,
                selection,
            ),
            None => Ok(base_model),
        }
    }

    fn resolve_agent_selection_model_ref(
        &self,
        provider_registry: &crate::provider::ProviderRegistry,
        base_model: &ModelRef,
        requested_selection: &crate::agents::AgentSelectionConfig,
    ) -> Result<ModelRef, AppError> {
        if requested_selection.is_empty() {
            return Ok(base_model.clone());
        }
        let requested_provider = requested_selection
            .provider
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_adapter = requested_selection
            .adapter
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let requested_model = requested_selection
            .model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let provider_changed =
            requested_provider.is_some_and(|provider| provider != base_model.provider_id.as_ref());
        let provider_id = requested_provider.unwrap_or(base_model.provider_id.as_ref());
        let base_adapter = (!provider_changed)
            .then(|| base_model.adapter_id.as_ref().map(AsRef::<str>::as_ref))
            .flatten();
        let adapter_id = requested_adapter.or(base_adapter);
        let base_model_id = (!provider_changed && requested_adapter.is_none())
            .then(|| base_model.model_id.as_ref());
        let model_id = requested_model.or(base_model_id);
        provider_registry.resolve_model_selection(provider_id, adapter_id, model_id)
    }

    pub(super) fn apply_tool_success_execution_context(
        &self,
        session: &mut Session,
        invocation: &ToolInvocation,
        execution: &ToolInvocationExecution,
    ) {
        let payload_tool_name = payload_tool_name_for_invocation(invocation);
        if let Some(output) = crate::tool::ToolPayloadOutput::from_tool_output(
            payload_tool_name.as_str(),
            &execution.output,
        ) {
            match output {
                crate::tool::ToolPayloadOutput::EnterSnapshot { path, .. } => {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                    return;
                }
                crate::tool::ToolPayloadOutput::ExitSnapshot { .. } => {
                    session.runtime.set_effective_workspace_root(None);
                    return;
                }
                _ => {}
            }
        }

        match execution
            .view
            .metadata
            .get("agena.effect")
            .map(String::as_str)
        {
            Some("enter_snapshot") => {
                if let Some(path) = custom_payload_value(&execution.output)
                    .and_then(|value| value.get("path").cloned())
                    .and_then(|value| value.as_str().map(str::to_string))
                {
                    session
                        .runtime
                        .set_effective_workspace_root(Some(PathBuf::from(path)));
                }
            }
            Some("exit_snapshot") => {
                session.runtime.set_effective_workspace_root(None);
            }
            _ => {}
        }
    }

    async fn persist_run_failed_event(
        &self,
        session_id: i64,
        reason: String,
        state: Arc<SessionManagerState>,
    ) -> Result<(), AppError> {
        let event = EventKind::ExecutionFailed(ExecutionFailedEvent {
            session_id,
            error: ErrorInfo {
                code: "session_run_failed".to_string(),
                message: reason,
            },
            ts_ms: Utc::now().timestamp_millis(),
        });
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        let _ = self
            .persist_session_changes(session, Vec::new(), vec![event], None, state)
            .await?;
        Ok(())
    }

    pub(super) async fn find_child_session_for_task(
        &self,
        parent_session_id: i64,
        task_id: Option<&str>,
    ) -> Result<Option<Session>, AppError> {
        let Some(task_id) = task_id.map(str::trim).filter(|value| !value.is_empty()) else {
            return Ok(None);
        };
        let summaries = self
            .list_session_summaries(SessionListRequest {
                offset: 0,
                limit: None,
                include_subagents: true,
            })
            .await?;
        let state = self.execution_state();
        for child_id in summaries
            .into_iter()
            .filter(|summary| summary.parent_id == Some(parent_session_id))
            .map(|summary| summary.id)
        {
            let session = self
                .store
                .load_session(child_id, state.cache_policy())
                .await?;
            if session.runtime.execution.task_id.as_deref() == Some(task_id) {
                return Ok(Some(session));
            }
        }
        Ok(None)
    }

    pub(super) fn subtask_run_options(
        &self,
        child: &Session,
        parent: &Session,
        state: &SessionManagerState,
        requested_model: Option<&str>,
        requested_selection: Option<&crate::agents::AgentSelectionConfig>,
    ) -> Result<SessionRunOptions, AppError> {
        let requested_model = requested_model
            .map(str::trim)
            .filter(|value| !value.is_empty());
        let base_model = match self.model_from_session_selection(child)? {
            Some(model) => model,
            None => match self.model_from_session_selection(parent)? {
                Some(model) => model,
                None => self.default_model_from_config(state)?.ok_or_else(|| {
                    AppError::Internal(
                        "subtask requires a child, parent, or global default model before it can run"
                            .to_string(),
                    )
                })?,
            },
        };
        let model = if let Some(model_id) = requested_model {
            self.resolve_requested_session_model_ref(&base_model, model_id)?
        } else if let Some(selection) = requested_selection.filter(|value| !value.is_empty()) {
            self.resolve_agent_selection_model_ref(
                state.processor.provider_registry(),
                &base_model,
                selection,
            )?
        } else {
            base_model
        };
        Ok(SessionRunOptions {
            model,
            thinking_mode: None,
            speed_mode: None,
            verbosity: None,
            thinking: None,
            request_override: Default::default(),
            system: child.runtime.execution.system_prompt_override.clone(),
            temperature: None,
            max_output_tokens: None,
            agent_profile: child.runtime.execution.selection.agent.clone(),
        })
    }

    fn resolve_requested_session_model_ref(
        &self,
        base_model: &ModelRef,
        requested_model: &str,
    ) -> Result<ModelRef, AppError> {
        let requested_model = requested_model.trim();
        if requested_model.is_empty() {
            return Ok(base_model.clone());
        }

        if requested_model.matches('/').count() >= 2
            && let Some((provider_id, model_id)) = requested_model.split_once('/')
        {
            return ModelRef::try_new(provider_id, model_id).map_err(|error| {
                AppError::Config(format!(
                    "invalid requested model reference `{requested_model}`: {error}"
                ))
            });
        }

        let mut model = ModelRef::new(
            base_model.provider_id.to_string(),
            requested_model.to_string(),
        );
        model.adapter_id = base_model.adapter_id.clone();
        Ok(model)
    }

    /// Drain every pending steer message (non-blocking) and append each as
    /// a User message before the next model run. A user steer becomes the
    /// next input the model sees.
    async fn drain_steer_input(
        &self,
        mut session: Session,
        steer_rx: &mut mpsc::UnboundedReceiver<Vec<PartContent>>,
        options: &SessionRunOptions,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        loop {
            let parts = match steer_rx.try_recv() {
                Ok(parts) => parts,
                Err(mpsc::error::TryRecvError::Empty | mpsc::error::TryRecvError::Disconnected) => {
                    return Ok(session);
                }
            };
            let ids = self.store.reserve_message_ids(parts.len()).await?;
            let user_message = build_message(
                ids,
                Role::User,
                MessageStatus::Completed,
                parts,
                MessageMetadata {
                    source: MessageSource::User,
                    parent_message_id: session.last_conversation_message().map(|m| m.id),
                    generated_by_call_id: None,
                    model_provider_id: options.model.provider_id.to_string(),
                    model_adapter_id: options.model.adapter_id.as_ref().map(ToString::to_string),
                    model_id: options.model.model_id.to_string(),
                    model_thinking_mode: options.thinking_mode.clone(),
                    model_speed_mode: options.speed_mode.clone(),
                },
            );
            session.messages.push(user_message.clone());
            session = self
                .persist_session_changes(
                    session,
                    vec![user_message],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
        }
    }

    fn execute_pending_tool(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let _host_user_input_sequence =
            self.host_user_input_sequence_guard(session_id, pending_tool.call_id);
        let scoped_executor = state
            .tool_executor
            .for_session_context(&pending_tool.session_runtime.execution);
        scoped_executor.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
            pending_tool.prepared_shell_command.clone(),
        )
    }

    fn execute_pending_tool_after_approval(
        &self,
        state: &SessionManagerState,
        session_id: i64,
        pending_tool: &ResolvedPendingTool,
    ) -> Result<ToolInvocationExecution, ToolError> {
        let _host_user_input_sequence =
            self.host_user_input_sequence_guard(session_id, pending_tool.call_id);
        let scoped_executor = state
            .tool_executor
            .for_session_context(&pending_tool.session_runtime.execution);
        scoped_executor.execute_invocation_detailed_bypassing_permissions_with_prepared_shell(
            &pending_tool.invocation,
            session_id,
            pending_tool.call_id,
            pending_tool.prepared_shell_command.clone(),
        )
    }

    pub(super) fn execution_state(&self) -> Arc<SessionManagerState> {
        self.execution.load_full()
    }
}

async fn responses_api_request_metadata(
    session: &Session,
    prompt_cache_key: &str,
    prompt_window_generation: u64,
    run_id: crate::session::RunId,
    turn_started_at_unix_ms: i64,
) -> crate::provider::ResponsesApiRequestMetadata {
    let installation_id = crate::installation_id::resolve_installation_id()
        .await
        .unwrap_or_else(|_| format!("workspace-{}", session.workspace_id));

    crate::provider::ResponsesApiRequestMetadata {
        installation_id,
        session_id: session.id.to_string(),
        thread_id: session.id.to_string(),
        turn_id: run_id.to_string(),
        window_id: format!("{prompt_cache_key}:{prompt_window_generation}"),
        parent_thread_id: session.parent_id.map(|value| value.to_string()),
        subagent_header: session.is_subagent.then_some("collab_spawn".to_owned()),
        subagent_kind: session.is_subagent.then_some("thread_spawn".to_owned()),
        request_kind: Some("turn".to_owned()),
        turn_started_at_unix_ms: Some(turn_started_at_unix_ms),
        extra: Default::default(),
    }
}

fn managed_project_state_permission(workspace_root: &Path) -> crate::agent::PermissionConfig {
    let managed_root = crate::project_paths::project_state_dir(workspace_root)
        .to_string_lossy()
        .replace('\\', "/");
    let read_write = crate::agent::PathAccessRuleConfig::Modes(crate::agent::PathAccessModes {
        read: Some(PermissionMode::Allow),
        write: Some(PermissionMode::Allow),
    });
    let mut rules = indexmap::IndexMap::new();
    rules.insert(managed_root.clone(), read_write.clone());
    rules.insert(format!("{managed_root}/**"), read_write);
    crate::agent::PermissionConfig {
        path: Some(crate::agent::PathPermissionConfig {
            rules,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[allow(dead_code)]
fn join_runtime_context_lines(lines: &[String]) -> String {
    lines
        .iter()
        .map(String::as_str)
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join("\n\n")
}

#[cfg(test)]
mod tests {
    use super::should_execute_pending_tools_concurrently;
    use crate::model::ModelSpeedModeRequestOverride;

    #[test]
    fn explicit_parallel_false_serializes_pending_tool_execution() {
        let mut disabled = ModelSpeedModeRequestOverride::default();
        disabled.set_parallel_tool_calls(Some(false));
        assert!(!should_execute_pending_tools_concurrently(&disabled));

        let enabled = {
            let mut request_override = ModelSpeedModeRequestOverride::default();
            request_override.set_parallel_tool_calls(Some(true));
            request_override
        };
        assert!(should_execute_pending_tools_concurrently(&enabled));
        assert!(should_execute_pending_tools_concurrently(
            &ModelSpeedModeRequestOverride::default()
        ));
    }
}
