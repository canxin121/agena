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
    pub agents: RuntimeAgentsStatus,
    pub plugin_ui_catalog: agena_plugin_host::PluginUiCatalog,
    pub tool_registry_generation: u64,
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeMcpStatus {
    pub servers: Vec<RuntimeMcpServerStatus>,
}

#[derive(Debug, Clone)]
pub struct RuntimeMcpServerStatus {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeLspStatus {
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerStatus>,
}

#[derive(Debug, Clone)]
pub struct RuntimeLspServerStatus {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeSkillsStatus {
    pub skills: Vec<RuntimeSkillStatus>,
    pub commands: Vec<RuntimeSkillStatus>,
}

#[derive(Debug, Clone)]
pub struct RuntimeSkillStatus {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RuntimeAgentsStatus {
    pub default_agent: String,
    pub agents: Vec<RuntimeAgentStatus>,
}

#[derive(Debug, Clone)]
pub struct RuntimeAgentStatus {
    pub name: String,
    pub description: String,
    pub permission: agena_domain::PermissionConfig,
    pub defaults: RuntimeAgentSelectionStatus,
    pub allowed_tools: Vec<String>,
    pub scope: agena_domain::AgentScope,
    pub source_path: Option<String>,
}

/// Complete agent-profile projection for presentation/editing flows that need
/// the profile prompt, while keeping the registry implementation in Runtime.
#[derive(Debug, Clone)]
pub struct RuntimeAgentProfile {
    pub name: String,
    pub description: String,
    pub permission: agena_domain::PermissionConfig,
    pub defaults: RuntimeAgentSelectionStatus,
    pub allowed_tools: Vec<String>,
    pub prompt: String,
    pub scope: agena_domain::AgentScope,
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct RuntimeAgentSelectionStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl RuntimeAgentSelectionStatus {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none()
            && self.adapter.is_none()
            && self.model.is_none()
            && self.thinking_mode.is_none()
            && self.speed_mode.is_none()
            && self.verbosity.is_none()
            && self.parallel_tool_calls.is_none()
    }
}

/// Read-only operational projection from a composed runtime.
#[async_trait]
pub trait RuntimeStatusService: Send + Sync {
    /// Lightweight agent directory for synchronous presentation paths. This is
    /// separate from the complete asynchronous status snapshot so TUI choice
    /// lists need not block on MCP/LSP diagnostics.
    fn agents_status(&self) -> RuntimeAgentsStatus;

    fn agent_profile(&self, name: &str) -> Option<RuntimeAgentProfile>;

    async fn runtime_status(&self) -> RuntimeStatusSnapshot;
}

#[cfg(test)]
mod tests {
    use super::RuntimeAgentSelectionStatus;

    #[test]
    fn agent_selection_status_uses_compact_config_serialization() {
        let empty = RuntimeAgentSelectionStatus::default();
        assert!(empty.is_empty());
        assert_eq!(serde_json::to_value(&empty).unwrap(), serde_json::json!({}));

        let selected = RuntimeAgentSelectionStatus {
            provider: Some("openai".to_string()),
            model: Some("gpt-5".to_string()),
            ..Default::default()
        };
        assert!(!selected.is_empty());
        assert_eq!(
            serde_json::to_value(selected).unwrap(),
            serde_json::json!({"provider": "openai", "model": "gpt-5"})
        );
    }
}
