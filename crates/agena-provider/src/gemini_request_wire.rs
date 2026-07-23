//! Gemini generate-content and Live request protocol records.

use serde::Serialize;
use serde_json::Value;

use crate::{GeminiContent, GeminiPart, GeminiThinkingConfig};

#[derive(Debug, Serialize)]
pub struct GeminiGenerateRequest {
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiInstruction>,
    pub contents: Vec<GeminiContent>,
    #[serde(rename = "generationConfig")]
    pub generation_config: GeminiGenerationConfig,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(rename = "toolConfig", skip_serializing_if = "Option::is_none")]
    pub tool_config: Option<GeminiToolConfig>,
}

#[derive(Debug, Serialize)]
pub struct GeminiInstruction {
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize)]
pub struct GeminiFunctionDeclaration {
    pub name: String,
    pub description: String,
    #[serde(
        rename = "parametersJsonSchema",
        skip_serializing_if = "Option::is_none"
    )]
    pub parameters_json_schema: Option<Value>,
}

#[derive(Debug, Serialize)]
pub struct GeminiToolConfig {
    #[serde(rename = "functionCallingConfig")]
    pub function_calling_config: GeminiFunctionCallingConfig,
}

#[derive(Debug, Serialize)]
pub struct GeminiFunctionCallingConfig {
    pub mode: String,
}

#[derive(Debug, Serialize)]
pub struct GeminiGenerationConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub temperature: Option<f32>,
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u32>,
    #[serde(rename = "topP", skip_serializing_if = "Option::is_none")]
    pub top_p: Option<f32>,
    #[serde(rename = "topK", skip_serializing_if = "Option::is_none")]
    pub top_k: Option<u32>,
    #[serde(
        rename = "stopSequences",
        skip_serializing_if = "Vec::is_empty",
        default
    )]
    pub stop_sequences: Vec<String>,
    #[serde(rename = "responseMimeType", skip_serializing_if = "Option::is_none")]
    pub response_mime_type: Option<String>,
    #[serde(rename = "responseJsonSchema", skip_serializing_if = "Option::is_none")]
    pub response_json_schema: Option<Value>,
    #[serde(rename = "thinkingConfig", skip_serializing_if = "Option::is_none")]
    pub thinking_config: Option<GeminiThinkingConfig>,
    #[serde(rename = "responseModalities", skip_serializing_if = "Option::is_none")]
    pub response_modalities: Option<Vec<String>>,
}

#[derive(Debug, Serialize)]
pub struct GeminiLiveConversationRequest {
    pub setup: GeminiLiveSetup,
    #[serde(rename = "clientContent")]
    pub client_content: GeminiLiveClientContent,
}

#[derive(Debug, Serialize)]
pub struct GeminiLiveSetup {
    pub model: String,
    #[serde(rename = "generationConfig")]
    pub generation_config: GeminiGenerationConfig,
    #[serde(rename = "systemInstruction", skip_serializing_if = "Option::is_none")]
    pub system_instruction: Option<GeminiInstruction>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
}

#[derive(Debug, Serialize)]
pub struct GeminiLiveClientContent {
    pub turns: Vec<GeminiContent>,
    #[serde(rename = "turnComplete", skip_serializing_if = "Option::is_none")]
    pub turn_complete: Option<bool>,
}
