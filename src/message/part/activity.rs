use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ExecutionStatus, TimeRange};

fn default_execution_pending() -> ExecutionStatus {
    ExecutionStatus::Pending
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TextPart {
    pub text: String,
    #[serde(default, skip_serializing_if = "is_false")]
    pub synthetic: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ignored: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReasoningPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub summary: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub raw_content: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub encrypted_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CommandExecutionPart {
    pub command: String,
    #[serde(default = "default_execution_pending")]
    pub status: ExecutionStatus,
    #[serde(default)]
    pub lifecycle: TimeRange,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangePart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changes: Vec<FileChangeEntry>,
    #[serde(default = "default_execution_pending")]
    pub status: ExecutionStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FileChangeEntry {
    pub path: String,
    pub kind: FileChangeKind,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FileChangeKind {
    Added,
    Updated,
    Deleted,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchPart {
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<WebSearchResult>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WebSearchResult {
    pub title: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoListPart {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TodoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoItem {
    pub content: String,
    pub status: TodoStatus,
    pub priority: TodoPriority,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Completed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TodoPriority {
    High,
    Medium,
    Low,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ErrorPart {
    pub code: String,
    pub message: String,
}
