//! Runtime-facing session execution commands with stable request and outcome
//! values. Message-part submission remains a separate concrete adapter while
//! it carries core-owned content values.

use async_trait::async_trait;
use thiserror::Error;

use crate::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest,
    SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions,
    SessionUserMessageRequest,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionExecutionCommandOutcome {
    pub session_id: i64,
}

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("session execution command failed: {message}")]
pub struct SessionExecutionCommandError {
    message: String,
}

impl SessionExecutionCommandError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

#[async_trait]
pub trait SessionExecutionCommandService: Send + Sync {
    async fn create_session(
        &self,
        request: crate::SessionCreateRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn submit_user_message(
        &self,
        request: SessionUserMessageRequest,
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
            Ok(SessionExecutionCommandOutcome { session_id: 1 })
        }

        async fn submit_user_message(
            &self,
            request: crate::SessionUserMessageRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.run.session_id,
            })
        }

        async fn continue_session(
            &self,
            request: crate::SessionExecutionRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.session_id,
            })
        }

        async fn compact_session(
            &self,
            request: crate::SessionExecutionRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.session_id,
            })
        }

        async fn rewind_session(
            &self,
            request: crate::SessionRewindRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.session_id,
            })
        }

        async fn fork_session(
            &self,
            request: crate::SessionForkRequest,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.session_id,
            })
        }

        async fn import_session_jsonl(
            &self,
            _jsonl: &str,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome { session_id: 1 })
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
            Ok(SessionExecutionCommandOutcome {
                session_id: request.request.session_id,
            })
        }

        async fn reply_user_input(
            &self,
            request: crate::SessionExecutionReplyRequest<agena_domain::UserInputReply>,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome {
                session_id: request.session_id,
            })
        }

        async fn update_session_selection(
            &self,
            session_id: i64,
            _options: crate::SessionRunOptions,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome { session_id })
        }

        async fn set_session_permission(
            &self,
            session_id: i64,
            _permission: agena_domain::PermissionConfig,
        ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError> {
            Ok(SessionExecutionCommandOutcome { session_id })
        }
    }

    #[tokio::test]
    async fn trait_object_returns_only_the_stable_session_id_outcome() {
        let service: &dyn SessionExecutionCommandService = &FakeService;
        let outcome = service.import_session_jsonl("{}").await.expect("import");
        assert_eq!(outcome.session_id, 1);
    }
}
