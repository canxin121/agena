use crate::message::{FileChangeEntry, FileChangeKind, BundledToolInput, BundledToolOutput};

use super::{
    BundledExecution, BundledExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
    apply_patch, ask_user, bash, cron, glob, grep, lsp, monitor_tool, notebook_edit, plan,
    powershell, read, task, todo_write, tool_search, view_file, web_fetch, web_search, worktree,
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

pub(crate) fn execute_bundled(
    executor: &ToolExecutor,
    input: &BundledToolInput,
    context: BundledExecutionContext,
) -> Result<BundledExecution, ToolError> {
    match input {
        BundledToolInput::ApplyPatch(payload) => {
            let result = apply_patch::execute(executor, payload)?;
            let output = BundledToolOutput::ApplyPatch {
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

            Ok(BundledExecution::new(output, view).with_apply_patch(result.clone()))
        }
        BundledToolInput::Read(payload) => read::execute(executor, payload),
        BundledToolInput::ViewFile(payload) => view_file::execute(executor, payload),
        BundledToolInput::Glob(payload) => glob::execute(executor, payload),
        BundledToolInput::Grep(payload) => grep::execute(executor, payload),
        BundledToolInput::Task(payload) => task::execute(executor, payload),
        BundledToolInput::ToolSearch(payload) => tool_search::execute(executor, payload),
        BundledToolInput::TodoWrite(payload) => Ok(todo_write::execute(payload)),
        BundledToolInput::AskUser(payload) => ask_user::execute(payload),
        BundledToolInput::Bash(payload) => bash::execute(executor, payload, context),
        BundledToolInput::Monitor(payload) => monitor_tool::execute(executor, payload),
        BundledToolInput::WebFetch(payload) => web_fetch::execute(executor, payload),
        BundledToolInput::WebSearch(payload) => web_search::execute(executor, payload),
        BundledToolInput::EnterPlanMode(payload) => {
            plan::execute_enter(executor, payload, context.session_id)
        }
        BundledToolInput::ExitPlanMode(payload) => {
            plan::execute_exit(executor, payload, context.session_id)
        }
        BundledToolInput::EnterWorktree(payload) => {
            worktree::execute_enter(executor, payload, context.session_id)
        }
        BundledToolInput::ExitWorktree(payload) => {
            worktree::execute_exit(executor, payload, context.session_id)
        }
        BundledToolInput::CronCreate(payload) => {
            cron::execute_create(executor, payload, context.session_id)
        }
        BundledToolInput::CronList(payload) => cron::execute_list(executor, payload),
        BundledToolInput::CronDelete(payload) => cron::execute_delete(executor, payload),
        BundledToolInput::ScheduleWakeup(payload) => {
            cron::execute_wakeup(executor, payload, context.session_id)
        }
        BundledToolInput::LspDefinition(payload) => lsp::execute_definition(executor, payload),
        BundledToolInput::LspReferences(payload) => lsp::execute_references(executor, payload),
        BundledToolInput::LspHover(payload) => lsp::execute_hover(executor, payload),
        BundledToolInput::LspDiagnostics(payload) => lsp::execute_diagnostics(executor, payload),
        BundledToolInput::NotebookEdit(payload) => notebook_edit::execute(executor, payload),
        BundledToolInput::PowerShell(payload) => powershell::execute(executor, payload, context),
    }
}
