use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
/// Wire shape of a chat tool call request.
pub struct ChatToolCallRequest {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    pub function: ChatFunctionCallRequest,
}

#[derive(Debug, Serialize, Deserialize)]
/// Wire shape of a chat function call request.
pub struct ChatFunctionCallRequest {
    pub name: String,
    pub arguments: String,
}
