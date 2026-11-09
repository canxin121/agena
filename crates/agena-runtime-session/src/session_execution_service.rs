//! Runtime-facing session execution commands with stable request and outcome
//! values. Message-part submission remains a separate concrete adapter while
//! it carries core-owned content values.

use async_trait::async_trait;

use crate::{
    SessionExecutionReplyRequest, SessionExecutionRequest, SessionForkRequest,
    SessionPermissionReplyRequest, SessionRewindRequest, SessionRunOptions, SessionUserRunRequest,
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
    pub message_id: agena_domain::session_kernel::EntryId,
    pub attempt_generation: agena_domain::session_kernel::AttemptGeneration,
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
        message_id: agena_domain::session_kernel::EntryId,
        attempt_generation: agena_domain::session_kernel::AttemptGeneration,
        turn_id: agena_domain::TurnId,
        reply_id: agena_domain::AssistantReplyId,
    ) -> Self {
        Self {
            session_id,
            receipt: Some(SessionExecutionReceipt {
                execution_id,
                message_id,
                attempt_generation,
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
    async fn ensure_bot_session(&self) -> Result<i64, SessionExecutionCommandError>;
    async fn list_bots(
        &self,
        include_archived: bool,
    ) -> Result<agena_assistant::bot::BotList, SessionExecutionCommandError>;
    async fn create_bot(
        &self,
        create: agena_assistant::bot::BotCreate,
    ) -> Result<agena_assistant::bot::BotProfile, SessionExecutionCommandError>;
    async fn get_bot(
        &self,
        id: uuid::Uuid,
    ) -> Result<agena_assistant::bot::BotProfile, SessionExecutionCommandError>;
    async fn update_bot(
        &self,
        id: uuid::Uuid,
        update: agena_assistant::bot::BotUpdate,
    ) -> Result<agena_assistant::bot::BotProfile, SessionExecutionCommandError>;
    async fn archive_bot(
        &self,
        id: uuid::Uuid,
        archive: agena_assistant::bot::BotArchive,
    ) -> Result<agena_assistant::bot::BotProfile, SessionExecutionCommandError>;
    async fn ensure_named_bot_session(
        &self,
        id: uuid::Uuid,
    ) -> Result<agena_assistant::bot::BotSession, SessionExecutionCommandError>;
    async fn ensure_default_bot_profile(
        &self,
    ) -> Result<agena_assistant::bot::BotProfile, SessionExecutionCommandError>;

    async fn edit_queued_message(
        &self,
        request: crate::SessionQueuedMessageEditRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    async fn create_session(
        &self,
        request: crate::SessionCreateRequest,
    ) -> Result<SessionExecutionCommandOutcome, SessionExecutionCommandError>;

    /// Return after the user run's parts have been committed and published.
    /// Provider execution continues in the background; clients can immediately
    /// read and render the stored message without creating local message parts.
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

    /// Import a complete archive from a caller-owned file without materializing
    /// it in memory. The file must remain available until this call completes
    /// or is cancelled. Reusing the request ID with the same archive returns
    /// the original session. Imported work is never scheduled.
    async fn import_session_file(
        &self,
        request_id: agena_domain::CommandId,
        path: &std::path::Path,
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
