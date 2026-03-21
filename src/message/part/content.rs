use serde::{Deserialize, Serialize};

use super::{
    AgentPart, CompactionPart, FilePart, PartKind, PatchPart, ReasoningPart, RetryPart,
    SnapshotPart, StepFinishPart, StepStartPart, SubtaskPart, TextPart, ToolCallPart,
    ToolResultPart,
};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PartContent {
    Text(TextPart),
    ToolCall(ToolCallPart),
    ToolResult(ToolResultPart),
    Reasoning(ReasoningPart),
    File(FilePart),
    StepStart(StepStartPart),
    StepFinish(StepFinishPart),
    Snapshot(SnapshotPart),
    Patch(PatchPart),
    Agent(AgentPart),
    Subtask(SubtaskPart),
    Retry(RetryPart),
    Compaction(CompactionPart),
}

/// Backward-compatible alias for older call-sites.
pub type PartType = PartContent;

impl PartContent {
    pub fn text(text: impl Into<String>) -> Self {
        Self::Text(TextPart {
            text: text.into(),
            synthetic: None,
            ignored: None,
        })
    }

    pub fn reasoning(text: impl Into<String>) -> Self {
        Self::Reasoning(ReasoningPart { text: text.into() })
    }

    pub const fn kind(&self) -> PartKind {
        match self {
            Self::Text(_) => PartKind::Text,
            Self::ToolCall(_) => PartKind::ToolCall,
            Self::ToolResult(_) => PartKind::ToolResult,
            Self::Reasoning(_) => PartKind::Reasoning,
            Self::File(_) => PartKind::File,
            Self::StepStart(_) => PartKind::StepStart,
            Self::StepFinish(_) => PartKind::StepFinish,
            Self::Snapshot(_) => PartKind::Snapshot,
            Self::Patch(_) => PartKind::Patch,
            Self::Agent(_) => PartKind::Agent,
            Self::Subtask(_) => PartKind::Subtask,
            Self::Retry(_) => PartKind::Retry,
            Self::Compaction(_) => PartKind::Compaction,
        }
    }

    pub fn text_value(&self) -> Option<&str> {
        match self {
            Self::Text(part) => Some(part.text.as_str()),
            _ => None,
        }
    }

    pub fn reasoning_value(&self) -> Option<&str> {
        match self {
            Self::Reasoning(part) => Some(part.text.as_str()),
            _ => None,
        }
    }
}
