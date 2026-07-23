#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agents: RuntimeAgentsResource,
    pub skills: RuntimeSkillsResource,
    pub ui: RuntimePluginUiResource,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimePluginUiResource {
    pub catalog: agena_plugin_host::PluginUiCatalog,
    pub tool_registry_generation: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event:
        Option<agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMcpResource {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<RuntimeMcpServerResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeMcpServerResource {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLspResource {
    pub server_count: usize,
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeLspServerResource {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSkillsResource {
    pub skill_count: usize,
    pub command_count: usize,
    pub skills: Vec<RuntimeSkillResource>,
    pub commands: Vec<RuntimeSkillResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSkillResource {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAgentsResource {
    pub default_agent: String,
    pub total_count: usize,
    pub agents: Vec<RuntimeAgentResource>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct RuntimeAgentSelectionResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

impl RuntimeAgentSelectionResource {
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

impl From<agena_runtime::RuntimeAgentSelectionStatus> for RuntimeAgentSelectionResource {
    fn from(value: agena_runtime::RuntimeAgentSelectionStatus) -> Self {
        Self {
            provider: value.provider,
            adapter: value.adapter,
            model: value.model,
            thinking_mode: value.thinking_mode,
            speed_mode: value.speed_mode,
            verbosity: value.verbosity,
            parallel_tool_calls: value.parallel_tool_calls,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAgentResource {
    pub name: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "PermissionConfig::is_empty")]
    pub permission: PermissionConfig,
    #[serde(
        default,
        skip_serializing_if = "RuntimeAgentSelectionResource::is_empty"
    )]
    pub defaults: RuntimeAgentSelectionResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_tools: Vec<String>,
    pub scope: AgentScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

impl From<agena_runtime::RuntimeAgentStatus> for RuntimeAgentResource {
    fn from(value: agena_runtime::RuntimeAgentStatus) -> Self {
        Self {
            name: value.name,
            description: value.description,
            permission: value.permission,
            defaults: value.defaults.into(),
            allowed_tools: value.allowed_tools,
            scope: value.scope,
            source_path: value.source_path,
        }
    }
}

/// Application-facing profile projection for Agent Studio.
///
/// Runtime retains its concrete profile registry and produces this value only
/// through `Application`; presentation code must not carry Runtime profile
/// types across the boundary.
#[derive(Debug, Clone)]
pub struct RuntimeAgentProfileResource {
    pub name: String,
    pub description: String,
    pub permission: PermissionConfig,
    pub defaults: RuntimeAgentSelectionResource,
    pub allowed_tools: Vec<String>,
    pub prompt: String,
    pub scope: AgentScope,
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
}

#[derive(Debug, Clone, Copy, Default)]
pub enum TuiColorSchemeResource {
    #[default]
    Auto,
    Dark,
    Light,
}

#[derive(Debug, Clone, Copy, Default)]
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
        }
    }
}

impl From<agena_runtime::RuntimeAgentProfile> for RuntimeAgentProfileResource {
    fn from(value: agena_runtime::RuntimeAgentProfile) -> Self {
        Self {
            name: value.name,
            description: value.description,
            permission: value.permission,
            defaults: value.defaults.into(),
            allowed_tools: value.allowed_tools,
            prompt: value.prompt,
            scope: value.scope,
            source_path: value.source_path,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReloadResponse {
    pub cause: &'static str,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeBackgroundTaskResource {
    pub id: String,
    pub kind: RuntimeBackgroundTaskKind,
    pub origin: RuntimeBackgroundTaskOrigin,
    pub title: String,
    pub status: RuntimeBackgroundTaskStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
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
            error_message: value.error_message,
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            cancellable: value.cancellable,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeBackgroundTaskStartResponse {
    pub started: bool,
    pub task: RuntimeBackgroundTaskResource,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeBackgroundTaskCancelResponse {
    pub task: RuntimeBackgroundTaskResource,
}
use super::{
    AgentScope, DateTime, PermissionConfig, RuntimeBackgroundTask, RuntimeBackgroundTaskKind,
    RuntimeBackgroundTaskOrigin, RuntimeBackgroundTaskStatus, Serialize, Utc,
};
