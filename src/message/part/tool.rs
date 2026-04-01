use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use strum::Display;

use super::{ExecutionStatus, StructuredObject, TimeRange, TodoItem};

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct ToolAttachment {
    pub url: String,
    #[serde(default)]
    pub filename: String,
    #[serde(default)]
    pub mime: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct BashToolInput {
    pub command: String,
    #[serde(default)]
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workdir: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ReadToolInput {
    pub file_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offset: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct WriteToolInput {
    pub file_path: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct EditToolInput {
    pub file_path: String,
    pub old_string: String,
    pub new_string: String,
    #[serde(default)]
    pub replace_all: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct GlobToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct GrepToolInput {
    pub pattern: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema, Display)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum TaskSubagentType {
    Explore,
    Implement,
    Verify,
}

impl TaskSubagentType {
    pub const fn guidance(self) -> &'static str {
        match self {
            Self::Explore => {
                "Focus on understanding the codebase, collecting evidence, and reporting findings without making edits."
            }
            Self::Implement => {
                "Own the requested code changes, adapt to concurrent edits, and avoid reverting unrelated work."
            }
            Self::Verify => {
                "Validate behavior with targeted checks, look for regressions, and summarize remaining risks."
            }
        }
    }

    pub fn apply_prompt_guidance(self, prompt: &str) -> String {
        let trimmed = prompt.trim();
        if trimmed.is_empty() {
            format!("Profile guidance: {}", self.guidance())
        } else {
            format!(
                "Profile guidance: {}\n\nDelegated task:\n{}",
                self.guidance(),
                trimmed
            )
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TaskToolInput {
    pub description: String,
    pub prompt: String,
    pub subagent_type: TaskSubagentType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ToolSearchToolInput {
    #[serde(default)]
    pub query: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct TodoWriteToolInput {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<TodoItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
pub struct ApplyPatchToolInput {
    pub patch: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Display)]
#[serde(tag = "tool", rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum BuiltinToolInput {
    Bash(BashToolInput),
    Read(ReadToolInput),
    Write(WriteToolInput),
    Edit(EditToolInput),
    ApplyPatch(ApplyPatchToolInput),
    Glob(GlobToolInput),
    Grep(GrepToolInput),
    Task(TaskToolInput),
    ToolSearch(ToolSearchToolInput),
    TodoWrite(TodoWriteToolInput),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ToolInvocation {
    Builtin {
        input: BuiltinToolInput,
    },
    Mcp {
        server: String,
        tool: String,
        #[serde(default)]
        input: StructuredObject,
    },
    Custom {
        name: String,
        #[serde(default)]
        input: StructuredObject,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "tool", rename_all = "snake_case")]
pub enum BuiltinToolOutput {
    Bash {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Read {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        preview: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        truncated: Option<bool>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_paths: Vec<String>,
    },
    Write {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    },
    Edit {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        diagnostics: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        diff: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        file_diff: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
    },
    ApplyPatch {
        operation_id: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        files: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        before_hash: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after_hash: Option<String>,
        inverse_patch: String,
    },
    Glob {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        count: Option<u32>,
    },
    Grep {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        matches: Option<u32>,
    },
    Task {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_provider_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_id: Option<String>,
    },
    ToolSearch {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        results: Vec<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        loaded_tools: Vec<String>,
    },
    TodoWrite {
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        items: Vec<TodoItem>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ToolResultBlock {
    Text {
        text: String,
    },
    Image {
        mime: String,
        url: String,
    },
    Audio {
        mime: String,
        url: String,
    },
    ResourceLink {
        uri: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    EmbeddedResource {
        uri: String,
        mime: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        base64: Option<String>,
    },
    File {
        url: String,
        filename: String,
        mime: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct McpToolOutput {
    pub server: String,
    pub tool: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub content_blocks: Vec<ToolResultBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<StructuredObject>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CustomToolOutput {
    pub name: String,
    #[serde(default)]
    pub payload: StructuredObject,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "source", rename_all = "snake_case")]
pub enum ToolOutput {
    None,
    Builtin {
        output: BuiltinToolOutput,
    },
    Mcp {
        output: McpToolOutput,
    },
    Custom {
        output: CustomToolOutput,
    },
    Unknown {
        name: String,
        #[serde(default)]
        payload: StructuredObject,
    },
}

impl Default for ToolOutput {
    fn default() -> Self {
        Self::None
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ToolExecutionPart {
    Pending {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    InProgress {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        title: String,
        #[serde(default, skip_serializing_if = "String::is_empty")]
        output_text: String,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    Completed {
        call_id: i64,
        invocation: ToolInvocation,
        #[serde(default)]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ToolResultBlock>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolAttachment>,
        #[serde(default)]
        details: ToolOutput,
        #[serde(default)]
        lifecycle: TimeRange,
    },
    Failed {
        call_id: i64,
        invocation: ToolInvocation,
        error_message: String,
        #[serde(default)]
        output_text: String,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        blocks: Vec<ToolResultBlock>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        attachments: Vec<ToolAttachment>,
        #[serde(default)]
        details: ToolOutput,
        #[serde(default)]
        lifecycle: TimeRange,
    },
}

impl ToolExecutionPart {
    pub const fn status(&self) -> ExecutionStatus {
        match self {
            Self::Pending { .. } => ExecutionStatus::Pending,
            Self::InProgress { .. } => ExecutionStatus::InProgress,
            Self::Completed { .. } => ExecutionStatus::Completed,
            Self::Failed { .. } => ExecutionStatus::Failed,
        }
    }

    pub fn append_output_delta(&mut self, delta: &str) -> bool {
        match self {
            Self::Pending {
                call_id,
                invocation,
                title,
                lifecycle,
            } => {
                *self = Self::InProgress {
                    call_id: call_id.clone(),
                    invocation: invocation.clone(),
                    title: title.clone(),
                    output_text: delta.to_string(),
                    lifecycle: lifecycle.clone(),
                };
                true
            }
            Self::InProgress { output_text, .. }
            | Self::Completed { output_text, .. }
            | Self::Failed { output_text, .. } => {
                output_text.push_str(delta);
                true
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn task_subagent_type_guidance_wraps_prompt() {
        let prompt = TaskSubagentType::Verify.apply_prompt_guidance("run focused checks");

        assert!(prompt.contains("Validate behavior with targeted checks"));
        assert!(prompt.contains("run focused checks"));
    }

    #[test]
    fn task_subagent_type_serializes_as_snake_case() {
        let value = serde_json::to_string(&TaskSubagentType::Implement)
            .expect("task subagent type should serialize");
        assert_eq!(value, "\"implement\"");
    }
}
