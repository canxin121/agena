use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum McpElicitation {
    Form {
        message: String,
        requested_schema: Value,
    },
    Url {
        message: String,
        url: String,
        elicitation_id: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum McpElicitationAction {
    Accept,
    Decline,
    Cancel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct McpElicitationReply {
    pub action: McpElicitationAction,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Value>,
}

impl McpElicitationReply {
    pub fn new(action: McpElicitationAction) -> Self {
        Self {
            action,
            content: None,
        }
    }

    pub fn prefer(&mut self, other: Self) {
        if self.action != McpElicitationAction::Decline {
            *self = other;
        }
    }
}
