//! Application-facing background-activity service port.
//!
//! [`RuntimeActivityService`] is the single control surface TUI/Web use to
//! list, inspect, follow, stop, and dismiss background activities of every
//! kind. The concrete implementation lives in the runtime composition and
//! delegates log reads and stop/dismiss to per-kind adapters.

use async_trait::async_trait;
use thiserror::Error;

use agena_domain::{
    BackgroundActivity, BackgroundActivityFilter, BackgroundActivityLogRead,
    BackgroundActivityStatus,
};

#[derive(Debug, Clone, Error, PartialEq, Eq)]
#[error("background activity control failed: {message}")]
/// Error controlling a background activity.
pub struct ActivityControlError {
    message: String,
}

impl ActivityControlError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn not_found(id: impl Into<String>) -> Self {
        Self::new(format!("background activity `{}` not found", id.into()))
    }

    pub fn not_running(id: impl Into<String>) -> Self {
        Self::new(format!(
            "background activity `{}` is not running",
            id.into()
        ))
    }

    pub fn not_stoppable(id: impl Into<String>) -> Self {
        Self::new(format!(
            "background activity `{}` cannot be stopped",
            id.into()
        ))
    }

    pub fn no_log_source(id: impl Into<String>) -> Self {
        Self::new(format!(
            "background activity `{}` has no log source",
            id.into()
        ))
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(message)
    }

    pub fn internal_error(error: &(dyn std::error::Error + 'static)) -> Self {
        Self::internal(agena_failure::diagnostic::format_error_chain(error))
    }
}

/// Control surface for the unified background-member projection. Durable
/// operation/scheduler state is authoritative; the runtime registry only
/// contributes transient detail and terminal history.
#[async_trait]
pub trait RuntimeActivityService: Send + Sync {
    async fn list_activities(
        &self,
        filter: &BackgroundActivityFilter,
    ) -> Result<Vec<BackgroundActivity>, ActivityControlError>;

    async fn get_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Read incremental activity logs with the unified `since_seq` cursor.
    /// `wait_ms` blocks for fresh output when no new lines are available yet
    /// (0 disables waiting).
    async fn activity_logs(
        &self,
        activity_id: &str,
        since_seq: u64,
        limit: Option<u32>,
        wait_ms: u64,
    ) -> Result<BackgroundActivityLogRead, ActivityControlError>;

    /// Stop a running activity. Shell processes are killed, runtime tasks are
    /// cancelled, delegated tasks are cancelled through their child session.
    async fn stop_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Pause a durable scheduled activity without deleting it.
    async fn pause_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Resume a paused durable scheduled activity.
    async fn resume_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Permanently remove a durable scheduled activity.
    async fn delete_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Remove a finished activity from the list without touching its
    /// underlying work.
    fn dismiss_activity(
        &self,
        activity_id: &str,
    ) -> Result<BackgroundActivity, ActivityControlError>;

    /// Remove every finished activity, including terminal scheduler jobs;
    /// returns the number removed.
    async fn clear_finished(&self) -> Result<usize, ActivityControlError>;
}

/// Convenience helper for status transitions.
#[allow(dead_code)]
pub(crate) fn terminal_status_for_shell(
    status: agena_domain::ProcessStatus,
) -> BackgroundActivityStatus {
    match status {
        agena_domain::ProcessStatus::Running => BackgroundActivityStatus::Running,
        agena_domain::ProcessStatus::Exited => BackgroundActivityStatus::Succeeded,
        agena_domain::ProcessStatus::TimedOut => BackgroundActivityStatus::Failed,
        agena_domain::ProcessStatus::Stopped => BackgroundActivityStatus::Stopped,
        agena_domain::ProcessStatus::Failed => BackgroundActivityStatus::Failed,
    }
}
