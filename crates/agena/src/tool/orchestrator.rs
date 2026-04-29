use crate::message::{BuiltinToolInput, BuiltinToolOutput, FileChangeEntry, FileChangeKind};

use super::{
    BuiltinExecution, BuiltinExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
    apply_patch, ask_user, bash, glob, grep, monitor_tool, plan, read, task, todo_write,
    tool_search, view_file, web_fetch, web_search,
};

pub(super) fn execute_builtin(
    executor: &ToolExecutor,
    input: &BuiltinToolInput,
    context: BuiltinExecutionContext,
) -> Result<BuiltinExecution, ToolError> {
    match input {
        BuiltinToolInput::ApplyPatch(payload) => {
            let result = apply_patch::execute(executor, payload)?;
            let output = BuiltinToolOutput::ApplyPatch {
                operation_id: result.operation_id.clone(),
                changes: result
                    .files
                    .iter()
                    .map(|f| FileChangeEntry {
                        path: f.path.clone(),
                        kind: match f.kind {
                            super::apply_patch::PatchOpKind::Add => FileChangeKind::Added,
                            super::apply_patch::PatchOpKind::Update => FileChangeKind::Updated,
                            super::apply_patch::PatchOpKind::Delete => FileChangeKind::Deleted,
                        },
                    })
                    .collect(),
                before_hash: Some(result.before_hash.clone()),
                after_hash: Some(result.after_hash.clone()),
                inverse_patch: result.inverse_patch.clone(),
            };

            let mut view = ToolExecutionView::simple(
                format!("Apply patch ({})", result.operation_id),
                format!(
                    "Applied {} file changes in patch operation {}.",
                    result.files.len(),
                    result.operation_id
                ),
            );
            view.metadata
                .insert("operation_id".to_string(), result.operation_id.clone());
            view.metadata
                .insert("changed_files".to_string(), result.files.len().to_string());

            Ok(BuiltinExecution::new(output, view).with_apply_patch(result.clone()))
        }
        BuiltinToolInput::Read(payload) => read::execute(executor, payload),
        BuiltinToolInput::ViewFile(payload) => view_file::execute(executor, payload),
        BuiltinToolInput::Glob(payload) => glob::execute(executor, payload),
        BuiltinToolInput::Grep(payload) => grep::execute(executor, payload),
        BuiltinToolInput::Task(payload) => task::execute(executor, payload),
        BuiltinToolInput::ToolSearch(payload) => tool_search::execute(executor, payload),
        BuiltinToolInput::TodoWrite(payload) => Ok(todo_write::execute(payload)),
        BuiltinToolInput::AskUser(payload) => ask_user::execute(payload),
        BuiltinToolInput::Bash(payload) => bash::execute(executor, payload, context),
        BuiltinToolInput::Monitor(payload) => monitor_tool::execute(executor, payload),
        BuiltinToolInput::WebFetch(payload) => web_fetch::execute(executor, payload),
        BuiltinToolInput::WebSearch(payload) => web_search::execute(executor, payload),
        BuiltinToolInput::EnterPlanMode(payload) => {
            plan::execute_enter(executor, payload, context.session_id)
        }
        BuiltinToolInput::ExitPlanMode(payload) => {
            plan::execute_exit(executor, payload, context.session_id)
        }
    }
}
