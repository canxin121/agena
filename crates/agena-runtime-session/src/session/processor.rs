use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::event::{
    EventKind, EventPublisher, MessagePartCheckpointedEvent, PublishContext,
    TranscriptPartUpsertedEvent,
};
use crate::message::{
    Message, MessageMetadata, MessagePart, MessageProviderState, OperationPart, PartContent,
};
use crate::provider::ProviderRegistry;
use agena_domain::ModelRef;
use agena_domain::{
    AssistantReasoningField, ExecutionStatus, FinishReason, MessageSource, Role, StreamErrorEvent,
    StructuredObject, TimeRange, ToolInvocation,
};
use agena_provider::CompletionFinishReason;
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;

use super::history::{MessageId as HistoryMessageId, RunBuffer, RunId, ToolCallId};
use super::{ContextGovernor, store::ProcessorPartIdAllocator};

const REASONING_PLACEHOLDER: &str = "(no reasoning recorded)";

#[derive(Clone)]
pub(crate) struct SessionRunRequest {
    pub run_id: RunId,
    pub execution_id: agena_domain::ExecutionId,
    pub turn_id: agena_domain::TurnId,
    pub reply_id: agena_domain::AssistantReplyId,
    pub session_id: i64,
    /// Internal provider/model turn that owns this assistant round. This is
    /// not the canonical UUID `turn_id`; `None` allocates from the model
    /// message id space.
    pub model_turn_id: Option<i64>,
    /// Last persisted conversation message before provider-input projection.
    pub completion_parent_message_id: Option<i64>,
    pub model: ModelRef,
    pub model_thinking_mode: Option<String>,
    pub model_speed_mode: Option<String>,
    pub completion: CompletionRequest,
    pub next_message_id: i64,
    pub part_ids: ProcessorPartIdAllocator,
    pub next_call_id: i64,
    /// Live publisher used to push streaming events ("running") onto the
    /// unified bus while the run is in flight. `None` keeps test harnesses
    /// terse — they observe the buffered `client_events` on the result.
    pub event_publisher: Option<Arc<EventPublisher>>,
    /// Optional cancel handle. When the token fires the stream loop
    /// terminates between provider events and surfaces a `RunAbortReason::
    /// Cancelled`-shaped terminal error. `None` runs the turn to completion.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

#[derive(Debug)]
pub(crate) struct SessionRunResult {
    pub assistant_message_id: i64,
    pub state: Vec<Message>,
    /// UI-projection events buffered during the run (also pushed onto the
    /// bus when `event_publisher` was set).
    pub client_events: Vec<EventKind>,
    pub provider_metadata: Option<serde_json::Value>,
    pub termination: SessionRunTermination,
    /// Append-only history events emitted by the run buffer. Routed by the
    /// manager into `SessionStore::append_history_items`.
    pub history_items: Vec<EventKind>,
    /// The run id used by `history_items` — the manager wraps this with
    /// `RunStarted` / `RunCompleted` / `RunAborted` boundary events.
    pub run_id: RunId,
    /// True when the provider terminal event carried an explicit
    /// `end_turn=false` signal: the model asked for another turn even though
    /// it did not request any tool call.
    pub follow_up_requested: bool,
    /// Normalized terminal finish reason for the model turn. Defaults to
    /// `Stop` when the provider stream did not report a terminal reason.
    pub finish_reason: FinishReason,
}

#[derive(Debug)]
pub(crate) enum SessionRunTermination {
    Completed,
    Cancelled,
    Failed(AppError),
}

#[derive(Clone)]
pub struct SessionProcessor {
    provider_registry: Arc<ProviderRegistry>,
    context_governor: ContextGovernor,
    plugins: Arc<agena_plugin_host::PluginHost>,
    workspace_root: PathBuf,
}

mod events;
mod helpers;
mod media;
mod parts;
mod run;
mod tool_call_helpers;
mod tool_calls;

pub(crate) use self::helpers::*;
pub(crate) use self::media::*;
pub(crate) use self::tool_call_helpers::*;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{PendingToolCall, pending_tool_call_stream_key};

    #[test]
    fn provider_call_id_merges_changing_adapter_stream_keys() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();

        let first =
            pending_tool_call_stream_key(&mut pending, "idx:0".to_string(), Some("call_shared"));
        assert_eq!(first, "id:call_shared");
        pending.insert(
            first.clone(),
            PendingToolCall {
                id: Some("call_shared".to_string()),
                ..Default::default()
            },
        );

        let replay =
            pending_tool_call_stream_key(&mut pending, "idx:6".to_string(), Some("call_shared"));
        assert_eq!(replay, first);
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn distinct_provider_call_ids_do_not_merge_even_when_stream_key_repeats() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();

        let first =
            pending_tool_call_stream_key(&mut pending, "idx:0".to_string(), Some("call_one"));
        pending.insert(
            first.clone(),
            PendingToolCall {
                id: Some("call_one".to_string()),
                ..Default::default()
            },
        );

        let second =
            pending_tool_call_stream_key(&mut pending, "idx:0".to_string(), Some("call_two"));
        assert_ne!(first, second);
        pending.insert(
            second.clone(),
            PendingToolCall {
                id: Some("call_two".to_string()),
                ..Default::default()
            },
        );

        assert_eq!(pending.len(), 2);
        assert!(pending.contains_key(first.as_str()));
        assert!(pending.contains_key(second.as_str()));
    }

    #[test]
    fn provider_id_rekeys_an_earlier_idless_stream_without_a_second_call() {
        let mut pending = BTreeMap::<String, PendingToolCall>::new();
        pending.insert("idx:0".to_string(), PendingToolCall::default());

        let key =
            pending_tool_call_stream_key(&mut pending, "idx:0".to_string(), Some("call_shared"));

        assert_eq!(key, "id:call_shared");
        assert_eq!(pending.len(), 1);
        assert!(pending.contains_key("id:call_shared"));
        assert!(!pending.contains_key("idx:0"));
    }
}
