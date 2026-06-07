use std::fs;

use diff::Result as DiffResult;
use serde_json::{Map, Value};

use crate::message::{
    FileChangeKind, FileChangeRecord, NotebookCellType, NotebookEditMode, NotebookEditToolInput,
};

use super::{ToolError, ToolExecutionView, ToolExecutor, ToolPayloadExecution, ToolPayloadOutput};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &NotebookEditToolInput,
) -> Result<ToolPayloadExecution, ToolError> {
    let target = executor.resolve_target_path(&input.notebook_path);
    executor.ensure_edit_permission(&target)?;

    if target.extension().and_then(|ext| ext.to_str()) != Some("ipynb") {
        return Err(ToolError::InvalidInput(format!(
            "notebook_edit only supports .ipynb files: {}",
            input.notebook_path
        )));
    }

    let raw = fs::read_to_string(&target)?;
    let mut notebook = serde_json::from_str::<Value>(&raw)
        .map_err(|err| ToolError::InvalidInput(format!("invalid notebook json: {err}")))?;
    let cells = notebook
        .get_mut("cells")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| {
            ToolError::InvalidInput("notebook json is missing cells array".to_string())
        })?;

    let index = apply_edit(cells, input)?;
    let cell_count = cells.len();
    let serialized = serde_json::to_string_pretty(&notebook)
        .map_err(|err| ToolError::InvalidInput(format!("serialize notebook: {err}")))?;
    let updated = format!("{serialized}\n");
    fs::write(&target, &updated)?;

    let display_path = executor.display_path(&target);
    let action = input.edit_mode.as_str();
    let changes = vec![FileChangeRecord {
        path: display_path.clone(),
        kind: FileChangeKind::Updated,
        from_path: None,
    }];
    let diff = render_notebook_diff(display_path.as_str(), raw.as_str(), updated.as_str());
    let output = ToolPayloadOutput::NotebookEdit {
        path: display_path.clone(),
        edit_mode: action.to_string(),
        cell_index: index as u32,
        cell_count: cell_count as u32,
        changes,
        diff,
    };
    let mut view = ToolExecutionView::simple(
        format!("Notebook edit {display_path}"),
        format!(
            "Updated notebook {display_path}: {action} cell {index}; notebook now has {cell_count} cells"
        ),
    );
    view.metadata.insert("path".to_string(), display_path);
    view.metadata
        .insert("edit_mode".to_string(), action.to_string());
    view.metadata
        .insert("cell_index".to_string(), index.to_string());
    view.metadata
        .insert("cell_count".to_string(), cell_count.to_string());

    Ok(ToolPayloadExecution::new(output, view))
}

fn render_notebook_diff(path: &str, before: &str, after: &str) -> String {
    let normalized_before = normalize_lf(before);
    let normalized_after = normalize_lf(after);
    if normalized_before == normalized_after {
        return String::new();
    }

    let mut lines = vec![format!("diff --git a/{path} b/{path}")];
    lines.push(format!("--- a/{path}"));
    lines.push(format!("+++ b/{path}"));

    for hunk in grouped_hunks(normalized_before.as_str(), normalized_after.as_str(), 3) {
        lines.push("@@".to_string());
        for diff_line in hunk {
            let prefix = match diff_line.kind {
                DiffLineKind::Context => ' ',
                DiffLineKind::Added => '+',
                DiffLineKind::Deleted => '-',
            };
            lines.push(format!("{prefix}{}", diff_line.text));
        }
    }

    lines.join("\n")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DiffLineKind {
    Context,
    Added,
    Deleted,
}

#[derive(Debug, Clone)]
struct DiffLine {
    kind: DiffLineKind,
    text: String,
}

fn grouped_hunks(before: &str, after: &str, context: usize) -> Vec<Vec<DiffLine>> {
    let lines = diff::lines(before, after)
        .into_iter()
        .map(|line| match line {
            DiffResult::Both(text, _) => DiffLine {
                kind: DiffLineKind::Context,
                text: text.to_string(),
            },
            DiffResult::Left(text) => DiffLine {
                kind: DiffLineKind::Deleted,
                text: text.to_string(),
            },
            DiffResult::Right(text) => DiffLine {
                kind: DiffLineKind::Added,
                text: text.to_string(),
            },
        })
        .collect::<Vec<_>>();
    let changed_indices = lines
        .iter()
        .enumerate()
        .filter_map(|(index, line)| (line.kind != DiffLineKind::Context).then_some(index))
        .collect::<Vec<_>>();
    if changed_indices.is_empty() {
        return Vec::new();
    }

    let mut ranges = Vec::<(usize, usize)>::new();
    for index in changed_indices {
        let start = index.saturating_sub(context);
        let end = (index + context).min(lines.len().saturating_sub(1));
        if let Some((_, previous_end)) = ranges.last_mut()
            && start <= previous_end.saturating_add(1)
        {
            *previous_end = (*previous_end).max(end);
        } else {
            ranges.push((start, end));
        }
    }

    ranges
        .into_iter()
        .map(|(start, end)| lines[start..=end].to_vec())
        .collect()
}

fn normalize_lf(input: &str) -> String {
    input.replace("\r\n", "\n")
}

fn apply_edit(cells: &mut Vec<Value>, input: &NotebookEditToolInput) -> Result<usize, ToolError> {
    match input.edit_mode {
        NotebookEditMode::Replace => {
            let index = cell_number(input, cells.len().saturating_sub(1))?;
            let Some(cell) = cells.get_mut(index) else {
                return Err(cell_index_error(index, cells.len()));
            };
            if let Some(cell_type) = input.cell_type {
                set_cell_type(cell, cell_type)?;
            }
            set_cell_source(cell, &input.new_source)?;
            Ok(index)
        }
        NotebookEditMode::Insert => {
            let cell_type = input.cell_type.ok_or_else(|| {
                ToolError::InvalidInput(
                    "cell_type is required when edit_mode is insert".to_string(),
                )
            })?;
            let index = if cells.is_empty() {
                0
            } else {
                cell_number(input, cells.len() - 1)? + 1
            };
            if index > cells.len() {
                return Err(cell_index_error(index, cells.len()));
            }
            cells.insert(index, new_cell(cell_type, &input.new_source));
            Ok(index)
        }
        NotebookEditMode::Delete => {
            let index = cell_number(input, cells.len().saturating_sub(1))?;
            if index >= cells.len() {
                return Err(cell_index_error(index, cells.len()));
            }
            cells.remove(index);
            Ok(index)
        }
    }
}

fn cell_number(input: &NotebookEditToolInput, default: usize) -> Result<usize, ToolError> {
    input
        .cell_number
        .map(|value| {
            usize::try_from(value)
                .map_err(|_| ToolError::InvalidInput("cell_number is too large".to_string()))
        })
        .transpose()
        .map(|value| value.unwrap_or(default))
}

fn cell_index_error(index: usize, len: usize) -> ToolError {
    ToolError::InvalidInput(format!(
        "cell index {index} is out of range for {len} cells"
    ))
}

fn set_cell_source(cell: &mut Value, source: &str) -> Result<(), ToolError> {
    let object = cell
        .as_object_mut()
        .ok_or_else(|| ToolError::InvalidInput("notebook cell must be an object".to_string()))?;
    object.insert("source".to_string(), source_to_lines(source));
    Ok(())
}

fn set_cell_type(cell: &mut Value, cell_type: NotebookCellType) -> Result<(), ToolError> {
    let object = cell
        .as_object_mut()
        .ok_or_else(|| ToolError::InvalidInput("notebook cell must be an object".to_string()))?;
    object.insert(
        "cell_type".to_string(),
        Value::String(cell_type.as_str().to_string()),
    );
    if cell_type == NotebookCellType::Code {
        object
            .entry("outputs")
            .or_insert_with(|| Value::Array(Vec::new()));
        object.entry("execution_count").or_insert(Value::Null);
    } else {
        object.remove("outputs");
        object.remove("execution_count");
    }
    Ok(())
}

fn new_cell(cell_type: NotebookCellType, source: &str) -> Value {
    let mut object = Map::new();
    object.insert(
        "cell_type".to_string(),
        Value::String(cell_type.as_str().to_string()),
    );
    object.insert("metadata".to_string(), Value::Object(Map::new()));
    object.insert("source".to_string(), source_to_lines(source));
    if cell_type == NotebookCellType::Code {
        object.insert("execution_count".to_string(), Value::Null);
        object.insert("outputs".to_string(), Value::Array(Vec::new()));
    }
    Value::Object(object)
}

fn source_to_lines(source: &str) -> Value {
    if source.is_empty() {
        return Value::Array(Vec::new());
    }
    let mut lines = source
        .split_inclusive('\n')
        .map(|line| Value::String(line.to_string()))
        .collect::<Vec<_>>();
    if !source.ends_with('\n')
        && let Some(Value::String(last)) = lines.last_mut()
    {
        *last = last.trim_end_matches('\n').to_string();
    }
    Value::Array(lines)
}
