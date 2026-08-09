use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use chrono::{DateTime, Utc};

use crate::error::AppError;
use crate::message::{
    Message, MessageMetadata, MessagePart, MessageProviderState, OperationPart, PartContent,
};
use crate::provider::ProviderRegistry;
use agena_domain::ModelRef;
use agena_domain::{
    AssistantReasoningField, ExecutionStatus, FinishReason, MessageSource, Role, StructuredObject,
    TimeRange, ToolInvocation,
};
use agena_provider::CompletionFinishReason;
use agena_provider::CompletionRequest;
use agena_provider::CompletionStreamEvent;

use super::{ContextGovernor, store::{ProcessorPartIdAllocator, StoreAdapter}};
use agena_storage::store::Part;

const REASONING_PLACEHOLDER: &str = "(no reasoning recorded)";

#[derive(Clone)]
pub(crate) struct SessionRunRequest {
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
    /// The v2 facade-backed store. R2 makes parts the only durable write
    /// source for a model turn: the processor appends content parts under the
    /// run marker (`append_parts`), streams deltas (`update_part`), and
    /// terminalizes the run (`complete_run`/`cancel_run`) itself, so the
    /// caller never re-persists the turn from a v1 message.
    pub store: Arc<StoreAdapter>,
    /// Optional cancel handle. When the token fires the stream loop
    /// terminates between provider events and surfaces a `RunAbortReason::
    /// Cancelled`-shaped terminal error. `None` runs the turn to completion.
    pub cancel: Option<tokio_util::sync::CancellationToken>,
}

/// Result of one processor model turn (R2). The turn's durable state is
/// carried as persisted parts — the run marker plus every content part under
/// it — never as a v1 [`Message`]. Callers rebuild the in-memory v1 projection
/// for the legacy prompt path with [`assistant_message_from_run_parts`].
#[derive(Debug)]
pub(crate) struct SessionRunResult {
    /// The run marker part id — the durable identity of this assistant message.
    pub assistant_message_id: i64,
    /// The run marker part after terminalization. On a successful text-only
    /// turn this is `Completed`; when the turn ended with in-flight tool calls
    /// the marker stays `Pending`/`InProgress` (the session remains Running).
    pub run_marker: Part,
    /// Every content part created under this run, in creation order, with the
    /// latest engine row applied (state/content after streaming and
    /// terminalization).
    pub parts: Vec<Part>,
    /// The final v1 message state for the in-memory projection (Completed on
    /// success even when the marker stays in-flight for pending tools).
    pub message_state: ExecutionStatus,
    pub provider_metadata: Option<serde_json::Value>,
    pub termination: SessionRunTermination,
    /// True when the provider terminal event carried an explicit
    /// `end_turn=false` signal: the model asked for another turn even though
    /// it did not request any tool call.
    pub follow_up_requested: bool,
    /// Normalized terminal finish reason for the model turn. Defaults to
    /// `Stop` when the provider stream did not report a terminal reason.
    pub finish_reason: FinishReason,
    /// Provider-reported usage for the turn, when the provider terminal event
    /// carried one. Persisted through the runtime anchor, not on a part.
    pub usage: Option<agena_provider::CompletionUsage>,
}

#[derive(Debug)]
pub(crate) enum SessionRunTermination {
    Completed,
    Cancelled,
    Failed(AppError),
}

#[derive(Clone)]
/// Processor executing session runs against providers and tools.
pub struct SessionProcessor {
    provider_registry: Arc<ProviderRegistry>,
    context_governor: ContextGovernor,
    plugins: Arc<agena_plugin_host::PluginHost>,
    workspace_root: PathBuf,
}

mod helpers;
mod media;
mod parts;
mod provider_media;
mod run;
mod tool_call_helpers;
mod tool_calls;

pub(crate) use self::helpers::*;
pub(crate) use self::media::*;
pub(crate) use self::parts::assistant_message_from_run_parts;
pub(crate) use self::tool_call_helpers::*;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use agena_domain::{AssistantReplyId, TurnId};
    use agena_storage::store::{Part, PartRole, PartState, PartVisibility};

    use super::{
        ExecutionStatus, PendingToolCall, Role, assistant_message_from_run_parts,
        pending_tool_call_stream_key,
    };
    use crate::session::store::run_marker_content;

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

    /// Build a raw persisted part fixture (parts model only — no v1
    /// projection involved) for the R2 projection tests.
    fn fixture_part(
        part_id: i64,
        kind: &str,
        state: PartState,
        content: serde_json::Value,
        parent_part_id: Option<i64>,
    ) -> Part {
        Part {
            part_id,
            kind: kind.to_owned(),
            role: PartRole::Assistant,
            state,
            content,
            summary: None,
            visibility: PartVisibility::Both,
            rendered_markdown: None,
            parent_part_id,
            run_id: parent_part_id,
            origin_session_id: 1,
            revision: 1,
            started_at_ms: 1_700_000_000_000,
            finished_at_ms: Some(1_700_000_000_100),
            created_at_ms: 1_700_000_000_000,
            updated_at_ms: 1_700_000_000_100,
            provider_state: None,
        }
    }

    #[test]
    fn run_parts_project_onto_the_legacy_message() {
        let turn_id = TurnId::new();
        let reply_id = AssistantReplyId::new();
        let marker = fixture_part(
            7,
            "run",
            PartState::Completed,
            run_marker_content(
                "continue",
                Some("fake"),
                Some("fake-model"),
                Some(turn_id),
                Some(reply_id),
            ),
            None,
        );
        let text = fixture_part(
            8,
            "text",
            PartState::Completed,
            serde_json::json!({ "type": "text", "text": "Hello world" }),
            Some(7),
        );
        let think = fixture_part(
            9,
            "think",
            PartState::Completed,
            serde_json::json!({
                "summary": ["step one"],
                "raw": ["raw step one"]
            }),
            Some(7),
        );

        let message = assistant_message_from_run_parts(
            7,
            ExecutionStatus::Completed,
            &marker,
            &[text, think],
            None,
            None,
        )
        .expect("project persisted run onto v1 message");

        assert_eq!(message.id, 7);
        assert_eq!(message.role, Role::Assistant);
        assert_eq!(message.state, ExecutionStatus::Completed);
        assert_eq!(message.parts.len(), 2, "marker excluded, content parts included");
        assert_eq!(message.parts[0].id, 8);
        assert_eq!(message.parts[0].message_id, 7);
        assert_eq!(message.parts[0].part_index, 0);
        assert_eq!(message.parts[0].text(), Some("Hello world"));
        assert_eq!(message.parts[1].id, 9);
        assert_eq!(message.parts[1].part_index, 1);
        let expected_summary: Vec<String> = vec!["step one".to_owned()];
        assert_eq!(
            message.parts[1].reasoning_summary(),
            Some(expected_summary.as_slice())
        );

        // Identity metadata is reconstructed from the run marker content.
        assert_eq!(message.metadata.model_turn_id, Some(7));
        assert_eq!(message.metadata.model_provider_id, "fake");
        assert_eq!(message.metadata.model_id, "fake-model");
        assert_eq!(message.metadata.conversation_turn_id, Some(turn_id));
        assert_eq!(message.metadata.conversation_reply_id, Some(reply_id));
    }
}
