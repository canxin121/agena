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
    execute_sandbox_tool_with_read_filter(operation, input, workspace_root, None)
}

/// Attach the parent's serialized path-policy predicate to recursive scans.
/// The caller retains its PathFilter and finishes it after dispatch to recover
/// a structured policy rejection in addition to the kernel's restrictions.
pub fn execute_sandbox_tool_with_read_filter(
    operation: &str,
    input: serde_json::Value,
    workspace_root: &Path,
    access: Option<agena_tool::path_filter::PathPredicate>,
) -> SdkResult<ToolInvokeOutput> {
    match operation {
        "fs.write" | "fs.replace" | "fs.read_many" | "fs.stat" => {
            crate::plugins::provided::fs::execute_sandbox_tool(operation, input, workspace_root)
        }
        "code.search_ast" | "code.syntax_tree" | "code.rewrite_ast" => {
            crate::plugins::provided::code::execute_sandbox_tool(
                operation,
                input,
                workspace_root,
                access,
            )
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

    #[test]
    fn ast_worker_preserves_preview_and_revision_checked_publication() {
        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let path = root.join("sample.rs");
        let original = "fn calculate() { let value = 1 + 2; }\n";
        std::fs::write(&path, original).unwrap();
        let mut input = json!({
            "path":"sample.rs", "language":"rust",
            "pattern":"$A + $B", "replacement":"$A - $B",
        });
        let preview = execute_sandbox_tool("code.rewrite_ast", input.clone(), root).unwrap();
        let payload = preview.payload.unwrap();
        assert_eq!(payload["applied"], false);
        assert_eq!(payload["changed"], true);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);
        input["apply"] = json!(true);
        input["expected_sha256"] = payload["before_sha256"].clone();
        execute_sandbox_tool("code.rewrite_ast", input.clone(), root).unwrap();
        let updated = std::fs::read_to_string(&path).unwrap();
        assert!(updated.contains("1 - 2"));
        assert!(execute_sandbox_tool("code.rewrite_ast", input, root).is_err());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), updated);
        let syntax =
            execute_sandbox_tool("code.syntax_tree", json!({"path":"sample.rs"}), root).unwrap();
        assert!(syntax.payload.unwrap()["root_kind"].is_string());
    }

    #[test]
    fn ast_worker_applies_recursive_permission_filter() {
        let directory = tempfile::tempdir().unwrap();
        let root = std::fs::canonicalize(directory.path()).unwrap();
        let secret = root.join("secret");
        std::fs::create_dir(&secret).unwrap();
        std::fs::write(root.join("public.rs"), "fn visible() {}\n").unwrap();
        std::fs::write(secret.join("hidden.rs"), "fn hidden() {}\n").unwrap();
        let access = agena_tool::path_filter::PathFilter::new(move |path| {
            if path.starts_with(&secret) {
                Err(PluginError::invalid_params("secret directory is denied"))
            } else {
                Ok(())
            }
        });
        let result = execute_sandbox_tool_with_read_filter(
            "code.search_ast",
            json!({"path":".", "language":"rust", "pattern":"fn $NAME() {}"}),
            &root,
            Some(access.predicate()),
        );
        let error = access.finish(result).unwrap_err();
        assert!(error.to_string().contains("secret directory is denied"));
    }
}
