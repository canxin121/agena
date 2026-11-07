//! Runtime-facing control operations for an already-composed session service.
//!
//! This deliberately exposes only ID-based lifecycle control. Message,
//! transcript, event, and persistence projections are concrete adapter work
//! and must not leak through this contract while they are still core-owned.

use async_trait::async_trait;

use agena_domain::{CancellationOutcome, ExecutionId, ExecutionLifecycle, ModelRef};

#[derive(Debug, Clone, PartialEq, Eq)]
/// Error of a session execution control operation.
pub struct SessionExecutionControlError {
    pub failure: agena_failure::Failure,
}

impl SessionExecutionControlError {
    pub fn internal(diagnostic: impl std::fmt::Display) -> Self {
        Self {
            failure: crate::service_failure::unexpected_service_failure(
                "session.control_failed",
                "The session operation could not be completed.",
                diagnostic,
            ),
        }
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }
}

impl std::fmt::Display for SessionExecutionControlError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::service_failure::display_service_failure(&self.failure, formatter)
    }
}

impl std::error::Error for SessionExecutionControlError {}

/// Ownership-aware observation of one accepted attempt. Durable phase and
/// process-local settlement are deliberately separate.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionExecutionAttempt {
    pub session_id: i64,
    pub execution_id: ExecutionId,
    pub message_id: agena_domain::session_kernel::EntryId,
    pub attempt_generation: agena_domain::session_kernel::AttemptGeneration,
    pub current_attempt_generation: agena_domain::session_kernel::AttemptGeneration,
    pub phase: agena_domain::session_kernel::ExecutionPhase,
    pub settlement: agena_domain::session_kernel::ExecutionAttemptSettlement,
    pub kernel_epoch: agena_domain::KernelEpoch,
    pub owner_epoch: Option<agena_domain::KernelEpoch>,
}

/// Narrow control port for a composed session execution service.
#[async_trait]
/// Control interface for session execution.
pub trait SessionExecutionControl: Send + Sync {
    /// Read an exact attempt, including its historical durable phase after a
    /// retry. Absence is distinct from a completed or recovered attempt.
    async fn execution_attempt(
        &self,
        _session_id: i64,
        _execution_id: ExecutionId,
        _generation: agena_domain::session_kernel::AttemptGeneration,
    ) -> Result<Option<SessionExecutionAttempt>, SessionExecutionControlError> {
        Err(SessionExecutionControlError::internal(
            "exact attempt observation is unavailable",
        ))
    }

    /// Persistently pauses claims or explicitly resumes the backend FIFO.
    async fn set_message_queue_paused(
        &self,
        session_id: i64,
        paused: bool,
    ) -> Result<(), SessionExecutionControlError>;
    /// Current provider backoff, cleared as soon as the stream progresses.
    async fn provider_retry(&self, _session_id: i64) -> Option<agena_domain::ProviderRetryStatus> {
        None
    }

    /// Returns the current registry-backed lifecycle, if an execution exists.
    async fn active_execution(&self, session_id: i64) -> Option<ExecutionLifecycle>;

    /// Requests cancellation for the active execution identified by the
    /// session and returns any restored user input. Implementations may cancel
    /// descendants as part of their concrete orchestration policy.
    async fn cancel_execution_with_outcome(
        &self,
        session_id: i64,
        execution_id: ExecutionId,
    ) -> Result<CancellationOutcome, SessionExecutionControlError>;

    /// Atomically cancel only the expected generation. A stale generation
    /// returns ExecutionMismatch without pausing the queue or cancelling the
    /// replacement's effects. This operation does not stop the entire session.
    async fn cancel_execution_attempt_with_outcome(
        &self,
        _session_id: i64,
        _execution_id: ExecutionId,
        _generation: agena_domain::session_kernel::AttemptGeneration,
    ) -> Result<CancellationOutcome, SessionExecutionControlError> {
        Err(SessionExecutionControlError::internal(
            "exact attempt cancellation is unavailable",
        ))
    }

    /// Cancels whichever execution currently owns the session and suppresses
    /// queued background notification wakes. This is the user-facing stop
    /// operation; unlike `cancel_execution`, it intentionally does not depend
    /// on a possibly stale execution id observed by a client.
    /// Session-scoped cancellation result with optional restoration data.
    async fn cancel_session_with_outcome(
        &self,
        session_id: i64,
    ) -> Result<CancellationOutcome, SessionExecutionControlError>;

    /// Lists scheduler-owned automation jobs visible to the composed session
    /// service. The job contract is independent of core transcript state.
    async fn list_scheduled_jobs(
        &self,
    ) -> Result<Vec<agena_scheduler::ScheduledJob>, SessionExecutionControlError>;

    /// Whether the composed service currently has scheduler support.
    fn scheduler_available(&self) -> bool;

    /// Resolves the persisted model selection without exposing the concrete
    /// session/message projection that stores it.
    async fn selected_model(
        &self,
        session_id: i64,
    ) -> Result<Option<ModelRef>, SessionExecutionControlError>;
}
