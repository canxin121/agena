use crate::message::{BuiltinToolInput, BuiltinToolOutput};

use super::{
    BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor, apply_patch, bash, edit, glob,
    grep, read, task, write,
};

pub(super) fn execute_builtin(
    executor: &ToolExecutor,
    input: &BuiltinToolInput,
) -> Result<BuiltinExecution, ToolError> {
    match input {
        BuiltinToolInput::ApplyPatch(payload) => {
            let result = apply_patch::execute(executor, payload)?;
            let output = BuiltinToolOutput::ApplyPatch {
                operation_id: result.operation_id.clone(),
                files: result
                    .files
                    .iter()
                    .map(|f| f.path.clone())
                    .collect::<Vec<_>>(),
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

            Ok(BuiltinExecution::new(output, view).with_apply_patch(result))
        }
        BuiltinToolInput::Read(payload) => read::execute(executor, payload),
        BuiltinToolInput::Write(payload) => write::execute(executor, payload),
        BuiltinToolInput::Edit(payload) => edit::execute(executor, payload),
        BuiltinToolInput::Glob(payload) => glob::execute(executor, payload),
        BuiltinToolInput::Grep(payload) => grep::execute(executor, payload),
        BuiltinToolInput::Task(payload) => task::execute(executor, payload),
        BuiltinToolInput::Bash(payload) => bash::execute(executor, payload),
    }
}
