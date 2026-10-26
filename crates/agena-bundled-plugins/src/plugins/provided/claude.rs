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
    ProviderUsageKind, configured_model, dedup_strings, endpoint, merge_object_options, post_json,
    provider_output,
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
    anthropic_version: String,
    max_tokens: u32,
    beta_headers: Vec<String>,
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
            anthropic_version: "2023-06-01".to_owned(),
            max_tokens: 4096,
            beta_headers: Vec::new(),
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
    /// Optional model override on the current provider; defaults to the current session model. Must be an Anthropic Claude model that supports this operation. Pass model explicitly if the current model is unsuitable.
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
    async fn media_service(&self) -> SdkResult<Service<'_>> {
        Service::new(
            "claude",
            self.workspace_root()?,
            self.host()?,
            self.host()?.builtin_provider_connection().await?,
            self.config()?.anthropic_version.as_str(),
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
        let connection = self.host()?.builtin_provider_connection().await?;
        let model = configured_model(
            input.model,
            connection.model.as_deref(),
            &format!("claude.{tool_name}"),
        )?;
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
        let url = connection
            .messages_url
            .clone()
            .unwrap_or(endpoint(&connection.base_url, "messages")?);
        let mut headers = connection.headers.clone();
        headers
            .entry("anthropic-version".into())
            .or_insert(self.config()?.anthropic_version.clone());
        let betas = dedup_strings(
            self.config()?
                .beta_headers
                .iter()
                .cloned()
                .chain(
                    connection
                        .headers
                        .get("anthropic-beta")
                        .into_iter()
                        .flat_map(|value| value.split(',').map(str::to_owned)),
                )
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
            &connection,
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
            &connection,
        )
        .await
    }
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "claude",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session’s provider connection. Anthropic cloud inference, search, code execution, files, Skills, batches and managed-agent APIs. Inputs leave this computer; no local execution fallback.",
    translations(
        locale("zh-CN", summary = "需要模型的工具默认使用当前会话模型。当前模型须为支持该操作的 Anthropic Claude 模型，否则必须显式传入 model 指定合适模型。复用当前会话的 provider 连接，无需单独配置。使用 Anthropic 云端搜索、网页抓取、计算和顾问模型。输入会离开本机；不会回退到本地执行。"),
        locale("zh-TW", summary = "需要模型的工具預設使用目前工作階段模型。目前模型須為支援該操作的 Anthropic Claude 模型，否則必須明確傳入 model 指定合適模型。沿用目前工作階段的 provider 連線，無需另外設定。使用 Anthropic 雲端搜尋、網頁擷取、運算與顧問模型。輸入會離開這台電腦；不會改用本機執行。"),
        locale("ja-JP", summary = "モデルが必要なツールは現在のセッションモデルを使用します。対象操作に対応する Anthropic Claude モデルが必要です。現在のモデルが適合しない場合は model を明示してください。現在のセッションのプロバイダー接続を使用します。Anthropic のクラウド検索・取得・計算・アドバイザー機能を利用します。入力は端末外へ送信され、ローカル実行には切り替わりません。"),
        locale("ko-KR", summary = "모델이 필요한 도구는 현재 세션 모델을 기본으로 사용합니다. 작업을 지원하는 Anthropic Claude 모델이 필요하며, 현재 모델이 적합하지 않으면 model을 명시하세요. 현재 세션의 공급자 연결을 사용합니다. Anthropic 클라우드 검색, 가져오기, 계산 및 어드바이저 기능을 사용합니다. 입력은 이 컴퓨터 밖으로 전송되며 로컬 실행으로 대체되지 않습니다."),
        locale("fr-FR", summary = "Les outils nécessitant un modèle utilisent celui de la session. Choisissez un modèle Anthropic Claude compatible avec l’opération ; sinon, renseignez explicitement model. Utilise la connexion du fournisseur de la session courante. Utiliser la recherche, la récupération, le calcul et le modèle conseiller dans le cloud Anthropic. Les données quittent cet ordinateur, sans repli local."),
        locale("de-DE", summary = "Modellbasierte Werkzeuge verwenden das aktuelle Sitzungsmodell. Ein für die Aktion geeignetes Anthropic Claude-Modell ist erforderlich; andernfalls model explizit angeben. Verwendet die Provider-Verbindung der aktuellen Sitzung. Anthropic-Cloudfunktionen für Suche, Abruf, Berechnungen und Beratung nutzen. Eingaben verlassen diesen Rechner; eine lokale Ausführung gibt es nicht als Ausweichlösung."),
        locale("es-ES", summary = "Las herramientas que requieren modelo usan el de la sesión. Se necesita un modelo de Anthropic Claude compatible con la operación; de lo contrario, indica model explícitamente. Usa la conexión del proveedor de la sesión actual. Usa las funciones de búsqueda, obtención, cálculo y asesoramiento en la nube de Anthropic. Los datos salen de este equipo y no hay ejecución local alternativa."),
        locale("hi-IN", summary = "मॉडल वाले टूल वर्तमान सत्र का मॉडल इस्तेमाल करते हैं। इस कार्य का समर्थन करने वाला Anthropic Claude मॉडल चाहिए; अन्यथा model स्पष्ट रूप से दें। वर्तमान सत्र के प्रदाता कनेक्शन का उपयोग करता है। Anthropic क्लाउड की खोज, फ़ेच, गणना और सलाहकार सुविधाएँ उपयोग करें। इनपुट इस कंप्यूटर से बाहर भेजे जाते हैं; स्थानीय विकल्प पर स्विच नहीं होता।"),
        locale("ar-SA", summary = "تستخدم الأدوات التي تحتاج نموذجًا نموذج الجلسة الحالي. يجب استخدام نموذج من Anthropic Claude يدعم العملية؛ وإلا فحدّد model صراحةً. يستخدم اتصال المزوّد للجلسة الحالية. استخدم البحث والجلب والحوسبة ونموذج الاستشارة في سحابة Anthropic. تُرسل المدخلات خارج هذا الجهاز ولا يوجد تنفيذ محلي بديل."),
        locale("pt-BR", summary = "Ferramentas que exigem modelo usam o da sessão. É necessário um modelo da Anthropic Claude compatível com a operação; caso contrário, informe model explicitamente. Usa a conexão do provedor da sessão atual. Use os recursos de busca, obtenção, computação e consultoria na nuvem da Anthropic. As entradas saem deste computador; não há execução local alternativa.")
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
        name = "files",
        tags(network, mutate),
        summary = "Manage remote files and download cloud-generated artifacts.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: list, get, wait, delete, download; OpenAI/Claude also upload using path and parameters.purpose where required. For Gemini uploads use file_upload, then import its remote file name into a knowledge store. download saves a managed attachment or a new output_path. Claude only downloads provider-generated files. IDs refer to resources on the current provider.",
        translations(locale(
            "zh-CN",
            summary = "管理远程文件，下载厂商生成的文件。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。支持 list、get、wait、delete、download。OpenAI/Claude 还支持 upload：path 指定本地文件，parameters.purpose 按厂商要求填写。Gemini 使用 file_upload 上传。download 将实际字节保存为附件或新的 output_path；Claude 仅支持下载厂商生成的文件。id 属于当前 provider，列表与管理接口不需要模型。"
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
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply prompt and/or official input/messages in parameters. Supports multiple hosted tools, remote MCP, tool search, programmatic calling, cloud Skills and native client-tool declarations using the vendor schema. Returns provider tool calls and continuation state as DATA; never executes local computer, browser, shell, editor, functions or tool callbacks. Use appropriate Agena tools through their normal permissions, then explicitly submit official tool results in another inference request. Cloud resources and local resources are separate. Do not run provider-returned commands implicitly. Top-level stream=true opts into bounded SSE; wait_seconds is 1..50. Complete events and SSE cursors are retained.",
        translations(locale(
            "zh-CN",
            summary = "使用厂商原生接口组合工具、推理并继续对话。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。默认 create；prompt 或 parameters 中填写厂商原生 input/messages，可组合托管工具、远程 MCP、工具搜索、编程式调用、云端 Skills 或客户端工具声明。返回的工具调用是数据，不会自动执行本机操作。需要本地执行时，使用正常 Agena 工具及其权限，再显式提交厂商格式的调用结果。顶层 stream=true 可接收限时 SSE，wait_seconds 为 1..50，保留完整事件与 SSE 游标。"
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
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, wait, cancel; Claude/Gemini also delete, Claude also results. parameters contains official batch input. OpenAI result output_file_id is downloaded with files.download; Gemini output file names use files.download. Claude results are saved as a JSONL attachment. A wait is bounded and never resubmits a job.",
        translations(locale(
            "zh-CN",
            summary = "创建、查询、取消异步批处理并获取结果。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。支持 create、list、get、wait、cancel；Claude/Gemini 支持 delete，Claude 支持 results 并保存 JSONL。parameters 使用官方批处理格式。Claude requests[].params 未指定 model 时沿用顶层 model 或当前模型；OpenAI 输入 JSONL 中的各行模型由文件内容决定。OpenAI output_file_id 和 Gemini 输出文件可用 files.download 下载。wait 有时限且不会重建任务。"
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
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Action: create. Supply official input/messages/contents in parameters. This performs token counting, not text generation. Use a compatible model from the invoked vendor.",
        translations(locale(
            "zh-CN",
            summary = "通过厂商接口计算输入 token 数量。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。默认 create；parameters 使用厂商的 input/messages/contents 格式。只计算输入 token，不生成文本。"
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
        help = "Uses the current provider connection; no separate credentials or endpoint settings. Model-based requests default to the current session model; use a compatible Anthropic Claude model or explicitly pass model for agent creation. Resource management needs no model. resource=agents (default): create/list/get/update/archive/list_versions/get_version; sessions: create/list/get/wait/update/delete/archive/events/create_event/stream_events; environments: create/list/get/update/delete/archive; deployments: create/list/get/update/archive/run/pause/unpause; deployment_runs: list/get. sessions/resources supports create/list/get/update/delete with id=session and item_id=resource. sessions/threads supports list/get/archive/events/stream_events with id=session and item_id=thread. Event streams retain id/event/retry and accept last_event_id; wait_seconds bounds receipt time. parameters/query/beta_headers follow official schemas. No local tools execute implicitly.",
        translations(locale(
            "zh-CN",
            summary = "管理厂商托管的智能体、会话与环境。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。resource=agents（默认）支持 create/list/get/update/archive/list_versions/get_version；sessions 支持 create/list/get/wait/update/delete/archive/events/create_event/stream_events；environments 支持 create/list/get/update/delete/archive；deployments 支持 create/list/get/update/archive/run/pause/unpause；deployment_runs 支持 list/get。sessions/resources 支持 create/list/get/update/delete，sessions/threads 支持 list/get/archive/events/stream_events；id 为会话，item_id 为资源或 thread。事件流保留 SSE 元信息，可用 last_event_id 续接，wait_seconds 限制等待。官方 beta 特性通过 beta_headers 指定。"
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
        name = "skills",
        tags(network, mutate),
        summary = "Create, inspect and manage Claude cloud Skills and versions.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model supporting this operation; otherwise explicitly pass model. Resource operations do not need a model. Uses the current provider endpoint, credentials and network settings; the gateway must support the API. Actions: create, list, get, delete, create_version, list_versions, get_version, delete_version. create/create_version use path or paths for multipart files; include SKILL.md and supporting files. id is skill ID; item_id is version. Run Skills using inference with parameters.container.skills and the official code_execution tool plus required beta_headers; then download generated file IDs with files.download. These are cloud Skills, separate from local Agena skills.",
        translations(locale(
            "zh-CN",
            summary = "管理 Claude 云端 Skills 及其版本。",
            help = "复用当前 provider 连接，无需单独配置。需要模型的操作默认使用当前会话模型，须为支持该操作的 Anthropic Claude 模型，否则显式传入 model。资源管理操作无需模型。支持 create/list/get/delete/create_version/list_versions/get_version/delete_version。创建使用 path 或 paths 上传 SKILL.md 及依赖文件，相对目录结构会保留。id 为 Skill，item_id 为版本。执行时使用 inference，传 parameters.container.skills、官方 code_execution 工具和需要的 beta_headers；生成的文件用 files.download 下载。这里管理的是 Claude 云端 Skill。"
        ))
    )]
    async fn api_skills(
        &self,
        input: super::provider_api::ResourceInput,
    ) -> SdkResult<ToolInvokeOutput> {
        super::provider_api::invoke(
            self.media_service().await?,
            super::provider_api::Family::Skills,
            input,
        )
        .await
    }

    #[tool(
        name = "image_understanding",
        tags(query, network, mutate),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Send explicit images to Anthropic cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "claude.image_understanding",
        )?;
        service.analyze(context, input, model, true).await
    }

    #[tool(
        name = "document_understanding",
        tags(query, network, mutate),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Send explicit PDF/text documents to Anthropic cloud for understanding; not local file viewing.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Sends only the specified, permission-checked inputs and prompt to Anthropic. Accepts local paths with expected_sha256 or owned file_upload handles. Local preparation is bounded; no automatic whole-workspace or conversation upload. Cloud inference may be billed. Input sent inline is not a separate remote file. Results return input hashes, provider/model and usage. No local execution fallback.",
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
        let service = self.media_service().await?;
        let model = configured_model(
            input.model.clone(),
            service.connection.model.as_deref(),
            "claude.document_understanding",
        )?;
        service.analyze(context, input, model, false).await
    }

    #[tool(
        name = "file_upload",
        tags(mutate, network),
        summary = "Upload one permitted local file to Anthropic cloud and return a session-owned handle.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Creates a remote file; does not analyze it. Inputs up to 20 MiB are content-checked and optionally revision-checked. The handle is bound to this workspace/session/provider connection; arbitrary vendor file IDs cannot be substituted. Local files remain unchanged. A timeout may leave remote acceptance unknown: inspect the returned handle, do not automatically repeat. Query status before using processing files and delete unneeded files explicitly.",
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
        self.media_service().await?.upload(context, input).await
    }

    #[tool(
        name = "file_status",
        tags(query, network, mutate),
        summary = "Query the remote status of an owned Anthropic cloud file, not a local path.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only file_upload handles from the same workspace, session and provider connection. Reports provider readiness/expiry and refreshes the signed local receipt. Does not download file contents or resubmit an unknown upload.",
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
        self.media_service()
            .await?
            .file_control(context, input, false)
            .await
    }

    #[tool(
        name = "file_delete",
        tags(mutate, network),
        summary = "Request deletion of an owned file from Anthropic cloud; preserve the local original.",
        help = "Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends authorized inputs only to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Accepts only session-owned cloud file handles. Deletes the remote resource and records the provider acknowledgement; it does not promise erasure of provider logs/backups. No arbitrary remote IDs or cross-provider deletion. A failed request is not reported as successful cleanup.",
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
        self.media_service()
            .await?
            .file_control(context, input, true)
            .await
    }
    #[tool(
        name = "code_execution",
        stream = code_execution_stream,
        tags(network, interactive, read_only),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Execute code in Anthropic cloud infrastructure, not on this computer.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Cloud filesystem and runtime are separate from the Agena workspace; provide needed input files explicitly. Uses code_execution_20260521 with persistent REPL state. Official allowed_callers, cache_control, defer_loading, and strict fields may be supplied in tool_options.",
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
        name = "web_search",
        tags(network, interactive, discovery, read_only),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Search the web in Anthropic cloud and return sources; not a local browser operation.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_search_20260318. tool_options supports allowed_callers, allowed_domains, blocked_domains, cache_control, defer_loading, max_uses, response_inclusion, strict, and user_location.",
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
        name = "web_fetch",
        tags(network, interactive, discovery, read_only),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Fetch and process web content in Anthropic cloud, not through the local browser.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Uses web_fetch_20260318. tool_options supports allowed/blocked domains, citations, max_content_tokens, max_uses, response_inclusion, strict, and use_cache.",
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
        name = "advisor",
        stream = advisor_stream,
        tags(network, interactive, read_only),
        summary = "Requires an Anthropic Claude model; defaults to the current model or accepts model override. Consult an advisor model in Anthropic cloud using the supplied context.",
        help = "Model-based tools default to the current session model. Use an Anthropic Claude model that supports the requested operation; if the current model is from another vendor or lacks that capability, explicitly pass model. Uses the current session provider’s endpoint, credentials and network settings; the provider or gateway must support this API. Runs in Anthropic cloud, not on this computer. Sends prompts and explicitly supplied inputs to the current session's provider endpoint; local project files are not automatically available. No local execution fallback. Only supplied context is available; the local repository and session transcript are not automatically uploaded. Uses advisor_20260301. Set tool_options.model and optional caching, max_tokens, max_uses, allowed_callers, cache_control, defer_loading, and strict.",
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
