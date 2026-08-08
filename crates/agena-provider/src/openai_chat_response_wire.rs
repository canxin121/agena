use serde::Deserialize;
use serde_json::Value;

use crate::ChatUsage;

#[derive(Debug, Deserialize)]
/// Wire shape of an OpenAI-compatible chat completion response.
pub struct ChatCompletionResponse {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub choices: Vec<ChatCompletionChoice>,
    #[serde(default)]
    pub usage: Option<ChatUsage>,
    #[serde(default)]
    pub error: Option<Value>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of a chat completion choice.
pub struct ChatCompletionChoice {
    #[serde(default)]
    pub message: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    pub delta: Option<ChatDeltaOrMessage>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub finish_reason: Option<String>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of a chat message or delta.
pub struct ChatDeltaOrMessage {
    #[serde(default)]
    pub content: Option<Value>,
    #[serde(default)]
    pub reasoning_content: Option<Value>,
    #[serde(default)]
    pub reasoning_details: Option<Value>,
    #[serde(default)]
    pub reasoning_text: Option<Value>,
    #[serde(default)]
    pub reasoning_opaque: Option<String>,
    #[serde(default)]
    pub tool_calls: Option<Vec<ChatToolCallWire>>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of a chat tool call.
pub struct ChatToolCallWire {
    #[serde(default)]
    pub index: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub function: Option<ChatFunctionCallWire>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of a chat function call.
pub struct ChatFunctionCallWire {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Option<String>,
}
