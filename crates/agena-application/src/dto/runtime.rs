#[derive(Debug, Clone, Serialize)]
/// Runtime operator surface: MCP, LSP, agent, skills, and plugin UI.
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agent_id: String,
    pub skills: RuntimeSkillsResource,
    pub ui: RuntimePluginUiResource,
}

#[derive(Debug, Clone, Serialize)]
/// Plugin UI catalog with the tool registry generation it reflects.
pub struct RuntimePluginUiResource {
    pub catalog: agena_plugin_host::PluginUiCatalog,
    pub tool_registry_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Serialize)]
/// MCP servers running inside the runtime.
pub struct RuntimeMcpResource {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<RuntimeMcpServerResource>,
}

#[derive(Debug, Clone, Serialize)]
/// One MCP server with its tool count.
pub struct RuntimeMcpServerResource {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize)]
/// LSP servers and diagnostics overview.
pub struct RuntimeLspResource {
    pub server_count: usize,
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerResource>,
}

#[derive(Debug, Clone, Serialize)]
/// One LSP server definition.
pub struct RuntimeLspServerResource {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
/// Loaded skills and skill commands.
pub struct RuntimeSkillsResource {
    pub skill_count: usize,
    pub command_count: usize,
    pub skills: Vec<RuntimeSkillResource>,
    pub commands: Vec<RuntimeSkillResource>,
}

#[derive(Debug, Clone, Serialize)]
/// One loaded skill or skill command.
pub struct RuntimeSkillResource {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// Application-facing projection of Runtime process counters.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct RuntimeMetricsResource {
    pub provider_calls_total: u64,
    pub provider_calls_error: u64,
    pub provider_stream_total: u64,
    pub tool_executions_total: u64,
    pub tool_executions_error: u64,
    pub session_active: u64,
}

/// Compact Application projection for terminal/runtime diagnostics.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSnapshotSummaryResource {
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub provider_count: usize,
    pub plugin_count: usize,
}

/// Complete Runtime diagnostic projection required by a process-local Studio
/// health surface. Runtime retains the live status record; upper layers never
/// receive that Runtime value or its service port.
#[derive(Debug, Clone, Serialize)]
pub struct RuntimeDiagnosticsResource {
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub workspace_root: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub config_found: bool,
    pub provider_ids: Vec<String>,
    pub session_runtime_available: bool,
}

/// Complete JSON configuration-source projection for presentation and
/// application-level configuration use cases. Runtime retains configuration
/// resolution and file settings implementation; consumers never assemble this
/// view from Runtime snapshots themselves.
#[derive(Debug, Clone)]
pub struct ConfigJsonSources {
    pub config_path: std::path::PathBuf,
    pub config_found: bool,
    pub project_config_path: std::path::PathBuf,
    pub project_config_found: bool,
    pub applied_layers: Vec<String>,
    pub file: serde_json::Value,
    pub project_file: serde_json::Value,
    pub effective: serde_json::Value,
}

impl From<agena_runtime::RuntimeMetricsSnapshot> for RuntimeMetricsResource {
    fn from(value: agena_runtime::RuntimeMetricsSnapshot) -> Self {
        Self {
            provider_calls_total: value.provider_calls_total,
            provider_calls_error: value.provider_calls_error,
            provider_stream_total: value.provider_stream_total,
            tool_executions_total: value.tool_executions_total,
            tool_executions_error: value.tool_executions_error,
            session_active: value.session_active,
        }
    }
}

/// Application-facing terminal preferences projection.
///
/// Runtime continues to resolve persisted configuration. This value prevents
/// terminal startup and palette reload from carrying the Runtime configuration
/// record or its presentation enums across the Application boundary.
#[derive(Debug, Clone, Default)]
pub struct TuiPreferencesResource {
    pub locale: Option<String>,
    pub theme: Option<String>,
    pub color_scheme: TuiColorSchemeResource,
    pub graphics: TuiGraphicsModeResource,
    /// Default transcript expansion for activities without a kind override.
    pub transcript_activity_default_expanded: bool,
    /// Per-kind transcript expansion overrides keyed by activity kind id.
    pub transcript_activity_kinds: std::collections::BTreeMap<String, bool>,
}

#[derive(Debug, Clone, Copy, Default)]
/// Color scheme preference for the TUI.
pub enum TuiColorSchemeResource {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default)]
/// Graphics mode preference for the TUI.
pub enum TuiGraphicsModeResource {
    #[default]
    Auto,
    Native,
    Unicode,
}

impl From<agena_runtime::RuntimeUiConfiguration> for TuiPreferencesResource {
    fn from(value: agena_runtime::RuntimeUiConfiguration) -> Self {
        Self {
            locale: value.locale,
            theme: value.theme,
            color_scheme: match value.color_scheme {
                agena_runtime::RuntimeTuiColorScheme::Auto => TuiColorSchemeResource::Auto,
                agena_runtime::RuntimeTuiColorScheme::Dark => TuiColorSchemeResource::Dark,
                agena_runtime::RuntimeTuiColorScheme::Light => TuiColorSchemeResource::Light,
            },
                        graphics: match value.graphics {
                agena_runtime::RuntimeTuiGraphicsMode::Auto => TuiGraphicsModeResource::Auto,
                agena_runtime::RuntimeTuiGraphicsMode::Native => TuiGraphicsModeResource::Native,
                agena_runtime::RuntimeTuiGraphicsMode::Unicode => TuiGraphicsModeResource::Unicode,
            },
            transcript_activity_default_expanded: value.transcript_activity_default_expanded,
            transcript_activity_kinds: value.transcript_activity_kinds,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
/// Response to a runtime configuration reload.
pub struct RuntimeReloadResponse {
    pub cause: &'static str,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
/// A runtime background task.
pub struct RuntimeBackgroundTaskResource {
    pub id: String,
    pub kind: RuntimeBackgroundTaskKind,
    pub origin: RuntimeBackgroundTaskOrigin,
    pub title: String,
    pub status: RuntimeBackgroundTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
    pub created_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub cancellable: bool,
}

impl From<RuntimeBackgroundTask> for RuntimeBackgroundTaskResource {
    fn from(value: RuntimeBackgroundTask) -> Self {
        Self {
            id: value.id,
            kind: value.kind,
            origin: value.origin,
            title: value.title,
            status: value.status,
            message: value.message,
            failure: value.failure.map(Into::into),
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            cancellable: value.cancellable,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
/// Response to starting a runtime background task.
pub struct RuntimeBackgroundTaskStartResponse {
    pub started: bool,
    pub task: RuntimeBackgroundTaskResource,
}

#[derive(Debug, Clone, Serialize)]
/// Response to cancelling a runtime background task.
pub struct RuntimeBackgroundTaskCancelResponse {
    pub task: RuntimeBackgroundTaskResource,
}
use super::{
    DateTime, RuntimeBackgroundTask, RuntimeBackgroundTaskKind, RuntimeBackgroundTaskOrigin,
    RuntimeBackgroundTaskStatus, Serialize, Utc,
};
