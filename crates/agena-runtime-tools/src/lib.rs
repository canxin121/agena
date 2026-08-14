//! # agena-runtime-tools
//!
//! Built-in tool execution and tool-facing runtime ports.
//!
//! Implements concrete built-in tool execution ([`tool`]), tool output
//! truncation ([`tool_output`]), process monitoring, project path
//! resolution, snapshot backends/operations, and the shared
//! [`ToolExecutionRequest`] plumbing used by executors.

pub use agena_runtime_contracts::ToolSessionContext;
pub use agena_runtime_contracts::{authorization, identity, part, permission, provider_state};

mod atomic_file;
mod bounded_process;
mod monitor;
mod project_paths;
mod snapshot_capabilities;
mod snapshot_managed;
mod snapshot_operations;
mod snapshot_registry;
pub mod tool;

pub use atomic_file::{
    atomic_create_file, atomic_replace_file, atomic_write_file, canonicalize_mutation_path,
    with_file_mutation_locks,
};
pub use monitor::{
    MonitorError, MonitorListener, MonitorRead, MonitorRegistry, MonitorService, MonitorStart,
    MonitorStopOutcome, MonitorWsParams, default_monitor_registry,
};
pub use monitor::{ReadParams as MonitorReadParams, StartParams as MonitorStartParams};

/// Deterministic external identity reserved before a session-owned process is
/// spawned. The session manager and tool adapters both derive this value, so
/// completion callbacks can resolve the durable aggregate even when a process
/// exits before the launch tool returns.
pub fn managed_process_id(session_id: i64, call_id: i64) -> String {
    format!("proc_{session_id}_{call_id}")
}
pub use project_paths::{
    MAX_GENERATED_IMAGE_BYTES, ManagedGeneratedImageArtifact, ManagedGeneratedImageError,
    agena_home_dir, generated_image_artifact_path, generated_media_extension,
    parse_base64_image_data_url, persist_generated_image_artifact, project_state_dir,
    snapshot_managed_dir, snapshot_rift_database_path,
};
pub use snapshot_capabilities::snapshot_backend_capabilities;
pub use snapshot_managed::{
    ManagedSnapshot, list_managed_snapshots, prune_stale_managed_snapshots,
};
pub use snapshot_operations::{
    SnapshotCreation, SnapshotOperationError, attach_existing_snapshot, create_managed_snapshot,
    remove_managed_snapshot, snapshot_has_local_changes,
};
pub use snapshot_registry::snapshot_rift_binary;
pub use snapshot_registry::{
    ActiveSnapshot, SnapshotRegistry, SnapshotSession, list_active_snapshots, snapshot_registry,
};
pub use tool_output::truncate_tool_output_text;

pub mod tool_output;

#[derive(Debug, Clone, PartialEq, Eq)]
/// Request to execute a tool.
pub struct ToolExecutionRequest {
    pub tool_name: String,
    pub input_json: String,
}
