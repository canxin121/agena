use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatToolCallRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub function: ChatFunctionCallRequest,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatFunctionCallRequest {
    pub name: String,
    pub arguments: String,
}
