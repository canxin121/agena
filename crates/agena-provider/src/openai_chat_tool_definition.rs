use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Serialize)]
/// Wire shape of a chat tool definition.
pub struct ChatToolDefinition {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: ChatFunctionDefinition,
}

#[derive(Debug, Serialize)]
/// Wire shape of a chat function definition.
pub struct ChatFunctionDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub strict: bool,
}
