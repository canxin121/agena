use crate::message::{BuiltinToolOutput, TaskToolInput};
use crate::session::SubtaskSessionRequest;

use super::{BuiltinExecution, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &TaskToolInput,
) -> Result<BuiltinExecution, super::ToolError> {
    let session = executor
        .subtask_manager()
        .create_or_resume(SubtaskSessionRequest {
            requested_task_id: input.task_id.clone(),
            description: input.description.clone(),
            prompt: input.prompt.clone(),
            subagent_type: input.subagent_type.clone(),
            command: input.command.clone(),
        })
        .map_err(|err| super::ToolError::InvalidInput(err.to_string()))?;

    let output = BuiltinToolOutput::Task {
        session_id: Some(session.session_id.clone()),
        model_provider_id: session.model_provider_id,
        model_id: session.model_id,
    };

    let mut view = ToolExecutionView::simple(
        format!("Task {}", input.description),
        format!(
            "Created/resumed subagent task session {} for type '{}' in workspace {}.",
            session.session_id,
            input.subagent_type,
            executor.display_path(executor.workspace_root())
        ),
    );
    view.metadata
        .insert("description".to_string(), input.description.clone());
    view.metadata
        .insert("subagent_type".to_string(), input.subagent_type.clone());
    view.metadata
        .insert("session_id".to_string(), session.session_id);
    if let Some(command) = &input.command {
        view.metadata.insert("command".to_string(), command.clone());
    }

    Ok(BuiltinExecution::new(output, view))
}
