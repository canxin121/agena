use std::fs;

use crate::message::{BuiltinToolOutput, EditToolInput};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &EditToolInput,
) -> Result<BuiltinExecution, ToolError> {
    if input.old_string.is_empty() {
        return Err(ToolError::InvalidInput(
            "edit old_string must not be empty".to_string(),
        ));
    }

    let target = executor.resolve_target_path(&input.file_path);
    executor.ensure_edit_permission(&target)?;

    let original = fs::read_to_string(&target)?;
    let matches = original.match_indices(&input.old_string).count();
    if matches == 0 {
        return Err(ToolError::InvalidInput(format!(
            "edit target text not found in {}",
            input.file_path
        )));
    }
    if !input.replace_all && matches > 1 {
        return Err(ToolError::InvalidInput(format!(
            "edit found {} matches; set replace_all=true or make old_string unique",
            matches
        )));
    }

    let updated = if input.replace_all {
        original.replace(&input.old_string, &input.new_string)
    } else {
        original.replacen(&input.old_string, &input.new_string, 1)
    };
    fs::write(&target, &updated)?;

    let replaced_count = if input.replace_all { matches } else { 1 };
    let display_path = executor.display_path(&target);
    let diagnostic = format!(
        "Replaced {} occurrence(s) in {}.",
        replaced_count, display_path
    );

    let output = BuiltinToolOutput::Edit {
        diagnostics: vec![diagnostic.clone()],
        diff: None,
        file_diff: None,
        files: vec![display_path.clone()],
    };
    let mut view = ToolExecutionView::simple(format!("Edit {}", display_path), diagnostic);
    view.metadata
        .insert("replacements".to_string(), replaced_count.to_string());

    Ok(BuiltinExecution::new(output, view))
}
