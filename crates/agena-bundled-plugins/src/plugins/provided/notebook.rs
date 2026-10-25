//! Revision-safe Jupyter notebook cell editing.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod schema;
mod validation;

pub(crate) const NOTEBOOK_PLUGIN_ID: &str = "agena.notebook";
const MAX_NOTEBOOK_BYTES: u64 = 32 * 1024 * 1024;

pub(crate) struct NotebookPlugin {
    permissions: crate::permissions::PluginPermissions,
}

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
#[input(
    trim("path", "expected_sha256"),
    non_empty("path", "expected_sha256"),
    max_chars("source", 16777216)
)]
#[serde(deny_unknown_fields)]
struct NotebookEditInput {
    path: String,
    action: NotebookEditAction,
    cell_index: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cell_type: Option<NotebookCellType>,
    #[serde(default)]
    source: String,
    /// Explicit opt-in to retaining code outputs; retained output may be stale.
    #[serde(default)]
    preserve_outputs: bool,
    expected_sha256: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "notebook",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Revision-safe Jupyter notebook cell editing.",
    translations(
        locale("zh-CN", summary = "带版本校验的 Jupyter 笔记本单元格编辑。"),
        locale("zh-TW", summary = "具版本檢查的 Jupyter 筆記本儲存格編輯。"),
        locale("ja-JP", summary = "リビジョンを確認しながら Jupyter ノートブックのセルを編集します。"),
        locale("ko-KR", summary = "리비전을 확인하며 Jupyter 노트북 셀을 편집합니다."),
        locale("fr-FR", summary = "Modifier des cellules Jupyter Notebook en vérifiant leur révision."),
        locale("de-DE", summary = "Jupyter-Notebookzellen mit Revisionsprüfung bearbeiten."),
        locale("es-ES", summary = "Edita celdas de Jupyter Notebook con comprobación de revisión."),
        locale("hi-IN", summary = "रिविज़न जाँच के साथ Jupyter Notebook सेल संपादित करें।"),
        locale("ar-SA", summary = "حرّر خلايا Jupyter Notebook مع التحقق من المراجعة."),
        locale("pt-BR", summary = "Edite células do Jupyter Notebook com verificação de revisão.")
    ),
)]
impl NotebookPlugin {
    #[hook(init)]
    async fn init(
        &self,
        _ctx: agena_plugin_host::sdk::InitContext,
        host: std::sync::Arc<dyn agena_plugin_host::sdk::host_api::HostClient>,
    ) -> SdkResult<agena_plugin_host::sdk::InitOutcome> {
        self.permissions.initialize(host)?;
        Ok(agena_plugin_host::sdk::InitOutcome::ack(
            agena_plugin_host::sdk::Plugin::manifest(self),
        ))
    }

    pub(crate) fn new() -> Self {
        Self {
            permissions: Default::default(),
        }
    }

    #[tool(
        tags(mutate, filesystem),
        name = "edit_cell",
        summary = "Replace, insert, or delete one Jupyter notebook cell with a revision check.",
        translations(
            locale(
                "zh-CN",
                summary = "校验版本后替换、插入或删除一个 Jupyter 笔记本单元格。"
            ),
            locale(
                "zh-TW",
                summary = "檢查版本後替換、插入或刪除一個 Jupyter 筆記本儲存格。"
            ),
            locale(
                "ja-JP",
                summary = "リビジョンを確認して Jupyter ノートブックのセルを置換・挿入・削除します。"
            ),
            locale(
                "ko-KR",
                summary = "리비전을 확인한 뒤 Jupyter 노트북 셀 하나를 바꾸거나 삽입 또는 삭제합니다."
            ),
            locale(
                "fr-FR",
                summary = "Remplacer, insérer ou supprimer une cellule Jupyter après vérification de sa révision."
            ),
            locale(
                "de-DE",
                summary = "Eine Jupyter-Notebookzelle nach Revisionsprüfung ersetzen, einfügen oder löschen."
            ),
            locale(
                "es-ES",
                summary = "Sustituye, inserta o elimina una celda de Jupyter Notebook tras comprobar su revisión."
            ),
            locale(
                "hi-IN",
                summary = "रिविज़न जाँचकर Jupyter Notebook की एक सेल बदलें, जोड़ें या हटाएँ।"
            ),
            locale(
                "ar-SA",
                summary = "استبدل خلية واحدة أو أدرجها أو احذفها في Jupyter Notebook بعد التحقق من المراجعة."
            ),
            locale(
                "pt-BR",
                summary = "Substitua, insira ou exclua uma célula do Jupyter Notebook após verificar a revisão."
            )
        )
    )]
    async fn edit_cell(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &NotebookEditInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.permissions
            .path(agena_domain::PathKind::Read, &input.path)
            .await?;
        self.permissions
            .path(agena_domain::PathKind::Write, &input.path)
            .await?;
        if input.source.len() > 16 * 1024 * 1024 {
            return Err(PluginError::invalid_params(
                "notebook source exceeds the 16 MiB byte limit",
            ));
        }
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
            .acquire()
            .await
            .map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "acquire a notebook plugin worker",
                    &error,
                ))
            })?;
        tokio::task::spawn_blocking(move || {
            let _worker_permit = worker_permit;
            let path = resolve_path(workspace_root.as_str(), input.path.as_str());
            if path.extension().and_then(|value| value.to_str()) != Some("ipynb") {
                return Err(PluginError::invalid_params(
                    "notebook.edit_cell requires an .ipynb file",
                ));
            }
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&path), || {
                let file = agena_tool::file_io::open_regular_file(&path).map_err(io_error)?;
                let capacity = match file.metadata() {
                    Ok(metadata) => usize::try_from(metadata.len().min(MAX_NOTEBOOK_BYTES))
                        .unwrap_or_default(),
                    Err(error) => {
                        tracing::warn!(
                            diagnostic = %agena_failure::diagnostic::format_error_chain_with_context(
                                "read notebook metadata for bounded preallocation",
                                &error,
                            ),
                            "notebook edit is continuing without a preallocated read buffer"
                        );
                        0
                    }
                };
                let mut original = Vec::with_capacity(capacity);
                file.take(MAX_NOTEBOOK_BYTES.saturating_add(1))
                    .read_to_end(&mut original)
                    .map_err(io_error)?;
                if original.len() as u64 > MAX_NOTEBOOK_BYTES {
                    return Err(PluginError::invalid_params(format!(
                        "notebook.edit_cell supports notebooks up to {} MiB",
                        MAX_NOTEBOOK_BYTES / 1024 / 1024
                    )));
                }
                let before_sha256 = sha256(original.as_slice());
                if !before_sha256.eq_ignore_ascii_case(input.expected_sha256.trim()) {
                    return Err(PluginError::invalid_params(format!(
                        "stale notebook revision: expected {}, actual {before_sha256}",
                        input.expected_sha256.trim()
                    )));
                }
                let mut notebook: serde_json::Value = serde_json::from_slice(original.as_slice())
                    .map_err(|error| {
                        PluginError::invalid_params(
                            agena_failure::diagnostic::format_error_chain_with_context(
                                "invalid notebook JSON",
                                &error,
                            ),
                        )
                    })?;
                let cells = notebook
                    .get_mut("cells")
                    .and_then(serde_json::Value::as_array_mut)
                    .ok_or_else(|| PluginError::invalid_params("notebook has no cells array"))?;

                let previous_source = cells.get(input.cell_index).map(cell_source_text).unwrap_or_default();
                let source_before = match input.action {
                    NotebookEditAction::InsertBefore | NotebookEditAction::InsertAfter => None,
                    _ => Some(previous_source.as_str()),
                };
                let source_after = if matches!(input.action, NotebookEditAction::Delete) { None } else { Some(input.source.as_str()) };
                let cell_label = if matches!(input.action, NotebookEditAction::InsertAfter) {
                    format!("after cell {}", input.cell_index)
                } else {
                    format!("cell {}", input.cell_index)
                };
                let preview = agena_runtime_tools::file_diff_preview(
                    &format!("{} ({cell_label})", input.path), source_before, source_after,
                );
                let mut outputs_may_be_stale = false;

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
                        let previous_type = object.get("cell_type").and_then(serde_json::Value::as_str).unwrap_or_default().to_string();
                        let kind = input.cell_type.map(|kind| kind.as_str().to_string()).unwrap_or(previous_type.clone());
                        if !matches!(kind.as_str(), "code" | "markdown" | "raw") {
                            return Err(PluginError::invalid_params("editing a future cell type requires an explicit supported cell_type"));
                        }
                        object.insert("cell_type".into(), serde_json::Value::String(kind.clone()));
                        object.insert("source".to_string(), notebook_source(input.source.as_str()));
                        let preserve_outputs = input.preserve_outputs && previous_type == "code" && kind == "code";
                        outputs_may_be_stale = preserve_outputs && object.get("outputs").and_then(serde_json::Value::as_array).is_some_and(|outputs| !outputs.is_empty());
                        validation::normalize_cell(object, &kind, preserve_outputs);
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
                validation::validate(&mut notebook)?;
                let updated = validation::serialize(&notebook)?;
                agena_runtime_tools::atomic_replace_file_with_check(&path, updated.as_slice(), || {
                    agena_runtime_tools::verify_file_contents(&path, &original)
                }).map_err(io_error)?;
                let after_sha256 = sha256(updated.as_slice());
                Ok(ToolInvokeOutput::from_parts(
                    format!("edited notebook {}", input.path),
                    format!(
                        "{:?} cell {} · {cell_count} cells",
                        input.action, input.cell_index
                    ),
                    format!(
                        "Applied {:?} at cell {} in '{}' ({} cells, sha256 {} -> {}).",
                        input.action,
                        input.cell_index,
                        input.path,
                        cell_count,
                        before_sha256,
                        after_sha256
                    ),
                    Some(serde_json::json!({
                        "path": input.path,
                        "action": input.action,
                        "cell_index": input.cell_index,
                        "cell_count": cell_count,
                        "before_sha256": before_sha256,
                        "after_sha256": after_sha256,
                        "outputs_may_be_stale": outputs_may_be_stale,
                        "diff": preview.diff,
                        "diff_truncated": preview.diff_truncated,
                        "additions": preview.additions,
                        "deletions": preview.deletions,
                    })),
                    std::collections::BTreeMap::from([
                        ("agena.effect".to_string(), "file_changes".to_string()),
                        ("path".to_string(), input.path.clone()),
                        ("after_sha256".to_string(), after_sha256),
                    ]),
                    Vec::new(),
                ))
            })
            .map_err(io_error)?
        })
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "notebook plugin worker failed",
                &error,
            ))
        })?
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
        agena_runtime_tools::canonicalize_mutation_path(path)
    } else {
        agena_runtime_tools::canonicalize_mutation_path(&Path::new(workspace_root).join(path))
    }
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn io_error(error: std::io::Error) -> PluginError {
    PluginError::internal(format!("notebook filesystem operation failed: {error}"))
}

fn cell_source_text(cell: &serde_json::Value) -> String {
    match cell.get("source") {
        Some(serde_json::Value::String(text)) => text.clone(),
        Some(serde_json::Value::Array(lines)) => {
            lines.iter().filter_map(serde_json::Value::as_str).collect()
        }
        _ => String::new(),
    }
}
