use serde::{Deserialize, Serialize};

/// Lifecycle status of a delegated subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
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

impl AsRef<str> for SubtaskStatus {
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

impl SubtaskStatus {
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
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

/// Persistent state of a session's execution workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum WorkflowState {
    #[default]
    Quiescent,
    ReadyForModel,
    ToolPending,
    Blocked,
}

#[cfg(test)]
mod tests {
    use super::{SessionLifecycleState, SessionRelationKind, SubtaskStatus, WorkflowState};

    #[test]
    fn session_state_values_have_stable_wire_spellings_and_semantics() {
        assert_eq!(SubtaskStatus::default(), SubtaskStatus::Created);
        assert_eq!(
            SubtaskStatus::parse("timed_out"),
            Some(SubtaskStatus::TimedOut)
        );
        assert!(SubtaskStatus::Cancelled.is_terminal());
        assert!(!SubtaskStatus::Running.is_terminal());
        assert_eq!(SubtaskStatus::Interrupted.as_ref(), "interrupted");

        assert_eq!(
            SessionRelationKind::parse("subagent"),
            Some(SessionRelationKind::Subagent)
        );
        assert!(SessionRelationKind::Subagent.is_subagent());
        assert_eq!(SessionRelationKind::Fork.as_str(), "fork");

        assert_eq!(
            SessionLifecycleState::default(),
            SessionLifecycleState::Ready
        );
        assert_eq!(
            SessionLifecycleState::parse("creating"),
            Some(SessionLifecycleState::Creating)
        );
        assert_eq!(SessionLifecycleState::Failed.as_str(), "failed");

        assert_eq!(
            serde_json::to_string(&WorkflowState::Blocked).unwrap(),
            "\"blocked\""
        );
        assert_eq!(
            serde_json::from_str::<WorkflowState>("\"ready_for_model\"").unwrap(),
            WorkflowState::ReadyForModel
        );
    }
}
