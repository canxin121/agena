use super::{CopilotModelExtension, Deserialize, Serialize, Value, prompt_cache};

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicMessagesRequest {
    pub(crate) model: String,
    pub(crate) max_tokens: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) system: Option<Vec<AnthropicTextBlock>>,
    pub(crate) messages: Vec<AnthropicMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) stream: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) thinking: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) output_config: Option<AnthropicOutputConfig>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub(crate) stop_sequences: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) top_p: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) top_k: Option<u32>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicOutputConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) effort: Option<&'static str>,
}

#[derive(Debug, Serialize)]
pub(crate) struct AnthropicMessage {
    pub(crate) role: String,
    pub(crate) content: Vec<AnthropicTextBlock>,
}

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AnthropicTextBlock {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) thinking: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) signature: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) data: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) source: Option<AnthropicBinarySource>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) input: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) tool_use_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) content: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) cache_control: Option<prompt_cache::PromptCacheControl>,
}

impl AnthropicTextBlock {
    pub(crate) fn text(text: impl Into<String>) -> Self {
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

    pub(crate) fn image(source: AnthropicBinarySource) -> Self {
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

    pub(crate) fn document(source: AnthropicBinarySource) -> Self {
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

    pub(crate) fn tool_use(
        id: impl Into<String>,
        name: impl Into<String>,
        input_json: impl Into<String>,
    ) -> Self {
        let input_json = input_json.into();
        let input = crate::provider::utils::parse_json_object_or_empty(&input_json);
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

    pub(crate) fn tool_result(tool_use_id: impl Into<String>, content: impl Into<String>) -> Self {
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

#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct AnthropicBinarySource {
    #[serde(rename = "type")]
    pub(crate) kind: String,
    pub(crate) media_type: String,
    pub(crate) data: String,
}

impl AnthropicBinarySource {
    pub(crate) fn base64(media_type: impl Into<String>, data: impl Into<String>) -> Self {
        Self {
            kind: "base64".to_owned(),
            media_type: media_type.into(),
            data: data.into(),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub(crate) enum AnthropicModelListResponse {
    Wrapped { data: Vec<AnthropicModel> },
    Bare(Vec<AnthropicModel>),
}

impl AnthropicModelListResponse {
    pub(crate) fn into_items(self) -> Vec<AnthropicModel> {
        match self {
            Self::Wrapped { data } => data,
            Self::Bare(data) => data,
        }
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicModel {
    pub(crate) id: String,
    #[serde(default, flatten)]
    pub(crate) copilot: CopilotModelExtension,
    #[serde(default)]
    pub(crate) display_name: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicMessagesResponse {
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) stop_reason: Option<String>,
    #[serde(default)]
    pub(crate) content: Vec<AnthropicTextBlock>,
    #[serde(default)]
    pub(crate) usage: Option<AnthropicUsage>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct AnthropicUsage {
    #[serde(default)]
    pub(crate) input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) output_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) output_tokens_details: Option<AnthropicOutputTokensDetails>,
    #[serde(default)]
    pub(crate) cache_creation_input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_read_input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) cache_creation: Option<AnthropicCacheCreationUsage>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicOutputTokensDetails {
    #[serde(default)]
    pub(crate) thinking_tokens: Option<u64>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicCacheCreationUsage {
    #[serde(default)]
    pub(crate) ephemeral_1h_input_tokens: Option<u64>,
    #[serde(default)]
    pub(crate) ephemeral_5m_input_tokens: Option<u64>,
}

impl AnthropicCacheCreationUsage {
    pub(crate) fn total_input_tokens(&self) -> u64 {
        self.ephemeral_1h_input_tokens.unwrap_or_default()
            + self.ephemeral_5m_input_tokens.unwrap_or_default()
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub(crate) enum AnthropicSseEvent {
    MessageStart {
        #[serde(default)]
        message: AnthropicSseMessage,
    },
    ContentBlockStart {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        content_block: AnthropicSseContentBlock,
    },
    ContentBlockDelta {
        #[serde(default)]
        index: Option<usize>,
        #[serde(default)]
        delta: AnthropicSseDelta,
    },
    ContentBlockStop {
        #[serde(default)]
        index: Option<usize>,
    },
    MessageDelta {
        #[serde(default)]
        delta: AnthropicSseMessageDelta,
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    MessageStop {
        #[serde(default)]
        usage: Option<AnthropicUsage>,
        #[serde(default)]
        message: Option<AnthropicSseMessage>,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicSseContentBlock {
    #[serde(default, rename = "type")]
    pub(crate) kind: String,
    #[serde(default)]
    pub(crate) id: Option<String>,
    #[serde(default)]
    pub(crate) name: Option<String>,
    #[serde(default)]
    pub(crate) input: Option<Value>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) signature: Option<String>,
    #[serde(default)]
    pub(crate) data: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicSseDelta {
    #[serde(default, rename = "type")]
    pub(crate) kind: Option<String>,
    #[serde(default)]
    pub(crate) text: Option<String>,
    #[serde(default)]
    pub(crate) thinking: Option<String>,
    #[serde(default)]
    pub(crate) signature: Option<String>,
    #[serde(default)]
    pub(crate) partial_json: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicSseMessageDelta {
    #[serde(default)]
    pub(crate) stop_reason: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub(crate) struct AnthropicSseMessage {
    #[serde(default)]
    pub(crate) stop_reason: Option<String>,
    #[serde(default)]
    pub(crate) usage: Option<AnthropicUsage>,
}

#[derive(Debug, Default)]
pub(crate) struct AnthropicToolCallState {
    pub(crate) id: String,
    pub(crate) name: String,
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
