//! Unified background-activity management.
//!
//! Long-running work created by Agena tools — background shell processes,
//! delegated subagent tasks, scheduled jobs, and runtime maintenance tasks —
//! is surfaced through one [`RuntimeActivityService`] projection. Durable
//! background-operation and scheduler rows own lifecycle truth; the bounded
//! [`registry::ActivityRegistry`] contributes live log/detail state and
//! terminal history. Every mutation publishes a `background_activity_changed`
//! signal so TUI and web can follow work live.

pub mod registry;
pub mod service;
pub(crate) mod state;

pub(crate) use registry::ActivityRegistry;
pub use service::{ActivityControlError, RuntimeActivityService};
pub(crate) use state::{
    ActivityRuntimeState, BackgroundCompletionBridge, MonitorActivityBridge,
    RuntimeTaskActivityBridge, read_shell_logs, read_task_logs, upsert_task_activity_from_meta,
};
