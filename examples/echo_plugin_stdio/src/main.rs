//! Same `Plugin` impl as the cdylib example, but exported as a stdio
//! JSON-RPC server. Used by the host's stdio transport integration test.

use agena_plugin_sdk::prelude::*;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, ToolCommand)]
#[tool_command(
    tool = "echo",
    description = "Echo via stdio.",
    summary = "Echo text over stdio transport.",
    trim("text"),
    handler_receiver = EchoPlugin,
    handle = EchoPlugin::invoke_echo,
    stream_handle = EchoPlugin::invoke_echo_stream,
    handle_field = text,
    handle_by_value = true,
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

impl EchoPlugin {
    async fn invoke_echo(&self, text: String) -> Result<ToolInvokeOutput> {
        Ok(ToolInvokeOutput::text(format!("stdio-echo: {text}")))
    }

    async fn invoke_echo_stream(
        &self,
        sink: ToolStreamSink,
        text: String,
    ) -> Result<ToolStreamEnd> {
        sink.text("stdio-").await;
        sink.text(format!("echo: {text}")).await;
        Ok(ToolStreamEnd {
            stream_id: sink.stream_id().to_string(),
            title: String::new(),
            output_text: format!("stdio-echo: {text}"),
            payload: None,
            metadata: Default::default(),
            attachments: Vec::new(),
        })
    }
}

#[plugin]
impl Plugin for EchoPlugin {
    #[plugin_manifest_method(
        id = "echo-stdio",
        version = env!("CARGO_PKG_VERSION"),
        description = "Echo via stdio.",
        hooks = HookSubscription::INIT
            | HookSubscription::TOOL_INVOKE
            | HookSubscription::TOOL_INVOKE_STREAM
            | HookSubscription::SHELL_ENV
            | HookSubscription::CHAT_PARAMS,
        display = compact,
        tool_surface = EchoToolInput,
    )]
    fn manifest(&self) -> PluginManifest {}

    #[plugin_init_method]
    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {}

    #[plugin_tool_invoke_method(surface(EchoToolInput))]
    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {}

    #[plugin_tool_invoke_stream_method(surface(EchoToolInput))]
    async fn tool_invoke_stream(
        &self,
        input: ToolInvokeInput,
        sink: ToolStreamSink,
    ) -> Result<ToolStreamEnd> {
        let _ = (input, sink);
    }

    async fn shell_env(&self, _input: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_STDIO_PLUGIN", "1")))
    }

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
