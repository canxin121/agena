use super::{ExecutionControlError, execution_control_to_app_error};
use crate::{
    AppError,
    message::{Message, MessageStatus, PartContent},
    role::Role,
    session::{Session, SessionForkRequest, SessionManager, SessionRewindRequest, SessionSummary},
};

impl SessionManager {
    pub async fn fork_session(&self, request: SessionForkRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        let title = request
            .title
            .unwrap_or_else(|| format!("Fork of {}", source.title));
        self.store
            .fork_session(source, request.at_message_id, title, state.cache_policy())
            .await
    }

    /// External entry: cancel the active execution for `session_id`.
    ///
    /// Cancellation is idempotent: a task can complete between the UI
    /// deciding to cancel and this call reaching the manager, so the absence
    /// of a control is a successful no-op rather than an error.
    pub async fn cancel_active_execution(&self, session_id: i64) -> Result<(), AppError> {
        let result = self.execution_registry.cancel(session_id).await;
        // A plugin-hosted tool can be suspended in a host permission or
        // user-input callback. A cancellation token is only observed between
        // run-loop iterations, so release those one-shot waiters as well;
        // otherwise Ctrl+C leaves the executor blocked forever.
        self.cancel_host_interactive_waiters(session_id).await;
        cancel_active_execution_result(result)
    }

    /// External entry: inject `parts` as a steer message into the active
    /// execution for `session_id`. Returns `Err` if no execution is active or the
    /// channel was closed.
    pub async fn steer_input(
        &self,
        session_id: i64,
        parts: Vec<PartContent>,
    ) -> Result<(), AppError> {
        self.execution_registry
            .steer(session_id, parts)
            .await
            .map_err(execution_control_to_app_error)
    }

    pub async fn rewind_session(&self, request: SessionRewindRequest) -> Result<Session, AppError> {
        let state = self.execution_state();
        let source = self
            .store
            .load_session(request.session_id, state.cache_policy())
            .await?;
        if let Some(expected) = request.expected_version
            && source.version != expected
        {
            return Err(AppError::Conflict {
                session_id: request.session_id,
                expected,
                current: source.version,
            });
        }
        if !is_completed_user_rewind_target(
            source
                .messages
                .iter()
                .find(|message| message.id == request.message_id)
                .ok_or_else(|| {
                    AppError::Internal(format!(
                        "message not found in session {}: {}",
                        source.id, request.message_id
                    ))
                })?,
        ) {
            return Err(AppError::Internal(format!(
                "rewind target must be a completed user message: {}",
                request.message_id
            )));
        }
        let title = format!("Rewind of {}", source.title);
        self.store
            .fork_session_before_message(source, request.message_id, title, state.cache_policy())
            .await
    }

    /// Reload `session_id` and bail with [`AppError::Conflict`] if the live
    /// `version` no longer equals `expected`. Used by command handlers that
    /// take an `If-Match`-style optimistic-lock parameter.
    pub async fn assert_session_version(
        &self,
        session_id: i64,
        expected: i64,
    ) -> Result<(), AppError> {
        let session = self
            .store
            .load_session(session_id, self.execution_state().cache_policy())
            .await?;
        if session.version != expected {
            return Err(AppError::Conflict {
                session_id,
                expected,
                current: session.version,
            });
        }
        Ok(())
    }

    /// Serialise `session_id` as a JSONL bundle. The first line is the
    /// session header (id, parent, depth, runtime); subsequent lines are
    /// persistent event payloads in `seq_global` order.
    pub async fn export_session_jsonl(&self, session_id: i64) -> Result<String, AppError> {
        self.store.export_session_jsonl(session_id).await
    }

    /// Replay a JSONL bundle produced by [`Self::export_session_jsonl`] into
    /// this manager's workspace as a fresh session.
    pub async fn import_session_jsonl(&self, bundle: &str) -> Result<Session, AppError> {
        let state = self.execution_state();
        self.store
            .import_session_jsonl(bundle, state.cache_policy())
            .await
    }

    /// Return every session that shares the same `root_id`, ordered by
    /// `(depth, id)`. Useful for tree visualisation and bulk export.
    pub async fn list_session_tree(&self, root_id: i64) -> Result<Vec<SessionSummary>, AppError> {
        self.store.list_session_tree(root_id).await
    }
}

fn cancel_active_execution_result(
    result: Result<(), ExecutionControlError>,
) -> Result<(), AppError> {
    match result {
        Ok(()) | Err(ExecutionControlError::NoActiveExecution(_)) => Ok(()),
        Err(error) => Err(execution_control_to_app_error(error)),
    }
}

fn is_completed_user_rewind_target(message: &Message) -> bool {
    message.role == Role::User && message.state == MessageStatus::Completed
}

#[cfg(test)]
mod tests {
    use super::{
        ExecutionControlError, Message, MessageStatus, Role, cancel_active_execution_result,
        is_completed_user_rewind_target,
    };

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
        let mut user = Message::prompt_text(Role::User, "undo this");
        user.state = MessageStatus::Completed;
        let mut assistant = Message::prompt_text(Role::Assistant, "response");
        assistant.state = MessageStatus::Completed;
        let mut pending_user = Message::prompt_text(Role::User, "pending");
        pending_user.state = MessageStatus::Pending;

        assert!(is_completed_user_rewind_target(&user));
        assert!(!is_completed_user_rewind_target(&assistant));
        assert!(!is_completed_user_rewind_target(&pending_user));
    }
}
