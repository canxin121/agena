//! Stable operational-status projection for an already-composed runtime.
//!
//! The projection deliberately contains presentation-neutral values only. It
//! lets application and transport layers render runtime status without
//! traversing a concrete core snapshot.

use std::path::PathBuf;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::RuntimeBackgroundTask;

#[derive(Debug, Clone)]
/// Snapshot of the runtime status.
pub struct RuntimeStatusSnapshot {
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub workspace_root: PathBuf,
    pub config_path: PathBuf,
    pub config_found: bool,
    pub provider_ids: Vec<String>,
    pub plugin_count: usize,
    pub session_runtime_available: bool,
    pub watch_paths: Vec<PathBuf>,
    pub reload_enabled: bool,
    pub reload_interval_secs: u64,
    pub session_gc_enabled: bool,
    pub session_gc_interval_secs: u64,
    pub session_cache: Option<agena_domain::SessionCacheStats>,
    pub model_catalog: agena_provider::ModelCatalogResponse,
    pub model_catalog_refreshing: bool,
    pub background_tasks: Vec<RuntimeBackgroundTask>,
    pub automation_available: bool,
    pub scheduled_jobs: Vec<agena_scheduler::ScheduledJob>,
    pub mcp: RuntimeMcpStatus,
    pub lsp: RuntimeLspStatus,
    pub skills: RuntimeSkillsStatus,
    pub agent_id: String,
    pub plugin_surface_catalog: agena_plugin_host::PluginSurfaceCatalog,
    pub tool_registry_generation: u64,
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
/// Status of MCP servers in the runtime.
pub struct RuntimeMcpStatus {
    pub servers: Vec<RuntimeMcpServerStatus>,
}

/// Safe operational projection of a configured MCP connection. It excludes
/// authorization headers, bearer values, and every other credential field.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeMcpServerStatus {
    pub name: String,
    pub connected: bool,
    pub tool_count: usize,
    pub network_target: Option<String>,
    pub last_failure: Option<agena_failure::UserProblem>,
    pub instructions_present: bool,
    pub tool_generation: u64,
    pub resource_generation: u64,
    pub prompt_generation: u64,
    pub last_refresh_failure: Option<agena_failure::UserProblem>,
    pub reconnect_supervisor_running: bool,
    /// Configuration-shape-only descriptor; never contains header values,
    /// token-store keys, or credentials.
    pub auth_mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_health: Option<RuntimeMcpOAuthHealth>,
    /// Redacted migration advisory for coexisting, separately scoped bearer
    /// and OAuth records.  It never changes which credential a connection
    /// uses and contains no token or keyring detail.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_migration: Option<RuntimeMcpCredentialMigration>,
}

/// Redacted, local-only health of an MCP OAuth credential record. Status
/// inspection never performs an authorization-server request or a refresh.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RuntimeMcpOAuthHealth {
    pub credential_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expiry_state: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_available: Option<bool>,
}

#[derive(Debug, Clone, serde::Serialize)]
/// MCP credential migration status.
pub struct RuntimeMcpCredentialMigration {
    pub state: String,
    pub recommendation: String,
}

#[derive(Debug, Clone, Default)]
/// Status of LSP servers in the runtime.
pub struct RuntimeLspStatus {
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerStatus>,
}

#[derive(Debug, Clone)]
/// Status of one LSP server.
pub struct RuntimeLspServerStatus {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
/// Status of skills in the runtime.
pub struct RuntimeSkillsStatus {
    pub skills: Vec<RuntimeSkillStatus>,
    pub commands: Vec<RuntimeSkillStatus>,
}

#[derive(Debug, Clone)]
/// Status of one skill.
pub struct RuntimeSkillStatus {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub source_path: Option<String>,
}

/// Read-only operational projection from a composed runtime.
#[async_trait]
pub trait RuntimeStatusService: Send + Sync {
    async fn runtime_status(&self) -> RuntimeStatusSnapshot;
}
