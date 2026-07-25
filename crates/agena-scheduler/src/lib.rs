//! `agena-scheduler` — persistent cron + one-shot job scheduler.
//!
//! The Agena runtime uses [`SqliteJobStore`] for restart-safe durability;
//! [`InMemoryJobStore`] remains available for tests and embedded callers.
//!
//! At runtime, [`Scheduler`] spawns a tokio task that wakes up every
//! `tick_interval` and fires any job whose `next_fire_at` has passed.
//! When a job fires, the registered [`JobSink`] receives the payload —
//! callers wire that to their session manager so the prompt is enqueued
//! into the target session.
//!
//! Cron parsing uses the `cron` crate (5- or 6-field expressions).
//! Recurring jobs auto-expire after `max_age_days` (default 7) — they
//! fire one final time, are deleted, and the runtime is bounded.

pub mod error;
pub mod job;
pub mod scheduler;
pub mod store;

pub use error::{SchedulerError, SchedulerResult};
pub use job::{
    ClaimDueDelivery, JobDeliveryAttempt, JobDeliveryResult, JobKind, JobOutcome, JobRunRecord,
    JobRunStatus, JobSink, MisfirePolicy, RetryPolicy, ScheduledJob, SchedulerHistoryEntry,
};
pub use scheduler::Scheduler;
pub use store::{InMemoryJobStore, JobStore, SqliteJobStore};
