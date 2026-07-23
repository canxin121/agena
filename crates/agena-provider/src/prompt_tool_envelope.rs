//! Provider-neutral wire values for Agena's prompt-envelope tool transport.
//!
//! The surrounding prompt construction and execution policy can vary by
//! adapter, but the serialized call envelope is part of the provider-facing
//! protocol. Keeping it here makes non-streaming responses, stream decoders,
//! and history projection agree on one strict JSON shape.

use serde::{Deserialize, Serialize};

/// A complete prompt-envelope payload containing one or more declared Tool
/// API function calls.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptToolCallsEnvelope {
    pub calls: Vec<PromptToolCall>,
}

/// One declared Tool API function call in the prompt-envelope protocol.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PromptToolCall {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// One Tool API function declaration rendered into the prompt protocol.
#[derive(Debug, Clone, Serialize)]
pub struct PromptToolDefinition {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub parameters: serde_json::Value,
    pub strict: bool,
}

/// A terminal Tool API receipt rendered into prompt replay history.
#[derive(Debug, Serialize)]
pub struct PromptToolResult<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub arguments: serde_json::Value,
    pub status: &'static str,
    pub output: &'a str,
}
