//! Operator-selected MCP channels on one project session.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum ClaudeChannelAction {
    Enable {
        server: String,
        #[serde(default)]
        permission_relay: bool,
    },
    Disable {
        server: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeChannelView {
    pub server: String,
    pub permission_relay: bool,
    pub connected: bool,
}

/// A declared plugin channel visible to the operator of a project session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeChannelDefinition {
    pub server: String,
    pub display_name: String,
    pub plugin_id: String,
    pub declared: bool,
    pub registered: bool,
    pub user_options: Vec<ClaudeChannelOption>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClaudeChannelOption {
    pub key: String,
    #[serde(rename = "type")]
    pub kind: ClaudeChannelOptionKind,
    pub title: String,
    pub description: String,
    pub required: bool,
    pub sensitive: bool,
    pub configured: bool,
    pub multiple: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeChannelOptionKind {
    String,
    Number,
    Boolean,
    Directory,
    File,
}
