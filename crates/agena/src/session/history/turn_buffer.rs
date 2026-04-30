//! In-memory aggregation for a single in-flight LLM turn.
//!
//! `TurnBuffer` is the bridge between two worlds:
//!
//! * **Streaming** — providers push text deltas, reasoning deltas, partial
//!   tool-call arguments, and tool outputs as they happen. The UI wants this
//!   live for responsiveness.
//! * **Append-only history** — only fully terminal events ever land in the
//!   `session_history_event` table.
//!
//! The buffer holds the live, mutable accumulator entirely in memory. When
//! the turn closes successfully, [`TurnBuffer::commit`] produces an ordered
//! `Vec<EventKind>` containing exclusively *terminal* events
//! (`AssistantMessageCompleted`, `ToolCallIssued`, `ToolCallCompleted`, …).
//! These events are then appended in a single transaction.
//!
//! If the process dies mid-turn, the buffer is lost and **nothing was ever
//! written** to the log — the next process start sees an unmatched
//! `TurnStarted` and emits a `TurnAborted` marker. There is no partial
//! recovery: this is an explicit design decision (see plan `Context`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use smol_str::SmolStr;
use thiserror::Error;

use crate::message::MessageMetadata;

use super::{
    AssistantMessageCompleted, FinishReason, MessageId, ToolCallCompleted, ToolCallId,
    ToolCallIssued, TurnId, UserMessageAppended,
    transcript::{TranscriptBlock, TranscriptContent, TranscriptToolOutput},
};
use crate::event::EventKind;

/// Errors raised when the turn buffer is driven into an inconsistent shape.
///
/// All variants represent programmer bugs in the streaming integration layer
/// — they should not happen at runtime against a well-behaved provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum TurnBufferError {
    #[error("no active assistant message; call begin_assistant() first")]
    NoActiveAssistant,
    #[error("tool call {0} already exists in this turn")]
    DuplicateToolCall(ToolCallId),
    #[error("tool call {0} was not registered before being completed")]
    UnknownToolCall(ToolCallId),
    #[error("tool call {0} is missing its name; call name_tool_call() before commit")]
    ToolCallMissingName(ToolCallId),
}

/// Allocator handed to the buffer so it can mint stable `MessageId`s.
///
/// Decoupling the allocator from the buffer keeps testing trivial — supply a
/// `SequentialIdAllocator` for unit tests and the real DB-backed allocator
/// (`SessionStore::reserve_message_ids`) in production.
pub trait MessageIdAllocator {
    fn next_message_id(&mut self) -> MessageId;
}

#[derive(Debug, Default)]
pub struct SequentialIdAllocator {
    next: i64,
}

impl SequentialIdAllocator {
    pub fn starting_at(start: i64) -> Self {
        Self { next: start }
    }
}

impl MessageIdAllocator for SequentialIdAllocator {
    fn next_message_id(&mut self) -> MessageId {
        let id = self.next;
        self.next += 1;
        MessageId(id)
    }
}

/// Tool call as it accumulates throughout a streaming response.
#[derive(Debug, Clone, Default)]
struct ToolCallInProgress {
    /// Set the moment the provider tells us the tool name. Required at commit.
    name: Option<SmolStr>,
    /// Streamed argument JSON. Providers may stream as text fragments; we
    /// concatenate and parse on commit. Pre-parsed `Value`s win when both
    /// are supplied.
    arguments_text: String,
    arguments_value: Option<Value>,
    output: Option<TranscriptToolOutput>,
    issued_at: DateTime<Utc>,
    completed_at: Option<DateTime<Utc>>,
}

/// One assistant message under construction.
#[derive(Debug, Default)]
struct AssistantInProgress {
    content: TranscriptContent,
    /// Insertion-ordered list of tool calls so commit-time event ordering is
    /// deterministic. `BTreeMap<call_id, _>` would re-order alphabetically,
    /// which we don't want.
    tool_call_order: Vec<ToolCallId>,
    tool_calls: BTreeMap<ToolCallId, ToolCallInProgress>,
    finish_reason: FinishReason,
    usage: Option<crate::message::MessageUsage>,
    metadata: MessageMetadata,
    started_at: DateTime<Utc>,
}

/// One commit-bounded sub-section of a turn.
#[derive(Debug)]
enum Section {
    UserInput {
        message_id: MessageId,
        created_at: DateTime<Utc>,
        content: TranscriptContent,
        metadata: MessageMetadata,
    },
    Assistant {
        message_id: MessageId,
        in_progress: AssistantInProgress,
    },
}

/// Live accumulator for a single LLM turn.
#[derive(Debug, Default)]
pub struct TurnBuffer {
    turn_id: TurnId,
    sections: Vec<Section>,
}

impl TurnBuffer {
    pub fn new(turn_id: TurnId) -> Self {
        Self {
            turn_id,
            sections: Vec::new(),
        }
    }

    pub fn turn_id(&self) -> TurnId {
        self.turn_id
    }

    pub fn is_empty(&self) -> bool {
        self.sections.is_empty()
    }

    /// Record the user message that opened the turn.
    pub fn record_user_input(
        &mut self,
        message_id: MessageId,
        content: TranscriptContent,
        metadata: MessageMetadata,
    ) {
        self.sections.push(Section::UserInput {
            message_id,
            created_at: Utc::now(),
            content,
            metadata,
        });
    }

    /// Begin a new assistant message inside this turn. Returns the freshly
    /// allocated `MessageId` so streaming callbacks can address it by id.
    pub fn begin_assistant<A: MessageIdAllocator>(&mut self, ids: &mut A) -> MessageId {
        let message_id = ids.next_message_id();
        self.sections.push(Section::Assistant {
            message_id,
            in_progress: AssistantInProgress {
                started_at: Utc::now(),
                ..AssistantInProgress::default()
            },
        });
        message_id
    }

    fn current_assistant(&mut self) -> Result<&mut AssistantInProgress, TurnBufferError> {
        match self.sections.last_mut() {
            Some(Section::Assistant { in_progress, .. }) => Ok(in_progress),
            _ => Err(TurnBufferError::NoActiveAssistant),
        }
    }

    pub fn push_text_delta(&mut self, delta: &str) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        match asst.content.blocks.last_mut() {
            Some(TranscriptBlock::Text { text }) => text.push_str(delta),
            _ => asst.content.push_text(delta),
        }
        Ok(())
    }

    pub fn push_reasoning_delta(&mut self, delta: &str) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        match asst.content.blocks.last_mut() {
            Some(TranscriptBlock::Reasoning { text }) => text.push_str(delta),
            _ => asst.content.blocks.push(TranscriptBlock::Reasoning {
                text: delta.to_owned(),
            }),
        }
        Ok(())
    }

    pub fn start_tool_call(&mut self, call_id: ToolCallId) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        if asst.tool_calls.contains_key(&call_id) {
            return Err(TurnBufferError::DuplicateToolCall(call_id));
        }
        asst.tool_call_order.push(call_id.clone());
        asst.tool_calls.insert(
            call_id,
            ToolCallInProgress {
                issued_at: Utc::now(),
                ..ToolCallInProgress::default()
            },
        );
        Ok(())
    }

    pub fn name_tool_call(
        &mut self,
        call_id: &ToolCallId,
        name: impl Into<SmolStr>,
    ) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| TurnBufferError::UnknownToolCall(call_id.clone()))?;
        entry.name = Some(name.into());
        Ok(())
    }

    pub fn append_tool_arguments(
        &mut self,
        call_id: &ToolCallId,
        chunk: &str,
    ) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| TurnBufferError::UnknownToolCall(call_id.clone()))?;
        entry.arguments_text.push_str(chunk);
        Ok(())
    }

    pub fn set_tool_arguments_value(
        &mut self,
        call_id: &ToolCallId,
        value: Value,
    ) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| TurnBufferError::UnknownToolCall(call_id.clone()))?;
        entry.arguments_value = Some(value);
        Ok(())
    }

    /// Record the final result of a previously issued tool call.
    pub fn complete_tool_call(
        &mut self,
        call_id: &ToolCallId,
        output: TranscriptToolOutput,
    ) -> Result<(), TurnBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| TurnBufferError::UnknownToolCall(call_id.clone()))?;
        entry.output = Some(output);
        entry.completed_at = Some(Utc::now());
        Ok(())
    }

    pub fn set_finish_reason(&mut self, reason: FinishReason) -> Result<(), TurnBufferError> {
        self.current_assistant()?.finish_reason = reason;
        Ok(())
    }

    pub fn set_usage(
        &mut self,
        usage: crate::message::MessageUsage,
    ) -> Result<(), TurnBufferError> {
        self.current_assistant()?.usage = Some(usage);
        Ok(())
    }

    pub fn set_metadata(&mut self, metadata: MessageMetadata) -> Result<(), TurnBufferError> {
        self.current_assistant()?.metadata = metadata;
        Ok(())
    }

    /// Drain the buffer into the canonical sequence of append-only events.
    ///
    /// Ordering inside the returned vector is the chronological order events
    /// must appear in the history log. Tool-call issuance precedes the
    /// matching `ToolCallCompleted` so that any reader that streams events
    /// can rely on `call_id` being introduced before it is referenced again.
    pub fn commit(self) -> Result<Vec<EventKind>, TurnBufferError> {
        let TurnBuffer { turn_id, sections } = self;
        let mut items = Vec::with_capacity(sections.len() * 2);

        for section in sections {
            match section {
                Section::UserInput {
                    message_id,
                    created_at,
                    content,
                    metadata,
                } => {
                    items.push(EventKind::UserMessageAppended(UserMessageAppended {
                        message_id,
                        turn_id,
                        created_at,
                        content,
                        metadata,
                    }));
                }
                Section::Assistant {
                    message_id,
                    in_progress,
                } => {
                    let AssistantInProgress {
                        content,
                        tool_call_order,
                        mut tool_calls,
                        finish_reason,
                        usage,
                        metadata,
                        started_at,
                    } = in_progress;

                    items.push(EventKind::AssistantMessageCompleted(
                        AssistantMessageCompleted {
                            message_id,
                            turn_id,
                            created_at: started_at,
                            content,
                            usage,
                            finish_reason,
                            metadata,
                        },
                    ));

                    let mut completions: Vec<EventKind> = Vec::new();
                    for call_id in tool_call_order {
                        let entry = tool_calls
                            .remove(&call_id)
                            .ok_or_else(|| TurnBufferError::UnknownToolCall(call_id.clone()))?;
                        let name = entry
                            .name
                            .clone()
                            .ok_or_else(|| TurnBufferError::ToolCallMissingName(call_id.clone()))?;
                        let arguments = entry.arguments_value.unwrap_or_else(|| {
                            // Best-effort parse; fall back to a JSON string
                            // if the chunks weren't valid JSON. We never
                            // panic on malformed provider data.
                            serde_json::from_str(&entry.arguments_text)
                                .unwrap_or(Value::String(entry.arguments_text.clone()))
                        });
                        items.push(EventKind::ToolCallIssued(ToolCallIssued {
                            message_id,
                            turn_id,
                            call_id: call_id.clone(),
                            name,
                            arguments,
                            created_at: entry.issued_at,
                        }));
                        if let Some(output) = entry.output {
                            completions.push(EventKind::ToolCallCompleted(ToolCallCompleted {
                                call_id,
                                turn_id,
                                output,
                                completed_at: entry.completed_at.unwrap_or_else(Utc::now),
                            }));
                        }
                    }
                    items.extend(completions);
                }
            }
        }

        Ok(items)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allocator() -> SequentialIdAllocator {
        SequentialIdAllocator::starting_at(100)
    }

    #[test]
    fn empty_turn_commits_to_empty_vec() {
        let buf = TurnBuffer::new(TurnId::new());
        assert!(buf.commit().unwrap().is_empty());
    }

    #[test]
    fn commit_preserves_user_then_assistant_order() {
        let mut ids = allocator();
        let mut buf = TurnBuffer::new(TurnId::new());
        buf.record_user_input(
            MessageId(1),
            TranscriptContent::from_text("hi"),
            MessageMetadata::default(),
        );
        buf.begin_assistant(&mut ids);
        buf.push_text_delta("hello").unwrap();
        buf.push_text_delta(" world").unwrap();
        buf.set_finish_reason(FinishReason::Stop).unwrap();

        let items = buf.commit().unwrap();
        assert_eq!(items.len(), 2);
        assert!(matches!(items[0], EventKind::UserMessageAppended(_)));
        match &items[1] {
            EventKind::AssistantMessageCompleted(asst) => {
                assert_eq!(asst.content.blocks.len(), 1);
                if let TranscriptBlock::Text { text } = &asst.content.blocks[0] {
                    assert_eq!(text, "hello world");
                } else {
                    panic!("expected text block");
                }
                assert_eq!(asst.finish_reason, FinishReason::Stop);
            }
            other => panic!("expected AssistantMessageCompleted, got {other:?}"),
        }
    }

    #[test]
    fn tool_calls_emit_in_insertion_order_with_completions_after() {
        let mut ids = allocator();
        let mut buf = TurnBuffer::new(TurnId::new());
        buf.begin_assistant(&mut ids);
        buf.push_text_delta("calling…").unwrap();
        let a: ToolCallId = "call_a".into();
        let b: ToolCallId = "call_b".into();
        buf.start_tool_call(a.clone()).unwrap();
        buf.name_tool_call(&a, "read").unwrap();
        buf.append_tool_arguments(&a, "{\"path\":\"x\"}").unwrap();
        buf.start_tool_call(b.clone()).unwrap();
        buf.name_tool_call(&b, "bash").unwrap();
        buf.set_tool_arguments_value(&b, serde_json::json!({"cmd": "ls"}))
            .unwrap();
        buf.complete_tool_call(&a, TranscriptToolOutput::Text { text: "x".into() })
            .unwrap();
        buf.complete_tool_call(
            &b,
            TranscriptToolOutput::Text {
                text: "a\nb\n".into(),
            },
        )
        .unwrap();
        buf.set_finish_reason(FinishReason::ToolCalls).unwrap();

        let items = buf.commit().unwrap();
        // assistant + 2 issued + 2 completed = 5
        assert_eq!(items.len(), 5);
        let kinds: Vec<&'static str> = items.iter().map(EventKind::tag_str).collect();
        assert_eq!(
            kinds,
            vec![
                "assistant_message_completed",
                "tool_call_issued",
                "tool_call_issued",
                "tool_call_completed",
                "tool_call_completed",
            ]
        );
        // Verify call_id order matches insertion order.
        match (&items[1], &items[2]) {
            (EventKind::ToolCallIssued(x), EventKind::ToolCallIssued(y)) => {
                assert_eq!(x.call_id, a);
                assert_eq!(y.call_id, b);
            }
            _ => panic!(),
        }
    }

    #[test]
    fn rejects_streaming_into_no_active_assistant() {
        let mut buf = TurnBuffer::new(TurnId::new());
        assert_eq!(
            buf.push_text_delta("hi"),
            Err(TurnBufferError::NoActiveAssistant)
        );
    }

    #[test]
    fn rejects_duplicate_tool_call() {
        let mut ids = allocator();
        let mut buf = TurnBuffer::new(TurnId::new());
        buf.begin_assistant(&mut ids);
        let id: ToolCallId = "call_dup".into();
        buf.start_tool_call(id.clone()).unwrap();
        assert_eq!(
            buf.start_tool_call(id.clone()),
            Err(TurnBufferError::DuplicateToolCall(id))
        );
    }

    #[test]
    fn commit_fails_when_tool_call_has_no_name() {
        let mut ids = allocator();
        let mut buf = TurnBuffer::new(TurnId::new());
        buf.begin_assistant(&mut ids);
        let id: ToolCallId = "call_anon".into();
        buf.start_tool_call(id.clone()).unwrap();
        assert_eq!(buf.commit(), Err(TurnBufferError::ToolCallMissingName(id)));
    }
}
