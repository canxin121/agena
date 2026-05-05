//! `agena-scheduler` — persistent cron + one-shot job scheduler.
//!
//! Jobs land in an [`InMemoryJobStore`] for the current process; users
//! who need durability can persist the same `ScheduledJob` shape to
//! whatever backend they prefer (the agena runtime currently uses
//! SeaORM via a thin adapter outside this crate).
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
pub use job::{JobDeliveryResult, JobKind, JobOutcome, JobRunRecord, JobRunStatus, JobSink, ScheduledJob};
pub use scheduler::Scheduler;
pub use store::{InMemoryJobStore, JobStore};
