use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ChatToolCallRequest, PromptCacheControl};

#[derive(Debug, Serialize, Deserialize)]
/// Wire shape of an OpenAI-compatible chat message.
pub struct ChatMessage {
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none", rename = "type")]
    pub kind: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_details: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning_opaque: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ChatToolCallRequest>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub copilot_cache_control: Option<PromptCacheControl>,
}

impl ChatMessage {
    pub fn system(content: String) -> Self {
        Self::new("system", Some(Value::String(content)), None)
    }
    pub fn user(content: Value) -> Self {
        Self::new("user", Some(content), None)
    }
    pub fn assistant(content: Option<Value>, tool_calls: Option<Vec<ChatToolCallRequest>>) -> Self {
        Self::new("assistant", content, tool_calls)
    }
    pub fn tool_result(tool_call_id: String, content: Value) -> Self {
        let mut message = Self::new("tool", Some(content), None);
        message.tool_call_id = Some(tool_call_id);
        message
    }
    fn new(
        role: &str,
        content: Option<Value>,
        tool_calls: Option<Vec<ChatToolCallRequest>>,
    ) -> Self {
        Self {
            role: role.to_owned(),
            kind: None,
            content,
            reasoning_content: None,
            reasoning_details: None,
            reasoning_text: None,
            reasoning_opaque: None,
            tool_calls,
            tool_call_id: None,
            copilot_cache_control: None,
        }
    }
}
