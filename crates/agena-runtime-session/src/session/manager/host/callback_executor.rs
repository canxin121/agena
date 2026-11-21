//! Read-only callback policy snapshots retain their originating principal and
//! reload every effective remembered resource decision before checking I/O.

use super::*;
use crate::session::model::SessionExecutionContext;

impl SessionManager {
    pub(super) async fn sandbox_executor_for_operator(
        &self,
        authority: &OperatorInvocationAuthority,
    ) -> Result<ToolExecutor, ToolError> {
        let records = self
            .kernel_runtime
            .permissions
            .resolve_snapshot(None, Some(authority.workspace))
            .await
            .map_err(|error| ToolError::invalid_input(error.to_string()))?;
        let executor = authority
            .executor
            .clone()
            .with_sandbox_persisted_rules(records)?;
        if authority.cancel.is_cancelled() {
            return Err(ToolError::Cancelled);
        }
        Ok(executor)
    }

    pub async fn host_callback_executor(
        &self,
        session: Option<i64>,
        call: Option<i64>,
    ) -> Result<(ToolExecutor, Option<SessionExecutionContext>), AppError> {
        self.prepare_callback_executor(session, call, true).await
    }

    pub async fn host_resource_callback_executor(
        &self,
        session: Option<i64>,
        call: Option<i64>,
    ) -> Result<ToolExecutor, AppError> {
        self.prepare_callback_executor(session, call, false)
            .await
            .map(|(executor, _)| executor)
    }

    async fn prepare_callback_executor(
        &self,
        session: Option<i64>,
        call: Option<i64>,
        include_tool_catalog: bool,
    ) -> Result<(ToolExecutor, Option<SessionExecutionContext>), AppError> {
        let _admission = HOST_POLICY_READS
            .acquire()
            .await
            .expect("host policy admission is never closed");
        if self.shutdown.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        if session == Some(OPERATOR_SESSION) {
            let call = call.ok_or_else(|| {
                AppError::Config("the host callback has no active operator call".into())
            })?;
            let authority = self.operator_invocation(call).map_err(tool_error)?;
            let executor = self
                .sandbox_executor_for_operator(&authority)
                .await
                .map_err(tool_error)?;
            return Ok((executor, None));
        }

        if let Some(session) = session {
            let key = session_id(session)?;
            let live = match call {
                Some(call) if call >= 0 => Some(self.live_tool(session, call)?),
                _ => None,
            };
            let state = match live.as_ref() {
                Some((_, control)) => control.state.clone(),
                None => self.state_for_session(key).await?,
            };
            let context = match live.as_ref() {
                Some((_, control)) => {
                    self.execution_context_for_execution(key, &state, Some(control.execution))
                        .await?
                }
                None => self.execution_context_with_state(key, &state).await?,
            };
            let cancel = live
                .as_ref()
                .map(|(_, control)| control.cancel.clone())
                .unwrap_or_else(|| self.shutdown.child_token());
            let executor = if include_tool_catalog {
                state.executor.for_session_context_async(&context).await
            } else {
                state.executor.for_session_resource_context(&context)
            }
            .with_cancellation_token(Some(cancel.clone()));
            // Scoping deliberately clears inherited snapshots. Bind the
            // owning session/workspace records only after that boundary.
            let executor = self
                .sandbox_executor_for_session(&executor, session)
                .await
                .map_err(tool_error)?;
            if cancel.is_cancelled() {
                return Err(AppError::Cancelled);
            }
            return Ok((executor, Some(context)));
        }

        // Background policy queries have the current workspace principal.
        // Resolve its identity from the same frozen executor, so a concurrent
        // reload cannot combine one workspace's policy with another's rules.
        let executor = self.tool_executor();
        let root = tokio::fs::canonicalize(executor.workspace_root())
            .await
            .map_err(|error| AppError::internal_error(&error))?;
        let workspace = self
            .kernel_runtime
            .workspaces
            .ensure_id(&root.to_string_lossy())
            .await
            .map_err(|error| AppError::internal_error(&error))?;
        let records = self
            .kernel_runtime
            .permissions
            .resolve_snapshot(None, Some(workspace))
            .await
            .map_err(|error| AppError::Internal(error.to_string()))?;
        let executor = executor
            .with_sandbox_persisted_rules(records)
            .map_err(tool_error)?;
        if self.shutdown.is_cancelled() {
            return Err(AppError::Cancelled);
        }
        Ok((executor, None))
    }
}
