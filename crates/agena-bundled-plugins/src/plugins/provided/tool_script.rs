//! Execute a small JavaScript program against the current session's tool surface.

mod vm;
mod worker;

pub(crate) use worker::try_run_worker;

use std::{
    io::{self, Write},
    sync::{Arc, OnceLock},
};

use agena_macros::ToolInput;
use agena_plugin_host::sdk::{
    InitContext, InitOutcome, Plugin, PluginError, PluginErrorKind, Result as SdkResult,
    ToolInvokeContext, ToolInvokeOutput,
    host_api::{HostClient, ToolDescriptor},
};
use agena_tool::tool_search::{ToolSearchDocument, search_tools};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tokio::sync::Semaphore;
use tokio_util::sync::CancellationToken;

pub(crate) const TOOL_SCRIPT_PLUGIN_ID: &str = "agena.tool_script";
const MAX_SOURCE_BYTES: usize = 64 * 1024;
const MAX_REQUESTS: usize = 50;
const MAX_INTERMEDIATE_BYTES: usize = 256 * 1024;
const MAX_RESULT_BYTES: usize = 16 * 1024;
static WORKERS: Semaphore = Semaphore::const_new(8);

#[derive(Default)]
pub(crate) struct ToolScriptPlugin {
    host: OnceLock<Arc<dyn HostClient>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolInput)]
#[serde(deny_unknown_fields)]
struct RunInput {
    /// JavaScript function body, up to 64 KiB of UTF-8. Use `return` to emit a JSON value.
    #[arg(non_empty, max_chars = 65536)]
    code: String,
}

#[agena_plugin_host::sdk::agena_plugin(
    namespace = "agena",
    name = "tool_script",
    version = env!("CARGO_PKG_VERSION"),
    display_name = "Tool Scripting",
    icon = crate::plugin_metadata::icon(include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/plugin-icons/tool_script.svg"))),
    authors = ["canxin121"],
    tags(agena_plugin_host::sdk::ToolTag::Execute),
    summary = "Compose session tools in a JavaScript program.",
    help = "Use JavaScript to chain tool calls, loop over results, and summarize the outcome in one model turn.",
    translations(locale(
        "zh-CN",
        summary = "用工具脚本串联当前会话的工具调用。",
        help = "在一个模型轮次内，用 JavaScript 串联依赖的工具调用、遍历结果并返回所需的数据。"
    ))
)]
impl ToolScriptPlugin {
    #[hook(
        init,
        summary = "Connect the session tool host",
        help = "Bind the scoped host client for tool discovery and invocation.",
        translations(
            locale(
                "zh-CN",
                summary = "连接会话工具宿主",
                help = "绑定受会话权限约束的宿主客户端，用于查找和调用工具。"
            ),
            locale(
                "zh-TW",
                summary = "連接工作階段工具主機",
                help = "綁定受工作階段權限限制的主機用戶端，用於尋找和呼叫工具。"
            ),
            locale(
                "ja-JP",
                summary = "セッションのツールホストに接続",
                help = "セッションの権限範囲でツールを検索し、呼び出すためのホストクライアントを接続します。"
            ),
            locale(
                "ko-KR",
                summary = "세션 도구 호스트 연결",
                help = "세션 권한 범위에서 도구를 검색하고 호출할 호스트 클라이언트를 연결합니다."
            ),
            locale(
                "fr-FR",
                summary = "Connecter l’hôte des outils de la session",
                help = "Associe le client hôte chargé de rechercher et d’appeler les outils dans le périmètre de la session."
            ),
            locale(
                "de-DE",
                summary = "Tool-Host der Sitzung verbinden",
                help = "Bindet den Host-Client, der Tools im Berechtigungsbereich der Sitzung sucht und aufruft."
            ),
            locale(
                "es-ES",
                summary = "Conectar el host de herramientas de la sesión",
                help = "Vincula el cliente del host para buscar e invocar herramientas con los permisos de la sesión."
            ),
            locale(
                "hi-IN",
                summary = "सत्र के टूल होस्ट से कनेक्ट करें",
                help = "सत्र की अनुमतियों के भीतर टूल खोजने और चलाने के लिए होस्ट क्लाइंट जोड़ता है।"
            ),
            locale(
                "ar-SA",
                summary = "الاتصال بمضيف أدوات الجلسة",
                help = "يربط عميل المضيف للعثور على الأدوات واستدعائها ضمن أذونات الجلسة."
            ),
            locale(
                "pt-BR",
                summary = "Conectar o host de ferramentas da sessão",
                help = "Vincula o cliente do host para buscar e chamar ferramentas com as permissões da sessão."
            )
        )
    )]
    async fn init(&self, _ctx: InitContext, host: Arc<dyn HostClient>) -> SdkResult<InitOutcome> {
        self.host
            .set(host)
            .map_err(|_| PluginError::internal("Tool script host is already initialized"))?;
        Ok(InitOutcome::ack(self.manifest()))
    }

    #[tool(
        name = "run",
        tags(execute),
        output(ProgramReport),
        summary = "Run JavaScript that chains tools through the current session.",
        help = r#"Supply a JavaScript function body. `tools.search(query, limit = 5)` returns `{tools, truncated}` with matching names and input schemas. `tools.call(name, args = {})` calls a tool serially with the session's usual permissions and returns `{payload, text, truncated, attachment_count}`. Errors throw with a `code` property. Host I/O goes through the tool interface. Interactive, streaming and background-launch tools require direct calls. Each program can perform up to 50 calls or searches and return up to 16 KiB of JSON.

Return a file's size in bytes.

```json
{"code":"const f = tools.call('fs.stat', {path: 'README.md'}); return {size: f.payload.size};"}
```

The payload includes the returned value and settled calls. When an unhandled error stops the script, `error` describes the failure and `calls` still records the calls that settled.

```json
{"result":{"size":4096},"truncated":false,"calls":[{"name":"fs.stat","status":"succeeded"}]}
```"#,
        translations(locale(
            "zh-CN",
            summary = "运行工具脚本，用 JavaScript 串联当前会话的工具调用。"
        ))
    )]
    async fn run(
        &self,
        ctx: &ToolInvokeContext<'_>,
        input: RunInput,
    ) -> SdkResult<ToolInvokeOutput> {
        if input.code.len() > MAX_SOURCE_BYTES {
            return Err(PluginError::invalid_params(
                "JavaScript source exceeds 64 KiB",
            ));
        }
        let host = self.host.get().ok_or_else(|| {
            PluginError::from_kind(
                PluginErrorKind::HostUnavailable,
                "Tool script host is unavailable",
            )
        })?;
        let report = run_program(host.as_ref(), input.code, &ctx.cancellation).await?;
        let summary = if report.error.is_some() {
            format!(
                "Tool script stopped after {} tool calls",
                report.calls.len()
            )
        } else {
            format!("Tool script completed {} tool calls", report.calls.len())
        };
        let payload =
            serde_json::to_value(report).map_err(|error| PluginError::internal_error(&error))?;
        Ok(ToolInvokeOutput::from_parts(
            "Run Tool Script",
            summary,
            payload.to_string(),
            Some(payload),
            Default::default(),
            Vec::new(),
        ))
    }
}

#[derive(Serialize, JsonSchema)]
struct ProgramReport {
    /// The returned JSON value, a preview string when truncated, or null on failure.
    result: Value,
    /// The return value exceeded its output limit and was replaced with a preview.
    truncated: bool,
    /// Settled nested tool calls in execution order.
    calls: Vec<CallReceipt>,
    /// Execution error if the script stopped before returning.
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Serialize, JsonSchema)]
struct CallReceipt {
    /// Tool name submitted by the script.
    name: String,
    /// Outcome of the nested call.
    status: CallStatus,
    /// Host error code for a denied or failed call.
    #[serde(skip_serializing_if = "Option::is_none")]
    error_code: Option<String>,
}

#[derive(Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum CallStatus {
    Succeeded,
    Denied,
    Failed,
}

async fn run_program(
    host: &dyn HostClient,
    code: String,
    cancellation: &CancellationToken,
) -> SdkResult<ProgramReport> {
    let _permit = tokio::select! {
        biased;
        _ = cancellation.cancelled() => return Err(cancelled()),
        permit = WORKERS.acquire() => permit.map_err(|_| cancelled())?,
    };
    let mut calls = Vec::new();
    let result = worker::execute(host, code, cancellation, &mut calls).await?;
    let (result, truncated, error) = match result {
        Ok((result, truncated)) => (result, truncated, None),
        Err(error) => (Value::Null, false, Some(error)),
    };
    Ok(ProgramReport {
        result,
        truncated,
        calls,
        error,
    })
}

fn cancelled() -> PluginError {
    PluginError::from_kind(
        PluginErrorKind::Cancelled,
        "Tool script invocation was cancelled",
    )
}

async fn handle_operation(
    host: &dyn HostClient,
    operation: vm::Operation,
    calls: &mut Vec<CallReceipt>,
) -> String {
    match operation {
        vm::Operation::Call { name, args } => match host.invoke_tool(name.clone(), args).await {
            Ok(output) => {
                calls.push(CallReceipt {
                    name,
                    status: CallStatus::Succeeded,
                    error_code: None,
                });
                encode_tool_output(output)
            }
            Err(error) => {
                let code = error.failure.code.as_str();
                let status = match error.kind {
                    PluginErrorKind::PolicyDenied | PluginErrorKind::UserDeclined => {
                        CallStatus::Denied
                    }
                    _ => CallStatus::Failed,
                };
                calls.push(CallReceipt {
                    name,
                    status,
                    error_code: Some(code.to_owned()),
                });
                encode_error(code, &error.to_string())
            }
        },
        vm::Operation::Search { query, limit } => match host.list_tools().await {
            Ok(tools) => encode_search_results(tools, &query, limit),
            Err(error) => encode_error(error.failure.code.as_str(), &error.to_string()),
        },
    }
}

fn encode_tool_output(output: ToolInvokeOutput) -> String {
    #[derive(Serialize)]
    struct Response<'a> {
        text: &'a str,
        payload: Option<Value>,
        attachment_count: usize,
        truncated: bool,
    }

    // Reserve space for structured data and JSON escaping of the text preview.
    let text = prefix(&output.output_text, MAX_INTERMEDIATE_BYTES / 8);
    let mut result = Response {
        text,
        payload: output.payload,
        attachment_count: output.attachments.len(),
        truncated: text.len() < output.output_text.len(),
    };
    if let Some(encoded) = bounded_json(&result, MAX_INTERMEDIATE_BYTES) {
        return encoded;
    }
    result.payload = None;
    result.truncated = true;
    serde_json::to_string(&result).expect("bounded Tool script tool response serializes")
}

fn encode_search_results(tools: Vec<ToolDescriptor>, query: &str, limit: usize) -> String {
    #[derive(Serialize)]
    struct Response<'a> {
        tools: &'a [ToolDescriptor],
        truncated: bool,
    }

    let documents: Vec<_> = tools
        .iter()
        .map(|tool| {
            ToolSearchDocument::new(
                tool.name.clone(),
                tool.summary.clone().unwrap_or_default(),
                Vec::new(),
                tool.plugin_id.clone(),
            )
        })
        .collect();
    let ranked = search_tools(&documents, query, limit);
    let mut tools = tools
        .into_iter()
        .map(|tool| (tool.name.clone(), tool))
        .collect::<std::collections::HashMap<_, _>>();
    let mut matches = ranked
        .into_iter()
        .filter_map(|hit| tools.remove(&hit.name))
        .map(|mut tool| {
            // Full help is available through the normal tool API.
            tool.help = None;
            tool
        })
        .collect::<Vec<_>>();
    let count = matches.len();
    loop {
        if let Some(encoded) = bounded_json(
            &Response {
                tools: &matches,
                truncated: matches.len() < count,
            },
            MAX_INTERMEDIATE_BYTES,
        ) {
            return encoded;
        }
        matches.pop();
    }
}

fn encode_error(code: &str, message: &str) -> String {
    json!({"error_code": code, "error": prefix(message, 1024)}).to_string()
}

fn prefix(text: &str, limit: usize) -> &str {
    &text[..text.floor_char_boundary(limit)]
}

fn bounded_json(value: &impl Serialize, limit: usize) -> Option<String> {
    struct Buffer(Vec<u8>, usize);
    impl Write for Buffer {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            if bytes.len() > self.1.saturating_sub(self.0.len()) {
                return Err(io::Error::other("Tool script JSON exceeds its size limit"));
            }
            self.0.extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let mut buffer = Buffer(Vec::new(), limit);
    serde_json::to_writer(&mut buffer, value).ok()?;
    String::from_utf8(buffer.0).ok()
}
