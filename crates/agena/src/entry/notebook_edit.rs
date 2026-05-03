use std::fs;

use serde_json::{Map, Value};

use crate::message::{
    BuiltinToolOutput, NotebookCellType, NotebookEditMode, NotebookEditToolInput,
};

use super::{BuiltinExecution, ToolError, ToolExecutionView, ToolExecutor};

pub(super) fn execute(
    executor: &ToolExecutor,
    input: &NotebookEditToolInput,
) -> Result<BuiltinExecution, ToolError> {
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
    fs::write(&target, format!("{serialized}\n"))?;

    let display_path = executor.display_path(&target);
    let action = input.edit_mode.as_str();
    let output = BuiltinToolOutput::NotebookEdit {
        path: display_path.clone(),
        edit_mode: action.to_string(),
        cell_index: index as u32,
        cell_count: cell_count as u32,
    };
    let mut view = ToolExecutionView::simple(
        format!("Notebook edit {display_path}"),
        format!("{action} cell {index} in {display_path}; notebook now has {cell_count} cells"),
    );
    view.metadata.insert("path".to_string(), display_path);
    view.metadata
        .insert("edit_mode".to_string(), action.to_string());
    view.metadata
        .insert("cell_index".to_string(), index.to_string());
    view.metadata
        .insert("cell_count".to_string(), cell_count.to_string());

    Ok(BuiltinExecution::new(output, view))
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

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use serde_json::json;
    use uuid::Uuid;

    use super::*;

    #[derive(Debug)]
    struct TempWorkspace {
        root: PathBuf,
    }

    impl TempWorkspace {
        fn new() -> Self {
            let root =
                std::env::temp_dir().join(format!("agena-notebook-tests-{}", Uuid::new_v4()));
            fs::create_dir_all(&root).expect("temp workspace");
            Self { root }
        }
    }

    impl Drop for TempWorkspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn executor(root: &Path) -> ToolExecutor {
        let agent =
            crate::agent::Agent::new("test", crate::permission::PermissionPolicy::allow_all());
        ToolExecutor::new(root, agent)
    }

    fn write_notebook(path: &Path) {
        let notebook = json!({
            "cells": [
                {"cell_type": "markdown", "metadata": {}, "source": ["# Title\n"]},
                {"cell_type": "code", "execution_count": null, "metadata": {}, "outputs": [], "source": ["print(1)\n"]}
            ],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        });
        fs::write(path, serde_json::to_string_pretty(&notebook).unwrap()).expect("write notebook");
    }

    fn cells(path: &Path) -> Vec<Value> {
        let value: Value = serde_json::from_str(&fs::read_to_string(path).unwrap()).unwrap();
        value["cells"].as_array().unwrap().clone()
    }

    #[test]
    fn replace_cell_source() {
        let workspace = TempWorkspace::new();
        let path = workspace.root.join("demo.ipynb");
        write_notebook(&path);

        let result = execute(
            &executor(&workspace.root),
            &NotebookEditToolInput {
                notebook_path: "demo.ipynb".to_string(),
                cell_number: Some(1),
                new_source: "print(2)\n".to_string(),
                edit_mode: NotebookEditMode::Replace,
                cell_type: None,
            },
        )
        .expect("replace should succeed");

        assert!(result.view.output_text.contains("replace cell 1"));
        assert_eq!(cells(&path)[1]["source"], json!(["print(2)\n"]));
    }

    #[test]
    fn insert_cell_after_index() {
        let workspace = TempWorkspace::new();
        let path = workspace.root.join("demo.ipynb");
        write_notebook(&path);

        execute(
            &executor(&workspace.root),
            &NotebookEditToolInput {
                notebook_path: "demo.ipynb".to_string(),
                cell_number: Some(0),
                new_source: "inserted\n".to_string(),
                edit_mode: NotebookEditMode::Insert,
                cell_type: Some(NotebookCellType::Markdown),
            },
        )
        .expect("insert should succeed");

        let cells = cells(&path);
        assert_eq!(cells.len(), 3);
        assert_eq!(cells[1]["cell_type"], "markdown");
        assert_eq!(cells[1]["source"], json!(["inserted\n"]));
    }

    #[test]
    fn delete_cell() {
        let workspace = TempWorkspace::new();
        let path = workspace.root.join("demo.ipynb");
        write_notebook(&path);

        execute(
            &executor(&workspace.root),
            &NotebookEditToolInput {
                notebook_path: "demo.ipynb".to_string(),
                cell_number: Some(0),
                new_source: String::new(),
                edit_mode: NotebookEditMode::Delete,
                cell_type: None,
            },
        )
        .expect("delete should succeed");

        let cells = cells(&path);
        assert_eq!(cells.len(), 1);
        assert_eq!(cells[0]["cell_type"], "code");
    }
}
