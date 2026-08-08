use serde::Serialize;
use serde_json::Value;

use crate::ResponseFormat;

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
/// Wire shape of an OpenAI-compatible response format.
pub enum ChatResponseFormat {
    Text,
    JsonObject,
    #[serde(rename = "json_schema")]
    JsonSchema {
        json_schema: ChatJsonSchemaSpec,
    },
}

#[derive(Debug, Serialize)]
/// Wire shape of an OpenAI-compatible JSON schema response format.
pub struct ChatJsonSchemaSpec {
    pub name: String,
    pub schema: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strict: bool,
}

pub fn openai_chat_response_format(fmt: Option<&ResponseFormat>) -> Option<ChatResponseFormat> {
    match fmt? {
        ResponseFormat::Text => Some(ChatResponseFormat::Text),
        ResponseFormat::JsonObject => Some(ChatResponseFormat::JsonObject),
        ResponseFormat::JsonSchema {
            name,
            schema,
            strict,
        } => Some(ChatResponseFormat::JsonSchema {
            json_schema: ChatJsonSchemaSpec {
                name: name.clone(),
                schema: schema.clone(),
                strict: *strict,
            },
        }),
    }
}
