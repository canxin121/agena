//! Gemini Live server-message protocol records.

use serde::Deserialize;
use serde_json::Value;

use crate::{GeminiContent, GeminiFunctionCall, GeminiUsageMetadata};

#[derive(Debug, Deserialize)]
/// A server message in the Gemini live API.
pub struct GeminiLiveServerMessage {
    #[serde(default, rename = "usageMetadata")]
    pub usage_metadata: Option<GeminiUsageMetadata>,
    #[serde(default, rename = "setupComplete")]
    pub setup_complete: Option<Value>,
    #[serde(default, rename = "serverContent")]
    pub server_content: Option<GeminiLiveServerContent>,
    #[serde(default, rename = "toolCall")]
    pub tool_call: Option<GeminiLiveToolCall>,
}

#[derive(Debug, Deserialize)]
/// Content of a Gemini live server message.
pub struct GeminiLiveServerContent {
    #[serde(default, rename = "turnComplete")]
    pub turn_complete: Option<bool>,
    #[serde(default, rename = "groundingMetadata")]
    pub grounding_metadata: Option<Value>,
    #[serde(default, rename = "modelTurn")]
    pub model_turn: Option<GeminiContent>,
}

#[derive(Debug, Deserialize)]
/// A tool call in a Gemini live server message.
pub struct GeminiLiveToolCall {
    #[serde(default, rename = "functionCalls")]
    pub function_calls: Vec<GeminiFunctionCall>,
}
