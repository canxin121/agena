//! Anthropic Claude beta/server tools exposed as ordinary Agena tools.

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
    ProviderUsageKind, configured_model, dedup_strings, endpoint, env_secret, merge_object_options,
    post_json, provider_output,
};
use agena_plugin_host::sdk::ToolInvokeContext;

pub(crate) const CLAUDE_PLUGIN_ID: &str = "agena.claude";

pub(crate) struct ClaudeToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<ClaudeToolsConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct ClaudeToolsConfig {
    base_url: String,
    api_key_env: String,
    anthropic_version: String,
    model: Option<String>,
    max_tokens: u32,
    beta_headers: Vec<String>,
    timeout_secs: u64,
    cache_ttl: ClaudeCacheTtl,
    stable_system: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, Default)]
#[serde(rename_all = "snake_case")]
enum ClaudeCacheTtl {
    Disabled,
    #[default]
    FiveMinutes,
    OneHour,
}

impl Default for ClaudeToolsConfig {
    fn default() -> Self {
        Self {
            base_url: "https://api.anthropic.com".to_owned(),
            api_key_env: "ANTHROPIC_API_KEY".to_owned(),
            anthropic_version: "2023-06-01".to_owned(),
            model: None,
            max_tokens: 4096,
            beta_headers: Vec::new(),
            timeout_secs: 180,
            cache_ttl: ClaudeCacheTtl::FiveMinutes,
            stable_system: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[input(
    trim("prompt", "model", "stable_system"),
    max_chars("prompt", 64000),
    max_chars("stable_system", 256000)
)]
#[serde(deny_unknown_fields)]
struct ClaudeToolInput {
    /// New user instruction. Optional when messages continue hosted server execution.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Stable system prefix placed before dynamic messages for cache reuse.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cache_ttl: Option<ClaudeCacheTtl>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    /// Anthropic message history for hosted results or pause_turn resumption. Client tool_use/tool_result callbacks are not accepted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    messages: Vec<serde_json::Value>,
    /// Additional official Anthropic beta feature headers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    beta_headers: Vec<String>,
}

impl ClaudeToolsPlugin {
    fn media_service(&self) -> SdkResult<Service<'_>> {
        Ok(Service {
            provider: "claude",
            root: self.workspace_root()?,
            host: self.host()?,
            base_url: &self.config()?.base_url,
            key_env: &self.config()?.api_key_env,
            timeout_secs: self.config()?.timeout_secs,
            anthropic_version: self.config()?.anthropic_version.as_str(),
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
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn workspace_root(&self) -> SdkResult<&Path> {
        self.workspace_root
            .get()
            .map(PathBuf::as_path)
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn config(&self) -> SdkResult<&ClaudeToolsConfig> {
        self.config
            .get()
            .ok_or_else(|| PluginError::internal("Claude tools plugin invoked before init"))
    }
    fn model(&self, requested: Option<String>, tool: &str) -> SdkResult<String> {
        configured_model(
            requested,
            self.config()?.model.as_deref(),
            &["CLAUDE_MODEL", "ANTHROPIC_MODEL"],
            tool,
        )
    }

    async fn messages_tool(
        &self,
        tool_name: &str,
        title: &str,
        declaration: serde_json::Value,
        default_betas: &[&str],
        input: ClaudeToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.tool_options, "tool_options")?;
        super::official_service::hosted::validate_options(
            &input.request_options,
            "request_options",
        )?;
        super::official_service::hosted::validate_history(
            "claude",
            &serde_json::json!(&input.messages),
        )?;
        let model = self.model(input.model, format!("claude.cloud_{tool_name}").as_str())?;
        let declaration = merge_object_options(
            declaration,
            &input.tool_options,
            &["type", "name"],
            "tool_options",
        )?;
        super::official_service::hosted::validate_declaration("claude", tool_name, &declaration)?;
        let mut messages = input.messages;
        if let Some(prompt) = input
            .prompt
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            messages.push(serde_json::json!({"role":"user","content":prompt}));
        }
        if messages.is_empty() {
            return Err(PluginError::invalid_params(
                "an initial Claude provider-tool request requires prompt or messages",
            ));
        }
        let mut base = serde_json::json!({
            "model": model.clone(),
            "max_tokens": input.max_tokens.unwrap_or(self.config()?.max_tokens),
            "messages": messages,
            "tools": [declaration],
            "stream": false
        });
        if let Some(system) = input
            .stable_system
            .or_else(|| self.config().ok()?.stable_system.clone())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
        {
            base["system"] = serde_json::Value::String(system);
        }
        match input.cache_ttl.unwrap_or(self.config()?.cache_ttl) {
            ClaudeCacheTtl::Disabled => {}
            ClaudeCacheTtl::FiveMinutes => {
                base["cache_control"] = serde_json::json!({
                    "type": "ephemeral",
                    "ttl": "5m"
                });
            }
            ClaudeCacheTtl::OneHour => {
                base["cache_control"] = serde_json::json!({
                    "type": "ephemeral",
                    "ttl": "1h"
                });
            }
        }
        let body = merge_object_options(
            base,
            &input.request_options,
            &[
                "model",
                "max_tokens",
                "messages",
                "tools",
                "stream",
                "system",
                "cache_control",
            ],
            "request_options",
        )?;
        let url = endpoint(self.config()?.base_url.as_str(), "v1/messages")?;
        let mut headers = BTreeMap::from([
            (
                "x-api-key".to_owned(),
                env_secret(self.config()?.api_key_env.as_str(), "Claude/Anthropic")?,
            ),
            (
                "anthropic-version".to_owned(),
                self.config()?.anthropic_version.clone(),
            ),
            ("content-type".to_owned(), "application/json".to_owned()),
        ]);
        let betas = dedup_strings(
            self.config()?
                .beta_headers
                .iter()
                .cloned()
                .chain(default_betas.iter().map(|value| value.to_string()))
                .chain(input.beta_headers),
        );
        if !betas.is_empty() {
            headers.insert("anthropic-beta".to_owned(), betas.join(","));
        }
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            self.config()?.timeout_secs,
            "claude",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "claude",
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::AnthropicMessages,
            response,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "claude",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Anthropic cloud search, fetch, computation and advisor capabilities. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "使用 Anthropic 云端搜索、网页抓取、计算和顾问模型。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "使用 Anthropic 雲端搜尋、網頁擷取、運算與顧問模型。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "Anthropic のクラウド検索・取得・計算・アドバイザー機能を利用します。入力は端末外へ送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "Anthropic 클라우드 검색, 가져오기, 계산 및 어드바이저 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Utiliser la recherche, la récupération, le calcul et le modèle conseiller dans le cloud Anthropic. Les données quittent cet ordinateur, sans repli local."),
        locale("de-DE", summary = "Anthropic-Cloudfunktionen für Suche, Abruf, Berechnungen und Beratung nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Usa las funciones de búsqueda, obtención, cálculo y asesoramiento en la nube de Anthropic. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "Anthropic क्लाउड की खोज, फ़ेच, गणना और सलाहकार सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "استخدم البحث والجلب والحوسبة ونموذج الاستشارة في سحابة Anthropic. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Use os recursos de busca, obtenção, computação e consultoria na nuvem da Anthropic. As entradas saem deste computador; não há execução local alternativa.")
    ),
    settings = ClaudeToolsConfig,
    settings_default = default
)]
impl ClaudeToolsPlugin {
    #[hook(init)]
    async fn init(&self, ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        let config: ClaudeToolsConfig =
            agena_plugin_host::sdk::macro_support::parse_defaulted_settings(
                ctx.settings,
                "invalid Claude tools plugin config",
            )?;
        self.workspace_root
            .set(ctx.workspace_root)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        self.config
            .set(config)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Claude tools plugin initialized more than once"))?;
        Ok(InitOutcome::ack(agena_plugin_host::sdk::Plugin::manifest(
            self,
        )))
    }

    #[tool(
        name = "cloud_image_understanding",
        tags(query, network, mutate),
        summary = "Send explicit images to Anthropic cloud for understanding; not local file viewing.",
        help = "Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل صورًا محددة صراحةً إلى سحابة Anthropic لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene Bilder zur Analyse an die Anthropic-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía imágenes explícitas a la nube de Anthropic para comprenderlas; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des images explicites au cloud Anthropic pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दी गई छवियाँ Anthropic क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した画像を Anthropic クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 이미지를 Anthropic 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia imagens explícitas para a nuvem da Anthropic para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的图像发送到 Anthropic 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的圖像傳送至 Anthropic 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn image_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "claude.cloud_image_understanding")?;
        self.media_service()?
            .analyze(context, input, model, true)
            .await
    }

    #[tool(
        name = "cloud_document_understanding",
        tags(query, network, mutate),
        summary = "Send explicit PDF/text documents to Anthropic cloud for understanding; not local file viewing.",
        help = "Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned cloud_file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
        translations(
            locale(
                "ar-SA",
                summary = "أرسل مستندات PDF/نصية محددة صراحةً إلى سحابة Anthropic لفهمها؛ وليس لعرض الملفات المحلية."
            ),
            locale(
                "de-DE",
                summary = "Sendet ausdrücklich angegebene PDF-/Textdokumente zur Analyse an die Anthropic-Cloud; kein lokales Anzeigen von Dateien."
            ),
            locale(
                "es-ES",
                summary = "Envía documentos PDF/texto explícitos a la nube de Anthropic para comprenderlos; no es visualización de archivos locales."
            ),
            locale(
                "fr-FR",
                summary = "Envoie des documents PDF/textuels explicites au cloud Anthropic pour analyse ; ce n’est pas la consultation de fichiers locaux."
            ),
            locale(
                "hi-IN",
                summary = "समझने के लिए स्पष्ट रूप से दिए गए PDF/टेक्स्ट दस्तावेज़ Anthropic क्लाउड पर भेजता है; यह स्थानीय फ़ाइल देखना नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "明示的に指定した PDF／テキスト文書を Anthropic クラウドに送って解析します。ローカルファイルの閲覧ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "명시적으로 지정한 PDF/텍스트 문서를 Anthropic 클라우드로 보내 분석합니다. 로컬 파일 보기가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia documentos PDF/texto explícitos para a nuvem da Anthropic para compreensão; não é visualização de arquivos locais."
            ),
            locale(
                "zh-CN",
                summary = "将显式指定的 PDF/文本文档发送到 Anthropic 云端进行理解；并非查看本地文件。"
            ),
            locale(
                "zh-TW",
                summary = "將明確指定的 PDF／文字文件傳送至 Anthropic 雲端進行理解；並非檢視本機檔案。"
            )
        )
    )]
    async fn document_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let model = self.model(input.model.clone(), "claude.cloud_document_understanding")?;
        self.media_service()?
            .analyze(context, input, model, false)
            .await
    }

    #[tool(
        name = "cloud_file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to Anthropic cloud and return a session-owned handle.",
        help = "Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
        translations(
            locale(
                "ar-SA",
                summary = "ارفع ملفًا محليًا واحدًا مسموحًا به إلى سحابة Anthropic وأعِد معرّفًا مملوكًا للجلسة."
            ),
            locale(
                "de-DE",
                summary = "Lädt eine zulässige lokale Datei in die Anthropic-Cloud hoch und gibt ein sitzungseigenes Handle zurück."
            ),
            locale(
                "es-ES",
                summary = "Sube un archivo local permitido a la nube de Anthropic y devuelve un identificador propiedad de la sesión."
            ),
            locale(
                "fr-FR",
                summary = "Téléverse un fichier local autorisé vers le cloud Anthropic et renvoie un identifiant propre à la session."
            ),
            locale(
                "hi-IN",
                summary = "एक अनुमत स्थानीय फ़ाइल Anthropic क्लाउड पर अपलोड करता है और सत्र के स्वामित्व वाला हैंडल लौटाता है।"
            ),
            locale(
                "ja-JP",
                summary = "許可されたローカルファイルを1件 Anthropic クラウドにアップロードし、セッション所有のハンドルを返します。"
            ),
            locale(
                "ko-KR",
                summary = "허용된 로컬 파일 하나를 Anthropic 클라우드에 업로드하고 세션 소유 핸들을 반환합니다."
            ),
            locale(
                "pt-BR",
                summary = "Envia um arquivo local permitido para a nuvem da Anthropic e retorna um identificador pertencente à sessão."
            ),
            locale(
                "zh-CN",
                summary = "将一个获准的本地文件上传到 Anthropic 云端，并返回会话持有的句柄。"
            ),
            locale(
                "zh-TW",
                summary = "將一個獲准的本機檔案上傳至 Anthropic 雲端，並傳回工作階段持有的控制代碼。"
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
        summary = "Query the remote status of an owned Anthropic cloud file, not a local path.",
        help = "Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only cloud_file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
        translations(
            locale(
                "ar-SA",
                summary = "استعلم عن الحالة البعيدة لملف Anthropic سحابي مملوك، وليس مسارًا محليًا."
            ),
            locale(
                "de-DE",
                summary = "Fragt den entfernten Status einer eigenen Anthropic-Cloud-Datei ab, nicht einen lokalen Pfad."
            ),
            locale(
                "es-ES",
                summary = "Consulta el estado remoto de un archivo propio en la nube de Anthropic, no una ruta local."
            ),
            locale(
                "fr-FR",
                summary = "Interroge l’état distant d’un fichier Anthropic détenu, et non un chemin local."
            ),
            locale(
                "hi-IN",
                summary = "स्वामित्व वाली Anthropic क्लाउड फ़ाइल की दूरस्थ स्थिति पूछता है, स्थानीय पथ की नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "所有する Anthropic クラウドファイルの遠隔状態を問い合わせます。ローカルパスではありません。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 Anthropic 클라우드 파일의 원격 상태를 조회합니다. 로컬 경로가 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Consulta o estado remoto de um arquivo próprio na nuvem da Anthropic, não um caminho local."
            ),
            locale(
                "zh-CN",
                summary = "查询对话持有的 Anthropic 云端文件的远端状态，而非本地路径。"
            ),
            locale(
                "zh-TW",
                summary = "查詢工作階段持有的 Anthropic 雲端檔案的遠端狀態，而非本機路徑。"
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
        summary = "Request deletion of an owned file from Anthropic cloud; preserve the local original.",
        help = "Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
        translations(
            locale(
                "ar-SA",
                summary = "اطلب حذف ملف مملوك من سحابة Anthropic؛ مع الحفاظ على الأصل المحلي."
            ),
            locale(
                "de-DE",
                summary = "Fordert das Löschen einer eigenen Datei aus der Anthropic-Cloud an; das lokale Original bleibt erhalten."
            ),
            locale(
                "es-ES",
                summary = "Solicita eliminar un archivo propio de la nube de Anthropic; conserva el original local."
            ),
            locale(
                "fr-FR",
                summary = "Demande la suppression d’un fichier détenu dans le cloud Anthropic ; l’original local est conservé."
            ),
            locale(
                "hi-IN",
                summary = "Anthropic क्लाउड से स्वामित्व वाली फ़ाइल हटाने का अनुरोध करता है; स्थानीय मूल सुरक्षित रहता है।"
            ),
            locale(
                "ja-JP",
                summary = "所有するファイルの Anthropic クラウドからの削除を要求します。ローカルの原本は保持されます。"
            ),
            locale(
                "ko-KR",
                summary = "소유한 파일을 Anthropic 클라우드에서 삭제하도록 요청합니다. 로컬 원본은 유지됩니다."
            ),
            locale(
                "pt-BR",
                summary = "Solicita a exclusão de um arquivo próprio da nuvem da Anthropic; o original local é preservado."
            ),
            locale(
                "zh-CN",
                summary = "请求从 Anthropic 云端删除持有的文件；保留本地原件。"
            ),
            locale(
                "zh-TW",
                summary = "要求從 Anthropic 雲端刪除持有的檔案；保留本機原始檔。"
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
        summary = "Execute code in Anthropic cloud infrastructure, not on this computer.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.",
    translations(
        locale("ar-SA", summary = "نفّذ تعليمة برمجية في البنية السحابية لـ Anthropic، وليس على هذا الحاسوب."),
        locale("de-DE", summary = "Führt Code in der Anthropic-Cloud-Infrastruktur aus, nicht auf diesem Computer."),
        locale("es-ES", summary = "Ejecuta código en la infraestructura de Anthropic, no en este equipo."),
        locale("fr-FR", summary = "Exécute du code dans l’infrastructure Anthropic, pas sur cet ordinateur."),
        locale("hi-IN", summary = "Anthropic क्लाउड अधोसंरचना में कोड चलाता है, इस कंप्यूटर पर नहीं।"),
        locale("ja-JP", summary = "Anthropic クラウド基盤でコードを実行します。このコンピューター上ではありません。"),
        locale("ko-KR", summary = "Anthropic 클라우드 인프라에서 코드를 실행합니다. 이 컴퓨터가 아닙니다."),
        locale("pt-BR", summary = "Executa código na infraestrutura da nuvem da Anthropic, não neste computador."),
        locale("zh-CN", summary = "在 Anthropic 云端基础设施中执行代码，而非在本机执行。"),
        locale("zh-TW", summary = "在 Anthropic 雲端基礎架構中執行程式碼，而非在本機執行。")
)
)]
    async fn code_execution(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "code_execution",
            "Claude code execution",
            serde_json::json!({"type":"code_execution_20260521","name":"code_execution"}),
            &[],
            input,
        )
        .await
    }

    /// Streaming variant: a cloud sandbox run stays silent for a while, so a
    /// reader sees the work before the result lands.
    async fn code_execution_stream(
        &self,
        sink: ToolStreamSink,
        input: ClaudeToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Running code in Anthropic cloud…\n").await;
        self.code_execution(input).await
    }

    #[tool(
        name = "cloud_web_search",
        tags(network, interactive, discovery, read_only),
        summary = "Search the web in Anthropic cloud and return sources; not a local browser operation.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.",
        translations(
            locale(
                "ar-SA",
                summary = "ابحث في الويب داخل سحابة Anthropic وأعِد المصادر؛ وليس عملية متصفح محلي."
            ),
            locale(
                "de-DE",
                summary = "Durchsucht das Web in der Anthropic-Cloud und liefert Quellen; kein lokaler Browser-Vorgang."
            ),
            locale(
                "es-ES",
                summary = "Busca en la web en la nube de Anthropic y devuelve fuentes; no es una operación del navegador local."
            ),
            locale(
                "fr-FR",
                summary = "Recherche sur le Web dans le cloud Anthropic et renvoie les sources ; ce n’est pas une opération du navigateur local."
            ),
            locale(
                "hi-IN",
                summary = "Anthropic क्लाउड में वेब खोजता है और स्रोत लौटाता है; यह स्थानीय ब्राउज़र कार्रवाई नहीं है।"
            ),
            locale(
                "ja-JP",
                summary = "Anthropic クラウドでウェブを検索し、出典を返します。ローカルブラウザーの操作ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "Anthropic 클라우드에서 웹을 검색하고 출처를 반환합니다. 로컬 브라우저 작업이 아닙니다."
            ),
            locale(
                "pt-BR",
                summary = "Pesquisa na web na nuvem da Anthropic e retorna fontes; não é uma operação do navegador local."
            ),
            locale(
                "zh-CN",
                summary = "在 Anthropic 云端中搜索网络并返回来源；并非本地浏览器操作。"
            ),
            locale(
                "zh-TW",
                summary = "在 Anthropic 雲端中搜尋網路並傳回來源；並非本機瀏覽器操作。"
            )
        )
    )]
    async fn web_search(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_search",
            "Claude web search",
            serde_json::json!({"type":"web_search_20260318","name":"web_search"}),
            &[],
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_web_fetch",
        tags(network, interactive, discovery, read_only),
        summary = "Fetch and process web content in Anthropic cloud, not through the local browser.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.",
        translations(
            locale(
                "ar-SA",
                summary = "اجلب محتوى الويب وعالجه في سحابة Anthropic، وليس عبر المتصفح المحلي."
            ),
            locale(
                "de-DE",
                summary = "Ruft Webinhalte in der Anthropic-Cloud ab und verarbeitet sie, nicht über den lokalen Browser."
            ),
            locale(
                "es-ES",
                summary = "Obtiene y procesa contenido web en la nube de Anthropic, no mediante el navegador local."
            ),
            locale(
                "fr-FR",
                summary = "Récupère et traite du contenu web dans le cloud Anthropic, pas via le navigateur local."
            ),
            locale(
                "hi-IN",
                summary = "वेब सामग्री Anthropic क्लाउड में प्राप्त और संसाधित करता है, स्थानीय ब्राउज़र से नहीं।"
            ),
            locale(
                "ja-JP",
                summary = "ウェブコンテンツを Anthropic クラウドで取得・処理します。ローカルブラウザー経由ではありません。"
            ),
            locale(
                "ko-KR",
                summary = "웹 콘텐츠를 Anthropic 클라우드에서 가져와 처리합니다. 로컬 브라우저를 통하지 않습니다."
            ),
            locale(
                "pt-BR",
                summary = "Obtém e processa conteúdo web na nuvem da Anthropic, não pelo navegador local."
            ),
            locale(
                "zh-CN",
                summary = "在 Anthropic 云端获取并处理网页内容，而非通过本地浏览器。"
            ),
            locale(
                "zh-TW",
                summary = "在 Anthropic 雲端取得並處理網頁內容，而非透過本機瀏覽器。"
            )
        )
    )]
    async fn web_fetch(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "web_fetch",
            "Claude web fetch",
            serde_json::json!({"type":"web_fetch_20260318","name":"web_fetch"}),
            &[],
            input,
        )
        .await
    }

    #[tool(
        name = "cloud_advisor",
        stream = advisor_stream,
        tags(network, interactive, read_only),
        summary = "Consult an advisor model in Anthropic cloud using the supplied context.",
        help = "Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the configured provider endpoint; local project files are not automatically available. No local execution fallback. Only supplied context is available; the local repository and session transcript are not automatically uploaded. Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.",
    translations(
        locale("ar-SA", summary = "استشر نموذجًا استشاريًا في سحابة Anthropic باستخدام السياق المقدَّم."),
        locale("de-DE", summary = "Zieht ein Beratermodell in der Anthropic-Cloud mit dem übergebenen Kontext zu Rate."),
        locale("es-ES", summary = "Consulta un modelo asesor en la nube de Anthropic usando el contexto proporcionado."),
        locale("fr-FR", summary = "Consulte un modèle conseiller dans le cloud Anthropic à partir du contexte fourni."),
        locale("hi-IN", summary = "दिए गए संदर्भ के साथ Anthropic क्लाउड में सलाहकार मॉडल से परामर्श लेता है।"),
        locale("ja-JP", summary = "指定したコンテキストを使って Anthropic クラウドのアドバイザーモデルに相談します。"),
        locale("ko-KR", summary = "제공된 컨텍스트로 Anthropic 클라우드의 어드바이저 모델에 자문합니다."),
        locale("pt-BR", summary = "Consulta um modelo conselheiro na nuvem da Anthropic usando o contexto fornecido."),
        locale("zh-CN", summary = "使用提供的上下文向 Anthropic 云端中的顾问模型咨询。"),
        locale("zh-TW", summary = "使用提供的內容向 Anthropic 雲端中的顧問模型諮詢。")
)
)]
    async fn advisor(&self, input: ClaudeToolInput) -> SdkResult<ToolInvokeOutput> {
        self.messages_tool(
            "advisor",
            "Claude advisor",
            serde_json::json!({"type":"advisor_20260301","name":"advisor"}),
            &["advisor-tool-2026-03-01"],
            input,
        )
        .await
    }

    /// Streaming variant: the advisor call is a long remote request, so a
    /// reader sees the work before the answer lands.
    async fn advisor_stream(
        &self,
        sink: ToolStreamSink,
        input: ClaudeToolInput,
    ) -> SdkResult<ToolInvokeOutput> {
        sink.text("Consulting the Anthropic advisor…\n").await;
        self.advisor(input).await
    }
}
