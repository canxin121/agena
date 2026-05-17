use agena_api::resource::MessageRole;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    agent::{AgentMode, AgentPermissionConfig, AgentRunConfig, PermissionConfig},
    agents::AgentScope,
    config::SharedGatewayEndpointLayout,
    message::{
        MessageMetadata, MessagePart, MessageStatus, MessageUsage, PartContent, UserInputReply,
        UserInputRequest,
    },
    model::ModelRef,
    model_catalog::{CatalogModelDefinition, ModelCatalogEntryRecord, ModelCatalogEntrySourceKind},
    permission::PermissionMode,
    permission::{PermissionReply, PermissionRequest},
    provider::ProviderModel,
    session::{GoalStatus, SessionStatus, SessionSummary},
};

#[cfg(test)]
use agena::message::ExecutionStatus;
#[cfg(test)]
use agena::role::Role;

#[derive(Debug, Clone, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub database_connected: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeTaskResource {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeSessionCacheResource {
    pub max_sessions: usize,
    pub ttl_secs: u64,
    pub max_bytes: usize,
    pub entry_count: usize,
    pub total_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub inserts: u64,
    pub evictions: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct ScheduledJobRunResource {
    pub triggered_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: agena_scheduler::JobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
pub struct SessionAutomationResource {
    pub job_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_job: Option<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAutomationResource {
    pub enabled: bool,
    pub job_count: usize,
    pub recent_jobs: Vec<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize)]
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
    pub janitor: RuntimeTaskResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_cache: Option<RuntimeSessionCacheResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_catalog: Option<ModelCatalogResponse>,
    pub automation: RuntimeAutomationResource,
    pub operator: RuntimeOperatorResource,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogResponse {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogEntrySourceKind>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub entry_count: usize,
    pub official_entry_count: usize,
    pub custom_entry_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogListResponse {
    pub summary: ModelCatalogResponse,
    pub total: usize,
    pub offset: usize,
    pub limit: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub available_origins: Vec<String>,
    pub items: Vec<ModelCatalogEntryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogLookupResponse {
    pub items: Vec<ModelCatalogEntryResource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogEntryKind {
    Official,
    Custom,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelCatalogSourceKind {
    Generated,
    Cache,
    Custom,
}

#[derive(Debug, Clone, Serialize)]
pub struct ModelCatalogEntryResource {
    pub model_id: String,
    pub kind: ModelCatalogEntryKind,
    pub source: ModelCatalogSourceKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_label: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub has_local_override: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lifecycle: Option<agena::model::ModelLifecycle>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_window_tokens: Option<u32>,
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
    pub default_thinking_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_parallel_tool_calls: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub supports_verbosity: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_verbosity: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub output_modalities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing: Option<agena::model::ModelPricing>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thinking_modes:
        std::collections::BTreeMap<String, agena::provider::ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub speed_modes: std::collections::BTreeMap<String, agena::provider::ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: agena::provider::ModelCapabilityPatch,
}

impl From<ModelCatalogEntryRecord> for ModelCatalogEntryResource {
    fn from(value: ModelCatalogEntryRecord) -> Self {
        Self::from_record(value, None)
    }
}

impl ModelCatalogEntryResource {
    pub fn from_record(
        value: ModelCatalogEntryRecord,
        last_successful_source: Option<ModelCatalogEntrySourceKind>,
    ) -> Self {
        let kind = if value.has_local_override {
            ModelCatalogEntryKind::Custom
        } else {
            ModelCatalogEntryKind::Official
        };
        let source = if value.has_local_override {
            ModelCatalogSourceKind::Custom
        } else {
            match last_successful_source.unwrap_or(ModelCatalogEntrySourceKind::Generated) {
                ModelCatalogEntrySourceKind::Generated => ModelCatalogSourceKind::Generated,
                ModelCatalogEntrySourceKind::Cache => ModelCatalogSourceKind::Cache,
            }
        };
        let source_label = Some(match source {
            ModelCatalogSourceKind::Generated => "generated catalog",
            ModelCatalogSourceKind::Cache => "cached catalog",
            ModelCatalogSourceKind::Custom => "workspace override",
        })
        .map(str::to_owned);

        Self {
            model_id: value.model_id,
            kind,
            source,
            source_label,
            has_local_override: value.has_local_override,
            display_name: value.display_name,
            origin: value.origin,
            lifecycle: value.lifecycle,
            context_window_tokens: value.context_window_tokens,
            max_output_tokens: value.max_output_tokens,
            description: value.description,
            knowledge_cutoff: value.knowledge_cutoff,
            release_date: value.release_date,
            last_updated: value.last_updated,
            open_weights: value.open_weights,
            default_thinking_mode: value.default_thinking_mode,
            supports_parallel_tool_calls: value.supports_parallel_tool_calls,
            supports_verbosity: value.supports_verbosity,
            default_verbosity: value.default_verbosity,
            output_modalities: value.output_modalities,
            pricing: value.pricing,
            thinking_modes: value.thinking_modes,
            speed_modes: value.speed_modes,
            capabilities: value.capabilities,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelCatalogEntryWriteRequest {
    pub model_id: String,
    #[serde(flatten)]
    pub definition: CatalogModelDefinition,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModelCatalogLookupRequest {
    #[serde(default)]
    pub model_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agents: RuntimeAgentsResource,
    pub skills: RuntimeSkillsResource,
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
    pub primary_count: usize,
    pub subagent_count: usize,
    pub hidden_count: usize,
    pub agents: Vec<RuntimeAgentResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeAgentResource {
    pub name: String,
    pub description: String,
    pub mode: AgentMode,
    pub hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub steps: Option<usize>,
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "AgentPermissionConfig::is_empty")]
    pub permission: AgentPermissionConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub aliases: Vec<String>,
    pub scope: AgentScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeReloadResponse {
    pub cause: &'static str,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginStatusListResponse {
    pub entries: Vec<agena::plugin::status::PluginStatus>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginInspectResponse {
    pub plugin: agena::plugin::PluginInspect,
}

#[derive(Debug, Clone, Serialize)]
pub struct PluginLogListResponse {
    pub plugin_id: String,
    pub entries: Vec<agena::plugin::PluginLogEntry>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PluginLogListQuery {
    #[serde(default)]
    pub after_seq: Option<u64>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplacePluginResource {
    pub plugin_id: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub version_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_platform: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceSearchResponse {
    pub registry_id: String,
    pub registry_url: String,
    pub entries: Vec<MarketplacePluginResource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceRegistryRequestBody {
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceSearchRequestBody {
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub refresh: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceSyncResponse {
    pub registry_id: String,
    pub registry_url: String,
    pub plugin_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstalledPluginResource {
    pub plugin_id: String,
    pub version: String,
    pub kind: String,
    pub platform: String,
    pub binary_path: String,
    pub config_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub installed_at: DateTime<Utc>,
    pub registry_id: String,
    pub registry_url: String,
    pub archive_extracted: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstalledListResponse {
    pub entries: Vec<MarketplaceInstalledPluginResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceOutdatedPluginResource {
    pub plugin_id: String,
    pub installed_version: String,
    pub latest_version: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceOutdatedListResponse {
    pub entries: Vec<MarketplaceOutdatedPluginResource>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceInstallRequestBody {
    pub spec: String,
    #[serde(default)]
    pub registry_id: Option<String>,
    pub registry_url: String,
    #[serde(default)]
    pub config_path: Option<String>,
    #[serde(default)]
    pub force: bool,
    #[serde(default)]
    pub dry_run: bool,
    #[serde(default)]
    pub allow_unverified: bool,
    #[serde(default)]
    pub refresh: bool,
    #[serde(default)]
    pub require_signature: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceInstallOutcomeResource {
    pub plugin_id: String,
    pub version: String,
    pub kind: String,
    pub artifact_path: String,
    pub config_path: String,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct MarketplaceUninstallRequestBody {
    pub plugin_id: String,
    #[serde(default)]
    pub cascade: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUninstallOutcomeResource {
    pub plugin_id: String,
    pub version: String,
    pub config_path: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUninstallResponse {
    pub entries: Vec<MarketplaceUninstallOutcomeResource>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MarketplaceUpgradeRequestBody {
    #[serde(default)]
    pub plugin_id: Option<String>,
    #[serde(default)]
    pub all: bool,
    #[serde(default)]
    pub registry_id: Option<String>,
    #[serde(default)]
    pub registry_url: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUpgradeOutcomeResource {
    pub plugin_id: String,
    pub previous_version: String,
    pub installed_version: String,
    pub upgraded: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<MarketplaceInstallOutcomeResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MarketplaceUpgradeResponse {
    pub entries: Vec<MarketplaceUpgradeOutcomeResource>,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthProviderResource {
    pub provider_id: String,
    pub configured: bool,
    pub credential_present: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credential_type: Option<AuthCredentialType>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub key_preview: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expired: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthApiKeyWriteRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiBrowserFinishRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserFinishRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthAtomGitBrowserStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthAtomGitBrowserPollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub state: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthOpenAiDeviceStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiDevicePollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthCopilotDeviceStartRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthCopilotDevicePollRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub device_code: String,
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthBrowserStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub instance_url: Option<String>,
    pub authorize_url: String,
    pub state: String,
    pub pkce_verifier: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthDeviceStartResource {
    pub provider_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub enterprise_domain: Option<String>,
    pub verification_url: String,
    pub user_code: String,
    pub device_code: String,
    pub interval_seconds: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthLoginResultResource {
    pub completed: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<AuthProviderResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderSummaryResource {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_adapter: Option<String>,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct ProviderAdapterDiscoveryRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub endpoint_layout: SharedGatewayEndpointLayout,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SavedProviderAdapterDiscoveryRequest {
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdapterDiscoveryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub supported: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ProviderModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderAdapterDiscoveryResponse {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterDiscoveryResource>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceResource {
    pub id: i64,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_count: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
    #[serde(default)]
    pub include_session_count: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceWriteRequest {
    pub path: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct WorkspaceResolveRequest {
    pub path: String,
    #[serde(default)]
    pub create_if_missing: bool,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct WorkspaceFileTreeQuery {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub depth: Option<usize>,
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceFileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceFileNode {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceFileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<WorkspaceFileNode>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct WorkspaceFileTreeResource {
    pub workspace_id: i64,
    pub root: String,
    pub path: String,
    pub entries: Vec<WorkspaceFileNode>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionGoalResource {
    pub id: i64,
    pub session_id: i64,
    pub objective: String,
    pub status: GoalStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_budget: Option<u64>,
    pub tokens_used: u64,
    pub time_used_seconds: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionGoalSetRequest {
    #[serde(default)]
    pub objective: Option<String>,
    #[serde(default)]
    pub status: Option<GoalStatus>,
    #[serde(default)]
    pub token_budget: Option<Option<u64>>,
    #[serde(default)]
    pub clear: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionResource {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    pub is_subagent: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoalResource>,
}

impl From<SessionSummary> for SessionResource {
    fn from(value: SessionSummary) -> Self {
        Self {
            id: value.id,
            parent_id: value.parent_id,
            depth: value.depth,
            root_id: value.root_id,
            workspace_id: value.workspace_id,
            title: value.title,
            version: value.version,
            is_subagent: value.is_subagent,
            created_at: value.created_at,
            updated_at: value.updated_at,
            message_count: value.message_count,
            child_session_count: value.child_session_count,
            last_message_at: value.last_message_at,
            goal: value.goal.map(|goal| SessionGoalResource {
                id: goal.id,
                session_id: goal.session_id,
                objective: goal.objective,
                status: goal.status,
                token_budget: goal.token_budget,
                tokens_used: goal.tokens_used,
                time_used_seconds: goal.time_used_seconds,
                created_at: goal.created_at,
                updated_at: goal.updated_at,
                completed_at: goal.completed_at,
            }),
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub workspace_id: Option<i64>,
    #[serde(default)]
    pub parent_id: Option<i64>,
    #[serde(default)]
    pub roots: bool,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionCreateRequest {
    pub workspace_id: i64,
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionReplaceRequest {
    pub title: String,
    #[serde(default)]
    pub parent_id: Option<i64>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionRunState {
    Idle,
    AwaitingModel,
}

impl From<SessionStatus> for SessionRunState {
    fn from(value: SessionStatus) -> Self {
        match value {
            SessionStatus::Idle => Self::Idle,
            SessionStatus::AwaitingModel => Self::AwaitingModel,
        }
    }
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionRunOptionsRequest {
    #[serde(default)]
    pub model: Option<ModelRef>,
    #[serde(default)]
    pub thinking_mode: Option<String>,
    #[serde(default)]
    pub speed_mode: Option<String>,
    #[serde(default)]
    pub agent_profile: Option<String>,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
    #[serde(default)]
    pub max_turn_loops: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionTurnRequest {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    #[serde(default)]
    pub parts: Vec<PartContent>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionContinueRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionPermissionReplyRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    pub reply: PermissionReply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionUserInputReplyRequestBody {
    #[serde(flatten)]
    pub options: SessionRunOptionsRequest,
    pub reply: UserInputReply,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionRewindRequestBody {
    pub message_id: i64,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionExecutionContextResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_profile: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_mode: Option<AgentMode>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub agent_hidden: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_color: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_override: Option<String>,
    pub allowed_tools: Vec<String>,
    #[serde(default, skip_serializing_if = "PermissionConfig::is_empty")]
    pub agent_permission: PermissionConfig,
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
    #[serde(default, skip_serializing_if = "AgentRunConfig::is_empty")]
    pub agent_run: AgentRunConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionExecutionResource {
    pub session: SessionResource,
    pub blocked: bool,
    pub run_state: SessionRunState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation: Option<SessionAutomationResource>,
    pub execution: SessionExecutionContextResource,
    pub pending_permission_requests: Vec<PermissionRequest>,
    pub pending_user_input_requests: Vec<UserInputRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoalResource>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartLoadMode {
    None,
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Serialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finish: Option<String>,
    pub part_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parts: Option<Vec<MessagePart>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct MessageListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub parts: PartLoadMode,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct MessagePartWriteRequest {
    pub content: PartContent,
    #[serde(default)]
    pub status: Option<ExecutionStatus>,
    #[serde(default)]
    pub operation_id: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
#[allow(dead_code)]
#[derive(Debug, Clone, Deserialize)]
pub struct MessageWriteRequest {
    pub role: Role,
    #[serde(default)]
    pub state: Option<MessageStatus>,
    #[serde(default)]
    pub metadata: Option<MessageMetadata>,
    #[serde(default)]
    pub usage: Option<MessageUsage>,
    #[serde(default)]
    pub finish: Option<String>,
    #[serde(default)]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub parts: Vec<MessagePartWriteRequest>,
}

#[derive(Debug, Clone, Serialize)]
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

#[derive(Debug, Clone, Serialize)]
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
    pub worktree_active_sessions: u64,
    pub worktree_managed_dirs: u64,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PermissionRuleListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub search: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PermissionRuleWriteRequest {
    #[serde(default)]
    pub action_key: Option<String>,
    #[serde(default)]
    pub subject_kind: Option<String>,
    #[serde(default)]
    pub tool_name: Option<String>,
    #[serde(default)]
    pub qualifier: Option<String>,
    #[serde(default)]
    pub path_access_kind: Option<String>,
    #[serde(default)]
    pub workspace_root: Option<String>,
    #[serde(default)]
    pub target_path: Option<String>,
    #[serde(default)]
    pub network_target: Option<String>,
    #[serde(default)]
    pub network_host: Option<String>,
    #[serde(default)]
    pub network_port: Option<u16>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub session_id: Option<i64>,
    pub mode: PermissionMode,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PermissionRuleRevokeRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionEventListQuery {
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct SessionEventStreamQuery {
    #[serde(default)]
    pub after_seq: Option<i64>,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub poll_interval_ms: Option<u64>,
    #[serde(default)]
    pub idle_timeout_ms: Option<u64>,
}
