//! Unified [`EventKind`] enum — the single source of truth for every event
//! flowing on the agena bus. Variant ordering only matters for serde tag
//! stability — `kind` strings (snake_case of the variant name) are part of
//! the wire protocol and must not be renamed without a versioning ceremony
//! (see [`ALL_KINDS`]).

use crate::event::filter::{EventKindTag, KindMatcher, KindPersistence};
use serde::{Deserialize, Serialize};

use crate::event::client::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, MessagePartDeltaEvent,
    MessagePartUpdatedEvent, PermissionRepliedEvent, PermissionRequestedEvent, PermissionRuleEvent,
    RunFailedEvent, RunStartedEvent, SessionGoalEvent, StreamErrorEvent,
};
use crate::session::history::{
    AssistantMessageCompleted, SystemNoticeAppended, ToolCallCompleted, ToolCallIssued,
    TurnAborted, TurnCompleted, TurnStarted, UserMessageAppended,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[allow(clippy::large_enum_variant)]
#[serde(tag = "kind", content = "payload", rename_all = "snake_case")]
pub enum EventKind {
    // --- runtime / UI projection ---
    RunStarted(RunStartedEvent),
    RunFailed(RunFailedEvent),
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
    SessionGoalUpdated(SessionGoalEvent),

    // --- append-only history ---
    TurnStarted(TurnStarted),
    TurnCompleted(TurnCompleted),
    TurnAborted(TurnAborted),
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
            Self::RunStarted(_) => "run_started",
            Self::RunFailed(_) => "run_failed",
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
            Self::SessionGoalUpdated(_) => "session_goal_updated",
            Self::TurnStarted(_) => "turn_started",
            Self::TurnCompleted(_) => "turn_completed",
            Self::TurnAborted(_) => "turn_aborted",
            Self::UserMessageAppended(_) => "user_message_appended",
            Self::AssistantMessageCompleted(_) => "assistant_message_completed",
            Self::ToolCallIssued(_) => "tool_call_issued",
            Self::ToolCallCompleted(_) => "tool_call_completed",
            Self::SystemNoticeAppended(_) => "system_notice_appended",
            Self::PluginEvent(_) => "plugin_event",
        }
    }

    /// Returns `true` for events that must be written to the persistent event
    /// log. UI-only events (streaming deltas, run lifecycle signals) are
    /// ephemeral: they are broadcast in-process but never written to SQLite.
    pub fn is_persistent(&self) -> bool {
        !matches!(
            self,
            Self::RunStarted(_)
                | Self::RunFailed(_)
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
    "run_started",
    "run_failed",
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
    "session_goal_updated",
    "turn_started",
    "turn_completed",
    "turn_aborted",
    "user_message_appended",
    "assistant_message_completed",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
];

/// Stable list of every known kind tag (UI + history). Order matches the
/// serde tag ordering in `EventKind`.
pub const ALL_KINDS: &[&str] = &[
    "run_started",
    "run_failed",
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
    "session_goal_updated",
    "turn_started",
    "turn_completed",
    "turn_aborted",
    "user_message_appended",
    "assistant_message_completed",
    "tool_call_issued",
    "tool_call_completed",
    "system_notice_appended",
    "plugin_event",
];

/// Concrete `DomainEvent` envelope specialised for agena's `EventKind`.
pub type DomainEvent = crate::event::envelope::DomainEvent<EventKind>;

/// Concrete `EventPublisher` specialised for agena's `EventKind`.
pub type EventPublisher = crate::event::publisher::EventPublisher<EventKind>;

pub use crate::event::publisher::PublishContext;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_serde_for_simple_variant() {
        let kind = EventKind::RunStarted(RunStartedEvent {
            session_id: 1,
            ts_ms: 42,
        });
        let value = serde_json::to_value(&kind).unwrap();
        assert_eq!(value["kind"], "run_started");
        let back: EventKind = serde_json::from_value(value).unwrap();
        assert_eq!(back, kind);
    }

    #[test]
    fn tag_matches_table() {
        let kind = EventKind::RunStarted(RunStartedEvent {
            session_id: 1,
            ts_ms: 0,
        });
        assert_eq!(kind.tag().as_str(), "run_started");
        assert!(ALL_KINDS.contains(&"run_started"));
    }

    #[test]
    fn ui_and_history_kinds_partition_all_kinds() {
        let all: std::collections::HashSet<&str> = ALL_KINDS.iter().copied().collect();
        let ui: std::collections::HashSet<&str> = UI_KINDS.iter().copied().collect();
        let history: std::collections::HashSet<&str> = HISTORY_KINDS.iter().copied().collect();
        // No overlap
        assert!(
            ui.is_disjoint(&history),
            "UI and history kinds must not overlap"
        );
        // Together they cover all kinds
        let union: std::collections::HashSet<&str> = ui.union(&history).copied().collect();
        assert_eq!(union, all, "UI ∪ history must equal all kinds");
    }

    #[test]
    fn is_persistent_matches_history_kinds_table() {
        let ui_set: std::collections::HashSet<&str> = UI_KINDS.iter().copied().collect();
        // Every EventKind variant: persistent ↔ not in ui_set
        let samples: &[(&str, bool)] = &[
            ("run_started", false),
            ("message_part_delta", false),
            ("command_output_delta", false),
            ("turn_started", true),
            ("user_message_appended", true),
            ("plugin_event", true),
        ];
        for (tag, expected_persistent) in samples {
            let in_ui = ui_set.contains(*tag);
            assert_eq!(
                !in_ui, *expected_persistent,
                "tag {tag}: is_persistent should be {expected_persistent}"
            );
        }
    }
}
