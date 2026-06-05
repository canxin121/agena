use serde_json::Value as JsonValue;

use crate::message::ApplyPatchToolInput;
use crate::message::{FileChangeKind, FileChangeRecord};
use crate::plugin::sdk::ToolInputShape;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    ToolRuntimeContext, apply_patch, ask_user, bash, cron, glob, grep, lsp, monitor_tool,
    notebook_edit, powershell, read, suggest_tool_names, task, todo_write, tool_search,
    unknown_tool_hint, worktree,
};

const BUILTIN_TOOL_NAMES: &[&str] = &[
    "apply_patch",
    "ask_user",
    "bash",
    "cron_create",
    "cron_delete",
    "cron_list",
    "exit_worktree",
    "glob",
    "grep",
    "lsp_definition",
    "lsp_diagnostics",
    "lsp_hover",
    "lsp_references",
    "monitor",
    "notebook_edit",
    "powershell",
    "read",
    "schedule_wakeup",
    "task",
    "todo_write",
    "tool_search",
    "enter_worktree",
];

fn apply_patch_output_text(result: &apply_patch::ApplyPatchExecution) -> String {
    let mut lines = vec![format!(
        "Applied {} file changes in patch operation {}.",
        result.files.len(),
        result.operation_id
    )];
    lines.extend(result.progress.iter().map(|line| format!("- {line}")));
    lines.join("\n")
}

pub(crate) fn execute_tool(
    executor: &ToolExecutor,
    tool_name: &str,
    input: JsonValue,
    context: ToolRuntimeContext,
) -> Result<ToolPayloadExecution, ToolError> {
    match tool_name {
        "apply_patch" => {
            let payload = parse_shape_input::<ApplyPatchToolInput>(input)?;
            let result = apply_patch::execute(executor, &payload)?;
            let output = ToolPayloadOutput::ApplyPatch {
                operation_id: result.operation_id.clone(),
                changes: result
                    .files
                    .iter()
                    .map(|f| FileChangeRecord {
                        path: f.path.clone(),
                        kind: match f.kind {
                            super::apply_patch::PatchOpKind::Add => FileChangeKind::Added,
                            super::apply_patch::PatchOpKind::Update => FileChangeKind::Updated,
                            super::apply_patch::PatchOpKind::Delete => FileChangeKind::Deleted,
                            super::apply_patch::PatchOpKind::Move => FileChangeKind::Moved,
                        },
                        from_path: f.from_path.clone(),
                    })
                    .collect(),
                before_hash: Some(result.before_hash.clone()),
                after_hash: Some(result.after_hash.clone()),
                inverse_patch: result.inverse_patch.clone(),
                diff: result.diff.clone(),
                progress: result.progress.clone(),
            };

            let output_text = apply_patch_output_text(&result);
            let mut view = ToolExecutionView::simple(
                format!("Apply patch ({})", result.operation_id),
                output_text,
            );
            view.metadata
                .insert("operation_id".to_string(), result.operation_id.clone());
            view.metadata
                .insert("changed_files".to_string(), result.files.len().to_string());

            Ok(ToolPayloadExecution::new(output, view).with_apply_patch(result.clone()))
        }
        "read" => read::execute(executor, &parse_shape_input(input)?),
        "glob" => glob::execute(executor, &parse_shape_input(input)?),
        "grep" => grep::execute(executor, &parse_shape_input(input)?),
        "task" => task::execute(executor, &parse_shape_input(input)?),
        "tool_search" => tool_search::execute(executor, &parse_shape_input(input)?),
        "todo_write" => Ok(todo_write::execute(&parse_shape_input(input)?)),
        "ask_user" => ask_user::execute(&parse_shape_input(input)?),
        "bash" => bash::execute(executor, &parse_shape_input(input)?, context),
        "monitor" => monitor_tool::execute(executor, &parse_shape_input(input)?),
        "enter_worktree" => {
            worktree::execute_enter(executor, &parse_shape_input(input)?, context.session_id)
        }
        "exit_worktree" => {
            worktree::execute_exit(executor, &parse_shape_input(input)?, context.session_id)
        }
        "cron_create" => {
            cron::execute_create(executor, &parse_shape_input(input)?, context.session_id)
        }
        "cron_list" => cron::execute_list(executor, &parse_shape_input(input)?),
        "cron_delete" => cron::execute_delete(executor, &parse_shape_input(input)?),
        "schedule_wakeup" => {
            cron::execute_wakeup(executor, &parse_shape_input(input)?, context.session_id)
        }
        "lsp_definition" => lsp::execute_definition(executor, &parse_shape_input(input)?),
        "lsp_references" => lsp::execute_references(executor, &parse_shape_input(input)?),
        "lsp_hover" => lsp::execute_hover(executor, &parse_shape_input(input)?),
        "lsp_diagnostics" => lsp::execute_diagnostics(executor, &parse_shape_input(input)?),
        "notebook_edit" => notebook_edit::execute(executor, &parse_shape_input(input)?),
        "powershell" => powershell::execute(executor, &parse_shape_input(input)?, context),
        other => {
            let suggestions = suggest_tool_names(other, BUILTIN_TOOL_NAMES, 1);
            if suggestions.is_empty() {
                Err(ToolError::UnknownTool {
                    tool: other.to_string(),
                })
            } else {
                Err(unknown_tool_hint(other, suggestions))
            }
        }
    }
}

fn parse_shape_input<T: ToolInputShape>(input: JsonValue) -> Result<T, ToolError> {
    T::parse_input(input).map_err(|err| ToolError::InvalidInput(err.to_string()))
}
