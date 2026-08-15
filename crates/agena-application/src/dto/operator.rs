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

#[cfg(test)]
mod tests {
    use super::OperatorToolInvokeRequest;

    #[test]
    fn operator_invoke_requires_an_explicit_workspace_identity() {
        assert!(
            serde_json::from_value::<OperatorToolInvokeRequest>(serde_json::json!({
                "tool": "fs.read",
                "input": {"path": "README.md"}
            }))
            .is_err()
        );
        let request = serde_json::from_value::<OperatorToolInvokeRequest>(serde_json::json!({
            "workspace_id": 42,
            "tool": "fs.read",
            "input": {"path": "README.md"}
        }))
        .expect("decode workspace-bound operator invocation");
        assert_eq!(request.workspace_id, 42);
    }
}

use super::{Deserialize, Serialize};
