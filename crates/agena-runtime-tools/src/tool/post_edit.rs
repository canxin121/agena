//! Bounded diagnostic evidence attached to a committed edit.

use super::*;
use agena_domain::PermissionDecision;
use agena_lsp::LspClientMode;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

const MAX_FILES: usize = 8;
const MAX_VALIDATION_BYTES: usize = 8 * 1024;
pub const POST_EDIT_VALIDATION: &str = "post_edit_validation";

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
    let Some(registry) = executor.lsp_registry() else {
        return;
    };
    if !registry.has_running_session(session).await {
        return;
    }
    let input = serde_json::Value::from(invocation.input.clone());
    let folder = input.get("folder").and_then(serde_json::Value::as_str);
    let targets = edited_targets(executor, &input, execution);
    let revision = match name {
        "fs.write" => execution.output.payload.get("sha256"),
        "fs.replace" => execution.output.payload.get("after_sha256"),
        _ => None,
    }
    .and_then(|hash| hash.as_text());
    let mut routed = Vec::new();
    let mut omitted = 0;
    for target in targets {
        if registry.server_for_path(&target).await.is_some() {
            if routed.len() == MAX_FILES {
                omitted += 1;
            } else {
                routed.push(target);
            }
        }
    }

    if routed.is_empty() {
        return;
    }

    let mut passive = executor.clone();
    passive.path_effect_authorizer = None;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut results = Vec::new();
    let mut remaining = MAX_VALIDATION_BYTES - 128;
    for target in routed {
        let path = target.to_string_lossy().into_owned();
        let label = executor.display_path(&target);
        let label = super::lsp::result_preview(&label, 512);
        let call = ToolInvocation::new(
            "lsp.diagnostics",
            StructuredObject::try_from(json!({"file_path": path})).expect("object input"),
        );
        let operation = async {
            let Some(connection) = super::lsp::connection_for_target(
                &passive,
                registry,
                &target,
                folder,
                None,
                LspClientMode::RunningOnly(session),
            )
            .await?
            else {
                return Ok(None);
            };
            let checks = passive
                .collect_permission_checks_for_invocation_in_session(&call, Some(session))
                .await?;
            if checks
                .iter()
                .any(|check| !matches!(check.decision, PermissionDecision::Allow))
            {
                return Ok(Some(json!({"file": label, "state": "permission_required"})));
            }
            let report = super::lsp::diagnostics_for(
                &passive,
                &crate::part::LspDiagnosticsToolInput {
                    file_path: path,
                    folder: folder.map(str::to_owned),
                    server: None,
                    expected_revision: revision.map(str::to_owned),
                },
                768,
                Some(connection),
            )
            .await?;
            let entries = report.diagnostics.len();
            Ok::<_, ToolError>(Some(json!({
                "file": label,
                "state": report.state,
                "source_revision": report.source.source_revision,
                "document_version": report.source.document_version,
                "reported_version": report.reported_version,
                "total_diagnostics": report.total_diagnostics,
                "truncated": report.truncated || report.total_diagnostics > entries,
                "diagnostics": report.diagnostics,
            })))
        };
        let result = match tokio::time::timeout_at(deadline, operation).await {
            Ok(Ok(Some(result))) => result,
            Ok(Ok(None)) => continue,
            Ok(Err(ToolError::Cancelled)) => {
                json!({"file": label, "state": "cancelled_after_edit"})
            }
            Ok(Err(ToolError::PolicyDenied(_))) => {
                json!({"file": label, "state": "permission_required"})
            }
            Ok(Err(error)) => json!({
                "file": label,
                "state": "unavailable_after_edit",
                "error": super::lsp::result_preview(&error.to_string(), 256)
            }),
            Err(_) => json!({"file": label, "state": "timeout_after_edit"}),
        };
        let bytes = serde_json::to_vec(&result).expect("JSON result").len() + 1;
        if let Some(next) = remaining.checked_sub(bytes) {
            remaining = next;
            results.push(result);
        } else {
            omitted += 1;
        }
    }
    if results.is_empty() {
        return;
    }
    let report = json!({"files": results, "omitted_files": omitted});
    execution
        .view
        .metadata
        .insert(POST_EDIT_VALIDATION.into(), report.to_string());
}

fn edited_targets(
    executor: &ToolExecutor,
    input: &serde_json::Value,
    execution: &ToolInvocationExecution,
) -> Vec<PathBuf> {
    let mut paths = execution
        .apply_patch
        .as_ref()
        .map(|patch| {
            patch
                .files
                .iter()
                .filter(|file| file.kind != agena_tool::PatchOpKind::Delete)
                .map(|file| file.path.as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    if paths.is_empty()
        && let Some(path) = input.get("path").and_then(serde_json::Value::as_str)
    {
        paths.push(path);
    }
    let folder = input.get("folder").and_then(serde_json::Value::as_str);
    let mut seen = std::collections::HashSet::new();
    paths
        .into_iter()
        .filter_map(|path| super::lsp::lsp_target(executor, path, folder).ok())
        .filter(|target| seen.insert(target.clone()))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorization::ExecutionPrincipal;
    use crate::permission::{PermissionPolicy, ToolPermissionPolicy};

    #[test]
    fn bot_edit_diagnostics_preserve_the_explicit_folder() {
        let directory = tempfile::tempdir().unwrap();
        let folder = directory.path().canonicalize().unwrap();
        let mut executor = ToolExecutor::new(
            folder.join("unrelated"),
            ExecutionPrincipal::new(
                PermissionPolicy::allow_all(),
                ToolPermissionPolicy::allow_all(),
            ),
            agena_plugin_host::PluginHost::new_empty(),
            None,
            None,
        );
        executor.global = true;
        let execution = ToolInvocationExecution::new(
            ToolOutput::default(),
            ToolExecutionView::simple("Write file", "Saved", ""),
        );
        let input = json!({"path": "src/lib.rs", "folder": folder});
        assert_eq!(
            edited_targets(&executor, &input, &execution),
            vec![folder.join("src/lib.rs")]
        );
    }

    #[test]
    fn post_edit_receipt_projects_validation_as_structured_data() {
        let mut view = ToolExecutionView::simple("Write file", "Saved", "");
        let output = ToolOutput::from_json_payload(Some(&json!({"path": "src/lib.rs"}))).unwrap();
        let validation =
            json!({"files": [{"file": "src/lib.rs", "state": "current"}], "omitted_files": 0});
        view.metadata
            .insert(POST_EDIT_VALIDATION.into(), validation.to_string());
        let raw = view.raw_output(&output).payload.unwrap();
        assert_eq!(raw["path"], "src/lib.rs");
        assert_eq!(raw[POST_EDIT_VALIDATION], validation);
    }
}
