//! Unified [`EventKind`] enum — the single source of truth for every event
//! flowing on the agena bus. Variant ordering only matters for serde tag
//! stability — `kind` strings (snake_case of the variant name) are part of
//! the wire protocol and must not be renamed without a versioning ceremony
//! (see [`kinds_table`]).

use agena_event::KindMatcher;
use agena_event::filter::EventKindTag;
use serde::{Deserialize, Serialize};

use crate::event::client::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, MessagePartDeltaEvent,
    MessagePartUpdatedEvent, RunFailedEvent, RunStartedEvent, StreamErrorEvent,
};
use crate::session::history::{
    AssistantMessageCompleted, MessageRevised, SystemNoticeAppended, ToolCallCompleted,
    ToolCallIssued, TurnAborted, TurnCompleted, TurnStarted, UserMessageAppended,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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

    // --- append-only history ---
    TurnStarted(TurnStarted),
    TurnCompleted(TurnCompleted),
    TurnAborted(TurnAborted),
    UserMessageAppended(UserMessageAppended),
    AssistantMessageCompleted(AssistantMessageCompleted),
    ToolCallIssued(ToolCallIssued),
    ToolCallCompleted(ToolCallCompleted),
    SystemNoticeAppended(SystemNoticeAppended),
    MessageRevised(MessageRevised),

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
            Self::TurnStarted(_) => "turn_started",
            Self::TurnCompleted(_) => "turn_completed",
            Self::TurnAborted(_) => "turn_aborted",
            Self::UserMessageAppended(_) => "user_message_appended",
            Self::AssistantMessageCompleted(_) => "assistant_message_completed",
            Self::ToolCallIssued(_) => "tool_call_issued",
            Self::ToolCallCompleted(_) => "tool_call_completed",
            Self::SystemNoticeAppended(_) => "system_notice_appended",
            Self::MessageRevised(_) => "message_revised",
            Self::PluginEvent(_) => "plugin_event",
        }
    }
}

impl KindMatcher for EventKind {
    fn tag(&self) -> EventKindTag {
        EventKindTag::from(self.tag_str())
    }
}

/// Stable list of every known kind tag. Useful for documentation and clients
/// that want to subscribe to "all known kinds".
pub fn kinds_table() -> &'static [&'static str] {
    &[
        "run_started",
        "run_failed",
        "stream_error",
        "message_part_updated",
        "message_part_delta",
        "command_begin",
        "command_output_delta",
        "command_end",
        "turn_started",
        "turn_completed",
        "turn_aborted",
        "user_message_appended",
        "assistant_message_completed",
        "tool_call_issued",
        "tool_call_completed",
        "system_notice_appended",
        "message_revised",
        "plugin_event",
    ]
}

/// Concrete `DomainEvent` envelope specialised for agena's `EventKind`.
pub type DomainEvent = agena_event::DomainEvent<EventKind>;

/// Concrete `EventPublisher` specialised for agena's `EventKind`.
pub type EventPublisher = agena_event::EventPublisher<EventKind>;

/// Re-export so call sites can write `event::PublishContext` instead of
/// `agena_event::publisher::PublishContext`.
pub use agena_event::publisher::PublishContext;

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
        assert!(kinds_table().contains(&"run_started"));
    }
}
