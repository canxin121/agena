//! Shared Gemini content and function-call wire records.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiContent {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    pub parts: Vec<GeminiPart>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct GeminiPart {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thought: Option<bool>,
    #[serde(
        default,
        rename = "thoughtSignature",
        skip_serializing_if = "Option::is_none"
    )]
    pub thought_signature: Option<String>,
    #[serde(
        default,
        rename = "inlineData",
        skip_serializing_if = "Option::is_none"
    )]
    pub inline_data: Option<GeminiInlineData>,
    #[serde(
        default,
        rename = "functionCall",
        skip_serializing_if = "Option::is_none"
    )]
    pub function_call: Option<GeminiFunctionCall>,
    #[serde(
        default,
        rename = "functionResponse",
        skip_serializing_if = "Option::is_none"
    )]
    pub function_response: Option<GeminiFunctionResponse>,
}

impl GeminiPart {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: Some(text.into()),
            ..Self::default()
        }
    }

    pub fn inline_data(mime_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            inline_data: Some(GeminiInlineData {
                mime_type: mime_type.into(),
                data: data.into(),
            }),
            ..Self::default()
        }
    }

    pub fn function_call(
        id: Option<String>,
        name: impl Into<String>,
        args: Value,
        thought_signature: Option<String>,
    ) -> Self {
        Self {
            function_call: Some(GeminiFunctionCall {
                id,
                name: name.into(),
                args,
            }),
            thought_signature,
            ..Self::default()
        }
    }

    pub fn function_response(id: Option<String>, name: impl Into<String>, response: Value) -> Self {
        Self {
            function_response: Some(GeminiFunctionResponse {
                id,
                name: name.into(),
                response,
            }),
            ..Self::default()
        }
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct GeminiFunctionCall {
    #[serde(default)]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiFunctionResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub response: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct GeminiInlineData {
    #[serde(rename = "mimeType")]
    pub mime_type: String,
    pub data: String,
}
