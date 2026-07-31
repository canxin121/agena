use super::{
    AppError, Arc, ExecutionStatus, OperationPart, PartContent, PersistedPermissionRule,
    SessionManager, SessionManagerState, SessionPendingTool, completed_lifecycle,
    resolve_pending_tool, text_result_blocks, tool_name, update_resolved_tool_message,
};
use crate::session::Session;
use agena_domain::ToolOutput;

impl SessionManager {
    pub(in crate::session::manager) async fn apply_tool_failure(
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

    pub(in crate::session::manager) async fn apply_tool_failure_with_rules(
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
                PartContent::Activity(crate::message::RuntimeActivity::Operation(operation)) => {
                    Some(operation.title.clone())
                }
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
                tool_part.set_content(PartContent::operation(operation));
                tool_part.status = ExecutionStatus::Failed;
            })?;

        self.persist_tool_completion(
            session,
            assistant_message,
            &resolved,
            persisted_rules,
            Vec::new(),
            state,
        )
        .await
    }
}
