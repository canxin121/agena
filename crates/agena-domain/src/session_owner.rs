//! Which Bot a conversation belongs to, and through which entrance.
//!
//! Ownership is orthogonal to execution scope: `workspace_id` decides where
//! tools run (a project root, or the instance scope with no directory), while
//! `owner` decides who the conversation belongs to and which entrance it
//! represents. A Bot's local conversation and its chat conversations share an
//! owner but not an execution scope.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The stable owner record for one conversation.
///
/// `surface` is the conversation's entrance shape, not an authority: the
/// capability a conversation holds still comes from its permission
/// configuration and, for chat, from the binding grant. Server code treats an
/// owner with `surface == Local` as the operator conversation that may manage
/// other project sessions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SessionOwner {
    #[schemars(with = "String")]
    pub bot: uuid::Uuid,
    pub surface: SessionSurface,
    /// Chat conversations: the stable scope key that maps one external room,
    /// thread and speaker set onto exactly one conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope_key: Option<String>,
}

/// Where a Bot conversation was entered from. Four mutually exclusive shapes
/// cover every adapter: the local surfaces, one-to-one chats, multi-party
/// rooms (including group DMs and interactive channels), and broadcast
/// surfaces where posting may be restricted to room owners.
#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Hash,
    PartialOrd,
    Ord,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SessionSurface {
    #[default]
    Local,
    Direct,
    Group,
    Broadcast,
}

impl SessionSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Local => "local",
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Broadcast => "broadcast",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "local" => Some(Self::Local),
            "direct" => Some(Self::Direct),
            "group" => Some(Self::Group),
            "broadcast" => Some(Self::Broadcast),
            _ => None,
        }
    }

    /// True for the local authenticated surfaces (Web, TUI, CLI, API).
    pub const fn is_local(self) -> bool {
        matches!(self, Self::Local)
    }

    /// True for every chat entrance.
    pub const fn is_chat(self) -> bool {
        !self.is_local()
    }
}

impl From<crate::ConversationKind> for SessionSurface {
    fn from(kind: crate::ConversationKind) -> Self {
        match kind {
            crate::ConversationKind::Direct => Self::Direct,
            crate::ConversationKind::Group => Self::Group,
            crate::ConversationKind::Broadcast => Self::Broadcast,
        }
    }
}

impl From<SessionSurface> for crate::ConversationKind {
    fn from(surface: SessionSurface) -> Self {
        match surface {
            SessionSurface::Local | SessionSurface::Direct => Self::Direct,
            SessionSurface::Group => Self::Group,
            SessionSurface::Broadcast => Self::Broadcast,
        }
    }
}

impl From<crate::LocalSurface> for SessionSurface {
    fn from(_: crate::LocalSurface) -> Self {
        Self::Local
    }
}
