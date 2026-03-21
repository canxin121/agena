mod command;
mod id;
mod item;
mod part_update;
mod reducer;
mod thread;

use serde::{Deserialize, Serialize};

pub use command::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, CommandOutputStream,
};
pub use id::{CallId, ItemId, MessageId, PartId, ThreadId, TurnId};
pub use item::{ItemCompletedEvent, ItemRef, ItemSnapshot, ItemStartedEvent, ItemUpdatedEvent};
pub use part_update::{MessagePartDeltaEvent, MessagePartUpdatedEvent, PartDeltaField};
pub use reducer::{AiStreamEvent, MessageReducer};
pub use thread::{
    ErrorInfo, StreamErrorEvent, ThreadStartedEvent, TurnCompletedEvent, TurnFailedEvent,
    TurnStartedEvent,
};

/// Unified runtime event stream combining codex-style lifecycle + command deltas
/// and opencode-style part-level updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    ThreadStarted(ThreadStartedEvent),
    TurnStarted(TurnStartedEvent),
    TurnCompleted(TurnCompletedEvent),
    TurnFailed(TurnFailedEvent),

    ItemStarted(ItemStartedEvent),
    ItemUpdated(ItemUpdatedEvent),
    ItemCompleted(ItemCompletedEvent),

    MessagePartUpdated(MessagePartUpdatedEvent),
    MessagePartDelta(MessagePartDeltaEvent),

    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),

    StreamError(StreamErrorEvent),
}
