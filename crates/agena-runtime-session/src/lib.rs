//! Session model, persistence, history, and execution services.

extern crate self as agena_runtime;

pub use agena_runtime_config as config;
pub use agena_runtime_contracts::{agent, agents, message, permission};
pub use agena_runtime_plugins as plugins;
pub use agena_runtime_provider as provider;
pub mod tool {
    pub use agena_runtime_tools::tool::*;
}

mod compaction_policy;
mod completion_request;
mod context_budget;
mod context_governor;
pub(crate) use agena_runtime_session_core::db;
mod error;
pub mod event;
mod event_bridge;
mod event_publish_service;
mod event_query_service;
mod execution_registry;
mod guards;
mod installation_id;
mod metrics;
mod periodic;
mod presentation_event;
mod prompt_budget;
mod prompt_merge;
pub mod session;
mod session_cache;
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

pub use agena_runtime_tools::{
    ActiveSnapshot, ManagedSnapshot, generated_image_artifact_path, list_active_snapshots,
    list_managed_snapshots, project_state_dir,
};
pub use compaction_policy::*;
pub use completion_request::{CompletionRequestInputs, build_completion_request};
pub use context_budget::*;
pub use context_governor::ContextGovernor;
pub use error::AppError;
pub use event_bridge::{
    RuntimeEventSubscription, RuntimeEventSubscriptionItem, spawn_event_forwarder,
};
pub use event_publish_service::*;
pub use event_query_service::*;
pub use execution_registry::{ExecutionControl, ExecutionControlError, ExecutionRegistry};
pub use guards::{AbortOnDrop, spawn_abortable, spawn_detached};
pub use installation_id::resolve_installation_id;
pub use metrics::{
    RuntimeMetricsSnapshot, record_provider_call, record_provider_stream, record_tool_execution,
    runtime_metrics_snapshot, session_finished, session_started,
};
pub use periodic::{PeriodicControl, run_periodic};
pub use presentation_event::*;
pub use prompt_budget::*;
pub use prompt_merge::merge_system_prompts;
pub use session::SessionManager;
pub use session::{Session, SessionProcessor};
pub use session::{SessionSubtaskRequest, SessionSubtaskResponse};

pub(crate) use session_cache::{CacheEntry, SessionCache};
pub use session_cache_policy::SessionCachePolicy;
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
