use agena_domain::{FileChangeKind, FileChangeRecord};
use serde_json::Value as JsonValue;

use crate::part::ApplyPatchToolInput;
use agena_plugin_host::sdk::ToolInput;

use super::{
    ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput,
    ToolRuntimeContext, apply_patch, ask_user, glob, grep, process_tool, read, snapshot,
    suggest_tool_names, task, tool_search, unknown_tool_hint,
};

const BUILTIN_TOOL_NAMES: &[&str] = &[
    "apply_patch",
    "ask_user",
    "cron_create",
    "cron_delete",
    "cron_history",
    "cron_list",
    "cron_pause",
    "cron_resume",
    "cron_update",
    "exit_snapshot",
    "glob",
    "grep",
    "lsp_definition",
    "lsp_diagnostics",
    "lsp_hover",
    "lsp_references",
    "shell",
    "read",
    "task",
    "tool_search",
    "enter_snapshot",
];

fn apply_patch_output_text(result: &agena_tool::ApplyPatchExecution) -> String {
    if result.files.is_empty() {
        return format!("Applied patch operation {}.", result.operation_id);
    }

    let preview = summarize_file_paths(result.files.iter().map(|file| file.path.as_str()), 3);
    format!(
        "Applied patch to {} file{}: {}",
        result.files.len(),
        if result.files.len() == 1 { "" } else { "s" },
        preview
    )
}

fn summarize_file_paths<'a, I>(paths: I, preview_limit: usize) -> String
where
    I: Iterator<Item = &'a str>,
{
    let mut preview = Vec::new();
    let mut omitted = 0_usize;

    for path in paths {
        if preview.len() < preview_limit {
            preview.push(path.to_string());
        } else {
            omitted += 1;
        }
    }

    if omitted > 0 {
        preview.push(format!("+{omitted} more"));
    }

    preview.join(", ")
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
                            agena_tool::PatchOpKind::Add => FileChangeKind::Added,
                            agena_tool::PatchOpKind::Update => FileChangeKind::Updated,
                            agena_tool::PatchOpKind::Delete => FileChangeKind::Deleted,
                            agena_tool::PatchOpKind::Move => FileChangeKind::Moved,
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
            let changed_files = result.files.len();
            let (additions, deletions) = patch_line_delta(result.diff.as_str());
            let mut summary_parts = vec![format!("{changed_files} files changed")];
            if additions > 0 {
                summary_parts.push(format!("+{additions}"));
            }
            if deletions > 0 {
                summary_parts.push(format!("−{deletions}"));
            }
            let mut view =
                ToolExecutionView::simple("Apply patch", summary_parts.join(" · "), output_text);
            view.metadata
                .insert("operation_id".to_string(), result.operation_id.clone());
            view.metadata
                .insert("changed_files".to_string(), changed_files.to_string());

            Ok(ToolPayloadExecution {
                output,
                view,
                apply_patch: Some(result.clone()),
            })
        }
        "read" => read::execute(executor, &parse_shape_input(input)?),
        "glob" => glob::execute(executor, &parse_shape_input(input)?),
        "grep" => grep::execute(executor, &parse_shape_input(input)?),
        "task" => task::execute(executor, &parse_shape_input(input)?),
        "tool_search" => tool_search::execute(executor, &parse_shape_input(input)?),
        "ask_user" => ask_user::execute(&parse_shape_input(input)?),
        "shell" => process_tool::execute(executor, &parse_shape_input(input)?, context),
        "enter_snapshot" => {
            snapshot::execute_enter(executor, &parse_shape_input(input)?, context.session_id)
        }
        "exit_snapshot" => {
            snapshot::execute_exit(executor, &parse_shape_input(input)?, context.session_id)
        }
        other => {
            let suggestions = suggest_tool_names(other, BUILTIN_TOOL_NAMES, 1);
            Err(unknown_tool_hint(other, suggestions))
        }
    }
}

fn patch_line_delta(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(additions, deletions), line| {
        if line.starts_with('+') && !line.starts_with("+++") {
            (additions + 1, deletions)
        } else if line.starts_with('-') && !line.starts_with("---") {
            (additions, deletions + 1)
        } else {
            (additions, deletions)
        }
    })
}

fn parse_shape_input<T: ToolInput>(input: JsonValue) -> Result<T, ToolError> {
    T::parse_input(input).map_err(|err| ToolError::invalid_input_error(&err))
}
