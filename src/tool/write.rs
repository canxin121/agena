use std::fs;

use crate::message::{BuiltinToolOutput, WriteToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &WriteToolInput,
) -> Result<BuiltinExecution, ToolError> {
    let target = executor.resolve_target_path(&input.file_path);
    executor.ensure_edit_permission(&target)?;

    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&target, &input.content)?;

    let display_path = executor.display_path(&target);
    let output = BuiltinToolOutput::Write {
        files: vec![display_path.clone()],
    };
    let mut view = ToolExecutionView::simple(
        format!("Write {}", display_path),
        format!(
            "Wrote {} bytes to {}.",
            input.content.as_bytes().len(),
            display_path
        ),
    );
    view.metadata.insert(
        "bytes".to_string(),
        input.content.as_bytes().len().to_string(),
    );

    Ok(BuiltinExecution::new(output, view))
}
