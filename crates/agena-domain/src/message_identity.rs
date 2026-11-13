//! Who sent a message, and where they sent it from.
//!
//! Three independent axes, deliberately kept apart:
//!
//! - [`Platform`] — the transport and surface the message arrived through.
//! - [`ConversationKind`] — the shape of the room (direct, group, channel).
//! - [`MessageAuthor`] — the verified sender and the authority that follows from
//!   that identity.
//!
//! The author's kind is an authorization *conclusion*, not a claim made by the
//! transport. [`AuthorKind::InstanceOperator`] can only be produced on the local
//! authenticated surfaces; a chat adapter can never construct it, however its
//! message is shaped. Everything a channel adapter supplies is likewise
//! re-derived on the host from `channel_policy`, never trusted from the wire.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::channel_access::ChannelRole;

/// A local, authenticated Agena surface. Reaching one requires the instance
/// token, so every author arriving through one is an instance operator.
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
pub enum LocalSurface {
    #[default]
    Web,
    Tui,
    Cli,
    Api,
}

impl LocalSurface {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Web => "web",
            Self::Tui => "tui",
            Self::Cli => "cli",
            Self::Api => "api",
        }
    }
}

/// A chat platform supplied by an installed adapter plugin.
///
/// One binding owns a whole platform account: every group, private chat and
/// channel that account can see. `id` is the adapter's canonical platform id
/// (for example `telegram`), and `account` distinguishes multiple accounts of
/// the same platform.
#[derive(
    Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct ChatPlatform {
    pub id: String,
    pub account: String,
}

/// Where a message came from: a local surface, or an installed chat platform.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "transport", rename_all = "snake_case")]
pub enum Platform {
    Local { surface: LocalSurface },
    Chat { platform: ChatPlatform },
}

impl Platform {
    pub fn local(surface: LocalSurface) -> Self {
        Self::Local { surface }
    }

    pub fn chat(id: impl Into<String>, account: impl Into<String>) -> Self {
        Self::Chat {
            platform: ChatPlatform {
                id: id.into(),
                account: account.into(),
            },
        }
    }

    /// True only for the locally authenticated surfaces.
    pub fn is_local(&self) -> bool {
        matches!(self, Self::Local { .. })
    }

    pub fn local_surface(&self) -> Option<LocalSurface> {
        match self {
            Self::Local { surface } => Some(*surface),
            Self::Chat { .. } => None,
        }
    }

    pub fn chat_platform(&self) -> Option<&ChatPlatform> {
        match self {
            Self::Local { .. } => None,
            Self::Chat { platform } => Some(platform),
        }
    }
}

/// The shape of the conversation a message was sent in.
///
/// This is a property of the message, not of the binding: one platform account
/// serves direct chats, groups and channels at the same time.
/// The shape of the conversation a message was sent in.
///
/// This is a property of the message, not of the binding: one platform account
/// serves direct chats, multi-party rooms and broadcast channels at the same
/// time. Four mutually exclusive values cover every adapter surveyed:
///
/// - `Direct` — a one-to-one conversation (Telegram private, WeCom single,
///   Weixin p2p, QQ c2c, DingTalk type 1, WhatsApp/Signal/iMessage/mail DM).
/// - `Group` — a named, multi-party, interactive room. Group DMs (Slack MPIM,
///   Discord group DM, QQ temporary C2C from a group) and interactive channels
///   (Slack channel, Discord guild text, Feishu group) belong here because the
///   assistant behaves identically: shared transcript, mention-gated trigger,
///   several readers.
/// - `Broadcast` — a one-to-many surface where posting is restricted to
///   owners/admins (Telegram channel, WhatsApp newsletter, Discord
///   announcement). The assistant may be unable to reply at all.
///
/// Visibility (public vs private) is deliberately *not* a kind: only Slack and
/// Discord private threads expose it, and no policy branches on it. Threads are
/// a separate axis carried by the message's `thread` field, never folded into
/// this enum.
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
pub enum ConversationKind {
    #[default]
    Direct,
    Group,
    Broadcast,
}

impl ConversationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Direct => "direct",
            Self::Group => "group",
            Self::Broadcast => "broadcast",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "direct" => Some(Self::Direct),
            "group" => Some(Self::Group),
            "broadcast" => Some(Self::Broadcast),
            _ => None,
        }
    }

    /// Threads only make sense inside a multi-party room.
    pub fn supports_threads(self) -> bool {
        !matches!(self, Self::Direct)
    }
}

/// A system-produced author. These never carry external authority.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SystemAuthorKind {
    /// Automated report session (`@service`).
    AutomationReport,
    /// Shared chat room (`@shared`); the concrete speaker is on the message.
    SharedChat,
    /// A delegated child agent's result.
    Subagent,
    /// An integration or automation notice.
    Integration,
    /// The operator Bot driving another session.
    Bot,
}

/// The verified identity of a message's sender, and the authority it implies.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuthorKind {
    /// The local instance operator. Only the local authenticated surfaces can
    /// produce this; no adapter payload can claim it.
    InstanceOperator,
    /// An externally authenticated chat participant. `role` is resolved from
    /// `channel_policy.members` on the host, never from platform metadata.
    ChannelMember { role: ChannelRole },
    /// A host-produced author (automation, subagent, Bot instruction).
    System { source: SystemAuthorKind },
}

impl AuthorKind {
    pub fn is_instance_operator(&self) -> bool {
        matches!(self, Self::InstanceOperator)
    }

    pub fn channel_role(&self) -> Option<ChannelRole> {
        match self {
            Self::ChannelMember { role } => Some(*role),
            _ => None,
        }
    }

    pub fn is_system(&self) -> bool {
        matches!(self, Self::System { .. })
    }
}

/// The sender of one message.
///
/// `display_name` is presentation only and must never be consulted for
/// authorization: a user can rename themselves on most platforms.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageAuthor {
    /// Stable identifier inside the platform.
    pub id: String,
    /// Human-readable name for display and prompt context.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    pub kind: AuthorKind,
}

impl MessageAuthor {
    pub fn instance_operator() -> Self {
        Self {
            id: "instance_operator".into(),
            display_name: None,
            kind: AuthorKind::InstanceOperator,
        }
    }

    pub fn channel_member(
        actor: impl Into<String>,
        role: ChannelRole,
        display_name: Option<String>,
    ) -> Self {
        Self {
            id: actor.into(),
            display_name,
            kind: AuthorKind::ChannelMember { role },
        }
    }

    pub fn system(source: SystemAuthorKind) -> Self {
        Self {
            id: format!("system:{}", system_slug(source)),
            display_name: None,
            kind: AuthorKind::System { source },
        }
    }

    /// True when this author holds instance-level authority.
    pub fn is_instance_operator(&self) -> bool {
        self.kind.is_instance_operator()
    }
}

fn system_slug(source: SystemAuthorKind) -> &'static str {
    match source {
        SystemAuthorKind::AutomationReport => "automation_report",
        SystemAuthorKind::SharedChat => "shared_chat",
        SystemAuthorKind::Subagent => "subagent",
        SystemAuthorKind::Integration => "integration",
        SystemAuthorKind::Bot => "bot",
    }
}

/// Where a message physically arrived: platform, exact external conversation,
/// and optional thread.
///
/// `conversation` is the adapter's own stable identifier for the room. Together
/// with the binding it selects exactly one session, so a group, a private chat
/// and a channel never share a transcript.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MessageOrigin {
    pub platform: Platform,
    /// Exact external conversation id (room / DM / channel id).
    pub conversation: String,
    pub kind: ConversationKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<String>,
}

impl MessageOrigin {
    pub fn local(surface: LocalSurface, conversation: impl Into<String>) -> Self {
        Self {
            platform: Platform::local(surface),
            conversation: conversation.into(),
            kind: ConversationKind::Direct,
            thread: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operator_identity_is_only_produced_locally() {
        let author = MessageAuthor::instance_operator();
        assert!(author.is_instance_operator());
        assert_eq!(author.kind.channel_role(), None);
        assert!(Platform::local(LocalSurface::Web).is_local());
        assert!(!Platform::chat("telegram", "bot-1").is_local());
    }

    #[test]
    fn channel_member_carries_its_resolved_role() {
        let author = MessageAuthor::channel_member("u1", ChannelRole::Operator, Some("Ann".into()));
        assert!(!author.is_instance_operator());
        assert_eq!(author.kind.channel_role(), Some(ChannelRole::Operator));
        assert_eq!(author.display_name.as_deref(), Some("Ann"));
    }

    #[test]
    fn conversation_kind_gates_threads() {
        assert!(!ConversationKind::Direct.supports_threads());
        assert!(ConversationKind::Group.supports_threads());
        assert!(ConversationKind::Broadcast.supports_threads());
    }

    #[test]
    fn message_origin_round_trips_both_transports() {
        let local = MessageOrigin::local(LocalSurface::Tui, "session-1");
        let chat = MessageOrigin {
            platform: Platform::chat("feishu", "app-1"),
            conversation: "oc_123".into(),
            kind: ConversationKind::Group,
            thread: Some("t-9".into()),
        };
        for value in [local, chat] {
            let encoded = serde_json::to_value(&value).unwrap();
            let decoded: MessageOrigin = serde_json::from_value(encoded).unwrap();
            assert_eq!(decoded, value);
        }
    }
}
