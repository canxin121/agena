//! Read Jupyter notebook cells and apply revision-safe edits.

use std::io::Read as _;
use std::path::{Path, PathBuf};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{Result as SdkResult, ToolInvokeContext, ToolInvokeOutput};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

mod read;
mod schema;
mod validation;

use read::NotebookReadInput;
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

    /// Base folder for a relative path. Only global (Bot) sessions use it;
    /// a project session keeps resolving against its project root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    folder: Option<String>,
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

#[derive(Clone, Copy, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum NotebookEditMode {
    #[default]
    Replace,
    Insert,
    Delete,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(trim("notebook_path"), non_empty("notebook_path"))]
#[serde(deny_unknown_fields)]
struct NotebookEditByIdInput {
    notebook_path: String,
    folder: Option<String>,
    /// Target cell ID. Omit to insert at the beginning.
    cell_id: Option<String>,
    /// Zero-based target for notebooks without cell IDs or older clients.
    cell_number: Option<usize>,
    #[serde(default)]
    edit_mode: NotebookEditMode,
    cell_type: Option<NotebookCellType>,
    new_source: Option<String>,
    expected_sha256: Option<String>,
}

struct CellEdit<'a> {
    action: NotebookEditAction,
    cell_index: usize,
    cell_type: Option<NotebookCellType>,
    source: &'a str,
    preserve_outputs: bool,
}

impl NotebookEditByIdInput {
    fn resolve(&self, cells: &[serde_json::Value]) -> SdkResult<CellEdit<'_>> {
        let target = match (self.cell_id.as_deref(), self.cell_number) {
            (Some(id), None) => {
                let mut matching = cells
                    .iter()
                    .enumerate()
                    .filter(|(_, cell)| cell["id"].as_str() == Some(id));
                let (index, _) = matching.next().ok_or_else(|| {
                    PluginError::invalid_params(format!("unknown notebook cell_id {id:?}"))
                })?;
                if matching.next().is_some() {
                    return Err(PluginError::invalid_params(format!(
                        "duplicate notebook cell_id {id:?}"
                    )));
                }
                Some(index)
            }
            (None, number) => number,
            _ => {
                return Err(PluginError::invalid_params(
                    "provide cell_id or cell_number",
                ));
            }
        };
        let (action, cell_index) = match (self.edit_mode, target) {
            (NotebookEditMode::Insert, _)
                if !matches!(
                    self.cell_type,
                    Some(NotebookCellType::Code | NotebookCellType::Markdown)
                ) =>
            {
                return Err(PluginError::invalid_params(
                    "insert requires a code or markdown cell_type",
                ));
            }
            (NotebookEditMode::Insert, Some(index)) if self.cell_id.is_some() => {
                (NotebookEditAction::InsertAfter, index)
            }
            (NotebookEditMode::Insert, index) => {
                (NotebookEditAction::InsertBefore, index.unwrap_or(0))
            }
            (NotebookEditMode::Replace, Some(index)) => (NotebookEditAction::Replace, index),
            (NotebookEditMode::Delete, Some(index)) => (NotebookEditAction::Delete, index),
            _ => {
                return Err(PluginError::invalid_params(
                    "replace and delete require a target cell",
                ));
            }
        };
        let source = match (action, self.new_source.as_deref()) {
            (NotebookEditAction::Delete, _) => "",
            (_, Some(source)) => source,
            _ => {
                return Err(PluginError::invalid_params(
                    "replace and insert require new_source",
                ));
            }
        };
        Ok(CellEdit {
            action,
            cell_index,
            cell_type: self.cell_type,
            source,
            preserve_outputs: false,
        })
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "notebook",
    version = env!("CARGO_PKG_VERSION"),
    display_name = "Notebooks",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/notebook.svg"))),
    authors = ["canxin121"],
    help = "Read notebook cells and saved outputs. Replace, insert or delete cells with revision checks. Code outputs are discarded by default when a cell is edited.",
    tags(agena_plugin_host::sdk::ToolTag::Filesystem, agena_plugin_host::sdk::ToolTag::Custom("notebooks".to_owned())),
    summary = "Revision-safe Jupyter notebook cell editing.",
    translations(
        locale("zh-CN", summary = "带版本校验的 Jupyter 笔记本单元格编辑。", help = "通过版本校验替换、插入或删除 Jupyter 笔记本单元格。代码输出默认清除，需要明确选择才能保留。"),
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
    #[hook(
        init,
        summary = "Initialize notebook permissions",
        help = "Binds permission services for notebook operations and workspace files.",
        translations(
            locale(
                "zh-CN",
                summary = "初始化笔记本权限",
                help = "准备笔记本权限功能并连接所需的 Host 服务。"
            ),
            locale(
                "zh-TW",
                summary = "初始化筆記本權限",
                help = "準備筆記本權限功能並連接所需的 Host 服務。"
            ),
            locale(
                "fr-FR",
                summary = "Initialiser les autorisations des notebooks",
                help = "Prépare les autorisations des notebooks et connecte les services hôtes nécessaires."
            ),
            locale(
                "es-ES",
                summary = "Inicializar los permisos de cuadernos",
                help = "Prepara los permisos de cuadernos y conecta los servicios del host necesarios."
            ),
            locale(
                "pt-BR",
                summary = "Inicializar as permissões de notebooks",
                help = "Prepara as permissões de notebooks e conecta os serviços necessários do host."
            ),
            locale(
                "de-DE",
                summary = "Die Notebook-Berechtigungen initialisieren",
                help = "Bereitet die Notebook-Berechtigungen vor und verbindet die erforderlichen Host-Dienste."
            ),
            locale(
                "ja-JP",
                summary = "ノートブックの権限制御を初期化",
                help = "必要なホストサービスに接続し、ノートブックの権限制御を利用できるようにします。"
            ),
            locale(
                "ko-KR",
                summary = "노트북 권한 초기화",
                help = "필요한 호스트 서비스에 연결하고 노트북 권한 기능을 준비합니다."
            ),
            locale(
                "hi-IN",
                summary = "नोटबुक अनुमतियाँ प्रारंभ करें",
                help = "“नोटबुक अनुमतियाँ” के लिए ज़रूरी होस्ट सेवाओं से जुड़कर संबंधित कार्यक्षमता तैयार करता है।"
            ),
            locale(
                "ar-SA",
                summary = "تهيئة أذونات دفاتر الملاحظات",
                help = "يُعِدّ أذونات دفاتر الملاحظات ويربط خدمات المضيف اللازمة."
            )
        )
    )]
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
        tags(query, filesystem, read_only),
        name = "read",
        summary = "Read Jupyter notebook cells, saved text outputs, and embedded figures.",
        help = "Use notebook_path and optional zero-based offset and limit to read an .ipynb file. The result includes cell IDs and a file SHA-256 for revision-checked edits. Use next_offset to continue a partial read.",
        translations(locale(
            "zh-CN",
            summary = "读取 Jupyter 笔记本单元格、已保存的文本输出和图片。"
        ))
    )]
    async fn read(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &NotebookReadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_file_operation(
            context,
            &input.notebook_path,
            input.folder.as_deref(),
            false,
            {
                let input = input.clone();
                move |target| read::read_file(&target, &input)
            },
        )
        .await
    }

    #[tool(
        tags(mutate, filesystem),
        name = "edit",
        summary = "Replace, insert, or delete a Jupyter notebook cell by its ID.",
        help = "Compatible with Claude NotebookEdit. Replace is the default. Insert adds a cell after cell_id, or at the beginning when no target is supplied. Specify code or markdown for insert. Older notebooks can use cell_number. Supply expected_sha256 from notebook.read to require that revision.",
        translations(locale(
            "zh-CN",
            summary = "按单元格 ID 替换、插入或删除 Jupyter 笔记本单元格。"
        ))
    )]
    async fn edit(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &NotebookEditByIdInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.run_file_operation(
            context,
            &input.notebook_path,
            input.folder.as_deref(),
            true,
            {
                let input = input.clone();
                move |target| {
                    edit_notebook_file(
                        &target,
                        &input.notebook_path,
                        input.expected_sha256.as_deref(),
                        |cells| input.resolve(cells),
                    )
                }
            },
        )
        .await
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
        self.run_file_operation(context, &input.path, input.folder.as_deref(), true, {
            let input = input.clone();
            move |target| edit_cell_file(&target, &input)
        })
        .await
    }

    async fn run_file_operation(
        &self,
        context: &ToolInvokeContext<'_>,
        path: &str,
        folder: Option<&str>,
        write: bool,
        operation: impl FnOnce(PathBuf) -> SdkResult<ToolInvokeOutput> + Send + 'static,
    ) -> SdkResult<ToolInvokeOutput> {
        super::fs::require_global_absolute_path_or_folder(context, path, folder)?;
        let root = super::fs::effective_root(context, folder);
        let target = resolve_path(&root.to_string_lossy(), path);
        let absolute = target.to_string_lossy();
        self.permissions
            .path(agena_domain::PathKind::Read, &absolute)
            .await?;
        if write {
            self.permissions
                .path(agena_domain::PathKind::Write, &absolute)
                .await?;
        }
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
            operation(target)
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

/// Pure notebook operation used by the isolated filesystem worker. The parent
/// holds the original-path capability and publishes this private copy only on
/// successful completion; no plugin host or runtime configuration is needed.
pub(crate) fn execute_sandbox_tool(
    operation: &str,
    input: serde_json::Value,
    workspace_root: &Path,
) -> SdkResult<ToolInvokeOutput> {
    if operation == "notebook.read" {
        let input: NotebookReadInput = serde_json::from_value(input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let target = resolve_path(&workspace_root.to_string_lossy(), &input.notebook_path);
        return read::read_file(&target, &input);
    }
    if operation == "notebook.edit" {
        let input: NotebookEditByIdInput = serde_json::from_value(input)
            .map_err(|error| PluginError::invalid_params_error(&error))?;
        let target = resolve_path(&workspace_root.to_string_lossy(), &input.notebook_path);
        return edit_notebook_file(
            &target,
            &input.notebook_path,
            input.expected_sha256.as_deref(),
            |cells| input.resolve(cells),
        );
    }
    let input: NotebookEditInput =
        serde_json::from_value(input).map_err(|error| PluginError::invalid_params_error(&error))?;
    let target = resolve_path(&workspace_root.to_string_lossy(), &input.path);
    edit_cell_file(&target, &input)
}

fn read_notebook_bytes(path: &Path) -> SdkResult<Vec<u8>> {
    let file = agena_tool::file_io::open_regular_file(path).map_err(io_error)?;
    let capacity = file.metadata().map_or(0, |metadata| {
        usize::try_from(metadata.len().min(MAX_NOTEBOOK_BYTES)).unwrap_or_default()
    });
    let mut bytes = Vec::with_capacity(capacity);
    file.take(MAX_NOTEBOOK_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(io_error)?;
    if bytes.len() as u64 > MAX_NOTEBOOK_BYTES {
        return Err(PluginError::invalid_params(format!(
            "notebook exceeds {} MiB",
            MAX_NOTEBOOK_BYTES / 1024 / 1024
        )));
    }
    Ok(bytes)
}

fn parse_notebook(bytes: &[u8]) -> SdkResult<serde_json::Value> {
    serde_json::from_slice(bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes)).map_err(|error| {
        PluginError::invalid_params(agena_failure::diagnostic::format_error_chain_with_context(
            "invalid notebook JSON",
            &error,
        ))
    })
}

fn edit_cell_file(path: &Path, input: &NotebookEditInput) -> SdkResult<ToolInvokeOutput> {
    edit_notebook_file(path, &input.path, Some(&input.expected_sha256), |_| {
        Ok(CellEdit {
            action: input.action,
            cell_index: input.cell_index,
            cell_type: input.cell_type,
            source: &input.source,
            preserve_outputs: input.preserve_outputs,
        })
    })
}

fn edit_notebook_file<'a>(
    path: &Path,
    display_path: &str,
    expected_sha256: Option<&str>,
    resolve: impl FnOnce(&[serde_json::Value]) -> SdkResult<CellEdit<'a>>,
) -> SdkResult<ToolInvokeOutput> {
    if path.extension().and_then(|value| value.to_str()) != Some("ipynb") {
        return Err(PluginError::invalid_params(
            "notebook editing requires an .ipynb file",
        ));
    }
    agena_runtime_tools::with_file_mutation_locks(&[path.to_path_buf()], || {
        let original = read_notebook_bytes(path)?;
        let before_sha256 = sha256(original.as_slice());
        if let Some(expected) = expected_sha256.map(str::trim)
            && !before_sha256.eq_ignore_ascii_case(expected)
        {
            return Err(PluginError::invalid_params(format!(
                "stale notebook revision: expected {expected}, actual {before_sha256}"
            )));
        }
        let mut notebook = parse_notebook(&original)?;
        let cells = notebook
            .get_mut("cells")
            .and_then(serde_json::Value::as_array_mut)
            .ok_or_else(|| PluginError::invalid_params("notebook has no cells array"))?;

        let input = resolve(cells)?;
        if input.source.len() > 16 * 1024 * 1024 {
            return Err(PluginError::invalid_params(
                "notebook source exceeds the 16 MiB byte limit",
            ));
        }
        let previous_source = cells
            .get(input.cell_index)
            .map(cell_source_text)
            .unwrap_or_default();
        let source_before = match input.action {
            NotebookEditAction::InsertBefore | NotebookEditAction::InsertAfter => None,
            _ => Some(previous_source.as_str()),
        };
        let source_after = if matches!(input.action, NotebookEditAction::Delete) {
            None
        } else {
            Some(input.source)
        };
        let cell_label = if matches!(input.action, NotebookEditAction::InsertAfter) {
            format!("after cell {}", input.cell_index)
        } else {
            format!("cell {}", input.cell_index)
        };
        let preview = agena_runtime_tools::file_diff_preview(
            &format!("{display_path} ({cell_label})"),
            source_before,
            source_after,
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
                let previous_type = object
                    .get("cell_type")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default()
                    .to_string();
                let kind = input
                    .cell_type
                    .map(|kind| kind.as_str().to_string())
                    .unwrap_or(previous_type.clone());
                if !matches!(kind.as_str(), "code" | "markdown" | "raw") {
                    return Err(PluginError::invalid_params(
                        "editing a future cell type requires an explicit supported cell_type",
                    ));
                }
                object.insert("cell_type".into(), serde_json::Value::String(kind.clone()));
                object.insert("source".to_string(), notebook_source(input.source));
                let preserve_outputs =
                    input.preserve_outputs && previous_type == "code" && kind == "code";
                outputs_may_be_stale = preserve_outputs
                    && object
                        .get("outputs")
                        .and_then(serde_json::Value::as_array)
                        .is_some_and(|outputs| !outputs.is_empty());
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
                cells.insert(insert_at, new_cell(cell_type, input.source));
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
        agena_runtime_tools::atomic_replace_file_with_check(path, updated.as_slice(), || {
            agena_runtime_tools::verify_file_contents(path, &original)
        })
        .map_err(io_error)?;
        let after_sha256 = sha256(updated.as_slice());
        Ok(ToolInvokeOutput::from_parts(
            format!("edited notebook {display_path}"),
            format!(
                "{:?} cell {} · {cell_count} cells",
                input.action, input.cell_index
            ),
            format!(
                "Applied {:?} at cell {} in '{}' ({} cells, sha256 {} -> {}).",
                input.action,
                input.cell_index,
                display_path,
                cell_count,
                before_sha256,
                after_sha256
            ),
            Some(serde_json::json!({
                "path": display_path,
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
                ("path".to_string(), display_path.to_owned()),
                ("after_sha256".to_string(), after_sha256),
            ]),
            Vec::new(),
        ))
    })
    .map_err(io_error)?
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

#[cfg(test)]
mod worker_tests {
    use super::*;

    #[test]
    fn notebook_bridge_reads_outputs_and_edits_by_cell_id() {
        use serde_json::json;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path();
        let path = root.join("exercise.ipynb");
        let image = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR4nGP4/5/hPwAH/QL+ppTFtAAAAABJRU5ErkJggg==";
        let notebook = json!({
            "nbformat":4, "nbformat_minor":5, "metadata":{},
            "cells":[
                {"id":"introduction","cell_type":"markdown","metadata":{},"source":["# Read\n"]},
                {"id":"observations","cell_type":"code","metadata":{},"source":"print(7)",
                 "execution_count":1,"outputs":[
                     {"output_type":"stream","name":"stdout","text":["7\n"]},
                     {"output_type":"display_data","metadata":{},"data":{"text/plain":"Plot","image/png":image}},
                 ]},
            ],
        });
        std::fs::write(&path, notebook.to_string()).unwrap();
        let read = |offset| {
            execute_sandbox_tool(
                "notebook.read",
                json!({"notebook_path":"exercise.ipynb", "offset":offset, "limit":1}),
                root,
            )
            .unwrap()
        };
        let first = read(0).payload.unwrap();
        assert_eq!(first["cells"][0]["cell_id"], "introduction");
        assert_eq!(first["next_offset"], 1);
        let second = read(1);
        let payload = second.payload.unwrap();
        assert_eq!(payload["cells"][0]["outputs"][0]["text"], "7\n");
        assert_eq!(
            payload["cells"][0]["outputs"][1]["image"]["attachment_index"],
            0
        );
        assert_eq!(second.attachments.len(), 1);
        assert_eq!(second.attachments[0].mime, "image/png");
        assert!(payload["next_offset"].is_null());

        let insert = json!({
            "notebook_path":"exercise.ipynb", "cell_id":"introduction", "edit_mode":"insert",
            "cell_type":"markdown", "new_source":"New paragraph", "expected_sha256":payload["sha256"],
        });
        execute_sandbox_tool("notebook.edit", insert.clone(), root).unwrap();
        assert!(execute_sandbox_tool("notebook.edit", insert, root).is_err());
        assert_eq!(
            read(1).payload.unwrap()["cells"][0]["source"],
            "New paragraph"
        );
        execute_sandbox_tool(
            "notebook.edit",
            json!({
                "notebook_path":"exercise.ipynb", "cell_id":"observations", "new_source":"print(9)",
            }),
            root,
        )
        .unwrap();
        assert!(read(2).attachments.is_empty());
        assert_eq!(read(2).payload.unwrap()["cells"][0]["source"], "print(9)");
        execute_sandbox_tool(
            "notebook.edit",
            json!({
                "notebook_path":"exercise.ipynb","cell_id":"observations","edit_mode":"delete",
            }),
            root,
        )
        .unwrap();
        assert_eq!(read(0).payload.unwrap()["cell_count"], 2);
    }

    #[test]
    fn isolated_notebook_operation_preserves_revision_and_cell_rules() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("notebook.ipynb");
        let original = serde_json::to_vec(&serde_json::json!({
            "nbformat": 4, "nbformat_minor": 5, "metadata": {},
            "cells": [{"id": "original", "cell_type": "code", "metadata": {},
                "source": ["print(1)"], "execution_count": 1,
                "outputs": [{"output_type": "stream", "name": "stdout", "text": ["1\n"]}]}]
        }))
        .unwrap();
        std::fs::write(&path, &original).unwrap();
        let request = serde_json::json!({
            "path": path, "expected_sha256": sha256(&original),
            "cell_index": 0, "action": "replace", "source": "print(2)"
        });
        let response =
            execute_sandbox_tool("notebook.edit_cell", request.clone(), directory.path()).unwrap();
        let updated = std::fs::read(&path).unwrap();
        let document: serde_json::Value = serde_json::from_slice(&updated).unwrap();
        assert!(
            document["cells"][0]["outputs"]
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(document["cells"][0]["execution_count"].is_null());
        assert_eq!(
            response.payload.as_ref().unwrap()["after_sha256"],
            sha256(&updated)
        );
        assert!(execute_sandbox_tool("notebook.edit_cell", request, directory.path()).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), updated);
    }
}
