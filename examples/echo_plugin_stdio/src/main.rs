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
    export = stdio
)]
impl EchoPlugin {
    #[tool(
        name = "echo",
        summary = "Echo text over stdio transport.",
        stream = echo_stream,
        tags(read_only)
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

    #[hook(
        init,
        summary = "Initialize stdio echo plugin",
        help = "Acknowledges initialization of the example stdio plugin transport."
    )]
    async fn init(&self, _ctx: InitContext, _host: Arc<dyn HostClient>) -> Result<InitOutcome> {
        Ok(InitOutcome::ack(Plugin::manifest(self)))
    }

    #[hook(shell.env, summary = "Set stdio echo environment", help = "Sets an environment marker for commands launched by the stdio example plugin.")]
    async fn shell_env(&self, _input: ShellEnvInput) -> ShellEnvPatch {
        ShellEnvPatch::set("AGENA_STDIO_PLUGIN", "1")
    }

    #[hook(chat.params, summary = "Adjust example chat parameters", help = "Demonstrates how to modify a model completion request through a chat parameter Hook.")]
    async fn chat_params(&self, _input: ChatParamsInput) -> ChatParamsPatch {
        ChatParamsPatch {
            params: Some(json!({ "stop": ["\nHuman:"] })),
        }
    }
}
