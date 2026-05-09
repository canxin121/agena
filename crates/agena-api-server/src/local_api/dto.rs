use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use agena::{
    config::ConfigSource,
    message::{
        MessageMetadata, MessagePart, MessageStatus, MessageUsage, PartContent, UserInputReply,
        UserInputRequest,
    },
    model::ModelRef,
    permission::PermissionMode,
    permission::{PermissionReply, PermissionRequest},
    provider::ProviderModel,
    role::Role,
    session::{SessionStatus, SessionSummary},
};

#[cfg(test)]
use agena::message::ExecutionStatus;

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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_mode_source: Option<ConfigSource>,
    pub auth_store_path: String,
    pub provider_ids: Vec<String>,
    pub plugin_count: usize,
    pub session_runtime_available: bool,
    pub watch_paths: Vec<String>,
    pub reload: RuntimeTaskResource,
    pub janitor: RuntimeTaskResource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_cache: Option<RuntimeSessionCacheResource>,
    pub automation: RuntimeAutomationResource,
    pub operator: RuntimeOperatorResource,
}

#[derive(Debug, Clone, Serialize)]
pub struct RuntimeOperatorResource {
    pub mcp: RuntimeMcpResource,
    pub lsp: RuntimeLspResource,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthApiKeyWriteRequest {
    pub api_key: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthBrowserStartRequest {
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiBrowserFinishRequest {
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserStartRequest {
    pub instance_url: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthGitLabBrowserFinishRequest {
    pub instance_url: String,
    pub code: String,
    pub pkce_verifier: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthOpenAiDevicePollRequest {
    pub device_code: String,
    pub user_code: String,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct AuthCopilotDeviceStartRequest {
    #[serde(default)]
    pub enterprise_domain: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AuthCopilotDevicePollRequest {
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
pub struct ProviderSummaryResource {
    pub provider_id: String,
    pub default_model: String,
    pub default_model_ref: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderModelsResponse {
    pub provider_id: String,
    pub models: Vec<ProviderModel>,
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
    pub system: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
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
    pub active_skill_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system_prompt_override: Option<String>,
    pub allowed_tools: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
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
    pub role: Role,
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
