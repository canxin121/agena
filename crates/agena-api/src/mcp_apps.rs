//! Authenticated MCP App documents and guest requests.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAppDocument {
    pub name: String,
    pub revision: String,
    pub html: String,
    pub arguments: Map<String, Value>,
    pub tool_result: Value,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum McpAppRequest {
    #[serde(rename = "tools/call")]
    CallTool {
        name: String,
        #[serde(default)]
        arguments: Map<String, Value>,
    },
    #[serde(rename = "resources/read")]
    ReadResource { uri: String },
}
