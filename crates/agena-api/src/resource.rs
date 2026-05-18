//! REST resource projections for the unified API surface, reorganised by
//! domain so external clients can `use agena_api::resource::*`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    agent::{AgentMode, AgentPermissionConfig, AgentRunConfig, PermissionConfig},
    agents::AgentScope,
    config::ProviderProtocolPathsConfig,
    message::{MessageMetadata, MessagePart, MessageStatus, MessageUsage},
    model::ModelRef,
    model_catalog::{CatalogModelDefinition, ModelCatalogEntrySourceKind},
    session::{GoalStatus, SessionStatus, SessionSummary},
};

// ─── Health / runtime ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
    pub database_connected: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeTaskResource {
    pub enabled: bool,
    pub interval_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduledJobRunResource {
    pub triggered_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
    pub status: agena_scheduler::JobRunStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct SessionAutomationResource {
    pub job_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_job: Option<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAutomationResource {
    pub enabled: bool,
    pub job_count: usize,
    pub recent_jobs: Vec<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
    pub agents: RuntimeAgentsResource,
    pub skills: RuntimeSkillsResource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMcpResource {
    pub server_count: usize,
    pub tool_count: usize,
    pub servers: Vec<RuntimeMcpServerResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeMcpServerResource {
    pub name: String,
    pub tool_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLspResource {
    pub server_count: usize,
    pub diagnostics_count: usize,
    pub files_with_diagnostics: usize,
    pub servers: Vec<RuntimeLspServerResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeLspServerResource {
    pub name: String,
    pub command: String,
    pub file_extensions: Vec<String>,
    pub root_markers: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSkillsResource {
    pub skill_count: usize,
    pub command_count: usize,
    pub skills: Vec<RuntimeSkillResource>,
    pub commands: Vec<RuntimeSkillResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeSkillResource {
    pub name: String,
    pub description: String,
    pub aliases: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeAgentsResource {
    pub default_agent: String,
    pub total_count: usize,
    pub primary_count: usize,
    pub subagent_count: usize,
    pub hidden_count: usize,
    pub agents: Vec<RuntimeAgentResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_successful_source: Option<ModelCatalogEntrySourceKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub entry_count: usize,
    pub official_entry_count: usize,
    pub custom_entry_count: usize,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    pub default_thinking_mode: Option<String>,
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
    pub pricing: Option<agena::model::ModelPricing>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub thinking_modes:
        std::collections::BTreeMap<String, agena::provider::ConfiguredModelThinkingMode>,
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub speed_modes: std::collections::BTreeMap<String, agena::provider::ConfiguredModelSpeedMode>,
    #[serde(flatten)]
    pub capabilities: agena::provider::ModelCapabilityPatch,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalogEntryWriteRequest {
    pub model_id: String,
    #[serde(flatten)]
    pub definition: CatalogModelDefinition,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeReloadResponse {
    pub cause: String,
    pub previous_generation: u64,
    pub generation: u64,
    pub loaded_at: DateTime<Utc>,
}

// ─── Workspaces ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionResource {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub title: String,
    pub version: i64,
    #[serde(default)]
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_verbosity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_parallel_tool_calls: Option<bool>,
    #[serde(default, skip_serializing_if = "AgentRunConfig::is_empty")]
    pub agent_run: AgentRunConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effective_workspace_root: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
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

// ─── Messages ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum PartLoadMode {
    None,
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ─── Permission rules ────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
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

// ─── Auth providers ──────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialType {
    Api,
    Oauth,
    WellKnown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAdapterSummaryResource {
    pub adapter_id: String,
    pub enabled: bool,
    pub configured_model_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderSummaryResource {
    pub provider_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_adapter: Option<String>,
    pub default_model: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub adapters: Vec<ProviderAdapterSummaryResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderAdapterModelsRequest {
    #[serde(default)]
    pub provider_id: Option<String>,
    pub base_url: String,
    #[serde(default)]
    pub protocol_paths: ProviderProtocolPathsConfig,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SavedProviderAdapterModelsRequest {
    #[serde(default)]
    pub adapter_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAdapterModelsResource {
    pub adapter_id: String,
    pub enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub resolved_base_url: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub models: Vec<ProviderModel>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl From<agena::config::ProviderAdapterModelsResult> for ProviderAdapterModelsResource {
    fn from(value: agena::config::ProviderAdapterModelsResult) -> Self {
        Self {
            adapter_id: value.adapter_id,
            enabled: value.enabled,
            resolved_base_url: value.resolved_base_url,
            models: value.models,
            error: value.error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAdapterModelsResponse {
    pub provider_id: String,
    pub adapters: Vec<ProviderAdapterModelsResource>,
}

// Re-export the common payload types so clients don't need an explicit
// `agena = …` dep just to construct them.
pub use agena::message::PartContent as MessagePartContent;
pub use agena::message::{UserInputReply, UserInputRequest};
pub use agena::permission::{PermissionMode, PermissionReply, PermissionRequest};
pub use agena::provider::ProviderModel;
pub use agena::role::Role;

/// Wire form of a rewind audit checkpoint exposed via the Command protocol.
/// Mirrors `agena::session::RewindCheckpoint` with a stable serde shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindCheckpointResource {
    pub schema: u32,
    pub at_ms: i64,
    pub target_message_id: i64,
    pub dropped: Vec<RewindCheckpointEntryResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RewindCheckpointEntryResource {
    pub message_id: i64,
    pub role: String,
    pub preview: String,
}

impl From<agena::session::RewindCheckpoint> for RewindCheckpointResource {
    fn from(value: agena::session::RewindCheckpoint) -> Self {
        Self {
            schema: value.schema,
            at_ms: value.at_ms,
            target_message_id: value.target_message_id,
            dropped: value.dropped.into_iter().map(Into::into).collect(),
        }
    }
}

impl From<agena::session::RewindCheckpointEntry> for RewindCheckpointEntryResource {
    fn from(value: agena::session::RewindCheckpointEntry) -> Self {
        Self {
            message_id: value.message_id,
            role: value.role,
            preview: value.preview,
        }
    }
}
