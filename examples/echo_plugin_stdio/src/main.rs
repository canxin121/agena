//! Same `Plugin` impl as the cdylib example, but exported as a stdio
//! JSON-RPC server. Used by the host's stdio transport integration test.

use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct EchoPlugin;

#[agena_plugin(
    namespace = "example",
    name = "echo_stdio",
    version = env!("CARGO_PKG_VERSION"),
    summary = "Echo via stdio.",
    display = compact,
    export = stdio
)]
impl EchoPlugin {
    #[tool(
        name = "echo",
        summary = "Echo text over stdio transport.",
        read_only,
        streaming,
        stream = echo_stream,
        concurrency_safe
    )]
    async fn echo(&self, #[arg(trim, non_empty)] text: String) -> String {
        format!("stdio-echo: {text}")
    }

    async fn echo_stream(&self, text: String, sink: ToolStreamSink) -> String {
        let output = format!("stdio-echo: {text}");
        sink.text("stdio-").await;
        sink.text(format!("echo: {text}")).await;
        output
    }

    #[hook]
    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }

    #[hook]
    async fn shell_env(&self, _input: ShellEnvInput) -> ShellEnvPatch {
        ShellEnvPatch::set("AGENA_STDIO_PLUGIN", "1")
    }

    #[hook]
    async fn chat_params(&self, _input: ChatParamsInput) -> ChatParamsPatch {
        ChatParamsPatch {
            params: Some(json!({ "stop": ["\nHuman:"] })),
        }
    }
}
