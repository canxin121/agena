//! Built-in tool execution and tool-facing runtime ports.

pub use agena_runtime_contracts::{agent, agents, message, permission};

mod monitor;
mod project_paths;
mod snapshot_capabilities;
mod snapshot_managed;
mod snapshot_operations;
mod snapshot_registry;
pub mod tool;

pub use monitor::{
    MonitorError, MonitorRead, MonitorService, MonitorStart, MonitorStopOutcome,
    default_monitor_registry,
};
pub use monitor::{ReadParams as MonitorReadParams, StartParams as MonitorStartParams};
pub use project_paths::{
    agena_home_dir, generated_image_artifact_path, project_state_dir, snapshot_managed_dir,
    snapshot_rift_database_path,
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

/// Session state needed by tool execution, kept as a port so tools do not
/// depend on the session implementation crate.
pub trait ToolSessionContext {
    fn effective_workspace_root(&self) -> Option<&std::path::Path>;
    fn effective_permission(&self) -> &agena_runtime_contracts::agent::PermissionConfig;
    fn permission_ceiling(&self) -> &agena_runtime_contracts::agent::PermissionConfig;
    fn allowed_tools(&self) -> &[String];
    fn selected_model(&self) -> Option<&str>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolExecutionRequest {
    pub tool_name: String,
    pub input_json: String,
}
