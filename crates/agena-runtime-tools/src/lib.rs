//! # agena-runtime-tools
//!
//! Built-in tool execution and tool-facing runtime ports.
//!
//! Implements concrete built-in tool execution ([`tool`]), process
//! monitoring, project path resolution, and the
//! shared [`ToolExecutionRequest`] plumbing used by executors.

pub use agena_runtime_contracts::ToolSessionContext;
pub use agena_runtime_contracts::{authorization, identity, part, permission, provider_state};

mod atomic_file;
pub mod cli_tools;
mod file_diff;
pub use file_diff::{FileDiffPreview, file_diff_preview};
pub mod media_input;
mod monitor;
mod project_instructions;
mod project_paths;
pub mod shell_sandbox;
mod terminal;
pub mod tool;

pub use terminal::{TerminalOwner, TerminalRead, TerminalRegistry, TerminalStartParams};

pub use atomic_file::{
    atomic_create_file, atomic_replace_file, atomic_replace_file_with_check, atomic_write_file,
    canonicalize_mutation_path, verify_file_contents, with_file_mutation_locks,
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
    prune_tool_output, tool_output_spill_dir, tool_output_spill_path,
};
#[derive(Debug, Clone, PartialEq, Eq)]
/// Request to execute a tool.
pub struct ToolExecutionRequest {
    pub tool_name: String,
    pub input_json: String,
}
