//! ChatGPT/OpenAI official provider tools exposed as ordinary Agena tools.

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use agena_macros::ToolInput;
use agena_plugin_host::PluginError;
use agena_plugin_host::sdk::host_api::HostClient;
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Result as SdkResult, ToolInvokeOutput, ToolStreamSink,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cloud_media::{AnalyzeInput, FileInput, Service, UploadInput};
use super::official_service::{
    ProviderHttpResponse, ProviderUsageKind, append_prompt_to_items, configured_model, endpoint,
    env_secret, merge_object_options, post_json, provider_output, read_image_input_bounded,
    read_json_response_bounded, resolve_local_path, stable_cache_key,
};
use agena_plugin_host::sdk::ToolInvokeContext;

pub(crate) const CHATGPT_PLUGIN_ID: &str = "agena.chatgpt";

pub(crate) struct ChatGptToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<ChatGptToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ChatGptToolsConfig {
    base_url: String,
    api_key_env: String,
    model: Option<String>,
    image_model: Option<String>,
    timeout_secs: u64,
    cache_namespace: String,
    cache_mode: OpenAiPromptCacheMode,
    stable_instructions: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum OpenAiPromptCacheMode {
    #[default]
    Automatic,
    Explicit,
    Disabled,
}

impl Default for ChatGptToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.openai.com/v1".to_owned(),
            api_key_env: "OPENAI_API_KEY".to_owned(),
            model: None,
            image_model: None,
            timeout_secs: 180,
            cache_namespace: "agena-provider-tools".to_owned(),
            cache_mode: OpenAiPromptCacheMode::Automatic,
            stable_instructions: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_instructions"),
    max_chars("prompt", 64000),
    max_chars("stable_instructions", 256000)
)]
#[serde(deny_unknown_fields)]
struct ChatGptToolInput {
    /// Instruction for a new hosted request. May be omitted when message history is supplied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable developer prefix eligible for an explicit OpenAI cache breakpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_instructions: Option<String>,
    /// Optional model override; otherwise plugin config, CHATGPT_MODEL, or OPENAI_MODEL is used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Official fields merged into this tool's declaration. `type` is protected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    /// Additional Responses request fields. `model`, `input`, `tools`, and `stream` are protected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    /// Responses API continuation token from an earlier call.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    /// Responses message history for hosted follow-up. Client Function/Computer/Patch/MCP/Shell callback items are rejected; use previous_response_id for hosted continuation.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_items: Vec<serde_json::Value>,
    /// Optional Responses include selectors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    include: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "images[]"),
    non_empty("prompt", "images[]"),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct ChatGptImageEditInput {
    prompt: String,
    images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    options: BTreeMap<String, serde_json::Value>,
}

impl ChatGptToolsPlugin {
    fn media_service(&self) -> SdkResult<Service<'_>> {
        Ok(Service {
            provider: "chatgpt",
            root: self.workspace_root()?,
            host: self.host()?,
            base_url: &self.config()?.base_url,
            key_env: &self.config()?.api_key_env,
            timeout_secs: self.config()?.timeout_secs,
            anthropic_version: "2023-06-01",
        })
    }

    pub(crate) fn new() -> Self {
        Self {
            host: OnceLock::new(),
            workspace_root: OnceLock::new(),
            config: OnceLock::new(),
        }
    }

    fn host(&self) -> SdkResult<&Arc<dyn HostClient>> {
        self.host
            .get()
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn config(&self) -> SdkResult<&ChatGptToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("ChatGPT tools plugin invoked before init"))
    }

    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["CHATGPT_MODEL", "OPENAI_MODEL"],
            tool,
        )
    }

    fn image_model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.image_model.as_deref(),
            &["CHATGPT_IMAGE_MODEL", "OPENAI_IMAGE_MODEL"],
            tool,
        )
    }

    async fn responses_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        input: ChatGptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.tool_options, "tool_options")?;
        super::official_service::hosted::validate_options(
            &input.request_options,
            "request_options",
        )?;
        super::official_service::hosted::validate_history(
            "chatgpt",
            &serde_json::json!(&input.input_items),
        )?;
        let model = self.model(input.model, format!("chatgpt.cloud_{tool_name}").as_str())?;
        let declaration =
            merge_object_options(declaration, &input.tool_options, &["type"], "tool_options")?;
        super::official_service::hosted::validate_declaration("chatgpt", tool_name, &declaration)?;
        let previous_response_id = input.previous_response_id.clone();
        let mut provider_input = append_prompt_to_items(input.input_items, input.prompt, true)?;
        let stable_instructions = input
            .stable_instructions
            .or_else(|| self.config().ok()?.stable_instructions.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let cache_mode = self.config()?.cache_mode;
        let supports_explicit_cache = model.to_ascii_lowercase().starts_with("gpt-5.6");
        if matches!(cache_mode, OpenAiPromptCacheMode::Explicit) && !supports_explicit_cache {
            return Err(PluginError::invalid_params(
                "explicit OpenAI prompt caching requires a GPT-5.6 or later model",
            ));
        }
        if matches!(cache_mode, OpenAiPromptCacheMode::Explicit)
            && previous_response_id.is_none()
            && stable_instructions.is_none()
        {
            return Err(PluginError::invalid_params(
                "explicit OpenAI prompt caching requires stable_instructions on the initial request",
            ));
        }
        if previous_response_id.is_none()
            && let Some(stable) = stable_instructions.as_deref()
        {
            let content = if matches!(cache_mode, OpenAiPromptCacheMode::Explicit) {
                serde_json::json!([{
                    "type": "input_text",
                    "text": stable,
                    "prompt_cache_breakpoint": {"mode": "explicit"}
                }])
            } else {
                serde_json::json!([{
                    "type": "input_text",
                    "text": stable
                }])
            };
            let developer = serde_json::json!({
                "role": "developer",
                "content": content
            });
            provider_input = match provider_input {
                serde_json::Value::Array(mut items) => {
                    items.insert(0, developer);
                    serde_json::Value::Array(items)
                }
                value => serde_json::Value::Array(vec![
                    developer,
                    serde_json::json!({"role":"user","content":value}),
                ]),
            };
        }
        let mut base = serde_json::json!({
            "model": model.clone(),
            "input": provider_input,
            "tools": [declaration],
            "stream": false,
        });
        if !matches!(cache_mode, OpenAiPromptCacheMode::Disabled) {
            base["prompt_cache_key"] = serde_json::Value::String(stable_cache_key(
                self.config()?.cache_namespace.as_str(),
                self.workspace_root()?,
                "chatgpt",
                model.as_str(),
                tool_name,
            ));
        }
        if supports_explicit_cache && !matches!(cache_mode, OpenAiPromptCacheMode::Disabled) {
            base["prompt_cache_options"] = match cache_mode {
                OpenAiPromptCacheMode::Automatic => {
                    serde_json::json!({"mode":"implicit","ttl":"30m"})
                }
                OpenAiPromptCacheMode::Explicit => {
                    serde_json::json!({"mode":"explicit","ttl":"30m"})
                }
                OpenAiPromptCacheMode::Disabled => unreachable!(),
            };
        }
        if let Some(previous_response_id) = previous_response_id {
            base["previous_response_id"] = serde_json::Value::String(previous_response_id);
        }
        if !input.include.is_empty() {
            base["include"] = serde_json::to_value(&input.include).map_err(|error| {
                PluginError::internal(agena_failure::diagnostic::format_error_chain_with_context(
                    "serialize ChatGPT Responses include fields",
                    &error,
                ))
            })?;
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "input",
                "tools",
                "stream",
                "previous_response_id",
                "prompt_cache_key",
                "prompt_cache_options",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "responses")?;
        let headers = BTreeMap::from([
            (
                "authorization".to_owned(),
                format!(
                    "Bearer {}",
                    env_secret(self.config()?.api_key_env.as_str(), "ChatGPT/OpenAI")?
                ),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            self.config()?.timeout_secs,
            "chatgpt",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "chatgpt",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::OpenAiResponses,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "chatgpt",
    version = env!("CARGO_PKG_VERSION"),
    summary = "OpenAI cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "使用 OpenAI 云端搜索、计算和图像能力。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "使用 OpenAI 雲端搜尋、運算與影像功能。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "OpenAI のクラウド検索・計算・画像機能を利用します。入力はこの端末の外部に送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "OpenAI 클라우드 검색, 계산, 이미지 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Utiliser les fonctions de recherche, de calcul et d’image dans le cloud OpenAI. Les données sont envoyées hors de cet ordinateur, sans solution de repli locale."),
        locale("de-DE", summary = "OpenAI-Cloudfunktionen für Suche, Berechnungen und Bilder nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Usa las funciones de búsqueda, cálculo e imagen en la nube de OpenAI. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "OpenAI क्लाउड की खोज, गणना और छवि सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "استخدم إمكانات البحث والحوسبة والصور في سحابة OpenAI. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Use os recursos de busca, computação e imagem na nuvem da OpenAI. As entradas saem deste computador; não há alternativa de execução local.")
    ),
    settings = ChatGptToolsConfig,
    settings_default = default,
)]
impl ChatGptToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ChatGptToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid ChatGPT tools plugin config",
            )?;
        self.workspace_root.set(ctx.workspace_root).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        self.config.set(config).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        self.host.set(host).map_err(|_| {
            PluginError::internal("ChatGPT tools plugin initialized more than once")
        })?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        name = "cloud_image_understanding",
        tags(query, network, mutate),
        summary = "Send explicit images to OpenAI cloud for understanding; not local file viewing.",
        help = "Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل صورًا محددة صراحةً إلى سحابة OpenAI لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene Bilder zur Analyse an die OpenAI-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía imágenes explícitas a la nube de OpenAI para comprenderlas; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des images explicites au cloud OpenAI pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दी गई छवियाँ OpenAI क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した画像を OpenAI クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 이미지를 OpenAI 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia imagens explícitas para a nuvem da OpenAI para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的图像发送到 OpenAI 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的圖像傳送至 OpenAI 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn image_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "chatgpt.cloud_image_understanding")?;
        self.media_service()?
            .analyze(context, input, model, true)
            .await
    }

    #[tool(
        name = "cloud_document_understanding",
        tags(query, network, mutate),
        summary = "Send explicit PDF/text documents to OpenAI cloud for understanding; not local file viewing.",
        help = "Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل مستندات PDF/نصية محددة صراحةً إلى سحابة OpenAI لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene PDF-/Textdokumente zur Analyse an die OpenAI-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía documentos PDF/texto explícitos a la nube de OpenAI para comprenderlos; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des documents PDF/textuels explicites au cloud OpenAI pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दिए गए PDF/टेक्स्ट दस्तावेज़ OpenAI क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した PDF／テキスト文書を OpenAI クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 PDF/텍스트 문서를 OpenAI 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia documentos PDF/texto explícitos para a nuvem da OpenAI para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的 PDF/文本文档发送到 OpenAI 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的 PDF／文字文件傳送至 OpenAI 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn document_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "chatgpt.cloud_document_understanding")?;
        self.media_service()?
            .analyze(context, input, model, false)
            .await
    }

    #[tool(
        name = "cloud_file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to OpenAI cloud and return a session-owned handle.",
        help = "Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
        translations(
            locale(
                "ar-SA",
                summary = "ارفع ملفًا محليًا واحدًا مسموحًا به إلى سحابة OpenAI وأعِد معرّفًا مملوكًا للجلسة."
            ),
            locale(
                "de-DE",
                summary = "Lädt eine zulässige lokale Datei in die OpenAI-Cloud hoch und gibt ein sitzungseigenes Handle zurück."
            ),
            locale(
                "es-ES",
                summary = "Sube un archivo local permitido a la nube de OpenAI y devuelve un identificador propiedad de la sesión."
            ),
            locale(
                "fr-FR",
                summary = "Téléverse un fichier local autorisé vers le cloud OpenAI et renvoie un identifiant propre à la session."
            ),
            locale(
                "hi-IN",
                summary = "एक अनुमत स्थानीय फ़ाइल OpenAI क्लाउड पर अपलोड करता है और सत्र के स्वामित्व वाला हैंडल लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "許可されたローカルファイルを1件 OpenAI クラウドにアップロードし、セッション所有のハンドルを返します。"
            ),
            locale(
                "ko-KR",
                summary = "허용된 로컬 파일 하나를 OpenAI 클라우드에 업로드하고 세션 소유 핸들을 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia um arquivo local permitido para a nuvem da OpenAI e retorna um identificador pertencente à sessão."
            ),
            locale(
                "zh-CN",
                summary = "将一个获准的本地文件上传到 OpenAI 云端，并返回会话持有的句柄。"
            ),
            locale(
                "zh-TW",
                summary = "將一個獲准的本機檔案上傳至 OpenAI 雲端，並傳回工作階段持有的控制代碼。"
            )
        )
    )]
    async fn file_upload(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &UploadInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?.upload(context, input).await
    }

    #[tool(
        name = "cloud_file_status",
        tags(query, network, mutate),
        summary = "Query the remote status of an owned OpenAI cloud file, not a local path.",
        help = "Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
        translations(
            locale(
                "ar-SA",
                summary = "استعلم عن الحالة البعيدة لملف OpenAI سحابي مملوك، وليس مسارًا محليًا."
            ),
            locale(
                "de-DE",
                summary = "Fragt den entfernten Status einer eigenen OpenAI-Cloud-Datei ab, nicht einen lokalen Pfad."
            ),
            locale(
                "es-ES",
                summary = "Consulta el estado remoto de un archivo propio en la nube de OpenAI, no una ruta local."
            ),
            locale(
                "fr-FR",
                summary = "Interroge l’état distant d’un fichier OpenAI détenu, et non un chemin local."
            ),
            locale(
                "hi-IN",
                summary = "स्वामित्व वाली OpenAI क्लाउड फ़ाइल की दूरस्थ स्थिति पूछता है, स्थानीय पथ की नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "所有する OpenAI クラウドファイルの遠隔状態を問い合わせます。ローカルパスではありません。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 OpenAI 클라우드 파일의 원격 상태를 조회합니다. 로컬 경로가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Consulta o estado remoto de um arquivo próprio na nuvem da OpenAI, não um caminho local."
            ),
            locale(
                "zh-CN",
                summary = "查询对话持有的 OpenAI 云端文件的远端状态，而非本地路径。"
            ),
            locale(
                "zh-TW",
                summary = "查詢工作階段持有的 OpenAI 雲端檔案的遠端狀態，而非本機路徑。"
            )
        )
    )]
    async fn file_status(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &FileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?
            .file_control(context, input, false)
            .await
    }

    #[tool(
        name = "cloud_file_delete",
        tags(mutate, network),
        summary = "Request deletion of an owned file from OpenAI cloud; preserve the local original.",
        help = "Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
        translations(
            locale(
                "ar-SA",
                summary = "اطلب حذف ملف مملوك من سحابة OpenAI؛ مع الحفاظ على الأصل المحلي."
            ),
            locale(
                "de-DE",
                summary = "Fordert das Löschen einer eigenen Datei aus der OpenAI-Cloud an; das lokale Original bleibt erhalten."
            ),
            locale(
                "es-ES",
                summary = "Solicita eliminar un archivo propio de la nube de OpenAI; conserva el original local."
            ),
            locale(
                "fr-FR",
                summary = "Demande la suppression d’un fichier détenu dans le cloud OpenAI ; l’original local est conservé."
            ),
            locale(
                "hi-IN",
                summary = "OpenAI क्लाउड से स्वामित्व वाली फ़ाइल हटाने का अनुरोध करता है; स्थानीय मूल सुरक्षित रहता है।"
            ),
            locale(
                "ja-JP",
                summary = "所有するファイルの OpenAI クラウドからの削除を要求します。ローカルの原本は保持されます。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 파일을 OpenAI 클라우드에서 삭제하도록 요청합니다. 로컬 원본은 유지됩니다."
            ),
            locale(
                "pt-BR",
                summary = "Solicita a exclusão de um arquivo próprio da nuvem da OpenAI; o original local é preservado."
            ),
            locale("zh-CN", summary = "请求从 OpenAI 云端删除持有的文件；保留本地原件。"),
            locale(
                "zh-TW",
                summary = "要求從 OpenAI 雲端刪除持有的檔案；保留本機原始檔。"
            )
        )
    )]
    async fn file_delete(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &FileInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.media_service()?
            .file_control(context, input, true)
            .await
    }
    #[tool(
        name = "cloud_web_search",
        tags(network, interactive, discovery, read_only),
        summary = "Search the web in OpenAI cloud and return sources; not a local browser operation.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Hosted results and response_id are returned for follow-up; this plugin never executes client tool callbacks.",
        translations(
            locale(
                "ar-SA",
                summary = "ابحث في الويب داخل سحابة OpenAI وأعِد المصادر؛ وليس عملية متصفح محلي."
            ),
            locale(
                "de-DE",
                summary = "Durchsucht das Web in der OpenAI-Cloud und liefert Quellen; kein lokaler Browser-Vorgang."
            ),
            locale(
                "es-ES",
                summary = "Busca en la web en la nube de OpenAI y devuelve fuentes; no es una operación del navegador local."
            ),
            locale(
                "fr-FR",
                summary = "Recherche sur le Web dans le cloud OpenAI et renvoie les sources ; ce n’est pas une opération du navigateur local."
            ),
            locale(
                "hi-IN",
                summary = "OpenAI क्लाउड में वेब खोजता है और स्रोत लौटाता है; यह स्थानीय ब्राउज़र कार्रवाई नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "OpenAI クラウドでウェブを検索し、出典を返します。ローカルブラウザーの操作ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "OpenAI 클라우드에서 웹을 검색하고 출처를 반환합니다. 로컬 브라우저 작업이 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Pesquisa na web na nuvem da OpenAI e retorna fontes; não é uma operação do navegador local."
            ),
            locale(
                "zh-CN",
                summary = "在 OpenAI 云端中搜索网络并返回来源；并非本地浏览器操作。"
            ),
            locale(
                "zh-TW",
                summary = "在 OpenAI 雲端中搜尋網路並傳回來源；並非本機瀏覽器操作。"
            )
        )
    )]
    async fn web_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "web_search",
            "ChatGPT web search",
            serde_json::json!({"type":"web_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_file_search",
        tags(network, interactive, discovery, read_only),
        summary = "Search configured OpenAI cloud file stores, not files on this computer.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.",
        translations(
            locale(
                "ar-SA",
                summary = "ابحث في مخازن ملفات OpenAI السحابية المُهيّأة، وليس في ملفات هذا الحاسوب."
            ),
            locale(
                "de-DE",
                summary = "Durchsucht konfigurierte OpenAI-Cloud-Dateispeicher, nicht Dateien auf diesem Computer."
            ),
            locale(
                "es-ES",
                summary = "Busca en los almacenes de archivos configurados de OpenAI, no en archivos de este equipo."
            ),
            locale(
                "fr-FR",
                summary = "Recherche dans les stockages de fichiers OpenAI configurés, pas dans les fichiers de cet ordinateur."
            ),
            locale(
                "hi-IN",
                summary = "कॉन्फ़िगर किए गए OpenAI क्लाउड फ़ाइल स्टोर खोजता है, इस कंप्यूटर की फ़ाइलें नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "設定済みの OpenAI クラウドファイルストアを検索します。このコンピューター上のファイルではありません。"
            ),
            locale(
                "ko-KR",
                summary = "설정된 OpenAI 클라우드 파일 저장소를 검색합니다. 이 컴퓨터의 파일이 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Pesquisa nos armazenamentos de arquivos configurados da OpenAI, não em arquivos deste computador."
            ),
            locale("zh-CN", summary = "搜索已配置的 OpenAI 云端文件存储，而非本机文件。"),
            locale(
                "zh-TW",
                summary = "搜尋已設定的 OpenAI 雲端檔案儲存庫，而非本機檔案。"
            )
        )
    )]
    async fn file_search(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "file_search",
            "ChatGPT file search",
            serde_json::json!({"type":"file_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_code_interpreter",
        stream = code_interpreter_stream,
        tags(network, interactive, read_only),
        summary = "Run Python in an OpenAI cloud container, not the Agena local workspace.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.",
    translations(
        locale("ar-SA", summary = "شغّل Python في حاوية OpenAI سحابية، وليس في مساحة عمل Agena المحلية."),
        locale("de-DE", summary = "Führt Python in einem OpenAI-Cloud-Container aus, nicht im lokalen Agena-Arbeitsbereich."),
        locale("es-ES", summary = "Ejecuta Python en un contenedor de OpenAI, no en el espacio de trabajo local de Agena."),
        locale("fr-FR", summary = "Exécute Python dans un conteneur OpenAI, pas dans l’espace de travail local d’Agena."),
        locale("hi-IN", summary = "Python को OpenAI क्लाउड कंटेनर में चलाता है, Agena के स्थानीय कार्यक्षेत्र में नहीं।"),
        locale("ja-JP", summary = "Python を OpenAI クラウドコンテナで実行します。Agena のローカルワークスペースではありません。"),
        locale("ko-KR", summary = "Python을 OpenAI 클라우드 컨테이너에서 실행합니다. Agena 로컬 작업 공간이 아닙니다."),
        locale("pt-BR", summary = "Executa Python em um contêiner da OpenAI, não no espaço de trabalho local do Agena."),
        locale("zh-CN", summary = "在 OpenAI 云端容器中运行 Python，而非 Agena 本地工作区。"),
        locale("zh-TW", summary = "在 OpenAI 雲端容器中執行 Python，而非 Agena 本機工作區。")
)
)]
    async fn code_interpreter(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "code_interpreter",
            "ChatGPT code interpreter",
            serde_json::json!({"type":"code_interpreter","container":{"type":"auto"}}),
            input,
        )
        .await
    }

    /// Streaming variant: a cloud sandbox run stays silent for a while, so a
    /// reader sees the work before the result lands.
    async fn code_interpreter_stream(
        &self,
        sink: ToolStreamSink,
        input: ChatGptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Running code in the OpenAI cloud container…\n")
            .await;
        self.code_interpreter(input).await
    }

    #[tool(
        name = "cloud_image_generation",
        tags(network, interactive, mutate),
        summary = "Generate images in OpenAI cloud; save returned images as local attachments.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports action, model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size. Returned base64 images are persisted as managed attachments.",
        translations(
            locale(
                "ar-SA",
                summary = "أنشئ صورًا في سحابة OpenAI؛ واحفظ الصور المُعادة كمرفقات محلية."
            ),
            locale(
                "de-DE",
                summary = "Erzeugt Bilder in der OpenAI-Cloud; zurückgegebene Bilder werden als lokale Anhänge gespeichert."
            ),
            locale(
                "es-ES",
                summary = "Genera imágenes en la nube de OpenAI; guarda las imágenes devueltas como adjuntos locales."
            ),
            locale(
                "fr-FR",
                summary = "Génère des images dans le cloud OpenAI ; les images reçues sont enregistrées comme pièces jointes locales."
            ),
            locale(
                "hi-IN",
                summary = "OpenAI क्लाउड में छवियाँ बनाता है; लौटाई गई छवियाँ स्थानीय अनुलग्नक के रूप में सहेजता है।"
            ),
            locale(
                "ja-JP",
                summary = "OpenAI クラウドで画像を生成します。返された画像はローカル添付として保存されます。"
            ),
            locale(
                "ko-KR",
                summary = "OpenAI 클라우드에서 이미지를 생성합니다. 반환된 이미지는 로컬 첨부로 저장됩니다."
            ),
            locale(
                "pt-BR",
                summary = "Gera imagens na nuvem da OpenAI; salva as imagens retornadas como anexos locais."
            ),
            locale(
                "zh-CN",
                summary = "在 OpenAI 云端生成图像；将返回的图像保存为本地附件。"
            ),
            locale(
                "zh-TW",
                summary = "在 OpenAI 雲端生成圖像；將傳回的圖像儲存為本機附件。"
            )
        )
    )]
    async fn image_generation(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "image_generation",
            "ChatGPT image generation",
            serde_json::json!({"type":"image_generation"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_shell",
        stream = shell_stream,
        tags(network, interactive, mutate),
        summary = "Run shell commands in an OpenAI cloud container, never in the local terminal.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Defaults to container_auto. Only container_auto or container_reference with container_id is accepted. Local/custom environments and client callbacks are rejected. Uploaded provider files are separate from Agena local files; there is no local execution fallback.",
    translations(
        locale("ar-SA", summary = "نفّذ أوامر الصدفة في حاوية OpenAI سحابية، وليس في الطرفية المحلية أبدًا."),
        locale("de-DE", summary = "Führt Shell-Befehle in einem OpenAI-Cloud-Container aus, niemals im lokalen Terminal."),
        locale("es-ES", summary = "Ejecuta comandos de shell en un contenedor de OpenAI, nunca en la terminal local."),
        locale("fr-FR", summary = "Exécute des commandes shell dans un conteneur OpenAI, jamais dans le terminal local."),
        locale("hi-IN", summary = "शेल कमांड OpenAI क्लाउड कंटेनर में चलाता है, स्थानीय टर्मिनल में कभी नहीं।"),
        locale("ja-JP", summary = "シェルコマンドを OpenAI クラウドコンテナで実行します。ローカル端末では実行しません。"),
        locale("ko-KR", summary = "셸 명령을 OpenAI 클라우드 컨테이너에서 실행합니다. 로컬 터미널에서는 실행하지 않습니다."),
        locale("pt-BR", summary = "Executa comandos de shell em um contêiner da OpenAI, nunca no terminal local."),
        locale("zh-CN", summary = "在 OpenAI 云端容器中运行 Shell 命令，绝不在本地终端运行。"),
        locale("zh-TW", summary = "在 OpenAI 雲端容器中執行 Shell 命令，絕不在本機終端執行。")
)
)]
    async fn shell(&self, input: ChatGptToolInput) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            "shell",
            "ChatGPT shell",
            serde_json::json!({"type":"shell","environment":{"type":"container_auto"}}),
            input,
        )
        .await
    }

    /// Streaming variant: a cloud shell run stays silent for a while, so a
    /// reader sees the work before the result lands.
    async fn shell_stream(
        &self,
        sink: ToolStreamSink,
        input: ChatGptToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Running a shell in the OpenAI cloud container…\n")
            .await;
        self.shell(input).await
    }

    #[tool(
        name = "cloud_image_edit",
        summary = "Upload permitted images for editing in OpenAI cloud; save the returned image separately.",
        help = "Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to OpenAI; returned images are saved as separate local artifacts. This convenience entry preserves the official image edit endpoint alongside the Responses image_generation tool. Every input and output path is permission checked.",
        tags(mutate),
        translations(
            locale(
                "ar-SA",
                summary = "ارفع صورًا مسموحًا بها للتحرير في سحابة OpenAI؛ واحفظ الصورة المُعادة على حدة."
            ),
            locale(
                "de-DE",
                summary = "Lädt zulässige Bilder zur Bearbeitung in die OpenAI-Cloud hoch; das zurückgegebene Bild wird separat gespeichert."
            ),
            locale(
                "es-ES",
                summary = "Sube imágenes permitidas para editarlas en la nube de OpenAI; guarda la imagen devuelta por separado."
            ),
            locale(
                "fr-FR",
                summary = "Téléverse des images autorisées pour édition dans le cloud OpenAI ; l’image renvoyée est enregistrée séparément."
            ),
            locale(
                "hi-IN",
                summary = "संपादन के लिए अनुमत छवियाँ OpenAI क्लाउड पर अपलोड करता है; लौटाई गई छवि अलग से सहेजता है।"
            ),
            locale(
                "ja-JP",
                summary = "編集用に許可された画像を OpenAI クラウドへアップロードします。返された画像は別途保存されます。"
            ),
            locale(
                "ko-KR",
                summary = "편집을 위해 허용된 이미지를 OpenAI 클라우드에 업로드합니다. 반환된 이미지는 따로 저장합니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia imagens permitidas para edição na nuvem da OpenAI; salva a imagem retornada separadamente."
            ),
            locale(
                "zh-CN",
                summary = "将获准的图像上传到 OpenAI 云端进行编辑；返回的图像单独保存。"
            ),
            locale(
                "zh-TW",
                summary = "將獲准的圖像上傳至 OpenAI 雲端進行編輯；傳回的圖像另行儲存。"
            )
        )
    )]
    async fn image_edit(&self, input: ChatGptImageEditInput) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.options, "options")?;
        let model = self.image_model(input.model, "chatgpt.cloud_image_edit")?;
        let url = endpoint(self.config()?.base_url.as_str(), "images/edits")?;
        super::official_service::validate_provider_endpoint(self.host()?, url.as_str()).await?;
        let mut form = reqwest::multipart::Form::new()
            .text("model", model.clone())
            .text("prompt", input.prompt)
            .text("output_format", "png");
        for (key, value) in input.options {
            if matches!(key.as_str(), "model" | "prompt" | "image") {
                return Err(PluginError::invalid_params(format!(
                    "options.{key} is protected"
                )));
            }
            form = form.text(
                key,
                match value {
                    serde_json::Value::String(value) => value,
                    value => value.to_string(),
                },
            );
        }
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes = read_image_input_bounded(&path, &mut image_input_bytes, "OpenAI").await?;
            let filename = path
                .file_name()
                .and_then(|value| value.to_str())
                .unwrap_or("image.png")
                .to_owned();
            let mime = match path
                .extension()
                .and_then(|value| value.to_str())
                .map(str::to_ascii_lowercase)
                .as_deref()
            {
                Some("jpg" | "jpeg") => "image/jpeg",
                Some("webp") => "image/webp",
                Some("gif") => "image/gif",
                _ => "image/png",
            };
            let part = reqwest::multipart::Part::bytes(bytes)
                .file_name(filename)
                .mime_str(mime)
                .map_err(|error| PluginError::internal(format!("invalid image MIME: {error}")))?;
            form = form.part("image[]", part);
        }
        let response = crate::PROVIDER_HTTP_CLIENT
            .post(url)
            .timeout(std::time::Duration::from_secs(
                self.config()?.timeout_secs.max(1),
            ))
            .bearer_auth(env_secret(
                self.config()?.api_key_env.as_str(),
                "ChatGPT/OpenAI",
            )?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| PluginError::internal(format!("OpenAI image edit failed: {error}")))?;
        let (status, request_id, value) =
            read_json_response_bounded(response, "OpenAI", "image edit").await?;
        if !status.is_success() {
            return Err(PluginError::internal(format!(
                "OpenAI image edit failed (HTTP {status}): {value}"
            )));
        }
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "chatgpt",
            "image_edit",
            model.as_str(),
            "ChatGPT image edit",
            ProviderUsageKind::OpenAiImage,
            ProviderHttpResponse { value, request_id },
        )
        .await
    }
}
