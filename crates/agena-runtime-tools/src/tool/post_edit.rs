//! Post-edit diagnostics are evidence, not a second mutation. Missing servers,
//! permission requirements, slow startup and cancellation are reported without
//! pretending that the already-committed edit failed or that code is clean.
use super::*;
use agena_domain::PermissionDecision;
use serde_json::json;
use std::time::Duration;

pub(super) async fn validate(
    executor: &ToolExecutor,
    invocation: &ToolInvocation,
    session: i64,
    execution: &mut ToolInvocationExecution,
) {
    let name = invocation
        .name
        .strip_prefix("agena.")
        .unwrap_or(&invocation.name);
    if !matches!(
        name,
        "fs.write" | "fs.replace" | "fs.apply_patch" | "notebook.edit_cell"
    ) {
        return;
    }
    let mut paths = execution
        .apply_patch
        .as_ref()
        .map(|patch| {
            patch
                .files
                .iter()
                .map(|file| file.path.clone())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if paths.is_empty()
        && let Some(path) = serde_json::Value::from(invocation.input.clone())
            .get("path")
            .and_then(serde_json::Value::as_str)
    {
        paths.push(path.into());
    }
    if paths.is_empty() {
        return;
    }
    let total = paths.len();
    paths.truncate(8);
    let mut results = Vec::new();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    for path in paths {
        let Some(registry) = executor.lsp_registry() else {
            results.push(json!({"file":path,"state":"not_configured"}));
            continue;
        };
        let target = executor.resolve_target_path(&path);
        if !target.is_file() {
            results.push(json!({"file":path,"state":"not_a_current_file"}));
            continue;
        }
        if registry.server_for_path(&target).await.is_none() {
            results.push(json!({"file":path,"state":"no_matching_server"}));
            continue;
        }
        let call = ToolInvocation::new(
            "lsp.diagnostics",
            StructuredObject::try_from(json!({"file_path":path})).expect("object input"),
        );
        let operation = async {
            let checks = executor
                .collect_permission_checks_for_invocation_in_session(&call, Some(session))
                .await?;
            if checks
                .iter()
                .any(|check| !matches!(check.decision, PermissionDecision::Allow))
            {
                return Ok(json!({"file":path,"state":"permission_required"}));
            }
            let result = super::lsp::execute_diagnostics_async(
                executor,
                &crate::part::LspDiagnosticsToolInput {
                    file_path: path.clone(),
                },
            )
            .await?;
            let payload = serde_json::to_value(result.output)
                .map_err(|error| ToolError::invalid_input_error(&error))?;
            Ok::<_, ToolError>(
                json!({"file":path,"state":payload["state"],"document_version":payload["document_version"],"reported_version":payload["reported_version"],"entries":payload["entries"]}),
            )
        };
        match tokio::time::timeout_at(deadline, operation).await {
            Ok(Ok(result)) => results.push(result),
            Ok(Err(ToolError::Cancelled)) => {
                results.push(json!({"file":path,"state":"cancelled_after_edit"}))
            }
            Ok(Err(_)) => results.push(json!({"file":path,"state":"unavailable_after_edit"})),
            Err(_) => {
                results.push(json!({"file":path,"state":"timeout_after_edit"}));
                break;
            }
        }
    }
    let attempted = results.len();
    let mut text = String::from("\nPost-edit validation (does not alter files):");
    for result in &results {
        let file = result["file"].as_str().unwrap_or_default();
        let state = result["state"].as_str().unwrap_or("unknown");
        let entries = result["entries"].as_array();
        text.push_str(&format!("\n{file}: {state}"));
        if state == "current" {
            text.push_str(&format!(
                " · {} current-version diagnostics",
                entries.map_or(0, Vec::len)
            ));
        } else {
            text.push_str(" · not evidence of clean code");
        }
        if let Some(entries) = entries {
            for entry in entries.iter().take(10) {
                if let Some(entry) = entry.as_str() {
                    text.push_str(&format!(
                        "\n  {}",
                        entry.chars().take(1000).collect::<String>()
                    ));
                }
            }
        }
    }
    if attempted < total {
        text.push_str(&format!(
            "\n{} additional file(s) not validated within this call's budget.",
            total - attempted
        ));
    }
    execution.view.output_text.push_str(&text);
    execution.view.metadata.insert(
        "post_edit_validation".into(),
        serde_json::to_string(&results).expect("JSON result"),
    );
}
