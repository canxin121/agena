//! Session-facing resource values shared by the HTTP surface and plugins.

use super::interaction::PendingInteractiveRequest;
use agena_domain::{
    AgentStatus, ExecutionPhase, SessionLifecycleState, SessionRelationKind, SessionStateKind,
    WorkflowState, session_kernel::ExecutionAttemptSettlement,
};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A workspace resource.
pub struct WorkspaceResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_stats: Option<WorkspaceSessionStats>,
    pub id: i64,
    pub path: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
/// A session resource.
pub struct SessionResource {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: Option<i64>,
    pub title: String,
    /// Bot this conversation belongs to, when any. Clients use it to place the
    /// conversation under the Bot instead of a project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_bot_id: Option<String>,
    /// Entrance shape of a Bot conversation: `local`, `direct`, `group`, `broadcast`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_surface: Option<String>,
    /// Durable user favorite state shared by TUI/Web/API clients.
    #[serde(default)]
    pub favorite: bool,
    /// Durable navigation pin state shared by TUI/Web/API clients.
    #[serde(default)]
    pub pinned: bool,
    pub version: i64,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    /// Authoritative processing state derived from persisted run markers and
    /// pending interactions. Unlike a client's request/loading flag, this
    /// survives disconnects and can therefore be used by every client to
    /// identify work still owned by the server.
    #[serde(default)]
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_cutoff_seq_global: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_message_id: Option<i64>,
    #[serde(default)]
    pub is_subagent: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentStatus>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub message_count: u64,
    pub child_session_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_message_at: Option<DateTime<Utc>>,
}

impl Default for SessionResource {
    /// The empty shape a caller fills in; never a persisted session.
    fn default() -> Self {
        Self {
            id: 0,
            parent_id: None,
            depth: 0,
            root_id: 0,
            workspace_id: None,
            title: String::new(),
            owner_bot_id: None,
            owner_surface: None,
            favorite: false,
            pinned: false,
            version: 0,
            relation_kind: agena_domain::SessionRelationKind::Root,
            lifecycle_state: agena_domain::SessionLifecycleState::Ready,
            state: SessionState::default(),
            source_cutoff_seq_global: None,
            source_message_id: None,
            is_subagent: false,
            agent_name: None,
            agent_status: None,
            created_at: DateTime::<Utc>::UNIX_EPOCH,
            updated_at: DateTime::<Utc>::UNIX_EPOCH,
            message_count: 0,
            child_session_count: 0,
            last_message_at: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
/// Current processing state of a session.
///
/// This is the single client-facing execution state. Durable session facts are
/// derived from durable parts; the optional live execution and workflow
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
        #[schemars(with = "Vec<serde_json::Value>")]
        requests: Vec<PendingInteractiveRequestResource>,
    },
    AwaitingInteraction {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        run_id: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        execution: Option<ActiveExecutionResource>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(with = "Vec<serde_json::Value>")]
        requests: Vec<PendingInteractiveRequestResource>,
    },
    Failed {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        failure: Option<serde_json::Value>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Full execution view of a session.
pub struct SessionExecutionResource {
    /// Identity of this accepted command, independent of the returned Part window.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<SessionExecutionReceiptResource>,
    /// Server-owned FIFO of accepted input that has not started. These are
    /// references to durable user messages, not client-local composer drafts.
    #[serde(default)]
    pub queued_messages: Vec<QueuedMessageResource>,
    /// Paused inputs remain durable and require an explicit backend resume.
    #[serde(default)]
    pub queue_paused: bool,
    pub session: SessionResource,
    /// The session's parts (ordered parts, including `run` markers).
    pub parts: Vec<super::part::PartResource>,
    /// `None` means no transcript page was requested. Status shells may still
    /// include the durable owners of pending interactions. A bounded history
    /// selection always carries metadata, including when its window is empty.
    pub part_page: Option<super::pagination::PageInfo>,
    /// Durable session control version for ordering execution snapshots and
    /// mutation receipts. The legacy field name is retained for wire
    /// compatibility; this is neither a metadata version nor an SSE cursor.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_event_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub automation: Option<SessionAutomationResource>,
    /// Non-terminal background members owned by this session. This is the
    /// single session-scoped projection consumed by both composer footers and
    /// Web; clients must not maintain a second counter.
    #[serde(default)]
    pub background_activities: Vec<super::activity::BackgroundActivityResource>,
    pub execution: SessionExecutionContextResource,
    pub usage: SessionUsageResource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueuedMessageResource {
    pub execution_id: uuid::Uuid,
    pub message_id: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct QueuedMessageEditResource {
    pub execution_id: uuid::Uuid,
    pub message_id: i64,
    pub revision: i64,
    pub document: agena_domain::ComposerDocument,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionExecutionReceiptResource {
    pub execution_id: uuid::Uuid,
    /// Committed user/control input identity, independent of transcript paging.
    pub message_id: i64,
    /// Allocated when accepted, including while the attempt is still queued.
    pub attempt_generation: i64,
    pub turn_id: uuid::Uuid,
    pub reply_id: uuid::Uuid,
}

/// Exact-attempt observation. A terminal phase alone does not mean that
/// post-run work has retired. Recovery/supersession must not be treated as
/// successful settlement, even when the durable phase is succeeded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionAttemptResource {
    pub session_id: i64,
    pub execution_id: Uuid,
    pub message_id: i64,
    pub attempt_generation: i64,
    pub current_attempt_generation: i64,
    pub phase: agena_domain::session_kernel::ExecutionPhase,
    pub settlement: ExecutionAttemptSettlement,
    pub kernel_epoch: Uuid,
    pub owner_epoch: Option<Uuid>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// A pending interactive request (permission or user input).
pub struct PendingInteractiveRequestResource {
    pub session_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_name: Option<String>,
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
    /// Temporary additional instructions; an empty string overrides inherited values.
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
    #[serde(default)]
    pub max_output_tokens: Option<u32>,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Role of a message.
pub enum RunRole {
    User,
    Assistant,
    System,
    Tool,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
/// How message parts are loaded in a listing.
pub enum PartLoadMode {
    None,
    #[default]
    Summary,
    Full,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
/// Reference to a command attached to a message.
pub struct PartCommandReference {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    pub content_hash: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skill_id: Option<String>,
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
    Url {
        url: String,
    },
    DataUrl {
        url: String,
    },
    /// Bytes bound to the exact provider route they were authorized for.
    ProviderData {
        route: String,
        data: String,
    },
    Base64 {
        data: String,
    },
    FileId {
        file_id: String,
    },
    LocalPath {
        path: String,
    },
    /// The bytes are gone: an upload from a route that is no longer selected,
    /// an expired provider file id, or a blob that no longer belongs to this
    /// conversation. The attachment keeps its identity and display metadata so
    /// the transcript still reads correctly; clients render the notice instead
    /// of media.
    Unavailable {
        #[serde(default, skip_serializing_if = "String::is_empty")]
        reason: String,
    },
}

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
        skip_serializing_if = "super::interaction::user_input_answers_is_empty"
    )]
    pub answers: std::collections::BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Kind of a workspace file entry.
pub enum WorkspaceFileKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A node in a workspace file tree.
pub struct WorkspaceFileNode {
    pub name: String,
    pub path: String,
    pub kind: WorkspaceFileKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size: Option<u64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<WorkspaceFileNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// A workspace file tree.
pub struct WorkspaceFileTreeResource {
    pub workspace_id: i64,
    pub root: String,
    pub path: String,
    pub entries: Vec<WorkspaceFileNode>,
}
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
/// An active execution inside a session.
pub struct ActiveExecutionResource {
    #[schemars(with = "String")]
    pub execution_id: Uuid,
    pub phase: ExecutionPhase,
}

/// Persistence scope selected for a permission decision in the public API.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PermissionScope {
    Session,
    Workspace,
    Global,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Automation (scheduled jobs) attached to a session.
pub struct SessionAutomationResource {
    pub job_count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_job: Option<ScheduledJobResource>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Execution context of a session: agent, access, and permission configuration.
pub struct SessionExecutionContextResource {
    /// The session prompt layer: inline fragments that apply to this session.
    #[serde(
        default,
        skip_serializing_if = "agena_prompt_layers::PromptLayer::is_empty"
    )]
    pub prompts: agena_prompt_layers::PromptLayer,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_retry: Option<agena_domain::ProviderRetryStatus>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conversation: Option<agena_domain::SessionConversation>,
    pub agent_id: String,
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
    pub agent_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_status: Option<AgentStatus>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_finished_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_failure: Option<agena_failure::UserProblem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Token usage of a session.
pub struct SessionUsageResource {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub measured_prompt_tokens: Option<u64>,
    pub current_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub projected_tokens: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_limit_tokens: Option<u64>,
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
    /// Prompt tokens the last request read from the provider cache.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_cache_read_tokens: Option<u64>,
    /// Prompt tokens the last request could have cached.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_cache_input_tokens: Option<u64>,
    /// `last_cache_read_tokens / last_cache_input_tokens` (0.0–1.0).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_cache_hit_rate: Option<f64>,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct WorkspaceSessionStats {
    pub total: u64,
    pub roots: u64,
    pub pinned: u64,
    pub running: u64,
    pub attention: u64,
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

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
/// Basis of a session usage limit.
pub enum SessionUsageLimitBasis {
    ContextWindow,
    PromptThreshold,
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

/// Path permission configuration of a session.
///
/// Every key is optional: an omitted key keeps the built-in default, so
/// deleting a key restores it. The built-ins are ordinary entries in `rules`,
/// which a user's same-pattern entry replaces; deleting the user's entry brings
/// the built-in back. A rule always takes the explicit read/write object form —
/// there is no shorthand string form.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct PathPermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<PathAccessModesResource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external: Option<PathAccessModesResource>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, PathAccessModesResource>,
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

/// Tool permission configuration of a session.
///
/// The built-in classes are ordinary entries: the interaction tool by name in
/// `names`, the command classes as keywords inside a shell tool's `rules`, and
/// the read-only class as the `read-only` keyword under the `*` tool. A user
/// entry of the same key replaces the built-in one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(default, deny_unknown_fields)]
pub struct ToolPermissionConfigResource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub names: BTreeMap<String, PermissionMode>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub rules: BTreeMap<String, ToolPermissionRulesResource>,
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

/// Wire status for a scheduler delivery attempt.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ScheduledJobRunStatus {
    Submitted,
    Skipped,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(untagged)]
/// Tool permission rules: a single mode or an ordered name-to-mode map.
pub enum ToolPermissionRulesResource {
    Mode(PermissionMode),
    Ordered(BTreeMap<String, PermissionMode>),
}

impl Default for SessionState {
    fn default() -> Self {
        Self::Ready { last_failure: None }
    }
}
impl SessionState {
    pub const fn as_str(&self) -> &'static str {
        self.kind().as_str()
    }

    /// The canonical state kind shared with storage, TUI, CLI, RPC, and the
    /// generated TypeScript mirror.
    pub const fn kind(&self) -> SessionStateKind {
        match self {
            Self::Creating => SessionStateKind::Creating,
            Self::Ready { .. } => SessionStateKind::Ready,
            Self::Running { .. } => SessionStateKind::Running,
            Self::AwaitingInteraction { .. } => SessionStateKind::AwaitingInteraction,
            Self::Failed { .. } => SessionStateKind::Failed,
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        SessionStateKind::parse(value).map(Self::from)
    }

    pub const fn is_running(&self) -> bool {
        self.kind().is_busy()
    }

    pub const fn is_awaiting_interaction(&self) -> bool {
        matches!(self.kind(), SessionStateKind::AwaitingInteraction)
    }

    pub const fn is_failed(&self) -> bool {
        self.kind().is_failed()
    }

    /// True only while a durable run is still executing. Waiting for a user
    /// is attention, not active model work.
    pub const fn is_busy(&self) -> bool {
        self.is_running()
    }

    pub const fn is_attention(&self) -> bool {
        self.kind().is_attention()
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
            // A foreground conversation can be idle while one of its durable
            // workers needs the real user's input. Preserve that attention in
            // the source chat without inventing a foreground execution lease.
            Self::Ready { .. } | Self::Failed { .. } if !requests.is_empty() => {
                Self::AwaitingInteraction {
                    run_id: None,
                    execution,
                    requests,
                }
            }
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

impl PathPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.workspace.is_none() && self.external.is_none() && self.rules.is_empty()
    }
}

impl NetworkPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.internet.is_none()
            && self.private.is_none()
            && self.loopback.is_none()
            && self.rules.is_empty()
    }
}

impl ToolPermissionConfigResource {
    pub fn is_empty(&self) -> bool {
        self.default.is_none() && self.names.is_empty() && self.rules.is_empty()
    }
}

impl From<SessionStateKind> for SessionState {
    /// The payload-free projection of a derived state kind.
    fn from(kind: SessionStateKind) -> Self {
        match kind {
            SessionStateKind::Creating => Self::Creating,
            SessionStateKind::Ready => Self::Ready { last_failure: None },
            SessionStateKind::Running => Self::Running {
                execution: None,
                workflow: WorkflowState::Quiescent,
                requests: Vec::new(),
            },
            SessionStateKind::AwaitingInteraction => Self::AwaitingInteraction {
                run_id: None,
                execution: None,
                requests: Vec::new(),
            },
            SessionStateKind::Failed => Self::Failed { failure: None },
        }
    }
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

impl From<&agena_domain::AttachmentSource> for PartAttachmentSource {
    fn from(source: &agena_domain::AttachmentSource) -> Self {
        match source {
            agena_domain::AttachmentSource::Url { url } => Self::Url { url: url.clone() },
            agena_domain::AttachmentSource::DataUrl { url } => Self::DataUrl { url: url.clone() },
            agena_domain::AttachmentSource::ProviderData { route, data } => Self::ProviderData {
                route: route.clone(),
                data: data.clone(),
            },
            agena_domain::AttachmentSource::Base64 { data } => Self::Base64 { data: data.clone() },
            agena_domain::AttachmentSource::FileId { file_id } => Self::FileId {
                file_id: file_id.clone(),
            },
            agena_domain::AttachmentSource::LocalPath { path } => {
                Self::LocalPath { path: path.clone() }
            }
            agena_domain::AttachmentSource::Unavailable { reason } => Self::Unavailable {
                reason: reason.clone(),
            },
        }
    }
}

impl From<&agena_domain::PermissionConfig> for PermissionConfigResource {
    fn from(value: &agena_domain::PermissionConfig) -> Self {
        PermissionConfigResource {
            path: value
                .path
                .as_ref()
                .map(|path| PathPermissionConfigResource {
                    workspace: path
                        .workspace
                        .as_ref()
                        .map(path_access_modes_resource_from_domain),
                    external: path
                        .external
                        .as_ref()
                        .map(path_access_modes_resource_from_domain),
                    rules: path
                        .rules
                        .iter()
                        .map(|(pattern, modes)| {
                            (
                                pattern.clone(),
                                path_access_modes_resource_from_domain(modes),
                            )
                        })
                        .collect(),
                }),
            network: value
                .network
                .as_ref()
                .map(|network| NetworkPermissionConfigResource {
                    internet: network.internet.map(permission_mode_resource_from_domain),
                    private: network.private.map(permission_mode_resource_from_domain),
                    loopback: network.loopback.map(permission_mode_resource_from_domain),
                    rules: network
                        .rules
                        .iter()
                        .map(|(pattern, mode)| {
                            (pattern.clone(), permission_mode_resource_from_domain(*mode))
                        })
                        .collect(),
                }),
            tools: value
                .tools
                .as_ref()
                .map(|tools| ToolPermissionConfigResource {
                    default: tools.default.map(permission_mode_resource_from_domain),
                    names: tools
                        .names
                        .iter()
                        .chain(tools.plugin.iter())
                        .map(|(name, mode)| {
                            (name.clone(), permission_mode_resource_from_domain(*mode))
                        })
                        .collect(),
                    rules: tools
                        .rules
                        .iter()
                        .map(|(name, rules)| {
                            let rules = match rules {
                                agena_domain::ToolPermissionRules::Mode(mode) => {
                                    ToolPermissionRulesResource::Mode(
                                        permission_mode_resource_from_domain(*mode),
                                    )
                                }
                                agena_domain::ToolPermissionRules::Ordered(rules) => {
                                    ToolPermissionRulesResource::Ordered(
                                        rules
                                            .iter()
                                            .map(|(pattern, mode)| {
                                                (
                                                    pattern.clone(),
                                                    permission_mode_resource_from_domain(*mode),
                                                )
                                            })
                                            .collect(),
                                    )
                                }
                            };
                            (name.clone(), rules)
                        })
                        .collect(),
                }),
            approval_model: value.approval_model.as_ref().map(|model| {
                ApprovalModelSelectionResource {
                    provider: model.provider.clone(),
                    adapter: model.adapter.clone(),
                    model: model.model.clone(),
                    thinking_mode: model.thinking_mode.clone(),
                    speed_mode: model.speed_mode.clone(),
                    verbosity: model.verbosity.clone(),
                }
            }),
        }
    }
}

impl From<PermissionConfigResource> for agena_domain::PermissionConfig {
    fn from(value: PermissionConfigResource) -> Self {
        Self {
            path: value.path.map(|path| agena_domain::PathPermissionConfig {
                workspace: path.workspace.map(path_access_modes_to_domain),
                external: path.external.map(path_access_modes_to_domain),
                rules: path
                    .rules
                    .into_iter()
                    .map(|(pattern, modes)| (pattern, path_access_modes_to_domain(modes)))
                    .collect(),
            }),
            network: value
                .network
                .map(|network| agena_domain::NetworkPermissionConfig {
                    internet: network.internet.map(permission_mode_to_domain),
                    private: network.private.map(permission_mode_to_domain),
                    loopback: network.loopback.map(permission_mode_to_domain),
                    rules: network
                        .rules
                        .into_iter()
                        .map(|(pattern, mode)| (pattern, permission_mode_to_domain(mode)))
                        .collect(),
                }),
            tools: value.tools.map(|tools| {
                agena_domain::ToolPermissionConfig {
                    default: tools.default.map(permission_mode_to_domain),
                    names: tools
                        .names
                        .into_iter()
                        .map(|(name, mode)| (name, permission_mode_to_domain(mode)))
                        .collect(),
                    plugin: Default::default(),
                    rules: tools
                        .rules
                        .into_iter()
                        .map(|(name, rules)| {
                            (
                                name,
                                match rules {
                                    ToolPermissionRulesResource::Mode(mode) => {
                                        agena_domain::ToolPermissionRules::Mode(
                                            permission_mode_to_domain(mode),
                                        )
                                    }
                                    ToolPermissionRulesResource::Ordered(rules) => {
                                        agena_domain::ToolPermissionRules::Ordered(
                                            rules
                                                .into_iter()
                                                .map(|(pattern, mode)| {
                                                    (pattern, permission_mode_to_domain(mode))
                                                })
                                                .collect(),
                                        )
                                    }
                                },
                            )
                        })
                        .collect(),
                }
                .with_canonical_tool_names()
            }),
            approval_model: value.approval_model.map(|model| {
                agena_domain::ApprovalModelSelection {
                    provider: model.provider,
                    adapter: model.adapter,
                    model: model.model,
                    thinking_mode: model.thinking_mode,
                    speed_mode: model.speed_mode,
                    verbosity: model.verbosity,
                    // This runtime option has never been part of the permission wire resource.
                    parallel_tool_calls: None,
                }
            }),
        }
    }
}

impl From<agena_domain::PermissionReplyKind> for PermissionReplyKind {
    fn from(value: agena_domain::PermissionReplyKind) -> Self {
        match value {
            agena_domain::PermissionReplyKind::AllowOnce => Self::AllowOnce,
            agena_domain::PermissionReplyKind::AllowAlways => Self::AllowAlways,
            agena_domain::PermissionReplyKind::DenyOnce => Self::DenyOnce,
            agena_domain::PermissionReplyKind::DenyAlways => Self::DenyAlways,
            agena_domain::PermissionReplyKind::AutoApprove => Self::AutoApprove,
        }
    }
}

impl From<agena_domain::PermissionScope> for PermissionScope {
    fn from(value: agena_domain::PermissionScope) -> Self {
        match value {
            agena_domain::PermissionScope::Session => Self::Session,
            agena_domain::PermissionScope::Workspace => Self::Workspace,
            agena_domain::PermissionScope::Global => Self::Global,
        }
    }
}

fn path_access_modes_resource_from_domain(
    value: &agena_domain::PathAccessModes,
) -> PathAccessModesResource {
    PathAccessModesResource {
        read: value.read.map(permission_mode_resource_from_domain),
        write: value.write.map(permission_mode_resource_from_domain),
    }
}

const fn permission_mode_resource_from_domain(
    value: agena_domain::PermissionMode,
) -> PermissionMode {
    match value {
        agena_domain::PermissionMode::Allow => PermissionMode::Allow,
        agena_domain::PermissionMode::Auto => PermissionMode::Auto,
        agena_domain::PermissionMode::Ask => PermissionMode::Ask,
        agena_domain::PermissionMode::Deny => PermissionMode::Deny,
    }
}

fn path_access_modes_to_domain(value: PathAccessModesResource) -> agena_domain::PathAccessModes {
    agena_domain::PathAccessModes {
        read: value.read.map(permission_mode_to_domain),
        write: value.write.map(permission_mode_to_domain),
    }
}

const fn permission_mode_to_domain(value: PermissionMode) -> agena_domain::PermissionMode {
    match value {
        PermissionMode::Allow => agena_domain::PermissionMode::Allow,
        PermissionMode::Auto => agena_domain::PermissionMode::Auto,
        PermissionMode::Ask => agena_domain::PermissionMode::Ask,
        PermissionMode::Deny => agena_domain::PermissionMode::Deny,
    }
}

impl From<PermissionReply> for agena_domain::PermissionReply {
    fn from(value: PermissionReply) -> Self {
        Self {
            request_id: value.request_id,
            kind: match value.kind {
                PermissionReplyKind::AllowOnce => agena_domain::PermissionReplyKind::AllowOnce,
                PermissionReplyKind::AllowAlways => agena_domain::PermissionReplyKind::AllowAlways,
                PermissionReplyKind::DenyOnce => agena_domain::PermissionReplyKind::DenyOnce,
                PermissionReplyKind::DenyAlways => agena_domain::PermissionReplyKind::DenyAlways,
                PermissionReplyKind::AutoApprove => agena_domain::PermissionReplyKind::AutoApprove,
            },
            reason: value.reason,
            scope: value.scope.map(|scope| match scope {
                PermissionScope::Session => agena_domain::PermissionScope::Session,
                PermissionScope::Workspace => agena_domain::PermissionScope::Workspace,
                PermissionScope::Global => agena_domain::PermissionScope::Global,
            }),
        }
    }
}

impl From<UserInputReply> for agena_domain::UserInputReply {
    fn from(value: UserInputReply) -> Self {
        Self {
            request_id: value.request_id,
            kind: match value.kind {
                UserInputReplyKind::Submit => agena_domain::UserInputReplyKind::Submit,
                UserInputReplyKind::Cancel => agena_domain::UserInputReplyKind::Cancel,
                UserInputReplyKind::Timeout => agena_domain::UserInputReplyKind::Timeout,
            },
            answers: value.answers,
            reason: value.reason,
        }
    }
}

impl From<agena_domain::UserInputReply> for UserInputReply {
    fn from(value: agena_domain::UserInputReply) -> Self {
        Self {
            request_id: value.request_id,
            kind: match value.kind {
                agena_domain::UserInputReplyKind::Submit => UserInputReplyKind::Submit,
                agena_domain::UserInputReplyKind::Cancel => UserInputReplyKind::Cancel,
                agena_domain::UserInputReplyKind::Timeout => UserInputReplyKind::Timeout,
            },
            answers: value.answers,
            reason: value.reason,
        }
    }
}

impl PermissionConfigResource {
    /// Project a selected configuration for a client write. Runtime-only plugin
    /// grants are deliberately excluded, as they are by the domain serializer.
    /// The `From<&PermissionConfig>` projection remains the effective-policy
    /// view used in server responses, where those grants belong.
    /// Configuration-only approval options cannot be silently dropped from a
    /// session write. Keep the previous serializer-to-wire validation failure.
    pub fn try_from_configuration(
        value: &agena_domain::PermissionConfig,
    ) -> Result<Self, serde_json::Error> {
        if value
            .approval_model
            .as_ref()
            .is_some_and(|model| model.parallel_tool_calls.is_some())
        {
            return Err(<serde_json::Error as serde::de::Error>::custom(
                "unknown field `parallel_tool_calls` in session permission approval_model",
            ));
        }
        let mut resource = Self::from(value);
        if let (Some(tools), Some(resource_tools)) = (&value.tools, &mut resource.tools) {
            resource_tools.names = tools
                .names
                .iter()
                .map(|(name, mode)| (name.clone(), permission_mode_resource_from_domain(*mode)))
                .collect();
        }
        Ok(resource)
    }
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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
/// An option of a user-input question.
pub struct UserInputOption {
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub custom: bool,
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

fn is_false(value: &bool) -> bool {
    !*value
}

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
    /// Persistent identity of the owning instance database.
    pub instance_id: String,
    pub id: Uuid,
    pub pid: u32,
    pub started_at: DateTime<Utc>,
    pub protocol_version: u32,
}
