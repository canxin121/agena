mod event;
#[cfg(test)]
mod projection;
mod store;
mod transcript;
#[allow(dead_code)]
mod turn_buffer;
#[cfg(test)]
mod view;

pub(crate) use crate::session::ids::{MessageId, ToolCallId, TurnId};
pub(crate) use event::{
    AssistantMessageCompleted, FinishReason, SystemNoticeAppended, SystemNoticeKind,
    ToolCallCompleted, ToolCallIssued, TurnAbortReason, TurnAborted, TurnCompleted, TurnStarted,
    UserMessageAppended,
};
pub use event::{RewindCheckpoint, RewindCheckpointEntry};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use projection::{HistoryFold, fold_history};
pub use store::ProjectedMessageHeader;
pub(crate) use store::SessionHistoryStore;
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use transcript::ProviderTranscriptBuilder;
pub(crate) use transcript::{
    ProviderTranscript, TranscriptBlock, TranscriptContent, TranscriptFragment, TranscriptToolCall,
    TranscriptToolOutput,
};
#[allow(unused_imports)]
pub(crate) use turn_buffer::{
    MessageIdAllocator, SequentialIdAllocator, TurnBuffer, TurnBufferError,
};
#[cfg(test)]
#[allow(unused_imports)]
pub(crate) use view::{SessionView, SessionViewBuilder, SessionViewError};
