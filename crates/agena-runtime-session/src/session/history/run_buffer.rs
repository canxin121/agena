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
//! (`AssistantMessageFinished`, `ToolCallIssued`, …).
//! These events are then appended in a single transaction.
//!
//! If the process dies mid-run, the buffer is lost and **nothing was ever
//! written** to the log — the next process start sees an unmatched
//! `RunStarted` and emits a `RunAborted` marker. There is no partial
//! recovery: this is an explicit design decision (see plan `Context`).

use std::collections::BTreeMap;

use agena_domain::{ExecutionStatus, FinishReason};
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::Value;
use smol_str::SmolStr;
use thiserror::Error;

use crate::message::{MessageMetadata, MessageProviderState};

use agena_storage::MessageIdAllocator;

use super::{
    AssistantMessageFinished, MessageId, RunId, ToolCallId, ToolCallIssued,
    transcript::{TranscriptBlock, TranscriptContent},
};
use crate::event::EventKind;
use agena_domain::ExecutionId;

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
    #[error("assistant message status is not terminal: {0:?}")]
    NonTerminalMessageStatus(ExecutionStatus),
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
    usage: Option<agena_provider::CompletionUsage>,
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
#[derive(Debug)]
pub struct RunBuffer {
    execution_id: ExecutionId,
    run_id: RunId,
    terminal_status: ExecutionStatus,
    sections: Vec<Section>,
}

impl RunBuffer {
    pub fn new(execution_id: ExecutionId, run_id: RunId) -> Self {
        Self {
            execution_id,
            run_id,
            terminal_status: ExecutionStatus::Completed,
            sections: Vec::new(),
        }
    }

    /// Begin a new assistant message inside this run. Returns the freshly
    /// allocated `MessageId` so streaming callbacks can address it by id.
    pub fn begin_assistant<A: MessageIdAllocator>(
        &mut self,
        ids: &mut A,
        started_at: DateTime<Utc>,
    ) -> MessageId {
        let message_id = ids.next_message_id();
        self.sections.push(Section::Assistant {
            message_id,
            in_progress: AssistantInProgress {
                started_at,
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

    pub fn replace_tool_call_id(
        &mut self,
        old_call_id: &ToolCallId,
        new_call_id: ToolCallId,
    ) -> Result<(), RunBufferError> {
        if old_call_id == &new_call_id {
            return Ok(());
        }

        let asst = self.current_assistant()?;
        if asst.tool_calls.contains_key(&new_call_id) {
            return Err(RunBufferError::DuplicateToolCall(new_call_id));
        }

        let entry = asst
            .tool_calls
            .remove(old_call_id)
            .ok_or_else(|| RunBufferError::UnknownToolCall(old_call_id.clone()))?;
        if let Some(order_entry) = asst
            .tool_call_order
            .iter_mut()
            .find(|call_id| *call_id == old_call_id)
        {
            *order_entry = new_call_id.clone();
        }
        asst.tool_calls.insert(new_call_id, entry);
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

    pub fn replace_tool_arguments(
        &mut self,
        call_id: &ToolCallId,
        arguments_text: impl Into<String>,
    ) -> Result<(), RunBufferError> {
        let asst = self.current_assistant()?;
        let entry = asst
            .tool_calls
            .get_mut(call_id)
            .ok_or_else(|| RunBufferError::UnknownToolCall(call_id.clone()))?;
        entry.arguments_text = arguments_text.into();
        Ok(())
    }

    pub fn set_finish_reason(&mut self, reason: FinishReason) -> Result<(), RunBufferError> {
        self.current_assistant()?.finish_reason = reason;
        Ok(())
    }

    pub fn set_usage(
        &mut self,
        usage: agena_provider::CompletionUsage,
    ) -> Result<(), RunBufferError> {
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

    /// Drop incomplete tool-call builders while retaining streamed assistant
    /// text/reasoning. Cancellation must never turn a partially received tool
    /// invocation into executable durable history.
    pub fn discard_incomplete_tool_calls(&mut self) -> Result<(), RunBufferError> {
        let assistant = self.current_assistant()?;
        assistant.tool_call_order.clear();
        assistant.tool_calls.clear();
        Ok(())
    }

    pub fn set_terminal_status(&mut self, status: ExecutionStatus) -> Result<(), RunBufferError> {
        if matches!(
            status,
            ExecutionStatus::Pending | ExecutionStatus::InProgress
        ) {
            return Err(RunBufferError::NonTerminalMessageStatus(status));
        }
        self.terminal_status = status;
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
        let RunBuffer {
            execution_id,
            run_id,
            terminal_status,
            sections,
        } = self;
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

                    items.push(EventKind::AssistantMessageFinished(
                        AssistantMessageFinished {
                            execution_id,
                            message_id,
                            run_id,
                            created_at: started_at,
                            content,
                            status: terminal_status,
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
                        let arguments = parse_tool_arguments_value(entry.arguments_text.as_str());
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

fn parse_tool_arguments_value(arguments_text: &str) -> Value {
    let body = if arguments_text.trim().is_empty() {
        "{}"
    } else {
        arguments_text
    };

    let mut deserializer = serde_json::Deserializer::from_str(body);
    let Ok(parsed) = Value::deserialize(&mut deserializer) else {
        return Value::String(arguments_text.to_string());
    };

    if deserializer.end().is_err() {
        return Value::String(arguments_text.to_string());
    }

    parsed
}

#[cfg(test)]
mod tests {
    use agena_storage::SequentialIdAllocator;

    use super::*;

    #[test]
    fn history_preserves_invalid_tool_arguments_instead_of_accepting_a_json_prefix() {
        let raw = r#"{"tool":"session.rename"} trailing"#;
        assert_eq!(
            parse_tool_arguments_value(raw),
            Value::String(raw.to_string())
        );
    }

    #[test]
    fn cancelled_partial_tool_call_cannot_become_executable_history() {
        let execution_id = ExecutionId::new();
        let run_id = RunId::new();
        let mut buffer = RunBuffer::new(execution_id, run_id);
        let mut ids = SequentialIdAllocator::starting_at(17);
        let started_at = Utc::now();
        let message_id = buffer.begin_assistant(&mut ids, started_at);
        buffer
            .push_text_delta("partial response")
            .expect("text delta");
        buffer
            .start_tool_call(ToolCallId::from("partial-call"))
            .expect("partial call");
        buffer
            .discard_incomplete_tool_calls()
            .expect("discard partial calls");
        buffer
            .set_terminal_status(ExecutionStatus::Cancelled)
            .expect("terminal status");

        let events = buffer.commit(&mut ids).expect("commit cancelled buffer");
        assert_eq!(events.len(), 1);
        let EventKind::AssistantMessageFinished(message) = &events[0] else {
            panic!("expected assistant terminal event");
        };
        assert_eq!(message.message_id, message_id);
        assert_eq!(message.execution_id, execution_id);
        assert_eq!(message.run_id, run_id);
        assert_eq!(message.status, ExecutionStatus::Cancelled);
        assert_eq!(message.created_at, started_at);
    }

    #[test]
    fn nonterminal_history_status_is_rejected() {
        let mut buffer = RunBuffer::new(ExecutionId::new(), RunId::new());
        assert_eq!(
            buffer.set_terminal_status(ExecutionStatus::InProgress),
            Err(RunBufferError::NonTerminalMessageStatus(
                ExecutionStatus::InProgress
            ))
        );
    }
}
