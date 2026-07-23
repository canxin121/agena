//! Anthropic Messages text/content wire values.
//!
//! These are provider protocol records shared by request projection and
//! response parsing. They deliberately carry no client, configuration, or
//! session dependency.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::PromptCacheControl;

/// A content block in the Anthropic Messages protocol.
#[derive(Debug, Serialize, Deserialize)]
pub struct AnthropicTextBlock {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AnthropicBinarySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<PromptCacheControl>,
}

impl AnthropicTextBlock {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            kind: "text".to_owned(),
            text: Some(text.into()),
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    pub fn image(source: AnthropicBinarySource) -> Self {
        Self {
            kind: "image".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    pub fn document(source: AnthropicBinarySource) -> Self {
        Self {
            kind: "document".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: Some(source),
            id: None,
            name: None,
            input: None,
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    pub fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input_json: impl Into<String>,
    ) -> Self {
        let input = json_object_or_empty(input_json.into());
        Self {
            kind: "tool_use".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: Some(id.into()),
            name: Some(name.into()),
            input: Some(input),
            tool_use_id: None,
            content: None,
            cache_control: None,
        }
    }

    pub fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
        Self {
            kind: "tool_result".to_owned(),
            text: None,
            thinking: None,
            signature: None,
            data: None,
            source: None,
            id: None,
            name: None,
            input: None,
            tool_use_id: Some(tool_use_id.into()),
            // Anthropic accepts either a string or an array of content blocks
            // here. A parsed JSON object is not a valid tool_result payload.
            content: Some(Value::String(content.into())),
            cache_control: None,
        }
    }
}

/// Base64 content source used by Anthropic image and document blocks.
#[derive(Debug, Serialize, Deserialize)]
pub struct AnthropicBinarySource {
    #[serde(rename = "type")]
    pub kind: String,
    pub media_type: String,
    pub data: String,
}

impl AnthropicBinarySource {
    pub fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: "base64".to_owned(),
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

fn json_object_or_empty(raw: String) -> Value {
    serde_json::from_str::<Value>(raw.as_str())
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_blocks_omit_irrelevant_null_fields() {
        let value =
            serde_json::to_value(AnthropicTextBlock::text("hello")).expect("serialize text block");

        assert_eq!(
            value,
            serde_json::json!({ "type": "text", "text": "hello" })
        );
    }

    #[test]
    fn tool_use_input_is_always_a_json_object() {
        let valid = serde_json::to_value(AnthropicTextBlock::tool_use(
            "toolu_1",
            "lookup",
            r#"{"query":"rust"}"#,
        ))
        .expect("serialize tool use");
        assert_eq!(valid["input"], serde_json::json!({ "query": "rust" }));

        let invalid = serde_json::to_value(AnthropicTextBlock::tool_use(
            "toolu_2",
            "lookup",
            r#"["not", "an", "object"]"#,
        ))
        .expect("serialize tool use");
        assert_eq!(invalid["input"], serde_json::json!({}));
    }

    #[test]
    fn tool_result_text_is_not_mistaken_for_an_arbitrary_json_payload() {
        let value =
            serde_json::to_value(AnthropicTextBlock::tool_result("toolu_1", r#"{"ok":true}"#))
                .expect("serialize tool result");

        assert_eq!(value["content"], serde_json::json!(r#"{"ok":true}"#));
    }
}
