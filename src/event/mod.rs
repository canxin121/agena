mod command;
mod part_update;
mod reducer;
mod thread;

use sea_orm::FromJsonQueryResult;
use serde::{Deserialize, Serialize};

pub use command::{
    CommandBeginEvent, CommandEndEvent, CommandOutputDeltaEvent, CommandOutputStream,
};
pub use part_update::{MessagePartDeltaEvent, MessagePartUpdatedEvent, PartDeltaField};
pub use reducer::{AiStreamEvent, MessageReducer};
pub use thread::{ErrorInfo, StreamErrorEvent, ThreadFailedEvent, ThreadStartedEvent};

/// Unified runtime event stream combining codex-style lifecycle + command deltas
/// and opencode-style part-level updates.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, FromJsonQueryResult)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum AgentEvent {
    ThreadStarted(ThreadStartedEvent),
    ThreadFailed(ThreadFailedEvent),

    MessagePartUpdated(MessagePartUpdatedEvent),
    MessagePartDelta(MessagePartDeltaEvent),

    CommandBegin(CommandBeginEvent),
    CommandOutputDelta(CommandOutputDeltaEvent),
    CommandEnd(CommandEndEvent),

    StreamError(StreamErrorEvent),
}
