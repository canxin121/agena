pub use agena_domain::mcp_elicitation::{McpElicitation, McpElicitationAction, McpElicitationReply};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportedMcpElicitationInput {
    pub session_id: i64,
    pub server: String,
    pub request: McpElicitation,
    /// Present when dispatching ElicitationResult after the user's decision.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply: Option<McpElicitationReply>,
}
