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
/// Runtime operator surface: MCP, LSP, agent, skills, and plugin UI.
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agent_id: String,
    pub skills: RuntimeSkillsResource,
    pub ui: RuntimePluginUiResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Plugin UI catalog with the tool registry generation it reflects.
pub struct RuntimePluginUiResource {
    #[serde(default)]
    pub catalog: PluginUiCatalogResource,
    pub tool_registry_generation: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_registry_last_event: Option<agena_plugin_sdk::host_api::ToolRegistryChangedEvent>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Plugin UI catalog combining TUI and studio contributions.
pub struct PluginUiCatalogResource {
    pub tui: PluginTuiUiCatalogResource,
    pub studio: PluginStudioUiCatalogResource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// TUI plugin UI contributions (display blocks and themes).
pub struct PluginTuiUiCatalogResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub display: Vec<PluginDisplayContributionResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub themes: Vec<PluginThemePaletteResource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Studio plugin UI contributions (commands, controls, and views).
pub struct PluginStudioUiCatalogResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginCommandResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControlResource>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<PluginStudioViewResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A display contribution provided by a plugin.
pub struct PluginDisplayContributionResource {
    pub plugin_id: String,
    pub id: String,
    pub kind: agena_plugin_sdk::ContributionKind,
    #[serde(default)]
    pub priority: i32,
    pub content: agena_plugin_sdk::PluginDisplayContent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A plugin-provided theme palette.
pub struct PluginThemePaletteResource {
    pub id: String,
    pub plugin_id: String,
    pub display_name: String,
    pub colors: PluginThemeColorsResource,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default, deny_unknown_fields)]
/// Colors of a plugin theme palette.
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plugin studio command.
pub struct PluginCommandResource {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub category: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<String>,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_schema: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handler: Option<String>,
    #[serde(default)]
    pub action: PluginUiActionResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plugin studio control.
pub struct PluginStudioControlResource {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub location: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<PluginStudioControlOptionResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<serde_json::Value>,
    #[serde(default)]
    pub action: PluginUiActionResource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An option of a plugin studio control.
pub struct PluginStudioControlOptionResource {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// A plugin studio view.
pub struct PluginStudioViewResource {
    pub plugin_id: String,
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub location: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<PluginStudioControlResource>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
/// Action triggered by a plugin UI element.
pub enum PluginUiActionResource {
    #[default]
    None,
    InvokeTool {
        tool: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
        #[serde(default)]
        submit_output_as_prompt: bool,
    },
    OpenPluginWorkbench {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tab: Option<String>,
    },
    OpenUrl {
        url: String,
    },
    SubmitPrompt {
        prompt: String,
    },
    InvokeCommand {
        command: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        input: Option<serde_json::Value>,
    },
}

#[cfg(test)]
mod plugin_ui_catalog_contract_tests {
    use super::{
        PluginCommandResource, PluginStudioUiCatalogResource, PluginTuiUiCatalogResource,
        PluginUiActionResource, PluginUiCatalogResource,
    };

    #[test]
    fn plugin_ui_catalog_has_an_api_owned_typed_action_shape() {
        let catalog = PluginUiCatalogResource {
            tui: PluginTuiUiCatalogResource::default(),
            studio: PluginStudioUiCatalogResource {
                commands: vec![PluginCommandResource {
                    plugin_id: "example.tools".to_owned(),
                    id: "summarize".to_owned(),
                    title: "Summarize".to_owned(),
                    description: String::new(),
                    category: "Plugin".to_owned(),
                    slash: Some("/summarize".to_owned()),
                    aliases: Vec::new(),
                    usage: None,
                    location: "command_palette".to_owned(),
                    input_schema: None,
                    handler: Some("summarize".to_owned()),
                    action: PluginUiActionResource::InvokeTool {
                        tool: "summarize".to_owned(),
                        input: None,
                        submit_output_as_prompt: true,
                    },
                }],
                controls: Vec::new(),
                views: Vec::new(),
            },
        };

        assert_eq!(
            serde_json::to_value(catalog).expect("serialize plugin catalog"),
            serde_json::json!({
                "tui": {},
                "studio": {
                    "commands": [{
                        "plugin_id": "example.tools",
                        "id": "summarize",
                        "title": "Summarize",
                        "category": "Plugin",
                        "slash": "/summarize",
                        "location": "command_palette",
                        "handler": "summarize",
                        "action": {
                            "kind": "invoke_tool",
                            "tool": "summarize",
                            "submit_output_as_prompt": true
                        }
                    }]
                }
            })
        );
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

/// The effective default execution selection the runtime applies when a
/// fresh session starts without explicit run options. Mirrors the
/// `ExecutionSelection` fields the UI needs to resolve default think/speed
/// modes before any session exists.
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
}

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
    pub version: i64,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
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
    Blocked,
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
    pub summary: Option<String>,
    pub created_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_part_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Full execution view of a session.
pub struct SessionExecutionResource {
    pub session: SessionResource,
    /// The session's v2 parts (ordered parts, including `run` markers).
    /// Replaces the v1 `TranscriptSnapshot` aggregate.
    pub parts: Vec<SessionTranscriptPart>,
    pub workflow_state: WorkflowState,
    pub active_execution: Option<ActiveExecutionResource>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation: Option<SessionAutomationResource>,
    pub execution: SessionExecutionContextResource,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_interactive_requests: Vec<PendingInteractiveRequestResource>,
    pub usage: SessionUsageResource,
}

/// The lazily-derived human detail of one tool Activity. Returned by the
/// `GetOperationDetail` query when a client expands an Operation; the runtime
/// derives the Markdown from the compact tool data, so nothing large is
/// persisted or transferred while the Activity stays collapsed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationDetailResource {
    pub activity_id: agena_domain::ActivityId,
    pub markdown: String,
    #[serde(default)]
    pub streaming: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub enum MessageRole {
    User,
    Assistant,
    System,
    Tool,
}

/// Public execution state of a message. This is a wire value, deliberately
/// separate from the persistence-enabled runtime state enum.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum MessageStatus {
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
pub struct MessageUsage {
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
pub enum MessageSource {
    User,
    Assistant,
    System,
}

/// Display and lineage metadata for a message. Provider-private replay state
/// remains runtime-only and is intentionally absent from this wire contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MessageMetadata {
    pub source: MessageSource,
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

impl Default for MessageMetadata {
    fn default() -> Self {
        Self {
            source: MessageSource::Assistant,
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
    use super::MessageStatus;

    #[test]
    fn message_status_has_a_stable_wire_name() {
        assert_eq!(
            serde_json::to_string(&MessageStatus::InProgress).expect("serialize message status"),
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
pub struct MessageResource {
    pub id: i64,
    pub session_id: i64,
    pub role: MessageRole,
    pub state: MessageStatus,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: MessageMetadata,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<MessageUsage>,
    pub part_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<crate::message_part::MessagePartResource>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Reference to a skill attached to a message.
pub struct MessageSkillReference {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub instructions: String,
    pub content_hash: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An attachment of a message part.
pub struct MessageAttachment {
    pub kind: MessageAttachmentKind,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub mime: String,
    pub source: MessageAttachmentSource,
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
pub enum MessageAttachmentKind {
    Image,
    Audio,
    Video,
    Pdf,
    File,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
/// Source of a message attachment.
pub enum MessageAttachmentSource {
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
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapter_id: Option<String>,
    pub model_id: String,
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
    #[serde(
        default,
        deserialize_with = "deserialize_user_input_answers",
        skip_serializing_if = "user_input_answers_is_empty"
    )]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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
