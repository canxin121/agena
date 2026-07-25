use thiserror::Error;

#[derive(Debug, Error)]
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
}

pub type SchedulerResult<T> = Result<T, SchedulerError>;
