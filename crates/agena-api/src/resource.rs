//! REST resource projections for the unified API surface, reorganised by
//! domain so external clients can `use agena_api::resource::*`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use std::collections::BTreeMap;

mod activity;
mod auth;
mod interaction;
mod notification;
pub use activity::*;
pub use auth::*;
pub use interaction::*;
pub use notification::*;

fn is_false(value: &bool) -> bool {
    !*value
}

// ─── Health / runtime ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Runtime health response.
pub struct HealthResponse {
    pub status: String,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub database_connected: bool,
    /// Identity of the long-lived server serving this API.
    pub server: ServerIdentityResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Stable identity for one server process lifetime.
pub struct ServerIdentityResource {
    pub id: Uuid,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub protocol_version: u32,
}

/// Atomically-published local discovery record for one server.
/// The record is only a hint: clients must call health and compare every
/// identity field before using it for lifecycle operations.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ServerEndpointRecord {
    pub schema: u32,
    pub url: String,
    pub server_id: Uuid,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub protocol_version: u32,
}

impl ServerEndpointRecord {
    pub const SCHEMA: u32 = 1;

    pub fn matches(&self, identity: &ServerIdentityResource) -> bool {
        self.schema == Self::SCHEMA
            && self.server_id == identity.id
            && self.pid == identity.pid
            && self.started_at == identity.started_at
            && self.protocol_version == identity.protocol_version
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A recurring runtime background task and whether it is enabled.
pub struct RuntimeTaskResource {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Session cache limits and statistics.
pub struct RuntimeSessionCacheResource {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
    pub session_count: usize,
    pub total_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One run of a scheduled job.
pub struct ScheduledJobRunResource {
    pub triggered_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: ScheduledJobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
}

/// Wire status for a scheduler delivery attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledJobRunStatus {
    Submitted,
    Skipped,
    Failed,
}

#[cfg(test)]
mod scheduled_job_status_tests {
    use super::ScheduledJobRunStatus;

    #[test]
    fn scheduler_status_uses_a_stable_wire_name() {
        assert_eq!(
            serde_json::to_string(&ScheduledJobRunStatus::Submitted)
                .expect("serialize scheduler status"),
            "\"submitted\""
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A scheduled job definition.
pub struct ScheduledJobResource {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expression: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<DateTime<Utc>>,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_fire_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_fired_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_run: Option<ScheduledJobRunResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Automation (scheduled jobs) attached to a session.
pub struct SessionAutomationResource {
    pub job_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_job: Option<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Runtime-wide automation overview.
pub struct RuntimeAutomationResource {
    pub enabled: bool,
    pub job_count: usize,
    pub recent_jobs: Vec<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Runtime operator surface: MCP, LSP, agent, skills, and plugins.
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agent_id: String,
    pub skills: RuntimeSkillsResource,
    pub plugins: RuntimePluginSurfaceResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Neutral plugin surface with the tool registry generation it reflects.
pub struct RuntimePluginSurfaceResource {
    #[serde(default)]
    pub catalog: PluginSurfaceCatalogResource,
    pub tool_registry_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event: Option<agena_plugin_sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Presentation-neutral plugin operations plus terminal-only decoration.
pub struct PluginSurfaceCatalogResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<PluginOperationResource>,
    #[serde(default)]
    pub terminal: PluginTerminalSurfaceCatalogResource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Terminal-only plugin decoration. No executable action is represented here.
pub struct PluginTerminalSurfaceCatalogResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display: Vec<PluginDisplayContributionResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PluginThemePaletteResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// One server-owned plugin operation.
pub struct PluginOperationResource {
    pub plugin_id: String,
    pub accepts_empty_input: bool,
    pub default_input: serde_json::Value,
    #[serde(flatten)]
    pub operation: agena_plugin_sdk::PluginOperationDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A passive display contribution provided by a plugin.
pub struct PluginDisplayContributionResource {
    pub plugin_id: String,
    pub id: String,
    pub kind: agena_plugin_sdk::ContributionKind,
    #[serde(default)]
    pub priority: i32,
    pub content: agena_plugin_sdk::PluginDisplayContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A plugin-provided terminal theme palette.
pub struct PluginThemePaletteResource {
    pub id: String,
    pub plugin_id: String,
    pub display_name: String,
    pub colors: PluginThemeColorsResource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
/// Colors of a plugin terminal theme palette.
pub struct PluginThemeColorsResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub muted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub accent: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub success: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub danger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub special: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_fg: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selection_bg: Option<String>,
}

#[cfg(test)]
mod plugin_surface_catalog_contract_tests {
    use super::{PluginOperationResource, PluginSurfaceCatalogResource};
    use agena_plugin_sdk::{
        OperationDiscoverability, PluginOperationDefinition, PluginOperationTarget,
        SettingsConstraints, SettingsContract, SettingsNode, SettingsNodeKind,
    };

    #[test]
    fn plugin_surface_catalog_has_one_server_owned_operation_shape() {
        let catalog = PluginSurfaceCatalogResource {
            operations: vec![PluginOperationResource {
                plugin_id: "example.tools".to_owned(),
                accepts_empty_input: true,
                default_input: serde_json::json!({}),
                operation: PluginOperationDefinition {
                    id: "summarize".to_owned(),
                    title: "Summarize".to_owned(),
                    description: String::new(),
                    group: "Plugin".to_owned(),
                    category: None,
                    slash: Some("summarize".to_owned()),
                    aliases: Vec::new(),
                    usage: None,
                    input: SettingsContract::new(SettingsNode {
                        id: "input".to_owned(),
                        path: String::new(),
                        title: "Input".to_owned(),
                        description: String::new(),
                        required: true,
                        default: Some(serde_json::json!({})),
                        constraints: SettingsConstraints::default(),
                        sensitive: false,
                        secret: false,
                        kind: SettingsNodeKind::Object { fields: Vec::new() },
                    }),
                    discoverability: OperationDiscoverability::default(),
                    target: PluginOperationTarget::Tool {
                        tool: "summarize".to_owned(),
                    },
                },
            }],
            ..PluginSurfaceCatalogResource::default()
        };

        let wire = serde_json::to_value(catalog).expect("serialize plugin catalog");
        assert_eq!(wire["operations"][0]["plugin_id"], "example.tools");
        assert_eq!(wire["operations"][0]["accepts_empty_input"], true);
        assert_eq!(wire["operations"][0]["id"], "summarize");
        assert_eq!(wire["operations"][0]["target"]["kind"], "tool");
        assert!(wire["operations"][0].get("action").is_none());
        assert!(wire.get("studio").is_none());
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// MCP servers running inside the runtime.
pub struct RuntimeMcpResource {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<RuntimeMcpServerResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One MCP server with its tool count.
pub struct RuntimeMcpServerResource {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// LSP servers and diagnostics overview.
pub struct RuntimeLspResource {
    pub server_count: usize,
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One LSP server definition.
pub struct RuntimeLspServerResource {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Loaded skills and skill commands.
pub struct RuntimeSkillsResource {
    pub skill_count: usize,
    pub command_count: usize,
    pub skills: Vec<RuntimeSkillResource>,
    pub commands: Vec<RuntimeSkillResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One loaded skill or skill command.
pub struct RuntimeSkillResource {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

/// The one runtime-wide default execution selection exposed to clients.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default)]
pub struct DefaultSelectionResource {
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

/// The effective default execution selection the runtime applies when a
/// fresh session starts without explicit run options. Mirrors the
/// `ExecutionSelection` fields the UI needs to resolve default think/speed
/// modes before any session exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
/// Overall runtime status.
pub struct RuntimeStatusResponse {
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub workspace_root: String,
    pub config_path: String,
    pub config_found: bool,
    pub provider_ids: Vec<String>,
    pub plugin_count: usize,
    pub session_runtime_available: bool,
    pub watch_paths: Vec<String>,
    pub reload: RuntimeTaskResource,
    pub session_gc: RuntimeTaskResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_cache: Option<RuntimeSessionCacheResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_catalog: Option<ModelCatalogResponse>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_selection: Option<DefaultSelectionResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub background_tasks: Vec<RuntimeBackgroundTaskResource>,
    pub automation: RuntimeAutomationResource,
    pub operator: RuntimeOperatorResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A runtime background task.
pub struct RuntimeBackgroundTaskResource {
    pub id: String,
    pub kind: RuntimeBackgroundTaskKind,
    pub origin: RuntimeBackgroundTaskOrigin,
    pub title: String,
    pub status: RuntimeBackgroundTaskStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
    pub created_at: DateTime<Utc>,
    pub started_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    pub cancellable: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a runtime background task.
pub enum RuntimeBackgroundTaskKind {
    ModelCatalogRefresh,
    RuntimeReload,
    MarketplaceRegistrySync,
    MarketplacePluginInstall,
    MarketplacePluginUninstall,
    MarketplacePluginUpgrade,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Origin of a runtime background task.
pub enum RuntimeBackgroundTaskOrigin {
    System,
    User,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Status of a runtime background task.
pub enum RuntimeBackgroundTaskStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// List of runtime background tasks.
pub struct RuntimeBackgroundTaskListResponse {
    pub items: Vec<RuntimeBackgroundTaskResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response to starting a runtime background task.
pub struct RuntimeBackgroundTaskStartResponse {
    pub started: bool,
    pub task: RuntimeBackgroundTaskResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response to cancelling a runtime background task.
pub struct RuntimeBackgroundTaskCancelResponse {
    pub task: RuntimeBackgroundTaskResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Summary of the model catalog.
pub struct ModelCatalogResponse {
    #[serde(default)]
    pub refreshing: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogSourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_failure: Option<agena_failure::UserProblem>,
    pub model_count: usize,
}

/// Page-shaped response for the model catalog REST resource.
///
/// This stays in the protocol crate so the REST adapter, remote client, and
/// alternate transports share one public model-catalog contract.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogListResponse {
    pub summary: ModelCatalogResponse,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_origins: Vec<String>,
    pub items: Vec<CatalogModelResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to look up catalog models by id.
pub struct ModelCatalogLookupRequest {
    #[serde(default)]
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response to refreshing the model catalog.
pub struct ModelCatalogRefreshResponse {
    pub started: bool,
    pub task: RuntimeBackgroundTaskResource,
    pub summary: ModelCatalogResponse,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Source of a catalog model entry.
pub enum ModelCatalogSourceKind {
    Generated,
    Cache,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A model entry in the catalog.
pub struct CatalogModelResource {
    pub model_id: String,
    pub source: ModelCatalogSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<ModelLifecycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub knowledge_cutoff: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub release_date: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_updated: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open_weights: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_temperature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_top_p: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_interleaved: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assistant_reasoning_field: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<ModelPricing>,
    #[serde(
        default,
        skip_serializing_if = "ConfiguredModelModeMapResource::is_empty"
    )]
    pub thinking_modes: ConfiguredModelModeMapResource<ConfiguredModelThinkingModeResource>,
    #[serde(
        default,
        skip_serializing_if = "ConfiguredModelModeMapResource::is_empty"
    )]
    pub speed_modes: ConfiguredModelModeMapResource<ConfiguredModelSpeedModeResource>,
    #[serde(
        default,
        skip_serializing_if = "ModelCapabilityPatchResource::is_empty"
    )]
    pub capabilities: ModelCapabilityPatchResource,
}

/// Configured model modes retain the distinction between inheriting a default,
/// explicitly clearing it, and choosing a named mode. Runtime merging logic is
/// intentionally not part of the protocol.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ConfiguredModelModeMapResource<T> {
    #[serde(default)]
    pub default: ConfiguredModeDefaultResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub modes: BTreeMap<String, T>,
}

impl<T> ConfiguredModelModeMapResource<T> {
    pub fn is_empty(&self) -> bool {
        matches!(self.default, ConfiguredModeDefaultResource::Inherit) && self.modes.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "kind", content = "mode", rename_all = "snake_case")]
/// How a default mode is configured (inherit, clear, or explicit).
pub enum ConfiguredModeDefaultResource {
    #[default]
    Inherit,
    Clear,
    Mode(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Configured thinking mode of a model.
pub struct ConfiguredModelThinkingModeResource {
    /// `None` means the catalog did not explicitly set a default marker.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "default")]
    pub is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<ThinkingRequestResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strategy: Option<ConfiguredThinkingStrategyResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effort: Option<ReasoningEffortResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub budget_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<ThinkingDisplayResource>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Configured thinking strategy of a model.
pub enum ConfiguredThinkingStrategyResource {
    Disabled,
    Effort,
    Budget,
    Adaptive,
    RequestOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Configured speed mode of a model.
pub struct ConfiguredModelSpeedModeResource {
    /// `None` means the catalog did not explicitly set a default marker.
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "default")]
    pub is_default: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "ProviderModelRequestOverrideResource::is_empty"
    )]
    pub request_override: ProviderModelRequestOverrideResource,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub adapter_overrides: BTreeMap<String, ProviderModelRequestOverrideResource>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
/// Patch selecting supported or unsupported capabilities.
pub enum CapabilitySelectionPatchResource<T> {
    Supported(Vec<T>),
    Patch(CapabilitySelectionPatchBodyResource<T>),
}

impl<T> CapabilitySelectionPatchResource<T> {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Supported(values) => values.is_empty(),
            Self::Patch(values) => values.is_empty(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(bound(serialize = "T: Serialize", deserialize = "T: Deserialize<'de>"))]
/// Body of a capability selection patch.
pub struct CapabilitySelectionPatchBodyResource<T> {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub supported: Vec<T>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unsupported: Vec<T>,
}

impl<T> CapabilitySelectionPatchBodyResource<T> {
    pub fn is_empty(&self) -> bool {
        self.supported.is_empty() && self.unsupported.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Patch of model capabilities (input modalities and features).
pub struct ModelCapabilityPatchResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<CapabilitySelectionPatchResource<ModelInputModalityResource>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<CapabilitySelectionPatchResource<ModelCapabilityFeatureResource>>,
}

impl ModelCapabilityPatchResource {
    pub fn is_empty(&self) -> bool {
        self.input
            .as_ref()
            .is_none_or(CapabilitySelectionPatchResource::is_empty)
            && self
                .features
                .as_ref()
                .is_none_or(CapabilitySelectionPatchResource::is_empty)
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Input modality of a model.
pub enum ModelInputModalityResource {
    Text,
    Image,
    Document,
    Audio,
    Video,
    File,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Feature capability of a model.
pub enum ModelCapabilityFeatureResource {
    ToolCalling,
    Streaming,
    Reasoning,
    StructuredOutput,
    #[serde(rename = "temperature")]
    Temperature,
}

#[cfg(test)]
mod configured_model_contract_tests {
    use std::collections::BTreeMap;

    use super::{
        CapabilitySelectionPatchBodyResource, CapabilitySelectionPatchResource,
        ConfiguredModeDefaultResource, ConfiguredModelModeMapResource,
        ModelCapabilityFeatureResource, ModelCapabilityPatchResource,
    };

    #[test]
    fn configured_model_defaults_and_capability_patches_are_typed() {
        let modes = ConfiguredModelModeMapResource::<()> {
            default: ConfiguredModeDefaultResource::Mode("high".to_owned()),
            modes: BTreeMap::new(),
        };
        assert_eq!(
            serde_json::to_value(modes).expect("serialize configured modes"),
            serde_json::json!({"default": {"kind": "mode", "mode": "high"}})
        );

        let patch = ModelCapabilityPatchResource {
            input: None,
            features: Some(CapabilitySelectionPatchResource::Patch(
                CapabilitySelectionPatchBodyResource {
                    supported: vec![ModelCapabilityFeatureResource::Reasoning],
                    unsupported: vec![ModelCapabilityFeatureResource::Temperature],
                },
            )),
        };
        assert_eq!(
            serde_json::to_value(patch).expect("serialize capability patch"),
            serde_json::json!({
                "features": {"supported": ["reasoning"], "unsupported": ["temperature"]}
            })
        );
    }
}

/// Lifecycle label for a catalog model in the public wire contract.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelLifecycle {
    Active,
    Preview,
    Beta,
    Alpha,
    Experimental,
    Deprecated,
}

/// Public, decimal-string pricing values for a catalog model. String values
/// preserve provider precision and avoid a floating-point wire contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct ModelPricing {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tiers: Vec<ModelPricingTier>,
}

impl ModelPricing {
    pub fn is_empty(&self) -> bool {
        self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
            && self.tiers.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
/// Pricing tier of a model.
pub struct ModelPricingTier {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_read_usd_per_million_tokens: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_write_usd_per_million_tokens: Option<String>,
}

impl ModelPricingTier {
    pub fn is_empty(&self) -> bool {
        self.tier_type.is_none()
            && self.size_tokens.is_none()
            && self.input_usd_per_million_tokens.is_none()
            && self.output_usd_per_million_tokens.is_none()
            && self.cache_read_usd_per_million_tokens.is_none()
            && self.cache_write_usd_per_million_tokens.is_none()
    }
}

#[cfg(test)]
mod model_pricing_contract_tests {
    use super::{ModelPricing, ModelPricingTier};

    #[test]
    fn pricing_preserves_decimal_values_without_float_coercion() {
        let pricing = ModelPricing {
            input_usd_per_million_tokens: Some("1.2500".to_owned()),
            tiers: vec![ModelPricingTier {
                tier_type: Some("batch".to_owned()),
                size_tokens: Some(1_000_000),
                output_usd_per_million_tokens: Some("2.5000".to_owned()),
                ..ModelPricingTier::default()
            }],
            ..ModelPricing::default()
        };

        assert_eq!(
            serde_json::to_value(pricing).expect("serialize pricing"),
            serde_json::json!({
                "input_usd_per_million_tokens": "1.2500",
                "tiers": [{
                    "tier_type": "batch",
                    "size_tokens": 1_000_000,
                    "output_usd_per_million_tokens": "2.5000"
                }]
            })
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response to a runtime configuration reload.
pub struct RuntimeReloadResponse {
    pub cause: String,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

// ─── Workspaces ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A workspace resource.
pub struct WorkspaceResource {
    pub id: i64,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_count: Option<u64>,
}

// ─── Sessions ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A session resource.
pub struct SessionResource {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    /// Durable user favorite state shared by TUI/Web/API clients.
    #[serde(default)]
    pub favorite: bool,
    /// Durable navigation pin state shared by TUI/Web/API clients.
    #[serde(default)]
    pub pinned: bool,
    pub version: i64,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    /// Authoritative processing state derived from persisted run markers,
    /// pending interactions, and the execution lease. Unlike a client's
    /// request/loading flag, this survives disconnects and can therefore be
    /// used by every client to identify work still owned by the server.
    #[serde(default)]
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_cutoff_seq_global: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<i64>,
    #[serde(default)]
    pub is_subagent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_access: Option<ExecutionAccess>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_status: Option<SubtaskStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Relation of a session to its parent session.
pub enum SessionRelationKind {
    #[default]
    Root,
    Child,
    Fork,
    Rewind,
    Subagent,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Lifecycle state of a session.
pub enum SessionLifecycleState {
    Creating,
    #[default]
    Ready,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
/// Current processing state of a session.
///
/// This is the single client-facing execution state. Durable session facts are
/// derived from parts and leases; the optional live execution and workflow
/// payloads are attached by the application service when it has a full
/// execution snapshot.
pub enum SessionState {
    Creating,
    Ready {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_failure: Option<serde_json::Value>,
    },
    Running {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<ActiveExecutionResource>,
        workflow: WorkflowState,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requests: Vec<PendingInteractiveRequestResource>,
    },
    AwaitingInteraction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<ActiveExecutionResource>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        requests: Vec<PendingInteractiveRequestResource>,
    },
    Interrupted {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        last_failure: Option<serde_json::Value>,
    },
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<serde_json::Value>,
    },
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Ready { last_failure: None }
    }
}

impl SessionState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready { .. } => "ready",
            Self::Running { .. } => "running",
            Self::AwaitingInteraction { .. } => "awaiting_interaction",
            Self::Interrupted { .. } => "interrupted",
            Self::Failed { .. } => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "creating" => Some(Self::Creating),
            "ready" => Some(Self::Ready { last_failure: None }),
            "running" => Some(Self::Running {
                execution: None,
                workflow: WorkflowState::Quiescent,
                requests: Vec::new(),
            }),
            "awaiting_interaction" => Some(Self::AwaitingInteraction {
                run_id: None,
                execution: None,
                requests: Vec::new(),
            }),
            "interrupted" => Some(Self::Interrupted {
                run_id: None,
                reason: None,
                last_failure: None,
            }),
            "failed" => Some(Self::Failed { failure: None }),
            _ => None,
        }
    }

    pub const fn is_running(&self) -> bool {
        matches!(self, Self::Running { .. })
    }

    pub const fn is_awaiting_interaction(&self) -> bool {
        matches!(self, Self::AwaitingInteraction { .. })
    }

    pub const fn needs_recovery(&self) -> bool {
        matches!(self, Self::Interrupted { .. })
    }

    pub const fn is_failed(&self) -> bool {
        matches!(self, Self::Failed { .. })
    }

    /// True only while a durable run is still executing. Waiting for a user
    /// or recovery is attention, not active model work.
    pub const fn is_busy(&self) -> bool {
        self.is_running()
    }

    pub const fn is_attention(&self) -> bool {
        self.is_awaiting_interaction()
            || self.needs_recovery()
            || self.is_failed()
            || matches!(self, Self::Running { requests, .. } if !requests.is_empty())
    }

    pub const fn workflow_state(&self) -> WorkflowState {
        match self {
            Self::Running { workflow, .. } => *workflow,
            Self::AwaitingInteraction { .. } => WorkflowState::AwaitingInteraction,
            _ => WorkflowState::Quiescent,
        }
    }

    pub fn active_execution(&self) -> Option<&ActiveExecutionResource> {
        match self {
            Self::Running { execution, .. } | Self::AwaitingInteraction { execution, .. } => {
                execution.as_ref()
            }
            _ => None,
        }
    }

    pub fn pending_interactive_requests(&self) -> &[PendingInteractiveRequestResource] {
        match self {
            Self::Running { requests, .. } | Self::AwaitingInteraction { requests, .. } => {
                requests.as_slice()
            }
            _ => &[],
        }
    }

    pub fn with_execution_snapshot(
        self,
        execution: Option<ActiveExecutionResource>,
        workflow: WorkflowState,
        requests: Vec<PendingInteractiveRequestResource>,
    ) -> Self {
        match self {
            Self::Running { .. } => Self::Running {
                execution,
                workflow,
                requests,
            },
            Self::AwaitingInteraction { run_id, .. } => Self::AwaitingInteraction {
                run_id,
                execution,
                requests,
            },
            other => other,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Server-owned session home view.
///
/// `attention`, `running`, and `recent` partition sessions by execution state.
/// `favorites` is an independent durable navigation bucket, so a favorite may
/// also appear in one of the state buckets.
pub struct SessionOverviewResource {
    /// User-favorited sessions, newest first.
    #[serde(default)]
    pub favorites: Vec<SessionResource>,
    /// Sessions paused on user input or left interrupted after owner loss.
    pub attention: Vec<SessionResource>,
    /// Sessions whose execution lease is fresh, plus sessions still creating.
    pub running: Vec<SessionResource>,
    /// Most recently changed terminal/quiescent sessions.
    pub recent: Vec<SessionResource>,
    pub generated_at: DateTime<Utc>,
}

#[cfg(test)]
mod session_state_contract_tests {
    use super::{SessionState, WorkflowState};

    #[test]
    fn session_processing_states_have_stable_wire_names() {
        let cases = [
            (SessionState::Creating, "creating"),
            (SessionState::Ready { last_failure: None }, "ready"),
            (
                SessionState::Running {
                    execution: None,
                    workflow: WorkflowState::Quiescent,
                    requests: Vec::new(),
                },
                "running",
            ),
            (
                SessionState::AwaitingInteraction {
                    run_id: None,
                    execution: None,
                    requests: Vec::new(),
                },
                "awaiting_interaction",
            ),
            (
                SessionState::Interrupted {
                    run_id: None,
                    reason: None,
                    last_failure: None,
                },
                "interrupted",
            ),
            (SessionState::Failed { failure: None }, "failed"),
        ];
        for (state, expected) in cases {
            assert_eq!(state.as_str(), expected);
            let wire = serde_json::to_value(&state).expect("serialize session state");
            assert_eq!(
                wire.get("kind").and_then(serde_json::Value::as_str),
                Some(expected)
            );
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Status of a subtask.
pub enum SubtaskStatus {
    #[default]
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An active execution inside a session.
pub struct ActiveExecutionResource {
    pub execution_id: Uuid,
    pub phase: ExecutionPhase,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Phase of an active execution.
pub enum ExecutionPhase {
    Starting,
    PreparingModel,
    StreamingModel,
    ExecutingTools,
    AwaitingInteraction,
    Cancelling,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Workflow state of a session.
pub enum WorkflowState {
    #[default]
    Quiescent,
    ToolPending,
    AwaitingInteraction,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// Execution access mode of a session.
pub enum ExecutionAccess {
    #[default]
    Inherit,
    ReadOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Execution context of a session: agent, access, and permission configuration.
pub struct SessionExecutionContextResource {
    pub agent_id: String,
    pub execution_access: ExecutionAccess,
    #[serde(default, skip_serializing_if = "PermissionConfigResource::is_empty")]
    pub selected_permission: PermissionConfigResource,
    #[serde(default, skip_serializing_if = "PermissionConfigResource::is_empty")]
    pub effective_permission: PermissionConfigResource,
    #[serde(default, skip_serializing_if = "PermissionConfigResource::is_empty")]
    pub permission_ceiling: PermissionConfigResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_thinking_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_speed_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_status: Option<SubtaskStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub subtask_failure: Option<agena_failure::UserProblem>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Basis of a session usage limit.
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Token usage of a session.
pub struct SessionUsageResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_prompt_tokens: Option<u64>,
    pub current_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projected_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_basis: Option<SessionUsageLimitBasis>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reserved_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_context_window_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_max_input_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_max_output_tokens: Option<u32>,
}

/// One v2 part in a session transcript projection.
///
/// The transcript is the session's ordered v2 part list ("everything is a
/// part", database-design-v2.md 4.1.1). Each projected run contributes a `run`
/// marker part followed by its content parts; `run_id` links content parts to
/// their marker, and the marker's `state` mirrors the run/reply status.
///
/// This is a wire projection of the parts surfaced by the runtime
/// `SessionQueryService`; `parent_part_id`/`run_id` are populated when the
/// projection exposes them (both are `None` for fields the current projection
/// does not carry). `kind`/`role`/`state` are stable strings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SessionTranscriptPart {
    pub part_id: i64,
    pub kind: String,
    pub role: String,
    pub state: String,
    pub content: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub presentation: Option<crate::live::ToolHumanPresentationResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_part_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
}

impl From<crate::live::PartResource> for SessionTranscriptPart {
    fn from(value: crate::live::PartResource) -> Self {
        Self {
            part_id: value.part_id,
            kind: value.kind,
            role: value.role,
            state: value.state,
            content: value.content,
            presentation: value.presentation,
            summary: value.summary,
            created_at_ms: value.created_at_ms,
            parent_part_id: value.parent_part_id,
            run_id: value.run_id,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Full execution view of a session.
pub struct SessionExecutionResource {
    pub session: SessionResource,
    /// The session's v2 parts (ordered parts, including `run` markers).
    pub parts: Vec<SessionTranscriptPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation: Option<SessionAutomationResource>,
    /// Non-terminal background members owned by this session. This is the
    /// single session-scoped projection consumed by both composer footers and
    /// Web; clients must not maintain a second counter.
    #[serde(default)]
    pub background_activities: Vec<BackgroundActivityResource>,
    pub execution: SessionExecutionContextResource,
    pub usage: SessionUsageResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A pending interactive request (permission or user input).
pub struct PendingInteractiveRequestResource {
    pub session_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(flatten)]
    pub request: PendingInteractiveRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default, deny_unknown_fields)]
/// Run options for a session execution.
pub struct RunOptions {
    #[serde(default)]
    pub model: Option<ModelRef>,
    #[serde(default)]
    pub thinking_mode: Option<String>,
    #[serde(default)]
    pub speed_mode: Option<String>,
    #[serde(default)]
    pub verbosity: Option<String>,
    #[serde(default)]
    pub parallel_tool_calls: Option<bool>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
}

#[cfg(test)]
mod run_options_contract_tests {
    use super::RunOptions;

    #[test]
    fn nested_run_options_reject_removed_agent_selection_fields() {
        for field in ["agent_profile", "profile", "subagent_type"] {
            let mut options = serde_json::Map::new();
            options.insert(field.to_owned(), serde_json::json!("build"));
            let error = serde_json::from_value::<RunOptions>(serde_json::Value::Object(options))
                .expect_err("removed agent selection must not be silently accepted");
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }
}

/// Provider, optional adapter, and model selection carried over the public
/// wire. The application validates these strings and constructs its internal
/// model identifier only after receiving the request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct ModelRef {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    pub model_id: String,
}

impl ModelRef {
    pub fn new(provider_id: impl Into<String>, model_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            adapter_id: None,
            model_id: model_id.into(),
        }
    }

    pub fn new_with_adapter(
        provider_id: impl Into<String>,
        adapter_id: impl Into<String>,
        model_id: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            adapter_id: Some(adapter_id.into()),
            model_id: model_id.into(),
        }
    }
}

#[cfg(test)]
mod model_ref_contract_tests {
    use super::ModelRef;

    #[test]
    fn model_ref_serializes_without_runtime_identifier_types() {
        let model = ModelRef::new_with_adapter("provider", "adapter", "model");
        assert_eq!(
            serde_json::to_value(model).expect("serialize model reference"),
            serde_json::json!({
                "provider_id": "provider",
                "adapter_id": "adapter",
                "model_id": "model"
            })
        );
    }
}

// ─── Messages ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Role of a message.
pub enum RunRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Public execution state of a message. This is a wire value, deliberately
/// separate from the persistence-enabled runtime state enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    #[default]
    Pending,
    InProgress,
    Completed,
    PolicyDenied,
    UserDeclined,
    CapabilityUnavailable,
    ToolUnavailable,
    Failed,
    Cancelled,
}

/// Token and cost accounting for one public message projection.
///
/// The wire representation intentionally does not carry the runtime's
/// database serialization implementation.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
pub struct RunUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cache_read_tokens: u64,
    pub total_cost: f64,
}

/// Origin of a message in the public conversation projection.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum RunSource {
    User,
    Assistant,
    System,
}

/// Display and lineage metadata for a message. Provider-private replay state
/// remains runtime-only and is intentionally absent from this wire contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RunMetadata {
    pub source: RunSource,
    /// Stable external delivery key, when this message was submitted by a
    /// retry-capable integration such as the scheduler.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_turn_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_message_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub generated_by_call_id: Option<i64>,
    pub model_provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_adapter_id: Option<String>,
    pub model_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_speed_mode: Option<String>,
}

impl Default for RunMetadata {
    fn default() -> Self {
        Self {
            source: RunSource::Assistant,
            idempotency_key: None,
            model_turn_id: None,
            parent_message_id: None,
            generated_by_call_id: None,
            model_provider_id: String::new(),
            model_adapter_id: None,
            model_id: String::new(),
            model_thinking_mode: None,
            model_speed_mode: None,
        }
    }
}

#[cfg(test)]
mod message_status_contract_tests {
    use super::RunStatus;

    #[test]
    fn message_status_has_a_stable_wire_name() {
        assert_eq!(
            serde_json::to_string(&RunStatus::InProgress).expect("serialize message status"),
            "\"in_progress\""
        );
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// How message parts are loaded in a listing.
pub enum PartLoadMode {
    None,
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A message in a session transcript.
pub struct RunResource {
    pub id: i64,
    pub session_id: i64,
    pub role: RunRole,
    pub state: RunStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: RunMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<RunUsage>,
    pub part_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<crate::part::PartResource>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Reference to a skill attached to a message.
pub struct PartSkillReference {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub content_hash: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An attachment of a message part.
pub struct PartAttachment {
    pub kind: PartAttachmentKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    pub source: PartAttachmentSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_count: Option<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a message attachment.
pub enum PartAttachmentKind {
    Image,
    Audio,
    Video,
    Pdf,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
/// Source of a message attachment.
pub enum PartAttachmentSource {
    Url { url: String },
    DataUrl { url: String },
    Base64 { data: String },
    FileId { file_id: String },
    LocalPath { path: String },
}

// ─── Permission rules ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A persisted permission rule.
pub struct PermissionRuleResource {
    pub id: i64,
    pub action_key: String,
    pub subject_kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub qualifier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path_access_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_target: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network_port: Option<u16>,
    pub mode: PermissionMode,
    pub scope: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<i64>,
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revoked_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Stable permission-rule decision persisted and exposed by the API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionMode {
    Allow,
    Auto,
    Ask,
    Deny,
}

/// Public projection of an execution permission configuration.
///
/// This is a declarative wire value. It deliberately contains no policy
/// evaluation code, filesystem handles, or plugin runtime types; the
/// application layer translates it to and from the runtime policy model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathPermissionConfigResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub network: Option<NetworkPermissionConfigResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools: Option<ToolPermissionConfigResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub approval_model: Option<ApprovalModelSelectionResource>,
}

impl PermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.path
            .as_ref()
            .is_none_or(PathPermissionConfigResource::is_empty)
            && self
                .network
                .as_ref()
                .is_none_or(NetworkPermissionConfigResource::is_empty)
            && self
                .tools
                .as_ref()
                .is_none_or(ToolPermissionConfigResource::is_empty)
            && self.approval_model.is_none()
    }
}

/// Concrete automatic-permission model selection, including the inference
/// variants chosen for the approval request.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ApprovalModelSelectionResource {
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    pub model: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speed_mode: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parallel_tool_calls: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// Path permission configuration of a session.
pub struct PathPermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModesResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModesResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, PathAccessRuleResource>,
}

impl PathPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.external.is_none() && self.rules.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// Read/write access modes for a path scope.
pub struct PathAccessModesResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub write: Option<PermissionMode>,
}

/// A path rule preserves the two accepted configuration forms: explicit
/// read/write modes or a concise policy shorthand.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
pub enum PathAccessRuleResource {
    Modes(PathAccessModesResource),
    Shorthand(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// Network permission configuration of a session.
pub struct NetworkPermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub internet: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub private: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loopback: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, PermissionMode>,
}

impl NetworkPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.internet.is_none()
            && self.private.is_none()
            && self.loopback.is_none()
            && self.rules.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
/// Tool permission configuration of a session.
pub struct ToolPermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, ToolPermissionRulesResource>,
}

impl ToolPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.names.is_empty() && self.rules.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
/// Tool permission rules: a single mode or an ordered name-to-mode map.
pub enum ToolPermissionRulesResource {
    Mode(PermissionMode),
    Ordered(BTreeMap<String, PermissionMode>),
}

#[cfg(test)]
mod permission_config_resource_contract_tests {
    use std::collections::BTreeMap;

    use super::{
        PathAccessModesResource, PathAccessRuleResource, PathPermissionConfigResource,
        PermissionConfigResource, PermissionMode,
    };

    #[test]
    fn permission_configuration_is_a_self_contained_wire_contract() {
        let config = PermissionConfigResource {
            path: Some(PathPermissionConfigResource {
                workspace: Some(PathAccessModesResource {
                    read: Some(PermissionMode::Allow),
                    write: Some(PermissionMode::Ask),
                }),
                external: None,
                rules: BTreeMap::from([(
                    "/tmp/**".to_owned(),
                    PathAccessRuleResource::Shorthand("deny".to_owned()),
                )]),
            }),
            network: None,
            tools: None,
            approval_model: None,
        };

        assert_eq!(
            serde_json::to_value(config).expect("serialize permission config"),
            serde_json::json!({
                "path": {
                    "workspace": {"read": "allow", "write": "ask"},
                    "rules": {"/tmp/**": "deny"}
                }
            })
        );
    }
}

/// Persistence scope selected for a permission decision in the public API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionScope {
    Session,
    Workspace,
    Global,
}

/// Permission decision sent by a client. It is a wire value, not a runtime
/// policy or persistence type.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionReplyKind {
    AllowOnce,
    AllowAlways,
    DenyOnce,
    DenyAlways,
    AutoApprove,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Reply to a pending permission request.
pub struct PermissionReply {
    pub request_id: String,
    pub kind: PermissionReplyKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a user-input reply.
pub enum UserInputReplyKind {
    Submit,
    Cancel,
    Timeout,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Reply to a pending user-input request.
pub struct UserInputReply {
    pub request_id: String,
    pub kind: UserInputReplyKind,
    #[serde(default, skip_serializing_if = "user_input_answers_is_empty")]
    pub answers: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// An action subject to permission checks.
pub enum PermissionActionResource {
    Tool {
        tool_name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        qualifier: Option<String>,
    },
    PathAccess {
        access_kind: String,
        workspace_root: String,
        target_path: String,
    },
    NetworkAccess {
        target: String,
        host: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        port: Option<u16>,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Source of a permission policy decision.
pub enum PolicySourceKind {
    StaticPolicy,
    PersistedRule,
    PluginAdvice,
    ManagedPolicy,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// One step of a permission decision trace.
pub struct DecisionTraceStep {
    pub source_kind: PolicySourceKind,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A pending permission request.
pub struct PermissionRequest {
    pub request_id: String,
    pub session_id: Option<i64>,
    pub action: PermissionActionResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_actions: Vec<PermissionActionResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requested_actions: Vec<PermissionActionResource>,
    pub reason: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub explanation: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<PermissionScope>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operator: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub trace: Vec<DecisionTraceStep>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An option of a user-input question.
pub struct UserInputOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A question asked to the user.
pub struct UserInputQuestion {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub header: String,
    pub question: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<UserInputOption>,
    #[serde(default, skip_serializing_if = "is_false")]
    pub multiple: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub allow_custom: bool,
}

#[cfg(test)]
mod provider_model_resource_contract_tests {
    use std::collections::BTreeMap;

    use super::{
        CapabilitySupportResource, ProviderModelCapabilitiesResource, ProviderModelResource,
        ProviderModelSpeedModeResource, ProviderModelThinkingModeResource, ThinkingRequestResource,
    };

    #[test]
    fn provider_model_is_a_complete_api_owned_route_projection() {
        let model = ProviderModelResource {
            provider_id: "example".to_owned(),
            adapter_id: Some("responses".to_owned()),
            id: "example-1".to_owned(),
            catalog_model_id: Some("catalog-1".to_owned()),
            display_name: Some("Example 1".to_owned()),
            native_compaction: true,
            capabilities: ProviderModelCapabilitiesResource {
                reasoning: CapabilitySupportResource::Supported,
                ..ProviderModelCapabilitiesResource::default()
            },
            metadata: Default::default(),
            thinking_modes: vec![ProviderModelThinkingModeResource {
                is_default: true,
                display_name: Some("High".to_owned()),
                description: None,
                preset: None,
                thinking: Some(ThinkingRequestResource::Budget {
                    budget_tokens: 4096,
                }),
                request_override: Default::default(),
                adapter_overrides: BTreeMap::new(),
            }],
            speed_modes: BTreeMap::from([(
                "fast".to_owned(),
                ProviderModelSpeedModeResource {
                    is_default: true,
                    display_name: Some("Fast".to_owned()),
                    description: None,
                    request_override: Default::default(),
                    adapter_overrides: BTreeMap::new(),
                },
            )]),
        };

        let value = serde_json::to_value(model).expect("serialize provider model");
        assert_eq!(value["provider_id"], "example");
        assert_eq!(value["capabilities"]["reasoning"], "supported");
        assert_eq!(
            value["thinking_modes"][0]["thinking"],
            serde_json::json!({
                "type": "budget",
                "budget_tokens": 4096
            })
        );
        assert_eq!(value["speed_modes"]["fast"]["default"], true);
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to probe adapter models from a provider endpoint.
pub struct ProviderAdapterModelsRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub protocol_paths: crate::queries::ProviderProtocolPaths,
    #[serde(default)]
    pub api_key: Option<crate::queries::ProviderSecretSource>,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to list saved provider adapter models.
pub struct SavedProviderAdapterModelsRequest {
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Adapter model probe result for one adapter.
pub struct ProviderAdapterModelsResource {
    pub adapter_id: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base_url: Option<String>,
    #[serde(default)]
    pub models: Vec<ProviderModelResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure: Option<agena_failure::UserProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response containing provider adapter model probes.
pub struct ProviderAdapterModelsResponse {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterModelsResource>,
}
