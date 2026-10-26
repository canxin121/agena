//! Google cloud inference, search, files, knowledge stores, images, audio, video, music, Live voice and agent APIs. Inputs leave this computer; no local execution fallback.

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
    ProviderUsageKind, append_prompt_to_items, configured_model, endpoint, merge_object_options,
    post_json, provider_output, read_image_input_bounded, resolve_local_path,
};
use agena_plugin_host::sdk::ToolInvokeContext;

pub(crate) const GEMINI_PLUGIN_ID: &str = "agena.gemini";

pub(crate) struct GeminiToolsPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
    workspace_root: OnceLock<PathBuf>,
    config: OnceLock<GeminiToolsConfig>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
struct GeminiToolsConfig {
    stable_system_instruction: Option<String>,
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
    /// Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.
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
    /// Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.
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
    /// Optional model override on the current provider; defaults to the current session model. Must be a Google Gemini model that supports this operation. Pass model explicitly if the current model is unsuitable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    model: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    generation_config: BTreeMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_content: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    request_options: BTreeMap<String, serde_json::Value>,
}

struct GenerateContentRequest {
    model: String,
    parts: Vec<serde_json::Value>,
    generation_config: BTreeMap<String, serde_json::Value>,
    cached_content: Option<String>,
    request_options: BTreeMap<String, serde_json::Value>,
}

impl GeminiToolsPlugin {
    async fn media_service(&self) -> SdkResult<Service<'_>> {
        Service::new(
            "gemini",
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
        let connection = self.host()?.builtin_provider_connection().await?;
        let model = configured_model(
            input.model,
            connection.model.as_deref(),
            &format!("gemini.{tool_name}"),
        )?;
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
        let url = endpoint(&connection.base_url, "interactions")?;
        let headers = connection.headers.clone();
        let response = post_json(
            self.host()?,
            url.as_str(),
            &headers,
            body,
            &connection,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiInteractions,
            response,
            &connection,
        )
        .await
    }

    async fn generate_content(
        &self,
        connection: agena_plugin_sdk::host_api::BuiltinProviderConnection,
        tool_name: &str,
        title: &str,
        request: GenerateContentRequest,
    ) -> SdkResult<ToolInvokeOutput> {
        let GenerateContentRequest {
            model,
            parts,
            mut generation_config,
            cached_content,
            request_options,
        } = request;
        super::official_service::hosted::validate_options(&request_options, "request_options")?;
        super::official_service::hosted::validate_options(&generation_config, "generation_config")?;
        generation_config
            .entry("responseModalities".to_owned())
            .or_insert_with(|| serde_json::json!(["TEXT", "IMAGE"]));
        let url = super::official_service::gemini_model_endpoint(&connection.base_url, &model)?;
        let headers = connection.headers.clone();
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
            &connection,
            "gemini",
            tool_name,
        )
        .await?;
        provider_output(
            self.host()?,
            self.workspace_root()?,
            tool_name,
            model.as_str(),
            title,
            ProviderUsageKind::GeminiGenerateContent,
            response,
            &connection,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "gemini",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Google cloud inference, search, files, knowledge stores, images, audio, video, music, Live voice and agent APIs. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "需要模型的工具默认使用当前会话模型。当前模型须为支持该操作的 Google Gemini 模型，否则必须显式传入 model 指定合适模型。复用当前会话的 provider 连接，无需单独配置。使用 Google 云端推理、搜索、文件、知识库、图像、音频、视频、音乐、Live 语音与智能体 API。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "需要模型的工具預設使用目前工作階段模型。目前模型須為支援該操作的 Google Gemini 模型，否則必須明確傳入 model 指定合適模型。沿用目前工作階段的 provider 連線，無需另外設定。使用 Google 雲端搜尋、運算與影像功能。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "モデルが必要なツールは現在のセッションモデルを使用します。対象操作に対応する Google Gemini モデルが必要です。現在のモデルが適合しない場合は model を明示してください。現在のセッションのプロバイダー接続を使用します。Google のクラウド検索・計算・画像機能を利用します。入力は端末外へ送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "모델이 필요한 도구는 현재 세션 모델을 기본으로 사용합니다. 작업을 지원하는 Google Gemini 모델이 필요하며, 현재 모델이 적합하지 않으면 model을 명시하세요. 현재 세션의 공급자 연결을 사용합니다. Google 클라우드 검색, 계산, 이미지 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Les outils nécessitant un modèle utilisent celui de la session. Choisissez un modèle Google Gemini compatible avec l’opération ; sinon, renseignez explicitement model. Utilise la connexion du fournisseur de la session courante. Utiliser les fonctions de recherche, de calcul et d’image dans le cloud Google. Les données sont envoyées hors de cet ordinateur, sans repli local."),
        locale("de-DE", summary = "Modellbasierte Werkzeuge verwenden das aktuelle Sitzungsmodell. Ein für die Aktion geeignetes Google Gemini-Modell ist erforderlich; andernfalls model explizit angeben. Verwendet die Provider-Verbindung der aktuellen Sitzung. Google-Cloudfunktionen für Suche, Berechnungen und Bilder nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Las herramientas que requieren modelo usan el de la sesión. Se necesita un modelo de Google Gemini compatible con la operación; de lo contrario, indica model explícitamente. Usa la conexión del proveedor de la sesión actual. Usa las funciones de búsqueda, cálculo e imagen en la nube de Google. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "मॉडल वाले टूल वर्तमान सत्र का मॉडल इस्तेमाल करते हैं। इस कार्य का समर्थन करने वाला Google Gemini मॉडल चाहिए; अन्यथा model स्पष्ट रूप से दें। वर्तमान सत्र के प्रदाता कनेक्शन का उपयोग करता है। Google क्लाउड की खोज, गणना और छवि सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "تستخدم الأدوات التي تحتاج نموذجًا نموذج الجلسة الحالي. يجب استخدام نموذج من Google Gemini يدعم العملية؛ وإلا فحدّد model صراحةً. يستخدم اتصال المزوّد للجلسة الحالية. استخدم إمكانات البحث والحوسبة والصور في سحابة Google. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Ferramentas que exigem modelo usam o da sessão. É necessário um modelo da Google Gemini compatível com a operação; caso contrário, informe model explicitamente. Usa a conexão do provedor da sessão atual. Use os recursos de busca, computação e imagem na nuvem do Google. As entradas saem deste computador; não há execução local alternativa.")
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
        name = "files",
        tags(network, mutate),
        summary = "Manage remote files and download cloud-generated artifacts.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.",
        translations(locale(
            "zh-CN",
            summary = "管理远程文件，下载厂商生成的文件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。支持 list、get、wait、delete、download。OpenAI/Claude 还支持 upload：path 指定本地文件，parameters.purpose 按厂商要求填写。Gemini 使用 file_upload 上传。download 将实际字节保存为附件或新的 output_path；Claude 仅支持下载厂商生成的文件。id 属于当前 provider，列表与管理接口不需要模型。"
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
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.",
        translations(locale(
            "zh-CN",
            summary = "使用厂商原生接口组合工具、推理并继续对话。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create；prompt 或 parameters 中填写厂商原生 input/messages，可组合托管工具、远程 MCP、工具搜索、编程式调用、云端 Skills 或客户端工具声明。返回的工具调用是数据，不会自动执行本机操作。需要本地执行时，使用正常 Agena 工具及其权限，再显式提交厂商格式的调用结果。顶层 stream=true 可接收限时 SSE，wait_seconds 为 1..50，保留完整事件与 SSE 游标。"
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
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Google Gemini model supporting generation or embedding batches, or explicitly pass model. Actions: create/list/get/wait/cancel/delete/update. resource=generate_content (default) creates batchGenerateContent; resource=embeddings creates asyncBatchEmbedContent. parameters uses the official batch body. update calls updateGenerateContentBatch or updateEmbedContentBatch, with id=batches/ID, top-level model and the official update fields; query.updateMask selects fields. Get/wait/cancel/delete need only id. Results may be inline or downloaded using files.download. wait_seconds is 1..50; waits never resubmit jobs.",
        translations(locale(
            "zh-CN",
            summary = "创建、查询、取消异步批处理并获取结果。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。支持 create/list/get/wait/cancel/delete/update。resource=generate_content（默认）创建生成批次，resource=embeddings 创建异步嵌入批次。parameters 使用官方请求体。update 按 resource 调用对应批次更新接口，id 为 batches/ID，模型用顶层 model，更新字段放 parameters，可传 query.updateMask。查询、等待、取消、删除无需模型。输出文件用 files.download 获取，wait 不会重建任务。"
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
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.",
        translations(locale(
            "zh-CN",
            summary = "通过厂商接口计算输入 token 数量。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create；parameters 使用厂商的 input/messages/contents 格式。只计算输入 token，不生成文本。"
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
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions depend on the vendor resource: create, list, get, update, delete, wait, archive, events, create_event, items, list_versions, get_version, run, pause, unpause. Unsupported actions are rejected per collection. resource selects the documented collection. OpenAI: agents, sessions (default), environments. Claude: agents (default), sessions, environments, deployments. Gemini: agents. parameters/query carry official fields, including nested agent.model when required. Runs on vendor infrastructure; local files and tools are not automatically available. Inference on a Gemini agent uses inference with parameters.agent and no model override. Provider support and access are required.",
        translations(locale(
            "zh-CN",
            summary = "管理厂商托管的智能体、会话与环境。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。resource 选择资源：OpenAI 为 agents、sessions（默认）、environments；Claude 为 agents（默认）、sessions、environments、deployments；Gemini 为 agents。操作因资源而异，不支持的 action 会在本地拒绝。会话可用 events/create_event；OpenAI 还支持 items，Claude agents 支持 list_versions/get_version。parameters/query 使用官方字段；创建内联 agent 配置时默认沿用当前模型，可用顶层 model 覆盖。运行已有 Gemini agent 请用 inference 的 parameters.agent，不传顶层 model。托管环境不会自动取得本地文件或工具。"
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
        summary = "Create, populate and manage Gemini File Search Stores.",
        help = "Uses the current provider connection. Model-based tools default to the current session model; use a compatible Google Gemini model or explicitly pass model. Resource management needs no model. Actions: create/list/get/delete/import_file/upload/list_files/get_file/delete_file. id is store ID or fileSearchStores/ID; item_id is document ID. upload directly sends one authorized path into the store using resumable upload; parameters supports displayName/mimeType/customMetadata/chunkingConfig. Alternatively file_upload then import_file with parameters.fileName=files/ID. Poll returned operations with operations.get/wait, including fileSearchStores/ID/upload/operations/ID. Upload readiness is different from indexing completion. Use file_search with the resulting store name. Delete query.force=true explicitly deletes a nonempty store.",
        translations(locale(
            "zh-CN",
            summary = "创建、导入和管理厂商知识库。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。资源管理无需模型。支持 create/list/get/delete/import_file/upload/list_files/get_file/delete_file。id 为 Store，item_id 为文档；upload 将 path 指定的单个获准文件直接上传并索引，可传 displayName/mimeType/customMetadata/chunkingConfig。也可先 file_upload，再 import_file。两种流程返回的 operation 均可用 operations.get/wait 查询，包括含 upload/operations 的路径。上传就绪不等于索引完成。"
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
        name = "operations",
        tags(network, mutate),
        summary = "Inspect or wait for Gemini file-import operations.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: get, wait. id is operations/ID or fileSearchStores/ID/operations/ID. wait_seconds is 1..50. Returns done/error/response without creating another import.",
        translations(locale(
            "zh-CN",
            summary = "查询或等待 Gemini 长任务。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。支持 get/wait；id 为 operations/ID、fileSearchStores/ID/operations/ID 或 files/ID/operations/ID。wait_seconds 为 1..50，返回 done/error/response，不会重新导入文件。"
        ))
    )]
    async fn api_operations(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Operations,
            input,
        )
        .await
    }

    #[tool(
        name = "interactions",
        tags(network, mutate),
        summary = "Retrieve, wait for, cancel or delete Gemini background interactions.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, get, wait, cancel, delete. create accepts prompt or parameters.input and background=true. For Deep Research/managed agents use parameters.agent instead of a model; supplying both is rejected. id is an existing interaction ID for get/wait/cancel/delete. Polling does not create a new interaction.",
        translations(locale(
            "zh-CN",
            summary = "创建和管理 Gemini 后台交互与深度研究任务。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。支持 create/get/wait/cancel/delete；创建可用 prompt 或 parameters.input，以及 parameters.background=true。Deep Research 或托管智能体使用 parameters.agent，不传顶层 model，两者同时传入会拒绝。已有任务通过 id 查询、等待、取消或删除。"
        ))
    )]
    async fn api_interactions(
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
        name = "speech",
        tags(network, mutate),
        summary = "Generate Gemini text-to-speech audio as local attachments.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. prompt is the text to speak. Use a Gemini TTS model and parameters.generation_config.speech_config for single/multiple speakers and voices. Advanced input with speech_metadata belongs in parameters.input. Agena sets response_format.type=audio. Audio-only output is saved; output_path optionally selects a new local file.",
        translations(locale(
            "zh-CN",
            summary = "生成语音并保存为本地音频附件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create，prompt 为待朗读文本。OpenAI 使用 parameters.voice 等官方语音字段。Gemini 使用 parameters.generation_config.speech_config，复杂输入和 speech_metadata 放入 parameters.input；自动设置 response_format.type=audio。必须使用支持语音合成的模型。output_path 可指定新的输出文件。"
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
        name = "video_generation",
        tags(network, mutate),
        summary = "Generate a video using the Gemini Interactions API.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Use a supported Gemini video generation model, such as the current Omni family, not a chat-only model. prompt or parameters.input supplies text/images/audio/video. parameters.response_format supplies official video settings such as aspect_ratio and resolution. Supports background execution; inspect the returned interaction ID with interactions.get/wait. Inline video becomes an attachment; remote file references can be retrieved using files.download. Do not use retired Veo preview aliases.",
        translations(locale(
            "zh-CN",
            summary = "通过 Gemini Interactions 接口生成视频。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create，需选择支持视频生成的模型。prompt 或 parameters.input 指定文本、图像、音频或视频输入；parameters.response_format 控制 aspect_ratio、resolution 等，自动设置 type=video。后台任务用 interactions.get/wait 查询；内联视频保存为附件，远程文件用 files.download 下载。"
        ))
    )]
    async fn api_video_generation(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Video,
            input,
        )
        .await
    }

    #[tool(
        name = "video_edit",
        tags(network, mutate),
        summary = "Edit or extend a video using Gemini multimodal interactions.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Select a supported video editing model. parameters.input contains the explicit video/media inputs and instructions, or use prompt with parameters.previous_interaction_id for conversational editing. parameters.response_format contains official video settings. Use interactions.get/wait for background work. Inline video becomes an attachment.",
        translations(locale(
            "zh-CN",
            summary = "通过 Gemini 多模态交互编辑或扩展视频。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create，需支持视频编辑的模型。parameters.input 传视频及编辑要求，或用 prompt 配合 parameters.previous_interaction_id 继续编辑。parameters.response_format 控制视频格式。后台任务通过 interactions 查询，视频结果保存为附件。"
        ))
    )]
    async fn api_video_edit(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Video,
            input,
        )
        .await
    }

    #[tool(
        name = "music_generation",
        tags(network, mutate),
        summary = "Generate music using Gemini Lyria and save audio attachments.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Select an available Lyria music model. prompt or parameters.input describes the music and optional image inputs; parameters.response_format contains supported audio settings. Returned lyrics and metadata remain in the receipt; inline audio becomes a local attachment. This is not a speech or ordinary chat model operation.",
        translations(locale(
            "zh-CN",
            summary = "使用 Gemini Lyria 生成音乐并保存音频。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。默认 create，需可用的 Lyria 音乐模型。prompt 或 parameters.input 描述音乐与可选图像，parameters.response_format 传音频设置，自动设置 type=audio。歌词与元数据保留在响应文件中，内联音频保存为本地附件。"
        ))
    )]
    async fn api_music_generation(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Music,
            input,
        )
        .await
    }

    #[tool(
        name = "embeddings",
        tags(network, mutate),
        summary = "Create Gemini text or multimodal embeddings.",
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Google Gemini embedding model or explicitly pass model. create uses models/{model}:embedContent with prompt or parameters.content and optional taskType/title/outputDimensionality. create_batch uses models/{model}:batchEmbedContents with parameters.requests array; each request inherits the selected model if absent, and an explicit top-level model overrides entry models. Async embedding batches use batches with resource=embeddings. Large vectors are retained in the receipt.",
        translations(locale(
            "zh-CN",
            summary = "生成文本或多模态嵌入向量。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。需要 Google 嵌入模型。create 使用 embedContent，传 prompt 或 parameters.content；create_batch 使用 batchEmbedContents，传 parameters.requests 数组，条目缺少模型时补入选定模型，显式顶层 model 会覆盖条目模型。异步批量嵌入使用 batches 的 resource=embeddings。向量保存在完整响应文件。"
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
        name = "caches",
        tags(network, mutate),
        summary = "Manage explicit Gemini context caches for compatible models.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, update, delete. create uses the current session model unless model is supplied; parameters contains contents, ttl/expireTime and other official fields. update changes supported lifecycle fields using query.updateMask. id accepts cachedContents/ID. Explicit caching is model-dependent; models with implicit caching only reject this API.",
        translations(locale(
            "zh-CN",
            summary = "管理 Gemini 显式上下文缓存。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Google Gemini 模型，否则显式传入 model。资源管理操作无需模型。支持 create/list/get/update/delete。create 使用支持显式缓存的模型与官方 contents/systemInstruction/tools/ttl/expireTime 等字段。update 用于更新生命周期；id 为 cachedContents/ID 或 ID。并非所有 Gemini 模型支持显式缓存。"
        ))
    )]
    async fn api_caches(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Caches,
            input,
        )
        .await
    }

    #[tool(
        name = "audio_understanding",
        tags(network),
        summary = "Analyze audio with a selected Google Gemini model.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting audio input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.",
        translations(locale(
            "zh-CN",
            summary = "使用指定模型理解音频。",
            help = "默认使用当前会话模型；必须为支持该媒体的 Google Gemini 模型，否则显式传入 model。复用当前 provider 连接。仅发送 inputs 显式指定并获准读取的文件；Gemini 可用本会话上传句柄，OpenAI 音频需本地 WAV/MP3。"
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
            "gemini.audio_understanding",
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
        name = "video_understanding",
        tags(network),
        summary = "Analyze video with a selected Google Gemini model.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model supporting video input; otherwise explicitly pass model. Uses only the current provider connection. inputs accepts explicit permission-checked local files or same-session file_upload handles. OpenAI audio requires local WAV/MP3. Gemini also accepts its uploaded audio/video files. Does not transcribe through a local service or upload the workspace implicitly.",
        translations(locale(
            "zh-CN",
            summary = "使用指定模型理解视频。",
            help = "默认使用当前会话模型；必须为支持该媒体的 Google Gemini 模型，否则显式传入 model。复用当前 provider 连接。仅发送 inputs 显式指定并获准读取的文件；Gemini 可用本会话上传句柄，OpenAI 音频需本地 WAV/MP3。"
        ))
    )]
    async fn video_understanding(
        &self,
        context: &ToolInvokeContext<'_>,
        input: &AnalyzeInput,
    ) -> SdkResult<ToolInvokeOutput> {
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "gemini.video_understanding",
        )?;
        service
            .analyze_kind(
                context,
                input,
                model,
                Some(agena_domain::AttachmentKind::Video),
            )
            .await
    }

    #[tool(
        name = "realtime",
        tags(network, mutate),
        summary = "Run one bounded Google Gemini realtime voice turn and save audio.",
        help = "Defaults to the current session model. Use a Google Gemini realtime voice model; otherwise explicitly pass model. Reuses the current provider endpoint and credentials. Supply text or an explicit raw 16-bit mono PCM audio_path. Returns transcript, actual WAV audio, completion state and provider events. Does not open a microphone or execute tool calls. duration_seconds is 1..50; partial output is retained on a receive deadline. Requires the gateway to expose the vendor WebSocket API.",
        translations(locale(
            "zh-CN",
            summary = "调用实时语音 API 并保存返回音频。",
            help = "默认使用当前会话模型；须为支持实时语音的 Google Gemini 模型，否则显式传入 model。复用当前 provider 连接。输入为 text 或明确指定的原始 16 位单声道 PCM 文件；返回转录与 WAV 附件，不会打开麦克风或执行工具回调。"
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
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Send explicit images to Google cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "gemini.image_understanding",
        )?;
        service.analyze(context, input, model, true).await
    }

    #[tool(
        name = "document_understanding",
        tags(query, network, mutate),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Send explicit PDF/text documents to Google cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Google. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "gemini.document_understanding",
        )?;
        service.analyze(context, input, model, false).await
    }

    #[tool(
        name = "file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to Google cloud and return a session-owned handle.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Uploads accept arbitrary explicit data files up to 512 MiB and optional SHA256 revision checks; analysis has separate format/size limits. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
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
        self.media_service().await?.upload(context, input).await
    }

    #[tool(
        name = "file_status",
        tags(query, network, mutate),
        summary = "Query the remote status of an owned Google cloud file, not a local path.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
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
        self.media_service()
            .await?
            .file_control(context, input, false)
            .await
    }

    #[tool(
        name = "file_delete",
        tags(mutate, network),
        summary = "Request deletion of an owned file from Google cloud; preserve the local original.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
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
        self.media_service()
            .await?
            .file_control(context, input, true)
            .await
    }
    #[tool(
        name = "code_execution",
        stream = code_execution_stream,
        tags(network, interactive, read_only),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Execute code in Google cloud infrastructure, not on this computer.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses the official Interactions code_execution declaration. Computation executes on Google infrastructure; no returned function call is executed by Agena.",
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
        name = "url_context",
        tags(network, interactive, discovery, read_only),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Retrieve and ground URL content in Google cloud; no local-file access.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses the official url_context tool. Put URLs in the prompt or official request fields.",
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
        name = "google_search",
        tags(network, interactive, discovery, read_only),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Search Google and ground answers in Google cloud, not the local browser.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options.search_types accepts web_search, image_search, and enterprise_web_search.",
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
        name = "file_search",
        tags(network, interactive, discovery, read_only),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Search configured Google cloud file stores, not files on this computer.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Provider file-store identifiers refer to remote resources, not local filesystem paths. tool_options supports file_search_store_names, metadata_filter, and top_k.",
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
        name = "google_maps",
        tags(network, interactive, discovery, read_only),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Query Google Maps data in Google cloud and return grounding sources.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. tool_options supports enable_widget, latitude, and longitude.",
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
        name = "image_generation",
        tags(network, interactive, mutate),
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Generate images in Google cloud; save returned images as local attachments.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses generateContent with responseModalities TEXT and IMAGE. Use the current session model or specify input.model. Inline image data is persisted as managed attachments.",
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
        let connection = self.host()?.builtin_provider_connection().await?;
        let model = configured_model(
            input.model,
            connection.model.as_deref(),
            "gemini.image_generation",
        )?;
        self.generate_content(
            connection,
            "image_generation",
            "Gemini image generation",
            GenerateContentRequest {
                model,
                parts: vec![serde_json::json!({"text":input.prompt})],
                generation_config: input.generation_config,
                cached_content: input.cached_content,
                request_options: input.request_options,
            },
        )
        .await
    }

    #[tool(
        name = "image_edit",
        summary = "Requires a Google Gemini model; defaults to the current model or accepts model override. Upload permitted images for editing in Google cloud; save the returned image separately.",
        help = "Model-based tools default to the current session model. Use a Google Gemini model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Google cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Permission-checked local images are uploaded to Google; returned images are saved as separate local artifacts. Uploads permission-checked local images as inlineData and requests an IMAGE response. Returned images are persisted as managed attachments.",
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
        let connection = self.host()?.builtin_provider_connection().await?;
        let model = configured_model(
            input.model,
            connection.model.as_deref(),
            "gemini.image_edit",
        )?;
        let mut parts = Vec::new();
        let mut image_input_bytes = 0_u64;
        for source in input.images {
            let path = resolve_local_path(self.workspace_root()?, source.as_str())?;
            let bytes =
                read_image_input_bounded(self.host()?, &path, &mut image_input_bytes, "Gemini")
                    .await?;
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
            connection,
            "image_edit",
            "Gemini image edit",
            GenerateContentRequest {
                model,
                parts,
                generation_config: input.generation_config,
                cached_content: input.cached_content,
                request_options: input.request_options,
            },
        )
        .await
    }
}
