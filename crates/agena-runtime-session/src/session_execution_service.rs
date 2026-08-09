//! Runtime-facing session execution commands with stable request and outcome
//! values. Message-part submission remains a separate concrete adapter while
//! it carries core-owned content values.

use async_trait::async_trait;

use crate::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest,
    SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions,
    SessionUserRunRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Outcome of a session execution command.
pub struct SessionExecutionCommandOutcome {
    pub session_id: i64,
    pub receipt: Option<SessionExecutionReceipt>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
/// Receipt of an accepted session execution.
pub struct SessionExecutionReceipt {
    pub execution_id: agena_domain::ExecutionId,
    pub turn_id: agena_domain::TurnId,
    pub reply_id: agena_domain::AssistantReplyId,
}

impl SessionExecutionCommandOutcome {
    pub fn completed(session_id: i64) -> Self {
        Self {
            session_id,
            receipt: None,
        }
    }

    pub fn accepted(
        session_id: i64,
        execution_id: agena_domain::ExecutionId,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Self {
        Self {
            session_id,
            receipt: Some(SessionExecutionReceipt {
                execution_id,
                turn_id,
                reply_id,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
/// Error of a session execution command.
pub struct SessionExecutionCommandError {
    pub failure: agena_failure::Failure,
}

impl SessionExecutionCommandError {
    pub fn from_failure(failure: agena_failure::Failure) -> Self {
        Self { failure }
    }
}

impl std::fmt::Display for SessionExecutionCommandError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.failure.user.fallback.as_str())
    }
}

impl std::error::Error for SessionExecutionCommandError {}

#[async_trait]
/// Service that accepts session execution commands.
pub trait SessionExecutionCommandService: Send + Sync {
    async fn create_session(
        &self,
        request: crate::SessionCreateRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn submit_user_run(
        &self,
        request: SessionUserRunRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn steer_input(
        &self,
        session_id: i64,
        document: agena_domain::ComposerDocument,
    ) -> Result<(), SessionExecutionCommandError>;

    async fn continue_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn compact_session(
        &self,
        request: SessionExecutionRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn rewind_session(
        &self,
        request: SessionRewindRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn fork_session(
        &self,
        request: SessionForkRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn import_session_jsonl(
        &self,
        jsonl: &str,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn reply_permission(
        &self,
        request: SessionPermissionReplyRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn reply_user_input(
        &self,
        request: SessionExecutionReplyRequest<agena_domain::UserInputReply>,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    /// Durable, idempotent acknowledgement that an interactive user-input
    /// request has been shown to the user. Clients call this when they open
    /// the request (fire-and-forget); the session manager persists the
    /// presentation so a never-presented request still auto-popups after a
    /// restart or on another client, while a presented-but-unanswered request
    /// can be surfaced through a persistent attention hint.
    async fn mark_interactive_request_presented(
        &self,
        session_id: i64,
        request_id: String,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn update_session_selection(
        &self,
        session_id: i64,
        options: SessionRunOptions,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    /// Replace the persisted per-session permission selection using the
    /// domain-owned policy value.
    async fn set_session_permission(
        &self,
        session_id: i64,
        permission: agena_domain::PermissionConfig,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;
}

#[cfg(test)]
mod tests {
    use super::{
        SessionExecutionCommandError, SessionExecutionCommandOutcome,
        SessionExecutionCommandService,
    };

    struct FakeService;

    #[async_trait::async_trait]
    impl SessionExecutionCommandService for FakeService {
        async fn create_session(
            &self,
            _request: crate::SessionCreateRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(1))
        }

        async fn submit_user_run(
            &self,
            request: crate::SessionUserRunRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.run.session_id,
            ))
        }

        async fn continue_session(
            &self,
            request: crate::SessionExecutionRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.session_id,
            ))
        }

        async fn compact_session(
            &self,
            request: crate::SessionExecutionRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.session_id,
            ))
        }

        async fn rewind_session(
            &self,
            request: crate::SessionRewindRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.session_id,
            ))
        }

        async fn fork_session(
            &self,
            request: crate::SessionForkRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.session_id,
            ))
        }

        async fn import_session_jsonl(
            &self,
            _jsonl: &str,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(1))
        }

        async fn steer_input(
            &self,
            _session_id: i64,
            _document: agena_domain::ComposerDocument,
        ) -> Result<(), SessionExecutionCommandError> {
            Ok(())
        }

        async fn reply_permission(
            &self,
            request: crate::SessionPermissionReplyRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.request.session_id,
            ))
        }

        async fn reply_user_input(
            &self,
            request: crate::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(
                request.session_id,
            ))
        }

        async fn mark_interactive_request_presented(
            &self,
            session_id: i64,
            _request_id: String,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(session_id))
        }

        async fn update_session_selection(
            &self,
            session_id: i64,
            _options: crate::SessionRunOptions,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(session_id))
        }

        async fn set_session_permission(
            &self,
            session_id: i64,
            _permission: agena_domain::PermissionConfig,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome::completed(session_id))
        }
    }

    #[tokio::test]
    async fn trait_object_returns_only_the_stable_session_id_outcome() {
        let service: &dyn SessionExecutionCommandService = &FakeService;
        let outcome = service.import_session_jsonl("{}").await.expect("import");
        assert_eq!(outcome.session_id, 1);
    }
}
