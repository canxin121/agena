use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle status of an agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum AgentStatus {
    #[default]
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
    TimedOut,
    Interrupted,
}

impl AsRef<str> for AgentStatus {
    fn as_ref(&self) -> &str {
        match self {
            Self::Created => "created",
            Self::Running => "running",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::TimedOut => "timed_out",
            Self::Interrupted => "interrupted",
        }
    }
}

impl AgentStatus {
    pub fn parse(value: &str) -> Option<Self> {
        match value.trim() {
            "created" => Some(Self::Created),
            "running" => Some(Self::Running),
            "completed" => Some(Self::Completed),
            "failed" => Some(Self::Failed),
            "cancelled" => Some(Self::Cancelled),
            "timed_out" => Some(Self::TimedOut),
            "interrupted" => Some(Self::Interrupted),
            _ => None,
        }
    }

    pub const fn is_terminal(self) -> bool {
        !matches!(self, Self::Created | Self::Running)
    }
}

/// Domain meaning of a session's immutable parent edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionRelationKind {
    #[default]
    Root,
    Child,
    Fork,
    Rewind,
    Subagent,
}

impl SessionRelationKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Child => "child",
            Self::Fork => "fork",
            Self::Rewind => "rewind",
            Self::Subagent => "subagent",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "root" => Some(Self::Root),
            "child" => Some(Self::Child),
            "fork" => Some(Self::Fork),
            "rewind" => Some(Self::Rewind),
            "subagent" => Some(Self::Subagent),
            _ => None,
        }
    }

    pub const fn is_subagent(self) -> bool {
        matches!(self, Self::Subagent)
    }
}

/// Visibility/readiness status for a persisted session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Creating,
    #[default]
    Ready,
    Failed,
}

impl SessionLifecycleState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "creating" => Some(Self::Creating),
            "ready" => Some(Self::Ready),
            "failed" => Some(Self::Failed),
            _ => None,
        }
    }
}

/// The single client-facing processing state of a session.
///
/// This is the one canonical definition of the derived session state shared by
/// every layer: storage derives it from the durable run markers and pending
/// interactions ([`agena_storage::store::SessionState`] is a re-export of this
/// type), the API projects it with per-kind payloads, and TUI/CLI/Web branch on
/// [`SessionStateKind::as_str`] or the generated TypeScript mirrors. Storing a
/// second copy of these variants anywhere else is a bug.
///
/// There is no "owner" or lease dimension here. One server process owns the
/// data directory, so a run marker that is in flight is a run this process is
/// executing: the marker alone answers "is this session busy" (17.3). A crash
/// leaves an ownerless marker behind, but it is resolved by the recovering
/// process before it serves a single read, so clients never observe it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum SessionStateKind {
    /// `sessions.lifecycle_state = creating` — not yet usable.
    Creating,
    /// No in-flight run, no pending interaction.
    #[default]
    Ready,
    /// A run marker is in flight: this server is executing a response.
    Running,
    /// An in-flight `tool_call` with unanswered `user_input` gates the session.
    AwaitingInteraction,
    /// Lifecycle failed, or the last run terminally failed and is not resumable.
    Failed,
}

impl SessionStateKind {
    /// Every kind, in derivation precedence order.
    pub const ALL: [Self; 5] = [
        Self::Creating,
        Self::Ready,
        Self::Running,
        Self::AwaitingInteraction,
        Self::Failed,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Creating => "creating",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::AwaitingInteraction => "awaiting_interaction",
            Self::Failed => "failed",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|candidate| candidate.as_str() == value)
    }

    /// True only while a durable run is still executing.
    pub const fn is_busy(self) -> bool {
        matches!(self, Self::Running)
    }

    /// Kinds that always need user attention: paused on user input, or
    /// terminally failed. `Running` is not included because its attention
    /// depends on pending requests.
    pub const fn is_attention(self) -> bool {
        matches!(self, Self::AwaitingInteraction | Self::Failed)
    }

    pub const fn is_failed(self) -> bool {
        matches!(self, Self::Failed)
    }
}

/// Persistent state of a session's execution workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    #[default]
    Quiescent,
    ToolPending,
    AwaitingInteraction,
}
