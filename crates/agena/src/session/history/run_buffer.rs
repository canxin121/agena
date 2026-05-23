//! In-memory aggregation for a single in-flight LLM run.
//!
//! `RunBuffer` is the bridge between two worlds:
//!
//! * **Streaming** — providers push text deltas, reasoning deltas, and
//!   partial tool-call arguments as they happen. The UI wants this live for
//!   responsiveness.
//! * **Append-only history** — only fully terminal events ever land in the
//!   `session_history_event` table.
//!
//! The buffer holds the live, mutable accumulator entirely in memory. When
//! the run closes successfully, [`RunBuffer::commit`] produces an ordered
//! `Vec<EventKind>` containing exclusively *terminal* events
//! (`AssistantMessageCompleted`, `ToolCallIssued`, …).
//! These events are then appended in a single transaction.
//!
//! If the process dies mid-run, the buffer is lost and **nothing was ever
//! written** to the log — the next process start sees an unmatched
//! `RunStarted` and emits a `RunAborted` marker. There is no partial
//! recovery: this is an explicit design decision (see plan `Context`).

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde_json::Value;
use smol_str::SmolStr;
use thiserror::Error;

use crate::message::{MessageMetadata, MessageProviderState};

use super::{
    AssistantMessageCompleted, FinishReason, MessageId, RunId, ToolCallId, ToolCallIssued,
    transcript::{TranscriptBlock, TranscriptContent},
};
use crate::event::EventKind;

/// Errors raised when the run buffer is driven into an inconsistent shape.
///
/// All variants represent programmer bugs in the streaming integration layer
/// — they should not happen at runtime against a well-behaved provider.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum RunBufferError {
    #[error("no active assistant message; call begin_assistant() first")]
    NoActiveAssistant,
    #[error("tool call {0} already exists in this run")]
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
    /// concatenate and parse on commit.
    arguments_text: String,
    issued_at: DateTime<Utc>,
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
    provider_state: Option<MessageProviderState>,
    started_at: DateTime<Utc>,
}

/// One commit-bounded sub-section of a run.
#[derive(Debug)]
enum Section {
    Assistant {
        message_id: MessageId,
        in_progress: AssistantInProgress,
    },
}

/// Live accumulator for a single LLM run.
#[derive(Debug, Default)]
pub struct RunBuffer {
    run_id: RunId,
    sections: Vec<Section>,
}

impl RunBuffer {
    pub fn new(run_id: RunId) -> Self {
        Self {
            run_id,
            sections: Vec::new(),
        }
    }

    /// Begin a new assistant message inside this run. Returns the freshly
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

    fn current_assistant(&mut self) -> Result<&mut AssistantInProgress, RunBufferError> {
        match self.sections.last_mut() {
            Some(Section::Assistant { in_progress, .. }) => Ok(in_progress),
            _ => Err(RunBufferError::NoActiveAssistant),
        }
    }

    pub fn push_text_delta(&mut self, delta: &str) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        match asst.content.blocks.last_mut() {
            Some(TranscriptBlock::Text { text }) => text.push_str(delta),
            _ => asst.content.push_text(delta),
        }
        Ok(())
    }

    pub fn push_reasoning_delta(&mut self, delta: &str) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        match asst.content.blocks.last_mut() {
            Some(TranscriptBlock::Reasoning { text }) => text.push_str(delta),
            _ => asst.content.blocks.push(TranscriptBlock::Reasoning {
                text: delta.to_owned(),
            }),
        }
        Ok(())
    }

    pub fn start_tool_call(&mut self, call_id: ToolCallId) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        if asst.tool_calls.contains_key(&call_id) {
            return Err(RunBufferError::DuplicateToolCall(call_id));
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
    ) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| RunBufferError::UnknownToolCall(call_id.clone()))?;
        entry.name = Some(name.into());
        Ok(())
    }

    pub fn append_tool_arguments(
        &mut self,
        call_id: &ToolCallId,
        chunk: &str,
    ) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| RunBufferError::UnknownToolCall(call_id.clone()))?;
        entry.arguments_text.push_str(chunk);
        Ok(())
    }

    pub fn set_finish_reason(&mut self, reason: FinishReason) -> Result<(), RunBufferError> {
        self.current_assistant()?.finish_reason = reason;
        Ok(())
    }

    pub fn set_usage(&mut self, usage: crate::message::MessageUsage) -> Result<(), RunBufferError> {
        self.current_assistant()?.usage = Some(usage);
        Ok(())
    }

    pub fn set_metadata(&mut self, metadata: MessageMetadata) -> Result<(), RunBufferError> {
        self.current_assistant()?.metadata = metadata;
        Ok(())
    }

    pub fn set_provider_state(
        &mut self,
        provider_state: Option<MessageProviderState>,
    ) -> Result<(), RunBufferError> {
        self.current_assistant()?.provider_state = provider_state;
        Ok(())
    }

    /// Drain the buffer into the canonical sequence of append-only events.
    ///
    /// Ordering inside the returned vector is the chronological order events
    /// must appear in the history log.
    ///
    /// `ids` is kept in the signature for callers that already allocate run
    /// message ids before committing; tool completions now update the owning
    /// assistant message instead of allocating synthetic tool messages.
    pub fn commit<A: MessageIdAllocator>(
        self,
        _ids: &mut A,
    ) -> Result<Vec<EventKind>, RunBufferError> {
        let RunBuffer { run_id, sections } = self;
        let mut items = Vec::with_capacity(sections.len() * 2);

        for section in sections {
            match section {
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
                        provider_state,
                        started_at,
                    } = in_progress;

                    items.push(EventKind::AssistantMessageCompleted(
                        AssistantMessageCompleted {
                            message_id,
                            run_id,
                            created_at: started_at,
                            content,
                            parts: Vec::new(),
                            usage,
                            finish_reason,
                            metadata,
                            provider_state,
                        },
                    ));

                    for call_id in tool_call_order {
                        let entry = tool_calls
                            .remove(&call_id)
                            .ok_or_else(|| RunBufferError::UnknownToolCall(call_id.clone()))?;
                        let name = entry
                            .name
                            .clone()
                            .ok_or_else(|| RunBufferError::ToolCallMissingName(call_id.clone()))?;
                        let arguments = serde_json::from_str(&entry.arguments_text)
                            .unwrap_or(Value::String(entry.arguments_text.clone()));
                        items.push(EventKind::ToolCallIssued(ToolCallIssued {
                            message_id,
                            run_id,
                            call_id: call_id.clone(),
                            name: name.clone(),
                            arguments,
                            created_at: entry.issued_at,
                        }));
                    }
                }
            }
        }

        Ok(items)
    }
}
