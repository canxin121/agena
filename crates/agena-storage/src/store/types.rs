//! Detached provider/transcript presentation values. Session authority lives
//! in typed kernel commands and handles; these values have no save operation.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Author of a part. The schema allows `runtime` in addition to the four
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

/// A persisted part — the only chat-content entity.
///
/// `kind` is an open set (`run`, `text`, `think`, `tool_call`, `file_ref`,
/// `paste_ref`, `skill_ref`, `notice`, `hook`, `compaction`, `error`, ...).
/// Ordering within a session is always
/// `(created_at_ms, part_id)` (decision D4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    pub part_id: i64,
    pub kind: String,
    pub role: PartRole,
    pub state: PartState,
    /// Read-only projection of kernel entry and tool facts.
    pub content: Value,
    pub summary: Option<String>,
    pub visibility: PartVisibility,
    pub parent_part_id: Option<i64>,
    /// Stable user entry or per-generation first assistant entry identity.
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
    /// Provider continuation projection; durable raw state belongs to its round.
    pub provider_state: Option<Value>,
}

impl Part {
    /// Membership callers select the audience separately. Resource identity,
    /// kind and origin ownership have one factual validation rule.
    pub fn references_content(&self, resource: &agena_domain::ContentResource) -> bool {
        self.part_id == resource.part_id
            && self.origin_session_id == resource.owner_session_id
            && self.resources().is_ok_and(|references| {
                references.iter().any(|reference| {
                    reference.resource_id == resource.resource_id && reference.kind == resource.kind
                })
            })
    }

    /// Only the canonical Part resource section establishes ownership.
    pub fn resources(&self) -> Result<Vec<agena_domain::ContentRef>, super::StoreError> {
        self.content.get("resources").map_or_else(
            || Ok(Vec::new()),
            |value| {
                serde_json::from_value(value.clone()).map_err(|error| {
                    super::StoreError::Serialization(format!("Part resource references: {error}"))
                })
            },
        )
    }

    pub fn is_run_marker(&self) -> bool {
        self.kind == "run"
    }

    /// A complete user/assistant message, including failed and cancelled
    /// endings. Content and tool parts are children of the message's run;
    /// finishing one of those children does not finish the message.
    pub fn is_terminal_message(&self) -> bool {
        self.is_run_marker()
            && matches!(self.role, PartRole::User | PartRole::Assistant)
            && self.state.is_terminal()
    }
}

/// Presentation vocabulary is the single domain definition.
pub use agena_domain::SessionStateKind as SessionState;

#[cfg(test)]
mod tests {
    use super::{PartVisibility, SessionState};
    #[test]
    fn visibility_is_a_read_projection_choice() {
        assert!(PartVisibility::Both.visible_to_ai());
        assert!(PartVisibility::Both.visible_to_user());
        assert!(PartVisibility::Ai.visible_to_ai());
        assert!(!PartVisibility::Ai.visible_to_user());
        assert!(!PartVisibility::User.visible_to_ai());
        assert!(PartVisibility::User.visible_to_user());
    }
    #[test]
    fn state_vocabulary_has_domain_type_identity() {
        let kind: agena_domain::SessionStateKind = SessionState::AwaitingInteraction;
        assert_eq!(kind, agena_domain::SessionStateKind::AwaitingInteraction);
        for state in SessionState::ALL {
            assert_eq!(SessionState::parse(state.as_str()), Some(state));
        }
    }
}
