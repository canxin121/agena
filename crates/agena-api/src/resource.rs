//! REST resource projections for the unified API surface, reorganised by
//! domain so external clients can `use agena_api::resource::*`.

use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// Session-facing values now live in the shared contract crate so every
// transport (REST, WebSocket, plugins) reads one definition.
pub use agena_session_api::activity::{BackgroundActivityLogResource, BackgroundActivityResource};
pub use agena_session_api::resource::{
    ActiveExecutionResource, ApprovalModelSelectionResource, DecisionTraceStep,
    ExecutionAttemptResource, HealthResponse, ModelRef, NetworkPermissionConfigResource,
    PartAttachment, PartAttachmentKind, PartAttachmentSource, PartCommandReference, PartLoadMode,
    PathAccessModesResource, PathPermissionConfigResource, PendingInteractiveRequestResource,
    PermissionActionResource, PermissionConfigResource, PermissionMode, PermissionReply,
    PermissionReplyKind, PermissionRequest, PermissionRuleResource, PermissionScope,
    PolicySourceKind, QueuedMessageEditResource, QueuedMessageResource, RunMetadata, RunOptions,
    RunRole, RunSource, RunUsage, ScheduledJobResource, ScheduledJobRunResource,
    ScheduledJobRunStatus, ServerIdentityResource, SessionAutomationResource,
    SessionExecutionContextResource, SessionExecutionReceiptResource, SessionExecutionResource,
    SessionResource, SessionState, SessionUsageLimitBasis, SessionUsageResource,
    ToolPermissionConfigResource, ToolPermissionRulesResource, UserInputOption, UserInputQuestion,
    UserInputReply, UserInputReplyKind, WorkspaceFileKind, WorkspaceFileNode,
    WorkspaceFileTreeResource, WorkspaceResource, WorkspaceSessionStats,
};

use std::collections::BTreeMap;

mod activity;
mod auth;
mod conversation;
mod file_changes;
mod interaction;
mod notification;
mod permission;
pub use activity::*;
pub use auth::*;
pub use conversation::*;
pub use file_changes::*;
pub use interaction::*;
pub use notification::*;

// Session-state enums are defined once in `agena-domain`; the API re-exports
// them so every server layer, client, and generated TypeScript mirror shares
// one definition instead of keeping local copies of the same variants.
pub use agena_domain::{
    AgentStatus, ExecutionPhase, SessionLifecycleState, SessionRelationKind, SessionStateKind,
    WorkflowState,
};

fn is_false(value: &bool) -> bool {
    !*value
}

// ─── Health / runtime ────────────────────────────────────────────────────

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

impl ModelCatalogListResponse {
    /// Client-local empty state before the first catalog page is loaded.
    pub fn empty() -> Self {
        Self {
            summary: ModelCatalogResponse {
                refreshing: false,
                last_refresh_at: None,
                last_successful_source: None,
                last_failure: None,
                model_count: 0,
            },
            total: 0,
            offset: 0,
            limit: 0,
            available_origins: Vec::new(),
            items: Vec::new(),
        }
    }
}

/// A verified, display-only catalog suggestion for a provider model ID.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogModelMatchResource {
    pub model_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog: Option<CatalogModelResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogMatchResponse {
    pub items: Vec<CatalogModelMatchResource>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A model entry in the catalog.
pub struct CatalogModelResource {
    pub model_id: String,
    pub source: ModelCatalogSourceKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_sources: Vec<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub verbosity_by_provider: BTreeMap<String, CatalogProviderVerbosityProfileResource>,
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
    // REST has always exposed `input` and `features` at the model level.
    #[serde(flatten)]
    pub capabilities: ModelCapabilityPatchResource,
}

// Shared provider configuration values have the same wire contract as catalog
// metadata. Their optional/default/flatten rules must be maintained once.
pub use agena_domain::ModelInputModality as ModelInputModalityResource;
pub use agena_provider::{
    CapabilitySelectionPatch as CapabilitySelectionPatchResource,
    CapabilitySelectionPatchBody as CapabilitySelectionPatchBodyResource,
    CatalogProviderVerbosityProfile as CatalogProviderVerbosityProfileResource,
    ConfiguredModeDefault as ConfiguredModeDefaultResource,
    ConfiguredModelModeMap as ConfiguredModelModeMapResource,
    ConfiguredModelSpeedMode as ConfiguredModelSpeedModeResource,
    ConfiguredModelThinkingMode as ConfiguredModelThinkingModeResource,
    ConfiguredThinkingStrategy as ConfiguredThinkingStrategyResource,
    ModelCapabilityFeature as ModelCapabilityFeatureResource,
    ModelCapabilityPatch as ModelCapabilityPatchResource,
};
pub mod model_catalog;

// Stable model value types have identical public and domain semantics.
pub use agena_domain::{ModelLifecycle, ModelPricing, ModelPricingTier};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response to a runtime configuration reload.
pub struct RuntimeReloadResponse {
    pub cause: String,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings edit.
pub struct ConfigSettingsEditResponse {
    #[serde(default)]
    pub before_revision: String,
    #[serde(default)]
    pub after_revision: String,
    pub config_path: std::path::PathBuf,
    pub config_found: bool,
    pub operation: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub dry_run: bool,
    pub changed: bool,
    pub created: bool,
    pub deleted: bool,
    pub validated: bool,
    pub reload_requested: bool,
    pub reload_required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reload: Option<ConfigSettingsReloadResponse>,
    pub previous: serde_json::Value,
    pub current: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Response of a settings reload.
pub struct ConfigSettingsReloadResponse {
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: String,
}

// ─── Workspaces ──────────────────────────────────────────────────────────

// ─── Sessions ────────────────────────────────────────────────────────────

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
    /// Sessions paused on user input, plus sessions that terminally failed.
    pub attention: Vec<SessionResource>,
    /// Sessions with an in-flight run, plus sessions still creating.
    pub running: Vec<SessionResource>,
    /// Most recently changed terminal/quiescent sessions.
    pub recent: Vec<SessionResource>,
    pub generated_at: DateTime<Utc>,
}

/// Identity of the session atomically published by an archive import.
/// This receipt does not require a provider or a subsequent state/history read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SessionArchiveImportResource {
    pub session_id: i64,
}

pub use agena_domain::session_kernel::ExecutionAttemptSettlement;

// ─── Messages ────────────────────────────────────────────────────────────

/// Public execution state of a message.
///
/// The canonical definition is [`agena_domain::ExecutionStatus`]; the API
/// re-exports it under the message-projection name so a message's status and
/// an operation part's [`PartExecutionStatusResource`] cannot drift apart.
pub use agena_domain::ExecutionStatus as RunStatus;

// ─── Permission rules ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
/// Request to list saved provider adapter models.
pub struct SavedProviderAdapterModelsRequest {
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[cfg(test)]
mod model_catalog_contract_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn catalog_reads_the_existing_flat_wire_contract_and_omits_absent_fields() {
        let value = json!({
            "model_id": "fixture-model",
            "source": "cache",
            "input": ["text", "image"],
            "features": { "supported": ["reasoning"], "unsupported": ["temperature"] },
            "thinking_modes": {
                "default": "high",
                "high": { "strategy": "effort", "effort": "high" }
            },
            "speed_modes": { "default": null, "fast": { "disabled": true } }
        });
        let model: CatalogModelResource = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(model.thinking_modes.default.mode(), Some("high"));
        assert_eq!(
            model.speed_modes.default,
            ConfiguredModeDefaultResource::Clear
        );
        assert_eq!(
            model
                .capabilities
                .input_support(ModelInputModalityResource::Image),
            Some(agena_domain::CapabilitySupport::Supported)
        );
        assert_eq!(serde_json::to_value(model).unwrap(), value);
    }

    #[test]
    fn mode_default_distinguishes_missing_null_and_name() {
        for (value, expected) in [
            (json!({}), ConfiguredModeDefaultResource::Inherit),
            (
                json!({"default": null}),
                ConfiguredModeDefaultResource::Clear,
            ),
            (
                json!({"default": "fast"}),
                ConfiguredModeDefaultResource::Mode("fast".into()),
            ),
        ] {
            let modes: ConfiguredModelModeMapResource<ConfiguredModelSpeedModeResource> =
                serde_json::from_value(value.clone()).unwrap();
            assert_eq!(modes.default, expected);
            assert_eq!(serde_json::to_value(modes).unwrap(), value);
        }
    }

    #[test]
    fn catalog_page_defaults_only_optional_fields() {
        let page: ModelCatalogListResponse = serde_json::from_value(json!({
            "summary": {"model_count": 0}, "items": [], "total": 0, "offset": 0, "limit": 50
        }))
        .unwrap();
        assert!(!page.summary.refreshing);
        assert!(page.available_origins.is_empty());
        assert!(page.summary.last_successful_source.is_none());
        assert!(
            serde_json::from_value::<ModelCatalogListResponse>(json!({
                "summary": {"model_count": 0}, "items": [], "total": 0, "offset": 0
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<CatalogModelResource>(json!({"model_id": "missing-source"}))
                .is_err()
        );
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A file staged in the selected instance for one workspace. `path` is the
/// absolute managed path clients should reference as a workspace attachment.
pub struct WorkspaceFileUploadResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub workspace_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    pub path: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Git status of a workspace.
pub struct GitStatusResource {
    pub workspace_root: String,
    pub git_available: bool,
    pub repo: bool,
    pub gh_available: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upstream: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub behind: Option<u64>,
    pub staged_files: u64,
    pub unstaged_files: u64,
    pub untracked_files: u64,
    pub changed_files: u64,
    pub clean: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Result of a git commit.
pub struct GitCommitResource {
    pub commit: String,
    pub summary: String,
    pub status: GitStatusResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A created pull request.
pub struct GitPullRequestResource {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// Presentation-neutral registered tool metadata used by permission UIs.
pub struct PermissionToolCatalogResource {
    pub name: String,
    pub summary: String,
    /// UI-only summaries keyed by BCP-47 locale; permission editors pick
    /// the client language and fall back to `summary`.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub summary_translations: std::collections::BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
}

/// Complete JSON configuration-source projection for presentation and
/// application-level configuration use cases. Runtime retains configuration
/// resolution and file settings implementation; consumers never assemble this
/// view from Runtime snapshots themselves.
#[derive(Debug, Clone, Serialize, Deserialize)]
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

mod plugins;
pub use plugins::*;

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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Source of a catalog model entry.
pub enum ModelCatalogSourceKind {
    Generated,
    Cache,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
/// One server-owned plugin command.
pub struct PluginCommandResource {
    pub plugin_id: String,
    pub accepts_empty_input: bool,
    pub default_input: serde_json::Value,
    #[serde(flatten)]
    pub command: agena_plugin_sdk::CommandDefinition,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
/// Presentation-neutral plugin commands plus terminal-only decoration.
pub struct PluginSurfaceCatalogResource {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<PluginCommandResource>,
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A plugin-provided terminal theme palette.
pub struct PluginThemePaletteResource {
    pub id: String,
    pub plugin_id: String,
    pub display_name: String,
    pub colors: PluginThemeColorsResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Runtime-wide automation overview.
pub struct RuntimeAutomationResource {
    pub enabled: bool,
    pub job_count: usize,
    pub recent_jobs: Vec<ScheduledJobResource>,
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
/// Status of a runtime background task.
pub enum RuntimeBackgroundTaskStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// One published command.
pub struct RuntimeCommandResource {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slash: Option<String>,
    pub description: String,
    pub aliases: Vec<String>,
    pub group: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    pub plugin_id: String,
    /// `client`, `method` or `tool` — how running the command reaches a handler.
    pub target: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// The published command catalog, exactly as every client renders it.
pub struct RuntimeCommandsResource {
    /// Commands a client runs locally through its own action vocabulary.
    pub client_count: usize,
    /// Commands served by the plugin that declared them.
    pub served_count: usize,
    pub commands: Vec<RuntimeCommandResource>,
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
/// Runtime operator surface: MCP, LSP, agent, commands, and plugins.
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agent_id: String,
    pub commands: RuntimeCommandsResource,
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
/// A recurring runtime background task and whether it is enabled.
pub struct RuntimeTaskResource {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// Adapter model probe result for one adapter.
pub struct ProviderAdapterModelsResource {
    pub adapter_id: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base_url: Option<String>,
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
