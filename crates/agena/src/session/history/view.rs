//! Append-only event log → `Vec<Message>` projection.
//!
//! [`SessionViewBuilder`] folds the new history event variants into the same
//! `Message` / `MessagePart` shape that the rest of agena consumes. It is the
//! parity counterpart to the legacy [`replay_history`](super::replay_history)
//! reducer — the latter still owns mutable-state events while this builder
//! handles the strict append-only variants introduced by the rewrite.
//!
//! Folding rules (mirrors `ProviderTranscriptBuilder`):
//!
//! * `TurnStarted` → registers a turn slot.
//! * `UserMessageAppended` / `AssistantMessageCompleted` / `ToolCallIssued` /
//!   `ToolCallCompleted` are buffered against their owning turn until the
//!   turn closes.
//! * `TurnCompleted` → flushes the buffered messages into the projection.
//! * `TurnAborted` → drops every buffered message in that turn.
//! * `SystemNoticeAppended` → emitted as a standalone `Role::System` message.
//!
//! Keys for ordering: messages are sorted by `(created_at, message_id)` to
//! match the historical `replay_history` ordering.

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};

use crate::{
    message::{
        ExecutionStatus, Message, MessageMetadata, MessagePart, MessageSource, MessageStatus,
        PartContent, ReasoningPart, StructuredObject, TextPart, TimeRange, ToolExecutionPart,
        ToolInvocation, ToolOutput,
    },
    role::Role,
    session::{
        SessionRuntimeState,
        ids::{MessageId, ToolCallId, TurnId},
    },
};

use super::{
    AssistantMessageCompleted, FinishReason, MessageRevised, RevisionKind, SystemNoticeAppended,
    SystemNoticeKind, ToolCallCompleted, ToolCallIssued, TurnAborted, TurnCompleted, TurnStarted,
    UserMessageAppended,
    projection::HistoryFold,
    transcript::{TranscriptBlock, TranscriptContent, TranscriptToolOutput},
};
use crate::event::{DomainEvent, EventKind};

/// Output of a `SessionViewBuilder` fold.
///
/// `runtime` is **always default-initialized** here. Session runtime state
/// (provider anchors, prompt-token snapshots, deferred tools, etc.) is
/// authoritative on `agena_sessions.runtime_state_json` and never passes
/// through the history log — callers that need it must read the session row
/// directly.
#[derive(Debug, Clone, Default)]
pub(crate) struct SessionView {
    /// Messages ordered by `(created_at, message_id)`.
    pub messages: Vec<Message>,
    pub runtime: SessionRuntimeState,
    pub last_seq: i64,
}

/// Errors raised while folding an event log into a [`SessionView`].
#[derive(Debug, thiserror::Error)]
pub(crate) enum SessionViewError {
    /// Provider supplied a `ToolCallCompleted` without ever issuing the
    /// matching `ToolCallIssued`. The builder records the result anyway under
    /// a synthesized message, but logs the inconsistency for callers that
    /// want to be strict.
    #[error("tool_call_completed for unknown call_id={0}")]
    UnknownToolCall(ToolCallId),
}

#[derive(Debug, Default)]
pub(crate) struct SessionViewBuilder {
    turn_state: HashMap<TurnId, TurnState>,
    turn_order: Vec<TurnId>,
    aborted_turns: HashSet<TurnId>,
    /// Messages that have been finalized (their owning turn has closed).
    finalized: BTreeMap<MessageKey, Message>,
    /// Maps a tool call id to where its result should be attached.
    tool_call_index: HashMap<ToolCallId, ToolCallLocation>,
    last_seq: i64,
    /// Monotonic part-id allocator scoped to the projection. Part ids are not
    /// stored in the event log (they were a property of the deleted
    /// `message_part` table); the projection synthesises them.
    next_part_id: i64,
    /// Messages dropped by a `MessageRevised { Compacted }` revision.
    compacted_messages: HashSet<i64>,
}

/// Sort key matching the legacy reducer's ordering of `(created_at, id)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct MessageKey {
    created_at: DateTime<Utc>,
    message_id: i64,
}

impl MessageKey {
    fn new(created_at: DateTime<Utc>, message_id: MessageId) -> Self {
        Self {
            created_at,
            message_id: message_id.raw(),
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct ToolCallLocation {
    turn_id: TurnId,
    /// Index into `TurnState.messages` of the assistant message that issued
    /// this call.
    #[allow(dead_code)]
    assistant_index: usize,
}

#[derive(Debug, Default)]
struct TurnState {
    /// Buffered messages for this turn; flushed wholesale into `finalized`
    /// once `TurnCompleted` arrives, dropped on `TurnAborted`.
    messages: Vec<BufferedMessage>,
}

#[derive(Debug)]
struct BufferedMessage {
    key: MessageKey,
    message: Message,
    /// Optional pointer back into the call index for cleanup on abort.
    issued_calls: Vec<ToolCallId>,
}

impl SessionViewBuilder {
    fn alloc_part_id(&mut self) -> i64 {
        if self.next_part_id == 0 {
            // Synthetic part ids must never collide with the real part-id
            // space handed out by the store. The store reserves positive
            // monotonic ids; we draw from the negative range so the two are
            // disjoint by construction.
            self.next_part_id = -1_000_000;
        }
        self.next_part_id += 1;
        self.next_part_id
    }

    fn ensure_turn(&mut self, turn_id: TurnId) -> &mut TurnState {
        if !self.turn_state.contains_key(&turn_id) {
            self.turn_state.insert(turn_id, TurnState::default());
            self.turn_order.push(turn_id);
        }
        self.turn_state
            .get_mut(&turn_id)
            .expect("inserted just above")
    }

    fn handle_user(&mut self, payload: &UserMessageAppended) {
        let parts = parts_from_transcript(
            &payload.content,
            payload.message_id.raw(),
            payload.created_at,
            ExecutionStatus::Completed,
            || self.alloc_part_id(),
        );
        let message = Message {
            id: payload.message_id.raw(),
            role: Role::User,
            state: MessageStatus::Completed,
            parts,
            created_at: payload.created_at,
            metadata: with_source(payload.metadata.clone(), MessageSource::User),
            usage: None,
            finish: None,
        };
        let key = MessageKey::new(payload.created_at, payload.message_id);
        self.ensure_turn(payload.turn_id)
            .messages
            .push(BufferedMessage {
                key,
                message,
                issued_calls: Vec::new(),
            });
    }

    fn handle_assistant(&mut self, payload: &AssistantMessageCompleted) {
        let parts = parts_from_transcript(
            &payload.content,
            payload.message_id.raw(),
            payload.created_at,
            ExecutionStatus::Completed,
            || self.alloc_part_id(),
        );
        let finish_label = finish_reason_label(payload.finish_reason);
        let message = Message {
            id: payload.message_id.raw(),
            role: Role::Assistant,
            state: MessageStatus::Completed,
            parts,
            created_at: payload.created_at,
            metadata: with_source(payload.metadata.clone(), MessageSource::Assistant),
            usage: payload.usage.clone(),
            finish: finish_label,
        };
        let key = MessageKey::new(payload.created_at, payload.message_id);
        let turn = self.ensure_turn(payload.turn_id);
        turn.messages.push(BufferedMessage {
            key,
            message,
            issued_calls: Vec::new(),
        });
    }

    fn handle_tool_call_issued(&mut self, payload: &ToolCallIssued) -> Result<(), SessionViewError> {
        let part_id = self.alloc_part_id();
        let turn = self.ensure_turn(payload.turn_id);

        // Locate the assistant message inside this turn that owns this call.
        let assistant_index = turn
            .messages
            .iter()
            .position(|buffered| {
                buffered.message.role == Role::Assistant
                    && buffered.message.id == payload.message_id.raw()
            });

        let Some(assistant_index) = assistant_index else {
            // Issuing a tool call against a missing assistant message would be
            // a producer bug; we silently drop it rather than corrupting the
            // projection.
            return Ok(());
        };

        let buffered = &mut turn.messages[assistant_index];
        let invocation = ToolInvocation::Custom {
            name: payload.name.to_string(),
            input: structured_from_json(&payload.arguments),
        };
        let mut part = MessagePart::with_content(
            part_id,
            buffered.message.id,
            payload.created_at,
            ExecutionStatus::Pending,
            PartContent::ToolExecution(ToolExecutionPart::Pending {
                call_id: 0,
                invocation,
                title: payload.name.to_string(),
                lifecycle: TimeRange::default(),
            }),
        );
        part.operation_id = Some(payload.call_id.as_str().to_owned());
        buffered.message.push_part(part);
        buffered.issued_calls.push(payload.call_id.clone());

        self.tool_call_index.insert(
            payload.call_id.clone(),
            ToolCallLocation {
                turn_id: payload.turn_id,
                assistant_index,
            },
        );
        Ok(())
    }

    fn handle_tool_call_completed(
        &mut self,
        payload: &ToolCallCompleted,
    ) -> Result<(), SessionViewError> {
        // Tool results live as a `Role::Tool` message that immediately follows
        // the assistant message that issued the call — this matches the
        // convention used by `prompt_window::messages_to_provider_transcript`.
        let part_id = self.alloc_part_id();
        let location = self.tool_call_index.get(&payload.call_id).copied();
        let Some(location) = location else {
            return Err(SessionViewError::UnknownToolCall(payload.call_id.clone()));
        };
        if location.turn_id != payload.turn_id {
            // Cross-turn completion — treat as unknown.
            return Err(SessionViewError::UnknownToolCall(payload.call_id.clone()));
        }

        let output_text = match &payload.output {
            TranscriptToolOutput::Text { text } => text.clone(),
            TranscriptToolOutput::Pruned { replacement } => replacement.clone(),
            TranscriptToolOutput::Error { message } => message.clone(),
        };

        // Synthesise a stable but unique message id derived from the call id.
        // The legacy code never persisted ids for synthetic tool messages
        // either; the only requirement is uniqueness within the projection.
        let message_id = synthetic_tool_message_id(&payload.call_id);
        let mut part = MessagePart::with_content(
            part_id,
            message_id,
            payload.completed_at,
            ExecutionStatus::Completed,
            PartContent::ToolExecution(ToolExecutionPart::Completed {
                call_id: 0,
                invocation: ToolInvocation::Custom {
                    name: "tool".to_owned(),
                    input: StructuredObject::default(),
                },
                output_text,
                blocks: Vec::new(),
                attachments: Vec::new(),
                details: ToolOutput::default(),
                lifecycle: TimeRange::default(),
            }),
        );
        part.operation_id = Some(payload.call_id.as_str().to_owned());

        let mut metadata = MessageMetadata::default();
        metadata.source = MessageSource::Tool;
        let message = Message {
            id: message_id,
            role: Role::Tool,
            state: MessageStatus::Completed,
            parts: vec![part],
            created_at: payload.completed_at,
            metadata,
            usage: None,
            finish: None,
        };

        let key = MessageKey::new(payload.completed_at, MessageId(message_id));
        self.ensure_turn(payload.turn_id)
            .messages
            .push(BufferedMessage {
                key,
                message,
                issued_calls: Vec::new(),
            });
        Ok(())
    }

    fn handle_system_notice(&mut self, payload: &SystemNoticeAppended) {
        // System notices are not bound to a turn; they finalize immediately
        // into the projection.
        let part_id = self.alloc_part_id();
        let mut part = MessagePart::with_content(
            part_id,
            payload.message_id.raw(),
            payload.created_at,
            ExecutionStatus::Completed,
            PartContent::Text(TextPart {
                text: payload.text.clone(),
                synthetic: matches!(payload.kind, SystemNoticeKind::CompactionSummary),
                ignored: false,
            }),
        );
        part.part_index = 0;

        let mut metadata = MessageMetadata::default();
        metadata.source = MessageSource::System;
        metadata.add_tag(system_notice_tag(payload.kind));
        let message = Message {
            id: payload.message_id.raw(),
            role: Role::System,
            state: MessageStatus::Completed,
            parts: vec![part],
            created_at: payload.created_at,
            metadata,
            usage: None,
            finish: None,
        };
        let key = MessageKey::new(payload.created_at, payload.message_id);
        self.finalized.insert(key, message);
    }

    fn close_turn(&mut self, turn_id: TurnId) {
        let Some(state) = self.turn_state.remove(&turn_id) else {
            return;
        };
        for buffered in state.messages {
            self.finalized.insert(buffered.key, buffered.message);
        }
    }

    fn abort_turn(&mut self, turn_id: TurnId) {
        let Some(state) = self.turn_state.remove(&turn_id) else {
            self.aborted_turns.insert(turn_id);
            return;
        };
        for buffered in &state.messages {
            for call_id in &buffered.issued_calls {
                self.tool_call_index.remove(call_id);
            }
        }
        self.aborted_turns.insert(turn_id);
    }
}

impl HistoryFold for SessionViewBuilder {
    type Output = Result<SessionView, SessionViewError>;
    type Error = SessionViewError;

    fn fold(&mut self, event: &DomainEvent) -> Result<(), Self::Error> {
        self.last_seq = event.meta.seq_global;
        match &event.kind {
            EventKind::TurnStarted(TurnStarted { turn_id, .. }) => {
                self.ensure_turn(*turn_id);
            }
            EventKind::TurnCompleted(TurnCompleted { turn_id, .. }) => {
                self.close_turn(*turn_id);
            }
            EventKind::TurnAborted(TurnAborted { turn_id, .. }) => {
                self.abort_turn(*turn_id);
            }
            EventKind::UserMessageAppended(payload) => self.handle_user(payload),
            EventKind::AssistantMessageCompleted(payload) => self.handle_assistant(payload),
            EventKind::ToolCallIssued(payload) => self.handle_tool_call_issued(payload)?,
            EventKind::ToolCallCompleted(payload) => self.handle_tool_call_completed(payload)?,
            EventKind::SystemNoticeAppended(payload) => self.handle_system_notice(payload),
            EventKind::MessageRevised(MessageRevised {
                target_message_id,
                kind,
            }) => match kind {
                RevisionKind::Compacted => {
                    self.compacted_messages.insert(*target_message_id);
                }
                RevisionKind::ToolResultPruned { .. }
                | RevisionKind::AttachmentStripped { .. } => {
                    // The session view shows the latest state of each message
                    // — the on-disk message bodies have already been rewritten
                    // by the upstream pruner/stripper. Nothing to do here.
                }
            },
            // Runtime / UI projection events do not contribute to the
            // transcript view.
            EventKind::RunStarted(_)
            | EventKind::RunFailed(_)
            | EventKind::StreamError(_)
            | EventKind::MessagePartUpdated(_)
            | EventKind::MessagePartDelta(_)
            | EventKind::CommandBegin(_)
            | EventKind::CommandOutputDelta(_)
            | EventKind::CommandEnd(_)
            | EventKind::PluginEvent(_) => {}
        }
        Ok(())
    }

    fn finish(self) -> Self::Output {
        let SessionViewBuilder {
            mut finalized,
            last_seq,
            compacted_messages,
            ..
        } = self;

        if !compacted_messages.is_empty() {
            finalized.retain(|_, message| !compacted_messages.contains(&message.id));
        }

        let messages: Vec<Message> = finalized.into_values().collect();
        Ok(SessionView {
            messages,
            runtime: SessionRuntimeState::default(),
            last_seq,
        })
    }
}

// ─── helpers ───────────────────────────────────────────────────────────────

#[allow(dead_code)]
fn with_source(mut metadata: MessageMetadata, source: MessageSource) -> MessageMetadata {
    metadata.source = source;
    metadata
}

#[allow(dead_code)]
fn finish_reason_label(reason: FinishReason) -> Option<String> {
    match reason {
        FinishReason::Stop => Some("stop".into()),
        FinishReason::ToolCalls => Some("tool_calls".into()),
        FinishReason::MaxTokens => Some("length".into()),
        FinishReason::ContentFilter => Some("content_filter".into()),
        FinishReason::Error => Some("error".into()),
        FinishReason::Other => None,
    }
}

#[allow(dead_code)]
fn system_notice_tag(kind: SystemNoticeKind) -> &'static str {
    match kind {
        SystemNoticeKind::CompactionSummary => "system_notice:compaction_summary",
        SystemNoticeKind::ContextInjection => "system_notice:context_injection",
        SystemNoticeKind::ToolPolicyHint => "system_notice:tool_policy_hint",
        SystemNoticeKind::Other => "system_notice:other",
    }
}

#[allow(dead_code)]
fn parts_from_transcript<F: FnMut() -> i64>(
    content: &TranscriptContent,
    message_id: i64,
    created_at: DateTime<Utc>,
    status: ExecutionStatus,
    mut alloc_part_id: F,
) -> Vec<MessagePart> {
    let mut parts = Vec::with_capacity(content.blocks.len());
    for (idx, block) in content.blocks.iter().enumerate() {
        let part_content = match block {
            TranscriptBlock::Text { text } => Some(PartContent::Text(TextPart {
                text: text.clone(),
                synthetic: false,
                ignored: false,
            })),
            TranscriptBlock::Reasoning { text } => Some(PartContent::Reasoning(ReasoningPart {
                summary: vec![text.clone()],
                raw_content: Vec::new(),
                encrypted_content: None,
            })),
            // Image / Attachment blocks in the transcript are inputs to the
            // provider that we do not have a faithful inverse for in the
            // `Message` shape (the originals carried richer metadata that
            // never went through the digest). Skip them in the projection —
            // the legacy code path surfaced them via separate Attachment
            // parts that are reconstructed by the input pipeline, not the
            // history projection.
            TranscriptBlock::Image { .. } | TranscriptBlock::Attachment { .. } => None,
        };
        let Some(part_content) = part_content else {
            continue;
        };
        let part_id = alloc_part_id();
        let mut part = MessagePart::with_content(part_id, message_id, created_at, status, part_content);
        part.part_index = idx as i32;
        parts.push(part);
    }
    parts
}

#[allow(dead_code)]
fn structured_from_json(value: &serde_json::Value) -> StructuredObject {
    serde_json::from_value(value.clone()).unwrap_or_default()
}

#[allow(dead_code)]
fn synthetic_tool_message_id(call_id: &ToolCallId) -> i64 {
    // Negative ids never collide with the positive monotonic id space the
    // store hands out. Hash the call id into the lower 62 bits so two calls
    // with different ids never collide either.
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    call_id.as_str().hash(&mut hasher);
    let raw = hasher.finish() as i64;
    // Ensure negative and avoid i64::MIN overflow on negation.
    -((raw & i64::MAX) | 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{EventKind, DomainEvent};
    use crate::session::history::{
        AssistantMessageCompleted, FinishReason, SystemNoticeAppended, SystemNoticeKind,
        ToolCallCompleted, ToolCallIssued, TurnAbortReason, TurnAborted, TurnCompleted,
        TurnStarted, UserMessageAppended, fold_history,
    };
    use crate::event::{EventMeta, envelope::ENVELOPE_SCHEMA_VERSION};
    use chrono::Utc;
    use smol_str::SmolStr;
    use uuid::Uuid;

    fn record(kind: EventKind) -> DomainEvent {
        DomainEvent {
            meta: EventMeta {
                id: Uuid::new_v4(),
                seq_global: 0,
                seq_session: Some(0),
                session_id: Some(1),
                workspace_id: None,
                created_at: Utc::now(),
                causation_id: None,
                correlation_id: None,
                envelope_schema: ENVELOPE_SCHEMA_VERSION,
            },
            kind,
        }
    }

    fn turn_started(turn_id: TurnId) -> DomainEvent {
        record(EventKind::TurnStarted(TurnStarted {
            turn_id,
            model_id: "m".into(),
            provider_id: "p".into(),
            request_digest: None,
        }))
    }

    fn turn_completed(turn_id: TurnId, reason: FinishReason) -> DomainEvent {
        record(EventKind::TurnCompleted(TurnCompleted {
            turn_id,
            finish_reason: reason,
        }))
    }

    fn user_msg(turn_id: TurnId, message_id: i64, text: &str) -> DomainEvent {
        record(EventKind::UserMessageAppended(UserMessageAppended {
            message_id: MessageId(message_id),
            turn_id,
            created_at: Utc::now(),
            content: TranscriptContent::from_text(text),
            metadata: MessageMetadata::default(),
        }))
    }

    fn assistant_msg(
        turn_id: TurnId,
        message_id: i64,
        text: &str,
        finish_reason: FinishReason,
    ) -> DomainEvent {
        record(EventKind::AssistantMessageCompleted(
            AssistantMessageCompleted {
                message_id: MessageId(message_id),
                turn_id,
                created_at: Utc::now(),
                content: TranscriptContent::from_text(text),
                usage: None,
                finish_reason,
                metadata: MessageMetadata::default(),
            },
        ))
    }

    fn run(records: Vec<DomainEvent>) -> SessionView {
        let mut records = records;
        records
            .iter_mut()
            .enumerate()
            .for_each(|(i, r)| r.meta.seq_global = i as i64);
        fold_history::<SessionViewBuilder>(&records).unwrap().unwrap()
    }

    #[test]
    fn single_turn_user_then_assistant() {
        let turn_id = TurnId::new();
        let records = vec![
            turn_started(turn_id),
            user_msg(turn_id, 1, "hi"),
            assistant_msg(turn_id, 2, "hello!", FinishReason::Stop),
            turn_completed(turn_id, FinishReason::Stop),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[0].role, Role::User);
        assert_eq!(view.messages[0].as_text_lossy(), "hi");
        assert_eq!(view.messages[1].role, Role::Assistant);
        assert_eq!(view.messages[1].as_text_lossy(), "hello!");
        assert_eq!(view.messages[1].finish.as_deref(), Some("stop"));
    }

    #[test]
    fn turn_with_tool_call_emits_assistant_then_tool_message() {
        let turn_id = TurnId::new();
        let call: ToolCallId = "call_alpha".into();
        let records = vec![
            turn_started(turn_id),
            assistant_msg(turn_id, 10, "running", FinishReason::ToolCalls),
            record(EventKind::ToolCallIssued(ToolCallIssued {
                message_id: MessageId(10),
                turn_id,
                call_id: call.clone(),
                name: SmolStr::new("read_file"),
                arguments: serde_json::json!({"path": "x"}),
                created_at: Utc::now(),
            })),
            record(EventKind::ToolCallCompleted(ToolCallCompleted {
                call_id: call.clone(),
                turn_id,
                output: TranscriptToolOutput::Text {
                    text: "fn main(){}".into(),
                },
                completed_at: Utc::now(),
            })),
            turn_completed(turn_id, FinishReason::ToolCalls),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 2);
        assert_eq!(view.messages[0].role, Role::Assistant);
        // Assistant message should now have 2 parts: the text + the tool call.
        assert_eq!(view.messages[0].parts.len(), 2);
        let tool_part = &view.messages[0].parts[1];
        assert!(matches!(
            tool_part.content.as_ref(),
            Some(PartContent::ToolExecution(ToolExecutionPart::Pending { .. }))
        ));
        assert_eq!(tool_part.operation_id.as_deref(), Some("call_alpha"));

        // Tool result lives in the trailing Role::Tool message keyed by the
        // same call id via operation_id.
        assert_eq!(view.messages[1].role, Role::Tool);
        assert_eq!(view.messages[1].parts.len(), 1);
        assert_eq!(
            view.messages[1].parts[0].operation_id.as_deref(),
            Some("call_alpha")
        );
        assert_eq!(view.messages[1].as_text_lossy(), "fn main(){}");
    }

    #[test]
    fn aborted_turn_drops_all_its_messages() {
        let kept_turn = TurnId::new();
        let aborted_turn = TurnId::new();
        let records = vec![
            turn_started(kept_turn),
            user_msg(kept_turn, 1, "kept-user"),
            assistant_msg(kept_turn, 2, "kept-asst", FinishReason::Stop),
            turn_completed(kept_turn, FinishReason::Stop),
            // Now an aborted turn — its UserMessageAppended must vanish.
            turn_started(aborted_turn),
            user_msg(aborted_turn, 3, "doomed"),
            assistant_msg(aborted_turn, 4, "also-doomed", FinishReason::Stop),
            record(EventKind::TurnAborted(TurnAborted {
                turn_id: aborted_turn,
                reason: TurnAbortReason::ProcessRestart,
                message: None,
            })),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 2);
        let texts: Vec<String> = view.messages.iter().map(Message::as_text_lossy).collect();
        assert_eq!(texts, vec!["kept-user".to_string(), "kept-asst".to_string()]);
    }

    #[test]
    fn multiple_sequential_turns_preserve_global_order() {
        let t1 = TurnId::new();
        let t2 = TurnId::new();
        let records = vec![
            turn_started(t1),
            user_msg(t1, 1, "u1"),
            assistant_msg(t1, 2, "a1", FinishReason::Stop),
            turn_completed(t1, FinishReason::Stop),
            turn_started(t2),
            user_msg(t2, 3, "u2"),
            assistant_msg(t2, 4, "a2", FinishReason::Stop),
            turn_completed(t2, FinishReason::Stop),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 4);
        let ids: Vec<i64> = view.messages.iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
    }

    #[test]
    fn system_notice_emits_standalone_system_message() {
        let turn_id = TurnId::new();
        let records = vec![
            turn_started(turn_id),
            user_msg(turn_id, 1, "hi"),
            turn_completed(turn_id, FinishReason::Stop),
            record(EventKind::SystemNoticeAppended(SystemNoticeAppended {
                message_id: MessageId(99),
                created_at: Utc::now(),
                kind: SystemNoticeKind::CompactionSummary,
                text: "summary text".into(),
            })),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 2);
        let system = view
            .messages
            .iter()
            .find(|m| m.role == Role::System)
            .expect("system notice projected");
        assert_eq!(system.as_text_lossy(), "summary text");
        assert!(
            system
                .metadata
                .has_tag("system_notice:compaction_summary")
        );
    }

    #[test]
    fn in_flight_turn_is_invisible_until_closed() {
        let turn_id = TurnId::new();
        let records = vec![
            turn_started(turn_id),
            user_msg(turn_id, 1, "hi"),
            assistant_msg(turn_id, 2, "hello", FinishReason::Stop),
            // No TurnCompleted — turn is in flight, must not be projected.
        ];
        let view = run(records);
        assert!(
            view.messages.is_empty(),
            "expected no messages from in-flight turn, got {:?}",
            view.messages
        );
    }

    use crate::session::history::{MessageRevised, RevisionKind};

    #[test]
    fn message_revised_compacted_drops_target_message() {
        let turn_id = TurnId::new();
        let records = vec![
            turn_started(turn_id),
            user_msg(turn_id, 1, "doomed"),
            assistant_msg(turn_id, 2, "kept", FinishReason::Stop),
            turn_completed(turn_id, FinishReason::Stop),
            record(EventKind::MessageRevised(MessageRevised {
                target_message_id: 1,
                kind: RevisionKind::Compacted,
            })),
        ];
        let view = run(records);
        assert_eq!(view.messages.len(), 1);
        assert_eq!(view.messages[0].id, 2);
    }
}
