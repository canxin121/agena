//! Session-scoped model sampling requested by an MCP server.

use serde_json::{Map, Value};

use crate::Role;

pub const MAX_INPUT_BYTES: usize = 128 * 1024;
pub const MAX_OUTPUT_TOKENS: u32 = 8192;
pub const MAX_MEDIA_BLOCKS: usize = 8;
pub const MAX_MEDIA_BLOCK_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_MEDIA_TOTAL_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_TOOLS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum McpSamplingIncludeContext {
    #[default]
    None,
    ThisServer,
    AllServers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpSamplingToolChoice {
    Auto,
    Required,
    None,
}

#[derive(Debug, Clone, PartialEq)]
pub struct McpSamplingTool {
    pub name: String,
    pub description: Option<String>,
    pub input_schema: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpSamplingToolResultBlock {
    Text(String),
    Image { mime_type: String, data: String },
    Audio { mime_type: String, data: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpSamplingBlock {
    Text(String),
    Image {
        mime_type: String,
        data: String,
    },
    Audio {
        mime_type: String,
        data: String,
    },
    ToolUse {
        id: String,
        name: String,
        input: Map<String, Value>,
    },
    ToolResult {
        tool_use_id: String,
        content: Vec<McpSamplingToolResultBlock>,
        is_error: bool,
    },
}

#[derive(Debug, Clone)]
pub struct McpSamplingRequest {
    pub messages: Vec<McpSamplingMessage>,
    pub system_prompt: Option<String>,
    pub include_context: McpSamplingIncludeContext,
    pub tools: Vec<McpSamplingTool>,
    pub tool_choice: Option<McpSamplingToolChoice>,
    pub max_tokens: u32,
    pub temperature: Option<f32>,
    pub stop_sequences: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct McpSamplingMessage {
    pub role: Role,
    pub content: Vec<McpSamplingBlock>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum McpSamplingReplyBlock {
    Text(String),
    ToolUse {
        id: String,
        name: String,
        input: Map<String, Value>,
    },
}

#[derive(Debug, Clone)]
pub struct McpSamplingReply {
    pub model: String,
    pub content: Vec<McpSamplingReplyBlock>,
    pub stop_reason: Option<String>,
}
