//! Same `Plugin` impl as the cdylib example, but exported as a stdio
//! JSON-RPC server. Used by the host's stdio transport integration test.

use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolCommand)]
#[tool_command(
    tool = "echo",
    description = "Echo via stdio.",
    summary = "Echo text over stdio transport.",
    trim("text"),
    streaming = "streaming",
    concurrency_safe = true
)]
#[serde(deny_unknown_fields)]
struct EchoToolInput {
    /// Text payload to echo back.
    text: String,
}

#[derive(Default)]
struct EchoPlugin;

#[plugin(
    id = "echo-stdio",
    version = env!("CARGO_PKG_VERSION"),
    description = "Echo via stdio.",
    display = compact
)]
impl EchoPlugin {
    #[tool]
    async fn invoke_echo(&self, input: EchoToolInput) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text(format!(
            "stdio-echo: {}",
            input.text
        )))
    }

    #[tool_stream]
    async fn invoke_echo_stream(
        &self,
        input: EchoToolInput,
        sink: ToolStreamSink,
    ) -> Result<ToolStreamEnd> {
        sink.text("stdio-").await;
        sink.text(format!("echo: {}", input.text)).await;
        Ok(ToolStreamEnd::text(
            sink.stream_id(),
            format!("stdio-echo: {}", input.text),
        ))
    }

    #[hook]
    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }

    #[hook]
    async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_STDIO_PLUGIN", "1")))
    }

    #[hook]
    async fn chat_params(&self, _input: ChatParamsInput) -> Result<Option<ChatParamsPatch>> {
        Ok(Some(ChatParamsPatch {
            params: Some(json!({ "stop": ["\nHuman:"] })),
        }))
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::io::Result<()> {
    agena_plugin_sdk::drivers::stdio::serve_stdio(EchoPlugin).await
}
