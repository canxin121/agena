//! `agena.fs` plugin: filesystem read/write/search tools.

mod document;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use crate::part::{
    ApplyPatchToolInput, GlobToolInput, GrepToolInput, ReadMediaInput, ReadToolInput,
};
use crate::plugins::provided::router;
use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::{
    Result as SdkResult, ToolInvokeContext, ToolInvokeOutput, ToolStreamSink,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub(crate) const FS_PLUGIN_ID: &str = "agena.fs";
const MAX_MUTATING_TEXT_BYTES: u64 = 16 * 1024 * 1024;
const MAX_STAT_HASH_BYTES: u64 = 64 * 1024 * 1024;

pub(crate) struct FsPlugin;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("path", "expected_sha256"),
    non_empty("path"),
    non_empty_if_present("expected_sha256"),
    max_chars("content", 16777216)
)]
#[serde(deny_unknown_fields)]
struct WriteFileInput {
    path: String,
    content: String,
    #[serde(default)]
    create_parents: bool,
    /// Required when replacing an existing file. Use the hash returned by
    /// `fs.stat` or a prior mutating result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("path", "expected_sha256"),
    non_empty("path"),
    min_chars("old", 1),
    non_empty_if_present("expected_sha256"),
    minimum("expected_occurrences", 1),
    max_chars("old", 16777216),
    max_chars("new", 16777216)
)]
#[serde(deny_unknown_fields)]
struct ReplaceFileInput {
    path: String,
    old: String,
    new: String,
    #[serde(default = "default_expected_occurrences")]
    expected_occurrences: u32,
    #[serde(default)]
    replace_all: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expected_sha256: Option<String>,
}

const fn default_expected_occurrences() -> u32 {
    1
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(
    trim("paths[]"),
    non_empty("paths[]"),
    min_items("paths", 1),
    max_items("paths", 64),
    minimum("max_total_bytes", 1),
    maximum("max_total_bytes", 1048576)
)]
#[serde(deny_unknown_fields)]
struct ReadManyInput {
    paths: Vec<String>,
    #[serde(default = "default_read_many_budget")]
    max_total_bytes: u32,
}

const fn default_read_many_budget() -> u32 {
    131_072
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, JsonSchema, ToolInput)]
#[input(trim("path"), non_empty("path"))]
#[serde(deny_unknown_fields)]
struct StatInput {
    path: String,
    #[serde(default = "default_true")]
    hash: bool,
}

const fn default_true() -> bool {
    true
}

pub(crate) fn new_plugin() -> FsPlugin {
    FsPlugin
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "fs",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Filesystem command tools for read/search and explicit edits.",
    translations(
        locale("zh-CN", summary = "文件系统工具：读取、搜索并按需修改工作区文件。"),
        locale("zh-TW", summary = "檔案系統工具：讀取、搜尋並依需求修改工作區檔案。"),
        locale("ja-JP", summary = "ワークスペース内のファイルを読み取り、検索し、必要に応じて編集するためのツールです。"),
        locale("ko-KR", summary = "작업 공간 파일을 읽고 검색하고 필요에 따라 편집하는 파일 시스템 도구입니다."),
        locale("fr-FR", summary = "Outils de fichiers pour lire, rechercher et modifier les fichiers de l’espace de travail."),
        locale("de-DE", summary = "Dateisystemwerkzeuge zum Lesen, Durchsuchen und gezielten Bearbeiten von Dateien im Arbeitsbereich."),
        locale("es-ES", summary = "Herramientas de archivos para leer, buscar y editar los archivos del espacio de trabajo."),
        locale("hi-IN", summary = "वर्कस्पेस की फ़ाइलें पढ़ने, खोजने और ज़रूरत के अनुसार संपादित करने के फ़ाइल सिस्टम टूल।"),
        locale("ar-SA", summary = "أدوات لنظام الملفات تتيح قراءة ملفات مساحة العمل والبحث فيها وتعديلها عند الحاجة."),
        locale("pt-BR", summary = "Ferramentas de arquivos para ler, pesquisar e editar os arquivos do espaço de trabalho.")
    ),
)]
impl FsPlugin {
    #[tool(
        stream = invoke_document_stream,
        tags(query, filesystem, read_only),
        summary = "Extract or search text in one local PDF/Office document.",
        translations(
            locale("zh-CN", summary = "提取或搜索本地 PDF、Office 文档中的文本。"),
            locale("zh-TW", summary = "從本機 PDF 或 Office 文件擷取或搜尋文字。"),
            locale("ja-JP", summary = "ローカルの PDF／Office 文書からテキストを抽出または検索します。"),
            locale("ko-KR", summary = "로컬 PDF/Office 문서에서 텍스트를 추출하거나 검색합니다."),
            locale("fr-FR", summary = "Extraire ou rechercher du texte dans un document PDF ou Office local."),
            locale("de-DE", summary = "Text aus einem lokalen PDF- oder Office-Dokument auslesen oder darin suchen."),
            locale("es-ES", summary = "Extrae o busca texto en un documento PDF u Office local."),
            locale("hi-IN", summary = "किसी स्थानीय PDF/Office दस्तावेज़ से पाठ निकालें या उसमें खोजें।"),
            locale("ar-SA", summary = "استخرج النص من مستند PDF أو Office محلي أو ابحث فيه."),
            locale("pt-BR", summary = "Extraia ou pesquise texto em um documento PDF ou Office local.")
        ),
        help = "Supported formats: pdf, docx, pptx, xlsx. backend=auto prefers pdftotext for PDFs and otherwise uses selected local MarkItDown converters. MarkItDown needs a Python environment with its format extras; set AGENA_DOCUMENT_PYTHON to that interpreter, or install in the host python3 environment. No dependency installation, plugins, audio/image transcription or remote document service is enabled. Supply pattern to search extracted lines (fixed_strings defaults true); start_line is 1-based, max_lines 1–500. Outputs include source_sha256, extraction warnings, line counts and explicit truncation. Source limit 32 MiB; conversion 30 seconds / 2 MiB per output stream; displayed records 128 KiB. Empty text can indicate a scanned PDF needing OCR. Extracted lines are not source page numbers."
    )]
    async fn invoke_document(
        &self,
        context: &ToolInvokeContext<'_>,
        input: document::DocumentInput,
    ) -> SdkResult<ToolInvokeOutput> {
        document::invoke(Path::new(context.workspace_root), input).await
    }

    /// Streaming variant: extraction shells out to a local converter, so a
    /// reader sees the work before the extracted text arrives.
    async fn invoke_document_stream(
        &self,
        sink: ToolStreamSink,
        context: &ToolInvokeContext<'_>,
        input: document::DocumentInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Extracting document text…\n").await;
        document::invoke(Path::new(context.workspace_root), input).await
    }

    #[tool(
        tags(query, filesystem, read_only),
        summary = "Read workspace files.",
        translations(
            locale("zh-CN", summary = "读取工作区文件。"),
            locale("zh-TW", summary = "讀取工作區檔案。"),
            locale("ja-JP", summary = "ワークスペースのファイルを読み取ります。"),
            locale("ko-KR", summary = "작업 공간의 파일을 읽습니다."),
            locale("fr-FR", summary = "Lire les fichiers de l’espace de travail."),
            locale("de-DE", summary = "Dateien im Arbeitsbereich lesen."),
            locale("es-ES", summary = "Lee archivos del espacio de trabajo."),
            locale("hi-IN", summary = "वर्कस्पेस की फ़ाइलें पढ़ें।"),
            locale("ar-SA", summary = "اقرأ ملفات مساحة العمل."),
            locale("pt-BR", summary = "Leia arquivos do espaço de trabalho.")
        ),
        help = "Use read for text previews and directory listings. offset/limit page 1-based lines or entries. For very long single lines, use byte_offset (zero-based) and byte_limit (4–16384, default 8192) to seek directly to a small UTF-8 text range; continue with read_info.next_byte_offset. Byte ranges cannot combine with line offsets/limits, directories or attachment mode. Binary files return local references, not model-visible bytes. Call fs.read_media to see a local image, PDF, audio or video with the current conversation model; use fs.document when you only need extracted PDF/Office text."
    )]
    async fn invoke_read(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ReadToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "read", args).await
    }

    #[tool(
        tags(query, filesystem, read_only),
        summary = "Read a local image, PDF, audio or video into the current model's context.",
        translations(
            locale(
                "zh-CN",
                summary = "将本地图片、PDF、音频或视频读入当前模型的上下文。",
                help = "当需要查看本地图片或截图、检查 PDF 页面画面、收听音频，或查看本地视频时，调用 fs.read_media。它会像用户附件一样，把实际且不可变的文件内容提供给当前对话模型；无需额外配置云端分析服务或凭据。先用 tools_help 获取当前工具架构，再通过 tools_call 调用 tool=fs.read_media、input={path:...}；路径以当前工作区为基准。图片和 PDF 能否处理取决于当前模型。音频需要支持 OpenAI Chat 音频的模型或 Gemini；视频需要 Gemini。协议、模型、MIME 类型或文件大小不受支持时会明确报错；文件名或引用本身并不代表模型已经理解了媒体内容。支持准备 PNG/JPEG/GIF/WebP、PDF、常见音视频及 UTF-8 文本；每个文件最多 20 MiB，部分协议的编码后请求上限更低。可用 expected_sha256 校验文件版本。普通文本用 fs.read，提取 PDF/Office 文本用 fs.document；不要反复用 fs.read 读取二进制文件并期待模型看到内容。"
            ),
            locale(
                "zh-TW",
                summary = "將本機圖片、PDF、音訊或影片讀入目前模型的內容脈絡。"
            ),
            locale(
                "ja-JP",
                summary = "ローカルの画像、PDF、音声、動画を現在のモデルのコンテキストに読み込みます。"
            ),
            locale(
                "ko-KR",
                summary = "로컬 이미지, PDF, 오디오 또는 비디오를 현재 모델의 컨텍스트로 읽어옵니다."
            ),
            locale(
                "fr-FR",
                summary = "Charger une image, un PDF, un fichier audio ou une vidéo locale dans le contexte du modèle actuel."
            ),
            locale(
                "de-DE",
                summary = "Ein lokales Bild, PDF, Audio- oder Videomedium in den Kontext des aktuellen Modells laden."
            ),
            locale(
                "es-ES",
                summary = "Carga una imagen, PDF, audio o vídeo local en el contexto del modelo actual."
            ),
            locale(
                "hi-IN",
                summary = "स्थानीय छवि, PDF, ऑडियो या वीडियो को मौजूदा मॉडल के संदर्भ में पढ़ें।"
            ),
            locale(
                "ar-SA",
                summary = "أدخل صورة أو ملف PDF أو صوتًا أو فيديو محليًا إلى سياق النموذج الحالي."
            ),
            locale(
                "pt-BR",
                summary = "Carregue uma imagem, PDF, áudio ou vídeo local no contexto do modelo atual."
            )
        ),
        help = "Call fs.read_media whenever you need to see an image/screenshot, inspect PDF page visuals, listen to audio, or inspect a video stored locally. This returns actual immutable file bytes to the current conversation model, like a user attachment; no separate cloud-analysis provider or credential configuration is needed. Use tools_help for the live schema, then tools_call with tool=fs.read_media and input={path:...}; paths resolve from the active workspace. Image and PDF support depends on the current model. Audio requires a supported OpenAI Chat audio model or Gemini; video requires Gemini. Unsupported protocols, models, MIME types or sizes return an explicit error; a filename/reference is not media understanding. Supported preparation includes PNG/JPEG/GIF/WebP, PDF, common audio/video and UTF-8 text; maximum 20 MiB/file, with lower encoded-request limits on some protocols. expected_sha256 optionally guards the revision. Use fs.read for ordinary text, fs.document for extracted PDF/Office text; do not repeatedly read binary data with fs.read expecting to see it."
    )]
    async fn invoke_read_media(
        &self,
        context: &ToolInvokeContext<'_>,
        input: ReadMediaInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "read_media", input).await
    }

    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Find paths with glob patterns.",
        translations(
            locale("zh-CN", summary = "使用 glob 模式查找文件和目录路径。"),
            locale("zh-TW", summary = "使用 glob 模式尋找檔案與目錄路徑。"),
            locale(
                "ja-JP",
                summary = "glob パターンでファイルやディレクトリのパスを検索します。"
            ),
            locale("ko-KR", summary = "glob 패턴으로 파일과 디렉터리 경로를 찾습니다."),
            locale(
                "fr-FR",
                summary = "Rechercher des chemins de fichiers et de dossiers avec des motifs glob."
            ),
            locale(
                "de-DE",
                summary = "Datei- und Verzeichnispfade mit Glob-Mustern suchen."
            ),
            locale(
                "es-ES",
                summary = "Busca rutas de archivos y directorios con patrones glob."
            ),
            locale("hi-IN", summary = "glob पैटर्न से फ़ाइल और निर्देशिका पथ खोजें।"),
            locale(
                "ar-SA",
                summary = "اعثر على مسارات الملفات والمجلدات باستخدام أنماط glob."
            ),
            locale(
                "pt-BR",
                summary = "Encontre caminhos de arquivos e diretórios usando padrões glob."
            )
        ),
        help = "Use `glob` for focused path discovery before reading or editing files. Use kind=file/directory/all and exclude globs to narrow results. Results are paginated (default 200, maximum 1000); scans are cancellable, bounded to 100,000 entries / 10 seconds between I/O, and 256 KiB of paths. Pagination is deterministic for an unchanged directory tree. Ripgrep-compatible hidden/ignore rules are applied unless `include_ignored` is true or the base path explicitly names an ignored directory."
    )]
    async fn invoke_glob(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GlobToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "glob", args).await
    }

    #[tool(
        tags(query, filesystem, discovery, read_only),
        summary = "Search text with ripgrep, returning lines, paths, or counts.",
        translations(
            locale("zh-CN", summary = "用 ripgrep 搜索文本，并返回匹配行、路径或计数。"),
            locale("zh-TW", summary = "使用 ripgrep 搜尋文字，並傳回相符行、路徑或計數。"),
            locale(
                "ja-JP",
                summary = "ripgrep でテキストを検索し、一致した行、パス、件数を返します。"
            ),
            locale(
                "ko-KR",
                summary = "ripgrep으로 텍스트를 검색하고 일치하는 줄, 경로 또는 개수를 반환합니다."
            ),
            locale(
                "fr-FR",
                summary = "Rechercher du texte avec ripgrep et renvoyer les lignes, chemins ou nombres de résultats."
            ),
            locale(
                "de-DE",
                summary = "Text mit ripgrep durchsuchen und Trefferzeilen, Pfade oder Anzahlen zurückgeben."
            ),
            locale(
                "es-ES",
                summary = "Busca texto con ripgrep y devuelve líneas, rutas o recuentos."
            ),
            locale(
                "hi-IN",
                summary = "ripgrep से पाठ खोजें और मिलान वाली पंक्तियाँ, पथ या संख्या लौटाएँ।"
            ),
            locale(
                "ar-SA",
                summary = "ابحث في النص باستخدام ripgrep وأعد الأسطر أو المسارات أو أعداد النتائج."
            ),
            locale(
                "pt-BR",
                summary = "Pesquise texto com ripgrep e retorne linhas, caminhos ou contagens."
            )
        ),
        help = "Use regex or fixed_strings with case=sensitive/insensitive/smart. Pattern whitespace is significant. mode=content returns structured lines with optional before_context/after_context (0–20); files returns each matching path once; count returns matching-line counts per file, omitting zeroes. max_results is global (1–500): lines for content, files otherwise, never a per-file count cap. include and includes are ORed relative-path globs; exclude wins. Hidden/ignored paths follow ripgrep rules unless explicitly targeted or include_ignored=true. Search is bounded to 32 MiB/file, 256 MiB total, 25,000 files, 100,000 entries, 20 seconds between I/O/callbacks, and 256 KiB of records. Lines over 4 KiB are visibly shortened; lines beyond the 2 MiB search buffer may be skipped. scan_complete distinguishes an incomplete scan from clipped display text; partial counts are lower bounds. Files mode stops at its first match. Binary data detected while scanning is excluded. Narrow path or filters if truncated; use fs.read for nearby lines. Blocking filesystem I/O itself has no hard deadline."
    )]
    async fn invoke_grep(
        &self,
        context: &ToolInvokeContext<'_>,
        args: GrepToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "grep", args).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Apply a text patch to workspace files.",
        translations(
            locale("zh-CN", summary = "将文本补丁应用到工作区文件。"),
            locale("zh-TW", summary = "將文字修補程式套用至工作區檔案。"),
            locale(
                "ja-JP",
                summary = "テキストパッチをワークスペースのファイルに適用します。"
            ),
            locale("ko-KR", summary = "텍스트 패치를 작업 공간 파일에 적용합니다."),
            locale(
                "fr-FR",
                summary = "Appliquer un correctif textuel aux fichiers de l’espace de travail."
            ),
            locale(
                "de-DE",
                summary = "Einen Text-Patch auf Dateien im Arbeitsbereich anwenden."
            ),
            locale(
                "es-ES",
                summary = "Aplica un parche de texto a los archivos del espacio de trabajo."
            ),
            locale("hi-IN", summary = "वर्कस्पेस की फ़ाइलों पर टेक्स्ट पैच लागू करें।"),
            locale("ar-SA", summary = "طبّق تصحيحًا نصيًا على ملفات مساحة العمل."),
            locale(
                "pt-BR",
                summary = "Aplique um patch de texto aos arquivos do espaço de trabalho."
            )
        ),
        help = "Use `apply_patch` for explicit text patch operations against workspace files. The `patch` argument is a plain-text patch that MUST start with the exact marker line `*** Begin Patch` and end with the exact marker line `*** End Patch`. Inside, use only these directives: `*** Update File: <path>` followed by `@@`-separated hunks (context lines start with a space, removed lines with `-`, added lines with `+`), `*** Add File: <path>` with every content line prefixed by `+`, or `*** Delete File: <path>`. A patch that does not start with `*** Begin Patch` is rejected. Relative paths resolve against the current Agena workspace, not a previous shell command's working directory. When intentionally editing another worktree, use explicit absolute paths permitted by the runtime; a shell cd does not change subsequent file-tool path resolution."
    )]
    async fn invoke_apply_patch(
        &self,
        context: &ToolInvokeContext<'_>,
        args: ApplyPatchToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        invoke_internal(context, "apply_patch", args).await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Create a UTF-8 text file or replace one at an expected revision.",
        translations(
            locale("zh-CN", summary = "创建 UTF-8 文本文件，或按预期版本替换现有文件。"),
            locale("zh-TW", summary = "建立 UTF-8 文字檔，或依預期版本取代現有檔案。"),
            locale(
                "ja-JP",
                summary = "UTF-8 テキストファイルを作成するか、指定したリビジョンの既存ファイルを置き換えます。"
            ),
            locale(
                "ko-KR",
                summary = "UTF-8 텍스트 파일을 만들거나 지정한 리비전의 기존 파일을 교체합니다."
            ),
            locale(
                "fr-FR",
                summary = "Créer un fichier texte UTF-8 ou en remplacer un si sa révision correspond à celle attendue."
            ),
            locale(
                "de-DE",
                summary = "Eine UTF-8-Textdatei erstellen oder eine vorhandene Datei bei passender Revision ersetzen."
            ),
            locale(
                "es-ES",
                summary = "Crea un archivo de texto UTF-8 o reemplaza uno si coincide con la revisión esperada."
            ),
            locale(
                "hi-IN",
                summary = "UTF-8 टेक्स्ट फ़ाइल बनाएँ या अपेक्षित रिविज़न मिलने पर उसे बदलें।"
            ),
            locale(
                "ar-SA",
                summary = "أنشئ ملفًا نصيًا بترميز UTF-8 أو استبدله عند تطابق المراجعة المتوقعة."
            ),
            locale(
                "pt-BR",
                summary = "Crie um arquivo de texto UTF-8 ou substitua um existente se a revisão corresponder à esperada."
            )
        ),
        help = "Creating a new file needs no hash. Replacing an existing file requires expected_sha256 from fs.stat, preventing stale or parallel overwrites."
    )]
    async fn invoke_write(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &WriteFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&target), || {
                if input.content.len() as u64 > MAX_MUTATING_TEXT_BYTES {
                    return Err(PluginError::invalid_params(format!(
                        "fs.write supports content up to {} MiB",
                        MAX_MUTATING_TEXT_BYTES / 1024 / 1024
                    )));
                }
                let existed = target.exists();
                if existed {
                    if !target.is_file() {
                        return Err(PluginError::invalid_params(format!(
                            "write target is not a file: {}",
                            input.path
                        )));
                    }
                    let expected = input.expected_sha256.as_deref().ok_or_else(|| {
                        PluginError::invalid_params(
                            "expected_sha256 is required when fs.write replaces an existing file",
                        )
                    })?;
                    verify_expected_hash(&target, expected)?;
                } else if input.expected_sha256.is_some() {
                    return Err(PluginError::invalid_params(
                        "expected_sha256 was supplied but the target does not exist",
                    ));
                }
                // Capture the revision being replaced while holding the same mutation lock.
                let (original, diff_unavailable_reason) = if existed {
                    let mut bytes = Vec::new();
                    agena_tool::file_io::open_regular_file(&target)
                        .map_err(fs_error)?
                        .take(MAX_MUTATING_TEXT_BYTES + 1)
                        .read_to_end(&mut bytes)
                        .map_err(fs_error)?;
                    if bytes.len() as u64 > MAX_MUTATING_TEXT_BYTES {
                        (None, Some("Previous file exceeds the text preview limit."))
                    } else {
                        match String::from_utf8(bytes) {
                            Ok(text) if !text.contains('\0') => (Some(text), None),
                            _ => (
                                None,
                                Some("Previous file is binary; text comparison is unavailable."),
                            ),
                        }
                    }
                } else {
                    (Some(String::new()), None)
                };
                let preview = original.as_deref().map(|before| {
                    agena_runtime_tools::file_diff_preview(
                        &input.path,
                        existed.then_some(before),
                        Some(&input.content),
                    )
                });
                if let Some(parent) = target.parent()
                    && !parent.exists()
                {
                    if input.create_parents {
                        std::fs::create_dir_all(parent).map_err(fs_error)?;
                    } else {
                        return Err(PluginError::invalid_params(format!(
                            "parent directory does not exist: {}",
                            parent.display()
                        )));
                    }
                }
                if existed {
                    agena_runtime_tools::atomic_replace_file_with_check(
                        &target,
                        input.content.as_bytes(),
                        || {
                            verify_expected_hash(
                                &target,
                                input
                                    .expected_sha256
                                    .as_deref()
                                    .expect("existing target requires revision"),
                            )
                            .map_err(|error| {
                                std::io::Error::new(
                                    std::io::ErrorKind::InvalidData,
                                    error.to_string(),
                                )
                            })
                        },
                    )
                    .map_err(fs_error)?;
                } else {
                    agena_runtime_tools::atomic_create_file(
                        &target,
                        input.content.as_bytes(),
                        None,
                    )
                    .map_err(fs_error)?;
                }
                let hash = sha256_bytes(input.content.as_bytes());
                Ok(ToolInvokeOutput::from_parts(
                    format!(
                        "{} {}",
                        if existed { "updated" } else { "created" },
                        input.path
                    ),
                    format!(
                        "{} · {} bytes",
                        if existed { "Updated" } else { "Created" },
                        input.content.len()
                    ),
                    format!(
                        "{} '{}' ({} bytes, sha256={hash}).",
                        if existed { "Updated" } else { "Created" },
                        input.path,
                        input.content.len()
                    ),
                    Some(serde_json::json!({
                        "path": input.path,
                        "kind": if existed { "updated" } else { "created" },
                        "bytes": input.content.len(),
                        "sha256": hash,
                        "diff": preview.as_ref().map(|view| &view.diff),
                        "diff_truncated": preview.as_ref().is_some_and(|view| view.diff_truncated),
                        "additions": preview.as_ref().map(|view| view.additions),
                        "deletions": preview.as_ref().map(|view| view.deletions),
                        "diff_unavailable_reason": diff_unavailable_reason,
                    })),
                    std::collections::BTreeMap::from([
                        ("agena.effect".to_string(), "file_changes".to_string()),
                        ("path".to_string(), input.path.clone()),
                        ("sha256".to_string(), hash),
                    ]),
                    Vec::new(),
                ))
            })
            .map_err(fs_error)?
        })
        .await
    }

    #[tool(
        tags(mutate, filesystem),
        summary = "Replace exact UTF-8 text with occurrence and revision checks.",
        translations(
            locale(
                "zh-CN",
                summary = "按匹配次数和文件版本校验，替换完全一致的 UTF-8 文本。"
            ),
            locale(
                "zh-TW",
                summary = "依相符次數與檔案版本檢查，取代完全一致的 UTF-8 文字。"
            ),
            locale(
                "ja-JP",
                summary = "一致件数とリビジョンを確認して、完全一致する UTF-8 テキストを置き換えます。"
            ),
            locale(
                "ko-KR",
                summary = "일치 횟수와 리비전을 확인한 뒤 정확히 일치하는 UTF-8 텍스트를 바꿉니다."
            ),
            locale(
                "fr-FR",
                summary = "Remplacer un texte UTF-8 exact après vérification du nombre d’occurrences et de la révision."
            ),
            locale(
                "de-DE",
                summary = "Exakten UTF-8-Text nach Prüfung von Trefferzahl und Revision ersetzen."
            ),
            locale(
                "es-ES",
                summary = "Reemplaza texto UTF-8 exacto tras comprobar las coincidencias y la revisión."
            ),
            locale(
                "hi-IN",
                summary = "मिलान की संख्या और रिविज़न जाँचकर बिल्कुल मेल खाने वाला UTF-8 पाठ बदलें।"
            ),
            locale(
                "ar-SA",
                summary = "استبدل نص UTF-8 المطابق تمامًا بعد التحقق من عدد مرات التطابق والمراجعة."
            ),
            locale(
                "pt-BR",
                summary = "Substitua texto UTF-8 exato após conferir as ocorrências e a revisão do arquivo."
            )
        )
    )]
    async fn invoke_replace(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReplaceFileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let target = resolve_path(workspace_root.as_str(), input.path.as_str());
            agena_runtime_tools::with_file_mutation_locks(std::slice::from_ref(&target), || {
                if !target.is_file() {
                    return Err(PluginError::invalid_params(format!(
                        "replace target is not a file: {}",
                        input.path
                    )));
                }
                if input.old.is_empty() {
                    return Err(PluginError::invalid_params("old text must contain at least one byte"));
                }
                let original = String::from_utf8(read_file_bounded(
                    &target,
                    MAX_MUTATING_TEXT_BYTES,
                    "fs.replace",
                )?)
                .map_err(|_| {
                    PluginError::invalid_params(format!(
                        "replace target is not UTF-8 text: {}",
                        input.path
                    ))
                })?;
                let before_sha256 = sha256_bytes(original.as_bytes());
                if let Some(expected) = input.expected_sha256.as_deref()
                    && !before_sha256.eq_ignore_ascii_case(expected)
                {
                    return Err(PluginError::invalid_params(format!(
                        "stale file revision for '{}': expected sha256 {}, actual {}",
                        input.path, expected, before_sha256
                    )));
                }
                let occurrences = original.match_indices(input.old.as_str()).count();
                if occurrences != input.expected_occurrences as usize {
                    return Err(PluginError::invalid_params(format!(
                        "expected {} occurrence(s) of old text in '{}', found {occurrences}",
                        input.expected_occurrences, input.path
                    )));
                }
                let replacements = if input.replace_all { occurrences } else { 1 };
                let result_bytes = original.len()
                    .checked_sub(input.old.len().checked_mul(replacements).ok_or_else(|| PluginError::invalid_params("replacement size overflow"))?)
                    .and_then(|size| input.new.len().checked_mul(replacements).and_then(|added| size.checked_add(added)))
                    .filter(|size| *size as u64 <= MAX_MUTATING_TEXT_BYTES)
                    .ok_or_else(|| PluginError::invalid_params("fs.replace result exceeds the 16 MiB limit"))?;
                let updated = if input.replace_all {
                    original.replace(input.old.as_str(), input.new.as_str())
                } else {
                    original.replacen(input.old.as_str(), input.new.as_str(), 1)
                };
                if updated.len() as u64 > MAX_MUTATING_TEXT_BYTES {
                    return Err(PluginError::invalid_params(format!(
                        "fs.replace result exceeds the {} MiB limit",
                        MAX_MUTATING_TEXT_BYTES / 1024 / 1024
                    )));
                }
                agena_runtime_tools::atomic_replace_file_with_check(&target, updated.as_bytes(), || {
                    agena_runtime_tools::verify_file_contents(&target, original.as_bytes())
                }).map_err(fs_error)?;
                debug_assert_eq!(updated.len(), result_bytes);
                let after_sha256 = sha256_bytes(updated.as_bytes());
                let preview = agena_runtime_tools::file_diff_preview(&input.path, Some(&original), Some(&updated));
                Ok(ToolInvokeOutput::from_parts(
                    format!("replaced text in {}", input.path),
                    format!(
                        "{} replacements",
                        if input.replace_all { occurrences } else { 1 }
                    ),
                    format!(
                        "Replaced {} occurrence(s) in '{}' (sha256 {before_sha256} -> {after_sha256}).",
                        if input.replace_all { occurrences } else { 1 },
                        input.path
                    ),
                    Some(serde_json::json!({
                        "path": input.path,
                        "replacements": if input.replace_all { occurrences } else { 1 },
                        "before_sha256": before_sha256,
                        "after_sha256": after_sha256,
                        "diff": preview.diff,
                        "diff_truncated": preview.diff_truncated,
                        "additions": preview.additions,
                        "deletions": preview.deletions,
                    })),
                    std::collections::BTreeMap::from([
                        ("agena.effect".to_string(), "file_changes".to_string()),
                        ("path".to_string(), input.path.clone()),
                        ("before_sha256".to_string(), before_sha256),
                        ("after_sha256".to_string(), after_sha256),
                    ]),
                    Vec::new(),
                ))
            })
            .map_err(fs_error)?
        })
        .await
    }

    #[tool(
        tags(query, filesystem, read_only),
        summary = "Read multiple UTF-8 files within one bounded byte budget.",
        translations(
            locale("zh-CN", summary = "在限定的字节预算内读取多个 UTF-8 文件。"),
            locale("zh-TW", summary = "在限定的位元組預算內讀取多個 UTF-8 檔案。"),
            locale(
                "ja-JP",
                summary = "上限付きのバイト数で複数の UTF-8 ファイルを読み取ります。"
            ),
            locale(
                "ko-KR",
                summary = "제한된 바이트 예산 안에서 여러 UTF-8 파일을 읽습니다."
            ),
            locale(
                "fr-FR",
                summary = "Lire plusieurs fichiers UTF-8 dans un budget d’octets limité."
            ),
            locale(
                "de-DE",
                summary = "Mehrere UTF-8-Dateien innerhalb eines begrenzten Bytebudgets lesen."
            ),
            locale(
                "es-ES",
                summary = "Lee varios archivos UTF-8 dentro de un límite de bytes."
            ),
            locale("hi-IN", summary = "सीमित बाइट बजट के भीतर कई UTF-8 फ़ाइलें पढ़ें।"),
            locale("ar-SA", summary = "اقرأ عدة ملفات UTF-8 ضمن حدّ محدد لعدد البايتات."),
            locale(
                "pt-BR",
                summary = "Leia vários arquivos UTF-8 dentro de um limite de bytes."
            )
        )
    )]
    async fn invoke_read_many(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &ReadManyInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let mut remaining = input.max_total_bytes as usize;
            let mut sections = Vec::new();
            let mut entries = Vec::new();
            let mut truncated = false;
            let mut succeeded = 0;
            let mut failed = 0;
            for path in &input.paths {
                if remaining == 0 {
                    truncated = true;
                    entries.push(serde_json::json!({"path":path,"status":"not_read_budget","returned_bytes":0}));
                    continue;
                }
                let target = resolve_path(workspace_root.as_str(), path);
                let read = (|| {
                    let metadata = std::fs::metadata(&target).map_err(fs_error)?;
                    if !metadata.is_file() {
                        return Err(PluginError::invalid_params("not a regular file"));
                    }
                    // Reserve source bytes even if UTF-8 validation or a
                    // concurrent-change check fails after reading them.
                    let reserved = remaining.min(usize::try_from(metadata.len()).unwrap_or(usize::MAX));
                    remaining -= reserved;
                    let (preview, returned_bytes, file_truncated) = read_utf8_prefix(&target, reserved, path)?;
                    Ok((metadata.len(), preview, returned_bytes, file_truncated))
                })();
                match read {
                    Ok((bytes, preview, returned_bytes, file_truncated)) => {
                        sections.push(format!("===== {path} =====\n{preview}"));
                        let hash = (!file_truncated).then(|| sha256_bytes(preview.as_bytes()));
                        entries.push(serde_json::json!({"path":path,"status":"read","bytes":bytes,
                            "returned_bytes":returned_bytes,"truncated":file_truncated,"sha256":hash,"content":preview}));
                        truncated |= file_truncated;
                        succeeded += 1;
                    }
                    Err(error) => {
                        failed += 1;
                        sections.push(format!("===== {path} =====\nRead failed: {error}"));
                        entries.push(serde_json::json!({"path":path,"status":"error","error":error.to_string()}));
                    }
                }
            }
            Ok(ToolInvokeOutput::from_parts(
                format!("read {succeeded} files · {failed} failed"),
                if truncated {
                    format!("{} files · truncated", entries.len())
                } else {
                    format!("{} files", entries.len())
                },
                sections.join("\n\n"),
                Some(serde_json::json!({
                    "files": entries,
                    "success_count": succeeded,
                    "error_count": failed,
                    "max_total_bytes": input.max_total_bytes,
                    "remaining_bytes": remaining,
                    "truncated": truncated,
                })),
                std::collections::BTreeMap::from([
                    ("file_count".to_string(), entries.len().to_string()),
                    ("truncated".to_string(), truncated.to_string()),
                ]),
                Vec::new(),
            ))
        })
        .await
    }

    #[tool(
        tags(query, filesystem, read_only),
        summary = "Inspect file metadata and an optional SHA-256 revision.",
        translations(
            locale("zh-CN", summary = "查看文件元数据，并可选读取其 SHA-256 版本标识。"),
            locale(
                "zh-TW",
                summary = "檢視檔案中繼資料，並可選擇讀取 SHA-256 版本識別碼。"
            ),
            locale(
                "ja-JP",
                summary = "ファイルのメタデータと、必要に応じて SHA-256 リビジョンを確認します。"
            ),
            locale(
                "ko-KR",
                summary = "파일 메타데이터와 선택적 SHA-256 리비전을 확인합니다."
            ),
            locale(
                "fr-FR",
                summary = "Consulter les métadonnées du fichier et, si besoin, sa révision SHA-256."
            ),
            locale(
                "de-DE",
                summary = "Dateimetadaten und optional die SHA-256-Revision prüfen."
            ),
            locale(
                "es-ES",
                summary = "Consulta los metadatos del archivo y, opcionalmente, su revisión SHA-256."
            ),
            locale("hi-IN", summary = "फ़ाइल मेटाडेटा और वैकल्पिक SHA-256 रिविज़न देखें।"),
            locale(
                "ar-SA",
                summary = "افحص بيانات الملف الوصفية، ويمكنك أيضًا قراءة مراجعة SHA-256."
            ),
            locale(
                "pt-BR",
                summary = "Consulte os metadados do arquivo e, opcionalmente, sua revisão SHA-256."
            )
        )
    )]
    async fn invoke_stat(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &StatInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let workspace_root = context.workspace_root.to_string();
        let input = input.clone();
        run_fs_blocking(move || {
            let requested = Path::new(&input.path);
            let target = if requested.is_absolute() {
                requested.to_path_buf()
            } else {
                Path::new(&workspace_root).join(requested)
            };
            let mut metadata = std::fs::symlink_metadata(&target).map_err(fs_error)?;
            let mut hash_skipped = false;
            let hash = if input.hash && metadata.is_file() {
                let file = agena_tool::file_io::open_regular_file(&target).map_err(fs_error)?;
                metadata = file.metadata().map_err(fs_error)?;
                if !metadata.is_file() {
                    return Err(PluginError::invalid_params(
                        "stat target changed while opening it",
                    ));
                }
                if metadata.len() > MAX_STAT_HASH_BYTES {
                    hash_skipped = true;
                    None
                } else {
                    let mut digest = Sha256::new();
                    let mut reader = (&file).take(MAX_STAT_HASH_BYTES + 1);
                    let mut bytes = 0_u64;
                    let mut buffer = [0_u8; 64 * 1024];
                    loop {
                        let count = reader.read(&mut buffer).map_err(fs_error)?;
                        if count == 0 {
                            break;
                        }
                        bytes += count as u64;
                        if bytes > MAX_STAT_HASH_BYTES {
                            return Err(PluginError::invalid_params(
                                "stat file grew beyond its hash budget",
                            ));
                        }
                        digest.update(&buffer[..count]);
                    }
                    let after = file.metadata().map_err(fs_error)?;
                    if bytes != metadata.len()
                        || after.len() != metadata.len()
                        || after.modified().map_err(fs_error)?
                            != metadata.modified().map_err(fs_error)?
                    {
                        return Err(PluginError::invalid_params(
                            "stat file changed while hashing; retry the read",
                        ));
                    }
                    Some(hex::encode(digest.finalize()))
                }
            } else {
                None
            };
            let file_type = if metadata.file_type().is_symlink() {
                "symlink"
            } else if metadata.is_dir() {
                "directory"
            } else if metadata.is_file() {
                "file"
            } else {
                "other"
            };
            let modified_at_ms = metadata
                .modified()
                .map_err(fs_error)?
                .duration_since(UNIX_EPOCH)
                .map_err(|error| PluginError::internal_error(&error))?
                .as_millis();
            let modified_at_ms = i64::try_from(modified_at_ms)
                .map_err(|error| PluginError::internal_error(&error))?;
            let symlink_target = if metadata.file_type().is_symlink() {
                Some(
                    std::fs::read_link(&target)
                        .map_err(fs_error)?
                        .display()
                        .to_string(),
                )
            } else {
                None
            };
            let payload = serde_json::json!({
                "path": input.path,
                "kind": file_type,
                "size": metadata.len(),
                "modified_at_ms": modified_at_ms,
                "readonly": metadata.permissions().readonly(),
                "sha256": hash,
                "hash_skipped": hash_skipped,
                "symlink_target": symlink_target,
            });
            Ok(ToolInvokeOutput::from_parts(
                format!("stat {}", input.path),
                format!("{file_type} · {} bytes", metadata.len()),
                serde_json::to_string_pretty(&payload)
                    .map_err(|error| PluginError::internal_error(&error))?,
                Some(payload),
                std::collections::BTreeMap::new(),
                Vec::new(),
            ))
        })
        .await
    }
}

async fn invoke_internal<T: Serialize + Send + 'static>(
    context: &ToolInvokeContext<'_>,
    tool: &'static str,
    input: T,
) -> SdkResult<ToolInvokeOutput> {
    let input = json_input(input)?;
    let session_id = context.session_id;
    let call_id = context.call_id;
    run_fs_blocking(move || router::invoke_tool(tool, input, session_id, call_id)).await
}

async fn run_fs_blocking<T, F>(operation: F) -> SdkResult<T>
where
    T: Send + 'static,
    F: FnOnce() -> SdkResult<T> + Send + 'static,
{
    let worker_permit = crate::BLOCKING_PLUGIN_WORKERS
        .acquire()
        .await
        .map_err(|error| {
            PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                "acquire a filesystem plugin worker",
                &error,
            ))
        })?;
    tokio::task::spawn_blocking(move || {
        let _worker_permit = worker_permit;
        operation()
    })
    .await
    .map_err(|error| {
        PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
            "filesystem plugin worker failed",
            &error,
        ))
    })?
}

fn json_input<T: Serialize>(input: T) -> SdkResult<serde_json::Value> {
    serde_json::to_value(input).map_err(|err| PluginError::invalid_params_error(&err))
}

fn resolve_path(workspace_root: &str, path: &str) -> PathBuf {
    let path = Path::new(path);
    if path.is_absolute() {
        agena_runtime_tools::canonicalize_mutation_path(path)
    } else {
        agena_runtime_tools::canonicalize_mutation_path(&Path::new(workspace_root).join(path))
    }
}

fn sha256_bytes(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn sha256_file(path: &Path) -> SdkResult<String> {
    let file = agena_tool::file_io::open_regular_file(path).map_err(fs_error)?;
    let before = file.metadata().map_err(fs_error)?;
    if before.len() > MAX_STAT_HASH_BYTES {
        return Err(PluginError::invalid_params(
            "revision hashing supports files up to 64 MiB",
        ));
    }
    let mut reader = (&file).take(MAX_STAT_HASH_BYTES + 1);
    let mut bytes = 0;
    let mut digest = Sha256::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = reader.read(&mut buffer).map_err(fs_error)?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > MAX_STAT_HASH_BYTES {
            return Err(PluginError::invalid_params(
                "file grew beyond the 64 MiB revision hash budget",
            ));
        }
        digest.update(&buffer[..read]);
    }
    let after = file.metadata().map_err(fs_error)?;
    if before.len() != bytes
        || after.len() != bytes
        || before.modified().ok() != after.modified().ok()
    {
        return Err(PluginError::invalid_params(
            "file changed while computing its revision; retry",
        ));
    }
    Ok(hex::encode(digest.finalize()))
}

fn read_file_bounded(path: &Path, max_bytes: u64, operation: &str) -> SdkResult<Vec<u8>> {
    let mut file = agena_tool::file_io::open_regular_file(path).map_err(fs_error)?;
    let before = file.metadata().map_err(fs_error)?;
    if before.len() > max_bytes {
        return Err(PluginError::invalid_params(format!(
            "{operation} source exceeds its {} MiB limit",
            max_bytes / 1024 / 1024
        )));
    }
    let capacity = usize::try_from(before.len()).unwrap_or_default();
    let mut bytes = Vec::with_capacity(capacity);
    (&mut file)
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(fs_error)?;
    if bytes.len() as u64 > max_bytes {
        return Err(PluginError::invalid_params(format!(
            "{operation} supports files up to {} MiB: {}",
            max_bytes / 1024 / 1024,
            path.display()
        )));
    }
    let after = file.metadata().map_err(fs_error)?;
    if before.len() != after.len() || before.modified().ok() != after.modified().ok() {
        return Err(PluginError::invalid_params(format!(
            "{operation} source changed while reading; retry"
        )));
    }
    Ok(bytes)
}

fn read_utf8_prefix(
    path: &Path,
    max_bytes: usize,
    display_path: &str,
) -> SdkResult<(String, usize, bool)> {
    read_utf8_prefix_checked(path, max_bytes, display_path, || {})
}

fn read_utf8_prefix_checked(
    path: &Path,
    max_bytes: usize,
    display_path: &str,
    after_read: impl FnOnce(),
) -> SdkResult<(String, usize, bool)> {
    let mut file = agena_tool::file_io::open_regular_file(path).map_err(fs_error)?;
    let metadata = file.metadata().map_err(fs_error)?;
    let mut bytes = Vec::with_capacity(max_bytes.min(64 * 1024));
    file.by_ref()
        .take(max_bytes.saturating_add(1) as u64)
        .read_to_end(&mut bytes)
        .map_err(fs_error)?;
    after_read();
    let after = file.metadata().map_err(fs_error)?;
    if metadata.len() != after.len()
        || metadata.modified().ok() != after.modified().ok()
        || bytes.len() as u64 != metadata.len().min(max_bytes.saturating_add(1) as u64)
    {
        return Err(PluginError::invalid_params(
            "read_many source changed while reading; retry",
        ));
    }
    let truncated = bytes.len() > max_bytes;
    bytes.truncate(max_bytes);
    let valid_bytes = match std::str::from_utf8(&bytes) {
        Ok(_) => bytes.as_slice(),
        Err(error) if truncated && error.error_len().is_none() => &bytes[..error.valid_up_to()],
        Err(error) => {
            return Err(PluginError::invalid_params_with_public_detail(
                agena_failure::diagnostic::format_error_chain_with_context(
                    format!("read_many target is not UTF-8 text: {display_path}"),
                    &error,
                ),
                format!("The requested file is not UTF-8 text: {display_path}"),
            ));
        }
    };
    let text = std::str::from_utf8(valid_bytes)
        .expect("valid UTF-8 prefix was checked above")
        .to_string();
    Ok((text, valid_bytes.len(), truncated))
}

#[cfg(test)]
#[test]
fn read_many_does_not_issue_a_complete_revision_after_observed_growth() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("changing.txt");
    std::fs::write(&path, "before").unwrap();
    let result = read_utf8_prefix_checked(&path, 100, "changing.txt", || {
        std::fs::write(&path, "before and after").unwrap();
    });
    assert!(
        result
            .unwrap_err()
            .diagnostic_message()
            .contains("changed while reading")
    );
}

fn verify_expected_hash(path: &Path, expected: &str) -> SdkResult<()> {
    let actual = sha256_file(path)?;
    if actual.eq_ignore_ascii_case(expected.trim()) {
        Ok(())
    } else {
        Err(PluginError::invalid_params(format!(
            "stale file revision for '{}': expected sha256 {}, actual {}",
            path.display(),
            expected.trim(),
            actual
        )))
    }
}

#[cfg(test)]
#[test]
fn revision_hash_rejects_files_above_the_stat_budget() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("large");
    std::fs::File::create(&path)
        .unwrap()
        .set_len(MAX_STAT_HASH_BYTES + 1)
        .unwrap();
    assert!(sha256_file(&path).is_err());
}

fn fs_error(error: std::io::Error) -> PluginError {
    PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
        "filesystem operation failed",
        &error,
    ))
}

#[cfg(test)]
mod tests {
    use agena_plugin_host::sdk::{Plugin, ToolInvokeContext};

    use super::*;

    #[test]
    fn manifest_exposes_safe_high_frequency_file_tools() {
        let manifest = FsPlugin.manifest();
        assert_eq!(
            manifest.summary_for_locale("zh-CN"),
            Some("文件系统工具：读取、搜索并按需修改工作区文件。")
        );
        assert_eq!(
            manifest
                .tools
                .iter()
                .map(|tool| tool.name.as_str())
                .collect::<Vec<_>>(),
            [
                "document",
                "read",
                "read_media",
                "glob",
                "grep",
                "apply_patch",
                "write",
                "replace",
                "read_many",
                "stat",
            ]
        );
        for locale in [
            "zh-CN", "zh-TW", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "hi-IN", "ar-SA",
            "pt-BR",
        ] {
            assert_ne!(
                manifest.summary_for_locale(locale),
                manifest.summary.as_deref(),
                "filesystem plugin summary needs a native {locale} translation"
            );
            assert!(
                manifest.tools.iter().all(|tool| {
                    tool.docs.summary_for_locale(locale) != tool.docs.summary.as_deref()
                }),
                "every filesystem tool summary needs a native {locale} translation"
            );
        }
        assert!(
            manifest
                .tools
                .iter()
                .find(|tool| tool.name == "read_media")
                .and_then(|tool| tool.docs.help_for_locale("zh-CN"))
                .is_some_and(|help| help.starts_with("当需要查看本地图片或截图"))
        );
    }

    #[tokio::test]
    async fn write_requires_revision_before_overwriting_and_replace_checks_count() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().display().to_string();
        let context = ToolInvokeContext {
            tool_name: "write",
            session_id: 1,
            call_id: 1,
            workspace_root: root.as_str(),
        };
        let plugin = FsPlugin;
        let created = plugin
            .invoke_write(
                &context,
                &WriteFileInput {
                    path: "demo.txt".to_string(),
                    content: "one one".to_string(),
                    create_parents: false,
                    expected_sha256: None,
                },
            )
            .await
            .expect("create file");
        let created_payload = created.payload.unwrap();
        assert_eq!(created_payload["additions"], 1);
        assert_eq!(created_payload["deletions"], 0);
        assert!(
            created_payload["diff"]
                .as_str()
                .unwrap()
                .contains("+one one")
        );
        assert!(
            plugin
                .invoke_write(
                    &context,
                    &WriteFileInput {
                        path: "demo.txt".to_string(),
                        content: "stale".to_string(),
                        create_parents: false,
                        expected_sha256: None,
                    },
                )
                .await
                .is_err()
        );
        let hash = sha256_file(&dir.path().join("demo.txt")).expect("hash");
        let replaced = plugin
            .invoke_replace(
                &context,
                &ReplaceFileInput {
                    path: "demo.txt".to_string(),
                    old: "one".to_string(),
                    new: "two".to_string(),
                    expected_occurrences: 2,
                    replace_all: true,
                    expected_sha256: Some(hash),
                },
            )
            .await
            .expect("replace file");
        let replaced_payload = replaced.payload.unwrap();
        let diff = replaced_payload["diff"].as_str().unwrap();
        assert!(diff.contains("-one one"), "{diff}");
        assert!(diff.contains("+two two"), "{diff}");
        assert_eq!(replaced_payload["additions"], 1);
        assert_eq!(replaced_payload["deletions"], 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("demo.txt")).expect("read result"),
            "two two"
        );
    }

    #[tokio::test]
    async fn parallel_writes_with_one_revision_cannot_both_commit() {
        let dir = tempfile::tempdir().expect("temp dir");
        let root = dir.path().display().to_string();
        let context = ToolInvokeContext {
            tool_name: "write",
            session_id: 1,
            call_id: 1,
            workspace_root: root.as_str(),
        };
        let path = dir.path().join("race.txt");
        std::fs::write(&path, "original").expect("race fixture");
        let expected = sha256_file(&path).expect("fixture revision");
        let plugin = FsPlugin;
        let first = WriteFileInput {
            path: "race.txt".to_string(),
            content: "first".to_string(),
            create_parents: false,
            expected_sha256: Some(expected.clone()),
        };
        let second = WriteFileInput {
            path: "race.txt".to_string(),
            content: "second".to_string(),
            create_parents: false,
            expected_sha256: Some(expected),
        };

        let (first_result, second_result) = tokio::join!(
            plugin.invoke_write(&context, &first),
            plugin.invoke_write(&context, &second)
        );

        assert_ne!(first_result.is_ok(), second_result.is_ok());
        let final_text = std::fs::read_to_string(path).expect("final race content");
        assert!(matches!(final_text.as_str(), "first" | "second"));
    }

    #[test]
    fn byte_budget_never_splits_utf8() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("utf8.txt");
        std::fs::write(&path, "a你b").expect("write UTF-8 fixture");

        let (prefix, returned, truncated) =
            read_utf8_prefix(&path, 2, "utf8.txt").expect("bounded UTF-8 prefix");

        assert_eq!(prefix, "a");
        assert_eq!(returned, 1);
        assert!(truncated);
    }
}
