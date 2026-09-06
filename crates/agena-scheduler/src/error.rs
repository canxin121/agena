//! Scheduler error types.

use thiserror::Error;

#[derive(Debug, Error)]
/// Error from the scheduler.
pub enum SchedulerError {
    #[error("invalid cron expression '{expr}': {source}")]
    InvalidCron {
        expr: String,
        #[source]
        source: cron::error::Error,
    },

    #[error("cron expression '{expr}' has no future fire time")]
    NoFutureFire { expr: String },

    #[error("job not found: {0}")]
    NotFound(uuid::Uuid),

    #[error("sink unavailable")]
    SinkGone,

    #[error("invalid scheduled job update: {0}")]
    InvalidUpdate(String),

    #[error("scheduled job {0} changed concurrently; reload before retrying")]
    Conflict(uuid::Uuid),

    #[error("scheduler persistence failed: {0}")]
    Persistence(#[from] sea_orm::DbErr),

    #[error("invalid scheduler data: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// Result alias for scheduler operations.
pub type SchedulerResult<T> = Result<T, SchedulerError>;
