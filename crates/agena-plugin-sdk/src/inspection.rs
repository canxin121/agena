//! Read-only plugin inspection and surface contracts. These values carry no
//! runtime handles, permission grants, or executable callbacks.
use crate::host_api::{HostHookRegistration, HostThemePalette};
use crate::{
    CommandDefinition, PluginDisplayContribution, PluginKey, PluginManifest, PluginNotifyAction,
    PluginServiceExport, PluginServiceImport,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
/// Run state of a plugin.
pub enum PluginRunState {
    Running,
    Restarting,
    Failed,
    Stopped,
}

impl AsRef<str> for PluginRunState {
    fn as_ref(&self) -> &str {
        match self {
            Self::Running => "running",
            Self::Restarting => "restarting",
            Self::Failed => "failed",
            Self::Stopped => "stopped",
        }
    }
}

impl fmt::Display for PluginRunState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_ref())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Status of a plugin.
pub struct PluginStatus {
    pub plugin_id: PluginKey,
    /// Owned so the status round-trips over the public API's JSON responses
    /// (the server serializes its in-process statuses for remote clients).
    pub kind: String,
    pub state: PluginRunState,
    pub pid: Option<u32>,
    pub restart_count: u32,
    pub last_exit_code: Option<i32>,
    pub last_restart_at_ms: Option<i64>,
    pub last_failure: Option<agena_failure::UserProblem>,
}

impl PluginStatus {
    pub fn initial(plugin_id: &PluginKey, kind: impl Into<String>) -> Self {
        Self {
            plugin_id: plugin_id.clone(),
            kind: kind.into(),
            state: PluginRunState::Running,
            pid: None,
            restart_count: 0,
            last_exit_code: None,
            last_restart_at_ms: None,
            last_failure: None,
        }
    }

    pub fn key(&self) -> PluginKey {
        self.plugin_id.clone()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
/// A log record produced by a plugin.
pub struct PluginLogRecord {
    pub seq: u64,
    pub timestamp_ms: i64,
    pub plugin_id: PluginKey,
    pub level: String,
    pub source: String,
    pub message: String,
    pub fields: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEffectScopeState {
    Active,
    Disposing,
    Disposed,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginEffectState {
    Active,
    Disposing,
    Disposed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectDescriptor {
    pub id: u64,
    pub kind: String,
    pub label: String,
    pub registered_at_ms: u64,
    pub state: PluginEffectState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginEffectScopeInspect {
    pub plugin_id: PluginKey,
    pub generation: u64,
    pub lifecycle: PluginEffectScopeState,
    pub accepting: bool,
    pub active_leases: usize,
    pub cancelled: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub effects: Vec<PluginEffectDescriptor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of a plugin authority and capabilities.
pub struct PluginAuthoritySummary {
    pub trust_level: String,
    pub provenance: Vec<String>,
    pub plugin_capabilities: Vec<String>,
    pub tool_capabilities: BTreeMap<String, Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Inspection view of a plugin.
pub struct PluginInspect {
    pub status: PluginStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manifest: Option<PluginManifest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub authority: Option<PluginAuthoritySummary>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hooks: Vec<HostHookRegistration>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub configured_plugin: Option<crate::configuration::ConfiguredPlugin>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub activation: Option<PluginActivationInspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub services: Option<PluginServiceInspect>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effects: Option<PluginEffectScopeInspect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginServiceInspect {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exports: Vec<PluginServiceExport>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imports: Vec<PluginServiceImportInspect>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginServiceImportInspect {
    #[serde(flatten)]
    pub declaration: PluginServiceImport,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_provider: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub methods: Vec<crate::PluginServiceMethod>,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Safe, structured activation state shown even when a plugin never reached
/// `meta/init`. Raw transport/init diagnostics remain in protected host logs.
pub struct PluginActivationInspect {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<PluginKey>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub after: Vec<PluginKey>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocked: Option<PluginActivationDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Non-sensitive reason an activation was blocked.
pub struct PluginActivationDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dependencies: Vec<PluginKey>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Neutral plugin surface catalog. Executable commands and terminal-only
/// presentation are deliberately separate from one another and from any
/// particular renderer.
pub struct PluginSurfaceCatalog {
    #[serde(default)]
    pub commands: Vec<CommandCatalogItem>,
    #[serde(default)]
    pub terminal: PluginTerminalSurfaceCatalog,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
/// Terminal-only presentation catalog.
pub struct PluginTerminalSurfaceCatalog {
    #[serde(default)]
    pub display: Vec<HostDisplayContribution>,
    #[serde(default)]
    pub themes: Vec<HostThemePalette>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Catalog item of a plugin command.
pub struct CommandCatalogItem {
    pub plugin_id: PluginKey,
    /// Host-derived fact used by every command palette. This is computed from
    /// the validated SettingsContract and cannot drift between clients.
    pub accepts_empty_input: bool,
    /// Deterministic editor/invocation seed produced by the shared contract.
    pub default_input: serde_json::Value,
    #[serde(flatten)]
    pub command: CommandDefinition,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Status of an explicit user-driven plugin tool invocation.
pub enum PluginToolInvokeStatus {
    Completed,
    CapabilityUnavailable,
    ToolUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Presentation-neutral result of an explicit plugin tool invocation.
pub struct PluginToolInvokeResponse {
    pub plugin_id: PluginKey,
    pub tool: String,
    pub status: PluginToolInvokeStatus,
    pub title: String,
    pub output_text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

/// Host-resolved declarative display contribution: the contributing plugin
/// plus its pure content contribution. The host owns placement and priority
/// ordering; plugins never target a location or color (Phase 6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HostDisplayContribution {
    pub plugin_id: PluginKey,
    pub contribution: PluginDisplayContribution,
}

/// A plugin-emitted notification intent through the unified `host.notify`
/// entry (Phase 6). The host keeps a bounded recent queue for frontends;
/// surface/priority/dedupe decisions stay on the host side.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct HostNotification {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attention: Option<crate::AttentionNotification>,
    pub plugin_id: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body: String,
    /// One of `info` | `success` | `warning` | `error`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub severity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<PluginNotifyAction>,
}
