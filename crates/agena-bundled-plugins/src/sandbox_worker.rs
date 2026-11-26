//! Narrow dispatch for the standalone MXC tool worker. Calling this entry
//! point constructs no plugin host, configuration store, model or database.

use std::path::Path;

use agena_plugin_host::sdk::{PluginError, Result as SdkResult, ToolInvokeOutput};

/// Execute an operation after the parent has authorized its concrete effects
/// and started this worker with the corresponding OS sandbox policy.
pub fn execute_sandbox_tool(
    operation: &str,
    input: serde_json::Value,
    workspace_root: &Path,
) -> SdkResult<ToolInvokeOutput> {
    match operation {
        "fs.write" | "fs.replace" | "fs.read_many" | "fs.stat" => {
            crate::plugins::provided::fs::execute_sandbox_tool(operation, input, workspace_root)
        }
        _ => Err(PluginError::invalid_params(format!(
            "unsupported sandbox worker operation: {operation}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn filesystem_worker_preserves_revision_checks_and_read_results() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let created = execute_sandbox_tool(
            "fs.write",
            json!({"path":"example.txt", "content":"first value\n"}),
            root,
        )
        .unwrap();
        let before = created.payload.unwrap()["sha256"]
            .as_str()
            .unwrap()
            .to_owned();
        let inspected =
            execute_sandbox_tool("fs.stat", json!({"path":"example.txt"}), root).unwrap();
        assert_eq!(inspected.payload.unwrap()["sha256"], before);
        execute_sandbox_tool(
            "fs.replace",
            json!({
                "path":"example.txt", "old":"first", "new":"second",
                "expected_sha256":before,
            }),
            root,
        )
        .unwrap();
        assert!(execute_sandbox_tool(
            "fs.write",
            json!({"path":"example.txt", "content":"stale overwrite", "expected_sha256":before}),
            root,
        ).is_err());
        assert_eq!(
            std::fs::read_to_string(root.join("example.txt")).unwrap(),
            "second value\n"
        );
        let read = execute_sandbox_tool(
            "fs.read_many",
            json!({"paths":["example.txt", "missing.txt"], "max_total_bytes":1024}),
            root,
        )
        .unwrap();
        let payload = read.payload.unwrap();
        assert_eq!(payload["success_count"], 1);
        assert_eq!(payload["error_count"], 1);
        assert_eq!(payload["files"][0]["content"], "second value\n");
    }
}
