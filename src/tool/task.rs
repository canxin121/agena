use uuid::Uuid;

use crate::message::{BuiltinToolOutput, TaskToolInput};

use super::{BuiltinExecution, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &TaskToolInput,
) -> Result<BuiltinExecution, super::ToolError> {
    let session_id = input
        .task_id
        .clone()
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let output = BuiltinToolOutput::Task {
        session_id: Some(session_id.clone()),
        model_provider_id: None,
        model_id: None,
    };

    let mut view = ToolExecutionView::simple(
        format!("Task {}", input.description),
        format!(
            "Created subagent task session {} for type '{}' in workspace {}.",
            session_id,
            input.subagent_type,
            executor.display_path(executor.workspace_root())
        ),
    );
    view.metadata
        .insert("description".to_string(), input.description.clone());
    view.metadata
        .insert("subagent_type".to_string(), input.subagent_type.clone());
    view.metadata.insert("session_id".to_string(), session_id);
    if let Some(command) = &input.command {
        view.metadata.insert("command".to_string(), command.clone());
    }

    Ok(BuiltinExecution::new(output, view))
}
