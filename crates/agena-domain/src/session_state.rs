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
/// Storage derives this state from current kernel execution facts and pending
/// interactions. API projections and TUI/CLI/Web consumers share the same
/// vocabulary; database ownership and recovery remain kernel responsibilities.

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
