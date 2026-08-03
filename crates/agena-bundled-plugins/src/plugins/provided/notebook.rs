//! Revision-safe Jupyter notebook cell editing.

use std::io::Write;
use std::path::{Path, PathBuf};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{PathRequest, Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const NOTEBOOK_PLUGIN_ID: &str = "agena.notebook";

pub(crate) struct NotebookPlugin;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum NotebookEditAction {
    Replace,
    InsertBefore,
    InsertAfter,
    Delete,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum NotebookCellType {
    Code,
    Markdown,
    Raw,
}

impl NotebookCellType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Code => "code",
            Self::Markdown => "markdown",
            Self::Raw => "raw",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("path", "expected_sha256"), non_empty("path", "expected_sha256"))]
#[serde(deny_unknown_fields)]
struct NotebookEditInput {
    path: String,
    action: NotebookEditAction,
    cell_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cell_type: Option<NotebookCellType>,
    #[serde(default)]
    source: String,
    #[serde(default = "default_true")]
    preserve_outputs: bool,
    expected_sha256: String,
}

const fn default_true() -> bool {
    true
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "notebook",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Revision-safe Jupyter notebook cell editing.",
    display = detailed
)]
impl NotebookPlugin {
    pub(crate) fn new() -> Self {
        Self
    }

    #[tool(
        tags(mutate, filesystem),
        name = "edit_cell",
        summary = "Replace, insert, or delete one Jupyter notebook cell with a revision check.",
        mutating,


        display = detailed,
        path(requests = vec![PathRequest::read(input.path.clone()), PathRequest::write(input.path.clone())]),
        concurrency_safe
    )]
    async fn edit_cell(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &NotebookEditInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let path = resolve_path(context.workspace_root, input.path.as_str());
        if path.extension().and_then(|value| value.to_str()) != Some("ipynb") {
            return Err(PluginError::invalid_params(
                "notebook.edit_cell requires an .ipynb file",
            ));
        }
        let original = std::fs::read(&path).map_err(io_error)?;
        let before_sha256 = sha256(original.as_slice());
        if !before_sha256.eq_ignore_ascii_case(input.expected_sha256.trim()) {
            return Err(PluginError::invalid_params(format!(
                "stale notebook revision: expected {}, actual {before_sha256}",
                input.expected_sha256.trim()
            )));
        }
        let mut notebook: serde_json::Value =
            serde_json::from_slice(original.as_slice()).map_err(|error| {
                PluginError::invalid_params(format!("invalid notebook JSON: {error}"))
            })?;
        let cells = notebook
            .get_mut("cells")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| PluginError::invalid_params("notebook has no cells array"))?;

        match input.action {
            NotebookEditAction::Replace => {
                let cell_count = cells.len();
                let cell = cells.get_mut(input.cell_index).ok_or_else(|| {
                    PluginError::invalid_params(format!(
                        "cell_index {} is out of range for {} cells",
                        input.cell_index, cell_count
                    ))
                })?;
                let object = cell.as_object_mut().ok_or_else(|| {
                    PluginError::invalid_params("target notebook cell is not an object")
                })?;
                if let Some(cell_type) = input.cell_type {
                    object.insert(
                        "cell_type".to_string(),
                        serde_json::Value::String(cell_type.as_str().to_string()),
                    );
                }
                object.insert("source".to_string(), notebook_source(input.source.as_str()));
                if !input.preserve_outputs
                    && object.get("cell_type").and_then(serde_json::Value::as_str) == Some("code")
                {
                    object.insert("outputs".to_string(), serde_json::json!([]));
                    object.insert("execution_count".to_string(), serde_json::Value::Null);
                }
            }
            NotebookEditAction::InsertBefore | NotebookEditAction::InsertAfter => {
                if input.cell_index > cells.len()
                    || (matches!(input.action, NotebookEditAction::InsertAfter)
                        && input.cell_index >= cells.len())
                {
                    return Err(PluginError::invalid_params(format!(
                        "cell_index {} is out of range for {} cells",
                        input.cell_index,
                        cells.len()
                    )));
                }
                let cell_type = input.cell_type.unwrap_or(NotebookCellType::Code);
                let insert_at = if matches!(input.action, NotebookEditAction::InsertAfter) {
                    input.cell_index + 1
                } else {
                    input.cell_index
                };
                cells.insert(insert_at, new_cell(cell_type, input.source.as_str()));
            }
            NotebookEditAction::Delete => {
                if input.cell_index >= cells.len() {
                    return Err(PluginError::invalid_params(format!(
                        "cell_index {} is out of range for {} cells",
                        input.cell_index,
                        cells.len()
                    )));
                }
                cells.remove(input.cell_index);
            }
        }

        let cell_count = cells.len();
        let updated = serde_json::to_vec_pretty(&notebook).map_err(|error| {
            PluginError::internal(format!("cannot serialize notebook: {error}"))
        })?;
        atomic_write(&path, updated.as_slice())?;
        let after_sha256 = sha256(updated.as_slice());
        Ok(ToolInvokeOutput::from_parts(
            format!("edited notebook {}", input.path),
            format!(
                "{:?} cell {} · {cell_count} cells",
                input.action, input.cell_index
            ),
            format!(
                "Applied {:?} at cell {} in '{}' ({} cells, sha256 {} -> {}).",
                input.action, input.cell_index, input.path, cell_count, before_sha256, after_sha256
            ),
            Some(serde_json::json!({
                "path": input.path,
                "action": input.action,
                "cell_index": input.cell_index,
                "cell_count": cell_count,
                "before_sha256": before_sha256,
                "after_sha256": after_sha256,
            })),
            std::collections::BTreeMap::from([
                ("agena.effect".to_string(), "file_changes".to_string()),
                ("path".to_string(), input.path.clone()),
                ("after_sha256".to_string(), after_sha256),
            ]),
            Vec::new(),
        ))
    }
}

fn new_cell(cell_type: NotebookCellType, source: &str) -> serde_json::Value {
    match cell_type {
        NotebookCellType::Code => serde_json::json!({
            "cell_type": "code",
            "execution_count": null,
            "metadata": {},
            "outputs": [],
            "source": notebook_source(source),
        }),
        NotebookCellType::Markdown => serde_json::json!({
            "cell_type": "markdown",
            "metadata": {},
            "source": notebook_source(source),
        }),
        NotebookCellType::Raw => serde_json::json!({
            "cell_type": "raw",
            "metadata": {},
            "source": notebook_source(source),
        }),
    }
}

fn notebook_source(source: &str) -> serde_json::Value {
    serde_json::Value::Array(
        source
            .split_inclusive('\n')
            .map(|line| serde_json::Value::String(line.to_string()))
            .collect(),
    )
}

fn resolve_path(workspace_root: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        Path::new(workspace_root).join(path)
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn atomic_write(path: &Path, bytes: &[u8]) -> SdkResult<()> {
    let parent = path
        .parent()
        .ok_or_else(|| PluginError::invalid_params("notebook path has no parent directory"))?;
    let temp = parent.join(format!(
        ".agena-notebook-{}.tmp",
        uuid::Uuid::new_v4().simple()
    ));
    let result = (|| -> std::io::Result<()> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(io_error)
}

fn io_error(error: std::io::Error) -> PluginError {
    PluginError::internal(format!("notebook filesystem operation failed: {error}"))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{Plugin, ToolInvokeContext};

    use super::*;

    #[tokio::test]
    async fn replaces_cell_with_revision_check_and_clears_outputs() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("demo.ipynb");
        let original = serde_json::to_vec_pretty(&serde_json::json!({
            "cells": [{
                "cell_type": "code",
                "execution_count": 1,
                "metadata": {},
                "outputs": [{"output_type": "stream", "text": ["old"]}],
                "source": ["old"]
            }],
            "metadata": {},
            "nbformat": 4,
            "nbformat_minor": 5
        }))
        .expect("notebook");
        std::fs::write(&path, &original).expect("write notebook");
        let context = ToolInvokeContext {
            tool_name: "edit_cell",
            session_id: 1,
            call_id: 1,
            workspace_root: dir.path().to_str().expect("root"),
        };
        NotebookPlugin
            .edit_cell(
                &context,
                &NotebookEditInput {
                    path: "demo.ipynb".to_string(),
                    action: NotebookEditAction::Replace,
                    cell_index: 0,
                    cell_type: None,
                    source: "print('new')\n".to_string(),
                    preserve_outputs: false,
                    expected_sha256: sha256(&original),
                },
            )
            .await
            .expect("edit cell");
        let updated: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).expect("read updated notebook"))
                .expect("updated json");
        assert_eq!(updated["cells"][0]["source"][0], "print('new')\n");
        assert_eq!(updated["cells"][0]["outputs"], serde_json::json!([]));
        assert!(updated["cells"][0]["execution_count"].is_null());
    }

    #[test]
    fn manifest_exposes_notebook_cell_editor() {
        let manifest = NotebookPlugin.manifest();
        assert_eq!(manifest.tools.len(), 1);
        assert_eq!(manifest.tools[0].name, "edit_cell");
    }
}
