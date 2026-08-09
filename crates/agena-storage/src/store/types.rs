//! Pure data types for the v2 parts-first store.
//!
//! These are the only values that cross the store boundary. No database or
//! SeaORM type appears anywhere in this module, so both the SQLite engine and
//! the in-memory engine can implement the same contract and external callers
//! never see a memory/DB split.

use agena_domain::{SessionLifecycleState, SessionRelationKind};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Author of a part. The v2 schema allows `runtime` in addition to the four
/// message roles, so this is a superset of `agena_domain::Role`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartRole {
    User,
    Assistant,
    System,
    Tool,
    Runtime,
}

impl PartRole {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Assistant => "assistant",
            Self::System => "system",
            Self::Tool => "tool",
            Self::Runtime => "runtime",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "user" => Some(Self::User),
            "assistant" => Some(Self::Assistant),
            "system" => Some(Self::System),
            "tool" => Some(Self::Tool),
            "runtime" => Some(Self::Runtime),
            _ => None,
        }
    }
}

/// Lifecycle state of a part (uniform for every part kind, section 17.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartState {
    Pending,
    InProgress,
    Completed,
    Failed,
    Cancelled,
}

impl PartState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "pending" => Some(Self::Pending),
            "in_progress" => Some(Self::InProgress),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// A state that still has work in flight (the run is not finished).
    pub const fn is_in_flight(self) -> bool {
        matches!(self, Self::Pending | Self::InProgress)
    }

    /// A state from which no further transition is allowed.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }

    /// Whether `self` may transition to `to` under the uniform lifecycle
    /// (17.2): `failed -> in_progress` is the retry edge; everything else is
    /// forward-only from `pending`/`in_progress`.
    pub const fn can_transition(self, to: Self) -> bool {
        matches!(
            (self, to),
            (Self::Pending, Self::InProgress)
                | (Self::Pending, Self::Cancelled)
                | (Self::InProgress, Self::Completed)
                | (Self::InProgress, Self::Failed)
                | (Self::InProgress, Self::Cancelled)
                | (Self::Failed, Self::InProgress)
        )
    }
}

/// Who may see a part (section 18.3).
///
/// The prompt builder (AI) receives `both | ai`; the UI (human) renders
/// `both | user`. Visibility is per part, not per session, so forks share
/// parts exactly as they are.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PartVisibility {
    Both,
    User,
    Ai,
}

impl PartVisibility {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::User => "user",
            Self::Ai => "ai",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "both" => Some(Self::Both),
            "user" => Some(Self::User),
            "ai" => Some(Self::Ai),
            _ => None,
        }
    }

    /// Whether the AI prompt builder should include this part.
    pub const fn visible_to_ai(self) -> bool {
        matches!(self, Self::Both | Self::Ai)
    }

    /// Whether the human UI should render this part.
    pub const fn visible_to_user(self) -> bool {
        matches!(self, Self::Both | Self::User)
    }
}

/// A persisted part — the only chat-content entity in v2.
///
/// `kind` is an open set (`run`, `text`, `think`, `tool_call`, `tool_result`,
/// `file_ref`, `paste_ref`, `skill_ref`, `notice`, `hook`, `compaction`,
/// `error`, `interaction`, ...). Ordering within a session is always
/// `(created_at_ms, part_id)` (decision D4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Part {
    pub part_id: i64,
    pub kind: String,
    pub role: PartRole,
    pub state: PartState,
    /// Canonical raw payload — exactly what the AI sees (section 18.4).
    pub content: Value,
    pub summary: Option<String>,
    pub visibility: PartVisibility,
    /// Human-friendly Markdown rendered by the producing plugin/tool, when any.
    pub rendered_markdown: Option<String>,
    pub parent_part_id: Option<i64>,
    /// The run marker this part belongs to (`NULL` on run markers themselves).
    pub run_id: Option<i64>,
    /// The session that created this part. Only the origin session may update
    /// a part in place (shared parts are read/append-only, section 8.4).
    pub origin_session_id: i64,
    /// Monotonic per-part counter, bumped on every update (retries increment).
    pub revision: i64,
    pub started_at_ms: i64,
    pub finished_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    /// Provider continuation state; run markers only (13.2).
    pub provider_state: Option<Value>,
}

impl Part {
    pub fn is_run_marker(&self) -> bool {
        self.kind == "run"
    }
}

/// A part to create. The engine allocates the id, timestamps, revision, and
/// membership edge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewPart {
    pub kind: String,
    pub role: PartRole,
    pub content: Value,
    pub summary: Option<String>,
    pub visibility: PartVisibility,
    pub rendered_markdown: Option<String>,
    pub parent_part_id: Option<i64>,
    /// Initial state. `pending` is typical; `completed` is used for parts
    /// created already-done (e.g. `tool_result`).
    pub state: PartState,
}

impl NewPart {
    pub fn pending(kind: impl Into<String>, role: PartRole, content: Value) -> Self {
        Self {
            kind: kind.into(),
            role,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id: None,
            state: PartState::Pending,
        }
    }
}

/// A streaming update applied to an existing part. Every field is optional;
/// `Some` fields are applied, `None` fields are left unchanged. The engine
/// bumps `revision` and `updated_at_ms` on every update.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct PartDelta {
    pub state: Option<PartState>,
    /// Replace the whole content JSON document.
    pub content: Option<Value>,
    /// Append a string delta to a text-shaped content value (streaming).
    pub content_text_delta: Option<String>,
    pub summary: Option<String>,
    pub rendered_markdown: Option<String>,
    pub provider_state: Option<Value>,
    /// Explicit terminal timestamp (used on completion); defaults to "now".
    pub finished_at_ms: Option<i64>,
}

/// Outcome of [`PersistenceEngine::complete_run`] — how a run marker ends.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunOutcome {
    /// `completed` | `failed` | `cancelled` (never a non-terminal state).
    pub status: PartState,
    /// Required for `failed`/`cancelled` (`lease_stolen`, `process_restart`,
    /// `user_cancelled`, ...). For `completed` the marker records JSON null.
    pub abort_reason: Option<String>,
    /// Optional replacement for the run marker content (e.g. model metadata).
    pub content: Option<Value>,
    pub provider_state: Option<Value>,
}

/// Result of [`PersistenceEngine::submit_user_message`].
#[derive(Debug, Clone, PartialEq)]
pub struct SubmitOutcome {
    /// The run marker part id (the run identity).
    pub run_id: i64,
    /// `false` when an idempotency key deduplicated this send.
    pub created: bool,
    /// The marker plus the created content parts, in creation order.
    pub parts: Vec<Part>,
}

/// The session-level metadata row. `sessions` stores only identity/lineage,
/// config, and provider anchors — session state is derived from parts + leases.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub workspace_id: i64,
    pub relation_kind: SessionRelationKind,
    pub cutoff_part_id: Option<i64>,
    pub title: String,
    /// Optimistic-lock counter, bumped on every session mutation.
    pub version: i64,
    pub lifecycle_state: SessionLifecycleState,
    pub creation_failure: Option<Value>,
    pub task_id: Option<String>,
    pub subtask_status: Option<String>,
    pub subtask_started_at_ms: Option<i64>,
    pub subtask_finished_at_ms: Option<i64>,
    pub subtask_failure: Option<Value>,
    /// Execution config only (D5): permission ceiling, capability denials,
    /// workspace root override, selection/access defaults.
    pub config_json: Option<Value>,
    /// Provider continuation anchors persisted so resume does not re-prime
    /// provider caches (D8, 13.3).
    pub provider_anchors_json: Option<Value>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A session's transcript: metadata plus parts ordered by `(created_at_ms,
/// part_id)` (one membership JOIN in the engine).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionView {
    pub meta: SessionMeta,
    pub parts: Vec<Part>,
}

/// Input for creating a new session row.
#[derive(Debug, Clone, PartialEq)]
pub struct NewSession {
    pub workspace_id: i64,
    pub parent_id: Option<i64>,
    pub relation_kind: SessionRelationKind,
    pub cutoff_part_id: Option<i64>,
    pub title: String,
    pub task_id: Option<String>,
    pub config_json: Option<Value>,
    pub provider_anchors_json: Option<Value>,
}

/// A compact row for session listing (sections 13.1 / 14.1).
#[derive(Debug, Clone, PartialEq)]
pub struct SessionSummary {
    pub id: i64,
    pub workspace_id: i64,
    pub parent_id: Option<i64>,
    pub depth: i64,
    pub root_id: i64,
    pub title: String,
    pub relation_kind: SessionRelationKind,
    pub lifecycle_state: SessionLifecycleState,
    /// Optimistic-lock counter, bumped on every session mutation.
    pub version: i64,
    pub task_id: Option<String>,
    pub subtask_status: Option<String>,
    /// Number of run markers in the session (D9: message_count = run markers).
    pub message_count: i64,
    pub child_session_count: i64,
    pub last_message_at_ms: Option<i64>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

/// A paging cursor for session listing, ordered by `(updated_at_ms, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SessionCursor {
    pub updated_at_ms: i64,
    pub id: i64,
}

/// Query for [`PersistenceEngine::list_session_summaries`].
///
/// The listing surface (13.1 / 14.1) filters by workspace, optional parent,
/// optional roots-only, optional title search, and pages newest-first by the
/// `(updated_at_ms, id)` cursor.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionListQuery {
    pub workspace_id: Option<i64>,
    /// Restrict to direct children of this session.
    pub parent_id: Option<i64>,
    /// Restrict to root sessions (`parent_id IS NULL`).
    pub roots_only: bool,
    /// Case-insensitive `title LIKE %search%` filter.
    pub search: Option<String>,
    pub limit: Option<i64>,
    pub before: Option<SessionCursor>,
}

/// A single lease row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaseState {
    pub session_id: i64,
    pub owner_id: String,
    pub run_id: Option<i64>,
    pub lease_started_at_ms: i64,
    pub heartbeat_at_ms: i64,
}

/// Result of [`PersistenceEngine::try_acquire_lease`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeaseAcquire {
    /// This caller now owns the lease. Any stale in-flight run markers were
    /// aborted atomically in the same transaction (invariant 2).
    Acquired {
        /// Run markers aborted by the steal (abort_reason = `lease_stolen`).
        reconciled_runs: Vec<i64>,
    },
    /// Another owner holds a fresh lease; acquisition refused.
    HeldBy {
        owner_id: String,
        heartbeat_at_ms: i64,
    },
}

/// Result of [`PersistenceEngine::reconcile`] (17.4 step 2c).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ReconcileOutcome {
    /// Run markers marked `failed` (abort_reason = `process_restart`).
    pub aborted_runs: Vec<i64>,
    /// Non-terminal child parts of those runs marked `cancelled`.
    pub cancelled_parts: usize,
}

/// One provider model call (section 16). Append-only, immutable, one row per
/// provider response.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    pub workspace_id: i64,
    pub session_id: i64,
    pub run_id: Option<i64>,
    pub provider_id: String,
    pub model_id: String,
    pub created_at_ms: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_write_tokens: i64,
    pub cache_read_tokens: i64,
    pub tool_use_tokens: i64,
    pub other_tokens: i64,
    pub total_cost_micros: i64,
    pub recorded_cost_micros: Option<i64>,
    pub cost_estimate_incomplete: bool,
    pub detail_json: Option<Value>,
}

/// Query for [`PersistenceEngine::usage_stats`]. All shapes are pure SQL over
/// index ranges (16.3).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsageQuery {
    pub session_id: Option<i64>,
    pub workspace_id: Option<i64>,
    pub provider_id: Option<String>,
    pub model_id: Option<String>,
    pub after_ms: Option<i64>,
    pub before_ms: Option<i64>,
}

/// Per-provider×model usage group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsageGroup {
    pub provider_id: String,
    pub model_id: String,
    pub calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub reasoning_tokens: i64,
    pub cache_write_tokens: i64,
    pub cache_read_tokens: i64,
    pub total_cost_micros: i64,
}

/// Aggregated usage stats returned through the facade.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UsageStats {
    pub groups: Vec<UsageGroup>,
    pub total_calls: i64,
    pub total_input_tokens: i64,
    pub total_output_tokens: i64,
    pub total_cost_micros: i64,
}

/// The single derived session state (17.3). One derivation function over
/// parts + leases produces the same state in every process.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// `sessions.lifecycle_state = creating` — not yet usable.
    Creating,
    /// No in-flight run, no pending interaction.
    Ready,
    /// An in-flight run marker with a fresh lease.
    Running,
    /// A pending interaction part gates the session — user's turn.
    AwaitingUser,
    /// An in-flight run marker with a stale/no lease (crash) — reconciling.
    Interrupted,
    /// Lifecycle failed, or the last run terminally failed and is not resumable.
    Failed,
}

/// A pending interaction the UI must surface when `SessionState::AwaitingUser`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InteractionRef {
    pub part_id: i64,
    /// `ask_user` | `plan_review` | `permission` | ...
    pub kind: String,
    pub prompt: String,
    pub content: Value,
}

/// The single object the UI reads for session-level state (17.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionPresentation {
    pub state: SessionState,
    pub pending_interaction: Option<InteractionRef>,
    /// Run marker part id when `Running`.
    pub active_run_id: Option<i64>,
    /// Last error part content when `Failed` or after an Interrupted reconcile.
    pub last_failure: Option<Value>,
}
