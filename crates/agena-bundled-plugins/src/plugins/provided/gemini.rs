//! Google cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.

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
use base64::Engine as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::cloud_media::{AnalyzeInput, FileInput, Service, UploadInput};
use super::official_service::{
    ProviderUsageKind, append_prompt_to_items, configured_model, endpoint, env_secret,
    merge_object_options, post_json, provider_output, read_image_input_bounded, resolve_local_path,
};
use agena_plugin_host::sdk::ToolInvokeContext;

pub(crate) const GEMINI_PLUGIN_ID: &str = "agena.gemini";

pub(crate) struct GeminiToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<GeminiToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct GeminiToolsConfig {
    base_url: String,
    api_key_env: String,
    model: Option<String>,
    image_model: Option<String>,
    timeout_secs: u64,
    stable_system_instruction: Option<String>,
}

impl Default for GeminiToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://generativelanguage.googleapis.com/v1beta".to_owned(),
            api_key_env: "GEMINI_API_KEY".to_owned(),
            model: None,
            image_model: None,
            timeout_secs: 180,
            stable_system_instruction: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_system_instruction"),
    max_chars("prompt", 64000),
    max_chars("stable_system_instruction", 256000)
)]
#[serde(deny_unknown_fields)]
struct GeminiToolInput {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable prefix used to improve Gemini implicit cache reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_system_instruction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_interaction_id: Option<String>,
    /// Official Interactions message/history steps for hosted operations. Client function callbacks are not accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_steps: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model"),
    non_empty("prompt"),
    max_chars("prompt", 64000)
)]
#[serde(deny_unknown_fields)]
struct GeminiImageGenerateInput {
    prompt: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    generation_config: BTreeMap<String, serde_json::Value>,
    /// Existing Gemini cachedContents resource name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "images[]"),
    non_empty("prompt", "images[]"),
    min_items("images", 1),
    max_items("images", 16)
)]
#[serde(deny_unknown_fields)]
struct GeminiImageEditInput {
    prompt: String,
    images: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    generation_config: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
}

impl GeminiToolsPlugin {
    fn media_service(&self) -> SdkResult<Service<'_>> {
        Ok(Service {
            provider: "gemini",
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
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn config(&self) -> SdkResult<&GeminiToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("Gemini tools plugin invoked before init"))
    }
    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["GEMINI_MODEL", "GOOGLE_GENAI_MODEL"],
            tool,
        )
    }
    fn image_model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.image_model.as_deref(),
            &["GEMINI_IMAGE_MODEL", "GOOGLE_GENAI_IMAGE_MODEL"],
            tool,
        )
    }

    async fn interactions_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        input: GeminiToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.tool_options, "tool_options")?;
        super::official_service::hosted::validate_options(
            &input.request_options,
            "request_options",
        )?;
        super::official_service::hosted::validate_history(
            "gemini",
            &serde_json::json!(&input.input_steps),
        )?;
        let model = self.model(input.model, format!("gemini.cloud_{tool_name}").as_str())?;
        let declaration =
            merge_object_options(declaration, &input.tool_options, &["type"], "tool_options")?;
        super::official_service::hosted::validate_declaration("gemini", tool_name, &declaration)?;
        let provider_input = append_prompt_to_items(input.input_steps, input.prompt, false)?;
        let mut base = serde_json::json!({
            "model": model.clone(),
            "input": provider_input,
            "tools": [declaration],
            "stream": false
        });
        if let Some(system_instruction) = input
            .stable_system_instruction
            .or_else(|| self.config().ok()?.stable_system_instruction.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["system_instruction"] = serde_json::Value::String(system_instruction);
        }
        if let Some(previous) = input.previous_interaction_id {
            base["previous_interaction_id"] = serde_json::Value::String(previous);
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "input",
                "tools",
                "stream",
                "previous_interaction_id",
                "system_instruction",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "interactions")?;
        let headers = BTreeMap::from([
            (
                "x-goog-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Gemini")?,
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            self.config()?.timeout_secs,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "gemini",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiInteractions,
            response,
        )
        .await
    }

    async fn generate_content(
        &self,
        tool_name: &str,
        title: &str,
        model: String,
        parts: Vec<serde_json::Value>,
        mut generation_config: BTreeMap<String, serde_json::Value>,
        cached_content: Option<String>,
        request_options: BTreeMap<String, serde_json::Value>,
    ) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&request_options, "request_options")?;
        super::official_service::hosted::validate_options(&generation_config, "generation_config")?;
        generation_config
            .entry("responseModalities".to_owned())
            .or_insert_with(|| serde_json::json!(["TEXT", "IMAGE"]));
        let model_path = if model.starts_with("models/") {
            model.clone()
        } else {
            format!("models/{model}")
        };
        let url = endpoint(
            self.config()?.base_url.as_str(),
            format!("{model_path}:generateContent").as_str(),
        )?;
        let headers = BTreeMap::from([
            (
                "x-goog-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Gemini")?,
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let mut base = serde_json::json!({
            "contents": [{"role":"user","parts":parts}],
            "generationConfig": generation_config
        });
        if let Some(cached_content) = cached_content
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["cachedContent"] = serde_json::Value::String(cached_content);
        }
        let body = merge_object_options(
            base,
            &request_options,
            &["contents", "generationConfig", "cachedContent"],
            "request_options",
        )?;
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            self.config()?.timeout_secs,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "gemini",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiGenerateContent,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "gemini",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Google cloud search, computation and image capabilities. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "使用 Google 云端搜索、计算和图像能力。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "使用 Google 雲端搜尋、運算與影像功能。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "Google のクラウド検索・計算・画像機能を利用します。入力は端末外へ送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "Google 클라우드 검색, 계산, 이미지 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Utiliser les fonctions de recherche, de calcul et d’image dans le cloud Google. Les données sont envoyées hors de cet ordinateur, sans repli local."),
        locale("de-DE", summary = "Google-Cloudfunktionen für Suche, Berechnungen und Bilder nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Usa las funciones de búsqueda, cálculo e imagen en la nube de Google. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "Google क्लाउड की खोज, गणना और छवि सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "استخدم إمكانات البحث والحوسبة والصور في سحابة Google. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Use os recursos de busca, computação e imagem na nuvem do Google. As entradas saem deste computador; não há execução local alternativa.")
    ),
    settings = GeminiToolsConfig,
    settings_default = default
)]
impl GeminiToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: GeminiToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid Gemini tools plugin config",
            )?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Gemini tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        name = "cloud_image_understanding",
        tags(query, network, mutate),
        summary = "Send explicit images to Google cloud for understanding; not local file viewing.",
        help = "Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل صورًا محددة صراحةً إلى سحابة Google لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene Bilder zur Analyse an die Google-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía imágenes explícitas a la nube de Google para comprenderlas; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des images explicites au cloud Google pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दी गई छवियाँ Google क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した画像を Google クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 이미지를 Google 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia imagens explícitas para a nuvem do Google para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的图像发送到 Google 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的圖像傳送至 Google 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn image_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "gemini.cloud_image_understanding")?;
        self.media_service()?
            .analyze(context, input, model, true)
            .await
    }

    #[tool(
        name = "cloud_document_understanding",
        tags(query, network, mutate),
        summary = "Send explicit PDF/text documents to Google cloud for understanding; not local file viewing.",
        help = "Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل مستندات PDF/نصية محددة صراحةً إلى سحابة Google لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene PDF-/Textdokumente zur Analyse an die Google-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía documentos PDF/texto explícitos a la nube de Google para comprenderlos; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des documents PDF/textuels explicites au cloud Google pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दिए गए PDF/टेक्स्ट दस्तावेज़ Google क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した PDF／テキスト文書を Google クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 PDF/텍스트 문서를 Google 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia documentos PDF/texto explícitos para a nuvem do Google para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的 PDF/文本文档发送到 Google 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的 PDF／文字文件傳送至 Google 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn document_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "gemini.cloud_document_understanding")?;
        self.media_service()?
            .analyze(context, input, model, false)
            .await
    }

    #[tool(
        name = "cloud_file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to Google cloud and return a session-owned handle.",
        help = "Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
        translations(
            locale(
                "ar-SA",
                summary = "ارفع ملفًا محليًا واحدًا مسموحًا به إلى سحابة Google وأعِد معرّفًا مملوكًا للجلسة."
            ),
            locale(
                "de-DE",
                summary = "Lädt eine zulässige lokale Datei in die Google-Cloud hoch und gibt ein sitzungseigenes Handle zurück."
            ),
            locale(
                "es-ES",
                summary = "Sube un archivo local permitido a la nube de Google y devuelve un identificador propiedad de la sesión."
            ),
            locale(
                "fr-FR",
                summary = "Téléverse un fichier local autorisé vers le cloud Google et renvoie un identifiant propre à la session."
            ),
            locale(
                "hi-IN",
                summary = "एक अनुमत स्थानीय फ़ाइल Google क्लाउड पर अपलोड करता है और सत्र के स्वामित्व वाला हैंडल लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "許可されたローカルファイルを1件 Google クラウドにアップロードし、セッション所有のハンドルを返します。"
            ),
            locale(
                "ko-KR",
                summary = "허용된 로컬 파일 하나를 Google 클라우드에 업로드하고 세션 소유 핸들을 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia um arquivo local permitido para a nuvem do Google e retorna um identificador pertencente à sessão."
            ),
            locale(
                "zh-CN",
                summary = "将一个获准的本地文件上传到 Google 云端，并返回会话持有的句柄。"
            ),
            locale(
                "zh-TW",
                summary = "將一個獲准的本機檔案上傳至 Google 雲端，並傳回工作階段持有的控制代碼。"
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
        summary = "Query the remote status of an owned Google cloud file, not a local path.",
        help = "Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
        translations(
            locale(
                "ar-SA",
                summary = "استعلم عن الحالة البعيدة لملف Google سحابي مملوك، وليس مسارًا محليًا."
            ),
            locale(
                "de-DE",
                summary = "Fragt den entfernten Status einer eigenen Google-Cloud-Datei ab, nicht einen lokalen Pfad."
            ),
            locale(
                "es-ES",
                summary = "Consulta el estado remoto de un archivo propio en la nube de Google, no una ruta local."
            ),
            locale(
                "fr-FR",
                summary = "Interroge l’état distant d’un fichier Google détenu, et non un chemin local."
            ),
            locale(
                "hi-IN",
                summary = "स्वामित्व वाली Google क्लाउड फ़ाइल की दूरस्थ स्थिति पूछता है, स्थानीय पथ की नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "所有する Google クラウドファイルの遠隔状態を問い合わせます。ローカルパスではありません。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 Google 클라우드 파일의 원격 상태를 조회합니다. 로컬 경로가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Consulta o estado remoto de um arquivo próprio na nuvem do Google, não um caminho local."
            ),
            locale(
                "zh-CN",
                summary = "查询对话持有的 Google 云端文件的远端状态，而非本地路径。"
            ),
            locale(
                "zh-TW",
                summary = "查詢工作階段持有的 Google 雲端檔案的遠端狀態，而非本機路徑。"
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
        summary = "Request deletion of an owned file from Google cloud; preserve the local original.",
        help = "Runs in Google cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
        translations(
            locale(
                "ar-SA",
                summary = "اطلب حذف ملف مملوك من سحابة Google؛ مع الحفاظ على الأصل المحلي."
            ),
            locale(
                "de-DE",
                summary = "Fordert das Löschen einer eigenen Datei aus der Google-Cloud an; das lokale Original bleibt erhalten."
            ),
            locale(
                "es-ES",
                summary = "Solicita eliminar un archivo propio de la nube de Google; conserva el original local."
            ),
            locale(
                "fr-FR",
                summary = "Demande la suppression d’un fichier détenu dans le cloud Google ; l’original local est conservé."
            ),
            locale(
                "hi-IN",
                summary = "Google क्लाउड से स्वामित्व वाली फ़ाइल हटाने का अनुरोध करता है; स्थानीय मूल सुरक्षित रहता है।"
            ),
            locale(
                "ja-JP",
                summary = "所有するファイルの Google クラウドからの削除を要求します。ローカルの原本は保持されます。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 파일을 Google 클라우드에서 삭제하도록 요청합니다. 로컬 원본은 유지됩니다."
            ),
            locale(
                "pt-BR",
                summary = "Solicita a exclusão de um arquivo próprio da nuvem do Google; o original local é preservado."
            ),
            locale("zh-CN", summary = "请求从 Google 云端删除持有的文件；保留本地原件。"),
            locale(
                "zh-TW",
                summary = "要求從 Google 雲端刪除持有的檔案；保留本機原始檔。"
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
        name = "cloud_code_execution",
        stream = code_execution_stream,
        tags(network, interactive, read_only),
        summary = "Execute code in Google cloud infrastructure, not on this computer.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses the official Interactions code_execution declaration. Computation executes on Google infrastructure; no returned function call is executed by Agena.",
    translations(
        locale("ar-SA", summary = "نفّذ تعليمة برمجية في البنية السحابية لـ Google، وليس على هذا الحاسوب."),
        locale("de-DE", summary = "Führt Code in der Google-Cloud-Infrastruktur aus, nicht auf diesem Computer."),
        locale("es-ES", summary = "Ejecuta código en la infraestructura de Google, no en este equipo."),
        locale("fr-FR", summary = "Exécute du code dans l’infrastructure Google, pas sur cet ordinateur."),
        locale("hi-IN", summary = "Google क्लाउड अधोसंरचना में कोड चलाता है, इस कंप्यूटर पर नहीं।"),
        locale("ja-JP", summary = "Google クラウド基盤でコードを実行します。このコンピューター上ではありません。"),
        locale("ko-KR", summary = "Google 클라우드 인프라에서 코드를 실행합니다. 이 컴퓨터가 아닙니다."),
        locale("pt-BR", summary = "Executa código na infraestrutura da nuvem do Google, não neste computador."),
        locale("zh-CN", summary = "在 Google 云端基础设施中执行代码，而非在本机执行。"),
        locale("zh-TW", summary = "在 Google 雲端基礎架構中執行程式碼，而非在本機執行。")
)
)]
    async fn code_execution(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "code_execution",
            "Gemini code execution",
            serde_json::json!({"type":"code_execution"}),
            input,
        )
        .await
    }

    /// Streaming variant: a cloud sandbox run stays silent for a while, so a
    /// reader sees the work before the result lands.
    async fn code_execution_stream(
        &self,
        sink: ToolStreamSink,
        input: GeminiToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Running code in Google cloud…\n").await;
        self.code_execution(input).await
    }

    #[tool(
        name = "cloud_url_context",
        tags(network, interactive, discovery, read_only),
        summary = "Retrieve and ground URL content in Google cloud; no local-file access.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses the official url_context tool. Put URLs in the prompt or official request fields.",
        translations(
            locale(
                "ar-SA",
                summary = "اجلب محتوى الروابط وأسنده في سحابة Google؛ دون الوصول إلى الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Ruft URL-Inhalte in der Google-Cloud ab und belegt sie; kein Zugriff auf lokale Dateien."
            ),
            locale(
                "es-ES",
                summary = "Obtiene y fundamenta el contenido de URL en la nube de Google; sin acceso a archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Récupère et étaye le contenu d’URL dans le cloud Google ; aucun accès aux fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "URL सामग्री Google क्लाउड में प्राप्त और आधारित करता है; स्थानीय फ़ाइलों तक पहुँच नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "URL の内容を Google クラウドで取得して根拠付けます。ローカルファイルへのアクセスはありません。"
            ),
            locale(
                "ko-KR",
                summary = "URL 콘텐츠를 Google 클라우드에서 가져와 근거로 삼습니다. 로컬 파일 접근은 없습니다."
            ),
            locale(
                "pt-BR",
                summary = "Obtém e fundamenta conteúdo de URL na nuvem do Google; sem acesso a arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "在 Google 云端获取并为 URL 内容提供依据；不访问本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "在 Google 雲端取得並為 URL 內容提供依據；不存取本機檔案。"
            )
        )
    )]
    async fn url_context(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "url_context",
            "Gemini URL context",
            serde_json::json!({"type":"url_context"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_google_search",
        tags(network, interactive, discovery, read_only),
        summary = "Search Google and ground answers in Google cloud, not the local browser.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options.search_types accepts web_search, image_search, and enterprise_web_search.",
        translations(
            locale(
                "ar-SA",
                summary = "ابحث في Google وأسند الإجابات في سحابة Google، وليس في المتصفح المحلي."
            ),
            locale(
                "de-DE",
                summary = "Durchsucht Google und belegt Antworten in der Google-Cloud, nicht im lokalen Browser."
            ),
            locale(
                "es-ES",
                summary = "Busca en Google y fundamenta las respuestas en la nube de Google, no en el navegador local."
            ),
            locale(
                "fr-FR",
                summary = "Recherche sur Google et étaye les réponses dans le cloud Google, pas dans le navigateur local."
            ),
            locale(
                "hi-IN",
                summary = "Google खोजता है और उत्तर Google क्लाउड में आधारित करता है, स्थानीय ब्राउज़र में नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "Google を検索し、回答を Google クラウドで根拠付けます。ローカルブラウザーではありません。"
            ),
            locale(
                "ko-KR",
                summary = "Google을 검색하고 답변을 Google 클라우드에서 근거로 삼습니다. 로컬 브라우저가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Pesquisa no Google e fundamenta respostas na nuvem do Google, não no navegador local."
            ),
            locale(
                "zh-CN",
                summary = "在 Google 云端中搜索 Google 并为答案提供依据，而非本地浏览器。"
            ),
            locale(
                "zh-TW",
                summary = "在 Google 雲端中搜尋 Google 並為答案提供依據，而非本機瀏覽器。"
            )
        )
    )]
    async fn google_search(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "google_search",
            "Gemini Google Search",
            serde_json::json!({"type":"google_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_file_search",
        tags(network, interactive, discovery, read_only),
        summary = "Search configured Google cloud file stores, not files on this computer.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. tool_options supports file_search_store_names, metadata_filter, and top_k.",
        translations(
            locale(
                "ar-SA",
                summary = "ابحث في مخازن ملفات Google السحابية المُهيّأة، وليس في ملفات هذا الحاسوب."
            ),
            locale(
                "de-DE",
                summary = "Durchsucht konfigurierte Google-Cloud-Dateispeicher, nicht Dateien auf diesem Computer."
            ),
            locale(
                "es-ES",
                summary = "Busca en los almacenes de archivos configurados de Google, no en archivos de este equipo."
            ),
            locale(
                "fr-FR",
                summary = "Recherche dans les stockages de fichiers Google configurés, pas dans les fichiers de cet ordinateur."
            ),
            locale(
                "hi-IN",
                summary = "कॉन्फ़िगर किए गए Google क्लाउड फ़ाइल स्टोर खोजता है, इस कंप्यूटर की फ़ाइलें नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "設定済みの Google クラウドファイルストアを検索します。このコンピューター上のファイルではありません。"
            ),
            locale(
                "ko-KR",
                summary = "설정된 Google 클라우드 파일 저장소를 검색합니다. 이 컴퓨터의 파일이 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Pesquisa nos armazenamentos de arquivos configurados do Google, não em arquivos deste computador."
            ),
            locale("zh-CN", summary = "搜索已配置的 Google 云端文件存储，而非本机文件。"),
            locale(
                "zh-TW",
                summary = "搜尋已設定的 Google 雲端檔案儲存庫，而非本機檔案。"
            )
        )
    )]
    async fn file_search(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "file_search",
            "Gemini file search",
            serde_json::json!({"type":"file_search"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_google_maps",
        tags(network, interactive, discovery, read_only),
        summary = "Query Google Maps data in Google cloud and return grounding sources.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports enable_widget, latitude, and longitude.",
        translations(
            locale(
                "ar-SA",
                summary = "استعلم عن بيانات خرائط Google في سحابة Google وأعِد مصادر الإسناد."
            ),
            locale(
                "de-DE",
                summary = "Fragt Google-Maps-Daten in der Google-Cloud ab und liefert Belegquellen."
            ),
            locale(
                "es-ES",
                summary = "Consulta datos de Google Maps en la nube de Google y devuelve fuentes de fundamento."
            ),
            locale(
                "fr-FR",
                summary = "Interroge les données Google Maps dans le cloud Google et renvoie les sources d’appui."
            ),
            locale(
                "hi-IN",
                summary = "Google क्लाउड में Google Maps डेटा पूछता है और आधार स्रोत लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "Google クラウドで Google マップのデータを照会し、根拠となる出典を返します。"
            ),
            locale(
                "ko-KR",
                summary = "Google 클라우드에서 Google 지도 데이터를 조회하고 근거 출처를 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Consulta dados do Google Maps na nuvem do Google e retorna fontes de fundamentação."
            ),
            locale(
                "zh-CN",
                summary = "在 Google 云端中查询 Google 地图数据并返回依据来源。"
            ),
            locale(
                "zh-TW",
                summary = "在 Google 雲端中查詢 Google 地圖資料並傳回依據來源。"
            )
        )
    )]
    async fn google_maps(&self, input: GeminiToolInput) -> SdkResult<ToolInvokeOutput> {
        self.interactions_tool(
            "google_maps",
            "Gemini Google Maps",
            serde_json::json!({"type":"google_maps"}),
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_image_generation",
        tags(network, interactive, mutate),
        summary = "Generate images in Google cloud; save returned images as local attachments.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses generateContent with responseModalities TEXT and IMAGE. Configure GEMINI_IMAGE_MODEL or input.model. Inline image data is persisted as managed attachments.",
        translations(
            locale(
                "ar-SA",
                summary = "أنشئ صورًا في سحابة Google؛ واحفظ الصور المُعادة كمرفقات محلية."
            ),
            locale(
                "de-DE",
                summary = "Erzeugt Bilder in der Google-Cloud; zurückgegebene Bilder werden als lokale Anhänge gespeichert."
            ),
            locale(
                "es-ES",
                summary = "Genera imágenes en la nube de Google; guarda las imágenes devueltas como adjuntos locales."
            ),
            locale(
                "fr-FR",
                summary = "Génère des images dans le cloud Google ; les images reçues sont enregistrées comme pièces jointes locales."
            ),
            locale(
                "hi-IN",
                summary = "Google क्लाउड में छवियाँ बनाता है; लौटाई गई छवियाँ स्थानीय अनुलग्नक के रूप में सहेजता है।"
            ),
            locale(
                "ja-JP",
                summary = "Google クラウドで画像を生成します。返された画像はローカル添付として保存されます。"
            ),
            locale(
                "ko-KR",
                summary = "Google 클라우드에서 이미지를 생성합니다. 반환된 이미지는 로컬 첨부로 저장됩니다."
            ),
            locale(
                "pt-BR",
                summary = "Gera imagens na nuvem do Google; salva as imagens retornadas como anexos locais."
            ),
            locale(
                "zh-CN",
                summary = "在 Google 云端生成图像；将返回的图像保存为本地附件。"
            ),
            locale(
                "zh-TW",
                summary = "在 Google 雲端生成圖像；將傳回的圖像儲存為本機附件。"
            )
        )
    )]
    async fn image_generation(
        &self,
        input: GeminiImageGenerateInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.image_model(input.model, "gemini.cloud_image_generation")?;
        self.generate_content(
            "image_generation",
            "Gemini image generation",
            model,
            vec![serde_json::json!({"text":input.prompt})],
            input.generation_config,
            input.cached_content,
            input.request_options,
        )
        .await
    }

    #[tool(
        name = "cloud_image_edit",
        summary = "Upload permitted images for editing in Google cloud; save the returned image separately.",
        help = "Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to Google; returned images are saved as separate local artifacts. Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.",
        tags(mutate),
        translations(
            locale(
                "ar-SA",
                summary = "ارفع صورًا مسموحًا بها للتحرير في سحابة Google؛ واحفظ الصورة المُعادة على حدة."
            ),
            locale(
                "de-DE",
                summary = "Lädt zulässige Bilder zur Bearbeitung in die Google-Cloud hoch; das zurückgegebene Bild wird separat gespeichert."
            ),
            locale(
                "es-ES",
                summary = "Sube imágenes permitidas para editarlas en la nube de Google; guarda la imagen devuelta por separado."
            ),
            locale(
                "fr-FR",
                summary = "Téléverse des images autorisées pour édition dans le cloud Google ; l’image renvoyée est enregistrée séparément."
            ),
            locale(
                "hi-IN",
                summary = "संपादन के लिए अनुमत छवियाँ Google क्लाउड पर अपलोड करता है; लौटाई गई छवि अलग से सहेजता है।"
            ),
            locale(
                "ja-JP",
                summary = "編集用に許可された画像を Google クラウドへアップロードします。返された画像は別途保存されます。"
            ),
            locale(
                "ko-KR",
                summary = "편집을 위해 허용된 이미지를 Google 클라우드에 업로드합니다. 반환된 이미지는 따로 저장합니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia imagens permitidas para edição na nuvem do Google; salva a imagem retornada separadamente."
            ),
            locale(
                "zh-CN",
                summary = "将获准的图像上传到 Google 云端进行编辑；返回的图像单独保存。"
            ),
            locale(
                "zh-TW",
                summary = "將獲准的圖像上傳至 Google 雲端進行編輯；傳回的圖像另行儲存。"
            )
        )
    )]
    async fn image_edit(&self, input: GeminiImageEditInput) -> SdkResult<ToolInvokeOutput> {
        let model = self.image_model(input.model, "gemini.cloud_image_edit")?;
        let mut parts = Vec::new();
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes = read_image_input_bounded(&path, &mut image_input_bytes, "Gemini").await?;
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
            parts.push(serde_json::json!({"inlineData":{"mimeType":mime,"data":base64::engine::general_purpose::STANDARD.encode(bytes)}}));
        }
        parts.push(serde_json::json!({"text":input.prompt}));
        self.generate_content(
            "image_edit",
            "Gemini image edit",
            model,
            parts,
            input.generation_config,
            input.cached_content,
            input.request_options,
        )
        .await
    }
}
