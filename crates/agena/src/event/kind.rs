//! Unified [`EventKind`] enum — the single source of truth for every event
//! flowing on the agena bus. Variant ordering only matters for serde tag
//! stability — `kind` strings (snake_case of the variant name) are part of
//! the wire protocol and must not be renamed without a versioning ceremony
//! (see [`ALL_KINDS`]).

use crate::event::filter::{EventKindTag, KindMatcher, KindPersistence};
use serde::{Deserialize, Serialize};

use crate::event::client::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, ExecutionFailedEvent,
    ExecutionStartedEvent, MessagePartDeltaEvent, MessagePartUpdatedEvent, PermissionRepliedEvent,
    PermissionRequestedEvent, PermissionRuleEvent, StreamErrorEvent,
};
pub type PluginToolRegistryChangedEvent = crate::plugin::sdk::host_api::ToolRegistryChangedEvent;
use crate::session::history::{
    AssistantMessageCompleted, RunAborted, RunCompleted, RunStarted, SystemNoticeAppended,
    ToolCallCompleted, ToolCallIssued, UserMessageAppended,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum EventKind {
    // --- runtime / UI projection ---
    ExecutionStarted(ExecutionStartedEvent),
    ExecutionFailed(ExecutionFailedEvent),
    StreamError(StreamErrorEvent),
    MessagePartUpdated(MessagePartUpdatedEvent),
    MessagePartDelta(MessagePartDeltaEvent),
    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),
    PermissionRequested(PermissionRequestedEvent),
    PermissionReplied(PermissionRepliedEvent),
    PermissionRuleCreated(PermissionRuleEvent),
    PermissionRuleUpdated(PermissionRuleEvent),
    PermissionRuleRevoked(PermissionRuleEvent),

    // --- append-only history ---
    RunStarted(RunStarted),
    RunCompleted(RunCompleted),
    RunAborted(RunAborted),
    UserMessageAppended(UserMessageAppended),
    AssistantMessageCompleted(AssistantMessageCompleted),
    ToolCallIssued(ToolCallIssued),
    ToolCallCompleted(ToolCallCompleted),
    SystemNoticeAppended(SystemNoticeAppended),

    // --- plugin-injected synthetic events ---
    /// Free-form payload published by a plugin via `host/event.publish`.
    /// `kind_label` carries the plugin's intended kind name so subscribers
    /// can filter by it; `payload` is opaque JSON.
    PluginEvent(PluginEventPayload),
    /// Structured runtime event emitted when the plugin tool registry changes.
    PluginToolRegistryChanged(PluginToolRegistryChangedEvent),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PluginEventPayload {
    pub plugin_id: String,
    pub kind_label: String,
    pub payload: serde_json::Value,
}

impl EventKind {
    pub fn tag_str(&self) -> &'static str {
        match self {
            Self::ExecutionStarted(_) => "execution_started",
            Self::ExecutionFailed(_) => "execution_failed",
            Self::StreamError(_) => "stream_error",
            Self::MessagePartUpdated(_) => "message_part_updated",
            Self::MessagePartDelta(_) => "message_part_delta",
            Self::CommandBegin(_) => "command_begin",
            Self::CommandOutputDelta(_) => "command_output_delta",
            Self::CommandEnd(_) => "command_end",
            Self::PermissionRequested(_) => "permission_requested",
            Self::PermissionReplied(_) => "permission_replied",
            Self::PermissionRuleCreated(_) => "permission_rule_created",
            Self::PermissionRuleUpdated(_) => "permission_rule_updated",
            Self::PermissionRuleRevoked(_) => "permission_rule_revoked",
            Self::RunStarted(_) => "run_started",
            Self::RunCompleted(_) => "run_completed",
            Self::RunAborted(_) => "run_aborted",
            Self::UserMessageAppended(_) => "user_message_appended",
            Self::AssistantMessageCompleted(_) => "assistant_message_completed",
            Self::ToolCallIssued(_) => "tool_call_issued",
            Self::ToolCallCompleted(_) => "tool_call_completed",
            Self::SystemNoticeAppended(_) => "system_notice_appended",
            Self::PluginEvent(_) => "plugin_event",
            Self::PluginToolRegistryChanged(_) => "plugin_tool_registry_changed",
        }
    }

    /// Returns `true` for events that must be written to the persistent event
    /// log. UI-only events (streaming deltas, run lifecycle signals) are
    /// ephemeral: they are broadcast in-process but never written to SQLite.
    pub fn is_persistent(&self) -> bool {
        !matches!(
            self,
            Self::ExecutionStarted(_)
                | Self::ExecutionFailed(_)
                | Self::StreamError(_)
                | Self::MessagePartDelta(_)
                | Self::CommandBegin(_)
                | Self::CommandOutputDelta(_)
                | Self::CommandEnd(_)
        )
    }
}

impl KindMatcher for EventKind {
    fn tag(&self) -> EventKindTag {
        EventKindTag::from(self.tag_str())
    }
}

impl KindPersistence for EventKind {
    fn is_persistent(&self) -> bool {
        EventKind::is_persistent(self)
    }
}

/// Ephemeral UI-only kind tags (never written to the event store).
pub const UI_KINDS: &[&str] = &[
    "execution_started",
    "execution_failed",
    "stream_error",
    "message_part_delta",
    "command_begin",
    "command_output_delta",
    "command_end",
];

/// Persistent history kind tags (written to SQLite and replayable).
pub const HISTORY_KINDS: &[&str] = &[
    "message_part_updated",
    "permission_requested",
    "permission_replied",
    "permission_rule_created",
    "permission_rule_updated",
    "permission_rule_revoked",
    "run_started",
    "run_completed",
    "run_aborted",
    "user_message_appended",
    "assistant_message_completed",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
    "plugin_tool_registry_changed",
];

/// Stable list of every known kind tag (UI + history). Order matches the
/// serde tag ordering in `EventKind`.
pub const ALL_KINDS: &[&str] = &[
    "execution_started",
    "execution_failed",
    "stream_error",
    "message_part_updated",
    "message_part_delta",
    "command_begin",
    "command_output_delta",
    "command_end",
    "permission_requested",
    "permission_replied",
    "permission_rule_created",
    "permission_rule_updated",
    "permission_rule_revoked",
    "run_started",
    "run_completed",
    "run_aborted",
    "user_message_appended",
    "assistant_message_completed",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
    "plugin_tool_registry_changed",
];

/// Concrete `DomainEvent` envelope specialised for agena's `EventKind`.
pub type DomainEvent = crate::event::envelope::DomainEvent<EventKind>;

/// Concrete `EventPublisher` specialised for agena's `EventKind`.
pub type EventPublisher = crate::event::publisher::EventPublisher<EventKind>;

pub use crate::event::publisher::PublishContext;
