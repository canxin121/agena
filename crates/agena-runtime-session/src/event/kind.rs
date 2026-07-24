//! Unified [`EventKind`] enum — the single source of truth for every event
//! flowing on the agena bus. Variant ordering only matters for serde tag
//! stability — `kind` strings are a coordinated wire contract shared by core,
//! API clients, and UIs (see [`ALL_KINDS`]).

use agena_domain::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, EventEnvelope, EventKindTag,
    ExecutionFinishedEvent, ExecutionStartedEvent, KindMatcher, KindPersistence,
    MessagePartDeltaEvent, PermissionRepliedEvent, PermissionRequestedEvent, PermissionRuleEvent,
    StreamErrorEvent, SubtaskStatusChangedEvent,
};
use agena_plugin_sdk::PluginKey;
use serde::{Deserialize, Serialize};

use crate::event::client::MessagePartCheckpointedEvent;
pub type PluginToolRegistryChangedEvent =
    agena_plugin_host::sdk::host_api::ToolRegistryChangedEvent;
use crate::session::history::{
    AssistantMessageFinished, RunAborted, RunCompleted, RunStarted, SystemNoticeAppended,
    ToolCallCompleted, ToolCallIssued, UserMessageAppended,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum EventKind {
    // --- runtime / UI projection ---
    ExecutionStarted(ExecutionStartedEvent),
    ExecutionFinished(ExecutionFinishedEvent),
    SubtaskStatusChanged(SubtaskStatusChangedEvent),
    StreamError(StreamErrorEvent),
    MessagePartCheckpointed(MessagePartCheckpointedEvent),
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
    AssistantMessageFinished(AssistantMessageFinished),
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
    pub plugin_id: PluginKey,
    pub kind_label: String,
    pub payload: serde_json::Value,
}

impl EventKind {
    pub fn tag_str(&self) -> &'static str {
        match self {
            Self::ExecutionStarted(_) => "execution_started",
            Self::ExecutionFinished(_) => "execution_finished",
            Self::SubtaskStatusChanged(_) => "subtask_status_changed",
            Self::StreamError(_) => "stream_error",
            Self::MessagePartCheckpointed(_) => "message_part_checkpointed",
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
            Self::AssistantMessageFinished(_) => "assistant_message_finished",
            Self::ToolCallIssued(_) => "tool_call_issued",
            Self::ToolCallCompleted(_) => "tool_call_completed",
            Self::SystemNoticeAppended(_) => "system_notice_appended",
            Self::PluginEvent(_) => "plugin_event",
            Self::PluginToolRegistryChanged(_) => "plugin_tool_registry_changed",
        }
    }

    /// Returns `true` for events that must be written to the persistent event
    /// log. UI-only events (streaming deltas and command output) are
    /// ephemeral: they are broadcast in-process but never written to SQLite.
    pub fn is_persistent(&self) -> bool {
        !matches!(
            self,
            Self::StreamError(_)
                | Self::MessagePartDelta(_)
                | Self::CommandBegin(_)
                | Self::CommandOutputDelta(_)
                | Self::CommandEnd(_)
        )
    }

    /// Whether this child-session event can change an ancestor session's
    /// projected execution state.
    ///
    /// Ancestor UIs must treat these as invalidation signals and re-read the
    /// ancestor projection. They must not reduce the child event into the
    /// ancestor transcript.
    pub fn invalidates_ancestor_projection(&self) -> bool {
        matches!(
            self,
            Self::PermissionRequested(_)
                | Self::PermissionReplied(_)
                | Self::SubtaskStatusChanged(_)
                | Self::ExecutionFinished(_)
        ) || matches!(
            self,
            Self::MessagePartCheckpointed(event)
                if matches!(event.part.content, Some(crate::message::PartContent::Request(_)))
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

/// Persistent history kind tags (written to SQLite and replayable).
#[cfg(test)]
pub const HISTORY_KINDS: &[&str] = &[
    "execution_started",
    "execution_finished",
    "subtask_status_changed",
    "message_part_checkpointed",
    "permission_requested",
    "permission_replied",
    "permission_rule_created",
    "permission_rule_updated",
    "permission_rule_revoked",
    "run_started",
    "run_completed",
    "run_aborted",
    "user_message_appended",
    "assistant_message_finished",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
    "plugin_tool_registry_changed",
];

/// Stable list of every known kind tag (UI + history). Order matches the
/// serde tag ordering in `EventKind`.
#[cfg(test)]
pub const ALL_KINDS: &[&str] = &[
    "execution_started",
    "execution_finished",
    "subtask_status_changed",
    "stream_error",
    "message_part_checkpointed",
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
    "assistant_message_finished",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
    "plugin_tool_registry_changed",
];

/// Concrete `DomainEvent` envelope specialised for agena's `EventKind`.
pub type DomainEvent = EventEnvelope<EventKind>;

/// Concrete `EventPublisher` specialised for agena's `EventKind`.
pub type EventPublisher = crate::event::publisher::EventPublisher<EventKind>;

pub use crate::event::publisher::PublishContext;

#[cfg(test)]
mod tests {
    use super::{ALL_KINDS, HISTORY_KINDS};
    use agena_domain::MESSAGE_CREATED_EVENT_KIND_TAGS;

    #[test]
    fn message_creation_kinds_are_known_persistent_events() {
        for kind in MESSAGE_CREATED_EVENT_KIND_TAGS {
            assert!(ALL_KINDS.contains(kind), "unknown message kind: {kind}");
            assert!(
                HISTORY_KINDS.contains(kind),
                "message kind must be persistent: {kind}"
            );
        }
        assert!(!MESSAGE_CREATED_EVENT_KIND_TAGS.contains(&"tool_call_completed"));
    }
}
