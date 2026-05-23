//! REST resource projections for the unified API surface, reorganised by
//! domain so external clients can `use agena_api::resource::*`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    agent::{AgentMode, AgentPermissionConfig, AgentRunConfig, PermissionConfig},
    agents::AgentScope,
    config::ProviderProtocolPathsConfig,
    event::MessagePartUpdatedEvent,
    message::{Message, MessageMetadata, MessagePart, MessageStatus, MessageUsage},
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
    #[serde(default)]
    pub ui: agena::plugin::PluginUiCatalog,
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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuntimeAgentDefaultResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl RuntimeAgentDefaultResource {
    pub fn is_empty(&self) -> bool {
        self.provider.is_none() && self.adapter.is_none() && self.model.is_none()
    }
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
    #[serde(
        default,
        rename = "default",
        skip_serializing_if = "RuntimeAgentDefaultResource::is_empty"
    )]
    pub default: RuntimeAgentDefaultResource,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pending_interactive_requests: Vec<PendingInteractiveRequest>,
    pub pending_permission_requests: Vec<PermissionRequest>,
    pub pending_user_input_requests: Vec<UserInputRequest>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub goal: Option<SessionGoalResource>,
    pub usage: SessionUsageResource,
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

impl From<agena::role::Role> for MessageRole {
    fn from(value: agena::role::Role) -> Self {
        match value {
            agena::role::Role::User => Self::User,
            agena::role::Role::Assistant => Self::Assistant,
            agena::role::Role::System => Self::System,
        }
    }
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

impl MessageResource {
    pub fn from_message(
        session_id: i64,
        message: &Message,
        updated_at: DateTime<Utc>,
        part_count: u64,
        parts: Option<Vec<MessagePart>>,
    ) -> Self {
        Self {
            id: message.id,
            session_id,
            role: message.role.into(),
            state: message.state,
            created_at: message.created_at,
            updated_at,
            metadata: message.metadata.clone(),
            usage: message.usage.clone(),
            finish: message.finish.clone(),
            part_count,
            parts,
        }
    }

    pub fn from_part_update(update: &MessagePartUpdatedEvent) -> Self {
        Self {
            id: update.message_id,
            session_id: update.session_id,
            role: update.message_role.into(),
            state: update.message_state,
            created_at: update.message_created_at,
            updated_at: timestamp_ms_or(update.ts_ms, update.message_created_at),
            metadata: MessageMetadata::default(),
            usage: None,
            finish: None,
            part_count: 1,
            parts: Some(vec![update.part.clone()]),
        }
    }

    pub fn from_completed_assistant_parts(
        session_id: i64,
        message_id: i64,
        created_at: DateTime<Utc>,
        metadata: MessageMetadata,
        usage: Option<MessageUsage>,
        finish: Option<String>,
        parts: Vec<MessagePart>,
    ) -> Self {
        Self {
            id: message_id,
            session_id,
            role: MessageRole::Assistant,
            state: MessageStatus::Completed,
            created_at,
            updated_at: created_at,
            metadata,
            usage,
            finish,
            part_count: parts.len() as u64,
            parts: Some(parts),
        }
    }
}

fn timestamp_ms_or(timestamp_ms: i64, fallback: DateTime<Utc>) -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp_millis(timestamp_ms).unwrap_or(fallback)
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
pub use agena::message::{PendingInteractiveRequest, UserInputReply, UserInputRequest};
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

#[cfg(test)]
mod tests {
    use super::*;
    use agena::{
        event::MessagePartUpdatedEvent,
        message::{ExecutionStatus, PartContent},
    };
    use chrono::TimeZone;

    #[test]
    fn message_resource_from_message_preserves_core_fields() {
        let created_at = Utc
            .timestamp_millis_opt(1_700_000_000_000)
            .single()
            .unwrap();
        let updated_at = Utc
            .timestamp_millis_opt(1_700_000_005_000)
            .single()
            .unwrap();
        let mut message = Message::prompt_parts(Role::Assistant, vec![PartContent::text("hello")]);
        message.id = 42;
        message.created_at = created_at;
        message.finish = Some("stop".to_string());
        for (index, part) in message.parts.iter_mut().enumerate() {
            part.id = 100 + index as i64;
            part.message_id = message.id;
            part.part_index = index as i32;
            part.created_at = created_at;
        }

        let resource = MessageResource::from_message(
            7,
            &message,
            updated_at,
            message.parts.len() as u64,
            Some(message.parts.clone()),
        );

        assert_eq!(resource.id, 42);
        assert_eq!(resource.session_id, 7);
        assert_eq!(resource.role, MessageRole::Assistant);
        assert_eq!(resource.state, MessageStatus::Completed);
        assert_eq!(resource.created_at, created_at);
        assert_eq!(resource.updated_at, updated_at);
        assert_eq!(resource.finish.as_deref(), Some("stop"));
        assert_eq!(resource.part_count, 1);
        assert_eq!(resource.parts.as_ref().map(Vec::len), Some(1));
    }

    #[test]
    fn message_resource_from_part_update_creates_incremental_shell() {
        let created_at = Utc
            .timestamp_millis_opt(1_700_000_010_000)
            .single()
            .unwrap();
        let part = agena::message::MessagePart::with_content(
            5,
            55,
            created_at,
            ExecutionStatus::InProgress,
            PartContent::text("chunk"),
        );
        let update = MessagePartUpdatedEvent {
            session_id: 9,
            message_id: 55,
            message_role: Role::Assistant,
            message_state: MessageStatus::InProgress,
            message_created_at: created_at,
            part: part.clone(),
            ts_ms: created_at.timestamp_millis() + 250,
        };

        let resource = MessageResource::from_part_update(&update);

        assert_eq!(resource.id, 55);
        assert_eq!(resource.session_id, 9);
        assert_eq!(resource.role, MessageRole::Assistant);
        assert_eq!(resource.state, MessageStatus::InProgress);
        assert_eq!(resource.created_at, created_at);
        assert_eq!(
            resource.updated_at.timestamp_millis(),
            created_at.timestamp_millis() + 250
        );
        assert_eq!(resource.metadata, MessageMetadata::default());
        assert_eq!(resource.part_count, 1);
        assert_eq!(resource.parts, Some(vec![part]));
    }

    #[test]
    fn message_resource_from_completed_assistant_parts_marks_completion() {
        let created_at = Utc
            .timestamp_millis_opt(1_700_000_020_000)
            .single()
            .unwrap();
        let part = agena::message::MessagePart::with_content(
            8,
            88,
            created_at,
            ExecutionStatus::Completed,
            PartContent::text("done"),
        );

        let resource = MessageResource::from_completed_assistant_parts(
            12,
            88,
            created_at,
            MessageMetadata::default(),
            None,
            Some("stop".to_string()),
            vec![part.clone()],
        );

        assert_eq!(resource.id, 88);
        assert_eq!(resource.session_id, 12);
        assert_eq!(resource.role, MessageRole::Assistant);
        assert_eq!(resource.state, MessageStatus::Completed);
        assert_eq!(resource.finish.as_deref(), Some("stop"));
        assert_eq!(resource.part_count, 1);
        assert_eq!(resource.parts, Some(vec![part]));
    }
}
