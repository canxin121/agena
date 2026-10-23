//! `agena-scheduler` — persistent cron + one-shot job scheduler.
//!
//! [`SqliteJobStore`] receives the owning service instance's initialized
//! SQLite connection. Its declarations in [`schema`] are composed with the
//! session and application tables into one exact database schema.
//! [`InMemoryJobStore`] remains available for tests and embedded callers.
//!
//! At runtime, [`Scheduler`] waits for the next fire, retry, or claim-expiry
//! deadline. Committed changes interrupt that wait immediately. Reads and
//! delivery concurrency are bounded; one session's deliveries stay ordered.
//! When a job fires, the registered [`JobSink`] receives the payload —
//! callers wire that to their session manager so the prompt is enqueued
//! into the target session.
//!
//! Cron parsing uses the `cron` crate (6 fields, with an optional year field).
//! Recurring jobs auto-expire after `max_age_days` (default 7) — they
//! fire one final time and remain available as completed jobs for audit.
//! Delivery claims renew every 30 seconds and become recoverable after a
//! 90-second lease expires. Recovery is at least once; sinks must deduplicate
//! using the stable [`JobDeliveryAttempt::delivery_key`].

pub mod error;
pub mod job;
pub mod scheduler;
pub mod schema;
pub mod store;

pub use error::{SchedulerError, SchedulerResult};
pub use job::{
    ClaimDueDelivery, JobDeliveryAttempt, JobDeliveryResult, JobKind, JobOutcome, JobRunRecord,
    JobRunStatus, JobSink, MisfirePolicy, RetryPolicy, ScheduledJob, ScheduledJobLaunchProvenance,
    SchedulerHistoryEntry,
};
pub use scheduler::{Scheduler, SchedulerChange, SchedulerChangeObserver};
pub use store::{DueJobQuery, InMemoryJobStore, JobSnapshot, JobStore, SqliteJobStore};
