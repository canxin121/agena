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

impl From<agena_runtime::RuntimeBackgroundTask>
    for agena_api::resource::RuntimeBackgroundTaskResource
{
    fn from(value: agena_runtime::RuntimeBackgroundTask) -> Self {
        Self {
            id: value.id,
            kind: runtime_background_task_kind_from_domain(value.kind),
            origin: runtime_background_task_origin_from_domain(value.origin),
            title: value.title,
            status: runtime_background_task_status_from_domain(value.status),
            message: value.message,
            failure: value.failure.map(Into::into),
            created_at: value.created_at,
            started_at: value.started_at,
            finished_at: value.finished_at,
            cancellable: value.cancellable,
        }
    }
}

const fn runtime_background_task_kind_from_domain(
    value: agena_runtime::RuntimeBackgroundTaskKind,
) -> agena_api::resource::RuntimeBackgroundTaskKind {
    match value {
        agena_runtime::RuntimeBackgroundTaskKind::ModelCatalogRefresh => {
            agena_api::resource::RuntimeBackgroundTaskKind::ModelCatalogRefresh
        }
        agena_runtime::RuntimeBackgroundTaskKind::RuntimeReload => {
            agena_api::resource::RuntimeBackgroundTaskKind::RuntimeReload
        }
        agena_runtime::RuntimeBackgroundTaskKind::MarketplaceRegistrySync => {
            agena_api::resource::RuntimeBackgroundTaskKind::MarketplaceRegistrySync
        }
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginInstall => {
            agena_api::resource::RuntimeBackgroundTaskKind::MarketplacePluginInstall
        }
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginUninstall => {
            agena_api::resource::RuntimeBackgroundTaskKind::MarketplacePluginUninstall
        }
        agena_runtime::RuntimeBackgroundTaskKind::MarketplacePluginUpgrade => {
            agena_api::resource::RuntimeBackgroundTaskKind::MarketplacePluginUpgrade
        }
    }
}

const fn runtime_background_task_origin_from_domain(
    value: agena_runtime::RuntimeBackgroundTaskOrigin,
) -> agena_api::resource::RuntimeBackgroundTaskOrigin {
    match value {
        agena_runtime::RuntimeBackgroundTaskOrigin::System => {
            agena_api::resource::RuntimeBackgroundTaskOrigin::System
        }
        agena_runtime::RuntimeBackgroundTaskOrigin::User => {
            agena_api::resource::RuntimeBackgroundTaskOrigin::User
        }
    }
}

const fn runtime_background_task_status_from_domain(
    value: agena_runtime::RuntimeBackgroundTaskStatus,
) -> agena_api::resource::RuntimeBackgroundTaskStatus {
    match value {
        agena_runtime::RuntimeBackgroundTaskStatus::Running => {
            agena_api::resource::RuntimeBackgroundTaskStatus::Running
        }
        agena_runtime::RuntimeBackgroundTaskStatus::Succeeded => {
            agena_api::resource::RuntimeBackgroundTaskStatus::Succeeded
        }
        agena_runtime::RuntimeBackgroundTaskStatus::Failed => {
            agena_api::resource::RuntimeBackgroundTaskStatus::Failed
        }
        agena_runtime::RuntimeBackgroundTaskStatus::Cancelled => {
            agena_api::resource::RuntimeBackgroundTaskStatus::Cancelled
        }
    }
}
use super::{DateTime, Serialize, Utc};
