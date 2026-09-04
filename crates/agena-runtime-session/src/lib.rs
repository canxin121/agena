//! # agena-runtime-session
//!
//! Session model, persistence, history, and execution services.
//!
//! This crate implements the session layer of the runtime: it owns the
//! session model ([`session::model`]), persists session state and events,
//! builds completion requests, governs context budgets, and executes model
//! runs with tool invocation, permission handling, and task control.
//!
//! ## Key items
//!
//! - [`AppError`] — the top-level application error type.
//! - [`ContextGovernor`] — context window budgeting and compaction policy.
//! - [`ExecutionRegistry`] / [`ExecutionControl`] — manage running
//!   executions (pause/cancel/resume).
//! - [`SessionStore`] (from `agena-storage`) — the sealed data facade; there
//!   is no event log in v2 (design 14.3).
//!
//! The crate also re-exports [`config`], [`provider`], [`plugins`], and the
//! shared contracts (`authorization`, `identity`, `part`, `permission`,
//! `provider_state`).

extern crate self as agena_runtime;

pub use agena_runtime_config as config;
pub use agena_runtime_contracts::{authorization, identity, part, permission, provider_state};
pub use agena_runtime_plugins as plugins;
pub use agena_runtime_provider as provider;
pub mod tool {
    pub use agena_runtime_tools::tool::*;
}

mod compaction_policy;
mod context_budget;
mod context_governor;
mod error;
mod execution_registry;
mod guards;
mod installation_id;
mod metrics;
mod periodic;
mod prompt_budget;
mod prompt_merge;
mod service_failure;
pub mod session;
mod session_cache_policy;
mod session_configuration;
mod session_execution_control;
mod session_execution_service;
mod session_maintenance;
mod session_plugin_command;
mod session_query_service;
mod session_requests;
mod session_tool_execution;
mod task_control;
mod usage_stats;
pub use session::model;
pub use session_cache_policy::SessionCachePolicy;

pub use agena_runtime_tools::{
    ActiveSnapshot, ManagedSnapshot, generated_image_artifact_path, list_active_snapshots,
    list_managed_snapshots, project_state_dir,
};
pub use compaction_policy::*;
pub use context_budget::*;
pub use context_governor::ContextGovernor;
pub use error::AppError;
pub use execution_registry::{ExecutionControl, ExecutionControlError, ExecutionRegistry};
pub use guards::{AbortOnDrop, spawn_abortable, spawn_detached};
pub use installation_id::resolve_installation_id;
pub use metrics::{
    RuntimeMetricsSnapshot, record_provider_call, record_provider_stream, record_tool_execution,
    runtime_metrics_snapshot, session_finished, session_started,
};
pub use periodic::{PeriodicControl, run_periodic};
pub use prompt_budget::*;
pub use prompt_merge::merge_system_prompts;
pub use session::SessionManager;
pub use session::{Session, SessionProcessor};
pub use session::{
    SessionSubtaskOutput, SessionSubtaskOutputChunk, SessionSubtaskRequest, SessionSubtaskResponse,
};
pub use session_configuration::RuntimeSessionManagerConfig;
pub use session_execution_control::*;
pub use session_execution_service::*;
pub use session_maintenance::*;
pub use session_plugin_command::*;
pub use session_query_service::*;
pub use session_requests::*;
pub use session_tool_execution::*;
pub use task_control::TaskControl;
pub use usage_stats::{UsageStatRecord, summarize_usage_records};
