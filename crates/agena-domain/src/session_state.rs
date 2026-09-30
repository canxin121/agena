use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Lifecycle status of a delegated subtask.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, Default)]
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

#[cfg(test)]
mod tests {
    use super::{
        SessionLifecycleState, SessionRelationKind, SessionStateKind, SubtaskStatus, WorkflowState,
    };
    use crate::ExecutionPhase;

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
            serde_json::to_string(&WorkflowState::AwaitingInteraction).unwrap(),
            "\"awaiting_interaction\""
        );
        assert!(serde_json::from_str::<WorkflowState>("\"ready_for_model\"").is_err());
    }

    /// Exhaustive guard: a new [`SessionStateKind`] variant fails to compile here
    /// until its wire spelling is pinned in this test.
    fn expected_wire_name(kind: SessionStateKind) -> &'static str {
        match kind {
            SessionStateKind::Creating => "creating",
            SessionStateKind::Ready => "ready",
            SessionStateKind::Running => "running",
            SessionStateKind::AwaitingInteraction => "awaiting_interaction",
            SessionStateKind::Failed => "failed",
        }
    }

    fn wire_names(keep: fn(SessionStateKind) -> bool) -> Vec<&'static str> {
        SessionStateKind::ALL
            .into_iter()
            .filter(|kind| keep(*kind))
            .map(SessionStateKind::as_str)
            .collect()
    }

    #[test]
    fn session_state_kinds_keep_one_wire_vocabulary_everywhere() {
        let mut seen = Vec::new();
        for kind in SessionStateKind::ALL {
            let wire = expected_wire_name(kind);
            assert_eq!(
                kind.as_str(),
                wire,
                "as_str must spell {kind:?} like the wire"
            );
            assert_eq!(SessionStateKind::parse(wire), Some(kind));
            assert_eq!(
                serde_json::to_value(kind).expect("serialize kind"),
                serde_json::json!(wire)
            );
            let decoded: SessionStateKind =
                serde_json::from_value(serde_json::json!(wire)).expect("decode kind");
            assert_eq!(decoded, kind);
            assert!(!seen.contains(&wire), "{wire} is spelled twice");
            seen.push(wire);
        }
        assert_eq!(seen.len(), SessionStateKind::ALL.len());
        assert_eq!(SessionStateKind::default(), SessionStateKind::Ready);
        assert_eq!(SessionStateKind::parse("succeeded"), None);
        assert!(
            serde_json::from_value::<SessionStateKind>(serde_json::json!("ready_for_model"))
                .is_err()
        );
    }

    #[test]
    fn session_state_kind_predicates_partition_the_same_vocabulary() {
        assert_eq!(wire_names(SessionStateKind::is_busy), vec!["running"]);
        assert_eq!(
            wire_names(SessionStateKind::is_attention),
            vec!["awaiting_interaction", "failed"]
        );
        assert_eq!(wire_names(SessionStateKind::is_failed), vec!["failed"]);

        for kind in SessionStateKind::ALL {
            if kind.is_failed() {
                assert!(kind.is_attention(), "{kind:?} must need attention");
            }
            if kind.is_busy() {
                assert!(
                    !kind.is_attention(),
                    "a busy kind is only attention through its pending requests"
                );
            }
        }
    }

    #[test]
    fn shared_state_enums_agree_between_as_str_parse_and_serde() {
        for status in [
            SubtaskStatus::Created,
            SubtaskStatus::Running,
            SubtaskStatus::Completed,
            SubtaskStatus::Failed,
            SubtaskStatus::Cancelled,
            SubtaskStatus::TimedOut,
            SubtaskStatus::Interrupted,
        ] {
            let wire: &str = status.as_ref();
            assert_eq!(SubtaskStatus::parse(wire), Some(status));
            assert_eq!(
                serde_json::to_value(status).expect("serialize subtask"),
                serde_json::json!(wire)
            );
        }

        for relation in [
            SessionRelationKind::Root,
            SessionRelationKind::Child,
            SessionRelationKind::Fork,
            SessionRelationKind::Rewind,
            SessionRelationKind::Subagent,
        ] {
            let wire = relation.as_str();
            assert_eq!(SessionRelationKind::parse(wire), Some(relation));
            assert_eq!(
                serde_json::to_value(relation).expect("serialize relation"),
                serde_json::json!(wire)
            );
        }

        for lifecycle in [
            SessionLifecycleState::Creating,
            SessionLifecycleState::Ready,
            SessionLifecycleState::Failed,
        ] {
            let wire = lifecycle.as_str();
            assert_eq!(SessionLifecycleState::parse(wire), Some(lifecycle));
            assert_eq!(
                serde_json::to_value(lifecycle).expect("serialize lifecycle"),
                serde_json::json!(wire)
            );
        }

        for workflow in [
            WorkflowState::Quiescent,
            WorkflowState::ToolPending,
            WorkflowState::AwaitingInteraction,
        ] {
            let value = serde_json::to_value(workflow).expect("serialize workflow");
            assert_eq!(
                serde_json::from_value::<WorkflowState>(value).expect("decode workflow"),
                workflow
            );
        }

        for phase in [
            ExecutionPhase::Starting,
            ExecutionPhase::PreparingModel,
            ExecutionPhase::StreamingModel,
            ExecutionPhase::ExecutingTools,
            ExecutionPhase::AwaitingInteraction,
            ExecutionPhase::Cancelling,
        ] {
            let value = serde_json::to_value(phase).expect("serialize phase");
            assert_eq!(
                serde_json::from_value::<ExecutionPhase>(value).expect("decode phase"),
                phase
            );
        }

        assert!(
            serde_json::from_value::<WorkflowState>(serde_json::json!("ready_for_model")).is_err()
        );
        assert!(serde_json::from_value::<ExecutionPhase>(serde_json::json!("thinking")).is_err());
    }
}
