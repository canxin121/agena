use std::path::Path;

use super::{ConversationIdentity, ExecutionConversationTarget, StableRunContext};
use crate::session::Session;
use crate::session::model::{
    SessionPartRef, SessionPendingInteractiveRequest, SessionPendingPermissionRequest,
    SessionPendingTool,
};
use crate::session::store::{OPERATION_ID_METADATA_KEY, part_content_from_value};
use agena_domain::UserInputReply;
use agena_provider::ResponsesApiRequestMetadata;
use agena_storage::store::{Part, PartRole, PartState};
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
        "pending tool part not found: part={}",
        part_ref.part_id
    ))
}

/// The run marker part owning `part_ref` (the part's `run_id`, or the part
/// itself when it is a marker). The marker's `part_id` is the durable message
/// id in v2 — the v1 bridge and the conversation identity both key off it.
pub(super) fn run_marker_for_part<'a>(
    session: &'a Session,
    part_ref: &SessionPartRef,
) -> Result<&'a Part, AppError> {
    let part = session
        .part(part_ref)
        .ok_or_else(|| pending_tool_part_not_found_error(part_ref))?;
    let run_id = part.run_id.unwrap_or(part.part_id);
    session
        .parts()
        .iter()
        .find(|candidate| candidate.part_id == run_id)
        .ok_or_else(|| {
            AppError::Internal(format!(
                "run marker {run_id} missing for part {}",
                part.part_id
            ))
        })
}

/// The durable message id (run marker part id) owning `part_ref`.
pub(super) fn assistant_message_id(
    session: &Session,
    part_ref: &SessionPartRef,
) -> Result<i64, AppError> {
    run_marker_for_part(session, part_ref).map(|marker| marker.part_id)
}

/// Whether the run marker owning `part_ref` represents an externally initiated
/// provider tool call (defaults to false; the marker content records it when
/// set). Mirrors the legacy `MessageMetadata::externally_initiated_tool`.
pub(super) fn run_marker_externally_initiated_tool(
    session: &Session,
    part_ref: &SessionPartRef,
) -> Result<bool, AppError> {
    run_marker_for_part(session, part_ref).map(|marker| {
        marker
            .content
            .get("externally_initiated_tool")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false)
    })
}

pub(super) fn update_resolved_tool_message(
    session: &mut Session,
    resolved: &ResolvedPendingTool,
    update: impl FnOnce(&mut Part),
) -> Result<i64, AppError> {
    {
        let tool_part = session
            .part_mut(&resolved.pending.part)
            .ok_or_else(|| pending_tool_part_not_found_error(&resolved.pending.part))?;
        update(tool_part);
    }
    assistant_message_id(session, &resolved.pending.part)
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
        .and_then(operation_from_part)
        .map(|operation| operation.authorization.clone())
        .unwrap_or_default()
}

/// Decode the v1 [`OperationPart`] payload from a `tool_call` part's canonical
/// content. Returns `None` for non-tool parts or undecodable payloads.
pub(super) fn operation_from_part(part: &Part) -> Option<OperationPart> {
    part_content_from_value(&part.kind, &part.content).ok().and_then(|content| match content {
        PartContent::Activity(crate::part::RuntimeActivity::Operation(operation)) => {
            Some(operation)
        }
        _ => None,
    })
}

/// The provider operation id stashed on a `tool_call` part's content metadata
/// (`OPERATION_ID_METADATA_KEY`), used to correlate interaction parts to their
/// owning operation across reloads.
pub(super) fn operation_id_from_part(part: &Part) -> Option<String> {
    operation_from_part(part).and_then(|operation| {
        operation
            .metadata
            .get(OPERATION_ID_METADATA_KEY)
            .and_then(serde_json::Value::as_str)
            .map(ToOwned::to_owned)
    })
}

/// Re-encode a mutated v1 [`OperationPart`] back onto the tool part's content
/// and return the part's changed state.
pub(super) fn apply_operation_mutation(
    part: &mut Part,
    mutation: impl FnOnce(&mut OperationPart),
) {
    let Some(mut operation) = operation_from_part(part) else {
        return;
    };
    mutation(&mut operation);
    let mut content = part.content.clone();
    content["extra"]["operation"] = serde_json::to_value(&operation)
        .expect("operation payload is always JSON serializable");
    part.content = content;
}

/// Stable terminal identity for an Operation on paths that carry no composed
/// result title (failure, non-execution, approval phase). The direct
/// execution-tool name is the fallback; success-path titles are composed as
/// "<tool> · <call summary>" by `agena_tool::compose_tool_title`.
fn terminal_operation_title(invocation: &ToolInvocation) -> String {
    invocation.name.clone()
}

fn is_authorization_phase_title(title: &str) -> bool {
    let title = title.trim().to_ascii_lowercase();
    title.starts_with("awaiting permission")
        || title.starts_with("awaiting approval")
        || title.starts_with("awaiting user approval")
        || title.starts_with("permission request")
}

/// Canonical `interaction` part content for an interactive request (permission
/// or user input). The `kind` key is what the core projection and the storage
/// presentation read (state.rs reads `content["kind"]`); the full request
/// payload rides under `request`, the tool correlation under `tool_part_id`,
/// and the reply (once recorded) under `response`.
pub(super) fn interaction_content(
    kind: &str,
    request_id: &str,
    prompt: Option<&str>,
    tool_part_id: i64,
    request: &serde_json::Value,
) -> serde_json::Value {
    let mut content = serde_json::Map::new();
    content.insert("kind".to_owned(), serde_json::Value::String(kind.to_owned()));
    content.insert(
        "request_id".to_owned(),
        serde_json::Value::String(request_id.to_owned()),
    );
    if let Some(prompt) = prompt {
        content.insert("prompt".to_owned(), serde_json::Value::String(prompt.to_owned()));
    }
    content.insert(
        "tool_part_id".to_owned(),
        serde_json::Value::Number(serde_json::Number::from(tool_part_id)),
    );
    content.insert("request".to_owned(), request.clone());
    serde_json::Value::Object(content)
}

/// The tool part id a pending interaction is correlated to, recorded under
/// `tool_part_id` when the interaction part was created.
pub(super) fn interaction_tool_part_id(part: &Part) -> Option<i64> {
    part.content
        .get("tool_part_id")
        .and_then(serde_json::Value::as_i64)
}

/// The owning operation id recorded on an interaction part.
pub(super) fn interaction_operation_id(part: &Part) -> Option<&str> {
    part.content
        .get("operation_id")
        .and_then(serde_json::Value::as_str)
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
        .parts()
        .iter()
        .enumerate()
        .filter_map(|(part_index, part)| {
            if part.kind != "interaction" {
                return None;
            }
            if pending_only && !part.state.is_in_flight() {
                return None;
            }
            let kind_matches = match request_kind {
                agena_domain::PendingInteractiveRequestKind::Permission => {
                    part.content.get("kind").and_then(serde_json::Value::as_str)
                        == Some("permission")
                }
                agena_domain::PendingInteractiveRequestKind::UserInput => {
                    part.content.get("kind").and_then(serde_json::Value::as_str)
                        != Some("permission")
                }
            };
            if !kind_matches {
                return None;
            }
            let matches_request =
                part.content.get("request_id").and_then(serde_json::Value::as_str)
                    == Some(request_id);
            matches_request.then_some(SessionPartRef {
                part_index,
                part_id: part.part_id,
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
        .parts()
        .iter()
        .enumerate()
        .filter_map(|(part_index, part)| {
            if part.kind != "interaction" || !part.state.is_in_flight() {
                return None;
            }
            (interaction_operation_id(part) == Some(operation_id)).then_some(SessionPartRef {
                part_index,
                part_id: part.part_id,
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
                "pending interaction part not found while closing operation {operation_id}: part={}",
                request_part.part_id
            ))
        })?;
        part.state = PartState::Cancelled;
        part.summary = Some(
            "Cancelled because the associated tool already reached a terminal result.".to_owned(),
        );
    }
    Ok(changed_part_ids)
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

pub(super) fn matching_model_turn_id(
    session: &Session,
    model_turn_id: i64,
    options: &SessionRunOptions,
) -> Option<i64> {
    session
        .parts()
        .iter()
        .rev()
        .find(|part| {
            part.kind == "run"
                && part.role == PartRole::Assistant
                && part.part_id == model_turn_id
        })
        .filter(|marker| {
            marker.content.get("provider_id").and_then(serde_json::Value::as_str)
                == Some(options.model.provider_id.as_ref())
                && marker.content.get("model_id").and_then(serde_json::Value::as_str)
                    == Some(options.model.model_id.as_ref())
        })
        .map(|_| model_turn_id)
}

/// Find the pending permission whose request id matches, re-expressed over
/// parts: a `tool_call` part still in flight whose decoded operation carries a
/// pending (unanswered) authorization record with `request_id`. The returned
/// pending's `tool` references the tool part itself — the v1 semantics the
/// reply flow (recording decisions into `operation.authorization`) depends on.
pub(super) fn find_pending_permission_by_request_id(
    session: &Session,
    request_id: &str,
) -> Option<SessionPendingPermissionRequest> {
    session
        .parts()
        .iter()
        .enumerate()
        .find_map(|(part_index, part)| {
            if part.kind != "tool_call" || !part.state.is_in_flight() {
                return None;
            }
            let operation = operation_from_part(part)?;
            if !operation
                .authorization
                .awaiting()
                .any(|permission| permission.request.request_id == request_id)
            {
                return None;
            }
            Some(SessionPendingPermissionRequest {
                request_id: request_id.to_owned(),
                tool: SessionPendingTool {
                    part: SessionPartRef {
                        part_index,
                        part_id: part.part_id,
                    },
                },
            })
        })
}

pub(super) fn has_replied_permission_request(session: &Session, request_id: &str) -> bool {
    session.parts().iter().any(|part| {
        part.kind == "tool_call"
            && operation_from_part(part).is_some_and(|operation| {
                operation
                    .authorization
                    .find(request_id)
                    .is_some_and(|permission| permission.reply.is_some())
            })
    })
}

pub(super) fn pending_permission_request(
    session: &Session,
    pending: &SessionPendingPermissionRequest,
) -> Option<agena_domain::PermissionRequest> {
    let part = session.part(&pending.tool.part)?;
    let operation = operation_from_part(part)?;
    operation
        .authorization
        .find(pending.request_id.as_str())
        .filter(|permission| permission.reply.is_none())
        .map(|permission| permission.request.clone())
}

pub(super) fn find_tool_part_by_id(session: &Session, part_id: i64) -> Option<(usize, &Part)> {
    session
        .parts()
        .iter()
        .enumerate()
        .find(|(_, part)| part.part_id == part_id && part.kind == "tool_call")
}

/// Find the pending user-input request whose request id matches: an in-flight
/// `interaction` part (kind `!= "permission"`) carrying `request_id`. The
/// request ref is the interaction part; the tool ref resolves through the
/// `tool_part_id` recorded when the request was created.
pub(super) fn find_pending_user_input_by_request_id(
    session: &Session,
    request_id: &str,
) -> Option<SessionPendingInteractiveRequest> {
    session
        .parts()
        .iter()
        .enumerate()
        .find_map(|(part_index, part)| {
            if part.kind != "interaction" || !part.state.is_in_flight() {
                return None;
            }
            if part.content.get("kind").and_then(serde_json::Value::as_str) == Some("permission") {
                return None;
            }
            if part.content.get("request_id").and_then(serde_json::Value::as_str)
                != Some(request_id)
            {
                return None;
            }
            let request = SessionPartRef {
                part_index,
                part_id: part.part_id,
            };
            let (tool_index, tool_part) =
                interaction_tool_part_id(part).and_then(|part_id| find_tool_part_by_id(session, part_id))?;
            Some(SessionPendingInteractiveRequest {
                request,
                tool: SessionPendingTool {
                    part: SessionPartRef {
                        part_index: tool_index,
                        part_id: tool_part.part_id,
                    },
                },
            })
        })
}

pub(super) fn has_replied_user_input_request(session: &Session, request_id: &str) -> bool {
    session.parts().iter().any(|part| {
        part.kind == "interaction"
            && part.content.get("request_id").and_then(serde_json::Value::as_str)
                == Some(request_id)
            && part.content.get("response").is_some()
    })
}

pub(super) fn pending_user_input_request(
    session: &Session,
    pending: &SessionPendingInteractiveRequest,
) -> Option<agena_domain::UserInputRequest> {
    let part = session.part(&pending.request)?;
    serde_json::from_value(part.content.get("request")?.clone()).ok()
}

/// Whether the operation owning `operation_id` reached a terminal state. Used
/// to treat a reply to an already-finished operation as a duplicate.
pub(super) fn has_finished_operation(session: &Session, operation_id: &str) -> bool {
    session.parts().iter().any(|part| {
        part.kind == "tool_call"
            && operation_id_from_part(part).as_deref() == Some(operation_id)
            && part.state.is_terminal()
    })
}

/// The permission actions a run's tool operation accumulated Allow approvals
/// for, keyed by the owning run marker id + operation id (mirrors the legacy
/// `Session::operation_permission_approved_actions`).
pub(super) fn operation_permission_approved_actions(
    session: &Session,
    assistant_message_id: i64,
    operation_id: &str,
) -> Vec<agena_domain::PermissionAction> {
    session
        .parts()
        .iter()
        .filter(|part| part.run_id == Some(assistant_message_id) && part.kind == "tool_call")
        .find_map(|part| {
            if operation_id_from_part(part).as_deref() != Some(operation_id) {
                return None;
            }
            let operation = operation_from_part(part)?;
            let mut approved = Vec::new();
            for permission in &operation.authorization.permissions {
                let Some(reply) = permission.reply.as_ref() else {
                    continue;
                };
                if !matches!(
                    reply.kind,
                    agena_domain::PermissionReplyKind::AllowOnce
                        | agena_domain::PermissionReplyKind::AllowAlways
                ) {
                    continue;
                }
                let actions = if permission.request.requested_actions.is_empty() {
                    std::slice::from_ref(&permission.request.action)
                } else {
                    permission.request.requested_actions.as_slice()
                };
                for action in actions {
                    if !approved.contains(action) {
                        approved.push(action.clone());
                    }
                }
            }
            Some(approved)
        })
        .unwrap_or_default()
}

/// The `sequence_index`-th unanswered interactive user-input request owned by
/// `operation_id` (legacy `Session::user_input_request_for_operation`).
pub(super) fn user_input_request_for_operation(
    session: &Session,
    operation_id: &str,
    sequence_index: usize,
) -> Option<crate::part::InteractiveRequestPart<agena_domain::UserInputRequest, agena_domain::UserInputReply>>
{
    session
        .parts()
        .iter()
        .filter(|part| part.kind == "interaction" && interaction_operation_id(part) == Some(operation_id))
        .filter_map(|part| {
            let request: agena_domain::UserInputRequest =
                serde_json::from_value(part.content.get("request")?.clone()).ok()?;
            let reply: Option<agena_domain::UserInputReply> = part
                .content
                .get("response")
                .and_then(|value| serde_json::from_value(value.clone()).ok());
            Some(crate::part::InteractiveRequestPart { request, reply })
        })
        .nth(sequence_index)
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
            None if has_replied(session, request_id) || has_finished_operation(session, request_id) => {
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

    /// Complete every pending interaction part for `request_id` with the
    /// replied canonical content and terminal `completed` state.
    fn complete_reply_request_parts(
        &self,
        session: &mut Session,
        request_id: &str,
        request_kind: agena_domain::PendingInteractiveRequestKind,
        content: serde_json::Value,
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
            part.content = content.clone();
            part.state = PartState::Completed;
        }
        Ok(())
    }

    async fn load_reply_session(
        &self,
        session_id: i64,
    ) -> Result<(Arc<SessionManagerState>, Session), AppError> {
        let state = self.execution_state();
        let mut session = self.store.load_session(session_id).await?;
        // Permission replies can arrive after a global reload or a live
        // session overlay update. Refresh before resolving the pending tool so
        // an approval continuation cannot use a stale permission snapshot.
        self.refresh_execution_policy(&mut session, &state);
        Ok((state, session))
    }

    async fn resume_active_reply(
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
                .persist_session_changes(session, Vec::new(), None, state.clone())
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
            self.persist_session_changes(session, Vec::new(), None, state.clone())
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
            let session = manager.store.load_session(session_id).await?;
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
        resolved: &ResolvedPendingTool,
        persisted_rules: Vec<PersistedPermissionRule>,
        state: Arc<SessionManagerState>,
    ) -> Result<Session, AppError> {
        let mut changed_part_ids = cancel_unanswered_request_parts_for_operation(
            &mut session,
            resolved.operation_id.as_str(),
        )?;
        changed_part_ids.push(resolved.pending.part.part_id);
        self.persist_session_changes_with_rules(
            session,
            changed_part_ids,
            persisted_rules,
            state.clone(),
        )
        .await
    }

    async fn reply_permission_dispatch(
        &self,
        mut request: SessionPermissionReplyRequest,
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
            find_pending_permission_by_request_id,
            has_replied_permission_request,
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
            pending_permission_request,
        )?;

        // An `AutoApprove` reply asks the automatic-approval classifier to
        // decide this exact request. Classify before the reply is recorded:
        // the classifier outcome is downgraded to a one-shot Allow/Deny, and
        // a classification failure keeps the pending permission untouched so
        // the interactive client can retry or pick a manual decision.
        if request.request.reply.kind == PermissionReplyKind::AutoApprove {
            let candidate = agena_permission::ClassifierCandidate {
                action: agena_domain::ActionSpec::from_action(&permission_request.action),
                policy_reason: permission_request.reason.clone(),
            };
            let outcomes = self
                .classify_auto_candidates(
                    Some(&session),
                    &state,
                    Some(request.request.session_id),
                    vec![candidate],
                )
                .await;
            let verdict = outcomes.into_iter().next().unwrap_or_else(|| {
                Err(agena_permission::ClassifyFailure::ApprovalModelUnavailable(
                    "no classification outcome was produced".to_owned(),
                ))
            });
            match verdict {
                Ok(true) => {
                    request.request.reply.kind = PermissionReplyKind::AllowOnce;
                    request.request.reply.scope = None;
                }
                Ok(false) => {
                    request.request.reply.kind = PermissionReplyKind::DenyOnce;
                    request.request.reply.scope = None;
                }
                Err(failure) => {
                    return Err(AppError::AutoApproveClassifyFailed(failure));
                }
            }
        }

        let replied_at_ms = Utc::now().timestamp_millis();
        let resolved_tool = resolve_pending_tool(&session, &pending.tool)?;
        let _operation_id = resolved_tool.operation_id.clone();
        let _call_id = resolved_tool.call_id;
        {
            let tool_part = session
                .part_mut(&pending.tool.part)
                .ok_or_else(|| pending_tool_part_not_found_error(&pending.tool.part))?;
            let Some(mut operation) = operation_from_part(tool_part) else {
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
                // Unreachable: an AutoApprove reply is downgraded to a one-shot
                // Allow/Deny before this point. Keep a clean fallback instead of
                // panicking if a future regression skips the downgrade.
                PermissionReplyKind::AutoApprove => "Permission auto-approved",
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
            let summary = operation.summary.clone();
            apply_operation_mutation(tool_part, |op| *op = operation);
            tool_part.summary = Some(summary);
        }
        let reply_message_id = assistant_message_id(&session, &pending.tool.part)?;
        let conversation_identity = self
            .conversation_identity_for_message(request.request.session_id, reply_message_id)
            .await?;
        let reply_model_turn_id = reply_message_id;

        // Only a genuine provider Tool API call may resume the model after
        // the approved target completes. A manually constructed or
        // application-originated operation has no provider call to replay;
        // treating it as one can re-enter the response loop without a model
        // turn (and used to permit legacy external approval paths).
        let continue_model = !run_marker_externally_initiated_tool(&session, &pending.tool.part)?
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
            self,
            request.request.session_id,
            persisted_actions.as_slice(),
            &request.request.reply,
            request.operator.as_deref(),
        )
        .await?;
        session = self
            .persist_session_changes_with_rules(
                session,
                vec![pending.tool.part.part_id],
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
                    .resume_active_reply(&session, conversation_identity, mode)
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
            // Unreachable: AutoApprove is downgraded before record_reply. A
            // regression that skips the downgrade must fail cleanly instead of
            // silently executing with an unhandled kind.
            PermissionReplyKind::AutoApprove => {
                return Err(AppError::Internal(
                    "unresolved auto-approve reply reached dispatch".to_owned(),
                ));
            }
        }

        if let Some(dispatch) = self
            .resume_active_reply(&session, conversation_identity, mode)
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

    /// Record that an interactive user-input request has been shown to the
    /// user. This is the durable replacement for a client's volatile "seen"
    /// set: the acknowledgement survives restarts and is shared across
    /// clients, so a request that was never presented always auto-popups
    /// while one that was presented but remains unanswered is surfaced through
    /// a persistent attention hint instead of a forced modal.
    ///
    /// Idempotent: replaying the same `request_id` is a no-op and does not
    /// rewrite the checkpoint. Presenting an already-resolved request is also
    /// a no-op (the presentation is moot once the request is answered).
    pub async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<Session, AppError> {
        let reply_lock = self.reply_session_lock(session_id).await;
        let reply_guard = reply_lock.lock().await;
        let (state, mut session) = self.load_reply_session(session_id).await?;
        let request_parts = matching_request_part_refs(
            &session,
            request_id.as_str(),
            agena_domain::PendingInteractiveRequestKind::UserInput,
            false,
        );
        if request_parts.is_empty() {
            return Err(pending_reply_part_missing_error(
                "user input",
                request_id.as_str(),
            ));
        }

        let mut changed_part_ids = Vec::new();
        let mut presented = false;
        for request_part in &request_parts {
            let part = session.part_mut(request_part).ok_or_else(|| {
                pending_reply_part_missing_error("user input", request_id.as_str())
            })?;
            let Some(mut request) = part
                .content
                .get("request")
                .and_then(|value| serde_json::from_value::<agena_domain::UserInputRequest>(value.clone()).ok())
            else {
                continue;
            };
            if part.state == PartState::Pending && request.presented_at.is_none() {
                request.presented_at = Some(Utc::now());
                part.content["request"] =
                    serde_json::to_value(&request).expect("user input request is JSON serializable");
                presented = true;
                changed_part_ids.push(request_part.part_id);
            }
        }

        if !presented {
            // Already presented (idempotent replay) or already resolved
            // (presentation is moot): nothing to persist.
            return Ok(session);
        }
        session = self
            .persist_session_changes(session, changed_part_ids, None, state.clone())
            .await?;
        drop(reply_guard);
        Ok(session)
    }

    pub async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<Session, AppError> {
        match Box::pin(self.reply_permission_dispatch(request, ReplyExecutionMode::Await)).await? {
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
        match Box::pin(self.reply_permission_dispatch(request, ReplyExecutionMode::Start)).await? {
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
            find_pending_user_input_by_request_id,
            has_replied_user_input_request,
        )? {
            PendingReplyLookup::Pending(pending) => pending,
            PendingReplyLookup::Duplicate => {
                return Ok(ReplyDispatch::Completed(Box::new(session)));
            }
        };
        // The run marker owning the pending tool carries the durable message id
        // and model turn id for the continuation (v2 collapses both onto the
        // marker's part id).
        let reply_model_turn_id = assistant_message_id(&session, &pending.tool.part)?;

        let user_input_request = self.clone_pending_reply_request(
            &session,
            &pending,
            request_id.as_str(),
            "user input",
            pending_user_input_request,
        )?;
        let mut replied_content = interaction_content(
            "ask_user",
            request_id.as_str(),
            (!user_input_request.title.is_empty())
                .then_some(user_input_request.title.as_str()),
            pending.tool.part.part_id,
            &serde_json::to_value(&user_input_request)
                .expect("UserInputRequest is always JSON serializable"),
        );
        replied_content["response"] = serde_json::to_value(&request.reply)
            .expect("UserInputReply is always JSON serializable");
        self.complete_reply_request_parts(
            &mut session,
            request_id.as_str(),
            agena_domain::PendingInteractiveRequestKind::UserInput,
            replied_content,
        )?;

        let is_host_request = request_id.starts_with("host-input:");
        if is_host_request {
            let response = host_user_input_response(&user_input_request, &request.reply)?;
            session = self
                .persist_session_changes(
                    session,
                    vec![pending.request.part_id],
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
                        &crate::part::AskUserToolInput {
                            title: user_input_request.title.clone(),
                            kind: user_input_request.kind.clone(),
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
        let request_message_id = assistant_message_id(&session, &pending.request)?;
        let conversation_identity = self
            .conversation_identity_for_message(request.session_id, request_message_id)
            .await?;

        if let Some(dispatch) = self
            .resume_active_reply(&session, conversation_identity, mode)
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

pub(super) fn managed_project_state_permission(
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
    use crate::session::store::run_marker_content;
    use crate::session::Session;
    use agena_domain::{ModelRef, ModelSpeedModeRequestOverride};
    use agena_runtime::SessionRunOptions;
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

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

    /// Build a session whose only part is a run marker at `turn_id` for `model`
    /// (v2: the marker's part id is both the durable message id and the model
    /// turn id; `matching_model_turn_id` keys off provider_id/model_id).
    fn session_with_assistant_turn(turn_id: i64, model: &ModelRef) -> Session {
        let now = chrono::Utc::now();
        let mut session = Session::new(1, 1, "test", now);
        session.install_projected_parts(vec![Part {
            part_id: turn_id,
            kind: "run".to_owned(),
            role: PartRole::Assistant,
            state: PartState::InProgress,
            content: run_marker_content(
                "user_send",
                Some(model.provider_id.as_ref()),
                Some(model.model_id.as_ref()),
                None,
                None,
            ),
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            run_id: None,
            origin_session_id: 1,
            revision: 0,
            started_at_ms: now.timestamp_millis(),
            finished_at_ms: None,
            created_at_ms: now.timestamp_millis(),
            updated_at_ms: now.timestamp_millis(),
            provider_state: None,
        }]);
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
    AppError, Arc, DecisionTraceStep, ExecutionControl, ExecutionSource, ExecutionStatus,
    ModelRef, ModelSpeedModeRequestOverride, OperationPart, PartContent, PathBuf, PermissionAction,
    PermissionMode, PermissionReplyKind, PermissionScope, PersistedPermissionRule,
    PromptRequestOptions, PromptTurnBudget, ProviderPromptAnchor, ResolvedPendingTool,
    SessionExecutionReplyRequest, SessionManager, SessionManagerState,
    SessionPermissionReplyRequest, SessionRunOptions, SessionRunRequest, SessionRunTermination,
    StreamingToolExecution, TimeRange, ToolError, ToolInvocation, ToolInvocationExecution,
    UserInputReplyKind, Utc, ask_user_title, completed_lifecycle, custom_payload_value,
    execution_control_to_app_error, host_user_input_response, mpsc,
    operation_blocks_from_tool_output,
    payload_tool_name_for_invocation, permission_action_key, persisted_rules_for_reply,
    resolve_pending_tool, run_abort_reason, text_result_blocks, tool_name, user_input_execution,
};
