use std::path::Path;

use super::{ConversationIdentity, ExecutionConversationTarget, StableRunContext};
use crate::session::Session;
use crate::session::model::SessionPartRef;
use agena_domain::{ToolApiFunction, UserInputReply};
use agena_provider::ResponsesApiRequestMetadata;
use agena_tool::ToolPermissionCheck;

mod replies_execution;
mod replies_state;
mod tool_failure;
mod tool_non_execution;

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
    Deny(Box<agena_domain::PolicyDeniedResult>),
}

enum PendingReplyLookup<P> {
    Pending(P),
    Duplicate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReplyExecutionMode {
    Await,
    Start,
}

enum ReplyDispatch {
    Completed(Box<Session>),
    Accepted(crate::SessionExecutionCommandOutcome),
}

/// Inputs specific to continuing a session after a permission grant.
///
/// These values are consumed as one transaction: separating them into a long
/// parameter list made it too easy to couple unrelated continuation details.
struct ApprovedPermissionContinuation {
    session_id: i64,
    conversation_identity: ConversationIdentity,
    options: SessionRunOptions,
    model_turn_id: i64,
    state: Arc<SessionManagerState>,
    pending_tool: SessionPendingTool,
    resolved_tool: ResolvedPendingTool,
    continue_model: bool,
}

/// Stable identity and execution context for continuing one canonical reply.
///
/// Keeping this data together prevents permission and user-input continuations
/// from accidentally mixing a turn/reply identity with another execution's
/// source, model turn, or runtime state.
struct ReplySessionContinuation {
    session_id: i64,
    conversation_identity: ConversationIdentity,
    options: SessionRunOptions,
    run_source: ExecutionSource,
    model_turn_id: i64,
    state: Arc<SessionManagerState>,
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
    authorization: agena_domain::OperationAuthorization,
) -> OperationPart {
    let mut operation = OperationPart::pending(resolved.call_id, invocation, title, lifecycle);
    operation.authorization = authorization;
    if let Some(identity) = resolved.advertised_tool_identity.as_deref() {
        operation.set_advertised_tool_identity(identity.to_string());
    }
    operation
}

fn operation_authorization(
    session: &Session,
    resolved: &ResolvedPendingTool,
) -> agena_domain::OperationAuthorization {
    session
        .part(&resolved.pending.part)
        .and_then(|part| part.content.as_ref())
        .and_then(|content| match content {
            PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                Some(operation.authorization.clone())
            }
            _ => None,
        })
        .unwrap_or_default()
}

/// Stable terminal identity for an Operation. Approval-phase prose and the
/// Tool API gateway are never allowed to become the final title.
fn terminal_operation_title(invocation: &ToolInvocation) -> String {
    let function = invocation
        .tool_api_call
        .as_ref()
        .map(|call| call.function)
        .or_else(|| ToolApiFunction::from_function_name(invocation.name.as_str()));
    match function {
        Some(ToolApiFunction::List) => "List tools".to_owned(),
        Some(ToolApiFunction::Search) => "Search tools".to_owned(),
        Some(ToolApiFunction::Help) => "Inspect tool".to_owned(),
        Some(ToolApiFunction::Tags) => "List tool tags".to_owned(),
        Some(ToolApiFunction::Call) => invocation.name.clone(),
        None => tool_name(invocation),
    }
}

fn is_authorization_phase_title(title: &str) -> bool {
    let title = title.trim().to_ascii_lowercase();
    title.starts_with("awaiting permission")
        || title.starts_with("awaiting approval")
        || title.starts_with("awaiting user approval")
        || title.starts_with("permission request")
}

fn append_resolved_message_part(
    session: &mut Session,
    resolved: &ResolvedPendingTool,
    mut part: MessagePart,
) -> Result<Message, AppError> {
    let message = session
        .messages
        .get_mut(resolved.pending.part.message_index)
        .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
    part.part_index = i32::try_from(message.parts.len()).map_err(|_| {
        AppError::Internal(format!(
            "message {} has too many parts to append an interaction activity",
            message.id
        ))
    })?;
    message.parts.push(part);
    assistant_message_for_part(session, &resolved.pending.part)
}

fn interactive_request_kind_label(
    request_kind: agena_domain::PendingInteractiveRequestKind,
) -> &'static str {
    match request_kind {
        agena_domain::PendingInteractiveRequestKind::Permission => "permission",
        agena_domain::PendingInteractiveRequestKind::UserInput => "user input",
    }
}

fn matching_request_part_refs(
    session: &Session,
    request_id: &str,
    request_kind: agena_domain::PendingInteractiveRequestKind,
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

                    let _operation_id = part.operation_id.as_deref()?;
                    let matches_request = match (request_kind, part.content.as_ref()) {
                        (
                            agena_domain::PendingInteractiveRequestKind::UserInput,
                            Some(PartContent::Activity(
                                crate::message::RuntimeActivity::Interaction(
                                    RequestPart::UserInput(request),
                                ),
                            )),
                        ) => request.request_id() == request_id,
                        _ => false,
                    };
                    matches_request.then_some(SessionPartRef {
                        message_index,
                        part_index,
                        message_id: message.id,
                        part_id: part.id,
                    })
                })
        })
        .collect()
}

/// A request Activity is a child of exactly one tool operation. Once that
/// operation reaches any terminal state, leaving an unanswered child request
/// pending would create an unanswerable approval: the UI can still submit it,
/// but there is no pending tool left to resume. Close such children in the
/// same message checkpoint as the operation's terminal result.
pub(super) fn cancel_unanswered_request_parts_for_operation(
    session: &mut Session,
    operation_id: &str,
) -> Result<Vec<i64>, AppError> {
    let request_parts = session
        .messages
        .iter()
        .enumerate()
        .flat_map(|(message_index, message)| {
            message
                .parts
                .iter()
                .enumerate()
                .filter_map(move |(part_index, part)| {
                    (part.status == ExecutionStatus::Pending
                        && part.operation_id.as_deref() == Some(operation_id)
                        && matches!(
                            part.content,
                            Some(PartContent::Activity(
                                crate::message::RuntimeActivity::Interaction(_)
                            ))
                        ))
                    .then_some(SessionPartRef {
                        message_index,
                        part_index,
                        message_id: message.id,
                        part_id: part.id,
                    })
                })
        })
        .collect::<Vec<_>>();

    let changed_part_ids = request_parts
        .iter()
        .map(|request_part| request_part.part_id)
        .collect::<Vec<_>>();
    for request_part in request_parts {
        let part = session.part_mut(&request_part).ok_or_else(|| {
            AppError::Internal(format!(
                "pending interaction part not found while closing operation {operation_id}: message={}, part={}",
                request_part.message_id, request_part.part_id
            ))
        })?;
        part.status = ExecutionStatus::Cancelled;
        part.summary = Some(
            "Cancelled because the associated tool already reached a terminal result.".to_owned(),
        );
    }
    Ok(changed_part_ids)
}

fn supersede_duplicate_request_parts(
    session: &mut Session,
    request_parts: &[SessionPartRef],
    request_kind: agena_domain::PendingInteractiveRequestKind,
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
    resolved_adapter_id: Option<&agena_domain::AdapterId>,
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

fn matching_model_turn_id(
    session: &Session,
    model_turn_id: i64,
    options: &SessionRunOptions,
) -> Option<i64> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == Role::Assistant && message.metadata.model_turn_id == Some(model_turn_id)
        })
        .filter(|message| {
            message.metadata.model_provider_id == options.model.provider_id.as_ref()
                && message.metadata.model_adapter_id.as_deref()
                    == options.model.adapter_id.as_ref().map(AsRef::as_ref)
                && message.metadata.model_id == options.model.model_id.as_ref()
        })
        .map(|_| model_turn_id)
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
        request_kind: agena_domain::PendingInteractiveRequestKind,
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
        request_kind: agena_domain::PendingInteractiveRequestKind,
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
        part.set_content(PartContent::Activity(
            crate::message::RuntimeActivity::Interaction(request),
        ));
        part.status = status;
        Ok(Some(assistant_message_for_part(
            session,
            &resolved.pending.part,
        )?))
    }

    async fn load_reply_session(
        &self,
        session_id: i64,
    ) -> Result<(Arc<SessionManagerState>, Session), AppError> {
        let state = self.execution_state();
        let session = self
            .store
            .load_session(session_id, state.cache_policy())
            .await?;
        Ok((state, session))
    }

    async fn resume_active_reply_execution(
        &self,
        session: &Session,
        conversation_identity: ConversationIdentity,
        mode: ReplyExecutionMode,
    ) -> Option<ReplyDispatch> {
        let control = self
            .execution_registry
            .signal_interaction_for_reply(session.id, conversation_identity.reply_id)
            .await?;
        Some(match mode {
            ReplyExecutionMode::Await => ReplyDispatch::Completed(Box::new(session.clone())),
            ReplyExecutionMode::Start => {
                ReplyDispatch::Accepted(crate::SessionExecutionCommandOutcome::accepted(
                    session.id,
                    control.execution_id(),
                    control.turn_id(),
                    control.reply_id(),
                ))
            }
        })
    }

    async fn dispatch_reply_session(
        &self,
        mut session: Session,
        continuation: ReplySessionContinuation,
        mode: ReplyExecutionMode,
    ) -> Result<ReplyDispatch, AppError> {
        let ReplySessionContinuation {
            session_id,
            conversation_identity,
            options,
            run_source,
            model_turn_id,
            state,
        } = continuation;
        let options = self.apply_execution_context_to_run_options(&session, options)?;
        let continuation_model_turn_id = matching_model_turn_id(&session, model_turn_id, &options);
        if self.apply_run_selection_to_session(&mut session, &options) {
            session = self
                .persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }

        let operation = move |manager: SessionManager, control: Arc<ExecutionControl>, steer_rx| async move {
            // `run_until_stable` owns the complete provider/tool state
            // machine. Keep that large future behind one heap boundary so a
            // permission continuation does not embed the entire machine in a
            // Tokio worker's comparatively small stack frame.
            Box::pin(manager.run_until_stable(
                session,
                &options,
                StableRunContext {
                    allow_goal_continuation: false,
                    base_run_source: run_source,
                    active_model_turn_id: continuation_model_turn_id,
                    state,
                    control,
                    steer_rx,
                    usage_budget: None,
                },
            ))
            .await
        };
        match mode {
            ReplyExecutionMode::Await => Box::pin(self.execute_registered(
                session_id,
                run_source,
                ExecutionConversationTarget::ExistingReply(conversation_identity),
                "reply continuation execution",
                operation,
            ))
            .await
            .map(|session| ReplyDispatch::Completed(Box::new(session))),
            ReplyExecutionMode::Start => Box::pin(self.start_registered(
                session_id,
                run_source,
                ExecutionConversationTarget::ExistingReply(conversation_identity),
                "reply continuation execution",
                operation,
            ))
            .await
            .map(ReplyDispatch::Accepted),
        }
    }

    async fn dispatch_approved_permission_session(
        &self,
        mut session: Session,
        continuation: ApprovedPermissionContinuation,
        mode: ReplyExecutionMode,
    ) -> Result<ReplyDispatch, AppError> {
        let ApprovedPermissionContinuation {
            session_id,
            conversation_identity,
            options,
            model_turn_id,
            state,
            pending_tool,
            resolved_tool,
            continue_model,
        } = continuation;
        let options = if continue_model {
            self.apply_execution_context_to_run_options(&session, options)?
        } else {
            options
        };
        let continuation_model_turn_id = if continue_model {
            matching_model_turn_id(&session, model_turn_id, &options)
        } else {
            None
        };
        if continue_model && self.apply_run_selection_to_session(&mut session, &options) {
            self.persist_session_changes(session, Vec::new(), Vec::new(), None, state.clone())
                .await?;
        }

        let operation = move |manager: SessionManager, control: Arc<ExecutionControl>, steer_rx| async move {
            let execution_manager = manager.background_handle();
            let execution_state = state.clone();
            let execution_tool = resolved_tool.clone();
            let cancellation = control.cancel.clone();
            let execution = tokio::task::spawn_blocking(move || {
                execution_manager.execute_pending_tool_after_approval(
                    execution_state.as_ref(),
                    session_id,
                    &execution_tool,
                    Some(cancellation),
                )
            })
            .await
            .map_err(|error| AppError::Internal(format!("approved tool task failed: {error}")))?;
            // A plugin may have persisted nested interaction state while the
            // blocking invocation was in flight. Apply the terminal result to
            // the latest projection, exactly as ordinary tool execution does.
            let session = manager
                .store
                .load_session(session_id, state.cache_policy())
                .await?;
            let session = manager
                .apply_tool_execution_result(session, &pending_tool, execution, state.clone())
                .await?;
            if !continue_model {
                return Ok(session);
            }
            Box::pin(manager.run_until_stable(
                session,
                &options,
                StableRunContext {
                    allow_goal_continuation: false,
                    base_run_source: ExecutionSource::PermissionReply,
                    active_model_turn_id: continuation_model_turn_id,
                    state,
                    control,
                    steer_rx,
                    usage_budget: None,
                },
            ))
            .await
        };
        match mode {
            ReplyExecutionMode::Await => Box::pin(self.execute_registered(
                session_id,
                ExecutionSource::PermissionReply,
                ExecutionConversationTarget::ExistingReply(conversation_identity),
                "approved permission execution",
                operation,
            ))
            .await
            .map(|session| ReplyDispatch::Completed(Box::new(session))),
            ReplyExecutionMode::Start => Box::pin(self.start_registered(
                session_id,
                ExecutionSource::PermissionReply,
                ExecutionConversationTarget::ExistingReply(conversation_identity),
                "approved permission execution",
                operation,
            ))
            .await
            .map(ReplyDispatch::Accepted),
        }
    }

    async fn persist_tool_completion(
        &self,
        mut session: Session,
        _assistant_message: Message,
        resolved: &ResolvedPendingTool,
        persisted_rules: Vec<PersistedPermissionRule>,
        mut terminal_events: Vec<EventKind>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let tool_call_id = tool_call_id_for(resolved);
        let mut changed_part_ids =
            cancel_unanswered_request_parts_for_operation(&mut session, tool_call_id.as_ref())?;
        changed_part_ids.push(resolved.pending.part.part_id);
        let assistant_message = assistant_message_for_part(&session, &resolved.pending.part)?;
        let completed_part = assistant_message
            .parts
            .iter()
            .find(|part| {
                matches!(
                    part.content.as_ref(),
                    Some(PartContent::Activity(
                        crate::message::RuntimeActivity::Operation(_)
                    ))
                ) && part.operation_id.as_deref() == Some(tool_call_id.as_ref())
            })
            .cloned()
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "completed operation part missing for tool call {}",
                    tool_call_id
                ))
            })?;
        let session = self
            .persist_session_changes_with_rules(
                session,
                vec![MessageCheckpoint::parts(
                    assistant_message.id,
                    changed_part_ids,
                )],
                Vec::new(),
                persisted_rules,
                state.clone(),
            )
            .await?;
        let mut events = vec![EventKind::ToolCallCompleted(ToolCallCompleted {
            message_id: HistoryMessageId(assistant_message.id),
            call_id: tool_call_id,
            run_id: HistoryRunId::new(),
            tool_name: resolved.invocation.name.clone().into(),
            part: completed_part,
            completed_at: Utc::now(),
        })];
        events.append(&mut terminal_events);
        self.store
            .append_history_items(session, events, state.cache_policy())
            .await
    }

    async fn reply_permission_dispatch(
        &self,
        request: SessionPermissionReplyRequest,
        mode: ReplyExecutionMode,
    ) -> Result<ReplyDispatch, AppError> {
        let request_id = request.request.reply.request_id.clone();
        let reply_lock = self.reply_session_lock(request.request.session_id).await;
        let reply_guard = reply_lock.lock().await;
        let (state, mut session) = self.load_reply_session(request.request.session_id).await?;
        let pending = match self.lookup_pending_reply(
            &session,
            request.request.session_id,
            request_id.as_str(),
            "permission",
            Session::find_pending_permission_by_request_id,
            Session::has_replied_permission_request,
        )? {
            PendingReplyLookup::Pending(pending) => pending,
            PendingReplyLookup::Duplicate => {
                return Ok(ReplyDispatch::Completed(Box::new(session)));
            }
        };

        let permission_request = self.clone_pending_reply_request(
            &session,
            &pending,
            request_id.as_str(),
            "permission",
            |session, pending| session.pending_permission_request(pending).cloned(),
        )?;
        let replied_at_ms = Utc::now().timestamp_millis();
        let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
        let operation_id = resolved_tool.operation_id.clone();
        let call_id = resolved_tool.call_id;
        {
            let tool_part = session
                .part_mut(&pending.tool.part)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending.tool.part))?;
            let Some(PartContent::Activity(crate::message::RuntimeActivity::Operation(operation))) =
                tool_part.content.as_mut()
            else {
                return Err(pending_reply_payload_missing_error(
                    "permission",
                    request_id.as_str(),
                ));
            };
            if !operation
                .authorization
                .record_reply(request.request.reply.clone(), replied_at_ms)
            {
                return Err(pending_reply_payload_missing_error(
                    "permission",
                    request_id.as_str(),
                ));
            }
            let decision = match request.request.reply.kind {
                PermissionReplyKind::AllowOnce => "Permission allowed once",
                PermissionReplyKind::AllowAlways => "Permission allowed always",
                PermissionReplyKind::DenyOnce => "Permission denied once",
                PermissionReplyKind::DenyAlways => "Permission denied always",
            };
            let summary = request
                .request
                .reply
                .reason
                .as_deref()
                .filter(|reason| !reason.trim().is_empty())
                .map(|reason| format!("{decision} · {reason}"))
                .unwrap_or_else(|| decision.to_owned());
            operation.set_summary(summary);
            tool_part.summary = Some(operation.summary.clone());
        }
        let replied_assistant_message = assistant_message_for_part(&session, &pending.tool.part)?;
        let conversation_identity = self
            .conversation_identity_for_message(
                request.request.session_id,
                pending.tool.part.message_id,
            )
            .await?;
        let reply_model_turn_id = replied_assistant_message
            .metadata
            .model_turn_id
            .ok_or_else(|| {
                AppError::Internal(format!(
                    "assistant message {} owning permission {} has no turn id",
                    replied_assistant_message.id, request_id
                ))
            })?;

        // Only a genuine provider Tool API call may resume the model after
        // the approved target completes. A manually constructed or
        // application-originated operation has no provider call to replay;
        // treating it as one can re-enter the response loop without a model
        // turn (and used to permit legacy external approval paths).
        let continue_model = !replied_assistant_message.metadata.externally_initiated_tool
            && resolve_pending_tool(&session, &pending.tool)?
                .invocation
                .tool_api_call
                .is_some();
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
                vec![MessageCheckpoint::part(
                    replied_assistant_message.id,
                    pending.tool.part.part_id,
                )],
                vec![EventKind::PermissionReplied(PermissionRepliedEvent {
                    session_id: request.request.session_id,
                    operation_id,
                    call_id,
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

        // The reply is now durable, so another reply will observe it as a
        // duplicate. Refresh the derived pending state before deciding whether
        // this reply is the batch barrier or merely one member of it.
        session.refresh_derived();

        // Release the per-session serialization lock only after the durable
        // reply and its derived state agree. Concurrent approval commands can
        // now collect decisions independently without registering competing
        // reply executions.
        drop(reply_guard);

        match request.request.reply.kind {
            PermissionReplyKind::AllowOnce | PermissionReplyKind::AllowAlways => {
                if let Some(dispatch) = self
                    .resume_active_reply_execution(&session, conversation_identity, mode)
                    .await
                {
                    return Ok(dispatch);
                }
                if continue_model {
                    // Provider tool batches resume as one canonical reply
                    // execution. Until every interaction has a durable reply,
                    // do not execute an approved member in isolation: doing so
                    // serializes otherwise parallel tools and lets one model
                    // continuation race the remaining approvals.
                    if session.blocked() {
                        return Ok(ReplyDispatch::Completed(Box::new(session)));
                    }
                    return Box::pin(self.dispatch_reply_session(
                        session,
                        ReplySessionContinuation {
                            session_id: request.request.session_id,
                            conversation_identity,
                            options: request.request.options,
                            run_source: ExecutionSource::PermissionReply,
                            model_turn_id: reply_model_turn_id,
                            state,
                        },
                        mode,
                    ))
                    .await;
                }

                let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
                return self
                    .dispatch_approved_permission_session(
                        session,
                        ApprovedPermissionContinuation {
                            session_id: request.request.session_id,
                            conversation_identity,
                            options: request.request.options,
                            model_turn_id: reply_model_turn_id,
                            state,
                            pending_tool: pending.tool,
                            resolved_tool,
                            continue_model,
                        },
                        mode,
                    )
                    .await;
            }
            PermissionReplyKind::DenyOnce | PermissionReplyKind::DenyAlways => {
                let decline = agena_domain::UserDeclinedResult {
                    request_id: request_id.clone(),
                    action: permission_request.action.clone(),
                    related_actions: permission_request.related_actions.clone(),
                    reason: request.request.reply.reason.clone(),
                    persisted_scope: matches!(
                        request.request.reply.kind,
                        PermissionReplyKind::DenyAlways
                    )
                    .then_some(request.request.reply.scope)
                    .flatten(),
                };
                session = self
                    .apply_tool_user_declined(
                        session,
                        &pending.tool,
                        decline,
                        Vec::new(),
                        state.clone(),
                    )
                    .await?;
                session.refresh_derived();
            }
        }

        if let Some(dispatch) = self
            .resume_active_reply_execution(&session, conversation_identity, mode)
            .await
        {
            return Ok(dispatch);
        }

        if !continue_model {
            return Ok(ReplyDispatch::Completed(Box::new(session)));
        }

        // A denial terminalizes its own Operation, but it must not resume the
        // model while sibling Operation authorization or UserInput remains unresolved.
        // The final interaction reply crosses the same batch barrier and owns
        // the single continuation execution.
        if session.blocked() {
            return Ok(ReplyDispatch::Completed(Box::new(session)));
        }

        Box::pin(self.dispatch_reply_session(
            session,
            ReplySessionContinuation {
                session_id: request.request.session_id,
                conversation_identity,
                options: request.request.options,
                run_source: ExecutionSource::PermissionReply,
                model_turn_id: reply_model_turn_id,
                state,
            },
            mode,
        ))
        .await
    }

    pub async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        match self
            .reply_permission_dispatch(request, ReplyExecutionMode::Await)
            .await?
        {
            ReplyDispatch::Completed(session) => Ok(*session),
            ReplyDispatch::Accepted(_) => Err(AppError::Internal(
                "awaited permission reply returned an accepted receipt".to_owned(),
            )),
        }
    }

    pub async fn start_reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        match self
            .reply_permission_dispatch(request, ReplyExecutionMode::Start)
            .await?
        {
            ReplyDispatch::Completed(session) => {
                Ok(crate::SessionExecutionCommandOutcome::completed(session.id))
            }
            ReplyDispatch::Accepted(outcome) => Ok(outcome),
        }
    }

    async fn reply_user_input_dispatch(
        &self,
        request: SessionExecutionReplyRequest<UserInputReply>,
        mode: ReplyExecutionMode,
    ) -> Result<ReplyDispatch, AppError> {
        let request_id = request.reply.request_id.clone();
        let reply_lock = self.reply_session_lock(request.session_id).await;
        let reply_guard = reply_lock.lock().await;
        let (state, mut session) = self.load_reply_session(request.session_id).await?;
        let pending = match self.lookup_pending_reply(
            &session,
            request.session_id,
            request_id.as_str(),
            "user input",
            Session::find_pending_user_input_by_request_id,
            Session::has_replied_user_input_request,
        )? {
            PendingReplyLookup::Pending(pending) => pending,
            PendingReplyLookup::Duplicate => {
                return Ok(ReplyDispatch::Completed(Box::new(session)));
            }
        };
        let replied_assistant_message = assistant_message_for_part(&session, &pending.tool.part)?;
        let reply_model_turn_id = replied_assistant_message.metadata.model_turn_id;

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
            agena_domain::PendingInteractiveRequestKind::UserInput,
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
                    vec![MessageCheckpoint::part(
                        assistant_message.id,
                        pending.request.part_id,
                    )],
                    Vec::new(),
                    None,
                    state.clone(),
                )
                .await?;
            // Wake the suspended host call only after its reply is durable,
            // but never while holding the session reply lock. The resumed tool
            // may immediately issue another interactive host request.
            drop(reply_guard);
            if let Some(waiter) = self
                .host_user_input_waiters
                .lock()
                .await
                .remove(request_id.as_str())
            {
                let _ = waiter.response.send(response);
                return Ok(ReplyDispatch::Completed(Box::new(session)));
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
                        .apply_user_declined(session, &pending.tool, reason, state.clone())
                        .await?;
                }
                UserInputReplyKind::Timeout => {
                    let execution = crate::tool::ask_user::execution_from_timeout(
                        &crate::message::AskUserToolInput {
                            title: user_input_request.title.clone(),
                            body_markdown: user_input_request.body_markdown.clone(),
                            kind: user_input_request.kind.clone(),
                            submit_label: user_input_request.submit_label.clone(),
                            cancel_label: user_input_request.cancel_label.clone(),
                            auto_resolution_ms: user_input_request.auto_resolution_ms,
                            questions: user_input_request.questions.clone(),
                        },
                    );
                    session = self
                        .apply_tool_success(
                            session,
                            &pending.tool,
                            execution.into(),
                            None,
                            state.clone(),
                        )
                        .await?;
                }
            }
            drop(reply_guard);
        }

        // A live host request resumes the already-running tool through its
        // waiter and returns above; it never needs a second session execution.
        // Only replay after a lost waiter, or an ordinary model user-input
        // Activity, reaches this continuation boundary. Resolve canonical
        // ownership here so an in-flight host reply cannot fail merely because
        // the Activity projection trails its durable model-message checkpoint.
        let conversation_identity = self
            .conversation_identity_for_message(request.session_id, pending.request.message_id)
            .await?;
        let reply_model_turn_id = reply_model_turn_id.ok_or_else(|| {
            AppError::Internal(format!(
                "assistant message {} owning user input {} has no turn id",
                pending.tool.part.message_id, request_id
            ))
        })?;

        if let Some(dispatch) = self
            .resume_active_reply_execution(&session, conversation_identity, mode)
            .await
        {
            return Ok(dispatch);
        }

        Box::pin(self.dispatch_reply_session(
            session,
            ReplySessionContinuation {
                session_id: request.session_id,
                conversation_identity,
                options: request.options,
                run_source: ExecutionSource::UserInputReply,
                model_turn_id: reply_model_turn_id,
                state,
            },
            mode,
        ))
        .await
    }

    pub async fn reply_user_input(
        &self,
        request: SessionExecutionReplyRequest<UserInputReply>,
    ) -> Result<Session, AppError> {
        match self
            .reply_user_input_dispatch(request, ReplyExecutionMode::Await)
            .await?
        {
            ReplyDispatch::Completed(session) => Ok(*session),
            ReplyDispatch::Accepted(_) => Err(AppError::Internal(
                "awaited user-input reply returned an accepted receipt".to_owned(),
            )),
        }
    }

    pub async fn start_reply_user_input(
        &self,
        request: SessionExecutionReplyRequest<UserInputReply>,
    ) -> Result<crate::SessionExecutionCommandOutcome, AppError> {
        match self
            .reply_user_input_dispatch(request, ReplyExecutionMode::Start)
            .await?
        {
            ReplyDispatch::Completed(session) => {
                Ok(crate::SessionExecutionCommandOutcome::completed(session.id))
            }
            ReplyDispatch::Accepted(outcome) => Ok(outcome),
        }
    }

    pub(super) fn execution_state(&self) -> Arc<SessionManagerState> {
        self.execution.load_full()
    }
}

async fn responses_api_request_metadata(
    session: &Session,
    prompt_cache_key: &str,
    prompt_window_generation: u64,
    run_id: agena_domain::RunId,
    turn_started_at_unix_ms: i64,
) -> ResponsesApiRequestMetadata {
    let installation_id = agena_runtime::resolve_installation_id()
        .await
        .unwrap_or_else(|_| format!("workspace-{}", session.workspace_id));

    ResponsesApiRequestMetadata {
        installation_id,
        session_id: session.id.to_string(),
        thread_id: session.id.to_string(),
        turn_id: run_id.to_string(),
        window_id: format!("{prompt_cache_key}:{prompt_window_generation}"),
        parent_thread_id: session.parent_id.map(|value| value.to_string()),
        subagent_header: session.is_subagent().then_some("collab_spawn".to_owned()),
        subagent_kind: session.is_subagent().then_some("thread_spawn".to_owned()),
        request_kind: Some("turn".to_owned()),
        turn_started_at_unix_ms: Some(turn_started_at_unix_ms),
        extra: Default::default(),
    }
}

fn managed_project_state_permission(
    workspace_root: &Path,
) -> crate::authorization::PermissionConfig {
    let managed_root = agena_runtime::project_state_dir(workspace_root)
        .to_string_lossy()
        .replace('\\', "/");
    let read_write =
        crate::authorization::PathAccessRuleConfig::Modes(crate::authorization::PathAccessModes {
            read: Some(PermissionMode::Allow),
            write: Some(PermissionMode::Allow),
        });
    let mut rules = indexmap::IndexMap::new();
    rules.insert(managed_root.clone(), read_write.clone());
    rules.insert(format!("{managed_root}/**"), read_write);
    crate::authorization::PermissionConfig {
        path: Some(crate::authorization::PathPermissionConfig {
            rules,
            ..Default::default()
        }),
        ..Default::default()
    }
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::{matching_model_turn_id, should_execute_pending_tools_concurrently};
    use crate::{message::Message, session::Session};
    use agena_domain::{ModelRef, ModelSpeedModeRequestOverride, Role};
    use agena_runtime::SessionRunOptions;

    fn run_options(model: ModelRef) -> SessionRunOptions {
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
        }
    }

    fn session_with_assistant_turn(turn_id: i64, model: &ModelRef) -> Session {
        let now = chrono::Utc::now();
        let mut session = Session::new(1, 1, "test", now);
        let mut message = Message::prompt_text(Role::Assistant, "pending continuation");
        message.id = 10;
        message.metadata.model_turn_id = Some(turn_id);
        message.metadata.model_provider_id = model.provider_id.to_string();
        message.metadata.model_adapter_id = model.adapter_id.as_ref().map(ToString::to_string);
        message.metadata.model_id = model.model_id.to_string();
        session.messages.push(message);
        session
    }

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

    #[test]
    fn interactive_continuation_preserves_turn_for_the_same_model_route() {
        let model = ModelRef::new_with_adapter("openai", "responses", "gpt-test");
        let session = session_with_assistant_turn(7, &model);

        assert_eq!(
            matching_model_turn_id(&session, 7, &run_options(model)),
            Some(7)
        );
    }

    #[test]
    fn interactive_continuation_starts_a_new_turn_after_model_route_change() {
        let original = ModelRef::new_with_adapter("openai", "responses", "gpt-test");
        let session = session_with_assistant_turn(7, &original);

        assert_eq!(
            matching_model_turn_id(
                &session,
                7,
                &run_options(ModelRef::new_with_adapter(
                    "openai",
                    "responses",
                    "gpt-other",
                )),
            ),
            None
        );
    }
}
use super::{
    AppError, Arc, DecisionTraceStep, EventKind, ExecutionControl, ExecutionSource,
    ExecutionStatus, HistoryMessageId, HistoryRunId, InteractiveRequestPart, Message,
    MessageCheckpoint, MessageMetadata, MessagePart, MessageSource, ModelRef,
    ModelSpeedModeRequestOverride, OperationPart, PartContent, PathBuf, PermissionAction,
    PermissionMode, PermissionRepliedEvent, PermissionReplyKind, PermissionRiskLevel,
    PermissionScope, PersistedPermissionRule, PromptRequestOptions, PromptTurnBudget,
    ProviderPromptAnchor, RequestPart, ResolvedPendingTool, Role, RunAborted, RunCompleted,
    RunStarted, SessionCommit, SessionExecutionReplyRequest, SessionManager, SessionManagerState,
    SessionPendingTool, SessionPermissionReplyRequest, SessionRunOptions, SessionRunRequest,
    SessionRunTermination, StreamingToolExecution, TimeRange, ToolCallCompleted, ToolError,
    ToolInvocation, ToolInvocationExecution, UserInputReplyKind, Utc, ask_user_title,
    build_message, build_request_part, completed_lifecycle, custom_payload_value,
    execution_control_to_app_error, host_user_input_response, max_permission_risk,
    merge_system_prompts, mpsc, operation_blocks_from_tool_output,
    payload_tool_name_for_invocation, permission_action_key, permission_scope_label,
    persisted_rules_for_reply, resolve_pending_tool, resolve_permission_with_persisted_rules,
    run_abort_reason, text_result_blocks, tool_call_id_for, tool_name, user_input_execution,
};
