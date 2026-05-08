use crate::message::{FirstPartyToolInput, FirstPartyToolOutput, FileChangeEntry, FileChangeKind};

use super::{
    FirstPartyExecution, FirstPartyExecutionContext, ToolError, ToolExecutionView, ToolExecutor,
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

pub(crate) fn execute_first_party(
    executor: &ToolExecutor,
    input: &FirstPartyToolInput,
    context: FirstPartyExecutionContext,
) -> Result<FirstPartyExecution, ToolError> {
    match input {
        FirstPartyToolInput::ApplyPatch(payload) => {
            let result = apply_patch::execute(executor, payload)?;
            let output = FirstPartyToolOutput::ApplyPatch {
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

            Ok(FirstPartyExecution::new(output, view).with_apply_patch(result.clone()))
        }
        FirstPartyToolInput::Read(payload) => read::execute(executor, payload),
        FirstPartyToolInput::ViewFile(payload) => view_file::execute(executor, payload),
        FirstPartyToolInput::Glob(payload) => glob::execute(executor, payload),
        FirstPartyToolInput::Grep(payload) => grep::execute(executor, payload),
        FirstPartyToolInput::Task(payload) => task::execute(executor, payload),
        FirstPartyToolInput::ToolSearch(payload) => tool_search::execute(executor, payload),
        FirstPartyToolInput::TodoWrite(payload) => Ok(todo_write::execute(payload)),
        FirstPartyToolInput::AskUser(payload) => ask_user::execute(payload),
        FirstPartyToolInput::Bash(payload) => bash::execute(executor, payload, context),
        FirstPartyToolInput::Monitor(payload) => monitor_tool::execute(executor, payload),
        FirstPartyToolInput::WebFetch(payload) => web_fetch::execute(executor, payload),
        FirstPartyToolInput::WebSearch(payload) => web_search::execute(executor, payload),
        FirstPartyToolInput::EnterPlanMode(payload) => {
            plan::execute_enter(executor, payload, context.session_id)
        }
        FirstPartyToolInput::ExitPlanMode(payload) => {
            plan::execute_exit(executor, payload, context.session_id)
        }
        FirstPartyToolInput::EnterWorktree(payload) => {
            worktree::execute_enter(executor, payload, context.session_id)
        }
        FirstPartyToolInput::ExitWorktree(payload) => {
            worktree::execute_exit(executor, payload, context.session_id)
        }
        FirstPartyToolInput::CronCreate(payload) => {
            cron::execute_create(executor, payload, context.session_id)
        }
        FirstPartyToolInput::CronList(payload) => cron::execute_list(executor, payload),
        FirstPartyToolInput::CronDelete(payload) => cron::execute_delete(executor, payload),
        FirstPartyToolInput::ScheduleWakeup(payload) => {
            cron::execute_wakeup(executor, payload, context.session_id)
        }
        FirstPartyToolInput::LspDefinition(payload) => lsp::execute_definition(executor, payload),
        FirstPartyToolInput::LspReferences(payload) => lsp::execute_references(executor, payload),
        FirstPartyToolInput::LspHover(payload) => lsp::execute_hover(executor, payload),
        FirstPartyToolInput::LspDiagnostics(payload) => lsp::execute_diagnostics(executor, payload),
        FirstPartyToolInput::NotebookEdit(payload) => notebook_edit::execute(executor, payload),
        FirstPartyToolInput::PowerShell(payload) => powershell::execute(executor, payload, context),
    }
}
