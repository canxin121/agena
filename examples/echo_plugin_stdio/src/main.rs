//! Same `Plugin` impl as the cdylib example, but exported as a stdio
//! JSON-RPC server. Used by the host's stdio transport integration test.

use agena_plugin_sdk::prelude::*;

#[derive(Default)]
struct EchoPlugin;

#[async_trait]
impl Plugin for EchoPlugin {
    fn manifest(&self) -> PluginManifest {
        PluginManifest::builder("echo-stdio", env!("CARGO_PKG_VERSION"))
            .hooks(
                HookSubscription::INIT
                    | HookSubscription::TOOL_INVOKE
                    | HookSubscription::SHELL_ENV
                    | HookSubscription::CHAT_PARAMS,
            )
            .entry(
                PluginEntryDecl::new(
                    "echo",
                    json!({"type":"object","properties":{"text":{"type":"string"}},"required":["text"]}),
                )
                .description("Echo via stdio."),
            )
            .build()
    }

    async fn tool_invoke(&self, input: ToolInvokeInput) -> Result<ToolInvokeOutput> {
        let text = input
            .input
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        Ok(ToolInvokeOutput::text(format!("stdio-echo: {text}")))
    }

    async fn shell_env(&self, _: ShellEnvInput) -> Result<Option<ShellEnvPatch>> {
        Ok(Some(ShellEnvPatch::set("AGENA_STDIO_PLUGIN", "1")))
    }

    async fn chat_params(
        &self,
        _: ChatParamsInput,
    ) -> Result<Option<ChatParamsPatch>> {
        Ok(Some(ChatParamsPatch {
            params: json!({ "stop": ["\nHuman:"] }),
        }))
    }
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> std::io::Result<()> {
    agena_plugin_sdk::drivers::stdio::serve_stdio(EchoPlugin::default()).await
}
