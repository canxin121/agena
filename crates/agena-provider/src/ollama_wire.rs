//! Ollama chat and model-list protocol records.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Deserialize)]
/// Wire shape of an Ollama tags response.
pub struct OllamaTagsResponse {
    #[serde(default)]
    pub models: Vec<OllamaTagModel>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Ollama tag model entry.
pub struct OllamaTagModel {
    pub name: Option<String>,
    pub model: Option<String>,
    #[serde(default)]
    pub details: Option<OllamaModelDetails>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of Ollama model details.
pub struct OllamaModelDetails {
    #[serde(default)]
    pub family: Option<String>,
}

#[derive(Debug, Serialize)]
/// Wire shape of an Ollama chat request.
pub struct OllamaChatRequest {
    pub model: String,
    pub messages: Vec<OllamaChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<OllamaToolDefinition>>,
    pub stream: bool,
    #[serde(skip_serializing_if = "OllamaOptions::is_empty")]
    pub options: OllamaOptions,
}

#[derive(Debug, Serialize)]
/// Wire shape of an Ollama chat message.
pub struct OllamaChatMessage {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Serialize, Default)]
/// Wire shape of Ollama generation options.
pub struct OllamaOptions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub num_predict: Option<u32>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub stop: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub seed: Option<u64>,
}

impl OllamaOptions {
    pub fn is_empty(&self) -> bool {
        self.temperature.is_none()
            && self.num_predict.is_none()
            && self.stop.is_empty()
            && self.top_p.is_none()
            && self.top_k.is_none()
            && self.seed.is_none()
    }
}

#[derive(Debug, Serialize)]
/// Wire shape of an Ollama tool definition.
pub struct OllamaToolDefinition {
    #[serde(rename = "type")]
    pub kind: &'static str,
    pub function: OllamaFunctionDefinition,
}

#[derive(Debug, Serialize)]
/// Wire shape of an Ollama function definition.
pub struct OllamaFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Deserialize, Default)]
/// Wire shape of an Ollama chat message response.
pub struct OllamaChatMessageResponse {
    #[serde(default)]
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<OllamaToolCall>,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Ollama tool call.
pub struct OllamaToolCall {
    pub function: OllamaFunctionCall,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Ollama function call.
pub struct OllamaFunctionCall {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub arguments: Value,
}

#[derive(Debug, Deserialize)]
/// Wire shape of an Ollama chat response.
pub struct OllamaChatResponse {
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub message: Option<OllamaChatMessageResponse>,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub done_reason: Option<String>,
    #[serde(default)]
    pub prompt_eval_count: Option<u64>,
    #[serde(default)]
    pub eval_count: Option<u64>,
}
