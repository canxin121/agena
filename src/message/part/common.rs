use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum PartKind {
    Text,
    ToolCall,
    ToolResult,
    Reasoning,
    File,
    StepStart,
    StepFinish,
    Snapshot,
    Patch,
    Agent,
    Subtask,
    Retry,
    Compaction,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct RunningTime {
    pub start: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CompletedTime {
    pub start: i64,
    pub end: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compacted: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ErrorTime {
    pub start: i64,
    pub end: i64,
}
