use super::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
/// Public descriptor for a Runtime tool owned by the server.
pub struct OperatorToolResource {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_help: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_help: Option<String>,
    pub input_schema: serde_json::Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<serde_json::Value>,
    /// Whether this tool is marked as interactive by its declared tags or
    /// discovery metadata. Consumers that cannot service interaction
    /// should hide it.
    #[serde(default)]
    pub interactive: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub open_world: bool,
    #[serde(default)]
    pub agent: bool,
    /// Stable full plugin id (for example, `agena.fs`).
    pub plugin_id: String,
}

#[derive(Debug, Clone, Deserialize)]
/// Server-owned invocation of a Runtime tool outside a session lifecycle.
pub struct OperatorToolInvokeRequest {
    /// Database-backed workspace identity the caller intends to operate on.
    /// The server resolves this id and requires its canonical path to equal
    /// the workspace root of the composed Runtime tool executor.
    pub workspace_id: i64,
    pub tool: String,
    #[serde(default)]
    pub input: Option<serde_json::Value>,
}
