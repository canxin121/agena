//! Authenticated MCP App documents and guest requests.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAppDocument {
    pub name: String,
    pub revision: String,
    pub tool_catalog_revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_catalog_revision: Option<String>,
    pub html: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub ui: Value,
    pub arguments: Map<String, Value>,
    pub tool_result: Value,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum McpAppCatalog {
    Tools,
    Resources,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAppCatalogChange {
    pub revision: String,
    pub retry_after_ms: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppViewPublication {
    pub tools: Vec<agena_domain::mcp_app_tools::McpAppToolDefinition>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub task_extension: bool,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct McpAppViewOpened {
    pub view: Uuid,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum McpAppRequest {
    #[serde(rename = "tools/list")]
    ListTools {
        #[serde(default)]
        cursor: Option<String>,
    },
    #[serde(rename = "tools/call")]
    CallTool {
        name: String,
        #[serde(default)]
        arguments: Map<String, Value>,
    },
    #[serde(rename = "resources/list")]
    ListResources {
        #[serde(default)]
        cursor: Option<String>,
    },
    #[serde(rename = "resources/templates/list")]
    ListResourceTemplates {
        #[serde(default)]
        cursor: Option<String>,
    },
    #[serde(rename = "resources/read")]
    ReadResource { uri: String },
}

/// Requests scoped to the authenticated App view that starts the work.
#[derive(Debug, Deserialize, Serialize)]
#[serde(tag = "method", content = "params", deny_unknown_fields)]
pub enum McpAppServerRequest {
    #[serde(rename = "tools/call")]
    CallTool {
        name: String,
        #[serde(default)]
        arguments: Map<String, Value>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        task: Option<McpAppLegacyTask>,
    },
    #[serde(rename = "resources/read")]
    ReadResource { uri: String },
    #[serde(rename = "sampling/createMessage")]
    Sample(Map<String, Value>),
    #[serde(rename = "tasks/get")]
    Get {
        #[serde(rename = "taskId")]
        task_id: String,
    },
    #[serde(rename = "tasks/update")]
    Update {
        #[serde(rename = "taskId")]
        task_id: String,
        #[serde(rename = "inputResponses")]
        input_responses: BTreeMap<String, Value>,
    },
    #[serde(rename = "tasks/result")]
    Result {
        #[serde(rename = "taskId")]
        task_id: String,
    },
    #[serde(rename = "tasks/cancel")]
    Cancel {
        #[serde(rename = "taskId")]
        task_id: String,
    },
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpAppLegacyTask {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ttl: Option<u64>,
}
