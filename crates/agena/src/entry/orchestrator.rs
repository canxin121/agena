use crate::message::{BuiltinToolInput, BuiltinToolOutput, FileChangeEntry, FileChangeKind};

use super::{
    BuiltinExecution, BuiltinExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
    apply_patch, ask_user, bash, cron, glob, grep, lsp, monitor_tool, notebook_edit, plan,
    powershell, read, skill, task, todo_write, tool_search, view_file, web_fetch, web_search,
    worktree,
};

fn apply_patch_output_text(result: &apply_patch::ApplyPatchExecution) -> String {
    let mut lines = vec![format!(
        "Applied {} file changes in patch operation {}.",
        result.files.len(),
        result.operation_id
    )];
    lines.extend(result.progress.iter().map(|line| format!("- {line}")));
    lines.join("\n")
}

pub(crate) fn execute_builtin(
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
        BuiltinToolInput::SkillRun(payload) => skill::execute(executor, payload),
        BuiltinToolInput::EnterWorktree(payload) => {
            worktree::execute_enter(executor, payload, context.session_id)
        }
        BuiltinToolInput::ExitWorktree(payload) => {
            worktree::execute_exit(executor, payload, context.session_id)
        }
        BuiltinToolInput::CronCreate(payload) => cron::execute_create(executor, payload),
        BuiltinToolInput::CronList(payload) => cron::execute_list(executor, payload),
        BuiltinToolInput::CronDelete(payload) => cron::execute_delete(executor, payload),
        BuiltinToolInput::ScheduleWakeup(payload) => cron::execute_wakeup(executor, payload),
        BuiltinToolInput::LspDefinition(payload) => lsp::execute_definition(executor, payload),
        BuiltinToolInput::LspReferences(payload) => lsp::execute_references(executor, payload),
        BuiltinToolInput::LspHover(payload) => lsp::execute_hover(executor, payload),
        BuiltinToolInput::LspDiagnostics(payload) => lsp::execute_diagnostics(executor, payload),
        BuiltinToolInput::NotebookEdit(payload) => notebook_edit::execute(executor, payload),
        BuiltinToolInput::PowerShell(payload) => powershell::execute(executor, payload, context),
    }
}
