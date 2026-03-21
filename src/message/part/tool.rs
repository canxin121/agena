use serde::{Deserialize, Serialize};

use crate::message::status::ToolCallStatus;

use super::{CompletedTime, ErrorTime, RunningTime, StructuredObject};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolAttachment {
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub mime: String,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    Bash,
    Read,
    Write,
    Edit,
    Glob,
    Grep,
    Task,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BashToolInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ReadToolInput {
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WriteToolInput {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EditToolInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlobToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct GrepToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TaskToolInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomToolInput {
    pub name: String,
    #[serde(default)]
    pub args: StructuredObject,
}

/// Tool input is fully static for built-ins, with a single Custom escape hatch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolInput {
    Bash(BashToolInput),
    Read(ReadToolInput),
    Write(WriteToolInput),
    Edit(EditToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(TaskToolInput),
    Custom(CustomToolInput),
}

impl ToolInput {
    pub const fn kind(&self) -> ToolKind {
        match self {
            Self::Bash(_) => ToolKind::Bash,
            Self::Read(_) => ToolKind::Read,
            Self::Write(_) => ToolKind::Write,
            Self::Edit(_) => ToolKind::Edit,
            Self::Glob(_) => ToolKind::Glob,
            Self::Grep(_) => ToolKind::Grep,
            Self::Task(_) => ToolKind::Task,
            Self::Custom(_) => ToolKind::Custom,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct BashToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ReadToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub truncated: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub loaded_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct WriteToolMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct EditToolMetadata {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub diagnostics: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diff: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_diff: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GlobToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct GrepToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matches: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct TaskToolMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_provider_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CustomToolMetadata {
    #[serde(default)]
    pub fields: StructuredObject,
}

/// Tool metadata is fixed for built-ins, with a single Custom variant.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum ToolMetadata {
    Bash(BashToolMetadata),
    Read(ReadToolMetadata),
    Write(WriteToolMetadata),
    Edit(EditToolMetadata),
    Glob(GlobToolMetadata),
    Grep(GrepToolMetadata),
    Task(TaskToolMetadata),
    Custom(CustomToolMetadata),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum ToolState {
    Pending {
        input: ToolInput,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        raw: Option<String>,
    },
    Running {
        input: ToolInput,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<ToolMetadata>,
        #[serde(default)]
        time: RunningTime,
    },
    Completed {
        input: ToolInput,
        #[serde(default)]
        output: String,
        #[serde(default)]
        title: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<ToolMetadata>,
        #[serde(default)]
        time: CompletedTime,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolAttachment>,
    },
    Error {
        input: ToolInput,
        #[serde(default)]
        error: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        metadata: Option<ToolMetadata>,
        #[serde(default)]
        time: ErrorTime,
    },
}

impl ToolState {
    pub const fn status(&self) -> ToolCallStatus {
        match self {
            Self::Pending { .. } => ToolCallStatus::Pending,
            Self::Running { .. } => ToolCallStatus::Running,
            Self::Completed { .. } => ToolCallStatus::Completed,
            Self::Error { .. } => ToolCallStatus::Error,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallPart {
    pub id: String,
    pub input: ToolInput,
    #[serde(default)]
    pub status: ToolCallStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<ToolState>,
}

impl ToolCallPart {
    pub const fn tool_kind(&self) -> ToolKind {
        self.input.kind()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolResultPart {
    pub tool_call_id: String,
    pub content: String,
    pub is_error: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<ToolMetadata>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<ToolAttachment>,
}
