//! Tools contributed by a live MCP App view.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

use crate::UserInputQuestion;

pub const CALL_MCP_APP_TOOL: &str = "agena.mcp.apps.call";
pub const MCP_APP_TOOL_MESSAGE_LIMIT: usize = 128 * 1024;

/// One request the server is waiting for the App to answer.
pub struct McpAppTaskInput {
    pub request: Value,
    validate: Box<dyn Fn(Value) -> Option<Value> + Send + Sync>,
}

impl McpAppTaskInput {
    pub fn new(
        request: Value,
        validate: impl Fn(Value) -> Option<Value> + Send + Sync + 'static,
    ) -> Self {
        Self {
            request,
            validate: Box::new(validate),
        }
    }

    pub fn validate_response(&self, response: Value) -> Option<Value> {
        (self.validate)(response)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAppViewBinding {
    pub session_id: i64,
    pub part_id: i64,
    pub revision: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpAppViewScope {
    pub binding: McpAppViewBinding,
    pub view: Uuid,
}

/// A question from Agena to the user of the App view.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAppPrompt {
    pub title: String,
    pub question: UserInputQuestion,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppToolDefinition {
    pub name: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    pub input_schema: Value,
    #[serde(default)]
    pub output_schema: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution: Option<McpAppToolExecution>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpAppToolExecution {
    #[serde(default)]
    pub task_support: McpAppTaskSupport,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpAppTaskSupport {
    #[default]
    Forbidden,
    Optional,
    Required,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAppToolCall {
    pub id: Uuid,
    #[serde(flatten)]
    pub request: McpAppToolRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum McpAppToolRequest {
    Tool {
        name: String,
        arguments: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<Map<String, Value>>,
        #[serde(
            rename = "requestState",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        request_state: Option<String>,
        #[serde(
            rename = "inputResponses",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        input_responses: Option<BTreeMap<String, Value>>,
    },
    Task {
        method: McpAppTaskMethod,
        #[serde(rename = "taskId")]
        task_id: String,
        #[serde(
            rename = "inputResponses",
            default,
            skip_serializing_if = "Option::is_none"
        )]
        input_responses: Option<BTreeMap<String, Value>>,
    },
    Input {
        method: McpAppInputMethod,
        params: Map<String, Value>,
    },
    Prompt {
        prompt: McpAppPrompt,
    },
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum McpAppInputMethod {
    #[serde(rename = "elicitation/create")]
    Elicit,
    #[serde(rename = "sampling/createMessage")]
    Sample,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum McpAppTaskMethod {
    #[serde(rename = "tasks/get")]
    Get,
    #[serde(rename = "tasks/result")]
    Result,
    #[serde(rename = "tasks/update")]
    Update,
    #[serde(rename = "tasks/cancel")]
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpAppToolCalls {
    pub revision: u64,
    pub calls: Vec<McpAppToolCall>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case", deny_unknown_fields)]
pub enum McpAppToolReply {
    Result { result: Value },
    Error { message: String },
    Input { answer: Option<String> },
}

pub struct McpAppToolResult {
    pub app: String,
    pub name: String,
    pub result: Value,
}
