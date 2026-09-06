//! `agena-scheduler` — persistent cron + one-shot job scheduler.
//!
//! The scheduler owns a dedicated SQLite database (separate from the chat
//! database) whose schema lives in [`schema`]; [`SqliteJobStore`] provides
//! restart-safe durability against it. [`InMemoryJobStore`] remains available
//! for tests and embedded callers (and as a fallback when no scheduler
//! database is configured).
//!
//! At runtime, [`Scheduler`] spawns a tokio task that wakes up every
//! `tick_interval` and fires any job whose `next_fire_at` has passed.
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
pub use scheduler::Scheduler;
pub use store::{InMemoryJobStore, JobSnapshot, JobStore, SqliteJobStore};
