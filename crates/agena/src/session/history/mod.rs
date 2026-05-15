mod event;
mod projection;
mod store;
#[allow(dead_code)]
mod transcript;
#[allow(dead_code)]
mod turn_buffer;
mod view;

pub(crate) use crate::session::ids::{MessageId, ToolCallId, TurnId};
pub(crate) use event::{
    AssistantMessageCompleted, FinishReason, MessageRevised, RevisionKind, SystemNoticeAppended,
    SystemNoticeKind, ToolCallCompleted, ToolCallIssued, TurnAbortReason, TurnAborted,
    TurnCompleted, TurnStarted, UserMessageAppended,
};
pub use event::{RewindCheckpoint, RewindCheckpointEntry};
#[allow(unused_imports)]
pub(crate) use projection::{HistoryFold, fold_history};
pub use store::ProjectedMessageHeader;
pub(crate) use store::SessionHistoryStore;
#[allow(unused_imports)]
pub(crate) use transcript::{
    ProviderTranscript, ProviderTranscriptBuilder, ProviderTranscriptError, TranscriptBlock,
    TranscriptContent, TranscriptFragment, TranscriptToolCall, TranscriptToolOutput,
};
#[allow(unused_imports)]
pub(crate) use turn_buffer::{
    MessageIdAllocator, SequentialIdAllocator, TurnBuffer, TurnBufferError,
};
#[allow(unused_imports)]
pub(crate) use view::{SessionView, SessionViewBuilder, SessionViewError};

/// Convenience: fold a slice of [`crate::event::DomainEvent`]s into a
/// [`SessionView`]. Used by the store to project the persisted log into
/// in-memory messages.
pub(crate) fn fold_session_view(
    events: &[crate::event::DomainEvent],
) -> Result<view::SessionView, view::SessionViewError> {
    fold_history::<SessionViewBuilder>(events).map_err(|err: view::SessionViewError| err)?
}
