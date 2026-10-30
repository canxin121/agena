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
    merge_object_options, post_json, provider_output, read_image_input_bounded,
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
    /// Optional model override on the current provider; defaults to the current session model. Must be an OpenAI model that supports this operation. Pass model explicitly if the current model is unsuitable.
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
    trim("prompt", "model", "stable_instructions"),
    max_chars("prompt", 64000),
    max_chars("stable_instructions", 256000)
)]
#[serde(deny_unknown_fields)]
struct ChatGptResponsesImageInput {
    /// Drawing or editing request for an additional Responses chat model, which calls the hosted GPT Image tool. May be omitted when input_items supplies the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    prompt: Option<String>,
    /// Developer instructions for this additional Responses request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    stable_instructions: Option<String>,
    /// Responses CHAT model that can call the hosted image_generation tool; defaults to the current session model. Select the GPT Image drawing model in tool_options.model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Hosted image tool settings. model selects the DRAWING model, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Other model-supported fields include action (auto/generate/edit), size, quality, and output_format. type is protected.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    tool_options: BTreeMap<String, serde_json::Value>,
    /// Additional outer Responses request fields. model, input, tools, stream, and previous_response_id are supplied separately.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
    /// OpenAI response ID from an earlier call to this tool, used to continue its server-side context. This is not an Agena session ID.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    previous_response_id: Option<String>,
    /// Explicit Responses history and image references (supported URLs, base64 data URLs, or file IDs). Upload or encode local images first. Agena conversation history is not automatically included; client tool callbacks are rejected.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    input_items: Vec<serde_json::Value>,
    /// Optional Responses include selectors.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    include: Vec<String>,
}

impl From<ChatGptResponsesImageInput> for ChatGptToolInput {
    fn from(input: ChatGptResponsesImageInput) -> Self {
        Self {
            prompt: input.prompt,
            stable_instructions: input.stable_instructions,
            model: input.model,
            tool_options: input.tool_options,
            request_options: input.request_options,
            previous_response_id: input.previous_response_id,
            input_items: input.input_items,
            include: input.include,
        }
    }
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
    /// Natural-language edit instructions, including details to preserve from the supplied images.
    prompt: String,
    /// One to sixteen permitted local image paths to upload as originals or references. URLs and remote file IDs are not accepted here.
    images: Vec<String>,
    /// GPT Image model ID on the current provider, e.g. gpt-image-2 or gpt-image-2.5-sunburst. Defaults to the session model; explicitly select an image model when the session uses a chat model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    /// Additional Images edit fields supported by the chosen model, e.g. size, quality, n, and output_format. model, prompt, and image are supplied separately.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    options: BTreeMap<String, serde_json::Value>,
}

impl ChatGptToolsPlugin {
    async fn media_service(&self) -> SdkResult<Service<'_>> {
        Service::new(
            "chatgpt",
            self.workspace_root()?,
            self.host()?,
            self.host()?.builtin_provider_connection().await?,
            "2023-06-01",
        )
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
        let connection = self.host()?.builtin_provider_connection().await?;
        let public_tool_name = agena_tool::provider_tools::cloud_operation("chatgpt", tool_name)
            .map_or(tool_name, |tool| tool.name);
        let model = configured_model(input.model, connection.model.as_deref(), public_tool_name)?;
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
        let url = endpoint(&connection.base_url, "responses")?;
        let headers = connection.headers.clone();
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            &connection,
            "chatgpt",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::OpenAiResponses,
            response,
            &connection,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "chatgpt",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. OpenAI cloud inference, files, knowledge stores, images, audio, realtime voice, batches and agent APIs. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "需要模型的工具默认使用当前会话模型。当前模型须为支持该操作的 OpenAI 模型，否则必须显式传入 model 指定合适模型。复用当前会话的 provider 连接，无需单独配置。使用 OpenAI 云端推理、文件、知识库、图像、音频、实时语音、批处理与智能体 API。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "需要模型的工具預設使用目前工作階段模型。目前模型須為支援該操作的 OpenAI 模型，否則必須明確傳入 model 指定合適模型。沿用目前工作階段的 provider 連線，無需另外設定。使用 OpenAI 雲端搜尋、運算與影像功能。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "モデルが必要なツールは現在のセッションモデルを使用します。対象操作に対応する OpenAI モデルが必要です。現在のモデルが適合しない場合は model を明示してください。現在のセッションのプロバイダー接続を使用します。OpenAI のクラウド検索・計算・画像機能を利用します。入力はこの端末の外部に送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "모델이 필요한 도구는 현재 세션 모델을 기본으로 사용합니다. 작업을 지원하는 OpenAI 모델이 필요하며, 현재 모델이 적합하지 않으면 model을 명시하세요. 현재 세션의 공급자 연결을 사용합니다. OpenAI 클라우드 검색, 계산, 이미지 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Les outils nécessitant un modèle utilisent celui de la session. Choisissez un modèle OpenAI compatible avec l’opération ; sinon, renseignez explicitement model. Utilise la connexion du fournisseur de la session courante. Utiliser les fonctions de recherche, de calcul et d’image dans le cloud OpenAI. Les données sont envoyées hors de cet ordinateur, sans solution de repli locale."),
        locale("de-DE", summary = "Modellbasierte Werkzeuge verwenden das aktuelle Sitzungsmodell. Ein für die Aktion geeignetes OpenAI-Modell ist erforderlich; andernfalls model explizit angeben. Verwendet die Provider-Verbindung der aktuellen Sitzung. OpenAI-Cloudfunktionen für Suche, Berechnungen und Bilder nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Las herramientas que requieren modelo usan el de la sesión. Se necesita un modelo de OpenAI compatible con la operación; de lo contrario, indica model explícitamente. Usa la conexión del proveedor de la sesión actual. Usa las funciones de búsqueda, cálculo e imagen en la nube de OpenAI. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "मॉडल वाले टूल वर्तमान सत्र का मॉडल इस्तेमाल करते हैं। इस कार्य का समर्थन करने वाला OpenAI मॉडल चाहिए; अन्यथा model स्पष्ट रूप से दें। वर्तमान सत्र के प्रदाता कनेक्शन का उपयोग करता है। OpenAI क्लाउड की खोज, गणना और छवि सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "تستخدم الأدوات التي تحتاج نموذجًا نموذج الجلسة الحالي. يجب استخدام نموذج من OpenAI يدعم العملية؛ وإلا فحدّد model صراحةً. يستخدم اتصال المزوّد للجلسة الحالية. استخدم إمكانات البحث والحوسبة والصور في سحابة OpenAI. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Ferramentas que exigem modelo usam o da sessão. É necessário um modelo da OpenAI compatível com a operação; caso contrário, informe model explicitamente. Usa a conexão do provedor da sessão atual. Use os recursos de busca, computação e imagem na nuvem da OpenAI. As entradas saem deste computador; não há alternativa de execução local.")
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
        name = "files",
        tags(network, mutate),
        summary = "Manage remote files and download cloud-generated artifacts.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.",
        translations(locale(
            "zh-CN",
            summary = "管理远程文件，下载厂商生成的文件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。支持 list、get、wait、delete、download。OpenAI/Claude 还支持 upload：path 指定本地文件，parameters.purpose 按厂商要求填写。Gemini 使用 file_upload 上传。download 将实际字节保存为附件或新的 output_path；Claude 仅支持下载厂商生成的文件。id 属于当前 provider，列表与管理接口不需要模型。"
        ))
    )]
    async fn api_files(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Files,
            input,
        )
        .await
    }

    #[tool(
        name = "inference",
        tags(network, mutate),
        summary = "Run a vendor inference request with official tool combinations and continuation.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.",
        translations(locale(
            "zh-CN",
            summary = "使用厂商原生接口组合工具、推理并继续对话。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create；prompt 或 parameters 中填写厂商原生 input/messages，可组合托管工具、远程 MCP、工具搜索、编程式调用、云端 Skills 或客户端工具声明。返回的工具调用是数据，不会自动执行本机操作。需要本地执行时，使用正常 Agena 工具及其权限，再显式提交厂商格式的调用结果。顶层 stream=true 可接收限时 SSE，wait_seconds 为 1..50，保留完整事件与 SSE 游标。"
        ))
    )]
    async fn api_inference(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Inference,
            input,
        )
        .await
    }

    #[tool(
        name = "batches",
        tags(network, mutate),
        summary = "Create, inspect, cancel and retrieve asynchronous batch jobs.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, cancel; Claude/Gemini also delete, Claude also results. parameters contains official batch input. OpenAI result output_file_id is downloaded with files.download; Gemini output file names use files.download. Claude results are saved as a JSONL attachment. A wait is bounded and never resubmits a job.",
        translations(locale(
            "zh-CN",
            summary = "创建、查询、取消异步批处理并获取结果。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。支持 create、list、get、wait、cancel；Claude/Gemini 支持 delete，Claude 支持 results 并保存 JSONL。parameters 使用官方批处理格式。Claude requests[].params 未指定 model 时沿用顶层 model 或当前模型；OpenAI 输入 JSONL 中的各行模型由文件内容决定。OpenAI output_file_id 和 Gemini 输出文件可用 files.download 下载。wait 有时限且不会重建任务。"
        ))
    )]
    async fn api_batches(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Batches,
            input,
        )
        .await
    }

    #[tool(
        name = "count_tokens",
        tags(network, mutate),
        summary = "Count tokens using the vendor token-counting endpoint.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.",
        translations(locale(
            "zh-CN",
            summary = "通过厂商接口计算输入 token 数量。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create；parameters 使用厂商的 input/messages/contents 格式。只计算输入 token，不生成文本。"
        ))
    )]
    async fn api_count_tokens(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Tokens,
            input,
        )
        .await
    }

    #[tool(
        name = "agents",
        tags(network, mutate),
        summary = "Manage vendor-hosted agents, sessions and environments.",
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model for agent creation. Resource management needs no model. resource=agents: create/list/get/update/delete; sessions (default): create/list/get/wait/update/delete/events/create_event/items; environments: create/list/get/add_file/list_files. Additional resources: sessions/artifacts list/get/download/delete; sessions/turns list/get/items; sessions/subagents list/get/items; sessions/subagents/turns list/get/items; sessions/traces list. Nested resources use id=session, item_id=artifact/turn/subagent; subagent turns also use sub_item_id=turn. download saves actual artifact bytes. Event streams are bounded by wait_seconds and accept last_event_id for resumption. parameters/query use official schemas. Hosted agents do not automatically receive local files or tools.",
        translations(locale(
            "zh-CN",
            summary = "管理厂商托管的智能体、会话与环境。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。resource=agents 支持 create/list/get/update/delete；sessions（默认）支持 create/list/get/wait/update/delete/events/create_event/items；environments 支持 create/list/get/add_file/list_files。新增 sessions/artifacts 支持 list/get/download/delete，sessions/turns、sessions/subagents、sessions/subagents/turns 支持 list/get/items，sessions/traces 支持 list。嵌套资源 id 为会话，item_id 为产物、turn 或子智能体；子智能体 turn 另用 sub_item_id。download 保存真实文件。事件流由 wait_seconds 限时，可传 last_event_id 续接。资源查询无需模型。"
        ))
    )]
    async fn api_agents(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Agents,
            input,
        )
        .await
    }

    #[tool(
        name = "knowledge",
        tags(network, mutate),
        summary = "Create, populate, search and manage OpenAI vector stores.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, update, delete, search, add_file, list_files, get_file, update_file, delete_file, download_file, create_batch, get_batch, cancel_batch, list_batch_files. id is the vector store ID; item_id identifies a nested file or file batch. Upload with files.upload purpose=assistants, then add_file with parameters.file_id. Poll get_file until indexing completes; file_upload alone does not index. File batch creation uses parameters.file_ids. search returns chunks directly without a generation model.",
        translations(locale(
            "zh-CN",
            summary = "创建、导入和管理厂商知识库。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。OpenAI 支持 create/list/get/wait/update/delete/search，以及 add_file/list_files/get_file/update_file/delete_file/download_file/create_batch/get_batch/cancel_batch/list_batch_files。id 为向量库，item_id 为文件或文件批次；先上传 purpose=assistants，再 add_file，等待索引完成后搜索。Gemini 支持 create/list/get/delete/import_file/list_files/get_file/delete_file；id 为 File Search Store，item_id 为文档。import_file 使用 parameters.fileName，返回的操作交给 operations.get/wait。上传文件本身不会自动建立知识库索引。"
        ))
    )]
    async fn api_knowledge(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Knowledge,
            input,
        )
        .await
    }

    #[tool(
        name = "containers",
        tags(network, mutate),
        summary = "Manage OpenAI cloud containers and their input/output files.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, delete, upload/add_file, list_files, get_file, delete_file, download_file. id is container ID; item_id is container-file ID. Add an existing file with parameters.file_id, or upload path. download_file saves actual bytes locally. Container execution is remote.",
        translations(locale(
            "zh-CN",
            summary = "管理 OpenAI 执行容器与生成文件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。支持 create/list/get/delete/upload/add_file/list_files/get_file/delete_file/download_file。id 为容器，item_id 为文件。upload 使用 path；也可 add_file 引用 parameters.file_id。下载保存实际字节，容器中的文件不会自动出现在本地工作区。"
        ))
    )]
    async fn api_containers(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Containers,
            input,
        )
        .await
    }

    #[tool(
        name = "responses",
        tags(network, mutate),
        summary = "Retrieve, wait for, cancel or delete OpenAI background responses.",
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model. Actions: create/get/wait/cancel/delete/items/compact. Create accepts prompt or parameters.input, optional background/tools, and top-level stream=true for bounded SSE. compact calls responses/compact with parameters.input and optional instructions. Other actions use id. wait_seconds is 1..50; get/wait never recreate a task. SSE returns events, sse_events with id/event/retry, and last_event_id; pass last_event_id to resume supported event resources. Continuation via previous_response_id creates a new response. Never retry create automatically.",
        translations(locale(
            "zh-CN",
            summary = "创建和管理 OpenAI Responses 后台任务。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。支持 create/get/wait/cancel/delete/items/compact。compact 调用 Responses 压缩接口，parameters.input 为待压缩消息，可传 instructions。create 可传 background/tools，顶层 stream=true 开启有时限的 SSE。后续查询使用 id；wait_seconds 为 1..50。SSE 保留事件类型、事件 ID 和重连等待值；查询不会重建任务。"
        ))
    )]
    async fn api_responses(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Tasks,
            input,
        )
        .await
    }

    #[tool(
        name = "image_generate",
        tags(network, mutate),
        summary = "Generate images directly with the OpenAI Images API; set model to a GPT Image drawing model such as gpt-image-2.",
        help = "Prefer this for ordinary text-to-image tasks. Sends POST images/generations directly to a GPT Image drawing model using the prompt supplied by the current Agena AI. A plain-text prompt is sufficient. To select an OpenAI GPT Image model supported by the current provider, explicitly pass model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare. These are examples, not an exhaustive list. Here model is the drawing model. It defaults to the current session model when omitted, so explicitly select a drawing model when the session uses a chat model. Use action=create. Example: {\"action\":\"create\",\"model\":\"gpt-image-2\",\"prompt\":\"A watercolor gray cat wearing a yellow scarf\"}. parameters accepts model-supported size, quality, n, output_format, background, and other official image fields. Use chatgpt.image_edit for local-image edits or references. Use chatgpt.responses_image_generation when you need Responses server-side conversation context; that tool makes an additional chat-model request. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support the Images API and selected model. Prompts are sent to that endpoint; returned base64 images are persisted as managed local attachments.",
        translations(
            locale(
                "zh-CN",
                summary = "通过 OpenAI Images API 直接文生图；model 指定 gpt-image-2、gpt-image-2.5-sunburst 等 GPT Image 绘图模型。",
                help = "普通文生图优先使用此工具。通过 POST images/generations 将当前 Agena AI 提供的绘图提示词直接交给 GPT Image 绘图模型，纯文本 prompt 即可。显式传入 model 选择当前 provider 支持的 OpenAI 绘图模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，也可使用 provider 支持的其他 GPT Image 模型 ID。这里的 model 是绘图模型；省略时默认使用当前会话模型，因此当前为聊天模型时应显式指定绘图模型。传 action=create，例如 {\"action\":\"create\",\"model\":\"gpt-image-2\",\"prompt\":\"一只戴黄色围巾的灰猫，水彩风格\"}。parameters 可传所选模型支持的 size、quality、n、output_format、background 等参数。已有本地图片的编辑或参考图生成使用 chatgpt.image_edit；需要 Responses 服务端保存的对话上下文时使用 chatgpt.responses_image_generation，后者会额外调用一个对话模型。复用当前 provider 的地址、认证和网络设置，该连接须支持 Images API 和所选模型。提示词发往该连接，返回图片保存为本地托管附件。"
            ),
            locale(
                "zh-TW",
                summary = "透過 OpenAI Images API 直接由文字繪圖；model 指定 gpt-image-2、gpt-image-2.5-sunburst 等 GPT Image 繪圖模型。",
                help = "一般文字繪圖優先使用此工具。透過 POST images/generations 將目前 Agena AI 提供的繪圖提示詞直接交給 GPT Image 繪圖模型，純文字 prompt 即可。明確傳入 model 選擇目前 provider 支援的繪圖模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，也可使用 provider 支援的其他 GPT Image 模型 ID。此處 model 是繪圖模型；省略時預設使用目前工作階段模型，因此目前為聊天模型時應明確指定繪圖模型。傳入 action=create；parameters 可傳所選模型支援的 size、quality、n、output_format、background 等參數。編輯本機圖片或使用參考圖時使用 chatgpt.image_edit；需要 Responses 伺服器儲存的對話上下文時使用 chatgpt.responses_image_generation，後者會額外呼叫一個對話模型。沿用目前 provider 的位址、認證及網路設定，該連線須支援 Images API 和所選模型。提示詞傳送至該連線，回傳圖片儲存為本機託管附件。"
            )
        )
    )]
    async fn api_image_generate(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Images,
            input,
        )
        .await
    }

    #[tool(
        name = "transcription",
        tags(network, mutate),
        summary = "Transcribe or translate an audio file using OpenAI Audio APIs.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. path selects one authorized audio file; model defaults to the current session model, so explicitly pass an appropriate transcription model when necessary. resource=translations selects audio translation; otherwise transcription. parameters supports language, prompt, response_format and timestamp/diarization fields according to model compatibility. Text, JSON and subtitle outputs are retained as artifacts.",
        translations(locale(
            "zh-CN",
            summary = "通过 OpenAI Audio 接口转写或翻译音频。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create；path 指定获准读取的音频，resource=translations 选择翻译，否则转写。parameters 支持 language、prompt、response_format、时间戳和说话人字段，具体依模型而定。文本、JSON 和字幕均保留为结果文件。"
        ))
    )]
    async fn api_transcription(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Transcription,
            input,
        )
        .await
    }

    #[tool(
        name = "speech",
        tags(network, mutate),
        summary = "Generate speech and save the resulting audio.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. prompt is the exact text to speak; parameters.voice and other official speech fields control voice, format and style. Select an OpenAI TTS model. Saves actual audio bytes as an attachment; output_path optionally selects a new workspace file.",
        translations(locale(
            "zh-CN",
            summary = "生成语音并保存为本地音频附件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create，prompt 为待朗读文本。OpenAI 使用 parameters.voice 等官方语音字段。Gemini 使用 parameters.generation_config.speech_config，复杂输入和 speech_metadata 放入 parameters.input；自动设置 response_format.type=audio。必须使用支持语音合成的模型。output_path 可指定新的输出文件。"
        ))
    )]
    async fn api_speech(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Speech,
            input,
        )
        .await
    }

    #[tool(
        name = "embeddings",
        tags(network, mutate),
        summary = "Create OpenAI embeddings for supplied inputs.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. parameters.input contains text or token inputs; parameters supports dimensions and encoding_format. Requires an embedding model. Large vectors are saved in the response receipt.",
        translations(locale(
            "zh-CN",
            summary = "生成文本或多模态嵌入向量。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create，需要嵌入模型。OpenAI 使用 parameters.input，可选 dimensions/encoding_format；Gemini 使用 parameters.content，按模型支持传 taskType/title/outputDimensionality。大向量保存在完整响应文件中。"
        ))
    )]
    async fn api_embeddings(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Embeddings,
            input,
        )
        .await
    }

    #[tool(
        name = "moderation",
        tags(network, mutate),
        summary = "Classify supplied content with the OpenAI Moderations API.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. parameters.input contains supported text/image inputs. Requires a moderation model, not an ordinary chat model. Returns provider categories and scores.",
        translations(locale(
            "zh-CN",
            summary = "通过 OpenAI Moderations 接口审核内容。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。默认 create；parameters.input 为待审核的文本或图像。需使用兼容的内容审核模型；结果保留分类、分数和标记。"
        ))
    )]
    async fn api_moderation(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Moderation,
            input,
        )
        .await
    }

    #[tool(
        name = "platform",
        tags(network, mutate),
        summary = "Manage OpenAI conversations, fine-tuning jobs, evaluations and uploads.",
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible OpenAI model or explicitly pass model for fine-tuning. Resource operations need no model. resource=conversations: create/get/update/delete/items/add_items/get_item/delete_item; fine_tuning/jobs: create/list/get/wait/cancel/events/pause/unpause; fine_tuning/jobs/checkpoints: list (id=job); fine_tuning/checkpoints/permissions: create/list/delete (id=checkpoint, item_id=permission); fine_tuning/alpha/graders: run/validate; evals: create/list/get/update/delete/create_run/list_runs/get_run/cancel_run/delete_run; evals/runs/output_items: list/get (id=eval, item_id=run, sub_item_id=output for get); uploads: create/add_part/complete/cancel (add_part path, complete parameters.part_ids); audio/voice_consents: create/list/get/update/delete (create path is the consent recording, parameters.name/language); audio/voices: create/delete (audio-sample creation uses path and parameters.name/consent, prompt creation uses parameters.type=prompt/name/prompt without a file). Voice recordings have a 10 MiB limit and supported audio MIME types. Nested grader model fields remain in parameters. No API key, billing or membership administration.",
        translations(locale(
            "zh-CN",
            summary = "管理 OpenAI 对话、微调、评估与分片上传。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 OpenAI 模型，否则显式传入 model。资源管理操作无需模型。resource=conversations 管理对话及 items；fine_tuning/jobs 管理微调任务，支持 create/list/get/wait/cancel/events/pause/unpause；fine_tuning/jobs/checkpoints 用 list 查看检查点；fine_tuning/checkpoints/permissions 支持 create/list/delete；fine_tuning/alpha/graders 支持 run/validate。evals 支持 create/list/get/update/delete/create_run/list_runs/get_run/cancel_run/delete_run；evals/runs/output_items 支持 list/get，id 为 eval、item_id 为 run，get 另需 sub_item_id。uploads 支持 create/add_part/complete/cancel。audio/voice_consents 支持 create/list/get/update/delete，创建时 path 为授权录音，parameters 包含 name/language；audio/voices 支持 create/delete，录音方式使用 path 和 name/consent，文字方式使用 parameters.type=prompt/name/prompt，无需文件。录音上限 10 MiB。资源管理不要求聊天模型。"
        ))
    )]
    async fn api_platform(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Platform,
            input,
        )
        .await
    }

    #[tool(
        name = "audio_understanding",
        tags(network),
        summary = "Analyze audio with a selected OpenAI model.",
        help = "Model-based tools default to the current session model. Use an OpenAI model supporting audio input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.",
        translations(locale(
            "zh-CN",
            summary = "使用指定模型理解音频。",
            help = "默认使用当前会话模型；必须为支持该媒体的 OpenAI 模型，否则显式传入 model。复用当前 provider 连接。仅发送 inputs 显式指定并获准读取的文件；Gemini 可用本会话上传句柄，OpenAI 音频需本地 WAV/MP3。"
        ))
    )]
    async fn audio_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "chatgpt.audio_understanding",
        )?;
        service
            .analyze_kind(
                context,
                input,
                model,
                Some(agena_domain::AttachmentKind::Audio),
            )
            .await
    }

    #[tool(
        name = "realtime",
        tags(network, mutate),
        summary = "Run one bounded OpenAI realtime voice turn and save audio.",
        help = "Defaults to the current session model. Use an OpenAI realtime voice model; otherwise explicitly pass model. Reuses the current provider endpoint and credentials. Supply text or an explicit raw 16-bit mono PCM audio_path. Returns transcript, actual WAV audio, completion state and provider events. Does not open a microphone or execute tool calls. duration_seconds is 1..50; partial output is retained on a receive deadline. Requires the gateway to expose the vendor WebSocket API.",
        translations(locale(
            "zh-CN",
            summary = "调用实时语音 API 并保存返回音频。",
            help = "默认使用当前会话模型；须为支持实时语音的 OpenAI 模型，否则显式传入 model。复用当前 provider 连接。输入为 text 或明确指定的原始 16 位单声道 PCM 文件；返回转录与 WAV 附件，不会打开麦克风或执行工具回调。"
        ))
    )]
    async fn realtime(
        &self,
        input: super::provider_api::realtime::RealtimeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::realtime::invoke(self.media_service().await?, input).await
    }

    #[tool(
        name = "image_understanding",
        tags(query, network, mutate),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Send explicit images to OpenAI cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "chatgpt.image_understanding",
        )?;
        service.analyze(context, input, model, true).await
    }

    #[tool(
        name = "document_understanding",
        tags(query, network, mutate),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Send explicit PDF/text documents to OpenAI cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to OpenAI. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "chatgpt.document_understanding",
        )?;
        service.analyze(context, input, model, false).await
    }

    #[tool(
        name = "file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to OpenAI cloud and return a session-owned handle.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Uploads accept arbitrary explicit data files up to 512 MiB and optional SHA256 revision checks; analysis has separate format/size limits. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
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
        self.media_service().await?.upload(context, input).await
    }

    #[tool(
        name = "file_status",
        tags(query, network, mutate),
        summary = "Query the remote status of an owned OpenAI cloud file, not a local path.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
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
        self.media_service()
            .await?
            .file_control(context, input, false)
            .await
    }

    #[tool(
        name = "file_delete",
        tags(mutate, network),
        summary = "Request deletion of an owned file from OpenAI cloud; preserve the local original.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
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
        self.media_service()
            .await?
            .file_control(context, input, true)
            .await
    }
    #[tool(
        name = "web_search",
        tags(network, interactive, discovery, read_only),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Search the web in OpenAI cloud and return sources; not a local browser operation.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options accepts the official WebSearchToolParam fields: filters.allowed_domains, search_context_size, user_location, and versioned type-compatible options. Hosted results and response_id are returned for follow-up; this plugin never executes client tool callbacks.",
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
        name = "file_search",
        tags(network, interactive, discovery, read_only),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Search configured OpenAI cloud file stores, not files on this computer.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. Set tool_options.vector_store_ids and optional filters, max_num_results, and ranking_options exactly as documented by OpenAI.",
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
        name = "code_interpreter",
        stream = code_interpreter_stream,
        tags(network, interactive, read_only),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Run Python in an OpenAI cloud container, not the Agena local workspace.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. tool_options.container may be a container id or an auto container object with file_ids, memory_limit, and network_policy.",
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
        name = "responses_image_generation",
        tags(network, interactive, mutate),
        summary = "Ask an additional OpenAI Responses chat model to call GPT Image; model selects chat, tool_options.model selects the drawing model.",
        help = "Use when you need Responses server-side conversation context for drawing or editing. Submits an additional POST responses request: current Agena AI -> Responses chat model -> hosted image_generation tool / GPT Image drawing model -> image. The Responses chat model interprets the request, may rewrite the drawing prompt, and calls the drawing model to produce the image. Top-level model selects this additional Responses CHAT model; it defaults to the current session model, which must support the hosted image tool. If needed, explicitly pass model to select a compatible chat model. tool_options.model separately selects the DRAWING model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare when supported by the current provider. Put GPT Image IDs in tool_options.model. prompt can be plain text. Supply reference images through input_items using supported image URLs, base64 data URLs, or file IDs; upload or encode local images first. previous_response_id references a prior OpenAI response whose context the provider retains; it is not an Agena session ID. tool_options accepts action (auto/generate/edit), model, background, input_fidelity, input_image_mask, moderation, output_compression, output_format, partial_images, quality, and size according to model support. Prefer chatgpt.image_generate for ordinary text-to-image and chatgpt.image_edit for local-image edits or references; repeated direct edits can pass the prior output path back to image_edit. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support Responses and its hosted image tool. Only explicitly supplied prompts, input_items and referenced response context are included; Agena history and local project files are not automatically attached. Returned base64 images are persisted as managed local attachments.",
        translations(
            locale(
                "ar-SA",
                summary = "اطلب من نموذج محادثة Responses إضافي استدعاء GPT Image؛ model للمحادثة وtool_options.model للرسم.",
                help = "يرسل طلب POST responses إضافيًا: ذكاء Agena الحالي ثم نموذج محادثة Responses ثم أداة GPT Image المستضافة لإنتاج الصورة. يحدد model نموذج المحادثة، ويحدد tool_options.model نموذج الرسم مثل gpt-image-2. قد يعيد نموذج المحادثة صياغة وصف الرسم. استخدم previous_response_id لسياق استجابة سابقة محفوظة لدى المزود؛ سجل Agena لا يُرفق تلقائيًا. للرسم العادي استخدم chatgpt.image_generate، ولتعديل صورة محلية استخدم chatgpt.image_edit. يستخدم اتصال المزود الحالي؛ يجب أن يدعم Responses وأداة الصور."
            ),
            locale(
                "de-DE",
                summary = "Bilder über ein zusätzliches Responses-Chatmodell; model wählt den Chat, tool_options.model das GPT-Image-Zeichenmodell.",
                help = "Sendet einen zusätzlichen POST-responses-Aufruf: aktuelle Agena-KI -> Responses-Chatmodell -> gehostetes GPT-Image-Werkzeug -> Bild. model wählt das Chatmodell, tool_options.model das Zeichenmodell, z. B. gpt-image-2. Das Chatmodell kann den Bildprompt umformulieren. previous_response_id verweist auf beim Anbieter gespeicherten Antwortkontext; der Agena-Verlauf wird nicht automatisch übertragen. Für normale Textbilder chatgpt.image_generate, für lokale Bildbearbeitung chatgpt.image_edit bevorzugen. Verwendet die aktuelle Anbieterverbindung, die Responses und das Bildwerkzeug unterstützen muss."
            ),
            locale(
                "es-ES",
                summary = "Un modelo de chat Responses adicional invoca GPT Image; model elige el chat y tool_options.model el modelo de dibujo.",
                help = "Envía una solicitud POST responses adicional: IA actual de Agena -> modelo de chat Responses -> herramienta alojada GPT Image -> imagen. model selecciona el modelo de chat y tool_options.model el de dibujo, por ejemplo gpt-image-2. El chat puede reformular la descripción de la imagen. previous_response_id reutiliza el contexto de una respuesta guardada por el proveedor; el historial de Agena no se adjunta automáticamente. Prefiera chatgpt.image_generate para texto a imagen y chatgpt.image_edit para imágenes locales. Reutiliza la conexión actual, que debe admitir Responses y su herramienta de imágenes."
            ),
            locale(
                "fr-FR",
                summary = "Un modèle de chat Responses supplémentaire appelle GPT Image ; model choisit le chat, tool_options.model le modèle de dessin.",
                help = "Envoie une requête POST responses supplémentaire : IA Agena actuelle -> modèle de chat Responses -> outil GPT Image hébergé -> image. model choisit le modèle de chat et tool_options.model le modèle de dessin, par exemple gpt-image-2. Le chat peut reformuler la consigne de dessin. previous_response_id reprend le contexte d’une réponse conservée par le fournisseur ; l’historique Agena n’est pas joint automatiquement. Préférer chatgpt.image_generate pour le texte vers image et chatgpt.image_edit pour les images locales. Utilise la connexion actuelle, qui doit prendre en charge Responses et son outil d’image."
            ),
            locale(
                "hi-IN",
                summary = "एक अतिरिक्त Responses चैट मॉडल से GPT Image चलाएँ; model चैट मॉडल और tool_options.model चित्र मॉडल चुनता है।",
                help = "अतिरिक्त POST responses अनुरोध भेजता है: वर्तमान Agena AI -> Responses चैट मॉडल -> होस्ट किया गया GPT Image टूल -> चित्र। model चैट मॉडल और tool_options.model चित्र मॉडल, जैसे gpt-image-2, चुनता है। चैट मॉडल चित्र का प्रॉम्प्ट दोबारा लिख सकता है। previous_response_id प्रदाता द्वारा रखे गए पिछले उत्तर का संदर्भ देता है; Agena का इतिहास अपने आप नहीं जुड़ता। सामान्य टेक्स्ट से चित्र के लिए chatgpt.image_generate और स्थानीय चित्र संपादन के लिए chatgpt.image_edit चुनें। वर्तमान प्रदाता कनेक्शन को Responses और उसके चित्र टूल का समर्थन करना चाहिए।"
            ),
            locale(
                "ja-JP",
                summary = "追加の Responses 会話モデルから GPT Image を呼び出します。model は会話、tool_options.model は描画モデルを指定します。",
                help = "追加の POST responses を送信します。現在の Agena AI -> Responses 会話モデル -> ホスト型 GPT Image ツール -> 画像の順に処理します。model は会話モデル、tool_options.model は gpt-image-2 などの描画モデルです。会話モデルは描画プロンプトを書き換える場合があります。previous_response_id でプロバイダーが保存した応答の文脈を参照できます。Agena の履歴は自動で付加されません。通常の文字からの生成は chatgpt.image_generate、ローカル画像の編集は chatgpt.image_edit を優先してください。現在の接続先は Responses と画像ツールに対応している必要があります。"
            ),
            locale(
                "ko-KR",
                summary = "추가 Responses 대화 모델이 GPT Image를 호출합니다. model은 대화 모델, tool_options.model은 그림 모델을 지정합니다.",
                help = "추가 POST responses 요청을 보냅니다. 현재 Agena AI -> Responses 대화 모델 -> 호스팅된 GPT Image 도구 -> 이미지 순서로 처리합니다. model은 대화 모델, tool_options.model은 gpt-image-2 같은 그림 모델을 선택합니다. 대화 모델이 그림 프롬프트를 다시 작성할 수 있습니다. previous_response_id는 제공자가 보관한 이전 응답의 문맥을 참조하며 Agena 대화 기록은 자동으로 첨부되지 않습니다. 일반 텍스트 이미지 생성에는 chatgpt.image_generate, 로컬 이미지 편집에는 chatgpt.image_edit를 우선 사용하세요. 현재 제공자 연결이 Responses와 이미지 도구를 지원해야 합니다."
            ),
            locale(
                "pt-BR",
                summary = "Um modelo de chat Responses adicional chama GPT Image; model escolhe o chat e tool_options.model o modelo de desenho.",
                help = "Envia uma solicitação POST responses adicional: IA atual do Agena -> modelo de chat Responses -> ferramenta GPT Image hospedada -> imagem. model seleciona o modelo de chat e tool_options.model o de desenho, por exemplo gpt-image-2. O chat pode reformular o prompt da imagem. previous_response_id reutiliza o contexto de uma resposta mantida pelo provedor; o histórico do Agena não é anexado automaticamente. Prefira chatgpt.image_generate para texto para imagem e chatgpt.image_edit para imagens locais. Reutiliza a conexão atual, que deve oferecer Responses e sua ferramenta de imagem."
            ),
            locale(
                "zh-CN",
                summary = "额外调用 OpenAI Responses 对话模型，再由它调用 GPT Image；model 选对话模型，tool_options.model 选绘图模型。",
                help = "需要 Responses 服务端保存的绘图或编辑上下文时使用。此工具会额外提交 POST responses 请求，链路为：当前 Agena AI -> Responses 对话模型 -> 托管 image_generation 工具 / GPT Image 绘图模型 -> 图片。Responses 对话模型理解要求，可能改写绘图提示词，再调用绘图模型实际生成或编辑图片。顶层 model 指定这个额外的对话模型，省略时默认使用当前会话模型，须支持 Responses 绘图工具；不兼容时显式传入 model 选择合适的对话模型。tool_options.model 单独指定绘图模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，具体取决于当前 provider 支持。GPT Image 模型 ID 应填在 tool_options.model。prompt 可直接写纯文本。参考图通过 input_items 提供受支持的图片 URL、base64 data URL 或文件 ID，本地图片需先上传或编码。previous_response_id 引用 provider 保留的上一轮 OpenAI 响应上下文，它不是 Agena 会话 ID。tool_options 可传 action（auto/generate/edit）、size、quality、output_format、background 等所选模型支持的参数。普通文生图优先使用 chatgpt.image_generate；本地图片编辑或参考图生成优先使用 chatgpt.image_edit，继续修改可再次传入上次输出的本地图片路径。复用当前 provider 的地址、认证和网络设置，须支持 Responses 及其绘图工具。只发送显式提供的提示词、input_items 和引用的响应上下文，Agena 历史及本地项目文件不会自动附带。返回图片保存为本地托管附件。"
            ),
            locale(
                "zh-TW",
                summary = "額外呼叫 OpenAI Responses 對話模型，再由它呼叫 GPT Image；model 選對話模型，tool_options.model 選繪圖模型。",
                help = "需要 Responses 伺服器儲存的繪圖或編輯上下文時使用。此工具會額外提交 POST responses 請求，流程為：目前 Agena AI -> Responses 對話模型 -> 託管 image_generation 工具 / GPT Image 繪圖模型 -> 圖片。Responses 對話模型理解要求，可能改寫繪圖提示詞，再呼叫繪圖模型實際產生或編輯圖片。頂層 model 指定這個額外的對話模型，省略時預設目前工作階段模型，須支援 Responses 繪圖工具；不相容時明確選擇合適的對話模型。tool_options.model 單獨指定繪圖模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，依目前 provider 支援情況而定。GPT Image 模型 ID 應填在 tool_options.model。prompt 可直接使用純文字。參考圖透過 input_items 提供支援的圖片 URL、base64 data URL 或檔案 ID，本機圖片須先上傳或編碼。previous_response_id 引用 provider 保留的上一輪 OpenAI 回應上下文，它不是 Agena 工作階段 ID。tool_options 可傳 action（auto/generate/edit）、size、quality、output_format、background 等所選模型支援的參數。一般文字繪圖優先使用 chatgpt.image_generate；本機圖片編輯或參考圖生成優先使用 chatgpt.image_edit，繼續修改時可再次傳入上次輸出的本機圖片路徑。沿用目前 provider 的位址、認證及網路設定，須支援 Responses 及其繪圖工具。僅傳送明確提供的提示詞、input_items 與引用的回應上下文，Agena 歷史及本機專案檔案不會自動附帶。回傳圖片儲存為本機託管附件。"
            )
        )
    )]
    async fn responses_image_generation(
        &self,
        input: ChatGptResponsesImageInput,
    ) -> SdkResult<ToolInvokeOutput> {
        self.responses_tool(
            // The public tool name is separate from OpenAI's hosted operation.
            "image_generation",
            "ChatGPT image generation via Responses",
            serde_json::json!({"type":"image_generation"}),
            input.into(),
        )
        .await
    }

    #[tool(
        name = "shell",
        stream = shell_stream,
        tags(network, interactive, mutate),
        summary = "Requires an OpenAI model; defaults to the current model or accepts model override. Run shell commands in an OpenAI cloud container, never in the local terminal.",
        help = "Model-based tools default to the current session model. Use an OpenAI model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in OpenAI cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Defaults to container_auto. Only container_auto or container_reference with container_id is accepted. Local/custom environments and client callbacks are rejected. Uploaded provider files are separate from Agena local files; there is no local execution fallback.",
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
        name = "image_edit",
        tags(mutate),
        summary = "Edit local images or compose from references directly with the OpenAI Images API; model selects a GPT Image drawing model.",
        help = "Prefer this for editing local images or composing from reference images. Sends POST images/edits directly to a GPT Image drawing model. Supply prompt as natural-language edit instructions and images as one to sixteen permitted local image paths; this tool reads and uploads them directly. To select an OpenAI GPT Image model supported by the current provider, explicitly pass model, for example gpt-image-2, gpt-image-2.5-sunburst, or gpt-image-2.5-flare. These are examples, not an exhaustive list. Here model is the drawing model. It defaults to the current session model when omitted, so explicitly select a drawing model when the session uses a chat model. Example: {\"model\":\"gpt-image-2\",\"prompt\":\"Make the scarf red; preserve the cat's face and composition\",\"images\":[\"./cat.png\"]}. options accepts model-supported size, quality, n, output_format, and other Images edit fields. For another edit, pass the previous output's local image path back in images with the next prompt. Use chatgpt.image_generate for text-only generation; use chatgpt.responses_image_generation when you need Responses server-side response context. Reuses the current provider endpoint, credentials and network settings; the provider or gateway must support Images edits and the selected model. Only supplied images and prompt are uploaded; local paths are permission checked. Returned images are saved as separate managed local attachments, preserving the originals.",
        translations(
            locale(
                "ar-SA",
                summary = "عدّل صورًا محلية أو استخدم مراجع مباشرة عبر OpenAI Images API؛ model يحدد نموذج الرسم GPT Image.",
                help = "يرسل POST images/edits مباشرة إلى نموذج الرسم. ضع تعليمات التعديل في prompt ومسارات من صورة إلى 16 صورة محلية في images. يحدد model نموذج GPT Image مثل gpt-image-2؛ حدده صراحةً إذا كان نموذج الجلسة للمحادثة. لمتابعة التعديل، أعد تمرير مسار الصورة الناتجة في images. تُستخدم وصلة المزود الحالية وتُحفظ النتائج كمرفقات محلية مستقلة."
            ),
            locale(
                "de-DE",
                summary = "Lokale Bilder direkt über die OpenAI Images API bearbeiten oder als Vorlagen nutzen; model wählt das GPT-Image-Zeichenmodell.",
                help = "Sendet POST images/edits direkt an das Zeichenmodell. prompt enthält die Bearbeitungsanweisung, images ein bis sechzehn erlaubte lokale Bildpfade. model wählt ein GPT-Image-Modell wie gpt-image-2; bei einem Chat-Sitzungsmodell ausdrücklich setzen. Für weitere Änderungen den lokalen Ausgabepfad erneut in images übergeben. Verwendet die aktuelle Anbieterverbindung und speichert Ergebnisse als separate lokale Anhänge."
            ),
            locale(
                "es-ES",
                summary = "Edita imágenes locales o usa referencias directamente con OpenAI Images API; model selecciona el modelo de dibujo GPT Image.",
                help = "Envía POST images/edits directamente al modelo de dibujo. prompt contiene las instrucciones e images de una a dieciséis rutas locales permitidas. model selecciona un modelo GPT Image como gpt-image-2; indíquelo explícitamente si la sesión usa un modelo de chat. Para otra edición, pase la ruta local del resultado anterior en images. Reutiliza la conexión actual y guarda los resultados como adjuntos locales separados."
            ),
            locale(
                "fr-FR",
                summary = "Modifie des images locales ou utilise des références via OpenAI Images API ; model choisit le modèle de dessin GPT Image.",
                help = "Envoie POST images/edits directement au modèle de dessin. prompt contient les consignes et images un à seize chemins locaux autorisés. model choisit un modèle GPT Image, par exemple gpt-image-2 ; le préciser si la session utilise un modèle de chat. Pour une nouvelle retouche, fournir dans images le chemin local du résultat précédent. Utilise la connexion actuelle et enregistre les résultats dans des pièces jointes locales distinctes."
            ),
            locale(
                "hi-IN",
                summary = "OpenAI Images API से स्थानीय चित्र सीधे संपादित करें या संदर्भ लें; model GPT Image चित्र मॉडल चुनता है।",
                help = "POST images/edits सीधे चित्र मॉडल को भेजता है। prompt में संपादन निर्देश और images में एक से सोलह अनुमत स्थानीय चित्र पथ दें। model से gpt-image-2 जैसा GPT Image मॉडल चुनें; सत्र में चैट मॉडल होने पर इसे स्पष्ट रूप से दें। अगला संपादन करने के लिए पिछले परिणाम का स्थानीय पथ फिर images में दें। वर्तमान प्रदाता कनेक्शन का उपयोग होता है और परिणाम अलग स्थानीय अनुलग्नकों में सहेजे जाते हैं।"
            ),
            locale(
                "ja-JP",
                summary = "OpenAI Images API でローカル画像を直接編集したり参照画像から生成します。model は GPT Image 描画モデルです。",
                help = "POST images/edits を描画モデルへ直接送信します。prompt に編集指示、images に読み取りを許可されたローカル画像のパスを 1〜16 個指定します。model は gpt-image-2 などの GPT Image モデルです。会話モデルを使うセッションでは明示してください。再編集では前回の出力画像のローカルパスを images に渡せます。現在の接続を使い、結果を別のローカル添付として保存します。"
            ),
            locale(
                "ko-KR",
                summary = "OpenAI Images API로 로컬 이미지를 직접 편집하거나 참조 이미지를 사용합니다. model은 GPT Image 그림 모델입니다.",
                help = "POST images/edits를 그림 모델에 직접 보냅니다. prompt에 편집 지시를, images에 읽기가 허용된 로컬 이미지 경로 1~16개를 넣으세요. model은 gpt-image-2 같은 GPT Image 모델이며 대화 모델을 쓰는 세션에서는 명시해야 합니다. 추가 편집 시 이전 결과의 로컬 경로를 images에 다시 전달할 수 있습니다. 현재 제공자 연결을 사용하며 결과는 별도의 로컬 첨부로 저장합니다."
            ),
            locale(
                "pt-BR",
                summary = "Edita imagens locais ou usa referências diretamente com OpenAI Images API; model seleciona o modelo de desenho GPT Image.",
                help = "Envia POST images/edits diretamente ao modelo de desenho. prompt contém as instruções e images de um a dezesseis caminhos locais permitidos. model seleciona um modelo GPT Image, como gpt-image-2; informe-o explicitamente se a sessão usa um modelo de chat. Para outra edição, passe o caminho local do resultado anterior em images. Reutiliza a conexão atual e salva os resultados como anexos locais separados."
            ),
            locale(
                "zh-CN",
                summary = "通过 OpenAI Images API 直接编辑本地图片或使用参考图；model 指定 GPT Image 绘图模型。",
                help = "本地图片编辑或参考图生成优先使用此工具。通过 POST images/edits 直接调用 GPT Image 绘图模型。prompt 写自然语言编辑要求及需要保留的细节，images 传 1 至 16 个获准读取的本地图片路径，工具会直接读取并上传。显式传入 model 选择当前 provider 支持的 OpenAI 绘图模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，也可使用该 provider 支持的其他 GPT Image 模型 ID。这里的 model 是绘图模型；省略时默认使用当前会话模型，因此当前为聊天模型时应显式指定绘图模型。例如 {\"model\":\"gpt-image-2\",\"prompt\":\"将围巾改成红色，保留猫的面部和构图\",\"images\":[\"./cat.png\"]}。options 可传所选模型支持的 size、quality、n、output_format 等编辑参数。继续修改时，将上次输出的本地图片路径再次传入 images 并提供新的 prompt。只有文字要求时使用 chatgpt.image_generate；需要引用 Responses 服务端保留的响应上下文时使用 chatgpt.responses_image_generation。复用当前 provider 的地址、认证和网络设置，须支持 Images edits 和所选模型。只上传显式指定的图片和提示词；路径经过权限检查，结果另存为本地托管附件，保留原图。"
            ),
            locale(
                "zh-TW",
                summary = "透過 OpenAI Images API 直接編輯本機圖片或使用參考圖；model 指定 GPT Image 繪圖模型。",
                help = "本機圖片編輯或參考圖生成優先使用此工具。透過 POST images/edits 直接呼叫 GPT Image 繪圖模型。prompt 填入自然語言編輯要求與需保留的細節，images 傳入 1 至 16 個獲准讀取的本機圖片路徑，工具會直接讀取並上傳。明確傳入 model 選擇目前 provider 支援的繪圖模型，例如 gpt-image-2、gpt-image-2.5-sunburst、gpt-image-2.5-flare，也可使用該 provider 支援的其他 GPT Image 模型 ID。此處 model 是繪圖模型；省略時預設使用目前工作階段模型，因此目前為聊天模型時應明確指定繪圖模型。options 可傳所選模型支援的 size、quality、n、output_format 等編輯參數。繼續修改時，將上次輸出的本機圖片路徑再次傳入 images 並提供新的 prompt。只有文字要求時使用 chatgpt.image_generate；需要引用 Responses 伺服器保留的回應上下文時使用 chatgpt.responses_image_generation。沿用目前 provider 的位址、認證及網路設定，須支援 Images edits 和所選模型。僅上傳明確指定的圖片與提示詞；路徑經過權限檢查，結果另存為本機託管附件，保留原圖。"
            )
        )
    )]
    async fn image_edit(&self, input: ChatGptImageEditInput) -> SdkResult<ToolInvokeOutput> {
        super::official_service::hosted::validate_options(&input.options, "options")?;
        let connection = self.host()?.builtin_provider_connection().await?;
        let model = configured_model(
            input.model,
            connection.model.as_deref(),
            "chatgpt.image_edit",
        )?;
        let url = endpoint(&connection.base_url, "images/edits")?;
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
            let bytes =
                read_image_input_bounded(self.host()?, &path, &mut image_input_bytes, "OpenAI")
                    .await?;
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
        let response = super::official_service::provider_http_client(&connection)?
            .post(url)
            .headers(super::official_service::provider_headers(&connection)?)
            .multipart(form)
            .send()
            .await
            .map_err(|error| {
                super::official_service::provider_transport_error(
                    "OpenAI",
                    "image edit",
                    &error,
                    Some(&connection),
                )
                .with_remote_effect_unknown()
            })?;
        let (status, request_id, value) =
            read_json_response_bounded(response, "OpenAI", "image edit", &connection.headers)
                .await
                .map_err(PluginError::with_remote_effect_unknown)?;
        if !status.is_success() {
            let error = super::official_service::provider_http_error(
                "OpenAI",
                "image edit",
                status,
                request_id.as_deref(),
                &value,
                &connection.headers,
            );
            return Err(
                if status.is_server_error() || status == reqwest::StatusCode::REQUEST_TIMEOUT {
                    error.with_remote_effect_unknown()
                } else {
                    error
                },
            );
        }
        provider_output(
            self.host()?,
            self.workspace_root()?,
            "image_edit",
            model.as_str(),
            "ChatGPT image edit",
            ProviderUsageKind::OpenAiImage,
            ProviderHttpResponse { value, request_id },
            &connection,
        )
        .await
    }
}
