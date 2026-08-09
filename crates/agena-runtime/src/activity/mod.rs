//! Unified background-activity management.
//!
//! Long-running work created by Agena tools — background shell processes,
//! delegated subagent tasks, and runtime maintenance tasks — is projected
//! into one bounded registry ([`registry::ActivityRegistry`]) and surfaced
//! through the application-facing [`RuntimeActivityService`] port. Every
//! mutation publishes a `background_activity_changed` event on the runtime
//! bus so TUI and web can follow work live.

pub mod registry;
pub mod service;
pub(crate) mod state;

pub(crate) use registry::ActivityRegistry;
pub use service::{ActivityControlError, RuntimeActivityService};
pub(crate) use state::{
    ActivityRuntimeState, MonitorActivityBridge, RuntimeTaskActivityBridge, read_shell_logs,
    read_task_logs, upsert_task_activity_from_meta,
};
